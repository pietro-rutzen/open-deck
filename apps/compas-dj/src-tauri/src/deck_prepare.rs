//! Playback becomes ready after decode. Beatgrid, key, colored waveform, and loudness
//! are a later phase on the same load generation.
//!
//! ReplayGain is measured with that later phase and is not written to the engine.
//! `LoadDeck` sets pregain to unity. Applying the measurement afterwards can sit behind
//! a Play, cue, or scratch command that has not yet updated telemetry, which would change
//! the level of a deck the listener already started. The measured factor stays on the
//! analysis event for a future load that can install it in the same command group as the buffer.

use std::sync::Arc;

use compas_audio::{compute_peaks, silence_spans, DeckBuffer};
use compas_core::TrackMetadata;
use compas_dsp::analysis::{self, BeatGrid, KeyEstimate};

pub(crate) struct DecodedTrack {
    pub buffer: DeckBuffer,
    pub path: String,
    pub title: String,
    pub artist: String,
}

pub(crate) struct ReadyBody {
    pub generation: u64,
    pub path: String,
    pub title: String,
    pub artist: String,
    pub duration_ms: u64,
    pub source_rate: u32,
    pub frames: usize,
    pub bpm: f32,
    pub bpm_confidence: f32,
    pub first_beat_sec: f32,
    pub beat_interval_sec: f32,
    pub key_camelot: String,
    pub key_name: String,
    pub peaks: Vec<f32>,
    pub band_peaks: Vec<[f32; 3]>,
    pub replay_gain: f32,
    pub analysis_pending: bool,
    pub silence: Vec<(f64, f64)>,
}

pub(crate) struct PreparedPlayback {
    pub buffer: Arc<DeckBuffer>,
    pub body: ReadyBody,
}

pub(crate) enum PrepareStep {
    Ready(Box<PreparedPlayback>),
    Superseded,
    Failed(String),
}

pub(crate) struct RawAnalysis {
    pub grid: BeatGrid,
    pub key: KeyEstimate,
    pub band_peaks: Vec<[f32; 3]>,
    pub replay_gain: f32,
}

pub(crate) struct AnalysisBody {
    pub generation: u64,
    pub path: String,
    pub bpm: f32,
    pub bpm_confidence: f32,
    pub first_beat_sec: f32,
    pub beat_interval_sec: f32,
    pub key_camelot: String,
    pub key_name: String,
    pub band_peaks: Vec<[f32; 3]>,
    pub measured_replay_gain: f32,
    pub apply_replay_gain: bool,
}

pub(crate) enum AnalysisStep {
    Ready(AnalysisBody),
    Superseded,
    Failed(String),
}

pub(crate) fn decoded_from(buffer: DeckBuffer, metadata: TrackMetadata) -> DecodedTrack {
    DecodedTrack {
        path: metadata.provider_id,
        title: metadata.title,
        artist: metadata.artist,
        buffer,
    }
}

pub(crate) fn prepare_step(
    generation: u64,
    bin_frames: usize,
    current: impl Fn() -> bool,
    decode: impl FnOnce() -> Result<Option<DecodedTrack>, String>,
) -> PrepareStep {
    if !current() {
        return PrepareStep::Superseded;
    }
    let decoded = match decode() {
        Ok(Some(decoded)) => decoded,
        Ok(None) => return PrepareStep::Superseded,
        Err(error) => return PrepareStep::Failed(error),
    };
    if !current() {
        return PrepareStep::Superseded;
    }
    let silence = silence_spans(&decoded.buffer.samples, decoded.buffer.source_rate);
    let buffer = decoded.buffer.with_silence(silence.clone());
    let peaks = compute_peaks(&buffer.samples, bin_frames);
    if !current() {
        return PrepareStep::Superseded;
    }
    let body = ReadyBody {
        generation,
        path: decoded.path,
        title: decoded.title,
        artist: decoded.artist,
        duration_ms: buffer.duration_ms(),
        source_rate: buffer.source_rate,
        frames: buffer.frames(),
        bpm: 0.0,
        bpm_confidence: 0.0,
        first_beat_sec: 0.0,
        beat_interval_sec: 0.0,
        key_camelot: String::new(),
        key_name: String::new(),
        peaks,
        band_peaks: Vec::new(),
        replay_gain: 1.0,
        analysis_pending: true,
        silence,
    };
    PrepareStep::Ready(Box::new(PreparedPlayback {
        buffer: Arc::new(buffer),
        body,
    }))
}

pub(crate) fn measure_analysis(buffer: &DeckBuffer) -> RawAnalysis {
    let max_frames = crate::ANALYSIS_MAX_SECS * buffer.source_rate as usize;
    let frames = buffer.frames().min(max_frames);
    let mut mono = Vec::with_capacity(frames);
    for frame in 0..frames {
        mono.push(0.5 * (buffer.samples[frame * 2] + buffer.samples[frame * 2 + 1]));
    }
    RawAnalysis {
        grid: analysis::estimate_beatgrid(&mono, buffer.source_rate),
        key: analysis::estimate_key(&mono, buffer.source_rate),
        band_peaks: analysis::band_peaks(
            &buffer.samples,
            buffer.source_rate,
            crate::WAVEFORM_BIN_FRAMES,
        ),
        replay_gain: analysis::replaygain_linear(&buffer.samples),
    }
}

pub(crate) fn analysis_step(
    generation: u64,
    path: &str,
    current: impl Fn() -> bool,
    buffer: &DeckBuffer,
    analyze: impl FnOnce(&DeckBuffer) -> Result<RawAnalysis, String>,
) -> AnalysisStep {
    if !current() {
        return AnalysisStep::Superseded;
    }
    let measured = match analyze(buffer) {
        Ok(measured) => measured,
        Err(error) => {
            return if current() {
                AnalysisStep::Failed(error)
            } else {
                AnalysisStep::Superseded
            };
        }
    };
    if !current() {
        return AnalysisStep::Superseded;
    }
    let interval = measured.grid.beat_interval_sec;
    let usable = interval.is_finite()
        && interval > 0.0
        && measured.grid.bpm.is_finite()
        && measured.grid.bpm > 0.0;
    AnalysisStep::Ready(AnalysisBody {
        generation,
        path: path.to_string(),
        bpm: if usable { measured.grid.bpm } else { 0.0 },
        bpm_confidence: if usable && measured.grid.confidence.is_finite() {
            measured.grid.confidence
        } else {
            0.0
        },
        first_beat_sec: if usable && measured.grid.first_beat_sec.is_finite() {
            measured.grid.first_beat_sec
        } else {
            0.0
        },
        beat_interval_sec: if usable { interval } else { 0.0 },
        key_camelot: measured.key.camelot,
        key_name: measured.key.name,
        band_peaks: measured.band_peaks,
        measured_replay_gain: if measured.replay_gain.is_finite() {
            measured.replay_gain
        } else {
            1.0
        },
        apply_replay_gain: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::time::Instant;

    fn tone(rate: u32, seconds: f32, amplitude: f32) -> DeckBuffer {
        let frames = (rate as f32 * seconds).round() as usize;
        let mut samples = Vec::with_capacity(frames * 2);
        for index in 0..frames {
            let sample =
                (index as f32 / rate as f32 * 440.0 * std::f32::consts::TAU).sin() * amplitude;
            samples.push(sample);
            samples.push(sample);
        }
        DeckBuffer::new(samples, rate)
    }

    fn track(buffer: DeckBuffer, path: &str) -> DecodedTrack {
        DecodedTrack {
            buffer,
            path: path.to_string(),
            title: "Fixture".into(),
            artist: "Open Deck".into(),
        }
    }

    fn ready_body(step: PrepareStep) -> (Arc<DeckBuffer>, ReadyBody) {
        match step {
            PrepareStep::Ready(prepared) => (prepared.buffer, prepared.body),
            PrepareStep::Superseded => panic!("load was superseded"),
            PrepareStep::Failed(error) => panic!("load failed: {error}"),
        }
    }

    #[test]
    fn playback_is_ready_before_analysis_and_has_no_grid() {
        let phase = AtomicUsize::new(0);
        let current = AtomicBool::new(true);
        let step = prepare_step(
            7,
            512,
            || current.load(Ordering::SeqCst),
            || {
                assert_eq!(phase.fetch_add(1, Ordering::SeqCst), 0);
                Ok(Some(track(tone(8_000, 0.25, 0.2), "/music/same.wav")))
            },
        );
        let (buffer, body) = ready_body(step);
        assert_eq!(phase.load(Ordering::SeqCst), 1);
        assert_eq!(body.generation, 7);
        assert!(body.analysis_pending);
        assert_eq!(body.bpm, 0.0);
        assert_eq!(body.beat_interval_sec, 0.0);
        assert!(body.key_camelot.is_empty());
        assert!(body.band_peaks.is_empty());
        assert_eq!(body.replay_gain, 1.0);
        assert!(!body.peaks.is_empty());
        assert!(body.peaks.len() < buffer.frames());
        let analysis = analysis_step(
            7,
            &body.path,
            || current.load(Ordering::SeqCst),
            &buffer,
            |_| {
                assert_eq!(phase.fetch_add(1, Ordering::SeqCst), 1);
                Ok(RawAnalysis {
                    grid: BeatGrid {
                        bpm: 120.0,
                        confidence: 0.9,
                        first_beat_sec: 0.1,
                        beat_interval_sec: 0.5,
                    },
                    key: KeyEstimate {
                        camelot: "8A".into(),
                        name: "Am".into(),
                        confidence: 1.0,
                    },
                    band_peaks: vec![[0.2, 0.4, 0.1]],
                    replay_gain: 1.8,
                })
            },
        );
        let AnalysisStep::Ready(done) = analysis else {
            panic!("analysis did not finish");
        };
        assert_eq!(phase.load(Ordering::SeqCst), 2);
        assert_eq!(done.bpm, 120.0);
        assert_eq!(done.beat_interval_sec, 0.5);
        assert_eq!(done.band_peaks, vec![[0.2, 0.4, 0.1]]);
        assert!((done.measured_replay_gain - 1.8).abs() < 1e-6);
        assert!(!done.apply_replay_gain);
    }

    #[test]
    fn replaced_load_drops_both_phases_and_same_path_keeps_its_own_generation() {
        let current = AtomicBool::new(false);
        let step = prepare_step(
            1,
            512,
            || current.load(Ordering::SeqCst),
            || panic!("superseded decode must not start"),
        );
        assert!(matches!(step, PrepareStep::Superseded));

        current.store(true, Ordering::SeqCst);
        let (buffer, first) = ready_body(prepare_step(
            1,
            512,
            || true,
            || Ok(Some(track(tone(8_000, 0.1, 0.2), "/music/same.wav"))),
        ));
        current.store(false, Ordering::SeqCst);
        let dropped = analysis_step(
            first.generation,
            &first.path,
            || false,
            &buffer,
            |_| panic!("analysis started after replacement"),
        );
        assert!(matches!(dropped, AnalysisStep::Superseded));

        let (replacement, second) = ready_body(prepare_step(
            2,
            512,
            || true,
            || Ok(Some(track(tone(8_000, 0.1, 0.2), "/music/same.wav"))),
        ));
        assert_eq!(second.path, first.path);
        assert_ne!(second.generation, first.generation);
        let late = analysis_step(
            first.generation,
            &first.path,
            || false,
            &replacement,
            |_| {
                Ok(RawAnalysis {
                    grid: BeatGrid {
                        bpm: 90.0,
                        confidence: 1.0,
                        first_beat_sec: 0.0,
                        beat_interval_sec: 60.0 / 90.0,
                    },
                    key: KeyEstimate {
                        camelot: "1A".into(),
                        name: "Abm".into(),
                        confidence: 1.0,
                    },
                    band_peaks: vec![[1.0, 0.0, 0.0]],
                    replay_gain: 0.5,
                })
            },
        );
        assert!(matches!(late, AnalysisStep::Superseded));
        let AnalysisStep::Ready(done) = analysis_step(
            2,
            &second.path,
            || true,
            &replacement,
            |_| {
                Ok(RawAnalysis {
                    grid: BeatGrid {
                        bpm: 140.0,
                        confidence: 0.4,
                        first_beat_sec: 0.2,
                        beat_interval_sec: 60.0 / 140.0,
                    },
                    key: KeyEstimate {
                        camelot: "4A".into(),
                        name: "Fm".into(),
                        confidence: 1.0,
                    },
                    band_peaks: vec![[0.0, 1.0, 0.0]],
                    replay_gain: 0.7,
                })
            },
        ) else {
            panic!("replacement analysis missing");
        };
        assert_eq!(done.generation, 2);
        assert_eq!(done.bpm, 140.0);
        assert!(!done.apply_replay_gain);
    }

    #[test]
    fn decode_and_analysis_failures_do_not_publish_the_other_phase() {
        let failed = prepare_step(3, 512, || true, || Err("bad header".into()));
        assert!(matches!(failed, PrepareStep::Failed(message) if message == "bad header"));
        let (buffer, body) = ready_body(prepare_step(
            4,
            512,
            || true,
            || Ok(Some(track(tone(8_000, 0.1, 0.2), "/music/a.wav"))),
        ));
        assert!(body.analysis_pending);
        let analysis = analysis_step(
            4,
            &body.path,
            || true,
            &buffer,
            |_| Err("beatgrid failed".into()),
        );
        assert!(matches!(analysis, AnalysisStep::Failed(message) if message == "beatgrid failed"));
        let other = analysis_step(
            9,
            "/music/b.wav",
            || true,
            &buffer,
            |_| {
                Ok(RawAnalysis {
                    grid: BeatGrid {
                        bpm: 100.0,
                        confidence: 1.0,
                        first_beat_sec: 0.0,
                        beat_interval_sec: 0.6,
                    },
                    key: KeyEstimate {
                        camelot: "8B".into(),
                        name: "C".into(),
                        confidence: 1.0,
                    },
                    band_peaks: Vec::new(),
                    replay_gain: 1.0,
                })
            },
        );
        let AnalysisStep::Ready(done) = other else {
            panic!("other deck analysis missing");
        };
        assert_eq!(done.generation, 9);
        assert_eq!(done.path, "/music/b.wav");
    }

    #[test]
    fn unusable_grid_and_non_finite_gain_cannot_arm_sync_or_a_level_change() {
        let buffer = tone(8_000, 0.05, 0.01);
        let step = analysis_step(
            1,
            "/music/quiet.wav",
            || true,
            &buffer,
            |_| {
                Ok(RawAnalysis {
                    grid: BeatGrid {
                        bpm: f32::NAN,
                        confidence: 1.0,
                        first_beat_sec: f32::INFINITY,
                        beat_interval_sec: 0.0,
                    },
                    key: KeyEstimate {
                        camelot: String::new(),
                        name: String::new(),
                        confidence: 0.0,
                    },
                    band_peaks: Vec::new(),
                    replay_gain: f32::NAN,
                })
            },
        );
        let AnalysisStep::Ready(done) = step else {
            panic!("analysis missing");
        };
        assert_eq!(done.bpm, 0.0);
        assert_eq!(done.beat_interval_sec, 0.0);
        assert_eq!(done.measured_replay_gain, 1.0);
        assert!(!done.apply_replay_gain);
    }

    #[test]
    fn in_flight_analysis_is_discarded_when_the_load_is_replaced() {
        let current = AtomicBool::new(true);
        let buffer = tone(8_000, 0.05, 0.2);
        let step = analysis_step(
            1,
            "/music/same.wav",
            || current.load(Ordering::SeqCst),
            &buffer,
            |_| {
                current.store(false, Ordering::SeqCst);
                Ok(RawAnalysis {
                    grid: BeatGrid {
                        bpm: 128.0,
                        confidence: 1.0,
                        first_beat_sec: 0.0,
                        beat_interval_sec: 60.0 / 128.0,
                    },
                    key: KeyEstimate {
                        camelot: "8A".into(),
                        name: "Am".into(),
                        confidence: 1.0,
                    },
                    band_peaks: vec![[1.0, 1.0, 1.0]],
                    replay_gain: 2.0,
                })
            },
        );
        assert!(matches!(step, AnalysisStep::Superseded));
    }

    /// Synthetic worker split only. These durations are not app or controller latency.
    #[test]
    fn offline_ready_phase_is_separate_from_analysis() {
        let buffer = tone(44_100, 8.0, 0.01);
        let mut ready_ms = [0.0_f64; 3];
        let mut analysis_ms = [0.0_f64; 3];
        for index in 0..3 {
            let started = Instant::now();
            let (prepared, body) = ready_body(prepare_step(
                1,
                512,
                || true,
                || Ok(Some(track(buffer.clone(), "/music/timing.wav"))),
            ));
            ready_ms[index] = started.elapsed().as_secs_f64() * 1_000.0;
            assert!(body.analysis_pending);
            assert_eq!(body.replay_gain, 1.0);
            let started = Instant::now();
            let measured = measure_analysis(&prepared);
            analysis_ms[index] = started.elapsed().as_secs_f64() * 1_000.0;
            assert!(!measured.replay_gain.is_finite() || measured.replay_gain > 1.0);
            assert!(measured.band_peaks.len() == body.peaks.len());
        }
        eprintln!(
            "DECK_READY_SYNTHETIC_MS ready={ready_ms:?} analysis={analysis_ms:?} buffer=8s/44100Hz/3-repeats"
        );
    }
}

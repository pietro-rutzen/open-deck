//! Pull-only deck meters. While a strip is visible the audio thread fills a preallocated
//! stereo ring. Analysis runs on a blocking worker, and an unchanged sample counter skips it.
use std::sync::{Arc, Mutex};

use compas_audio::{ScopeBlock, NUM_DECKS, SPECTRUM_FRAMES};
use rustfft::{num_complex::Complex32, Fft, FftPlanner};
use serde::{Deserialize, Serialize};
use tauri::State;

use crate::EngineHandle;

const TRACE_POINTS: usize = 192;
const DB_FLOOR: f32 = -72.0;
/// Zero-pad the captured window so an off-bin bass peak can be localized.
const FFT_LEN: usize = 16384;
const SPECTRUM_ROWS: usize = 384;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DeckMeterFrame {
    left: Vec<f32>,
    right: Vec<f32>,
    spectrum: Vec<f32>,
    spectrum_hz: Vec<f32>,
    min_hz: f32,
    max_hz: f32,
    /// Sample counter for this window. Unchanged means the display can skip the repaint.
    sequence: u64,
}

#[derive(Default)]
pub(crate) struct DeckMeters {
    slots: [Mutex<MeterSlot>; NUM_DECKS],
}

#[derive(Default)]
struct MeterSlot {
    captured: Vec<[f32; 2]>,
    spectrum: Option<Spectrum>,
}

struct Spectrum {
    fft: Arc<dyn Fft<f32>>,
    window: Vec<f32>,
    /// Coherent gain. A unit sine's positive-frequency bin has magnitude `window_sum / 2`.
    window_sum: f32,
    buffer: Vec<Complex32>,
    scratch: Vec<Complex32>,
    db: Vec<f32>,
    row_db: Vec<f32>,
    row_hz: Vec<f32>,
    ordered_hz: Vec<f32>,
    sample_rate: f32,
}

impl Spectrum {
    fn new(n: usize, sample_rate: f32) -> Self {
        let fft_len = FFT_LEN.max(n).next_power_of_two();
        let fft = FftPlanner::new().plan_fft_forward(fft_len);
        // 4-term Blackman-Harris. Its sidelobes stay near −92 dB, so a tone is a
        // narrow band. Amplitude still uses the window's coherent gain below.
        let window: Vec<_> = (0..n)
            .map(|i| {
                let phase = std::f32::consts::TAU * i as f32 / n as f32;
                0.35875 - 0.48829 * phase.cos() + 0.14128 * (2.0 * phase).cos()
                    - 0.01168 * (3.0 * phase).cos()
            })
            .collect();
        let window_sum = window.iter().sum::<f32>().max(1.0);
        let scratch = vec![Complex32::default(); fft.get_inplace_scratch_len()];
        Self {
            fft,
            window,
            window_sum,
            scratch,
            db: vec![DB_FLOOR; fft_len / 2 + 1],
            row_db: vec![DB_FLOOR; SPECTRUM_ROWS],
            row_hz: vec![0.0; SPECTRUM_ROWS],
            ordered_hz: vec![0.0; SPECTRUM_ROWS],
            sample_rate,
            buffer: vec![Complex32::default(); fft_len],
        }
    }

    fn analyze(&mut self, samples: &[[f32; 2]]) -> (Vec<f32>, Vec<f32>) {
        self.db.fill(DB_FLOOR);
        let n = self.buffer.len();
        for channel in 0..2 {
            for slot in &mut self.buffer {
                *slot = Complex32::default();
            }
            for ((bin, frame), win) in self.buffer.iter_mut().zip(samples).zip(&self.window) {
                *bin = Complex32::new(finite(frame[channel]) * win, 0.0);
            }
            self.fft
                .process_with_scratch(&mut self.buffer, &mut self.scratch);
            for (k, bin) in self.buffer.iter().take(n / 2 + 1).enumerate() {
                let mirrors = if k == 0 || k == n / 2 { 1.0 } else { 2.0 };
                let amplitude = bin.norm() * mirrors / self.window_sum;
                let db = if amplitude > 1e-8 {
                    (20.0 * amplitude.log10()).clamp(DB_FLOOR, 12.0)
                } else {
                    DB_FLOOR
                };
                self.db[k] = self.db[k].max(db);
            }
        }
        let (min_hz, max_hz) = frequency_range(self.sample_rate);
        let bin_hz = self.sample_rate / n as f32;
        let first = (min_hz / bin_hz).ceil().max(1.0) as usize;
        let last = ((max_hz / bin_hz).floor() as usize).clamp(first, n / 2 - 1);
        self.row_db.fill(DB_FLOOR);
        self.row_hz.fill(0.0);
        let log_span = (max_hz / min_hz).ln();
        for k in first..=last {
            let (bin, level) = interpolate_peak(&self.db, k);
            let frequency = (bin * bin_hz).clamp(min_hz, max_hz);
            let row = ((frequency / min_hz).ln() / log_span * (SPECTRUM_ROWS - 1) as f32)
                .round()
                .clamp(0.0, (SPECTRUM_ROWS - 1) as f32) as usize;
            if level >= self.row_db[row] {
                self.row_db[row] = level.clamp(DB_FLOOR, 0.0);
                self.row_hz[row] = frequency;
            }
        }
        // Bass rows are narrower than one FFT bin. Sample the measured curve
        // there so a tone is a solid band, while a real peak keeps its own level.
        for index in 0..SPECTRUM_ROWS {
            if self.row_hz[index] > 0.0 {
                continue;
            }
            let center = min_hz * (max_hz / min_hz).powf(index as f32 / (SPECTRUM_ROWS - 1) as f32);
            self.row_db[index] = sample_db(&self.db, bin_hz, center).clamp(DB_FLOOR, 0.0);
            self.row_hz[index] = center;
        }
        let mut previous = 0.0_f32;
        for i in 0..SPECTRUM_ROWS {
            let center = min_hz * (max_hz / min_hz).powf(i as f32 / (SPECTRUM_ROWS - 1) as f32);
            let mut frequency = if self.row_hz[i] > 0.0 {
                self.row_hz[i]
            } else {
                center
            };
            if frequency <= previous {
                frequency = center.max(previous + bin_hz * 0.01);
            }
            previous = frequency;
            self.ordered_hz[i] = frequency;
        }
        (self.row_db.clone(), self.ordered_hz.clone())
    }
}

/// Move a local maximum to the parabolic vertex. Off-bin sines otherwise
/// read low and sit on the nearest FFT-bin center.
fn interpolate_peak(db: &[f32], k: usize) -> (f32, f32) {
    let level = db[k];
    if k == 0 || k + 1 >= db.len() || level <= DB_FLOOR {
        return (k as f32, level);
    }
    let left = db[k - 1];
    let right = db[k + 1];
    if left >= level || right >= level {
        return (k as f32, level);
    }
    let curvature = left - 2.0 * level + right;
    if curvature >= -1e-4 {
        return (k as f32, level);
    }
    let offset = (0.5 * (left - right) / curvature).clamp(-0.5, 0.5);
    let peak = level - (left - right).powi(2) / (8.0 * curvature);
    (k as f32 + offset, peak)
}

fn frequency_range(rate: f32) -> (f32, f32) {
    (20.0, 20_000.0_f32.min(rate / 2.0))
}

fn sample_db(db: &[f32], bin_hz: f32, hz: f32) -> f32 {
    if db.len() < 2 || !(bin_hz > 0.0) {
        return DB_FLOOR;
    }
    let position = (hz / bin_hz).clamp(0.0, (db.len() - 1) as f32);
    let index = position.floor() as usize;
    let next = (index + 1).min(db.len() - 1);
    let fraction = position - index as f32;
    db[index] + (db[next] - db[index]) * fraction
}

fn finite(v: f32) -> f32 {
    if v.is_finite() {
        v
    } else {
        0.0
    }
}

impl MeterSlot {
    fn frame(&mut self, block: &ScopeBlock, include_spectrum: bool) -> DeckMeterFrame {
        let samples = &block.frames;
        let (min_hz, max_hz) = frequency_range(block.sample_rate);
        let (spectrum, spectrum_hz) = if include_spectrum {
            let needs_plan = self.spectrum.as_ref().is_none_or(|spectrum| {
                spectrum.sample_rate != block.sample_rate || spectrum.window.len() != samples.len()
            });
            if needs_plan {
                self.spectrum = Some(Spectrum::new(samples.len(), block.sample_rate));
            }
            self.spectrum
                .as_mut()
                .map(|spectrum| spectrum.analyze(samples))
                .unwrap_or_default()
        } else {
            (Vec::new(), Vec::new())
        };
        // Stable rising-edge trigger for the short waveform. Keep L/R paired for the scope.
        let span =
            ((block.sample_rate * 0.020).round() as usize).clamp(TRACE_POINTS, samples.len());
        let last_start = samples.len() - span;
        let channel = usize::from(
            samples[last_start..]
                .iter()
                .map(|f| finite(f[1]).abs())
                .fold(0.0_f32, f32::max)
                > samples[last_start..]
                    .iter()
                    .map(|f| finite(f[0]).abs())
                    .fold(0.0_f32, f32::max),
        );
        let start = (1..=last_start)
            .rev()
            .find(|&i| samples[i - 1][channel] <= 0.0 && samples[i][channel] > 0.0)
            .unwrap_or(last_start);
        let (left, right) = (0..TRACE_POINTS)
            .map(|i| &samples[start + i * (span - 1) / (TRACE_POINTS - 1)])
            .map(|s| (finite(s[0]).clamp(-1.0, 1.0), finite(s[1]).clamp(-1.0, 1.0)))
            .unzip();
        DeckMeterFrame {
            left,
            right,
            spectrum,
            spectrum_hz,
            min_hz,
            max_hz,
            sequence: 0,
        }
    }
}

#[derive(Deserialize)]
pub(crate) struct MeterQuery {
    deck: usize,
    spectrum: bool,
    since: u64,
}

#[tauri::command]
pub(crate) async fn read_deck_meters(
    engine: State<'_, EngineHandle>,
    meters: State<'_, Arc<DeckMeters>>,
    queries: Vec<MeterQuery>,
) -> Result<Vec<Option<DeckMeterFrame>>, String> {
    if queries.is_empty() {
        return Ok(Vec::new());
    }
    let telemetry = Arc::clone(&engine.telemetry);
    let meters = Arc::clone(meters.inner());
    tauri::async_runtime::spawn_blocking(move || {
        queries
            .into_iter()
            .map(|query| {
                meters
                    .latest(&telemetry, query.deck, query.spectrum, query.since)
                    .ok()
                    .flatten()
            })
            .collect()
    })
    .await
    .map_err(|_| "Meter worker stopped".to_string())
}

impl DeckMeters {
    fn latest(
        &self,
        telemetry: &compas_audio::DeckTelemetry,
        deck: usize,
        spectrum: bool,
        since: u64,
    ) -> Result<Option<DeckMeterFrame>, String> {
        let slot = self.slots.get(deck).ok_or("Invalid meter deck")?;
        let Ok(mut slot) = slot.try_lock() else {
            return Ok(None);
        };
        if !telemetry.is_loaded(deck) {
            telemetry.release_scope(deck);
            return Ok(None);
        }
        telemetry.request_scope(deck);
        let Some(sequence) = telemetry.scope_sequence(deck) else {
            return Ok(None);
        };
        if sequence == since {
            return Ok(None);
        }
        if slot.captured.capacity() < SPECTRUM_FRAMES {
            slot.captured.reserve_exact(SPECTRUM_FRAMES);
        }
        let Some((rate, sequence)) = telemetry.copy_scope(deck, &mut slot.captured) else {
            return Ok(None);
        };
        let block = ScopeBlock {
            sample_rate: rate,
            frames: std::mem::take(&mut slot.captured),
        };
        let mut frame = slot.frame(&block, spectrum);
        slot.captured = block.frames;
        frame.sequence = sequence;
        Ok(Some(frame))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::hint::black_box;
    use std::time::Instant;

    fn stereo_tone(phase: f32) -> ScopeBlock {
        let rate = 48000.0;
        ScopeBlock {
            sample_rate: rate,
            frames: (0..2048)
                .map(|i| {
                    let s = 0.5 * (std::f32::consts::TAU * 40.0 * i as f32 / 2048.0).sin();
                    [s, phase * s]
                })
                .collect(),
        }
    }

    #[test]
    fn identifies_tone_at_level_and_keeps_opposite_phase_stereo() {
        let mut slot = MeterSlot::default();
        let stereo = slot.frame(&stereo_tone(1.0), true);
        let inverted = slot.frame(&stereo_tone(-1.0), true);
        let (bin, db) = stereo
            .spectrum
            .iter()
            .copied()
            .enumerate()
            .max_by(|a, b| a.1.total_cmp(&b.1))
            .unwrap();
        let hz = stereo.spectrum_hz[bin];
        assert!((hz - 937.5).abs() < 0.5);
        assert!((db + 6.02).abs() < 0.1);
        assert!((db - inverted.spectrum[bin]).abs() < 0.01);
        assert_eq!(inverted.left.len(), TRACE_POINTS);
        assert!(inverted
            .left
            .iter()
            .zip(&inverted.right)
            .all(|(l, r)| (*l + *r).abs() < 1e-6));
    }

    fn tones(rate: f32, input: &[(f32, f32)]) -> ScopeBlock {
        ScopeBlock {
            sample_rate: rate,
            frames: (0..SPECTRUM_FRAMES)
                .map(|i| {
                    let value = input
                        .iter()
                        .map(|(hz, level)| {
                            level * (std::f32::consts::TAU * hz * i as f32 / rate).sin()
                        })
                        .sum();
                    [value, value]
                })
                .collect(),
        }
    }

    #[test]
    fn bass_peak_location_and_level_are_accurate_off_bin() {
        for rate in [44100.0, 48000.0, 96000.0] {
            let tone = tones(rate, &[(65.36, 0.25)]);
            let frame = MeterSlot::default().frame(&tone, true);
            let (i, db) = frame
                .spectrum
                .iter()
                .enumerate()
                .max_by(|a, b| a.1.total_cmp(b.1))
                .unwrap();
            assert!(
                (frame.spectrum_hz[i] - 65.36).abs() < 0.5,
                "rate {rate}: peak {} Hz at {db} dBFS",
                frame.spectrum_hz[i]
            );
            assert!((db + 12.0412).abs() < 0.25);
            assert!(frame.spectrum_hz.windows(2).all(|w| w[0] < w[1]));
            assert_eq!(frame.spectrum.len(), SPECTRUM_ROWS);
        }
        let left_only = ScopeBlock {
            sample_rate: 48_000.0,
            frames: (0..SPECTRUM_FRAMES)
                .map(|i| {
                    let value = 0.5 * (std::f32::consts::TAU * 937.5 * i as f32 / 48_000.0).sin();
                    [value, 0.0]
                })
                .collect(),
        };
        let frame = MeterSlot::default().frame(&left_only, true);
        let db = frame.spectrum.iter().copied().fold(DB_FLOOR, f32::max);
        assert!((db + 6.0206).abs() < 0.15, "left-only sine was {db} dBFS");
    }

    #[test]
    fn close_treble_tones_have_separate_peaks_and_a_valley() {
        let frame =
            MeterSlot::default().frame(&tones(48000.0, &[(8000.0, 0.25), (8240.0, 0.125)]), true);
        for (hz, db) in [(8000.0, -12.0412), (8240.0, -18.0618)] {
            let i = frame
                .spectrum_hz
                .iter()
                .position(|f| (f - hz).abs() < 0.5)
                .expect("tone peak");
            assert!((frame.spectrum[i] - db).abs() < 0.3);
        }
        assert!(frame
            .spectrum_hz
            .iter()
            .zip(&frame.spectrum)
            .any(|(hz, db)| (8050.0..8190.0).contains(hz) && *db < -36.0));
    }

    #[test]
    fn traces_span_a_bass_cycle_at_common_output_rates() {
        for sample_rate in [44100.0, 48000.0, 96000.0] {
            let block = ScopeBlock {
                sample_rate,
                frames: (0..2048)
                    .map(|i| {
                        let v = (std::f32::consts::TAU * 55.0 * i as f32 / sample_rate).sin();
                        [v, 0.0]
                    })
                    .collect(),
            };
            let trace = MeterSlot::default().frame(&block, false);
            assert!(trace.left.iter().any(|v| *v > 0.98));
            assert!(trace.left.iter().any(|v| *v < -0.98));
        }
    }

    #[test]
    fn spectrum_is_lazy_and_silent_output_is_finite() {
        let mut slot = MeterSlot::default();
        let mut silence = stereo_tone(0.0);
        silence.frames.fill([0.0, 0.0]);
        silence.frames[0][0] = f32::NAN;
        silence.sample_rate = 22050.0;
        let traces = slot.frame(&silence, false);
        assert!(slot.spectrum.is_none() && traces.spectrum.is_empty());
        let all = slot.frame(&silence, true);
        assert_eq!(all.max_hz, 11025.0);
        assert!(all.spectrum.iter().all(|db| *db == DB_FLOOR));
        assert!(all.left.iter().all(|s| *s == 0.0));
    }

    #[test]
    #[ignore = "local timing measurement; run with --ignored --nocapture"]
    fn measure_meter_cost() {
        let block = tones(48000.0, &[(65.36, 0.25), (8240.0, 0.125)]);
        let mut slot = MeterSlot::default();
        const N: usize = 500;
        for spectrum in [false, true] {
            black_box(slot.frame(&block, spectrum));
            let mut times = Vec::with_capacity(N);
            for _ in 0..N {
                let started = Instant::now();
                let frame = slot.frame(black_box(&block), spectrum);
                black_box(serde_json::to_vec(&frame).unwrap());
                times.push(started.elapsed());
            }
            times.sort_unstable();
            eprintln!(
                "analysis + JSON, spectrum={spectrum}, n={N}, median={:?}, p95={:?}",
                times[N / 2],
                times[N * 95 / 100]
            );
        }
    }
}

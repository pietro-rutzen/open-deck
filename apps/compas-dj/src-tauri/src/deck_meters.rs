//! Pull-only deck meters. The callback supplies at most one requested block per deck.
//! Formatting and the optional windowed FFT run on Tauri's asynchronous command runtime.
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use compas_audio::{ScopeBlock, NUM_DECKS};
use rustfft::{num_complex::Complex32, Fft, FftPlanner};
use serde::Serialize;
use tauri::State;

use crate::EngineHandle;

const TRACE_POINTS: usize = 192;
const SPECTRUM_BANDS: usize = 72;
const DB_FLOOR: f32 = -72.0;
const MIN_POLL: Duration = Duration::from_millis(50);
const STALE_AFTER: Duration = Duration::from_millis(250);

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DeckMeterFrame {
    left: Vec<f32>,
    right: Vec<f32>,
    spectrum: Vec<f32>,
    min_hz: f32,
    max_hz: f32,
}

#[derive(Default)]
pub(crate) struct DeckMeters {
    slots: [Mutex<MeterSlot>; NUM_DECKS],
}

#[derive(Default)]
struct MeterSlot {
    last_request: Option<Instant>,
    spectrum: Option<Spectrum>,
}

struct Spectrum {
    fft: Arc<dyn Fft<f32>>,
    window: Vec<f32>,
    inverse_window_sum_squared: f32,
    buffer: Vec<Complex32>,
    scratch: Vec<Complex32>,
    band_bins: Vec<(usize, usize)>,
    sample_rate: f32,
}

impl Spectrum {
    fn new(n: usize, sample_rate: f32) -> Self {
        let fft = FftPlanner::new().plan_fft_forward(n);
        let window: Vec<_> = (0..n)
            .map(|i| 0.5 - 0.5 * (std::f32::consts::TAU * i as f32 / n as f32).cos())
            .collect();
        let inverse_window_sum_squared = window.iter().sum::<f32>().powi(-2);
        let scratch = vec![Complex32::default(); fft.get_inplace_scratch_len()];
        let (min_hz, max_hz) = frequency_range(sample_rate);
        let ratio = (max_hz / min_hz).powf(1.0 / (SPECTRUM_BANDS - 1) as f32);
        let bin_hz = sample_rate / n as f32;
        let band_bins = (0..SPECTRUM_BANDS)
            .map(|i| {
                let hz = min_hz * ratio.powi(i as i32);
                let lower = ((hz / ratio.sqrt() / bin_hz).ceil() as usize).clamp(1, n / 2);
                let upper = ((hz * ratio.sqrt() / bin_hz).floor() as usize).clamp(1, n / 2);
                if lower > upper {
                    let closest = ((hz / bin_hz).round() as usize).clamp(1, n / 2);
                    (closest, closest)
                } else {
                    (lower, upper)
                }
            })
            .collect();
        Self {
            fft,
            window,
            inverse_window_sum_squared,
            scratch,
            band_bins,
            sample_rate,
            buffer: vec![Complex32::default(); n],
        }
    }

    fn analyze(&mut self, samples: &[[f32; 2]]) -> Vec<f32> {
        for ((bin, frame), win) in self.buffer.iter_mut().zip(samples).zip(&self.window) {
            *bin = Complex32::new(finite(frame[0]) * win, finite(frame[1]) * win);
        }
        self.fft
            .process_with_scratch(&mut self.buffer, &mut self.scratch);
        let n = self.buffer.len();
        self.band_bins
            .iter()
            .map(|&(lower, upper)| {
                // One complex FFT carries both real channels. Summing the powers of
                // conjugate-index bins removes the cross term even for antiphase stereo.
                let mut peak_power = 0.0_f32;
                for k in lower..=upper {
                    let power = if k == n / 2 {
                        self.buffer[k].norm_sqr() * 0.5
                    } else {
                        self.buffer[k].norm_sqr() + self.buffer[n - k].norm_sqr()
                    };
                    peak_power = peak_power.max(power);
                }
                (10.0 * (peak_power * self.inverse_window_sum_squared).log10()).clamp(DB_FLOOR, 0.0)
            })
            .collect()
    }
}

fn frequency_range(rate: f32) -> (f32, f32) {
    (20.0, 20_000.0_f32.min(rate / 2.0))
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
        let spectrum = if include_spectrum {
            if self.spectrum.as_ref().is_none_or(|s| {
                s.sample_rate != block.sample_rate || s.buffer.len() != samples.len()
            }) {
                self.spectrum = Some(Spectrum::new(samples.len(), block.sample_rate));
            }
            self.spectrum.as_mut().unwrap().analyze(samples)
        } else {
            Vec::new()
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
            min_hz,
            max_hz,
        }
    }
}

#[tauri::command]
pub(crate) async fn read_deck_meters(
    engine: State<'_, EngineHandle>,
    meters: State<'_, DeckMeters>,
    deck: usize,
    spectrum: bool,
) -> Result<Option<DeckMeterFrame>, String> {
    let slot = meters.slots.get(deck).ok_or("Invalid meter deck")?;
    let Ok(mut slot) = slot.try_lock() else {
        return Ok(None);
    };
    let telemetry = &engine.telemetry;
    let now = Instant::now();
    let gap = slot
        .last_request
        .map(|previous| now.duration_since(previous));
    if gap.is_some_and(|g| g < MIN_POLL) {
        return Ok(None);
    }
    slot.last_request = Some(now);
    let block = telemetry.take_scope(deck);
    if !telemetry.is_loaded(deck) {
        return Ok(None);
    }
    telemetry.request_scope(deck);
    if gap.is_none_or(|g| g > STALE_AFTER) {
        return Ok(None);
    }
    Ok(block
        .filter(|b| b.sample_rate.is_finite() && b.sample_rate > 40.0)
        .map(|b| slot.frame(&b, spectrum)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::hint::black_box;

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
        let hz = stereo.min_hz
            * (stereo.max_hz / stereo.min_hz).powf(bin as f32 / (SPECTRUM_BANDS - 1) as f32);
        assert!((hz - 937.5).abs() < 55.0);
        assert!((db + 6.02).abs() < 0.1);
        assert!((db - inverted.spectrum[bin]).abs() < 0.01);
        assert_eq!(inverted.left.len(), TRACE_POINTS);
        assert!(inverted
            .left
            .iter()
            .zip(&inverted.right)
            .all(|(l, r)| (*l + *r).abs() < 1e-6));
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
        let block = stereo_tone(-1.0);
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

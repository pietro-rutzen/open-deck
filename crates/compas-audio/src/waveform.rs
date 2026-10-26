//! Waveform peak extraction for rendering. Offline (runs on a worker after decode).

/// Quiet stretches the deck should skip, as `[start, end)` source frames.
///
/// A window counts as silence when it is below 2% of the track peak. The floor is
/// about −48 dBFS, so a quiet recording is not swallowed. Only runs of at least a
/// quarter-second are returned, so gaps between hits stay. A file that is almost
/// entirely quiet is left unchanged.
pub fn silence_spans(samples: &[f32], source_rate: u32) -> Vec<(f64, f64)> {
    let frames = samples.len() / 2;
    if frames == 0 || source_rate == 0 {
        return Vec::new();
    }
    let window = (source_rate as usize / 100).max(1);
    let min_run = ((source_rate as f64) * 0.25).round().max(1.0) as usize;
    let mut track_peak = 0.0f32;
    let mut index = 0;
    while index < frames {
        let end = (index + window).min(frames);
        track_peak = track_peak.max(window_peak(samples, index, end));
        index = end;
    }
    let threshold = (track_peak * 0.02).max(0.004);
    let mut spans = Vec::new();
    let mut run_start = None;
    let mut silent_frames = 0usize;
    index = 0;
    while index < frames {
        let end = (index + window).min(frames);
        let silent = window_peak(samples, index, end) < threshold;
        if silent {
            if run_start.is_none() {
                run_start = Some(index);
            }
        } else if let Some(start) = run_start.take() {
            if index - start >= min_run {
                spans.push((start as f64, index as f64));
                silent_frames += index - start;
            }
        }
        index = end;
    }
    if let Some(start) = run_start {
        if frames - start >= min_run {
            spans.push((start as f64, frames as f64));
            silent_frames += frames - start;
        }
    }
    if frames.saturating_sub(silent_frames) < source_rate as usize / 2 {
        Vec::new()
    } else {
        spans
    }
}

fn window_peak(samples: &[f32], start: usize, end: usize) -> f32 {
    let mut peak = 0.0f32;
    for frame in start..end {
        peak = peak
            .max(samples[frame * 2].abs())
            .max(samples[frame * 2 + 1].abs());
    }
    peak
}

/// Compute one peak (max absolute amplitude across both channels) per bin of
/// `bin_frames` source frames. The result is a compact envelope the frontend renders
/// on a WebGL canvas; it is sent once per load rather than streaming the whole buffer.
///
/// `samples` is interleaved stereo. A 5-minute track at `bin_frames = 512` yields
/// ~25k peaks (~100 KB) — cheap to ship over IPC and to draw.
pub fn compute_peaks(samples: &[f32], bin_frames: usize) -> Vec<f32> {
    let bin_frames = bin_frames.max(1);
    let frames = samples.len() / 2;
    if frames == 0 {
        return Vec::new();
    }
    let n_bins = frames.div_ceil(bin_frames);
    let mut peaks = Vec::with_capacity(n_bins);

    let mut frame = 0;
    while frame < frames {
        let end = (frame + bin_frames).min(frames);
        let mut peak = 0.0f32;
        for f in frame..end {
            let l = samples[f * 2].abs();
            let r = samples[f * 2 + 1].abs();
            peak = peak.max(l).max(r);
        }
        peaks.push(peak);
        frame = end;
    }
    peaks
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_input_yields_no_peaks() {
        assert!(compute_peaks(&[], 512).is_empty());
    }

    #[test]
    fn trailing_silence_becomes_a_span() {
        let rate = 1_000;
        let mut samples = vec![0.5f32; 1_000 * 2];
        samples.extend(std::iter::repeat(0.0).take(500 * 2));
        let spans = silence_spans(&samples, rate);
        assert_eq!(spans.len(), 1, "{spans:?}");
        assert!(spans[0].0 >= 900.0 && spans[0].0 <= 1_100.0, "{spans:?}");
        assert_eq!(spans[0].1, 1_500.0);
    }

    #[test]
    fn short_gap_is_not_silence() {
        let rate = 1_000;
        let mut samples = vec![0.5f32; 1_000 * 2];
        samples.extend(std::iter::repeat(0.0).take(100 * 2));
        samples.extend(std::iter::repeat(0.5).take(1_000 * 2));
        assert!(silence_spans(&samples, rate).is_empty());
    }

    #[test]
    fn a_silent_file_is_not_skipped() {
        assert!(silence_spans(&vec![0.0f32; 2_000 * 2], 1_000).is_empty());
    }

    #[test]
    fn peak_is_max_abs_in_bin() {
        // 4 frames, bin of 2 -> 2 peaks. frame amps (max of l/r): 0.2, 0.9, 0.3, 0.5
        let s = [0.2, 0.1, -0.9, 0.4, 0.3, 0.0, 0.1, 0.5];
        let peaks = compute_peaks(&s, 2);
        assert_eq!(peaks.len(), 2);
        assert!((peaks[0] - 0.9).abs() < 1e-6);
        assert!((peaks[1] - 0.5).abs() < 1e-6);
    }
}

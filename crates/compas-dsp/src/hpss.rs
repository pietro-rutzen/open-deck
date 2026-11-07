//! Offline harmonic/percussive separation.
//!
//! This is a classical DSP experiment. It is not a product stem feature, it loads
//! no model, and nothing in the audio callback calls it. Playback keeps the
//! original mix.
//!
//! The split follows Fitzgerald's median-filter idea (DAFx 2010) with the soft
//! mask used by Driedger, Müller, and Disch (ISMIR 2014) and by librosa's
//! `hpss` defaults: magnitude median of 31 along time for the harmonic estimate,
//! the same length along frequency for the percussive estimate, then a power-2
//! mask. Parameters are those published defaults. They are not fitted to the
//! fixtures in the tests below.
//!
//! What the two streams are, and are not:
//!
//! * Harmonic means energy that is stable across time and narrow in frequency.
//!   A centered vocal is not detected. A low note is not a bass stem. A pitched
//!   drum body stays here because it is tonal.
//! * Percussive means energy that is brief and spread across frequency. A noise
//!   burst is percussive without being a drum. A tom's ring is not.
//!
//! Channels are processed independently. Stereo width is not a vocal cue.
//!
//! Masks at each bin sum to one, so the two streams add back to the short-time
//! transform of the input. That mixture consistency is required and is not a
//! separation score. Separation is judged against synthesized sources with the
//! energy ratios documented on [`scale_invariant_ratio`].

use std::mem::size_of;

use rustfft::num_complex::Complex32;
use rustfft::{Fft, FftPlanner};

/// Largest PCM length accepted per channel.
///
/// At 48 kHz this is 30 seconds. The cap keeps the spectrogram bounded together
/// with the hop rule in [`HpssParams`].
pub const MAX_SAMPLES_PER_CHANNEL: usize = 1_440_000;

/// Power applied to the median magnitudes before the mask ratio. Fixed at 2.
const MASK_POWER: u32 = 2;

/// Sum of squared samples. The scale-invariant ratio and the residual use this
/// as their energy.
pub const ABSOLUTE_ENERGY_FLOOR: f64 = 1e-20;

/// Smallest denominator, relative to the target energy, in [`scale_invariant_ratio`].
///
/// The reported decibel value therefore cannot exceed 80 dB. A result that hits
/// the floor has [`ScaleInvariantRatio::capped`] set.
pub const RELATIVE_ENERGY_FLOOR: f64 = 1e-8;

/// Published separation settings. `median_taps` is used on both axes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HpssParams {
    pub fft_len: usize,
    pub hop: usize,
    pub median_taps: usize,
}

impl Default for HpssParams {
    fn default() -> Self {
        Self {
            fft_len: 2048,
            hop: 512,
            median_taps: 31,
        }
    }
}

/// Harmonic and percussive PCM, one vector per input channel, same length as
/// the input. The input slice is not modified.
#[derive(Debug, Clone, PartialEq)]
pub struct HpssSplit {
    pub harmonic: Vec<Vec<f32>>,
    pub percussive: Vec<Vec<f32>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HpssError {
    NoChannels,
    TooManyChannels(usize),
    ChannelLengthMismatch {
        channel: usize,
        expected: usize,
        got: usize,
    },
    TooManySamples {
        samples: usize,
        limit: usize,
    },
    NonFinite {
        channel: usize,
        index: usize,
    },
    UnsupportedParams(&'static str),
    Cancelled,
}

impl std::fmt::Display for HpssError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoChannels => formatter.write_str("hpss needs at least one channel"),
            Self::TooManyChannels(count) => {
                write!(formatter, "hpss accepts one or two channels, got {count}")
            }
            Self::ChannelLengthMismatch {
                channel,
                expected,
                got,
            } => write!(
                formatter,
                "channel {channel} has {got} samples, expected {expected}"
            ),
            Self::TooManySamples { samples, limit } => {
                write!(formatter, "hpss input of {samples} samples exceeds {limit}")
            }
            Self::NonFinite { channel, index } => {
                write!(
                    formatter,
                    "non-finite sample at channel {channel} index {index}"
                )
            }
            Self::UnsupportedParams(reason) => formatter.write_str(reason),
            Self::Cancelled => formatter.write_str("hpss cancelled"),
        }
    }
}

impl std::error::Error for HpssError {}

#[derive(Debug, Clone, PartialEq)]
pub struct ScaleInvariantRatio {
    /// `10 * log10(target_energy / denominator_energy)`.
    pub decibels: f64,
    /// `||α reference||²`, with `α = dot(reference, estimate) / ||reference||²`.
    pub target_energy: f64,
    /// Error energy raised to [`RELATIVE_ENERGY_FLOOR`] times the target, and
    /// at least [`ABSOLUTE_ENERGY_FLOOR`].
    pub denominator_energy: f64,
    /// `true` when the denominator was raised to a floor, so the decibel value
    /// is a cap rather than an exact ratio.
    pub capped: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ComparedEnergy {
    Ratio(ScaleInvariantRatio),
    ReferenceSilent { estimate_energy: f64 },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MetricError {
    LengthMismatch,
    NonFinite,
    ReferenceSilent,
}

impl std::fmt::Display for MetricError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::LengthMismatch => formatter.write_str("energy metric vectors differ in length"),
            Self::NonFinite => formatter.write_str("energy metric received a non-finite sample"),
            Self::ReferenceSilent => {
                formatter.write_str("energy metric reference is at the absolute floor")
            }
        }
    }
}

impl std::error::Error for MetricError {}

/// Separate each channel into a harmonic stream and a percussive stream.
///
/// `cancel` is polled between channels and between FFT frames. Returning `true`
/// abandons the call with [`HpssError::Cancelled`] and no output.
pub fn separate_harmonic_percussive(
    channels: &[&[f32]],
    params: HpssParams,
    mut cancel: impl FnMut() -> bool,
) -> Result<HpssSplit, HpssError> {
    validate(channels, params)?;
    if cancel() {
        return Err(HpssError::Cancelled);
    }
    if channels.iter().all(|channel| channel.is_empty()) {
        let count = channels.len();
        return Ok(HpssSplit {
            harmonic: vec![Vec::new(); count],
            percussive: vec![Vec::new(); count],
        });
    }

    let mut transform = FftState::new(params.fft_len);
    let mut harmonic = Vec::with_capacity(channels.len());
    let mut percussive = Vec::with_capacity(channels.len());
    for channel in channels {
        if cancel() {
            return Err(HpssError::Cancelled);
        }
        let (harmonic_channel, percussive_channel) =
            separate_channel(channel, params, &mut transform, &mut cancel)?;
        harmonic.push(harmonic_channel);
        percussive.push(percussive_channel);
    }
    Ok(HpssSplit {
        harmonic,
        percussive,
    })
}

/// Bytes this call allocates for the scratch spectrogram of one channel plus the
/// retained harmonic and percussive outputs of every channel.
///
/// The figure includes the `rustfft` scratch length. It is not a resident-set
/// measurement and it does not include the caller's mix buffer.
pub fn planned_bytes(
    samples_per_channel: usize,
    channels: usize,
    params: HpssParams,
) -> Result<usize, HpssError> {
    if channels == 0 {
        return Err(HpssError::NoChannels);
    }
    if channels > 2 {
        return Err(HpssError::TooManyChannels(channels));
    }
    validate_params(params)?;
    if samples_per_channel > MAX_SAMPLES_PER_CHANNEL {
        return Err(HpssError::TooManySamples {
            samples: samples_per_channel,
            limit: MAX_SAMPLES_PER_CHANNEL,
        });
    }
    if samples_per_channel == 0 {
        return Ok(0);
    }
    let mut planner = FftPlanner::<f32>::new();
    let forward = planner.plan_fft_forward(params.fft_len);
    let inverse = planner.plan_fft_inverse(params.fft_len);
    let scratch = forward
        .get_inplace_scratch_len()
        .max(inverse.get_inplace_scratch_len());
    Ok(scratch_bytes(samples_per_channel, params, scratch)
        + retained_output_bytes(samples_per_channel, channels))
}

/// Scale-invariant comparison of `estimate` with `reference`.
///
/// `α` is the least-squares gain of `reference` onto `estimate`. The numerator
/// is the energy of `α * reference`. The denominator is the energy of
/// `estimate - α * reference`, replaced by
/// `max(RELATIVE_ENERGY_FLOOR * numerator, ABSOLUTE_ENERGY_FLOOR)` when the
/// error is smaller than that. Decibels are `10 * log10(numerator / denominator)`.
///
/// A silent estimate of a real reference scores at the low cap (−80 dB) with
/// `capped` set. A half-gain copy of the reference scores as a match: the ratio
/// is invariant to gain, so it is not by itself proof that a stream isolated
/// one instrument.
pub fn scale_invariant_ratio(
    reference: &[f32],
    estimate: &[f32],
) -> Result<ComparedEnergy, MetricError> {
    if reference.len() != estimate.len() {
        return Err(MetricError::LengthMismatch);
    }
    if reference.iter().any(|sample| !sample.is_finite())
        || estimate.iter().any(|sample| !sample.is_finite())
    {
        return Err(MetricError::NonFinite);
    }
    let reference_energy = energy(reference);
    if reference_energy <= ABSOLUTE_ENERGY_FLOOR {
        return Ok(ComparedEnergy::ReferenceSilent {
            estimate_energy: energy(estimate),
        });
    }
    let estimate_energy = energy(estimate);
    let dot = dot(reference, estimate);
    let gain = dot / reference_energy;
    let target_energy = (gain * gain * reference_energy).max(0.0);
    let error_energy = (estimate_energy - target_energy).max(0.0);
    let floor = (RELATIVE_ENERGY_FLOOR * target_energy).max(ABSOLUTE_ENERGY_FLOOR);
    let capped = error_energy < floor || target_energy <= ABSOLUTE_ENERGY_FLOOR;
    let denominator_energy = error_energy.max(floor);
    let decibels = if target_energy <= ABSOLUTE_ENERGY_FLOOR {
        -10.0 * RELATIVE_ENERGY_FLOOR.log10()
    } else {
        10.0 * (target_energy / denominator_energy).log10()
    };
    Ok(ComparedEnergy::Ratio(ScaleInvariantRatio {
        decibels,
        target_energy,
        denominator_energy,
        capped,
    }))
}

/// Fraction of `estimate`'s energy explained by the least-squares gain of
/// `reference`. One means `estimate` is a scaled copy of `reference`.
pub fn projected_energy_fraction(reference: &[f32], estimate: &[f32]) -> Result<f64, MetricError> {
    if reference.len() != estimate.len() {
        return Err(MetricError::LengthMismatch);
    }
    if reference.iter().any(|sample| !sample.is_finite())
        || estimate.iter().any(|sample| !sample.is_finite())
    {
        return Err(MetricError::NonFinite);
    }
    let reference_energy = energy(reference);
    if reference_energy <= ABSOLUTE_ENERGY_FLOOR {
        return Err(MetricError::ReferenceSilent);
    }
    let estimate_energy = energy(estimate).max(ABSOLUTE_ENERGY_FLOOR);
    let gain = dot(reference, estimate) / reference_energy;
    let explained = (gain * gain * reference_energy).max(0.0);
    Ok((explained / estimate_energy).clamp(0.0, 1.0))
}

/// `10 * log10(||left - right||² / max(||reference||², ABSOLUTE_ENERGY_FLOOR))`.
///
/// A perfect match sits on the −80 dB cap when the residual energy is below the
/// absolute floor. The reference is the denominator signal, usually the mix.
pub fn residual_decibels(reference: &[f32], other: &[f32]) -> Result<f64, MetricError> {
    if reference.len() != other.len() {
        return Err(MetricError::LengthMismatch);
    }
    let mut residual = 0.0;
    for (left, right) in reference.iter().zip(other.iter()) {
        if !left.is_finite() || !right.is_finite() {
            return Err(MetricError::NonFinite);
        }
        let difference = f64::from(*left) - f64::from(*right);
        residual += difference * difference;
    }
    let denominator = energy(reference).max(ABSOLUTE_ENERGY_FLOOR);
    let numerator = residual.max(ABSOLUTE_ENERGY_FLOOR);
    Ok(10.0 * (numerator / denominator).log10())
}

pub fn signal_energy(samples: &[f32]) -> f64 {
    energy(samples)
}

fn validate(channels: &[&[f32]], params: HpssParams) -> Result<(), HpssError> {
    validate_params(params)?;
    if channels.is_empty() {
        return Err(HpssError::NoChannels);
    }
    if channels.len() > 2 {
        return Err(HpssError::TooManyChannels(channels.len()));
    }
    let expected = channels[0].len();
    if expected > MAX_SAMPLES_PER_CHANNEL {
        return Err(HpssError::TooManySamples {
            samples: expected,
            limit: MAX_SAMPLES_PER_CHANNEL,
        });
    }
    for (channel, samples) in channels.iter().enumerate().skip(1) {
        if samples.len() != expected {
            return Err(HpssError::ChannelLengthMismatch {
                channel,
                expected,
                got: samples.len(),
            });
        }
    }
    for (channel, samples) in channels.iter().enumerate() {
        if let Some(index) = samples.iter().position(|sample| !sample.is_finite()) {
            return Err(HpssError::NonFinite { channel, index });
        }
    }
    Ok(())
}

fn validate_params(params: HpssParams) -> Result<(), HpssError> {
    if !params.fft_len.is_power_of_two() || !(16..=8192).contains(&params.fft_len) {
        return Err(HpssError::UnsupportedParams(
            "fft length must be a power of two from 16 to 8192",
        ));
    }
    let hop_in_range = params.hop > 0
        && params.hop <= params.fft_len / 2
        && params.hop * 4 >= params.fft_len
        && params.fft_len % params.hop == 0;
    if !hop_in_range {
        return Err(HpssError::UnsupportedParams(
            "hop must divide the fft length and sit between one quarter and one half of it",
        ));
    }
    if params.median_taps % 2 == 0 || !(1..=127).contains(&params.median_taps) {
        return Err(HpssError::UnsupportedParams(
            "median length must be an odd count from 1 to 127",
        ));
    }
    Ok(())
}

struct FftState {
    forward: std::sync::Arc<dyn Fft<f32>>,
    inverse: std::sync::Arc<dyn Fft<f32>>,
    scratch: Vec<Complex32>,
    window: Vec<f32>,
    time: Vec<Complex32>,
}

impl FftState {
    fn new(fft_len: usize) -> Self {
        let mut planner = FftPlanner::<f32>::new();
        let forward = planner.plan_fft_forward(fft_len);
        let inverse = planner.plan_fft_inverse(fft_len);
        let scratch_len = forward
            .get_inplace_scratch_len()
            .max(inverse.get_inplace_scratch_len());
        Self {
            forward,
            inverse,
            scratch: vec![Complex32::default(); scratch_len],
            window: hann(fft_len),
            time: vec![Complex32::default(); fft_len],
        }
    }
}

fn separate_channel(
    samples: &[f32],
    params: HpssParams,
    transform: &mut FftState,
    cancel: &mut impl FnMut() -> bool,
) -> Result<(Vec<f32>, Vec<f32>), HpssError> {
    let padded_len = samples.len() + params.fft_len * 2;
    let mut padded = vec![0.0; padded_len];
    padded[params.fft_len..params.fft_len + samples.len()].copy_from_slice(samples);
    let frames = frame_count(padded_len, params.fft_len, params.hop);
    let mut spectrum = vec![Complex32::default(); frames * params.fft_len];
    for frame in 0..frames {
        if cancel() {
            return Err(HpssError::Cancelled);
        }
        write_windowed_frame(
            &padded,
            frame * params.hop,
            &transform.window,
            &mut transform.time,
        );
        transform
            .forward
            .process_with_scratch(&mut transform.time, &mut transform.scratch);
        let nyquist = params.fft_len / 2;
        transform.time[0].im = 0.0;
        transform.time[nyquist].im = 0.0;
        spectrum[frame * params.fft_len..(frame + 1) * params.fft_len]
            .copy_from_slice(&transform.time);
    }

    let bins = params.fft_len / 2 + 1;
    let mut magnitude = vec![0.0; frames * bins];
    for frame in 0..frames {
        for bin in 0..bins {
            let value = spectrum[frame * params.fft_len + bin];
            magnitude[frame * bins + bin] = value.norm();
        }
    }
    let mut median_scratch = vec![0.0; params.median_taps];
    let harmonic_median = median_time(
        &magnitude,
        frames,
        bins,
        params.median_taps,
        &mut median_scratch,
    );
    let percussive_median = median_frequency(
        &magnitude,
        frames,
        bins,
        params.median_taps,
        &mut median_scratch,
    );
    let mut harmonic_mask = vec![0.0; frames * bins];
    let mut percussive_mask = vec![0.0; frames * bins];
    for (index, (harmonic_value, percussive_value)) in harmonic_median
        .iter()
        .zip(percussive_median.iter())
        .enumerate()
    {
        let harmonic_weight = harmonic_value.powi(MASK_POWER as i32);
        let percussive_weight = percussive_value.powi(MASK_POWER as i32);
        let weight = harmonic_weight + percussive_weight;
        let harmonic_share = if weight > 0.0 {
            harmonic_weight / weight
        } else {
            0.5
        };
        harmonic_mask[index] = harmonic_share;
        percussive_mask[index] = 1.0 - harmonic_share;
    }

    let harmonic = inverse_masked(
        &spectrum,
        &harmonic_mask,
        samples.len(),
        params,
        transform,
        cancel,
    )?;
    let percussive = inverse_masked(
        &spectrum,
        &percussive_mask,
        samples.len(),
        params,
        transform,
        cancel,
    )?;
    Ok((harmonic, percussive))
}

fn write_windowed_frame(padded: &[f32], start: usize, window: &[f32], time: &mut [Complex32]) {
    for (offset, (destination, coefficient)) in time.iter_mut().zip(window.iter()).enumerate() {
        let sample = padded.get(start + offset).copied().unwrap_or(0.0);
        *destination = Complex32::new(sample * coefficient, 0.0);
    }
}

fn inverse_masked(
    spectrum: &[Complex32],
    mask: &[f32],
    output_len: usize,
    params: HpssParams,
    transform: &mut FftState,
    cancel: &mut impl FnMut() -> bool,
) -> Result<Vec<f32>, HpssError> {
    let padded_len = output_len + params.fft_len * 2;
    let frames = frame_count(padded_len, params.fft_len, params.hop);
    let bins = params.fft_len / 2 + 1;
    let nyquist = params.fft_len / 2;
    let mut accum = vec![0.0; padded_len];
    let mut weight = vec![0.0; padded_len];
    for frame in 0..frames {
        if cancel() {
            return Err(HpssError::Cancelled);
        }
        transform
            .time
            .copy_from_slice(&spectrum[frame * params.fft_len..(frame + 1) * params.fft_len]);
        for bin in 0..bins {
            let share = mask[frame * bins + bin];
            transform.time[bin] *= share;
            if bin != 0 && bin != nyquist {
                transform.time[params.fft_len - bin] *= share;
            }
        }
        transform
            .inverse
            .process_with_scratch(&mut transform.time, &mut transform.scratch);
        let start = frame * params.hop;
        let scale = params.fft_len as f32;
        for (offset, coefficient) in transform.window.iter().enumerate() {
            if let Some(sum) = accum.get_mut(start + offset) {
                *sum += transform.time[offset].re / scale;
                weight[start + offset] += coefficient;
            }
        }
    }
    let pad = params.fft_len;
    let mut output = vec![0.0; output_len];
    for (index, sample) in output.iter_mut().enumerate() {
        let envelope = weight[pad + index];
        if envelope > 1.0e-6 {
            *sample = accum[pad + index] / envelope;
        }
    }
    Ok(output)
}

fn median_time(
    magnitude: &[f32],
    frames: usize,
    bins: usize,
    taps: usize,
    scratch: &mut [f32],
) -> Vec<f32> {
    let mut filtered = vec![0.0; frames * bins];
    let half = taps as isize / 2;
    for bin in 0..bins {
        for frame in 0..frames {
            for (tap, slot) in scratch.iter_mut().enumerate().take(taps) {
                let source =
                    (frame as isize + tap as isize - half).clamp(0, frames as isize - 1) as usize;
                *slot = magnitude[source * bins + bin];
            }
            scratch[..taps].sort_unstable_by(f32::total_cmp);
            filtered[frame * bins + bin] = scratch[taps / 2];
        }
    }
    filtered
}

fn median_frequency(
    magnitude: &[f32],
    frames: usize,
    bins: usize,
    taps: usize,
    scratch: &mut [f32],
) -> Vec<f32> {
    let mut filtered = vec![0.0; frames * bins];
    let half = taps as isize / 2;
    for frame in 0..frames {
        for bin in 0..bins {
            for (tap, slot) in scratch.iter_mut().enumerate().take(taps) {
                let source =
                    (bin as isize + tap as isize - half).clamp(0, bins as isize - 1) as usize;
                *slot = magnitude[frame * bins + source];
            }
            scratch[..taps].sort_unstable_by(f32::total_cmp);
            filtered[frame * bins + bin] = scratch[taps / 2];
        }
    }
    filtered
}

fn frame_count(len: usize, fft_len: usize, hop: usize) -> usize {
    if len <= fft_len {
        1
    } else {
        1 + (len - fft_len).div_ceil(hop)
    }
}

fn hann(len: usize) -> Vec<f32> {
    (0..len)
        .map(|index| {
            let phase = 2.0 * std::f32::consts::PI * index as f32 / len as f32;
            0.5 - 0.5 * phase.cos()
        })
        .collect()
}

fn scratch_bytes(samples: usize, params: HpssParams, scratch_len: usize) -> usize {
    let padded = samples + params.fft_len * 2;
    let frames = frame_count(padded, params.fft_len, params.hop);
    let bins = params.fft_len / 2 + 1;
    let sample_bytes = size_of::<f32>();
    let complex_bytes = size_of::<Complex32>();
    padded * sample_bytes
        + padded * sample_bytes
        + frames * params.fft_len * complex_bytes
        + frames * bins * sample_bytes * 3
        + params.fft_len * complex_bytes
        + scratch_len * complex_bytes
        + params.median_taps * sample_bytes
}

fn retained_output_bytes(samples: usize, channels: usize) -> usize {
    channels * samples * size_of::<f32>() * 2
}

fn energy(samples: &[f32]) -> f64 {
    samples
        .iter()
        .map(|sample| {
            let value = f64::from(*sample);
            value * value
        })
        .sum()
}

fn dot(left: &[f32], right: &[f32]) -> f64 {
    left.iter()
        .zip(right.iter())
        .map(|(left_sample, right_sample)| f64::from(*left_sample) * f64::from(*right_sample))
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: f32 = 44_100.0;
    const LEN: usize = 44_100;

    fn params() -> HpssParams {
        HpssParams::default()
    }

    fn sine(len: usize, hz: f32, amplitude: f32) -> Vec<f32> {
        (0..len)
            .map(|index| {
                let phase = 2.0 * std::f32::consts::PI * hz * index as f32 / RATE;
                amplitude * phase.sin()
            })
            .collect()
    }

    fn noise_bursts(len: usize, period: usize, burst: usize, seed: u64) -> Vec<f32> {
        let mut state = seed;
        let mut samples = vec![0.0; len];
        let mut start = 0;
        while start < len {
            for offset in 0..burst {
                if start + offset >= len {
                    break;
                }
                samples[start + offset] = noise_sample(&mut state);
            }
            start += period;
        }
        samples
    }

    fn noise_sample(state: &mut u64) -> f32 {
        let mut value = *state;
        value ^= value << 13;
        value ^= value >> 7;
        value ^= value << 17;
        *state = value;
        let unit = ((value >> 40) as u16) as f32 / 65_535.0;
        unit * 2.0 - 1.0
    }

    fn scale_to_energy(samples: &mut [f32], target: f64) {
        let current = energy(samples);
        let gain = (target / current).sqrt();
        for sample in samples {
            *sample *= gain as f32;
        }
    }

    fn add(parts: &[&[f32]]) -> Vec<f32> {
        let mut mixed = vec![0.0; parts[0].len()];
        for part in parts {
            for (destination, sample) in mixed.iter_mut().zip(part.iter()) {
                *destination += *sample;
            }
        }
        mixed
    }

    fn ringing(len: usize, hz: f32, period: usize, decay_samples: f32) -> Vec<f32> {
        let mut samples = vec![0.0; len];
        let mut hit = 0;
        while hit < len {
            for offset in 0..(len - hit) {
                let envelope = (-(offset as f32) / decay_samples).exp();
                if envelope < 1.0e-4 {
                    break;
                }
                let phase = 2.0 * std::f32::consts::PI * hz * offset as f32 / RATE;
                samples[hit + offset] += envelope * phase.sin();
            }
            hit += period;
        }
        samples
    }

    fn separate_default(channels: &[&[f32]]) -> HpssSplit {
        separate_harmonic_percussive(channels, params(), || false).expect("hpss")
    }

    fn ratio_db(reference: &[f32], estimate: &[f32]) -> f64 {
        match scale_invariant_ratio(reference, estimate).expect("ratio") {
            ComparedEnergy::Ratio(ratio) => ratio.decibels,
            ComparedEnergy::ReferenceSilent { .. } => panic!("reference was silent"),
        }
    }

    fn fraction(reference: &[f32], estimate: &[f32]) -> f64 {
        projected_energy_fraction(reference, estimate).expect("fraction")
    }

    fn same_bits(left: &[f32], right: &[f32]) -> bool {
        left.len() == right.len()
            && left
                .iter()
                .zip(right)
                .all(|(left_sample, right_sample)| left_sample.to_bits() == right_sample.to_bits())
    }

    fn sum_streams(split: &HpssSplit, channel: usize) -> Vec<f32> {
        split.harmonic[channel]
            .iter()
            .zip(split.percussive[channel].iter())
            .map(|(harmonic, percussive)| harmonic + percussive)
            .collect()
    }

    #[test]
    fn soft_masks_rebuild_a_chirp_near_the_reporting_floor() {
        let chirp: Vec<f32> = (0..LEN)
            .map(|index| {
                let time = index as f32 / RATE;
                let phase = 2.0 * std::f32::consts::PI * (80.0 * time + 1_000.0 * time * time);
                0.4 * phase.sin()
            })
            .collect();
        let split = separate_default(&[&chirp]);
        let rebuilt = sum_streams(&split, 0);
        let residual = residual_decibels(&chirp, &rebuilt).expect("residual");
        eprintln!("HPSS_CHIRP residual_db={residual:.2}");
        assert!(
            residual < -60.0,
            "mixture residual {residual:.2} dB is above -60 dB"
        );
    }

    #[test]
    fn tone_plus_bursts_beats_the_mixture_on_both_streams() {
        let tone = sine(LEN, 440.0, 0.2);
        let mut bursts = noise_bursts(LEN, 3_528, 180, 0xA5A5_1234_F00D);
        scale_to_energy(&mut bursts, energy(&tone));
        let mixture = add(&[&tone, &bursts]);
        let split = separate_default(&[&mixture]);
        let tone_gain = ratio_db(&tone, &split.harmonic[0]) - ratio_db(&tone, &mixture);
        let burst_gain = ratio_db(&bursts, &split.percussive[0]) - ratio_db(&bursts, &mixture);
        let burst_in_harmonic = fraction(&bursts, &split.harmonic[0]);
        let tone_in_percussive = fraction(&tone, &split.percussive[0]);
        eprintln!(
            "HPSS_FAVORABLE tone_gain_db={tone_gain:.2} burst_gain_db={burst_gain:.2} tone_db={:.2} burst_db={:.2} mixture_tone_db={:.2} mixture_burst_db={:.2} burst_in_harmonic={burst_in_harmonic:.3} tone_in_percussive={tone_in_percussive:.3}",
            ratio_db(&tone, &split.harmonic[0]),
            ratio_db(&bursts, &split.percussive[0]),
            ratio_db(&tone, &mixture),
            ratio_db(&bursts, &mixture),
        );
        assert!(tone_gain >= 6.0, "tone improvement {tone_gain:.2} dB");
        assert!(burst_gain >= 6.0, "burst improvement {burst_gain:.2} dB");
        assert!(
            burst_in_harmonic < 0.25,
            "burst leakage {burst_in_harmonic:.3}"
        );
        assert!(
            tone_in_percussive < 0.25,
            "tone leakage {tone_in_percussive:.3}"
        );
        let rebuilt = sum_streams(&split, 0);
        let residual = residual_decibels(&mixture, &rebuilt).expect("residual");
        eprintln!("HPSS_FAVORABLE_RESIDUAL residual_db={residual:.2}");
        assert!(residual < -60.0, "residual {residual:.2} dB");
    }

    #[test]
    fn a_low_sine_stays_harmonic_and_is_not_a_bass_split() {
        let bass_note = sine(LEN, 80.0, 0.25);
        let mut bursts = noise_bursts(LEN, 4_410, 120, 0x0B55);
        scale_to_energy(&mut bursts, energy(&bass_note));
        let mixture = add(&[&bass_note, &bursts]);
        let split = separate_default(&[&mixture]);
        let harmonic_db = ratio_db(&bass_note, &split.harmonic[0]);
        let percussive_db = ratio_db(&bass_note, &split.percussive[0]);
        eprintln!("HPSS_LOW_SINE harmonic_db={harmonic_db:.2} percussive_db={percussive_db:.2}");
        assert!(
            harmonic_db >= percussive_db + 6.0,
            "80 Hz landed {harmonic_db:.2} dB harmonic and {percussive_db:.2} dB percussive"
        );
    }

    #[test]
    fn centering_a_tone_does_not_change_the_split() {
        let tone = sine(LEN, 330.0, 0.2);
        let mut bursts = noise_bursts(LEN, 3_528, 160, 0xCE17);
        scale_to_energy(&mut bursts, energy(&tone));
        let mixture = add(&[&tone, &bursts]);
        let zeros = vec![0.0; LEN];
        let centered = separate_default(&[&mixture, &mixture]);
        let hard_left = separate_default(&[&mixture, &zeros]);
        let split_sides = separate_default(&[&bursts, &tone]);
        let centered_db = ratio_db(&tone, &centered.harmonic[0]);
        let hard_left_db = ratio_db(&tone, &hard_left.harmonic[0]);
        let right_tone_db = ratio_db(&tone, &split_sides.harmonic[1]);
        eprintln!(
        "HPSS_CENTER centered_db={centered_db:.2} hard_left_db={hard_left_db:.2} right_tone_db={right_tone_db:.2}"
    );
        assert!(same_bits(&centered.harmonic[0], &hard_left.harmonic[0]));
        assert!(centered_db >= 6.0, "centered tone {centered_db:.2} dB");
        assert!(right_tone_db >= 6.0, "one-sided tone {right_tone_db:.2} dB");
        assert!(fraction(&tone, &split_sides.harmonic[0]) < 0.05);
    }

    #[test]
    fn steady_noise_is_split_instead_of_isolated() {
        let mut noise = noise_bursts(LEN, LEN, LEN, 0x5015E);
        scale_to_energy(&mut noise, 1_000.0);
        let split = separate_default(&[&noise]);
        let harmonic_share = energy(&split.harmonic[0]) / energy(&noise);
        let percussive_share = energy(&split.percussive[0]) / energy(&noise);
        eprintln!(
            "HPSS_NOISE harmonic_share={harmonic_share:.3} percussive_share={percussive_share:.3}"
        );
        assert!(harmonic_share > 0.1 && percussive_share > 0.1);
        assert!(harmonic_share < 0.5 && percussive_share < 0.5);
        assert!((harmonic_share - percussive_share).abs() < 0.15);
        let rebuilt = sum_streams(&split, 0);
        assert!(residual_decibels(&noise, &rebuilt).expect("residual") < -60.0);
    }

    #[test]
    fn a_ringing_tone_stays_on_the_harmonic_side() {
        let mut ring = ringing(LEN, 140.0, 22_050, RATE * 0.18);
        scale_to_energy(&mut ring, 1_000.0);
        let steady = sine(LEN, 140.0, 0.2);
        let ring_split = separate_default(&[&ring]);
        let steady_split = separate_default(&[&steady]);
        let ring_harmonic = fraction(&ring, &ring_split.harmonic[0]);
        let ring_percussive = fraction(&ring, &ring_split.percussive[0]);
        let steady_percussive = fraction(&steady, &steady_split.percussive[0]);
        eprintln!(
            "HPSS_RING harmonic={ring_harmonic:.3} percussive={ring_percussive:.3} steady_percussive={steady_percussive:.3}"
        );
        assert!(
            ring_harmonic > 0.95,
            "pitched decay stayed harmonic at {ring_harmonic:.3}"
        );
        assert!(
            ring_percussive < 0.10,
            "pitched decay was not a transient at {ring_percussive:.3}"
        );
        assert!(ring_percussive > steady_percussive);
    }

    #[test]
    fn overlapping_pitched_and_transient_sources_do_not_become_four_stems() {
        let mut pad = add(&[
            &sine(LEN, 220.0, 0.15),
            &sine(LEN, 277.0, 0.12),
            &sine(LEN, 330.0, 0.1),
        ]);
        let mut hats = noise_bursts(LEN, 3_528, 140, 0x4A75);
        let mut tom = ringing(LEN, 110.0, 22_050, RATE * 0.16);
        let target = energy(&pad);
        scale_to_energy(&mut hats, target);
        scale_to_energy(&mut tom, target);
        scale_to_energy(&mut pad, target);
        let mixture = add(&[&pad, &hats, &tom]);
        let untouched = mixture.clone();
        let split = separate_harmonic_percussive(&[&mixture], params(), || false).expect("hpss");
        assert_eq!(mixture, untouched);
        let pad_harmonic = fraction(&pad, &split.harmonic[0]);
        let pad_percussive = fraction(&pad, &split.percussive[0]);
        let hat_harmonic = fraction(&hats, &split.harmonic[0]);
        let hat_percussive = fraction(&hats, &split.percussive[0]);
        let tom_harmonic = fraction(&tom, &split.harmonic[0]);
        let tom_percussive = fraction(&tom, &split.percussive[0]);
        eprintln!(
            "HPSS_OVERLAP pad_h={pad_harmonic:.3} pad_p={pad_percussive:.3} hat_h={hat_harmonic:.3} hat_p={hat_percussive:.3} tom_h={tom_harmonic:.3} tom_p={tom_percussive:.3}"
        );
        assert!(pad_harmonic > pad_percussive);
        assert!(
            hat_percussive > 0.90,
            "hats belonged to the transient stream at {hat_percussive:.3}"
        );
        assert!(tom_harmonic > tom_percussive);
        assert!(
            tom_percussive < 0.05,
            "the pitched ring was not a drum stream at {tom_percussive:.3}"
        );
        assert!(pad_harmonic < 0.75 && tom_harmonic < 0.75);
        assert!(pad_harmonic + tom_harmonic > 0.90);
    }

    #[test]
    fn mono_and_a_copied_stereo_pair_agree() {
        let tone = sine(LEN, 523.25, 0.18);
        let mut bursts = noise_bursts(LEN, 4_000, 150, 0x5107);
        scale_to_energy(&mut bursts, energy(&tone));
        let mixture = add(&[&tone, &bursts]);
        let mono = separate_default(&[&mixture]);
        let stereo = separate_default(&[&mixture, &mixture]);
        assert!(same_bits(&mono.harmonic[0], &stereo.harmonic[1]));
        assert!(ratio_db(&tone, &mono.harmonic[0]) >= 6.0);
    }

    #[test]
    fn silence_short_input_and_repeat_calls_stay_defined() {
        let silent = vec![0.0; 4_410];
        let silent_split = separate_default(&[&silent]);
        assert!(silent_split.harmonic[0]
            .iter()
            .chain(silent_split.percussive[0].iter())
            .all(|sample| sample.to_bits() == 0));

        let short = sine(300, 440.0, 0.3);
        let short_split = separate_default(&[&short]);
        assert_eq!(short_split.harmonic[0].len(), short.len());
        assert_eq!(short_split.percussive[0].len(), short.len());
        assert!(short_split.harmonic[0]
            .iter()
            .chain(short_split.percussive[0].iter())
            .all(|sample| sample.is_finite()));
        let rebuilt = sum_streams(&short_split, 0);
        let residual = residual_decibels(&short, &rebuilt).expect("residual");
        eprintln!("HPSS_SHORT residual_db={residual:.2}");
        assert!(residual < -40.0, "short residual {residual:.2} dB");

        let again = separate_default(&[&short]);
        assert!(same_bits(&short_split.harmonic[0], &again.harmonic[0]));
    }

    #[test]
    fn invalid_input_and_cancellation_fail_without_a_partial_split() {
        let tone = sine(2_048, 440.0, 0.2);
        assert_eq!(
            separate_harmonic_percussive(&[], params(), || false).unwrap_err(),
            HpssError::NoChannels
        );
        assert!(matches!(
            separate_harmonic_percussive(&[&tone, &tone, &tone], params(), || false).unwrap_err(),
            HpssError::TooManyChannels(3)
        ));
        let other = vec![0.0; 10];
        assert!(matches!(
            separate_harmonic_percussive(&[&tone, &other], params(), || false).unwrap_err(),
            HpssError::ChannelLengthMismatch { .. }
        ));
        let mut non_finite = tone.clone();
        non_finite[4] = f32::NAN;
        assert_eq!(
            separate_harmonic_percussive(&[&non_finite], params(), || false).unwrap_err(),
            HpssError::NonFinite {
                channel: 0,
                index: 4
            }
        );
        let mut too_long_params = params();
        too_long_params.hop = 100;
        assert!(matches!(
            separate_harmonic_percussive(&[&tone], too_long_params, || false).unwrap_err(),
            HpssError::UnsupportedParams(_)
        ));
        assert_eq!(
            separate_harmonic_percussive(&[&tone], params(), || true).unwrap_err(),
            HpssError::Cancelled
        );
        let huge = MAX_SAMPLES_PER_CHANNEL + 1;
        assert_eq!(
            planned_bytes(huge, 1, params()).unwrap_err(),
            HpssError::TooManySamples {
                samples: huge,
                limit: MAX_SAMPLES_PER_CHANNEL
            }
        );
    }

    #[test]
    fn a_two_second_stereo_call_stays_inside_the_byte_budget() {
        let samples = RATE as usize * 2;
        let budget = planned_bytes(samples, 2, params()).expect("budget");
        let mut left = sine(samples, 220.0, 0.15);
        let mut right = noise_bursts(samples, 4_000, 160, 0x71EE);
        scale_to_energy(&mut right, energy(&left));
        scale_to_energy(&mut left, energy(&right));
        let started = std::time::Instant::now();
        let split = separate_default(&[&left, &right]);
        let elapsed_ms = started.elapsed().as_secs_f64() * 1_000.0;
        let cap = planned_bytes(MAX_SAMPLES_PER_CHANNEL, 2, params()).expect("cap");
        eprintln!(
            "HPSS_TIME samples={samples} channels=2 elapsed_ms={elapsed_ms:.1} planned_bytes={budget} cap_bytes={cap}"
        );
        assert!(split.harmonic[0].iter().all(|sample| sample.is_finite()));
        assert!(budget < 64 * 1024 * 1024, "planned {budget} bytes");
        assert!(elapsed_ms < 10_000.0, "elapsed {elapsed_ms:.1} ms");
        assert!(cap < 256 * 1024 * 1024, "cap budget {cap} bytes");
    }
}

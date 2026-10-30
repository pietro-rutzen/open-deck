//! Bounded sampler decoding. Call only from a blocking worker, never the UI/audio callback.
use compas_core::{DeckBuffer, TrackMetadata};
use compas_sources::{AudioSource, LocalFileSource, PcmSource};
use std::{
    path::Path,
    sync::atomic::{AtomicBool, Ordering},
};

const MAX_BYTES: usize = 64 * 1024 * 1024;
const MAX_SECONDS: usize = 60;
static DECODER_BUSY: AtomicBool = AtomicBool::new(false);

pub struct DecodePermit;
impl Drop for DecodePermit {
    fn drop(&mut self) {
        DECODER_BUSY.store(false, Ordering::Release);
    }
}

/// Reject concurrent requests before they enter the blocking worker queue.
pub fn acquire() -> Result<DecodePermit, String> {
    DECODER_BUSY
        .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
        .map(|_| DecodePermit)
        .map_err(|_| "Another sample is loading; try again shortly".into())
}

pub fn validate_slot(slot: usize) -> Result<(), String> {
    if slot >= compas_audio::NUM_SAMPLER_PADS {
        return Err("Invalid sampler pad".into());
    }
    Ok(())
}

fn append(samples: &mut Vec<f32>, chunk: &[f32], limit: usize) -> Result<(), String> {
    if chunk.len() > limit.saturating_sub(samples.len()) {
        return Err(
            "Sample is too long or too large: use up to 60 seconds and 64 MiB of decoded audio"
                .into(),
        );
    }
    samples.extend_from_slice(chunk);
    Ok(())
}

pub fn decode(path: &Path, _permit: DecodePermit) -> Result<(DeckBuffer, TrackMetadata), String> {
    let file = std::fs::metadata(path).map_err(|e| e.to_string())?;
    if !file.is_file() || file.len() > MAX_BYTES as u64 {
        return Err("Choose an audio file smaller than 64 MiB".into());
    }
    let mut source = LocalFileSource::open(path).map_err(|e| e.to_string())?;
    let metadata = source.metadata().clone();
    let rate = source.sample_rate();
    if rate == 0 || rate > 192_000 {
        return Err("Sampler supports audio up to 192 kHz".into());
    }
    if metadata
        .duration_ms
        .is_some_and(|ms| ms > (MAX_SECONDS * 1000) as u64)
    {
        return Err("Sample is too long: use up to 60 seconds".into());
    }
    let limit = (rate as usize * MAX_SECONDS * 2).min(MAX_BYTES / size_of::<f32>());
    // Never reserve using untrusted duration metadata. Check the actual decoded length too.
    let mut samples = Vec::new();
    while let Some(chunk) = source.next_chunk().map_err(|e| e.to_string())? {
        append(&mut samples, &chunk, limit)?;
    }
    if samples.is_empty() {
        return Err("This file contains no playable audio".into());
    }
    Ok((DeckBuffer::new(samples, rate), metadata))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn validates_pad_before_loading() {
        assert!(validate_slot(0).is_ok());
        assert!(validate_slot(compas_audio::NUM_SAMPLER_PADS).is_err());
        assert!(validate_slot(usize::MAX).is_err());
    }
    #[test]
    fn over_budget_chunk_leaves_existing_audio_unchanged() {
        let mut samples = vec![0.25, -0.25];
        assert!(append(&mut samples, &[1.0, 1.0, 1.0], 4).is_err());
        assert_eq!(samples, vec![0.25, -0.25]);
        assert!(append(&mut samples, &[0.5, -0.5], 4).is_ok());
        assert!(append(&mut samples, &[0.0], 4).is_err());
    }
    #[test]
    fn concurrent_load_is_rejected_and_panic_releases_admission() {
        let panic_result = std::panic::catch_unwind(|| {
            let _permit = acquire().expect("first load");
            assert!(acquire().is_err());
            panic!("simulated decoder panic");
        });
        assert!(panic_result.is_err());
        assert!(
            acquire().is_ok(),
            "a failed worker must not lock out future loads"
        );
    }
}

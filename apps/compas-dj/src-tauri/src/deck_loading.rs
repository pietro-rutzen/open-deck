//! Bounded PCM accumulation and cooperative cancellation on the deck decoder worker.
//! This is not a limit on codec-internal allocations or total process memory.
use compas_core::{DeckBuffer, TrackMetadata};
use compas_sources::{AudioSource, LocalFileSource, PcmSource};
use std::path::Path;

const MAX_PCM_BYTES: usize = 512 * 1024 * 1024;
const LIMIT_MESSAGE: &str =
    "Track exceeds the 512 MiB decoded-audio limit; use a shorter track or a lower sample rate";

fn collect(
    mut next: impl FnMut() -> Result<Option<Vec<f32>>, String>,
    mut current: impl FnMut() -> bool,
    limit: usize,
) -> Result<Option<Vec<f32>>, String> {
    let mut samples = Vec::new();
    loop {
        if !current() {
            return Ok(None);
        }
        let chunk = next();
        // Superseded requests discard both results and errors quietly.
        if !current() {
            return Ok(None);
        }
        let Some(chunk) = chunk? else {
            break;
        };
        if chunk.len() % 2 != 0 {
            return Err("Invalid stereo audio data".into());
        }
        if chunk.len() > limit.saturating_sub(samples.len()) {
            return Err(LIMIT_MESSAGE.into());
        }
        let needed = samples.len() + chunk.len();
        if needed > samples.capacity() {
            let target = needed.max(samples.capacity().saturating_mul(2)).min(limit);
            samples
                .try_reserve_exact(target - samples.len())
                .map_err(|_| "Not enough memory to load this track".to_string())?;
        }
        samples.extend_from_slice(&chunk);
    }
    if samples.is_empty() {
        return Err("This file contains no playable audio".into());
    }
    Ok(Some(samples))
}

pub(crate) fn decode(
    path: &Path,
    mut current: impl FnMut() -> bool,
) -> Result<Option<(DeckBuffer, TrackMetadata)>, String> {
    if !current() {
        return Ok(None);
    }
    let mut source = LocalFileSource::open(path).map_err(|e| e.to_string())?;
    let rate = source.sample_rate();
    if rate == 0 || rate > 192_000 {
        return Err("Deck playback supports sample rates up to 192 kHz".into());
    }
    let metadata = source.metadata().clone();
    // Duration headers are not allocation instructions. Bound actual decoded stereo samples.
    let samples = collect(
        || source.next_chunk().map_err(|e| e.to_string()),
        current,
        MAX_PCM_BYTES / size_of::<f32>(),
    )?;
    Ok(samples.map(|samples| (DeckBuffer::new(samples, rate), metadata)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    #[test]
    fn real_wav_matches_existing_decoder_and_cancelled_missing_file_is_skipped() {
        assert!(decode(Path::new("/missing/cancelled-track.wav"), || false)
            .unwrap()
            .is_none());
        let path = std::env::temp_dir().join(format!(
            "opendeck-decode-{}-{}.wav",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let mut wav = Vec::new();
        wav.extend_from_slice(b"RIFF");
        wav.extend_from_slice(&44u32.to_le_bytes());
        wav.extend_from_slice(b"WAVEfmt ");
        wav.extend_from_slice(&16u32.to_le_bytes());
        wav.extend_from_slice(&1u16.to_le_bytes());
        wav.extend_from_slice(&2u16.to_le_bytes());
        wav.extend_from_slice(&48_000u32.to_le_bytes());
        wav.extend_from_slice(&192_000u32.to_le_bytes());
        wav.extend_from_slice(&4u16.to_le_bytes());
        wav.extend_from_slice(&16u16.to_le_bytes());
        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&8u32.to_le_bytes());
        for value in [8192i16, -8192, 16384, -16384] {
            wav.extend_from_slice(&value.to_le_bytes());
        }
        std::fs::write(&path, wav).unwrap();
        let bounded = decode(&path, || true).unwrap().unwrap();
        let original = compas_sources::decode_full(&path).unwrap();
        assert_eq!(bounded.0.samples, original.0.samples);
        assert_eq!(bounded.0.source_rate, 48_000);
        assert_eq!(bounded.0.frames(), 2);
        std::fs::write(&path, b"not an audio file").unwrap();
        assert!(decode(&path, || true).is_err());
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn exact_limit_preserves_samples_and_overflow_is_rejected() {
        let chunks = vec![vec![0.1, -0.1], vec![0.2, -0.2]];
        let mut input = chunks.clone().into_iter();
        assert_eq!(
            collect(|| Ok(input.next()), || true, 4).unwrap().unwrap(),
            chunks.concat()
        );
        let mut input = chunks.into_iter();
        assert!(collect(|| Ok(input.next()), || true, 2)
            .unwrap_err()
            .contains("512 MiB"));
    }
    #[test]
    fn cancellation_skips_next_chunk_and_discards_inflight_errors() {
        assert!(collect(|| panic!("cancelled job decoded"), || false, 4)
            .unwrap()
            .is_none());
        let active = Cell::new(true);
        assert!(collect(
            || {
                active.set(false);
                Err("stale decode error".into())
            },
            || active.get(),
            4
        )
        .unwrap()
        .is_none());
    }
    #[test]
    fn empty_and_malformed_streams_fail() {
        assert!(collect(|| Ok(None), || true, 4).is_err());
        assert!(collect(|| Ok(Some(vec![0.0])), || true, 4).is_err());
    }
    #[test]
    fn cancellation_stops_before_reading_remaining_packets() {
        let checks = Cell::new(0);
        let reads = Cell::new(0);
        let result = collect(
            || {
                reads.set(reads.get() + 1);
                Ok(Some(vec![0.2, -0.2]))
            },
            || {
                checks.set(checks.get() + 1);
                checks.get() <= 2
            },
            100,
        )
        .unwrap();
        assert!(result.is_none());
        assert_eq!(reads.get(), 1);
    }
}

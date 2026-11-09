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
    let declared = source.declared_frames();
    let max_samples = MAX_PCM_BYTES / size_of::<f32>();
    if let Some(frames) = declared {
        if frames.saturating_mul(2) > max_samples as u64 {
            return Err(LIMIT_MESSAGE.into());
        }
    }
    // Duration headers are not allocation instructions. Bound actual decoded stereo samples.
    let samples = collect(
        || source.next_chunk().map_err(|e| e.to_string()),
        current,
        max_samples,
    )?;
    let Some(samples) = samples else {
        return Ok(None);
    };
    if let Some(expected) = declared {
        let got = (samples.len() / 2) as u64;
        if got < expected {
            return Err("The file ended before its audio was complete".into());
        }
    }
    Ok(Some((DeckBuffer::new(samples, rate), metadata)))
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

    fn scratch(name: &str) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!(
            "opendeck-load-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|elapsed| elapsed.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    fn pcm_wav(rate: u32, channels: u16, interleaved: &[i16], declared_data_bytes: u32) -> Vec<u8> {
        let bits = 16u16;
        let block = (channels.max(1) as u32) * u32::from(bits / 8);
        let byte_rate = rate.saturating_mul(block);
        let mut wav = Vec::new();
        wav.extend_from_slice(b"RIFF");
        wav.extend_from_slice(&(36u32.saturating_add(declared_data_bytes)).to_le_bytes());
        wav.extend_from_slice(b"WAVEfmt ");
        wav.extend_from_slice(&16u32.to_le_bytes());
        wav.extend_from_slice(&1u16.to_le_bytes());
        wav.extend_from_slice(&channels.to_le_bytes());
        wav.extend_from_slice(&rate.to_le_bytes());
        wav.extend_from_slice(&byte_rate.to_le_bytes());
        wav.extend_from_slice(&(block as u16).to_le_bytes());
        wav.extend_from_slice(&bits.to_le_bytes());
        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&declared_data_bytes.to_le_bytes());
        for sample in interleaved {
            wav.extend_from_slice(&sample.to_le_bytes());
        }
        wav
    }

    fn write_case(dir: &Path, name: &str, bytes: &[u8]) -> std::path::PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, bytes).unwrap();
        path
    }

    #[test]
    fn bad_files_fail_without_replacing_a_loaded_track() {
        let dir = scratch("bad");
        let stereo = pcm_wav(48_000, 2, &[1000, -1000, 2000, -2000], 8);
        let previous = write_case(&dir, "previous.wav", &stereo);
        let loaded = decode(&previous, || true)
            .unwrap()
            .expect("playable fixture");
        let installed = vec![loaded.0.frames()];

        let header_only = {
            let mut bytes = stereo.clone();
            bytes.truncate(44);
            bytes
        };
        let truncated = pcm_wav(48_000, 2, &[1000, -1000], 8_000);
        let cases = [
            ("empty.wav", Vec::new()),
            ("zero.wav", pcm_wav(48_000, 2, &[], 0)),
            ("header.wav", header_only),
            ("truncated.wav", truncated),
            ("corrupt.wav", b"not audio".to_vec()),
            ("notes.wav", b"hello".to_vec()),
            ("random.bin", vec![0, 1, 2, 3, 4, 5, 6, 7]),
            ("fast.wav", pcm_wav(192_001, 2, &[1, -1], 4)),
            ("silent-rate.wav", pcm_wav(0, 2, &[1, -1], 4)),
            ("no-channels.wav", pcm_wav(48_000, 0, &[], 0)),
            (
                "claimed-long.wav",
                pcm_wav(48_000, 2, &[1, -1, 2, -2], 0x7fff_0000),
            ),
        ];
        for (name, bytes) in cases {
            let path = write_case(&dir, name, &bytes);
            let started = std::time::Instant::now();
            let result = decode(&path, || true);
            assert!(
                started.elapsed() < std::time::Duration::from_secs(2),
                "{name} took {:?}",
                started.elapsed()
            );
            let error = result.expect_err(name);
            assert!(!error.is_empty(), "{name}");
            let step = crate::deck_prepare::prepare_step(1, 512, || true, || Err(error.clone()));
            assert!(matches!(step, crate::deck_prepare::PrepareStep::Failed(_)));
            assert_eq!(installed, vec![2], "{name} replaced the previous track");
        }

        let renamed = dir.join("renamed-away.wav");
        std::fs::write(&renamed, &stereo).unwrap();
        std::fs::rename(&renamed, dir.join("renamed-here.wav")).unwrap();
        assert!(decode(&renamed, || true).is_err());

        let removed = write_case(&dir, "removed.wav", &stereo);
        std::fs::remove_file(&removed).unwrap();
        assert!(decode(&removed, || true).is_err());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn odd_layouts_and_a_mislabeled_wav_decode_or_fail_cleanly() {
        let dir = scratch("odd");
        let playable = [
            (
                "mono.wav",
                pcm_wav(48_000, 1, &[1000, -1000, 500], 6),
                3usize,
            ),
            ("three.wav", pcm_wav(44_100, 3, &[1, 2, 9, 3, 4, 9], 12), 2),
            ("slow.wav", pcm_wav(1, 2, &[1000, -1000, 500, -500], 8), 2),
            ("seven.wav", pcm_wav(7, 2, &[1000, -1000], 4), 1),
            ("cd.wav", pcm_wav(22_050, 2, &[1000, -1000], 4), 1),
            ("high.wav", pcm_wav(192_000, 2, &[1000, -1000], 4), 1),
        ];
        for (name, bytes, frames) in playable {
            let path = write_case(&dir, name, &bytes);
            let decoded = decode(&path, || true).unwrap().expect(name);
            assert_eq!(decoded.0.frames(), frames, "{name}");
            assert_eq!(decoded.0.samples.len() % 2, 0, "{name}");
            let step = crate::deck_prepare::prepare_step(
                3,
                512,
                || true,
                || {
                    Ok(Some(crate::deck_prepare::decoded_from(
                        decoded.0.clone(),
                        decoded.1.clone(),
                    )))
                },
            );
            assert!(
                matches!(step, crate::deck_prepare::PrepareStep::Ready(_)),
                "{name}"
            );
            let _ = crate::deck_prepare::measure_analysis(&decoded.0);
        }
        let mislabeled = write_case(
            &dir,
            "song.mp3",
            &pcm_wav(48_000, 2, &[1000, -1000, 2000, -2000], 8),
        );
        let sniffed = decode(&mislabeled, || true);
        if let Ok(Some((buffer, _))) = sniffed {
            assert_eq!(buffer.frames(), 2);
        }
        let checks = Cell::new(0);
        let long = write_case(
            &dir,
            "cancel.wav",
            &pcm_wav(48_000, 2, &[1, -1, 2, -2, 3, -3, 4, -4], 16),
        );
        assert!(decode(&long, || {
            let seen = checks.get();
            checks.set(seen + 1);
            seen < 2
        })
        .unwrap()
        .is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }
}

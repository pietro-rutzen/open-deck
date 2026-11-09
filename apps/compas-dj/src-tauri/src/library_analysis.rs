//! Library-only bounded analysis. Playback still decodes the complete track separately.
//! Preserve the exact native-rate first-90-second estimator input used by deck loading.
use std::path::Path;

use compas_dsp::analysis::{BeatGrid, KeyEstimate};
use compas_sources::{LocalFileSource, PcmSource};

const MAX_RATE: u32 = 192_000;

fn decode_window(path: &Path) -> Result<(Vec<f32>, u32), String> {
    let mut source = LocalFileSource::open(path).map_err(|e| e.to_string())?;
    let rate = source.sample_rate();
    if rate == 0 || rate > MAX_RATE || source.channels() > 8 {
        return Err("Library analysis supports up to 192 kHz and eight channels".into());
    }
    let limit = crate::ANALYSIS_MAX_SECS * rate as usize;
    let mut mono = Vec::with_capacity(limit);
    while mono.len() < limit {
        let Some(chunk) = source.next_chunk().map_err(|e| e.to_string())? else {
            break;
        };
        for stereo in chunk.chunks_exact(2).take(limit - mono.len()) {
            mono.push(0.5 * (stereo[0] + stereo[1]));
        }
    }
    if mono.is_empty() {
        return Err("Track contains no decoded audio".into());
    }
    Ok((mono, rate))
}

pub(crate) fn analyze_file(path: &Path) -> Result<(BeatGrid, KeyEstimate), String> {
    let (mono, rate) = decode_window(path)?;
    Ok((
        compas_dsp::analysis::estimate_beatgrid(&mono, rate),
        compas_dsp::analysis::estimate_key(&mono, rate),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Seek, SeekFrom, Write};

    // A generated stereo WAV longer than the window proves samples after 90 s are not read.
    #[test]
    fn bounded_decode_matches_old_analysis_input() {
        let path = std::env::temp_dir().join(format!("opendeck-window-{}.wav", std::process::id()));
        let rate = 8_000_u32;
        let frames = 92 * rate;
        let bytes = frames * 4;
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(b"RIFF").unwrap();
        f.write_all(&(bytes + 36).to_le_bytes()).unwrap();
        f.write_all(b"WAVEfmt ").unwrap();
        f.write_all(&16_u32.to_le_bytes()).unwrap();
        f.write_all(&1_u16.to_le_bytes()).unwrap();
        f.write_all(&2_u16.to_le_bytes()).unwrap();
        f.write_all(&rate.to_le_bytes()).unwrap();
        f.write_all(&(rate * 4).to_le_bytes()).unwrap();
        f.write_all(&4_u16.to_le_bytes()).unwrap();
        f.write_all(&16_u16.to_le_bytes()).unwrap();
        f.write_all(b"data").unwrap();
        f.write_all(&bytes.to_le_bytes()).unwrap();
        f.set_len(u64::from(bytes) + 44).unwrap();
        f.seek(SeekFrom::Start(44)).unwrap();
        f.write_all(&[0x00, 0x20, 0x00, 0x10]).unwrap();
        drop(f);
        let (mono, actual_rate) = decode_window(&path).unwrap();
        let (old, _) = compas_sources::decode_full(&path).unwrap();
        assert_eq!(actual_rate, rate);
        assert_eq!(mono.len(), 90 * rate as usize);
        assert!(mono.iter().copied().eq(old
            .samples
            .chunks_exact(2)
            .take(mono.len())
            .map(|s| 0.5 * (s[0] + s[1]))));
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn missing_track_is_an_error() {
        assert!(decode_window(Path::new("/nonexistent-opendeck-fixture.wav")).is_err());
    }

    #[test]
    fn empty_corrupt_and_odd_rate_files_do_not_panic() {
        let dir = std::env::temp_dir().join(format!(
            "opendeck-analysis-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|elapsed| elapsed.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let empty = dir.join("empty.wav");
        std::fs::write(&empty, b"").unwrap();
        assert!(analyze_file(&empty).is_err());
        let corrupt = dir.join("corrupt.flac");
        std::fs::write(&corrupt, b"fLaC not a stream").unwrap();
        assert!(analyze_file(&corrupt).is_err());
        let slow = dir.join("slow.wav");
        let mut wav = Vec::new();
        wav.extend_from_slice(b"RIFF");
        wav.extend_from_slice(&40u32.to_le_bytes());
        wav.extend_from_slice(b"WAVEfmt ");
        wav.extend_from_slice(&16u32.to_le_bytes());
        wav.extend_from_slice(&1u16.to_le_bytes());
        wav.extend_from_slice(&2u16.to_le_bytes());
        wav.extend_from_slice(&1u32.to_le_bytes());
        wav.extend_from_slice(&4u32.to_le_bytes());
        wav.extend_from_slice(&4u16.to_le_bytes());
        wav.extend_from_slice(&16u16.to_le_bytes());
        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&4u32.to_le_bytes());
        wav.extend_from_slice(&1_000i16.to_le_bytes());
        wav.extend_from_slice(&(-1_000i16).to_le_bytes());
        std::fs::write(&slow, wav).unwrap();
        let analyzed = analyze_file(&slow);
        assert!(analyzed.is_ok());
        let _ = std::fs::remove_dir_all(&dir);
    }
}

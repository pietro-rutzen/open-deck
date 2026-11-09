use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use compas_core::{CompasError, MusicProvider, Result, SourceCapabilities, TrackMetadata};
use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::{Decoder, DecoderOptions, CODEC_TYPE_NULL};
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::{FormatOptions, FormatReader};
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::{Limit, MetadataOptions, MetadataRevision, StandardTagKey};
use symphonia::core::probe::Hint;

use crate::{AudioSource, PcmSource};

/// A local DRM-free audio file decoded to PCM. Full DSP applies.
pub struct LocalFileSource {
    metadata: TrackMetadata,
    format: Box<dyn FormatReader>,
    decoder: Box<dyn Decoder>,
    track_id: u32,
    sample_rate: u32,
    channels: u16,
    declared_frames: Option<u64>,
    consecutive_packet_errors: u32,
    skipped_packets: u32,
}

impl LocalFileSource {
    /// Open and probe a file, preparing a decoder for its default audio track.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref().to_path_buf();
        if wav_sample_rate(&path) == Some(0) {
            return Err(CompasError::Decode("The file has no sample rate".into()));
        }
        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| open_probed(path))) {
            Ok(result) => result,
            Err(_) => Err(CompasError::Decode("The file could not be opened".into())),
        }
    }
}

fn wav_sample_rate(path: &Path) -> Option<u32> {
    let mut file = File::open(path).ok()?;
    let mut header = [0u8; 12];
    file.read_exact(&mut header).ok()?;
    if &header[0..4] != b"RIFF" || &header[8..12] != b"WAVE" {
        return None;
    }
    let file_len = file.metadata().ok()?.len();
    let mut offset = 12u64;
    while offset.saturating_add(8) <= file_len {
        file.seek(SeekFrom::Start(offset)).ok()?;
        let mut chunk = [0u8; 8];
        file.read_exact(&mut chunk).ok()?;
        let size = u32::from_le_bytes(chunk[4..8].try_into().ok()?) as u64;
        if &chunk[0..4] == b"fmt " && size >= 8 {
            let mut fmt = [0u8; 8];
            file.read_exact(&mut fmt).ok()?;
            return Some(u32::from_le_bytes(fmt[4..8].try_into().ok()?));
        }
        if size == 0 {
            break;
        }
        offset = offset.saturating_add(8).saturating_add(size + (size & 1));
    }
    None
}

fn open_probed(path: PathBuf) -> Result<LocalFileSource> {
    let file = File::open(&path)?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());

    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }

    let mut probed = symphonia::default::get_probe()
        .format(
            &hint,
            mss,
            &FormatOptions {
                enable_gapless: true,
                ..Default::default()
            },
            &MetadataOptions {
                limit_metadata_bytes: Limit::Maximum(64 * 1024),
                limit_visual_bytes: Limit::Maximum(2 * 1024 * 1024),
            },
        )
        .map_err(|e| CompasError::UnsupportedFormat(e.to_string()))?;

    let mut tags = TextTags::default();
    if let Some(metadata) = probed.metadata.get() {
        if let Some(revision) = metadata.current() {
            tags.absorb(revision);
        }
    }
    let mut format = probed.format;
    if let Some(revision) = format.metadata().current() {
        tags.absorb(revision);
    }
    let track = format
        .tracks()
        .iter()
        .find(|t| t.codec_params.codec != CODEC_TYPE_NULL)
        .ok_or_else(|| CompasError::Decode("no decodable audio track".into()))?;

    let track_id = track.id;
    let sample_rate = track
        .codec_params
        .sample_rate
        .ok_or_else(|| CompasError::Decode("unknown sample rate".into()))?;
    let channels = track
        .codec_params
        .channels
        .map(|c| c.count() as u16)
        .unwrap_or(2);

    let declared_frames = track.codec_params.n_frames.filter(|frames| *frames > 0);
    let duration_ms = match (declared_frames, sample_rate) {
        (Some(frames), sr) if sr > 0 => Some(frames * 1000 / sr as u64),
        _ => None,
    };

    let decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())
        .map_err(|e| CompasError::Decode(e.to_string()))?;

    let fallback_title = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("Unknown")
        .to_string();

    let metadata = TrackMetadata {
        id: fallback_title.clone(), // Preserve the existing source identity.
        // TODO(P1): assign a real UUID at import time.
        provider: MusicProvider::Local,
        provider_id: path.to_string_lossy().into_owned(),
        title: tags.title.unwrap_or(fallback_title),
        artist: tags
            .artist
            .or(tags.album_artist)
            .unwrap_or_else(|| "Unknown".to_string()),
        album: tags.album,
        artwork_url: None,
        duration_ms,
        bpm: None,
        musical_key: None,
    };

    Ok(LocalFileSource {
        metadata,
        format,
        decoder,
        track_id,
        sample_rate,
        channels,
        declared_frames,
        consecutive_packet_errors: 0,
        skipped_packets: 0,
    })
}

impl LocalFileSource {
    /// Frames the container header claimed, when it made a claim.
    pub fn declared_frames(&self) -> Option<u64> {
        self.declared_frames
    }

    fn note_bad_packet(&mut self) -> Result<()> {
        self.consecutive_packet_errors = self.consecutive_packet_errors.saturating_add(1);
        self.skipped_packets = self.skipped_packets.saturating_add(1);
        if self.consecutive_packet_errors > 32 || self.skipped_packets > 4_096 {
            return Err(CompasError::Decode(
                "The file has too many undecodable audio packets".into(),
            ));
        }
        Ok(())
    }
}

impl AudioSource for LocalFileSource {
    fn metadata(&self) -> &TrackMetadata {
        &self.metadata
    }

    fn capabilities(&self) -> SourceCapabilities {
        SourceCapabilities::FULL_DSP
    }
}

impl PcmSource for LocalFileSource {
    fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    fn channels(&self) -> u16 {
        self.channels
    }

    fn next_chunk(&mut self) -> Result<Option<Vec<f32>>> {
        loop {
            let packet = match self.format.next_packet() {
                Ok(p) => p,
                // Clean EOF.
                Err(SymphoniaError::IoError(e))
                    if e.kind() == std::io::ErrorKind::UnexpectedEof =>
                {
                    return Ok(None)
                }
                // Stream told us to reset; for the scaffold we treat it as end-of-stream.
                Err(SymphoniaError::ResetRequired) => return Ok(None),
                Err(e) => return Err(CompasError::Decode(e.to_string())),
            };

            if packet.track_id() != self.track_id {
                continue;
            }

            match self.decoder.decode(&packet) {
                Ok(decoded) => {
                    let spec = *decoded.spec();
                    let mut buf = SampleBuffer::<f32>::new(decoded.capacity() as u64, spec);
                    buf.copy_interleaved_ref(decoded);
                    let src_channels = spec.channels.count();
                    let interleaved = to_stereo(buf.samples(), src_channels);
                    if interleaved.is_empty() {
                        self.note_bad_packet()?;
                        continue;
                    }
                    self.consecutive_packet_errors = 0;
                    return Ok(Some(interleaved));
                }
                // Recoverable decode errors: skip this packet, keep going, until the file
                // is clearly not making progress.
                Err(SymphoniaError::DecodeError(_)) => {
                    self.note_bad_packet()?;
                    continue;
                }
                Err(e) => return Err(CompasError::Decode(e.to_string())),
            }
        }
    }
}

/// Text-only metadata, resolved on the file worker before decoding/playback.
#[derive(Default)]
struct TextTags {
    title: Option<String>,
    artist: Option<String>,
    album_artist: Option<String>,
    album: Option<String>,
}
impl TextTags {
    fn absorb(&mut self, revision: &MetadataRevision) {
        for tag in revision.tags() {
            let target = match tag.std_key {
                Some(StandardTagKey::TrackTitle) => &mut self.title,
                Some(StandardTagKey::Artist) => &mut self.artist,
                Some(StandardTagKey::AlbumArtist) => &mut self.album_artist,
                Some(StandardTagKey::Album) => &mut self.album,
                _ => continue,
            };
            if target.is_none() {
                let text: String = tag
                    .value
                    .to_string()
                    .chars()
                    .filter(|c| !c.is_control())
                    .take(1024)
                    .collect();
                let text = text.trim();
                if !text.is_empty() {
                    *target = Some(text.to_owned());
                }
            }
        }
    }
}

/// Convert an interleaved buffer with `src_channels` channels to interleaved stereo.
fn to_stereo(samples: &[f32], src_channels: usize) -> Vec<f32> {
    match src_channels {
        0 => Vec::new(),
        1 => {
            let mut out = Vec::with_capacity(samples.len() * 2);
            for &s in samples {
                out.push(s);
                out.push(s);
            }
            out
        }
        2 => samples.to_vec(),
        n => {
            // Downmix: take the first two channels of each frame.
            let frames = samples.len() / n;
            let mut out = Vec::with_capacity(frames * 2);
            for f in 0..frames {
                out.push(samples[f * n]);
                out.push(samples[f * n + 1]);
            }
            out
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tagged_flac(tags: &[&str]) -> PathBuf {
        static ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let id = ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let path =
            std::env::temp_dir().join(format!("opendeck-tags-{}-{id}.flac", std::process::id()));
        let mut bytes = b"fLaC".to_vec();
        bytes.extend_from_slice(&[0, 0, 0, 34]);
        let mut stream = [0u8; 34];
        stream[..4].copy_from_slice(&[0, 16, 0, 16]);
        let info = (48_000u64 << 44) | (1 << 41) | (15 << 36) | 16;
        stream[10..18].copy_from_slice(&info.to_be_bytes());
        bytes.extend_from_slice(&stream);
        let mut comments = vec![0u8; 4]; // empty vendor
        comments.extend_from_slice(&(tags.len() as u32).to_le_bytes());
        for tag in tags {
            comments.extend_from_slice(&(tag.len() as u32).to_le_bytes());
            comments.extend_from_slice(tag.as_bytes());
        }
        let size = comments.len() as u32;
        bytes.push(0x84); // final metadata block, Vorbis comments
        bytes.extend_from_slice(&size.to_be_bytes()[1..]);
        bytes.extend_from_slice(&comments);
        // A valid16-sample stereo constant frame, including header/frame CRCs.
        let mut frame = vec![0xff, 0xf8, 0x6a, 0x18, 0, 15];
        let mut crc8 = 0u8;
        for byte in &frame {
            crc8 ^= byte;
            for _ in 0..8 {
                crc8 = if crc8 & 0x80 != 0 {
                    (crc8 << 1) ^ 0x07
                } else {
                    crc8 << 1
                };
            }
        }
        frame.push(crc8);
        frame.extend_from_slice(&[0, 0, 0, 0, 0, 0]);
        let mut crc16 = 0u16;
        for byte in &frame {
            crc16 ^= u16::from(*byte) << 8;
            for _ in 0..8 {
                crc16 = if crc16 & 0x8000 != 0 {
                    (crc16 << 1) ^ 0x8005
                } else {
                    crc16 << 1
                };
            }
        }
        frame.extend_from_slice(&crc16.to_be_bytes());
        bytes.extend_from_slice(&frame);
        std::fs::write(&path, bytes).unwrap();
        path
    }

    #[test]
    fn local_probe_reads_embedded_flac_tags_and_preserves_identity() {
        let path = tagged_flac(&[
            "TITLE=  Mon Amour  ",
            "ARTIST=Jessie Ware",
            "ALBUM=Superbloom",
        ]);
        let mut source = LocalFileSource::open(&path).unwrap();
        let m = source.metadata();
        assert_eq!(m.title, "Mon Amour");
        assert_eq!(m.artist, "Jessie Ware");
        assert_eq!(m.album.as_deref(), Some("Superbloom"));
        assert_eq!(m.id, path.file_stem().unwrap().to_str().unwrap());
        assert_eq!(m.provider_id, path.to_string_lossy());
        let samples = source.next_chunk().unwrap().unwrap();
        assert_eq!(samples.len(), 32);
        assert!(samples.iter().all(|sample| *sample == 0.0));
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn local_probe_falls_back_only_for_missing_tags() {
        let path = tagged_flac(&["TITLE=   ", "ARTIST=", "ALBUMARTIST=Album Artist"]);
        let source = LocalFileSource::open(&path).unwrap();
        assert_eq!(
            source.metadata().title,
            path.file_stem().unwrap().to_str().unwrap()
        );
        assert_eq!(source.metadata().artist, "Album Artist");
        std::fs::remove_file(path).unwrap();
        let path = tagged_flac(&[]);
        let source = LocalFileSource::open(&path).unwrap();
        assert_eq!(source.metadata().artist, "Unknown");
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn to_stereo_duplicates_mono() {
        let mono = [0.1, 0.2, 0.3];
        let st = to_stereo(&mono, 1);
        assert_eq!(st, vec![0.1, 0.1, 0.2, 0.2, 0.3, 0.3]);
    }

    #[test]
    fn to_stereo_passes_stereo() {
        let s = [0.1, 0.2, 0.3, 0.4];
        assert_eq!(to_stereo(&s, 2), s.to_vec());
    }

    #[test]
    fn to_stereo_downmixes_surround() {
        // 4-channel frame -> keep first two.
        let s = [1.0, 2.0, 9.0, 9.0, 3.0, 4.0, 9.0, 9.0];
        assert_eq!(to_stereo(&s, 4), vec![1.0, 2.0, 3.0, 4.0]);
    }

    #[test]
    fn open_missing_file_errors() {
        assert!(LocalFileSource::open("does-not-exist.wav").is_err());
    }

    #[test]
    fn deleting_or_replacing_an_open_file_does_not_panic() {
        let path = tagged_flac(&["TITLE=Stay"]);
        let mut source = LocalFileSource::open(&path).unwrap();
        let replacement = tagged_flac(&["TITLE=Next"]);
        std::fs::rename(&replacement, &path).unwrap();
        let first = source.next_chunk().unwrap().expect("already open file");
        assert_eq!(first.len(), 32);
        std::fs::remove_file(&path).unwrap();
        let rest = source.next_chunk();
        assert!(rest.is_ok());
    }

    #[test]
    fn a_broken_frame_stops_instead_of_spinning() {
        let path = tagged_flac(&[]);
        let mut bytes = std::fs::read(&path).unwrap();
        let last = bytes.len() - 1;
        bytes[last] ^= 0xff;
        std::fs::write(&path, &bytes).unwrap();
        let opened = LocalFileSource::open(&path);
        if let Ok(mut source) = opened {
            let mut steps = 0;
            loop {
                steps += 1;
                assert!(steps < 8, "corrupt frame kept producing packets");
                match source.next_chunk() {
                    Ok(None) | Err(_) => break,
                    Ok(Some(samples)) => assert!(samples.iter().all(|sample| sample.is_finite())),
                }
            }
        }
        std::fs::remove_file(path).unwrap();
    }
}

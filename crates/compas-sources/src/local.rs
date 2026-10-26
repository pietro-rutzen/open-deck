use std::fs::File;
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
}

impl LocalFileSource {
    /// Open and probe a file, preparing a decoder for its default audio track.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path: PathBuf = path.as_ref().to_path_buf();
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

        let duration_ms = match (track.codec_params.n_frames, sample_rate) {
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
        })
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
                    return Ok(Some(interleaved));
                }
                // Recoverable decode errors: skip this packet, keep going.
                Err(SymphoniaError::DecodeError(msg)) => {
                    tracing::warn!("recoverable decode error: {msg}");
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
}

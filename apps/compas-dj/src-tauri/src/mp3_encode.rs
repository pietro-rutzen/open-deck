//! LAME 320 kbps CBR mirror encoder. Runs on the MP3 worker, never on the audio callback.
//! The shared library is loaded with dlopen so a recipient can replace it (LGPL).

use std::fs::File;
use std::io::{Cursor, Write};
use std::path::Path;
use std::sync::OnceLock;

use id3::TagLike;
use rubato::{
    Resampler, SincFixedIn, SincInterpolationParameters, SincInterpolationType, WindowFunction,
};
use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::{Decoder, DecoderOptions, CODEC_TYPE_NULL};
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::{FormatOptions, FormatReader};
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::{MetadataOptions, MetadataRevision, StandardTagKey, Value};
use symphonia::core::probe::Hint;

const TARGET_RATE: u32 = 44_100;
const LAME_BITRATE_KBPS: i32 = 320;
const LAME_JOINT_STEREO: i32 = 1;
const LAME_QUALITY_BEST: i32 = 0;
const RESAMPLE_CHUNK: usize = 1024;
const ARTWORK_MAX_EDGE: u32 = 800;

#[derive(Clone, Default)]
pub(crate) struct MirrorTags {
    pub(crate) title: Option<String>,
    pub(crate) artist: Option<String>,
    pub(crate) album: Option<String>,
    pub(crate) genre: Option<String>,
    pub(crate) comment: Option<String>,
    pub(crate) bpm: Option<String>,
    pub(crate) musical_key: Option<String>,
    pub(crate) artwork_jpeg: Option<Vec<u8>>,
}

struct Dither {
    state: u32,
}

impl Dither {
    fn new() -> Self {
        Self { state: 0xA5A5_1234 }
    }

    fn unit(&mut self) -> f32 {
        self.state = self
            .state
            .wrapping_mul(1_664_525)
            .wrapping_add(1_013_904_223);
        (self.state >> 8) as f32 / 16_777_216.0
    }

    /// Two uniform draws, so the noise is triangular and one 16-bit LSB wide.
    fn triangular(&mut self) -> f32 {
        self.unit() - self.unit()
    }
}

fn quantize(sample: f32, dither: bool, rng: &mut Dither) -> i16 {
    let scaled = sample.clamp(-1.0, 1.0) * 32_767.0;
    let value = if dither {
        scaled + rng.triangular()
    } else {
        scaled
    };
    value.round().clamp(-32_768.0, 32_767.0) as i16
}

fn quantize_interleaved(left: &[f32], right: &[f32], dither: bool, rng: &mut Dither) -> Vec<i16> {
    let mut pcm = Vec::with_capacity(left.len().saturating_mul(2));
    for (left_sample, right_sample) in left.iter().zip(right.iter()) {
        pcm.push(quantize(*left_sample, dither, rng));
        pcm.push(quantize(*right_sample, dither, rng));
    }
    pcm
}

pub(crate) struct Mp3FrameFacts {
    pub(crate) frames: usize,
    pub(crate) bitrate_kbps: u32,
    pub(crate) sample_rate: u32,
    pub(crate) cbr: bool,
    pub(crate) joint_stereo: bool,
}

pub(crate) fn mp3_frame_facts(bytes: &[u8]) -> Result<Mp3FrameFacts, String> {
    let mut offset = id3v2_audio_offset(bytes);
    let mut frames = 0usize;
    let mut bitrate_kbps = 0u32;
    let mut sample_rate = 0u32;
    let mut cbr = true;
    let mut joint_stereo = true;
    while offset + 4 <= bytes.len() {
        let synced = bytes[offset] == 0xFF && bytes[offset + 1] & 0xE0 == 0xE0;
        if !synced {
            if frames > 0 {
                break;
            }
            offset += 1;
            continue;
        }
        let header = match mpeg1_layer3_header(&bytes[offset..]) {
            Ok(header) => header,
            Err(_) if frames == 0 => {
                offset += 1;
                continue;
            }
            Err(error) => return Err(error),
        };
        if offset + header.length > bytes.len() {
            break;
        }
        if frames == 0 {
            bitrate_kbps = header.bitrate_kbps;
            sample_rate = header.sample_rate;
        } else if header.bitrate_kbps != bitrate_kbps || header.sample_rate != sample_rate {
            cbr = false;
        }
        if header.mode != LAME_JOINT_STEREO as u8 {
            joint_stereo = false;
        }
        offset += header.length;
        frames += 1;
    }
    if frames == 0 {
        return Err("the MP3 has no MPEG frames".into());
    }
    Ok(Mp3FrameFacts {
        frames,
        bitrate_kbps,
        sample_rate,
        cbr,
        joint_stereo,
    })
}

struct MpegHeader {
    bitrate_kbps: u32,
    sample_rate: u32,
    length: usize,
    mode: u8,
}

fn id3v2_audio_offset(bytes: &[u8]) -> usize {
    if bytes.len() < 10 || &bytes[0..3] != b"ID3" {
        return 0;
    }
    let size = ((bytes[6] as usize & 0x7F) << 21)
        | ((bytes[7] as usize & 0x7F) << 14)
        | ((bytes[8] as usize & 0x7F) << 7)
        | (bytes[9] as usize & 0x7F);
    let offset = 10 + size;
    if offset <= bytes.len() {
        offset
    } else {
        0
    }
}

fn mpeg1_layer3_header(bytes: &[u8]) -> Result<MpegHeader, String> {
    let version = (bytes[1] >> 3) & 0x03;
    let layer = (bytes[1] >> 1) & 0x03;
    if version != 0b11 || layer != 0b01 {
        return Err("the frame is not MPEG-1 Layer III".into());
    }
    let bitrate_index = ((bytes[2] >> 4) & 0x0F) as usize;
    let rate_index = ((bytes[2] >> 2) & 0x03) as usize;
    let padding = ((bytes[2] >> 1) & 0x01) as usize;
    let mode = bytes[3] >> 6;
    const BITRATE_KBPS: [u32; 16] = [
        0, 32, 40, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320, 0,
    ];
    const SAMPLE_RATE: [u32; 4] = [44_100, 48_000, 32_000, 0];
    let bitrate_kbps = BITRATE_KBPS[bitrate_index];
    let sample_rate = SAMPLE_RATE[rate_index];
    if bitrate_kbps == 0 || sample_rate == 0 {
        return Err("the MPEG frame has no bitrate".into());
    }
    let length = (144 * bitrate_kbps as usize * 1000) / sample_rate as usize + padding;
    if length < 4 {
        return Err("the MPEG frame is too small".into());
    }
    Ok(MpegHeader {
        bitrate_kbps,
        sample_rate,
        length,
        mode,
    })
}

type LameInit = unsafe extern "C" fn() -> *mut std::ffi::c_void;
type LameClose = unsafe extern "C" fn(*mut std::ffi::c_void) -> i32;
type LameSet = unsafe extern "C" fn(*mut std::ffi::c_void, i32) -> i32;
type LameInitParams = unsafe extern "C" fn(*mut std::ffi::c_void) -> i32;
type LameEncode = unsafe extern "C" fn(*mut std::ffi::c_void, *mut i16, i32, *mut u8, i32) -> i32;
type LameFlush = unsafe extern "C" fn(*mut std::ffi::c_void, *mut u8, i32) -> i32;

struct LameLibrary {
    _library: libloading::Library,
    init: LameInit,
    close: LameClose,
    set_in_samplerate: LameSet,
    set_out_samplerate: LameSet,
    set_num_channels: LameSet,
    set_brate: LameSet,
    set_mode: LameSet,
    set_quality: LameSet,
    set_vbr: LameSet,
    set_write_vbr_tag: LameSet,
    init_params: LameInitParams,
    encode_interleaved: LameEncode,
    encode_flush: LameFlush,
}

static LAME: OnceLock<LameLibrary> = OnceLock::new();

fn lame() -> Result<&'static LameLibrary, String> {
    if let Some(library) = LAME.get() {
        return Ok(library);
    }
    let loaded = LameLibrary::load()?;
    Ok(LAME.get_or_init(|| loaded))
}

impl LameLibrary {
    fn load() -> Result<Self, String> {
        let path = lame_dylib().ok_or_else(|| {
            "LAME shared library was not found. Run scripts/build-lame-dylib.sh".to_string()
        })?;
        // SAFETY: `path` is a shared libmp3lame this project built, or a library the
        // process substituted through OPENDECK_LAME_DYLIB. Loading it does not run on
        // the audio callback. Each symbol below matches include/lame.h from LAME 3.100.
        let library = unsafe { libloading::Library::new(&path) }
            .map_err(|error| format!("could not load {}: {error}", path.display()))?;
        Ok(Self {
            init: load_symbol(&library, b"lame_init\0")?,
            close: load_symbol(&library, b"lame_close\0")?,
            set_in_samplerate: load_symbol(&library, b"lame_set_in_samplerate\0")?,
            set_out_samplerate: load_symbol(&library, b"lame_set_out_samplerate\0")?,
            set_num_channels: load_symbol(&library, b"lame_set_num_channels\0")?,
            set_brate: load_symbol(&library, b"lame_set_brate\0")?,
            set_mode: load_symbol(&library, b"lame_set_mode\0")?,
            set_quality: load_symbol(&library, b"lame_set_quality\0")?,
            set_vbr: load_symbol(&library, b"lame_set_VBR\0")?,
            set_write_vbr_tag: load_symbol(&library, b"lame_set_bWriteVbrTag\0")?,
            init_params: load_symbol(&library, b"lame_init_params\0")?,
            encode_interleaved: load_symbol(&library, b"lame_encode_buffer_interleaved\0")?,
            encode_flush: load_symbol(&library, b"lame_encode_flush\0")?,
            _library: library,
        })
    }
}

fn load_symbol<T: Copy>(library: &libloading::Library, name: &[u8]) -> Result<T, String> {
    let label = String::from_utf8_lossy(name)
        .trim_end_matches('\0')
        .to_string();
    // SAFETY: `name` is a NUL-terminated LAME 3.100 export and T is that function's C type.
    // The function pointer is only called while `library` remains loaded in `LAME`.
    let symbol: libloading::Symbol<T> = unsafe { library.get(name) }
        .map_err(|error| format!("LAME is missing {label}: {error}"))?;
    Ok(*symbol)
}

fn lame_dylib() -> Option<std::path::PathBuf> {
    let mut candidates = Vec::new();
    if let Some(path) = std::env::var_os("OPENDECK_LAME_DYLIB") {
        candidates.push(std::path::PathBuf::from(path));
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            candidates.push(dir.join("../Frameworks/libmp3lame.0.dylib"));
            candidates.push(dir.join("libmp3lame.0.dylib"));
        }
    }
    candidates.push(std::path::PathBuf::from(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../.build-cache/lame/lib/libmp3lame.0.dylib"
    )));
    candidates.into_iter().find(|path| path.is_file())
}

struct Encoder {
    flags: *mut std::ffi::c_void,
    output: Vec<u8>,
}

impl Encoder {
    fn open() -> Result<Self, String> {
        let library = lame()?;
        // SAFETY: lame_init allocates one encoder. Null means the allocation failed.
        let flags = unsafe { (library.init)() };
        if flags.is_null() {
            return Err("LAME could not allocate an encoder".into());
        }
        let encoder = Self {
            flags,
            output: Vec::new(),
        };
        encoder.configure()?;
        Ok(encoder)
    }

    fn configure(&self) -> Result<(), String> {
        let library = lame()?;
        // SAFETY: `flags` came from lame_init and these setters match lame.h.
        // Quality 0 is LAME's slowest psychoacoustic model. 320 kbps, joint stereo,
        // VBR off, and no Xing tag rewrite keep the file a single-pass CBR stream.
        unsafe {
            check(
                (library.set_in_samplerate)(self.flags, TARGET_RATE as i32),
                "sample rate",
            )?;
            check(
                (library.set_out_samplerate)(self.flags, TARGET_RATE as i32),
                "output rate",
            )?;
            check((library.set_num_channels)(self.flags, 2), "channels")?;
            check(
                (library.set_brate)(self.flags, LAME_BITRATE_KBPS),
                "bitrate",
            )?;
            check(
                (library.set_mode)(self.flags, LAME_JOINT_STEREO),
                "channel mode",
            )?;
            check(
                (library.set_quality)(self.flags, LAME_QUALITY_BEST),
                "quality",
            )?;
            check((library.set_vbr)(self.flags, 0), "CBR mode")?;
            check((library.set_write_vbr_tag)(self.flags, 0), "VBR tag")?;
            check((library.init_params)(self.flags), "init")?;
        }
        Ok(())
    }

    fn write_pcm(&mut self, pcm: &mut [i16]) -> Result<(), String> {
        if pcm.is_empty() {
            return Ok(());
        }
        if pcm.len() % 2 != 0 {
            return Err("the encoder expected stereo frames".into());
        }
        let frames =
            i32::try_from(pcm.len() / 2).map_err(|_| "an encode chunk is too large".to_string())?;
        let buffer_len = pcm.len() + 7200;
        let buffer_len_i32 =
            i32::try_from(buffer_len).map_err(|_| "an encode buffer is too large".to_string())?;
        let mut buffer = vec![0u8; buffer_len];
        let library = lame()?;
        // SAFETY: pcm and buffer live for this call. `frames` counts samples per channel.
        let wrote = unsafe {
            (library.encode_interleaved)(
                self.flags,
                pcm.as_mut_ptr(),
                frames,
                buffer.as_mut_ptr(),
                buffer_len_i32,
            )
        };
        if wrote < 0 {
            return Err(format!("LAME encode failed ({wrote})"));
        }
        self.output.extend_from_slice(&buffer[..wrote as usize]);
        Ok(())
    }

    fn finish(mut self) -> Result<Vec<u8>, String> {
        let mut buffer = vec![0u8; 7200];
        let library = lame()?;
        // SAFETY: buffer is 7200 bytes, the size LAME documents for a flush.
        let wrote = unsafe { (library.encode_flush)(self.flags, buffer.as_mut_ptr(), 7200) };
        if wrote < 0 {
            return Err(format!("LAME flush failed ({wrote})"));
        }
        self.output.extend_from_slice(&buffer[..wrote as usize]);
        Ok(std::mem::take(&mut self.output))
    }
}

impl Drop for Encoder {
    fn drop(&mut self) {
        if self.flags.is_null() {
            return;
        }
        if let Some(library) = LAME.get() {
            // SAFETY: flags was allocated by lame_init and is closed once.
            unsafe { (library.close)(self.flags) };
        }
        self.flags = std::ptr::null_mut();
    }
}

fn check(code: i32, what: &str) -> Result<(), String> {
    if code < 0 {
        Err(format!("LAME rejected the {what} ({code})"))
    } else {
        Ok(())
    }
}

fn build_resampler(input_rate: u32) -> Result<SincFixedIn<f32>, String> {
    let ratio = f64::from(TARGET_RATE) / f64::from(input_rate);
    let parameters = SincInterpolationParameters {
        sinc_len: 256,
        f_cutoff: 0.95,
        interpolation: SincInterpolationType::Linear,
        oversampling_factor: 256,
        window: WindowFunction::BlackmanHarris2,
    };
    SincFixedIn::<f32>::new(ratio, 1.0, parameters, RESAMPLE_CHUNK, 2)
        .map_err(|error| error.to_string())
}

struct OpenFile {
    sample_rate: u32,
    bits_per_sample: Option<u32>,
    tags: MirrorTags,
    format: Box<dyn FormatReader>,
    decoder: Box<dyn Decoder>,
    track_id: u32,
    declared_frames: Option<u64>,
    decoded_frames: u64,
    consecutive_errors: u32,
    skipped_packets: u32,
}

impl OpenFile {
    fn open(path: &Path) -> Result<Self, String> {
        let file = File::open(path).map_err(|error| error.to_string())?;
        let stream = MediaSourceStream::new(Box::new(file), Default::default());
        let mut hint = Hint::new();
        if let Some(extension) = path.extension().and_then(|extension| extension.to_str()) {
            hint.with_extension(extension);
        }
        let mut probed = symphonia::default::get_probe()
            .format(
                &hint,
                stream,
                &FormatOptions::default(),
                &MetadataOptions::default(),
            )
            .map_err(|error| error.to_string())?;
        let mut tags = MirrorTags::default();
        if let Some(metadata) = probed.metadata.get() {
            if let Some(revision) = metadata.current() {
                absorb_tags(&mut tags, revision);
            }
        }
        let mut format = probed.format;
        if let Some(revision) = format.metadata().current() {
            absorb_tags(&mut tags, revision);
        }
        let (track_id, sample_rate, bits_per_sample, declared_frames, params) = {
            let track = format
                .tracks()
                .iter()
                .find(|track| track.codec_params.codec != CODEC_TYPE_NULL)
                .ok_or_else(|| "no decodable audio track".to_string())?;
            let sample_rate = track
                .codec_params
                .sample_rate
                .filter(|rate| *rate > 0)
                .ok_or_else(|| "the file has no sample rate".to_string())?;
            (
                track.id,
                sample_rate,
                track.codec_params.bits_per_sample,
                track.codec_params.n_frames.filter(|frames| *frames > 0),
                track.codec_params.clone(),
            )
        };
        let decoder = symphonia::default::get_codecs()
            .make(&params, &DecoderOptions::default())
            .map_err(|error| error.to_string())?;
        Ok(Self {
            sample_rate,
            bits_per_sample,
            tags,
            format,
            decoder,
            track_id,
            declared_frames,
            decoded_frames: 0,
            consecutive_errors: 0,
            skipped_packets: 0,
        })
    }

    fn next_stereo(&mut self) -> Result<Option<Vec<f32>>, String> {
        loop {
            let packet = match self.format.next_packet() {
                Ok(packet) => packet,
                Err(SymphoniaError::IoError(error))
                    if error.kind() == std::io::ErrorKind::UnexpectedEof =>
                {
                    return Ok(None);
                }
                Err(SymphoniaError::ResetRequired) => return Ok(None),
                Err(error) => return Err(error.to_string()),
            };
            if let Some(revision) = self.format.metadata().current() {
                absorb_tags(&mut self.tags, revision);
            }
            if packet.track_id() != self.track_id {
                continue;
            }
            match self.decoder.decode(&packet) {
                Ok(decoded) => {
                    self.consecutive_errors = 0;
                    let spec = *decoded.spec();
                    let mut buffer = SampleBuffer::<f32>::new(decoded.capacity() as u64, spec);
                    buffer.copy_interleaved_ref(decoded);
                    let stereo = to_stereo(buffer.samples(), spec.channels.count());
                    self.decoded_frames = self
                        .decoded_frames
                        .saturating_add((stereo.len() / 2) as u64);
                    return Ok(Some(stereo));
                }
                Err(SymphoniaError::DecodeError(_)) => {
                    self.consecutive_errors = self.consecutive_errors.saturating_add(1);
                    self.skipped_packets = self.skipped_packets.saturating_add(1);
                    if self.consecutive_errors > 32 || self.skipped_packets > 4_096 {
                        return Err("the file has too many undecodable audio packets".into());
                    }
                }
                Err(error) => return Err(error.to_string()),
            }
        }
    }
}

impl OpenFile {
    /// A source that stops well before the length its header declares is a
    /// cut-off file: a copy still in progress, a failed download, or a damaged
    /// disk. Publishing its MP3 would put a silently short track on the stick,
    /// so the encode fails and the next pass tries again.
    fn ensure_complete(&self) -> Result<(), String> {
        let Some(declared) = self.declared_frames else {
            return Ok(());
        };
        let slack = u64::from(self.sample_rate) / 2;
        if self.decoded_frames.saturating_add(slack) >= declared {
            return Ok(());
        }
        let rate = f64::from(self.sample_rate);
        Err(format!(
            "the file ends early: {:.1} s of audio decoded, its header declares {:.1} s",
            self.decoded_frames as f64 / rate,
            declared as f64 / rate
        ))
    }
}

fn to_stereo(samples: &[f32], channels: usize) -> Vec<f32> {
    match channels {
        0 => Vec::new(),
        1 => samples
            .iter()
            .flat_map(|sample| [*sample, *sample])
            .collect(),
        2 => samples.to_vec(),
        count => samples
            .chunks(count)
            .filter(|frame| frame.len() == count)
            .flat_map(|frame| [frame[0], frame[1]])
            .collect(),
    }
}

fn absorb_tags(tags: &mut MirrorTags, revision: &MetadataRevision) {
    for tag in revision.tags() {
        let Some(text) = clean_value(&tag.value) else {
            continue;
        };
        match tag.std_key {
            Some(StandardTagKey::TrackTitle) => fill(&mut tags.title, text),
            Some(StandardTagKey::Artist) => fill(&mut tags.artist, text),
            Some(StandardTagKey::Album) => fill(&mut tags.album, text),
            Some(StandardTagKey::Genre) => fill(&mut tags.genre, text),
            Some(StandardTagKey::Comment) => fill(&mut tags.comment, text),
            Some(StandardTagKey::Bpm) => fill(&mut tags.bpm, text),
            _ => {
                let key = tag.key.to_ascii_lowercase();
                if matches!(key.as_str(), "initialkey" | "tkey" | "key") {
                    fill(&mut tags.musical_key, text);
                } else if matches!(key.as_str(), "bpm" | "tbpm" | "tempo") {
                    fill(&mut tags.bpm, text);
                }
            }
        }
    }
    if tags.artwork_jpeg.is_none() {
        for visual in revision.visuals() {
            if let Some(jpeg) = jpeg_for_embed(&visual.data, &visual.media_type) {
                tags.artwork_jpeg = Some(jpeg);
                break;
            }
        }
    }
}

fn fill(slot: &mut Option<String>, text: String) {
    if slot.is_none() {
        *slot = Some(text);
    }
}

fn clean_value(value: &Value) -> Option<String> {
    if matches!(value, Value::Binary(_)) {
        return None;
    }
    let text = clean_text(&value.to_string());
    if text.is_empty() {
        None
    } else {
        Some(text)
    }
}

fn clean_text(value: &str) -> String {
    value
        .chars()
        .filter(|character| !character.is_control())
        .take(256)
        .collect::<String>()
        .trim()
        .to_string()
}

fn format_bpm(bpm: f64) -> String {
    let rounded = (bpm * 100.0).round() / 100.0;
    format!("{rounded:.2}")
        .trim_end_matches('0')
        .trim_end_matches('.')
        .to_string()
}

fn apply_analysis(tags: &mut MirrorTags, bpm: Option<f64>, musical_key: Option<&str>) {
    if let Some(bpm) = bpm.filter(|bpm| bpm.is_finite() && *bpm > 0.0) {
        tags.bpm = Some(format_bpm(bpm));
    }
    if let Some(key) = musical_key.map(str::trim).filter(|key| !key.is_empty()) {
        tags.musical_key = Some(clean_text(key));
    }
}

pub(crate) fn jpeg_for_embed(data: &[u8], mime: &str) -> Option<Vec<u8>> {
    let format = image_format(data, mime)?;
    let image = image::load_from_memory_with_format(data, format).ok()?;
    if format == image::ImageFormat::Jpeg
        && image.width() <= ARTWORK_MAX_EDGE
        && image.height() <= ARTWORK_MAX_EDGE
        && data.len() <= 250_000
    {
        return Some(data.to_vec());
    }
    let scaled = if image.width() > ARTWORK_MAX_EDGE || image.height() > ARTWORK_MAX_EDGE {
        image.resize(
            ARTWORK_MAX_EDGE,
            ARTWORK_MAX_EDGE,
            image::imageops::FilterType::Lanczos3,
        )
    } else {
        image
    };
    let mut bytes = Vec::new();
    scaled
        .write_to(&mut Cursor::new(&mut bytes), image::ImageFormat::Jpeg)
        .ok()?;
    Some(bytes)
}

fn image_format(data: &[u8], mime: &str) -> Option<image::ImageFormat> {
    let mime = mime.to_ascii_lowercase();
    if mime.contains("png") || data.starts_with(b"\x89PNG") {
        Some(image::ImageFormat::Png)
    } else if mime.contains("jpeg") || mime.contains("jpg") || data.starts_with(&[0xFF, 0xD8]) {
        Some(image::ImageFormat::Jpeg)
    } else {
        None
    }
}

fn tags_empty(tags: &MirrorTags) -> bool {
    tags.title.is_none()
        && tags.artist.is_none()
        && tags.album.is_none()
        && tags.genre.is_none()
        && tags.comment.is_none()
        && tags.bpm.is_none()
        && tags.musical_key.is_none()
        && tags.artwork_jpeg.is_none()
}

pub(crate) fn write_id3v23(path: &Path, tags: &MirrorTags) -> Result<(), String> {
    let mut tag = id3::Tag::new();
    if let Some(value) = tags.title.as_deref() {
        tag.set_title(value);
    }
    if let Some(value) = tags.artist.as_deref() {
        tag.set_artist(value);
    }
    if let Some(value) = tags.album.as_deref() {
        tag.set_album(value);
    }
    if let Some(value) = tags.genre.as_deref() {
        tag.set_genre(value);
    }
    if let Some(value) = tags.comment.as_deref() {
        tag.add_frame(id3::frame::Comment {
            lang: "eng".to_string(),
            description: String::new(),
            text: value.to_string(),
        });
    }
    if let Some(value) = tags.bpm.as_deref() {
        tag.set_text("TBPM", value);
    }
    if let Some(value) = tags.musical_key.as_deref() {
        tag.set_text("TKEY", value);
    }
    if let Some(jpeg) = tags.artwork_jpeg.as_deref() {
        tag.add_frame(id3::frame::Picture {
            mime_type: "image/jpeg".to_string(),
            picture_type: id3::frame::PictureType::CoverFront,
            description: String::new(),
            data: jpeg.to_vec(),
        });
    }
    tag.write_to_path(path, id3::Version::Id3v23)
        .map_err(|error| error.to_string())
}

fn publish(dest: &Path, audio: &[u8], tags: &MirrorTags) -> Result<(), String> {
    crate::mp3_mirror::atomic_replace(dest, |tmp| {
        {
            let mut file = File::create(tmp)?;
            file.write_all(audio)?;
            file.sync_all()?;
        }
        if !tags_empty(tags) {
            write_id3v23(tmp, tags)
                .map_err(|error| std::io::Error::new(std::io::ErrorKind::Other, error))?;
            let file = File::open(tmp)?;
            file.sync_all()?;
        }
        Ok(())
    })
    .map_err(|error| error.to_string())
}

pub(crate) fn encode_mirror(
    source: &Path,
    dest: &Path,
    analysis_bpm: Option<f64>,
    analysis_key: Option<&str>,
    mut during_encode: impl FnMut(),
) -> Result<(), String> {
    let mut opened = OpenFile::open(source)?;
    apply_analysis(&mut opened.tags, analysis_bpm, analysis_key);
    let dither = opened.bits_per_sample.map(|bits| bits > 16).unwrap_or(true);
    let mut resampler = if opened.sample_rate == TARGET_RATE {
        None
    } else {
        Some(build_resampler(opened.sample_rate)?)
    };
    let mut encoder = Encoder::open()?;
    let mut left = Vec::new();
    let mut right = Vec::new();
    let mut rng = Dither::new();
    let mut packets = 0u32;
    while let Some(stereo) = opened.next_stereo()? {
        push_stereo(&mut left, &mut right, &stereo);
        packets = packets.saturating_add(1);
        if packets % 8 == 0 {
            during_encode();
        }
        drain(
            false,
            &mut resampler,
            &mut left,
            &mut right,
            dither,
            &mut rng,
            &mut encoder,
        )?;
    }
    during_encode();
    opened.ensure_complete()?;
    drain(
        true,
        &mut resampler,
        &mut left,
        &mut right,
        dither,
        &mut rng,
        &mut encoder,
    )?;
    let audio = encoder.finish()?;
    let facts = mp3_frame_facts(&audio)?;
    if !facts.cbr
        || i32::try_from(facts.bitrate_kbps).ok() != Some(LAME_BITRATE_KBPS)
        || facts.sample_rate != TARGET_RATE
        || !facts.joint_stereo
    {
        return Err(format!(
            "LAME produced {} frames that are not a 320 kbps 44.1 kHz joint-stereo CBR stream",
            facts.frames
        ));
    }
    publish(dest, &audio, &opened.tags)
}

fn push_stereo(left: &mut Vec<f32>, right: &mut Vec<f32>, stereo: &[f32]) {
    for frame in stereo.chunks_exact(2) {
        left.push(frame[0]);
        right.push(frame[1]);
    }
}

fn drain(
    flush: bool,
    resampler: &mut Option<SincFixedIn<f32>>,
    left: &mut Vec<f32>,
    right: &mut Vec<f32>,
    dither: bool,
    rng: &mut Dither,
    encoder: &mut Encoder,
) -> Result<(), String> {
    if let Some(resampler) = resampler {
        let need = resampler.input_frames_next().max(1);
        while left.len() >= need && right.len() >= need {
            let input = [&left[..need], &right[..need]];
            let mut out_left = vec![0.0; resampler.output_frames_max()];
            let mut out_right = vec![0.0; resampler.output_frames_max()];
            let (_consumed, produced) = resampler
                .process_into_buffer(&input, &mut [&mut out_left[..], &mut out_right[..]], None)
                .map_err(|error| error.to_string())?;
            write_planar(
                &out_left[..produced],
                &out_right[..produced],
                dither,
                rng,
                encoder,
            )?;
            left.drain(..need);
            right.drain(..need);
        }
        let tail_remains = flush && !left.is_empty() && left.len() == right.len();
        if tail_remains {
            let input = [&left[..], &right[..]];
            let mut out_left = vec![0.0; resampler.output_frames_max()];
            let mut out_right = vec![0.0; resampler.output_frames_max()];
            let (_consumed, produced) = resampler
                .process_partial_into_buffer(
                    Some(&input),
                    &mut [&mut out_left[..], &mut out_right[..]],
                    None,
                )
                .map_err(|error| error.to_string())?;
            write_planar(
                &out_left[..produced],
                &out_right[..produced],
                dither,
                rng,
                encoder,
            )?;
            left.clear();
            right.clear();
        } else if flush {
            let mut out_left = vec![0.0; resampler.output_frames_max()];
            let mut out_right = vec![0.0; resampler.output_frames_max()];
            let (_consumed, produced) = resampler
                .process_partial_into_buffer::<&[f32], &mut [f32]>(
                    None,
                    &mut [&mut out_left[..], &mut out_right[..]],
                    None,
                )
                .map_err(|error| error.to_string())?;
            write_planar(
                &out_left[..produced],
                &out_right[..produced],
                dither,
                rng,
                encoder,
            )?;
        }
        return Ok(());
    }
    let frames = if flush {
        left.len().min(right.len())
    } else {
        left.len().min(right.len()) / 1152 * 1152
    };
    if frames == 0 {
        return Ok(());
    }
    write_planar(&left[..frames], &right[..frames], dither, rng, encoder)?;
    left.drain(..frames);
    right.drain(..frames);
    Ok(())
}

fn write_planar(
    left: &[f32],
    right: &[f32],
    dither: bool,
    rng: &mut Dither,
    encoder: &mut Encoder,
) -> Result<(), String> {
    let mut pcm = quantize_interleaved(left, right, dither, rng);
    encoder.write_pcm(&mut pcm)
}

#[cfg(test)]
pub(crate) fn write_stereo_pcm_wav(path: &Path, rate: u32, interleaved: &[i16]) {
    let data_len = interleaved.len() * 2;
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&(36 + data_len as u32).to_le_bytes());
    bytes.extend_from_slice(b"WAVEfmt ");
    bytes.extend_from_slice(&16u32.to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes());
    bytes.extend_from_slice(&2u16.to_le_bytes());
    bytes.extend_from_slice(&rate.to_le_bytes());
    bytes.extend_from_slice(&(rate * 4).to_le_bytes());
    bytes.extend_from_slice(&4u16.to_le_bytes());
    bytes.extend_from_slice(&16u16.to_le_bytes());
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&(data_len as u32).to_le_bytes());
    for sample in interleaved {
        bytes.extend_from_slice(&sample.to_le_bytes());
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(path, bytes).unwrap();
}

#[cfg(test)]
pub(crate) fn test_encode_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    LOCK.lock().unwrap_or_else(|error| error.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    fn scratch(name: &str) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!(
            "opendeck-mp3-encode-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|elapsed| elapsed.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    fn stereo_sine(rate: u32, seconds: f32, hz: f32) -> Vec<i16> {
        let frames = (rate as f32 * seconds) as usize;
        (0..frames)
            .flat_map(|index| {
                let sample = ((index as f32 / rate as f32) * hz * std::f32::consts::TAU).sin();
                let value = (sample * 0.2 * 32_767.0) as i16;
                [value, value]
            })
            .collect()
    }

    fn write_aiff(path: &Path, interleaved: &[i16]) {
        let data_len = interleaved.len() * 2;
        let form_size = 46 + data_len as u32;
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"FORM");
        bytes.extend_from_slice(&form_size.to_be_bytes());
        bytes.extend_from_slice(b"AIFFCOMM");
        bytes.extend_from_slice(&18u32.to_be_bytes());
        bytes.extend_from_slice(&2u16.to_be_bytes());
        bytes.extend_from_slice(&((interleaved.len() / 2) as u32).to_be_bytes());
        bytes.extend_from_slice(&16u16.to_be_bytes());
        bytes.extend_from_slice(&[0x40, 0x0E, 0xAC, 0x44, 0, 0, 0, 0, 0, 0]);
        bytes.extend_from_slice(b"SSND");
        bytes.extend_from_slice(&(8 + data_len as u32).to_be_bytes());
        bytes.extend_from_slice(&0u32.to_be_bytes());
        bytes.extend_from_slice(&0u32.to_be_bytes());
        for sample in interleaved {
            bytes.extend_from_slice(&sample.to_be_bytes());
        }
        std::fs::write(path, bytes).unwrap();
    }

    fn tagged_flac(path: &Path, tags: &[&str]) {
        let mut bytes = b"fLaC".to_vec();
        bytes.extend_from_slice(&[0, 0, 0, 34]);
        let mut stream = [0u8; 34];
        stream[..4].copy_from_slice(&[0, 16, 0, 16]);
        let info = (48_000u64 << 44) | (1 << 41) | (15 << 36) | 16;
        stream[10..18].copy_from_slice(&info.to_be_bytes());
        bytes.extend_from_slice(&stream);
        let mut comments = vec![0u8; 4];
        comments.extend_from_slice(&(tags.len() as u32).to_le_bytes());
        for tag in tags {
            comments.extend_from_slice(&(tag.len() as u32).to_le_bytes());
            comments.extend_from_slice(tag.as_bytes());
        }
        bytes.push(0x84);
        bytes.extend_from_slice(&(comments.len() as u32).to_be_bytes()[1..]);
        bytes.extend_from_slice(&comments);
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
        std::fs::write(path, bytes).unwrap();
    }

    fn symphonia_mp3(path: &Path) -> (u32, u64) {
        let file = File::open(path).unwrap();
        let stream = MediaSourceStream::new(Box::new(file), Default::default());
        let probed = symphonia::default::get_probe()
            .format(
                &Hint::new(),
                stream,
                &FormatOptions::default(),
                &MetadataOptions::default(),
            )
            .unwrap();
        let mut format = probed.format;
        let track = format
            .tracks()
            .iter()
            .find(|track| track.codec_params.codec != CODEC_TYPE_NULL)
            .unwrap();
        let rate = track.codec_params.sample_rate.unwrap();
        let track_id = track.id;
        let params = track.codec_params.clone();
        let mut decoder = symphonia::default::get_codecs()
            .make(&params, &DecoderOptions::default())
            .unwrap();
        let mut frames = 0u64;
        while let Ok(packet) = format.next_packet() {
            if packet.track_id() != track_id {
                continue;
            }
            if let Ok(decoded) = decoder.decode(&packet) {
                frames += decoded.frames() as u64;
            }
        }
        (rate, frames)
    }

    fn assert_cbr_320(path: &Path) {
        let bytes = std::fs::read(path).unwrap();
        let facts = mp3_frame_facts(&bytes).unwrap();
        assert!(
            facts.frames > 0,
            "symphonia and the frame scan both see audio"
        );
        assert_eq!(facts.bitrate_kbps, 320);
        assert_eq!(facts.sample_rate, 44_100);
        assert!(facts.cbr);
        assert!(facts.joint_stereo);
        let (rate, frames) = symphonia_mp3(path);
        assert_eq!(rate, 44_100);
        assert!(frames > 0);
    }

    #[test]
    fn frame_header_reads_cbr_320_joint_stereo() {
        let mut frame = vec![0u8; 1045];
        frame[0] = 0xFF;
        frame[1] = 0xFB;
        frame[2] = 0xE2;
        frame[3] = 0x40;
        let facts = mp3_frame_facts(&frame).unwrap();
        assert_eq!(facts.frames, 1);
        assert_eq!(facts.bitrate_kbps, 320);
        assert_eq!(facts.sample_rate, 44_100);
        assert!(facts.cbr);
        assert!(facts.joint_stereo);
        let mut tagged = b"ID3\x03\x00\x00\x00\x00\x00\x00".to_vec();
        tagged.extend_from_slice(&frame);
        assert_eq!(mp3_frame_facts(&tagged).unwrap().frames, 1);
    }

    #[test]
    fn jpeg_artwork_is_limited_to_800_px() {
        let image =
            image::RgbImage::from_fn(1000, 40, |x, _y| image::Rgb([(x % 255) as u8, 20, 40]));
        let mut bytes = Vec::new();
        image::DynamicImage::ImageRgb8(image)
            .write_to(&mut Cursor::new(&mut bytes), image::ImageFormat::Jpeg)
            .unwrap();
        let embedded = jpeg_for_embed(&bytes, "image/jpeg").unwrap();
        let decoded =
            image::load_from_memory_with_format(&embedded, image::ImageFormat::Jpeg).unwrap();
        assert!(decoded.width() <= 800);
        assert!(decoded.height() <= 800);
        assert!(embedded.starts_with(&[0xFF, 0xD8]));
    }

    #[test]
    fn flac_wav_and_aiff_become_cbr_320_mp3() {
        let _guard = test_encode_lock();
        let root = scratch("fixtures");
        let flac = root.join("song.flac");
        tagged_flac(
            &flac,
            &[
                "TITLE=Mon Amour",
                "ARTIST=Jessie Ware",
                "ALBUM=Superbloom",
                "GENRE=House",
                "COMMENT=Booth",
                "BPM=126",
                "INITIALKEY=8A",
            ],
        );
        let flac_mp3 = root.join("song.mp3");
        encode_mirror(&flac, &flac_mp3, Some(128.5), Some("Am"), || {}).unwrap();
        assert_cbr_320(&flac_mp3);
        let tag = id3::Tag::read_from_path(&flac_mp3).unwrap();
        assert_eq!(tag.title(), Some("Mon Amour"));
        assert_eq!(tag.artist(), Some("Jessie Ware"));
        assert_eq!(tag.album(), Some("Superbloom"));
        assert_eq!(tag.genre(), Some("House"));
        assert_eq!(
            tag.comments().next().map(|comment| comment.text.as_str()),
            Some("Booth")
        );
        assert_eq!(
            tag.get("TBPM").and_then(|frame| frame.content().text()),
            Some("128.5")
        );
        assert_eq!(
            tag.get("TKEY").and_then(|frame| frame.content().text()),
            Some("Am")
        );
        let (rate, frames) = symphonia_mp3(&flac_mp3);
        assert_eq!(rate, 44_100);
        assert!(frames > 0);
        assert!(frames < 44_100 / 4, "a 16-sample flac stays a short file");

        let aiff = root.join("take.aiff");
        write_aiff(&aiff, &stereo_sine(44_100, 0.25, 440.0));
        let aiff_mp3 = root.join("take.mp3");
        encode_mirror(&aiff, &aiff_mp3, None, None, || {}).unwrap();
        assert_cbr_320(&aiff_mp3);
        let (_rate, aiff_frames) = symphonia_mp3(&aiff_mp3);
        let aiff_seconds = aiff_frames as f64 / 44_100.0;
        assert!(aiff_seconds > 0.20 && aiff_seconds < 0.45, "{aiff_seconds}");

        let wav = root.join("long.wav");
        let audio_seconds = 5.0f32;
        write_stereo_pcm_wav(&wav, 48_000, &stereo_sine(48_000, audio_seconds, 220.0));
        let wav_mp3 = root.join("long.mp3");
        let (user_before, system_before) = rusage_cpu();
        let started = Instant::now();
        encode_mirror(&wav, &wav_mp3, None, None, || {}).unwrap();
        let elapsed = started.elapsed().as_secs_f64();
        let (user_after, system_after) = rusage_cpu();
        assert_cbr_320(&wav_mp3);
        let (_rate, wav_frames) = symphonia_mp3(&wav_mp3);
        let wav_seconds = wav_frames as f64 / 44_100.0;
        assert!(wav_seconds > 4.9 && wav_seconds < 5.25, "{wav_seconds}");
        assert!(elapsed < 60.0, "the fixture encode did not finish");
        let line = format!(
            "mp3 mirror fixture: {audio_seconds:.3}s of 48 kHz stereo -> 320 kbps in {elapsed:.3}s ({:.2}x realtime), user_cpu {:.3}s, system_cpu {:.3}s\n",
            audio_seconds as f64 / elapsed,
            user_after - user_before,
            system_after - system_before,
        );
        eprintln!("{line}");
        let report = std::path::PathBuf::from(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../../.build-cache/mp3-mirror-speed.txt"
        ));
        let _ = std::fs::create_dir_all(report.parent().unwrap());
        let _ = std::fs::write(report, line);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn aac_in_m4a_becomes_cbr_320_mp3() {
        let _guard = test_encode_lock();
        let root = scratch("aac");
        let m4a = std::path::PathBuf::from(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../../crates/compas-sources/fixtures/tone.m4a"
        ));
        let mp3 = root.join("tone.mp3");
        encode_mirror(&m4a, &mp3, None, None, || {}).unwrap();
        assert_cbr_320(&mp3);
        let (_rate, frames) = symphonia_mp3(&mp3);
        let seconds = frames as f64 / 44_100.0;
        assert!(seconds > 0.25 && seconds < 0.40, "{seconds}");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_cut_off_source_fails_and_leaves_no_mp3() {
        let _guard = test_encode_lock();
        let root = scratch("cut");
        let wav = root.join("cut.wav");
        write_stereo_pcm_wav(&wav, 44_100, &stereo_sine(44_100, 4.0, 330.0));
        let full = std::fs::read(&wav).unwrap();
        std::fs::write(&wav, &full[..full.len() / 2]).unwrap();
        let mp3 = root.join("cut.mp3");
        let error = encode_mirror(&wav, &mp3, None, None, || {}).unwrap_err();
        assert!(error.contains("ends early"), "{error}");
        assert!(!mp3.exists());
        std::fs::write(&wav, &full).unwrap();
        encode_mirror(&wav, &mp3, None, None, || {}).unwrap();
        assert_cbr_320(&mp3);
        let _ = std::fs::remove_dir_all(&root);
    }

    fn decoded_left(path: &Path) -> Vec<f32> {
        let file = File::open(path).unwrap();
        let stream = MediaSourceStream::new(Box::new(file), Default::default());
        let options = FormatOptions {
            enable_gapless: true,
            ..Default::default()
        };
        let probed = symphonia::default::get_probe()
            .format(&Hint::new(), stream, &options, &MetadataOptions::default())
            .unwrap();
        let mut format = probed.format;
        let track = format
            .tracks()
            .iter()
            .find(|track| track.codec_params.codec != CODEC_TYPE_NULL)
            .unwrap();
        let track_id = track.id;
        let params = track.codec_params.clone();
        let mut decoder = symphonia::default::get_codecs()
            .make(&params, &DecoderOptions::default())
            .unwrap();
        let mut left = Vec::new();
        while let Ok(packet) = format.next_packet() {
            if packet.track_id() != track_id {
                continue;
            }
            if let Ok(decoded) = decoder.decode(&packet) {
                let spec = *decoded.spec();
                let mut buffer = SampleBuffer::<f32>::new(decoded.capacity() as u64, spec);
                buffer.copy_interleaved_ref(decoded);
                left.extend(buffer.samples().iter().step_by(spec.channels.count()));
            }
        }
        left
    }

    /// The deck reuses the source track's beatgrid for an encoded copy, shifted by
    /// `ENCODED_COPY_DELAY_FRAMES`. This measures that shift the way the deck decodes.
    #[test]
    fn an_encoded_copy_starts_one_fixed_delay_after_its_source() {
        let _guard = test_encode_lock();
        let root = scratch("delay");
        let mut state = 0x1234_5678u32;
        let noise: Vec<i16> = (0..44_100usize)
            .flat_map(|_| {
                state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                let value = ((state >> 16) as i16) / 4;
                [value, value]
            })
            .collect();
        let wav = root.join("noise.wav");
        write_stereo_pcm_wav(&wav, 44_100, &noise);
        let mp3 = root.join("noise.mp3");
        encode_mirror(&wav, &mp3, None, None, || {}).unwrap();
        let copy = decoded_left(&mp3);
        let source: Vec<f32> = noise
            .iter()
            .step_by(2)
            .take(8_192)
            .map(|sample| f32::from(*sample) / 32_768.0)
            .collect();
        let mut best = (0usize, f32::MIN);
        for lag in 0..4_000usize {
            let score: f32 = source
                .iter()
                .zip(&copy[lag..])
                .map(|(left, right)| left * right)
                .sum();
            if score > best.1 {
                best = (lag, score);
            }
        }
        assert_eq!(best.0 as u32, crate::mp3_mirror::ENCODED_COPY_DELAY_FRAMES);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn id3_copies_artwork_bpm_and_key() {
        let _guard = test_encode_lock();
        let root = scratch("tags");
        let wav = root.join("plain.wav");
        write_stereo_pcm_wav(&wav, 44_100, &stereo_sine(44_100, 0.05, 440.0));
        let mp3 = root.join("plain.mp3");
        encode_mirror(&wav, &mp3, None, None, || {}).unwrap();
        let image = image::RgbImage::from_pixel(2, 2, image::Rgb([10, 20, 30]));
        let mut jpeg = Vec::new();
        image::DynamicImage::ImageRgb8(image)
            .write_to(&mut Cursor::new(&mut jpeg), image::ImageFormat::Jpeg)
            .unwrap();
        let tags = MirrorTags {
            title: Some("Title".into()),
            artist: Some("Artist".into()),
            album: Some("Album".into()),
            genre: Some("Techno".into()),
            comment: Some("Comment".into()),
            bpm: Some("140".into()),
            musical_key: Some("F#m".into()),
            artwork_jpeg: jpeg_for_embed(&jpeg, "image/jpeg"),
        };
        write_id3v23(&mp3, &tags).unwrap();
        let bytes = std::fs::read(&mp3).unwrap();
        assert_eq!(&bytes[0..4], b"ID3\x03");
        let tag = id3::Tag::read_from_path(&mp3).unwrap();
        assert_eq!(
            tag.get("TBPM").and_then(|frame| frame.content().text()),
            Some("140")
        );
        assert_eq!(
            tag.get("TKEY").and_then(|frame| frame.content().text()),
            Some("F#m")
        );
        let picture = tag.pictures().next().unwrap();
        assert_eq!(picture.mime_type, "image/jpeg");
        assert!(picture.data.starts_with(&[0xFF, 0xD8]));
        assert_cbr_320(&mp3);
        let _ = std::fs::remove_dir_all(&root);
    }

    fn rusage_cpu() -> (f64, f64) {
        #[cfg(target_os = "macos")]
        {
            unsafe extern "C" {
                fn getrusage(who: i32, usage: *mut u8) -> i32;
            }
            let mut buffer = [0u8; 512];
            // SAFETY: the buffer is larger than macOS struct rusage. The first two
            // fields are ru_utime and ru_stime, each a 16-byte timeval on LP64.
            let code = unsafe { getrusage(0, buffer.as_mut_ptr()) };
            if code != 0 {
                return (0.0, 0.0);
            }
            return (timeval(&buffer, 0), timeval(&buffer, 16));
        }
        #[cfg(not(target_os = "macos"))]
        (0.0, 0.0)
    }

    #[cfg(target_os = "macos")]
    fn timeval(buffer: &[u8], offset: usize) -> f64 {
        let seconds = i64::from_ne_bytes(buffer[offset..offset + 8].try_into().unwrap());
        let microseconds = i64::from_ne_bytes(buffer[offset + 8..offset + 16].try_into().unwrap());
        seconds as f64 + microseconds as f64 / 1_000_000.0
    }
}

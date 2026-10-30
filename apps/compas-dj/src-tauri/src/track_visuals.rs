//! Bounded library previews, isolated from playback and the real-time callback.
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use base64::Engine;
use compas_sources::{AudioSource, LocalFileSource, PcmSource};
use serde::Serialize;
use symphonia::core::meta::{
    Limit, MetadataOptions, MetadataRevision, StandardTagKey, StandardVisualKey,
};
use tauri::{AppHandle, State};

const PEAK_COUNT: usize = 128;
const MAX_FILE_BYTES: u64 = 256 * 1024 * 1024;
const MAX_ART_BYTES: usize = 2 * 1024 * 1024;
const MAX_DURATION_SECS: u64 = 20 * 60;
const MAX_FRAMES: u64 = 96_000 * MAX_DURATION_SECS;
const WORK_BUDGET: Duration = Duration::from_secs(15);
static BUSY: AtomicBool = AtomicBool::new(false);

#[derive(Default, Serialize)]
pub struct TrackVisuals {
    peaks: Vec<f32>,
    band_peaks: Vec<[f32; 3]>,
    artwork_data_url: Option<String>,
    title: Option<String>,
    artist: Option<String>,
    album: Option<String>,
    #[serde(skip)]
    artwork_bytes: Option<Vec<u8>>,
    #[serde(skip)]
    artwork_mime: Option<String>,
}

struct WorkerPermit;
impl Drop for WorkerPermit {
    fn drop(&mut self) {
        BUSY.store(false, Ordering::Release);
    }
}

/// UI callers serialize/cache requests. Reject overlap rather than building a work queue.
#[tauri::command]
pub async fn track_visuals(
    _app: AppHandle,
    db: State<'_, crate::db::Db>,
    path: String,
) -> Result<TrackVisuals, String> {
    if path.len() > 4096 || !Path::new(&path).is_absolute() {
        return Err("Track path must be an absolute library path".into());
    }
    let stamp = file_stamp(Path::new(&path));
    {
        let connection = db.0.lock().map_err(|e| e.to_string())?;
        if !crate::db::track_exists(&connection, &path).map_err(|e| e.to_string())? {
            return Err("Import this track into the library before requesting visuals".into());
        }
        if let Some((size, modified_ms)) = stamp {
            if let Some(cached) =
                crate::db::load_track_visual(&connection, &path, size, modified_ms)
                    .map_err(|e| e.to_string())?
            {
                return Ok(visual_from_cache(cached));
            }
        }
    }
    BUSY.compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
        .map_err(|_| "Track visuals are busy; retry shortly".to_string())?;
    let permit = WorkerPermit;
    let source_path = path.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        let _permit = permit;
        analyze(Path::new(&path))
    })
    .await
    .map_err(|e| e.to_string())??;
    {
        let connection = db.0.lock().map_err(|e| e.to_string())?;
        let _ = crate::db::fill_missing_metadata(
            &connection,
            &source_path,
            result.title.as_deref(),
            result.artist.as_deref(),
        );
        if let Some((size, modified_ms)) = stamp {
            let _ = crate::db::save_track_visual(
                &connection,
                &source_path,
                size,
                modified_ms,
                &result.peaks,
                &result.band_peaks,
                result.artwork_bytes.as_deref(),
                result.artwork_mime.as_deref(),
                result.title.as_deref(),
                result.artist.as_deref(),
                result.album.as_deref(),
            );
        }
    }
    Ok(result)
}

fn file_stamp(path: &Path) -> Option<(i64, i64)> {
    let meta = std::fs::metadata(path).ok()?;
    let size = i64::try_from(meta.len()).ok()?;
    let modified_ms = meta
        .modified()
        .ok()?
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|duration| i64::try_from(duration.as_millis()).ok())?;
    Some((size, modified_ms))
}

fn visual_from_cache(cached: crate::db::CachedVisual) -> TrackVisuals {
    TrackVisuals {
        peaks: cached.peaks,
        band_peaks: cached.band_peaks,
        artwork_data_url: cached
            .artwork
            .as_deref()
            .zip(cached.artwork_mime.as_deref())
            .and_then(|(bytes, mime)| artwork_url(mime, bytes)),
        title: cached.title,
        artist: cached.artist,
        album: cached.album,
        artwork_bytes: None,
        artwork_mime: None,
    }
}

fn analyze(path: &Path) -> Result<TrackVisuals, String> {
    let started = Instant::now();
    let file = std::fs::metadata(path).map_err(|e| e.to_string())?;
    if !file.is_file() || file.len() > MAX_FILE_BYTES {
        return Err("Track preview requires a regular file of at most 256 MiB".into());
    }
    let mut result = embedded_metadata(path)?;
    let mut source = LocalFileSource::open(path).map_err(|e| e.to_string())?;
    // Shared source also resolves AlbumArtist when no track Artist tag exists.
    let metadata = source.metadata();
    if result.artist.is_none() && metadata.artist != "Unknown" {
        result.artist = Some(metadata.artist.clone());
    }
    let rate = source.sample_rate();
    if rate == 0 || rate > 96_000 || source.channels() > 8 {
        return Err("Track preview supports up to 96 kHz and eight source channels".into());
    }
    if source
        .metadata()
        .duration_ms
        .is_some_and(|ms| ms > MAX_DURATION_SECS * 1000)
    {
        return Err("Track preview supports tracks up to 20 minutes".into());
    }
    let mut overview = Overview::default();
    let mut splitter = BandSplitter::new(rate);
    loop {
        if started.elapsed() > WORK_BUDGET {
            return Err("Track preview exceeded its 15-second processing budget".into());
        }
        let Some(chunk) = source.next_chunk().map_err(|e| e.to_string())? else {
            break;
        };
        let frames = (chunk.len() / 2) as u64;
        if overview.frames + frames > MAX_FRAMES.min(u64::from(rate) * MAX_DURATION_SECS) {
            return Err("Track preview exceeds its decoded-frame budget".into());
        }
        for stereo in chunk.chunks_exact(2) {
            let stereo = [finite_sample(stereo[0]), finite_sample(stereo[1])];
            overview.push(stereo, splitter.process(stereo));
        }
    }
    if overview.frames == 0 {
        return Err("Track contains no decoded audio".into());
    }
    (result.peaks, result.band_peaks) = overview.finish();
    Ok(result)
}

/// Finite absolute-scale samples: no track-wise or per-bin normalization.
fn finite_sample(value: f32) -> f32 {
    if value.is_finite() {
        value.clamp(-1.0, 1.0)
    } else {
        0.0
    }
}

/// Lightweight first-order crossovers at 250 Hz / 2.5 kHz. Independent L/R
/// histories preserve energy for stereo material whose channels are out of phase.
/// These are broad measured bands, not FFT magnitudes or brick-wall filters.
struct BandSplitter {
    alpha_low: f32,
    alpha_high: f32,
    low: [f32; 2],
    upper: [f32; 2],
}
impl BandSplitter {
    fn new(rate: u32) -> Self {
        let alpha = |hz: f32| 1.0 - (-std::f32::consts::TAU * hz / rate as f32).exp();
        Self {
            alpha_low: alpha(250.0),
            alpha_high: alpha(2500.0),
            low: [0.0; 2],
            upper: [0.0; 2],
        }
    }
    fn process(&mut self, stereo: [f32; 2]) -> [[f32; 3]; 2] {
        let mut result = [[0.0; 3]; 2];
        for ch in 0..2 {
            self.low[ch] += self.alpha_low * (stereo[ch] - self.low[ch]);
            self.upper[ch] += self.alpha_high * (stereo[ch] - self.upper[ch]);
            result[ch] = [
                self.low[ch],
                self.upper[ch] - self.low[ch],
                stereo[ch] - self.upper[ch],
            ];
        }
        result
    }
}

/// Sum-of-squares and frame counts survive pairwise coarsening. Memory remains
/// bounded at 256 bins. Maxima alone flatten compressed music; the overview uses
/// 80% stereo RMS + 20% true peak, retaining short attacks without filling every bin.
/// band_peaks are absolute stereo RMS [low, mid, high], aligned to the 128 peaks.
#[derive(Clone, Copy, Default)]
struct Bin {
    sum_squares: f64,
    band_squares: [f64; 3],
    peak: f32,
    count: u64,
}
impl Bin {
    fn merge(self, other: Self) -> Self {
        Self {
            sum_squares: self.sum_squares + other.sum_squares,
            band_squares: std::array::from_fn(|i| self.band_squares[i] + other.band_squares[i]),
            peak: self.peak.max(other.peak),
            count: self.count + other.count,
        }
    }
}
struct Overview {
    bins: Vec<Bin>,
    width: u64,
    partial: Bin,
    frames: u64,
}
impl Default for Overview {
    fn default() -> Self {
        Self {
            bins: Vec::with_capacity(256),
            width: 1,
            partial: Bin::default(),
            frames: 0,
        }
    }
}
impl Overview {
    fn push(&mut self, stereo: [f32; 2], bands: [[f32; 3]; 2]) {
        let square = |a: f32, b: f32| (f64::from(a).powi(2) + f64::from(b).powi(2)) * 0.5;
        self.partial.sum_squares += square(stereo[0], stereo[1]);
        for i in 0..3 {
            self.partial.band_squares[i] += square(bands[0][i], bands[1][i]);
        }
        self.partial.peak = self.partial.peak.max(stereo[0].abs()).max(stereo[1].abs());
        self.partial.count += 1;
        self.frames += 1;
        if self.partial.count == self.width {
            self.bins.push(self.partial);
            self.partial = Bin::default();
            if self.bins.len() == 256 {
                for i in 0..128 {
                    self.bins[i] = self.bins[i * 2].merge(self.bins[i * 2 + 1]);
                }
                self.bins.truncate(128);
                self.width *= 2;
            }
        }
    }
    fn finish(mut self) -> (Vec<f32>, Vec<[f32; 3]>) {
        if self.partial.count > 0 {
            self.bins.push(self.partial);
        }
        let mut peaks = vec![0.0_f32; PEAK_COUNT];
        let mut bands = vec![[0.0_f32; 3]; PEAK_COUNT];
        if self.frames == 0 {
            return (peaks, bands);
        }
        // Weight overlap in source-frame coordinates. Count-weighted energy avoids
        // treating a short final partial bin like a complete bin. Coarse source bins
        // introduce at most about one thumbnail-bin of temporal smoothing.
        for out in 0..PEAK_COUNT {
            let left = out as f64 * self.frames as f64 / PEAK_COUNT as f64;
            let right = (out + 1) as f64 * self.frames as f64 / PEAK_COUNT as f64;
            let mut energy = [0.0_f64; 4];
            let mut peak = 0.0_f32;
            let mut count = 0.0;
            for (i, bin) in self.bins.iter().enumerate() {
                let start = i as f64 * self.width as f64;
                let overlap = (right.min(start + bin.count as f64) - left.max(start)).max(0.0);
                if overlap == 0.0 {
                    continue;
                }
                let weight = overlap / bin.count as f64;
                energy[0] += bin.sum_squares * weight;
                for band in 0..3 {
                    energy[band + 1] += bin.band_squares[band] * weight;
                }
                peak = peak.max(bin.peak);
                count += overlap;
            }
            if count > 0.0 {
                peaks[out] = (0.8 * (energy[0] / count).sqrt() as f32 + 0.2 * peak).clamp(0.0, 1.0);
                bands[out] = std::array::from_fn(|i| {
                    ((energy[i + 1] / count).sqrt() as f32).clamp(0.0, 1.0)
                });
            }
        }
        (peaks, bands)
    }
}

fn artwork_url(mime: &str, bytes: &[u8]) -> Option<String> {
    if bytes.len() > MAX_ART_BYTES {
        return None;
    }
    let valid = match mime {
        "image/png" => bytes.starts_with(b"\x89PNG\r\n\x1a\n"),
        "image/jpeg" => bytes.starts_with(&[0xff, 0xd8, 0xff]),
        _ => false,
    };
    valid.then(|| {
        format!(
            "data:{mime};base64,{}",
            base64::engine::general_purpose::STANDARD.encode(bytes)
        )
    })
}

fn absorb_metadata(result: &mut TrackVisuals, revision: &MetadataRevision) {
    for tag in revision.tags() {
        let target = match tag.std_key {
            Some(StandardTagKey::TrackTitle) => &mut result.title,
            Some(StandardTagKey::Artist) => &mut result.artist,
            Some(StandardTagKey::Album) => &mut result.album,
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
            if !text.trim().is_empty() {
                *target = Some(text);
            }
        }
    }
    if result.artwork_data_url.is_none() {
        let visual = revision
            .visuals()
            .iter()
            .filter(|v| v.usage == Some(StandardVisualKey::FrontCover))
            .chain(revision.visuals().iter())
            .find(|v| artwork_url(&v.media_type, &v.data).is_some());
        if let Some(visual) = visual {
            result.artwork_data_url = artwork_url(&visual.media_type, &visual.data);
            result.artwork_mime = Some(visual.media_type.clone());
            result.artwork_bytes = Some(visual.data.to_vec());
        }
    }
}

fn embedded_metadata(path: &Path) -> Result<TrackVisuals, String> {
    use symphonia::core::{formats::FormatOptions, io::MediaSourceStream, probe::Hint};
    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut hint = Hint::new();
    if let Some(extension) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(extension);
    }
    let options = MetadataOptions {
        limit_metadata_bytes: Limit::Maximum(64 * 1024),
        limit_visual_bytes: Limit::Maximum(MAX_ART_BYTES),
    };
    let mut probe = symphonia::default::get_probe()
        .format(
            &hint,
            MediaSourceStream::new(Box::new(file), Default::default()),
            &FormatOptions::default(),
            &options,
        )
        .map_err(|e| e.to_string())?;
    let mut result = TrackVisuals::default();
    if let Some(metadata) = probe.metadata.get() {
        if let Some(revision) = metadata.current() {
            absorb_metadata(&mut result, revision);
        }
    }
    if let Some(revision) = probe.format.metadata().current() {
        absorb_metadata(&mut result, revision);
    }
    Ok(result)
}

/// Header-level file facts for the library inspector. No PCM decode.
#[derive(Default, Serialize)]
pub struct TrackFileFacts {
    size_bytes: u64,
    created_ms: Option<i64>,
    modified_ms: Option<i64>,
    sample_rate_hz: Option<u32>,
    channels: Option<u16>,
    bits_per_sample: Option<u32>,
    bitrate_kbps: Option<u32>,
    bitrate_is_average: bool,
    album: Option<String>,
    album_artist: Option<String>,
    genre: Option<String>,
    recorded_date: Option<String>,
    track_number: Option<String>,
    track_total: Option<String>,
    disc_number: Option<String>,
    disc_total: Option<String>,
    composer: Option<String>,
    lyricist: Option<String>,
    comment: Option<String>,
    label: Option<String>,
    remixer: Option<String>,
    original_artist: Option<String>,
}

#[tauri::command]
pub async fn track_file_facts(
    db: State<'_, crate::db::Db>,
    path: String,
) -> Result<TrackFileFacts, String> {
    if path.len() > 4096 || !Path::new(&path).is_absolute() {
        return Err("Track path must be an absolute library path".into());
    }
    {
        let connection = db.0.lock().map_err(|e| e.to_string())?;
        if !crate::db::track_exists(&connection, &path).map_err(|e| e.to_string())? {
            return Err("Import this track into the library before reading file details".into());
        }
    }
    tauri::async_runtime::spawn_blocking(move || read_file_facts(Path::new(&path)))
        .await
        .map_err(|e| e.to_string())?
}

fn system_time_ms(time: Option<SystemTime>) -> Option<i64> {
    time.and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .and_then(|duration| i64::try_from(duration.as_millis()).ok())
}

fn clean_tag(raw: &str) -> Option<String> {
    let text: String = raw.chars().filter(|c| !c.is_control()).take(1024).collect();
    let trimmed = text.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_owned())
    }
}

/// PCM bit depth yields an exact bitrate. Compressed files get an average from
/// size and duration, which is not the encoder's nominal CBR/VBR setting.
fn pcm_bitrate_kbps(sample_rate: u32, channels: u16, bits: u32) -> Option<u32> {
    if !matches!(bits, 8 | 16 | 24 | 32) || sample_rate == 0 || channels == 0 {
        return None;
    }
    u32::try_from(u64::from(sample_rate) * u64::from(channels) * u64::from(bits) / 1000).ok()
}

fn average_bitrate_kbps(size_bytes: u64, duration_ms: u64) -> Option<u32> {
    if size_bytes == 0 || duration_ms == 0 {
        return None;
    }
    u32::try_from(size_bytes.saturating_mul(8) / duration_ms).ok()
}

fn read_file_facts(path: &Path) -> Result<TrackFileFacts, String> {
    let meta = std::fs::metadata(path).map_err(|e| e.to_string())?;
    if !meta.is_file() {
        return Err("Track details require a regular file".into());
    }
    let mut facts = TrackFileFacts {
        size_bytes: meta.len(),
        created_ms: system_time_ms(meta.created().ok()),
        modified_ms: system_time_ms(meta.modified().ok()),
        ..TrackFileFacts::default()
    };
    if meta.len() <= MAX_FILE_BYTES {
        let _ = fill_container_facts(path, &mut facts);
    }
    Ok(facts)
}

fn fill_container_facts(path: &Path, facts: &mut TrackFileFacts) -> Result<(), String> {
    use symphonia::core::codecs::CODEC_TYPE_NULL;
    use symphonia::core::formats::FormatOptions;
    use symphonia::core::io::MediaSourceStream;
    use symphonia::core::probe::Hint;

    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut hint = Hint::new();
    if let Some(extension) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(extension);
    }
    let options = MetadataOptions {
        limit_metadata_bytes: Limit::Maximum(64 * 1024),
        limit_visual_bytes: Limit::Maximum(0),
    };
    let mut probe = symphonia::default::get_probe()
        .format(
            &hint,
            MediaSourceStream::new(Box::new(file), Default::default()),
            &FormatOptions::default(),
            &options,
        )
        .map_err(|e| e.to_string())?;
    if let Some(metadata) = probe.metadata.get() {
        if let Some(revision) = metadata.current() {
            absorb_file_tags(facts, revision);
        }
    }
    if let Some(revision) = probe.format.metadata().current() {
        absorb_file_tags(facts, revision);
    }
    let Some(track) = probe
        .format
        .tracks()
        .iter()
        .find(|track| track.codec_params.codec != CODEC_TYPE_NULL)
    else {
        return Ok(());
    };
    let params = &track.codec_params;
    facts.sample_rate_hz = params
        .sample_rate
        .filter(|rate| (1..=384_000).contains(rate));
    facts.channels = params
        .channels
        .map(|channels| channels.count() as u16)
        .filter(|count| (1..=32).contains(count));
    facts.bits_per_sample = params
        .bits_per_sample
        .filter(|bits| matches!(bits, 8 | 16 | 24 | 32));
    if let (Some(rate), Some(channels), Some(bits)) =
        (facts.sample_rate_hz, facts.channels, facts.bits_per_sample)
    {
        facts.bitrate_kbps = pcm_bitrate_kbps(rate, channels, bits);
        facts.bitrate_is_average = false;
    }
    if facts.bitrate_kbps.is_none() {
        let duration_ms = match (params.n_frames, facts.sample_rate_hz) {
            (Some(frames), Some(rate)) if rate > 0 && frames > 0 => {
                Some(frames.saturating_mul(1000) / u64::from(rate))
            }
            _ => None,
        };
        if let Some(duration_ms) = duration_ms {
            facts.bitrate_kbps = average_bitrate_kbps(facts.size_bytes, duration_ms);
            facts.bitrate_is_average = facts.bitrate_kbps.is_some();
        }
    }
    Ok(())
}

fn absorb_file_tags(facts: &mut TrackFileFacts, revision: &MetadataRevision) {
    for tag in revision.tags() {
        let slot = match tag.std_key {
            Some(StandardTagKey::Album) => &mut facts.album,
            Some(StandardTagKey::AlbumArtist) => &mut facts.album_artist,
            Some(StandardTagKey::Genre) => &mut facts.genre,
            Some(StandardTagKey::Date) => {
                if let Some(text) = clean_tag(&tag.value.to_string()) {
                    facts.recorded_date = Some(text);
                }
                continue;
            }
            Some(StandardTagKey::OriginalDate) => &mut facts.recorded_date,
            Some(StandardTagKey::TrackNumber) => &mut facts.track_number,
            Some(StandardTagKey::TrackTotal) => &mut facts.track_total,
            Some(StandardTagKey::DiscNumber) => &mut facts.disc_number,
            Some(StandardTagKey::DiscTotal) => &mut facts.disc_total,
            Some(StandardTagKey::Composer) => &mut facts.composer,
            Some(StandardTagKey::Lyricist) => &mut facts.lyricist,
            Some(StandardTagKey::Comment) => &mut facts.comment,
            Some(StandardTagKey::Label) => &mut facts.label,
            Some(StandardTagKey::Remixer) => &mut facts.remixer,
            Some(StandardTagKey::OriginalArtist) => &mut facts.original_artist,
            _ => continue,
        };
        if slot.is_none() {
            *slot = clean_tag(&tag.value.to_string());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn measure(samples: impl Iterator<Item = f32>, rate: u32) -> (Vec<f32>, Vec<[f32; 3]>) {
        let mut overview = Overview::default();
        let mut splitter = BandSplitter::new(rate);
        for value in samples {
            let stereo = [finite_sample(value), -finite_sample(value)];
            overview.push(stereo, splitter.process(stereo));
            assert!(overview.bins.len() <= 256);
        }
        overview.finish()
    }

    #[test]
    fn envelope_preserves_end_impulse_across_compaction() {
        let (peaks, bands) = measure(
            (0..16_384).map(|i| if i == 16_383 { 0.9 } else { 0.1 }),
            48_000,
        );
        assert_eq!(peaks.len(), 128);
        assert_eq!(bands.len(), peaks.len());
        assert!((peaks[0] - 0.1).abs() < 0.001);
        assert!(peaks[127] > peaks[0] + 0.1);
        assert!(peaks[..126].iter().all(|&p| (p - 0.1).abs() < 0.001));
    }

    #[test]
    fn envelope_ignores_nonfinite_and_clamps() {
        let (peaks, bands) = measure([f32::NAN, 5.0, f32::INFINITY].into_iter(), 48_000);
        assert!(peaks
            .iter()
            .chain(bands.iter().flatten())
            .all(|p| p.is_finite() && (0.0..=1.0).contains(p)));
    }

    #[test]
    fn measured_bands_identify_bass_midrange_and_treble_without_stereo_cancellation() {
        for (frequency, expected) in [(60.0, 0), (1000.0, 1), (10_000.0, 2)] {
            let (_, bands) = measure(
                (0..48_000)
                    .map(|i| 0.8 * (std::f32::consts::TAU * frequency * i as f32 / 48_000.0).sin()),
                48_000,
            );
            let mean: [f32; 3] =
                std::array::from_fn(|b| bands[1..].iter().map(|v| v[b]).sum::<f32>() / 127.0);
            assert!(mean[expected] > 0.25, "stereo energy was lost: {mean:?}");
            assert!(
                (0..3).all(|b| b == expected || mean[expected] > mean[b] * 1.4),
                "frequency={frequency} bands={mean:?}"
            );
        }
    }

    #[test]
    fn rms_mix_retains_body_variation_when_every_peak_is_identical() {
        let frames = 16_384;
        let sample = |i: usize| {
            if i % 128 == 0 {
                0.95
            } else if i < frames / 2 {
                0.1
            } else {
                0.6
            }
        };
        let old_max: Vec<f32> = (0..128)
            .map(|bin| {
                (bin * 128..(bin + 1) * 128)
                    .map(sample)
                    .fold(0.0_f32, f32::max)
            })
            .collect();
        let (peaks, _) = measure((0..frames).map(sample), 48_000);
        assert!(old_max.iter().all(|&p| p == 0.95));
        let quiet = peaks[..64].iter().sum::<f32>() / 64.0;
        let loud = peaks[64..].iter().sum::<f32>() / 64.0;
        println!("identical-peak fixture: old max=0.95 throughout; mixed quiet={quiet:.4}, loud={loud:.4}");
        assert!(loud > quiet + 0.3);
        assert!(quiet < 0.35 && loud < 0.8);
    }

    #[test]
    fn artwork_requires_supported_mime_signature_and_size() {
        assert!(artwork_url("image/png", b"\x89PNG\r\n\x1a\n")
            .unwrap()
            .starts_with("data:image/png;base64,"));
        assert!(artwork_url("image/jpeg", &[0xff, 0xd8, 0xff]).is_some());
        assert!(artwork_url("image/svg+xml", b"<svg/>").is_none());
        assert!(artwork_url("image/png", b"wrong").is_none());
        assert!(artwork_url("image/jpeg", &vec![0xff; MAX_ART_BYTES + 1]).is_none());
    }

    #[test]
    fn missing_file_returns_error() {
        assert!(analyze(Path::new("/nonexistent/opendeck-track-preview.wav")).is_err());
    }

    #[test]
    fn streamed_wav_measures_start_and_end_not_just_prefix() {
        use std::io::Write;
        let path =
            std::env::temp_dir().join(format!("opendeck-visuals-{}.wav", std::process::id()));
        let frames = 16_384_u32;
        let data_len = frames * 4;
        let mut bytes = Vec::with_capacity(data_len as usize + 44);
        bytes.extend_from_slice(b"RIFF");
        bytes.extend_from_slice(&(36 + data_len).to_le_bytes());
        bytes.extend_from_slice(b"WAVEfmt ");
        bytes.extend_from_slice(&16_u32.to_le_bytes());
        bytes.extend_from_slice(&1_u16.to_le_bytes());
        bytes.extend_from_slice(&2_u16.to_le_bytes());
        bytes.extend_from_slice(&48_000_u32.to_le_bytes());
        bytes.extend_from_slice(&192_000_u32.to_le_bytes());
        bytes.extend_from_slice(&4_u16.to_le_bytes());
        bytes.extend_from_slice(&16_u16.to_le_bytes());
        bytes.extend_from_slice(b"data");
        bytes.extend_from_slice(&data_len.to_le_bytes());
        for frame in 0..frames {
            let amplitude: i16 = if frame == frames - 1 { 24_576 } else { 4096 };
            bytes.extend_from_slice(&amplitude.to_le_bytes());
            bytes.extend_from_slice(&(-amplitude).to_le_bytes());
        }
        std::fs::File::create(&path)
            .unwrap()
            .write_all(&bytes)
            .unwrap();
        let result = analyze(&path);
        std::fs::remove_file(&path).unwrap();
        let result = result.unwrap();
        assert_eq!(result.peaks.len(), 128);
        assert!((result.peaks[0] - 0.125).abs() < 0.001);
        assert!(result.peaks[127] > result.peaks[0] + 0.1);
        assert_eq!(result.band_peaks.len(), result.peaks.len());
        assert!(result.artwork_data_url.is_none());
        assert!(result.title.is_none());
    }

    #[test]
    fn file_facts_read_pcm_bitrate_from_wav_header() {
        use std::io::Write;
        let path =
            std::env::temp_dir().join(format!("opendeck-file-facts-{}.wav", std::process::id()));
        let frames = 48_u32;
        let data_len = frames * 4;
        let mut bytes = Vec::with_capacity(44 + data_len as usize);
        bytes.extend_from_slice(b"RIFF");
        bytes.extend_from_slice(&(36 + data_len).to_le_bytes());
        bytes.extend_from_slice(b"WAVEfmt ");
        bytes.extend_from_slice(&16_u32.to_le_bytes());
        bytes.extend_from_slice(&1_u16.to_le_bytes());
        bytes.extend_from_slice(&2_u16.to_le_bytes());
        bytes.extend_from_slice(&48_000_u32.to_le_bytes());
        bytes.extend_from_slice(&192_000_u32.to_le_bytes());
        bytes.extend_from_slice(&4_u16.to_le_bytes());
        bytes.extend_from_slice(&16_u16.to_le_bytes());
        bytes.extend_from_slice(b"data");
        bytes.extend_from_slice(&data_len.to_le_bytes());
        bytes.extend(std::iter::repeat(0).take(data_len as usize));
        std::fs::File::create(&path)
            .unwrap()
            .write_all(&bytes)
            .unwrap();
        let facts = read_file_facts(&path);
        std::fs::remove_file(&path).unwrap();
        let facts = facts.unwrap();
        assert_eq!(facts.size_bytes, bytes.len() as u64);
        assert!(facts.modified_ms.is_some());
        assert_eq!(facts.sample_rate_hz, Some(48_000));
        assert_eq!(facts.channels, Some(2));
        assert_eq!(facts.bits_per_sample, Some(16));
        assert_eq!(facts.bitrate_kbps, Some(1_536));
        assert!(!facts.bitrate_is_average);
    }

    #[test]
    fn bitrate_helpers_distinguish_pcm_from_average() {
        assert_eq!(pcm_bitrate_kbps(48_000, 2, 16), Some(1_536));
        assert_eq!(pcm_bitrate_kbps(44_100, 2, 12), None);
        assert_eq!(average_bitrate_kbps(180_000, 8_000), Some(180));
        assert_eq!(average_bitrate_kbps(0, 8_000), None);
    }

    #[test]
    fn file_facts_missing_file_returns_error() {
        assert!(read_file_facts(Path::new("/nonexistent/opendeck-file-facts.wav")).is_err());
    }
}

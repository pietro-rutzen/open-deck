//! Independent headphone/booth outputs, fed once by the master mixer's stereo buses.
//! Explicit routes select an exact device and zero-based stereo channel pair. A bounded
//! linear converter bridges nominal sample rates. Independent clock drift is handled by
//! silence/re-prime on underrun and bounded backlog discard; this is not adaptive ASRC.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use compas_core::{CompasError, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SizedSample};
use rtrb::Consumer;

const PRIME_SAMPLES: usize = 2048;
const MAX_BACKLOG_SAMPLES: usize = PRIME_SAMPLES * 4;
const MAX_DISCARD_FRAMES_PER_BLOCK: usize = 256;
const MIN_SAMPLE_RATE: u32 = 8_000;
const MAX_SAMPLE_RATE: u32 = 384_000;
const MAX_ADVANCE_PER_FRAME: usize = 48;

#[derive(Default)]
pub struct MonitorLatency {
    device_latency_bits: AtomicU64,
    prime_latency_bits: AtomicU64,
    underruns: AtomicU64,
    discarded_frames: AtomicU64,
    queued_frames: AtomicU64,
    primed: AtomicBool,
}
impl MonitorLatency {
    pub fn device_latency_secs(&self) -> f64 {
        f64::from_bits(self.device_latency_bits.load(Ordering::Relaxed))
    }
    pub fn prime_latency_secs(&self) -> f64 {
        f64::from_bits(self.prime_latency_bits.load(Ordering::Relaxed))
    }
    /// Buffer depletion events after playback was primed, since this stream opened.
    /// Startup waiting is excluded; this is not a physical-device xrun counter.
    pub fn underruns(&self) -> u64 {
        self.underruns.load(Ordering::Relaxed)
    }
    /// Source frames skipped to bound backlog from independently clocked devices.
    pub fn discarded_frames(&self) -> u64 {
        self.discarded_frames.load(Ordering::Relaxed)
    }
    /// Most recent callback's queued source frames; excludes two interpolation frames.
    pub fn queued_frames(&self) -> u64 {
        self.queued_frames.load(Ordering::Relaxed)
    }
    pub fn primed(&self) -> bool {
        self.primed.load(Ordering::Relaxed)
    }
    fn publish_reader(&self, reader: &MonitorReader) {
        self.underruns.store(reader.underruns, Ordering::Relaxed);
        self.discarded_frames
            .store(reader.discarded_frames, Ordering::Relaxed);
        self.queued_frames
            .store((reader.consumer.slots() / 2) as u64, Ordering::Relaxed);
        self.primed.store(reader.primed, Ordering::Relaxed);
    }
}

/// Default output configuration advertised by the OS; channel numbers do not establish
/// which physical sockets carry each pair. Controller presets need separate provenance.
#[derive(Debug, Clone)]
pub struct OutputDeviceInfo {
    pub name: String,
    pub channels: u16,
    pub sample_rate: u32,
}

pub fn output_device_details() -> Vec<OutputDeviceInfo> {
    let host = cpal::default_host();
    let Ok(devices) = host.output_devices() else {
        return Vec::new();
    };
    devices
        .filter_map(|device| {
            let name = device.name().ok()?;
            let config = device.default_output_config().ok()?;
            Some(OutputDeviceInfo {
                name,
                channels: config.channels(),
                sample_rate: config.sample_rate().0,
            })
        })
        .collect()
}

pub fn output_device_names() -> Vec<String> {
    let host = cpal::default_host();
    host.output_devices()
        .map(|devices| devices.filter_map(|d| d.name().ok()).collect())
        .unwrap_or_default()
}

/// Owns a stream on its creating control thread. Health is reported by CPAL's separate
/// error callback; the audio data callback never locks, logs, allocates or performs I/O.
pub struct CueOutput {
    _stream: cpal::Stream,
    pub device_name: String,
    pub sample_rate: u32,
    pub channels: u16,
    pub first_channel: u16,
    failed: Arc<AtomicBool>,
    last_error: Arc<Mutex<Option<String>>>,
}
impl CueOutput {
    pub fn failed(&self) -> bool {
        self.failed.load(Ordering::Acquire)
    }
    pub fn last_error(&self) -> Option<String> {
        self.last_error.lock().ok().and_then(|e| e.clone())
    }
}

/// Compatibility wrapper: first pair of named/default device, same nominal rate.
/// An explicitly missing or ambiguous name is an error, never a default-device fallback.
pub fn open_cue_output(device_name: Option<&str>, consumer: Consumer<f32>) -> Result<CueOutput> {
    open_monitor_output("cue", device_name, 0, None, None, consumer, None, true)
}
pub fn open_cue_output_with_latency(
    device_name: Option<&str>,
    consumer: Consumer<f32>,
    latency: Arc<MonitorLatency>,
) -> Result<CueOutput> {
    open_monitor_output(
        "cue",
        device_name,
        0,
        None,
        None,
        consumer,
        Some(latency),
        true,
    )
}
/// Explicit headphone route. `first_channel=2` means physical channels 3/4.
pub fn open_cue_output_route_with_latency(
    device: &str,
    first_channel: u16,
    source_sample_rate: u32,
    active_sample_rate: Option<u32>,
    consumer: Consumer<f32>,
    latency: Arc<MonitorLatency>,
) -> Result<CueOutput> {
    open_monitor_output(
        "cue",
        Some(device),
        first_channel,
        Some(source_sample_rate),
        active_sample_rate,
        consumer,
        Some(latency),
        false,
    )
}
pub fn open_booth_output(device_name: Option<&str>, consumer: Consumer<f32>) -> Result<CueOutput> {
    open_monitor_output("booth", device_name, 0, None, None, consumer, None, true)
}
pub fn open_booth_output_with_latency(
    device_name: Option<&str>,
    source_sample_rate: Option<u32>,
    active_sample_rate: Option<u32>,
    consumer: Consumer<f32>,
    latency: Arc<MonitorLatency>,
) -> Result<CueOutput> {
    open_monitor_output(
        "booth",
        device_name,
        0,
        source_sample_rate,
        active_sample_rate,
        consumer,
        Some(latency),
        true,
    )
}

fn route_error(message: impl Into<String>) -> CompasError {
    CompasError::Device(message.into())
}
fn validate_route(
    channels: u16,
    first: u16,
    source: u32,
    target: u32,
    allow_mono: bool,
) -> Result<()> {
    if channels == 0
        || !(allow_mono && channels == 1 && first == 0)
            && first.checked_add(1).is_none_or(|right| right >= channels)
    {
        return Err(route_error(
            "Headphone stereo channel pair is outside the selected device configuration",
        ));
    }
    if !(MIN_SAMPLE_RATE..=MAX_SAMPLE_RATE).contains(&source)
        || !(MIN_SAMPLE_RATE..=MAX_SAMPLE_RATE).contains(&target)
    {
        return Err(route_error(
            "Monitor sample rates must be between 8000 and 384000 Hz",
        ));
    }
    Ok(())
}

fn open_monitor_output(
    label: &str,
    device_name: Option<&str>,
    first: u16,
    source_rate: Option<u32>,
    active_sample_rate: Option<u32>,
    consumer: Consumer<f32>,
    latency: Option<Arc<MonitorLatency>>,
    allow_mono: bool,
) -> Result<CueOutput> {
    let host = cpal::default_host();
    let device = match device_name {
        Some(want) => {
            let mut matches = host
                .output_devices()
                .map_err(|e| route_error(e.to_string()))?
                .filter(|device| device.name().is_ok_and(|name| name == want));
            let device = matches
                .next()
                .ok_or_else(|| route_error(format!("{label} output device '{want}' not found")))?;
            if matches.next().is_some() {
                return Err(route_error(format!(
                    "Multiple output devices are named '{want}'; a unique device is required"
                )));
            }
            device
        }
        None => host
            .default_output_device()
            .ok_or_else(|| route_error("no default output device"))?,
    };
    let name = device.name().map_err(|e| route_error(e.to_string()))?;
    let current = device
        .default_output_config()
        .map_err(|e| route_error(e.to_string()))?;
    let chosen = match device.supported_output_configs() {
        Ok(ranges) => {
            crate::device_format::config_at_active_rate(current, ranges, active_sample_rate)
        }
        Err(error) => {
            tracing::warn!(
                "could not list {label} output formats ({error}); using the device default"
            );
            current
        }
    };
    let sample_format = chosen.sample_format();
    let config: cpal::StreamConfig = chosen.into();
    let source_rate = source_rate.unwrap_or(config.sample_rate.0);
    validate_route(
        config.channels,
        first,
        source_rate,
        config.sample_rate.0,
        allow_mono,
    )?;
    let failed = Arc::new(AtomicBool::new(false));
    let last_error = Arc::new(Mutex::new(None));
    let stream = match sample_format {
        cpal::SampleFormat::F32 => build_cue_stream::<f32>(
            &device,
            &config,
            first,
            source_rate,
            consumer,
            latency,
            failed.clone(),
            last_error.clone(),
        ),
        cpal::SampleFormat::I16 => build_cue_stream::<i16>(
            &device,
            &config,
            first,
            source_rate,
            consumer,
            latency,
            failed.clone(),
            last_error.clone(),
        ),
        cpal::SampleFormat::U16 => build_cue_stream::<u16>(
            &device,
            &config,
            first,
            source_rate,
            consumer,
            latency,
            failed.clone(),
            last_error.clone(),
        ),
        other => Err(route_error(format!("unsupported sample format: {other:?}"))),
    }?;
    stream.play().map_err(|e| route_error(e.to_string()))?;
    Ok(CueOutput {
        _stream: stream,
        device_name: name,
        sample_rate: config.sample_rate.0,
        channels: config.channels,
        first_channel: first,
        failed,
        last_error,
    })
}

/// Fixed-size interpolation state. Work per output frame is bounded by the validated
/// source/target ratio (at most 48 input frames) and uses complete stereo pairs only.
struct MonitorReader {
    consumer: Consumer<f32>,
    ratio: f64,
    phase: f64,
    a: [f32; 2],
    b: [f32; 2],
    primed: bool,
    underruns: u64,
    discarded_frames: u64,
}
impl MonitorReader {
    fn new(consumer: Consumer<f32>, source: u32, target: u32) -> Self {
        Self {
            consumer,
            ratio: f64::from(source) / f64::from(target),
            phase: 0.0,
            a: [0.0; 2],
            b: [0.0; 2],
            primed: false,
            underruns: 0,
            discarded_frames: 0,
        }
    }
    fn pop_frame(&mut self) -> Option<[f32; 2]> {
        if self.consumer.slots() < 2 {
            return None;
        }
        Some([
            self.consumer.pop().unwrap_or(0.0),
            self.consumer.pop().unwrap_or(0.0),
        ])
    }
    fn begin_block(&mut self) {
        // Bound accumulated latency from independent clocks. This deliberately drops
        // stale frames rather than continuously changing pitch to chase ring occupancy.
        let mut discarded = false;
        for _ in 0..MAX_DISCARD_FRAMES_PER_BLOCK {
            if self.consumer.slots() <= MAX_BACKLOG_SAMPLES {
                break;
            }
            let _ = self.pop_frame();
            self.discarded_frames = self.discarded_frames.saturating_add(1);
            discarded = true;
        }
        if discarded {
            self.primed = false;
            self.phase = 0.0;
        }
    }
    fn next_frame(&mut self) -> [f32; 2] {
        if !self.primed {
            if self.consumer.slots() < PRIME_SAMPLES {
                return [0.0; 2];
            }
            let Some(a) = self.pop_frame() else {
                return [0.0; 2];
            };
            let Some(b) = self.pop_frame() else {
                return [0.0; 2];
            };
            self.a = a;
            self.b = b;
            self.phase = 0.0;
            self.primed = true;
        }
        let phase = self.phase as f32;
        let output = [
            self.a[0] + (self.b[0] - self.a[0]) * phase,
            self.a[1] + (self.b[1] - self.a[1]) * phase,
        ];
        self.phase += self.ratio;
        for _ in 0..MAX_ADVANCE_PER_FRAME {
            if self.phase < 1.0 {
                break;
            }
            self.a = self.b;
            let Some(b) = self.pop_frame() else {
                self.underruns = self.underruns.saturating_add(1);
                self.primed = false;
                self.phase = 0.0;
                return output;
            };
            self.b = b;
            self.phase -= 1.0;
        }
        output
    }
}

fn write_frame<T: FromSample<f32> + SizedSample>(
    frame: &mut [T],
    first: usize,
    signal: [f32; 2],
) -> bool {
    frame.fill(T::from_sample(0.0));
    if frame.len() == 1 {
        frame[0] = T::from_sample((signal[0] + signal[1]) * 0.5);
        return true;
    }
    let Some(right) = first.checked_add(1) else {
        return false;
    };
    if right >= frame.len() {
        return false;
    }
    frame[first] = T::from_sample(signal[0]);
    frame[right] = T::from_sample(signal[1]);
    true
}

#[allow(clippy::too_many_arguments)]
fn build_cue_stream<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    first: u16,
    source_rate: u32,
    consumer: Consumer<f32>,
    latency: Option<Arc<MonitorLatency>>,
    failed: Arc<AtomicBool>,
    last_error: Arc<Mutex<Option<String>>>,
) -> Result<cpal::Stream>
where
    T: SizedSample + FromSample<f32> + Send + 'static,
{
    let channels = usize::from(config.channels);
    if let Some(latency) = latency.as_ref() {
        latency.prime_latency_bits.store(
            ((PRIME_SAMPLES / 2) as f64 / f64::from(source_rate)).to_bits(),
            Ordering::Relaxed,
        );
    }
    let mut reader = MonitorReader::new(consumer, source_rate, config.sample_rate.0);
    if let Some(latency) = latency.as_ref() {
        latency.device_latency_bits.store(0, Ordering::Relaxed);
        latency.publish_reader(&reader);
    }
    let callback_failed = Arc::clone(&failed);
    let err_fn = move |error: cpal::StreamError| {
        if let Ok(mut last) = last_error.lock() {
            *last = Some(error.to_string());
        }
        failed.store(true, Ordering::Release);
    };
    device
        .build_output_stream(
            config,
            move |out: &mut [T], info: &cpal::OutputCallbackInfo| {
                if let Some(latency) = latency.as_ref() {
                    let ts = info.timestamp();
                    if let Some(d) = ts.playback.duration_since(&ts.callback) {
                        latency
                            .device_latency_bits
                            .store(d.as_secs_f64().to_bits(), Ordering::Relaxed);
                    }
                }
                reader.begin_block();
                for frame in out.chunks_exact_mut(channels) {
                    if !write_frame(frame, usize::from(first), reader.next_frame()) {
                        callback_failed.store(true, Ordering::Release);
                    }
                }
                if let Some(latency) = latency.as_ref() {
                    latency.publish_reader(&reader);
                }
            },
            err_fn,
            None,
        )
        .map_err(|e| route_error(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rtrb::RingBuffer;
    fn reader(source: u32, target: u32, frames: usize) -> MonitorReader {
        let (mut producer, consumer) = RingBuffer::new(frames * 2 + 2);
        for i in 0..frames {
            producer.push(i as f32).unwrap();
            producer.push(-(i as f32)).unwrap();
        }
        MonitorReader::new(consumer, source, target)
    }
    #[test]
    fn monitor_diagnostics_distinguish_startup_underrun_and_backlog() {
        let (mut tx, rx) = RingBuffer::new(16_384);
        let mut r = MonitorReader::new(rx, 48_000, 48_000);
        let stats = MonitorLatency::default();
        assert_eq!(r.next_frame(), [0.0; 2]);
        stats.publish_reader(&r);
        assert_eq!(stats.underruns(), 0, "startup is not an underrun");
        assert!(!stats.primed());
        for _ in 0..PRIME_SAMPLES / 2 {
            tx.push(0.25).unwrap();
            tx.push(-0.25).unwrap();
        }
        for _ in 0..PRIME_SAMPLES / 2 + 10 {
            r.next_frame();
        }
        stats.publish_reader(&r);
        assert_eq!(
            stats.underruns(),
            1,
            "one depletion, not one event per silent frame"
        );
        assert_eq!(stats.queued_frames(), 0);
        for _ in 0..6000 {
            tx.push(0.25).unwrap();
            tx.push(-0.25).unwrap();
        }
        r.begin_block();
        stats.publish_reader(&r);
        assert_eq!(
            stats.discarded_frames(),
            MAX_DISCARD_FRAMES_PER_BLOCK as u64
        );
        assert_eq!(
            stats.queued_frames(),
            6000 - MAX_DISCARD_FRAMES_PER_BLOCK as u64
        );
        assert_eq!(
            stats.underruns(),
            1,
            "backlog discard is a different condition"
        );
        assert_eq!(r.next_frame(), [0.25, -0.25]);
        stats.publish_reader(&r);
        assert!(stats.primed());
        let (_tx2, rx2) = RingBuffer::new(4096);
        stats.publish_reader(&MonitorReader::new(rx2, 48_000, 48_000));
        assert_eq!(stats.underruns(), 0, "new stream resets counters");
        assert_eq!(stats.discarded_frames(), 0);
        assert!(!stats.primed());
    }

    #[test]
    fn sixty_seconds_simulated_nominal_clocks_preserve_stereo_without_drops() {
        // Deterministic nominal scheduling only: not wall-clock or physical hardware testing.
        for (source, target) in [(48_000u32, 48_000u32), (44_100, 48_000), (48_000, 44_100)] {
            let (mut tx, rx) = RingBuffer::new(16_384);
            let mut r = MonitorReader::new(rx, source, target);
            for _ in 0..PRIME_SAMPLES / 2 {
                tx.push(0.25).unwrap();
                tx.push(-0.5).unwrap();
            }
            let mut generated = 0u64;
            let blocks = u64::from(target) * 60 / 128;
            for block in 0..blocks {
                let due = (block + 1) * 128 * u64::from(source) / u64::from(target);
                for _ in generated..due {
                    tx.push(0.25).unwrap();
                    tx.push(-0.5).unwrap();
                }
                generated = due;
                r.begin_block();
                for _ in 0..128 {
                    assert_eq!(r.next_frame(), [0.25, -0.5]);
                }
            }
            assert_eq!(r.underruns, 0, "{source}->{target}");
            assert_eq!(r.discarded_frames, 0, "{source}->{target}");
            assert!(r.consumer.slots() <= PRIME_SAMPLES + 4);
        }
    }

    #[test]
    fn routed_pair_is_isolated() {
        let mut frame = [9.0; 6];
        assert!(write_frame(&mut frame, 2, [0.25, -0.5]));
        assert_eq!(frame, [0.0, 0.0, 0.25, -0.5, 0.0, 0.0]);
    }
    #[test]
    fn a_short_buffer_stays_silent_instead_of_using_the_master_pair() {
        let mut frame = [0.5; 2];
        assert!(!write_frame(&mut frame, 2, [0.25, -0.5]));
        assert_eq!(frame, [0.0, 0.0]);
    }
    #[test]
    fn route_validation_rejects_invalid_pairs_and_rates() {
        assert!(validate_route(4, 2, 44_100, 48_000, false).is_ok());
        for (channels, first) in [(0, 0), (1, 0), (2, 1), (4, 3), (4, u16::MAX)] {
            assert!(validate_route(channels, first, 48_000, 48_000, false).is_err());
        }
        assert!(validate_route(1, 0, 48_000, 48_000, true).is_ok());
        assert!(validate_route(4, 2, 0, 48_000, false).is_err());
        assert!(validate_route(4, 2, 48_000, 1_000_000, false).is_err());
    }
    #[test]
    fn same_rate_preserves_samples_and_stereo_alignment() {
        let mut reader = reader(48_000, 48_000, 2048);
        for i in 0..1000 {
            assert_eq!(reader.next_frame(), [i as f32, -(i as f32)]);
        }
    }
    #[test]
    fn rate_conversion_44100_to_48000_preserves_timeline() {
        let mut reader = reader(44_100, 48_000, 6000);
        for i in 0..4800 {
            let frame = reader.next_frame();
            let expected = i as f32 * 44_100.0 / 48_000.0;
            assert!((frame[0] - expected).abs() < 0.001);
            assert_eq!(frame[0], -frame[1]);
        }
    }
    #[test]
    fn rate_conversion_48000_to_44100_preserves_timeline() {
        let mut reader = reader(48_000, 44_100, 6000);
        for i in 0..4410 {
            let frame = reader.next_frame();
            let expected = i as f32 * 48_000.0 / 44_100.0;
            assert!((frame[0] - expected).abs() < 0.001);
            assert_eq!(frame[0], -frame[1]);
        }
    }
    #[test]
    fn underrun_stays_silent_until_reprimed() {
        let mut reader = reader(48_000, 48_000, 1024);
        for _ in 0..1024 {
            let _ = reader.next_frame();
        }
        assert_eq!(reader.next_frame(), [0.0; 2]);
        assert!(!reader.primed);
    }
    #[test]
    fn clock_backlog_discard_is_bounded_and_restarts_phase() {
        let mut reader = reader(48_000, 48_000, 6000);
        let before = reader.consumer.slots();
        reader.begin_block();
        assert_eq!(
            before - reader.consumer.slots(),
            MAX_DISCARD_FRAMES_PER_BLOCK * 2
        );
        assert!(!reader.primed);
        assert_eq!(reader.next_frame(), [256.0, -256.0]);
    }
}

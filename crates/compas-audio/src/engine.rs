use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use compas_core::{CompasError, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SizedSample};
use rtrb::{Consumer, Producer, PushError, RingBuffer};

use crate::mixer::{AudioCommand, DeckTelemetry, Mixer, RetiredResource};

/// Engine configuration.
#[derive(Debug, Clone)]
pub struct EngineConfig {
    /// Capacity of the control-command ring (commands per audio block are tiny).
    pub command_capacity: usize,
    /// Capacity of the buffer-reclaim ring (retired decks awaiting drop on control thread).
    pub reclaim_capacity: usize,
    /// Sample rate the output device is already running, when the host can report it.
    pub active_sample_rate: Option<u32>,
    /// Open this output instead of the host default. `None` keeps the macOS default device.
    pub output_device: Option<String>,
    /// Stereo pair inside this same stream for the headphone bus. `Some(2)` is outputs 3/4.
    pub embedded_cue_first_channel: Option<u16>,
    /// Live gate for the embedded headphone pair. The callback only reads this atomic.
    pub embed_cue_live: Option<Arc<AtomicBool>>,
}

impl Default for EngineConfig {
    fn default() -> Self {
        EngineConfig {
            command_capacity: 256,
            // Sized to absorb a burst of deck reloads each retiring their mix buffer, with headroom.
            reclaim_capacity: 16,
            active_sample_rate: None,
            output_device: None,
            embedded_cue_first_channel: None,
            embed_cue_live: None,
        }
    }
}

/// Owns the cpal output stream and the control-side command producer. Dropping the
/// engine stops the stream. Created and owned on a dedicated audio thread because
/// `cpal::Stream` is not `Send` on all platforms.
pub struct AudioEngine {
    _stream: cpal::Stream,
    commands: Producer<AudioCommand>,
    reclaim: Consumer<RetiredResource>,
    telemetry: Arc<DeckTelemetry>,
    sample_rate: u32,
    channels: u16,
    embedded_cue_first_channel: Option<u16>,
    device_name: String,
    stream_failed: Arc<AtomicBool>,
    last_stream_error: Arc<Mutex<Option<String>>>,
}

impl AudioEngine {
    /// Open the default output device and start the audio stream. `telemetry` is shared
    /// so other threads can read deck position/state without locking.
    pub fn new(config: EngineConfig, telemetry: Arc<DeckTelemetry>) -> Result<Self> {
        let host = cpal::default_host();
        let device = select_output_device(&host, config.output_device.as_deref())?;
        let current = device
            .default_output_config()
            .map_err(|e| CompasError::Device(e.to_string()))?;
        let chosen = match device.supported_output_configs() {
            Ok(ranges) => crate::device_format::config_at_active_rate(
                current,
                ranges,
                config.active_sample_rate,
            ),
            Err(error) => {
                tracing::warn!("could not list output formats ({error}); using the device default");
                current
            }
        };

        let sample_rate = chosen.sample_rate().0;
        let sample_format = chosen.sample_format();
        let stream_config: cpal::StreamConfig = chosen.into();
        let embedded_cue = config.embedded_cue_first_channel.filter(|first| {
            first
                .checked_add(1)
                .is_some_and(|right| *first >= 2 && right < stream_config.channels)
        });
        let embed_cue_live = config
            .embed_cue_live
            .unwrap_or_else(|| Arc::new(AtomicBool::new(embedded_cue.is_some())));

        let (cmd_tx, cmd_rx) = RingBuffer::<AudioCommand>::new(config.command_capacity);
        let (reclaim_tx, reclaim_rx) = RingBuffer::<RetiredResource>::new(config.reclaim_capacity);
        let mixer = Mixer::new(sample_rate as f32, cmd_rx, reclaim_tx, telemetry.clone());
        let stream_failed = Arc::new(AtomicBool::new(false));
        let last_stream_error = Arc::new(Mutex::new(None));

        let stream = match sample_format {
            cpal::SampleFormat::F32 => build_stream::<f32>(
                &device,
                &stream_config,
                mixer,
                embedded_cue,
                embed_cue_live,
                stream_failed.clone(),
                last_stream_error.clone(),
            ),
            cpal::SampleFormat::I16 => build_stream::<i16>(
                &device,
                &stream_config,
                mixer,
                embedded_cue,
                embed_cue_live,
                stream_failed.clone(),
                last_stream_error.clone(),
            ),
            cpal::SampleFormat::U16 => build_stream::<u16>(
                &device,
                &stream_config,
                mixer,
                embedded_cue,
                embed_cue_live,
                stream_failed.clone(),
                last_stream_error.clone(),
            ),
            other => Err(CompasError::Device(format!(
                "unsupported sample format: {other:?}"
            ))),
        }?;

        stream
            .play()
            .map_err(|e| CompasError::Device(e.to_string()))?;

        Ok(AudioEngine {
            _stream: stream,
            commands: cmd_tx,
            reclaim: reclaim_rx,
            telemetry,
            sample_rate,
            channels: stream_config.channels,
            embedded_cue_first_channel: embedded_cue,
            device_name: device.name().unwrap_or_else(|_| "Unknown output".into()),
            stream_failed,
            last_stream_error,
        })
    }

    pub fn device_name(&self) -> &str {
        &self.device_name
    }

    pub fn channels(&self) -> u16 {
        self.channels
    }

    /// Headphone pair written by this stream, when the opened device has room for it.
    pub fn embedded_cue_first_channel(&self) -> Option<u16> {
        self.embedded_cue_first_channel
    }

    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    pub fn telemetry(&self) -> Arc<DeckTelemetry> {
        self.telemetry.clone()
    }

    /// Whether cpal has reported a stream/device error after startup. This is set from cpal's
    /// error callback (not the audio data callback) so the owner thread can rebuild the stream.
    pub fn stream_failed(&self) -> bool {
        self.stream_failed.load(Ordering::Relaxed)
    }

    /// Last stream/device error text, if cpal provided one.
    pub fn last_stream_error(&self) -> Option<String> {
        self.last_stream_error.lock().ok().and_then(|e| e.clone())
    }

    /// Drop any buffers the audio thread has retired (call periodically / after sends so
    /// freeing never happens on the RT path).
    pub fn drain_reclaimed(&mut self) {
        while let Ok(resource) = self.reclaim.pop() {
            match resource {
                RetiredResource::Buffer(buffer) => drop(buffer),
                RetiredResource::Monitor(sink) => drop(sink),
            }
        }
    }

    /// Preserve a full-ring command for ordered retry by its control-thread owner.
    /// Ring-full telemetry counts attempts; the caller decides retry versus explicit rejection.
    pub fn try_send_command(&mut self, cmd: AudioCommand) -> std::result::Result<(), AudioCommand> {
        let result = push_command(&mut self.commands, &self.telemetry, cmd);
        self.drain_reclaimed();
        result
    }

    /// Compatibility API: rejects/drops on the control thread if the ring is full.
    pub fn send(&mut self, cmd: AudioCommand) -> Result<()> {
        self.try_send_command(cmd)
            .map_err(|_| CompasError::Other("audio command ring full".into()))
    }
}

fn select_output_device(host: &cpal::Host, name: Option<&str>) -> Result<cpal::Device> {
    let Some(want) = name else {
        return host
            .default_output_device()
            .ok_or_else(|| CompasError::Device("no default output device".into()));
    };
    let mut matches = host
        .output_devices()
        .map_err(|error| CompasError::Device(error.to_string()))?
        .filter(|device| device.name().is_ok_and(|device_name| device_name == want));
    let device = matches
        .next()
        .ok_or_else(|| CompasError::Device(format!("output device '{want}' not found")))?;
    if matches.next().is_some() {
        return Err(CompasError::Device(format!(
            "Multiple output devices are named '{want}'"
        )));
    }
    Ok(device)
}

fn render_output_block<T>(
    out: &mut [T],
    channels: usize,
    cue_pair: Option<u16>,
    mut next_buses: impl FnMut() -> ((f32, f32), (f32, f32)),
) where
    T: SizedSample + FromSample<f32>,
{
    let mut frames = out.chunks_exact_mut(channels);
    for frame in frames.by_ref() {
        let (master, cue_bus) = next_buses();
        let cue = cue_pair.map(|first| (usize::from(first), [cue_bus.0, cue_bus.1]));
        write_output_frame(frame, master, cue);
    }
    for sample in frames.into_remainder() {
        *sample = T::from_sample(0.0);
    }
}

fn write_output_frame<T>(frame: &mut [T], master: (f32, f32), cue: Option<(usize, [f32; 2])>)
where
    T: FromSample<f32> + SizedSample,
{
    let (left, right) = master;
    if frame.is_empty() {
        return;
    }
    if frame.len() == 1 {
        frame[0] = T::from_sample((left + right) * 0.5);
        return;
    }
    frame[0] = T::from_sample(left);
    frame[1] = T::from_sample(right);
    for sample in frame.iter_mut().skip(2) {
        *sample = T::from_sample(0.0);
    }
    let Some((first, signal)) = cue else {
        return;
    };
    let Some(right_index) = first.checked_add(1) else {
        return;
    };
    if first >= 2 && right_index < frame.len() {
        frame[first] = T::from_sample(signal[0]);
        frame[right_index] = T::from_sample(signal[1]);
    }
}

fn push_command(
    commands: &mut Producer<AudioCommand>,
    telemetry: &DeckTelemetry,
    command: AudioCommand,
) -> std::result::Result<(), AudioCommand> {
    match commands.push(command) {
        Ok(()) => Ok(()),
        Err(PushError::Full(command)) => {
            telemetry.inc_command_ring_full();
            Err(command)
        }
    }
}

/// Build a cpal output stream for sample type `T`, driving it from `mixer`.
fn build_stream<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    mut mixer: Mixer,
    embedded_cue: Option<u16>,
    embed_cue_live: Arc<AtomicBool>,
    stream_failed: Arc<AtomicBool>,
    last_stream_error: Arc<Mutex<Option<String>>>,
) -> Result<cpal::Stream>
where
    T: SizedSample + FromSample<f32> + Send + 'static,
{
    let channels = config.channels as usize;
    let sample_rate = config.sample_rate.0 as f64;
    let err_fn = move |e: cpal::StreamError| {
        let message = e.to_string();
        stream_failed.store(true, Ordering::Relaxed);
        if let Ok(mut last) = last_stream_error.lock() {
            *last = Some(message.clone());
        }
        tracing::error!("cpal stream error: {message}");
    };

    let stream = device
        .build_output_stream(
            config,
            move |out: &mut [T], info: &cpal::OutputCallbackInfo| {
                let t0 = std::time::Instant::now();
                // Measured DAC latency: time from this callback to when the audio is heard.
                let ts = info.timestamp();
                if let Some(d) = ts.playback.duration_since(&ts.callback) {
                    mixer.publish_latency(d.as_secs_f32());
                }
                mixer.drain_commands();
                if channels == 0 {
                    for sample in out.iter_mut() {
                        *sample = T::from_sample(0.0);
                    }
                    return;
                }
                let cue_pair = embedded_cue.filter(|_| embed_cue_live.load(Ordering::Relaxed));
                render_output_block(out, channels, cue_pair, || {
                    let buses = mixer.next_buses();
                    (buses.master, buses.cue)
                });
                // RT load = time spent in the callback ÷ this block's real-time budget.
                let block_secs = (out.len() / channels) as f64 / sample_rate;
                let load = if block_secs > 0.0 {
                    (t0.elapsed().as_secs_f64() / block_secs) as f32
                } else {
                    0.0
                };
                mixer.publish_rt_load(load);
                mixer.publish_telemetry();
            },
            err_fn,
            None,
        )
        .map_err(|e| CompasError::Device(e.to_string()))?;
    Ok(stream)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn full_command_ring_returns_ownership_for_ordered_retry() {
        let (mut tx, mut rx) = RingBuffer::new(1);
        let telemetry = DeckTelemetry::new();
        assert!(push_command(&mut tx, &telemetry, AudioCommand::SetMasterGain(0.5)).is_ok());
        let buffer = Arc::new(compas_core::DeckBuffer::new(vec![0.0; 16], 48_000));
        let weak = Arc::downgrade(&buffer);
        let retained = push_command(
            &mut tx,
            &telemetry,
            AudioCommand::LoadDeck {
                deck: 0,
                buffer,
                beat_offset: 0.0,
                beat_interval: 0.0,
            },
        )
        .err()
        .expect("full ring returns the owning command");
        assert!(weak.upgrade().is_some());
        assert_eq!(telemetry.command_ring_full(), 1);
        assert!(matches!(
            rx.pop().unwrap(),
            AudioCommand::SetMasterGain(value) if value == 0.5
        ));
        assert!(push_command(&mut tx, &telemetry, retained).is_ok());
        let loaded = rx.pop().unwrap();
        assert!(matches!(loaded, AudioCommand::LoadDeck { deck: 0, .. }));
        drop(loaded);
        assert!(weak.upgrade().is_none());
    }

    #[test]
    fn master_pair_stays_on_outputs_1_and_2_and_cue_uses_3_and_4() {
        let mut frame = [1.0_f32; 4];
        write_output_frame(&mut frame, (0.25, -0.5), Some((2, [0.75, -0.25])));
        assert_eq!(frame, [0.25, -0.5, 0.75, -0.25]);
    }

    #[test]
    fn a_quiet_headphone_pair_does_not_replace_the_master() {
        let mut frame = [1.0_f32; 4];
        write_output_frame(&mut frame, (0.25, -0.5), None);
        assert_eq!(frame, [0.25, -0.5, 0.0, 0.0]);
        write_output_frame(&mut frame, (0.25, -0.5), Some((0, [0.9, 0.9])));
        assert_eq!(frame, [0.25, -0.5, 0.0, 0.0]);
    }

    #[test]
    fn stereo_and_mono_outputs_keep_the_master_mix() {
        let mut stereo = [0.0_f32; 2];
        write_output_frame(&mut stereo, (0.4, -0.2), Some((2, [1.0, 1.0])));
        assert_eq!(stereo, [0.4, -0.2]);
        let mut mono = [0.0_f32; 1];
        write_output_frame(&mut mono, (0.4, -0.2), None);
        assert_eq!(mono, [0.1]);
    }

    #[test]
    fn a_short_output_block_ends_in_silence_instead_of_stale_samples() {
        let mut block = [1.0_f32; 5];
        let mut frames = 0;
        render_output_block(&mut block, 2, None, || {
            frames += 1;
            ((0.5, -0.5), (1.0, 1.0))
        });
        assert_eq!(frames, 2);
        assert_eq!(block, [0.5, -0.5, 0.5, -0.5, 0.0]);
    }
}

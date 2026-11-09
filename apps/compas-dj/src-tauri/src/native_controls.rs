//! Controller-originated mixer and transport commands resolved on the controller thread and sent
//! straight to the engine queue, so performance controls never round-trip through React/IPC.
use crate::EngineMsg;
use compas_audio::{DeckTelemetry, FilterMode, NUM_DECKS, NUM_SAMPLER_PADS};
use serde::Serialize;
use std::sync::atomic::{AtomicU32, AtomicU8, Ordering};
use std::sync::Mutex;

const EQ_BANDS: usize = 3;
const EQ_LOW: usize = 0;
const EQ_MID: usize = 1;
const EQ_HIGH: usize = 2;
const EQ_LIMIT_DB: f64 = 26.0;
const TRIM_MAX: f64 = compas_core::control::TRIM_MAX_GAIN;
const TEMPO_LIMIT_PERCENT: f64 = 100.0;
const FILTER_DEAD_ZONE: f64 = 0.02;

/// Last EQ sent to each deck. A controller knob moves one band, but the engine command carries all
/// three, so UI and controller writes share this store instead of overwriting each other's bands.
#[derive(Default)]
pub(crate) struct DeckEqState([[AtomicU32; EQ_BANDS]; NUM_DECKS]);

impl DeckEqState {
    pub(crate) fn record(&self, deck: usize, low: f32, mid: f32, high: f32) {
        if let Some(bands) = self.0.get(deck) {
            for (band, value) in bands.iter().zip([low, mid, high]) {
                band.store(value.to_bits(), Ordering::Relaxed);
            }
        }
    }

    pub(crate) fn with_band(
        &self,
        deck: usize,
        band: usize,
        value: f32,
    ) -> Option<[f32; EQ_BANDS]> {
        let bands = self.0.get(deck)?;
        bands.get(band)?.store(value.to_bits(), Ordering::Relaxed);
        Some([
            f32::from_bits(bands[EQ_LOW].load(Ordering::Relaxed)),
            f32::from_bits(bands[EQ_MID].load(Ordering::Relaxed)),
            f32::from_bits(bands[EQ_HIGH].load(Ordering::Relaxed)),
        ])
    }
}

/// Finite EQ gain clamped to the engine's ±26 dB range.
pub(crate) fn clamp_eq_db(value: f32) -> Option<f32> {
    value
        .is_finite()
        .then_some(value.clamp(-(EQ_LIMIT_DB as f32), EQ_LIMIT_DB as f32))
}

/// A controller action the native path owns. `mirror` asks the UI to reflect the new value
/// without re-sending it to the engine.
pub(crate) struct NativeAction {
    pub message: Option<EngineMsg>,
    /// Further engine commands applied in order after `message` (sync sends mode, tempo, then follow).
    pub messages: Vec<EngineMsg>,
    pub mirror: bool,
    /// Replaces the control value the UI reflects, when the musical result is not the raw MIDI value.
    pub mirror_value: Option<f64>,
    /// Loop, cue, quantize, sync, and beat-jump state the UI displays. The engine command already ran.
    pub performance: Option<PerformanceMirror>,
    /// Extra controls to mirror, such as the tempo fader after sync.
    pub extra_mirrors: Vec<(String, f64)>,
    /// Mirror this control id instead of the incoming pad control.
    pub mirror_as: Option<String>,
    /// Deck pitch to commit through the effects snapshot after the other messages.
    pub pitch: Option<f64>,
    /// A non-realtime UI event, such as loading the selected library row into a sampler slot.
    pub ui_event: Option<(String, f64)>,
}

fn single(message: Option<EngineMsg>, mirror: bool) -> NativeAction {
    NativeAction {
        message,
        messages: Vec::new(),
        mirror,
        mirror_value: None,
        performance: None,
        extra_mirrors: Vec::new(),
        mirror_as: None,
        pitch: None,
        ui_event: None,
    }
}

/// Active sampler bank. Pads address sixteen locals; the bank selects which sixteen.
#[derive(Default)]
pub(crate) struct SamplerBanks {
    bank: AtomicU8,
}

impl SamplerBanks {
    pub(crate) fn bank(&self) -> usize {
        usize::from(self.bank.load(Ordering::Relaxed)).min(compas_audio::SAMPLER_BANKS - 1)
    }

    pub(crate) fn set_bank(&self, bank: usize) -> usize {
        let bank = bank.min(compas_audio::SAMPLER_BANKS - 1);
        self.bank.store(bank as u8, Ordering::Relaxed);
        bank
    }
}

pub(crate) struct FilterSettings {
    pub mode: FilterMode,
    pub cutoff: f32,
    pub resonance: f32,
}

/// Bipolar DJ filter knob: −1 = full low-pass, 0 = off, +1 = full high-pass.
pub(crate) fn filter_settings(position: f64) -> FilterSettings {
    if position.abs() < FILTER_DEAD_ZONE {
        return FilterSettings {
            mode: FilterMode::Off,
            cutoff: 1000.0,
            resonance: 0.9,
        };
    }
    if position < 0.0 {
        return FilterSettings {
            mode: FilterMode::LowPass,
            cutoff: (20_000.0 * (200.0_f64 / 20_000.0).powf(-position)) as f32,
            resonance: (0.9 - position) as f32,
        };
    }
    FilterSettings {
        mode: FilterMode::HighPass,
        cutoff: (20.0 * (4000.0_f64 / 20.0).powf(position)) as f32,
        resonance: (0.9 + position) as f32,
    }
}

/// Same fixed velocity as an on-screen pad click. The control bus only reports press or release.
const SAMPLE_VELOCITY: u8 = 110;

/// Sampler trigger, and SHIFT stop-or-load. A release never starts playback or a decode.
fn sampler_action(control: &str, value: f64, playing: u64) -> Option<NativeAction> {
    let (slot, action) = control.strip_prefix("sampler.")?.split_once('.')?;
    if action != "trigger" && action != "stop" {
        return None;
    }
    let slot = slot.parse::<usize>().ok()?;
    if slot >= NUM_SAMPLER_PADS {
        return None;
    }
    if !value.is_finite() || value < 0.5 {
        return Some(single(None, false));
    }
    if action == "trigger" {
        return Some(single(
            Some(EngineMsg::TriggerSample {
                slot,
                velocity: SAMPLE_VELOCITY,
            }),
            false,
        ));
    }
    if playing & (1u64 << slot) != 0 {
        return Some(single(Some(EngineMsg::StopSample { slot }), false));
    }
    let mut action = single(None, false);
    action.ui_event = Some((format!("sampler.load.{slot}"), 1.0));
    Some(action)
}

fn deck_control(control: &str) -> Option<(usize, &str)> {
    let rest = control.strip_prefix("deck.")?;
    let (index, name) = rest.split_once('.')?;
    let deck = index.parse::<usize>().ok()?;
    (deck < NUM_DECKS).then_some((deck, name))
}

/// Resolve a controller update to an engine command when the control is natively owned.
/// `None` means the control still belongs to the UI command path.
pub(crate) fn resolve(
    control: &str,
    value: f64,
    telemetry: &DeckTelemetry,
    eq: &DeckEqState,
    performance: &PerformanceState,
    captured: Option<CapturedPosition>,
    sampler_playing: u64,
) -> Option<NativeAction> {
    if let Some(action) = sampler_action(control, value, sampler_playing) {
        return Some(action);
    }
    let (deck, name) = deck_control(control)?;
    if !value.is_finite() {
        return Some(single(None, false));
    }
    let pressed = value >= 0.5;
    let mirrored = |message: EngineMsg| Some(single(Some(message), true));
    let unmirrored = |message: Option<EngineMsg>| Some(single(message, false));
    let eq_band = |band: usize| {
        let db = value.clamp(-EQ_LIMIT_DB, EQ_LIMIT_DB) as f32;
        eq.with_band(deck, band, db)?;
        mirrored(EngineMsg::DeckEqBand { deck, band, db })
    };
    match name {
        "volume" => mirrored(EngineMsg::DeckGain {
            deck,
            gain: value.clamp(0.0, 1.0) as f32,
        }),
        "trim" => mirrored(EngineMsg::DeckTrim {
            deck,
            gain: value.clamp(0.0, TRIM_MAX) as f32,
        }),
        "eq.low" => eq_band(EQ_LOW),
        "eq.mid" => eq_band(EQ_MID),
        "eq.high" => eq_band(EQ_HIGH),
        "filter" => {
            let settings = filter_settings(value.clamp(-1.0, 1.0));
            mirrored(EngineMsg::DeckFilter {
                deck,
                mode: settings.mode,
                cutoff: settings.cutoff,
                resonance: settings.resonance,
            })
        }
        "tempo" => mirrored(EngineMsg::DeckTempo {
            deck,
            ratio: 1.0 + value.clamp(-TEMPO_LIMIT_PERCENT, TEMPO_LIMIT_PERCENT) / 100.0,
        }),
        "play" => {
            unmirrored(
                (pressed && telemetry.is_loaded(deck)).then(|| EngineMsg::DeckPlaying {
                    deck,
                    playing: !telemetry.is_playing(deck),
                }),
            )
        }
        "cue" => unmirrored(Some(EngineMsg::CueButton { deck, pressed })),
        "censor" => unmirrored(Some(EngineMsg::SetCensor {
            deck,
            active: pressed,
        })),
        "start" => unmirrored(pressed.then_some(EngineMsg::DeckSeek { deck, frame: 0.0 })),
        _ => performance_action(control, deck, name, value, telemetry, performance, captured),
    }
}

/// Playhead positions captured when a controller message arrives, so cue/loop/jump actions use
/// the moment of the press instead of the last 30 Hz UI telemetry sample.
pub(crate) struct CapturedPosition {
    /// Engine read position.
    pub frame: f64,
    /// Position currently leaving the speakers (read position minus output latency).
    pub heard_frame: f64,
}

pub(crate) fn capture_position(
    control: &str,
    telemetry: &DeckTelemetry,
) -> Option<CapturedPosition> {
    let (deck, _) = deck_control(control)?;
    if !telemetry.is_loaded(deck) {
        return None;
    }
    let frame = telemetry.playhead_frames(deck);
    let latency_frames = telemetry.deck_rate(deck) * f64::from(telemetry.output_latency_secs());
    let heard_frame = (frame - latency_frames).max(0.0);
    (frame.is_finite() && heard_frame.is_finite())
        .then_some(CapturedPosition { frame, heard_frame })
}

const HOT_CUES: usize = 8;
const BEAT_LOOP_BEATS: [f64; HOT_CUES] = [0.25, 0.5, 1.0, 2.0, 4.0, 8.0, 16.0, 32.0];
const BEAT_JUMP_BEATS: [f64; HOT_CUES] = [-1.0, 1.0, -2.0, 2.0, -4.0, 4.0, -8.0, 8.0];
const JUMP_SCALE_MIN: f64 = 1.0 / 16.0;
const JUMP_SCALE_MAX: f64 = 16.0;
const LOOP_MIN_FRAMES: f64 = 8.0;
const MEMORY_LOOPS: usize = 8;
const SAME_LOOP_FRAMES: f64 = 1.0;

/// Display state for a controller action the engine has already applied.
#[derive(Serialize, Clone, Default)]
pub(crate) struct PerformanceMirror {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub loop_in: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub loop_out: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub loop_active: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub loop_armed: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub loop_beats: Option<f64>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub clear_loop_beats: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hot_cue_frame: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hot_cue_out: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub jump_scale: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quantize: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub synced: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub memory: Option<Vec<MemoryPoint>>,
}

/// One saved loop on a track. Slot 0 of the transport `loops` table is not used for these.
#[derive(Serialize, Clone, Copy)]
pub(crate) struct MemoryPoint {
    pub slot: u8,
    pub in_frame: f64,
    pub out_frame: f64,
    pub active: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub point: bool,
}

#[derive(Clone, Copy)]
struct HotCue {
    frame: f64,
    out_frame: Option<f64>,
}

#[derive(Clone, Copy)]
struct MemoryLoop {
    in_frame: f64,
    out_frame: f64,
    active: bool,
    point: bool,
}

fn hot_loop_out(cue: HotCue) -> Option<f64> {
    cue.out_frame
        .filter(|out| out.is_finite() && *out > cue.frame)
}

fn is_false(value: &bool) -> bool {
    !*value
}

#[derive(Clone)]
struct DeckSnapshot {
    hot_cues: [Option<HotCue>; HOT_CUES],
    quantize: bool,
    grid_offset: f64,
    grid_interval: f64,
    jump_scale: f64,
    track_frames: usize,
    loop_in: f64,
    loop_out: f64,
    loop_active: bool,
    loop_armed: bool,
    loop_beats: Option<f64>,
    source_rate: f64,
    bpm: f64,
    synced: bool,
    partner: Option<usize>,
    keyboard_cue: Option<usize>,
    keyboard_transpose: i8,
    keyboard_restore: Option<f64>,
    keyboard_held: u8,
    keyboard_last: Option<u8>,
    memory: [Option<MemoryLoop>; MEMORY_LOOPS],
    memory_selected: Option<usize>,
}

impl Default for DeckSnapshot {
    fn default() -> Self {
        Self {
            hot_cues: [None; HOT_CUES],
            quantize: false,
            grid_offset: 0.0,
            grid_interval: 0.0,
            jump_scale: 1.0,
            track_frames: 0,
            loop_in: 0.0,
            loop_out: 0.0,
            loop_active: false,
            loop_armed: false,
            loop_beats: None,
            source_rate: 0.0,
            bpm: 0.0,
            synced: false,
            partner: None,
            keyboard_cue: None,
            keyboard_transpose: 0,
            keyboard_restore: None,
            keyboard_held: 0,
            keyboard_last: None,
            memory: [None; MEMORY_LOOPS],
            memory_selected: None,
        }
    }
}

/// Hot-cue, loop, grid, and sync memory read on the controller thread.
/// The audio callback never locks this.
pub(crate) struct PerformanceState {
    decks: [Mutex<DeckSnapshot>; NUM_DECKS],
}

impl Default for PerformanceState {
    fn default() -> Self {
        Self::new()
    }
}

impl PerformanceState {
    pub(crate) fn new() -> Self {
        Self {
            decks: std::array::from_fn(|_| Mutex::new(DeckSnapshot::default())),
        }
    }

    fn with_deck<T>(&self, deck: usize, update: impl FnOnce(&mut DeckSnapshot) -> T) -> Option<T> {
        let deck = self.decks.get(deck)?;
        let mut snapshot = deck.lock().unwrap_or_else(|poison| poison.into_inner());
        Some(update(&mut snapshot))
    }

    pub(crate) fn set_hot_cue(&self, deck: usize, slot: usize, frame: Option<f64>) {
        self.set_hot_cue_range(deck, slot, frame, None);
    }

    pub(crate) fn set_hot_cue_range(
        &self,
        deck: usize,
        slot: usize,
        frame: Option<f64>,
        out_frame: Option<f64>,
    ) {
        if slot >= HOT_CUES {
            return;
        }
        let frame = frame.filter(|frame| frame.is_finite());
        let out_frame = out_frame.filter(|out| out.is_finite());
        self.with_deck(deck, |snapshot| {
            snapshot.hot_cues[slot] = frame.map(|frame| HotCue {
                frame,
                out_frame: out_frame.filter(|out| *out > frame),
            });
        });
    }

    /// Leave or re-enter keyboard mode. Returns the pitch to restore, when a pad was holding one.
    pub(crate) fn reset_keyboard(&self, deck: usize) -> Option<f64> {
        self.with_deck(deck, |snapshot| {
            snapshot.keyboard_cue = None;
            snapshot.keyboard_held = 0;
            snapshot.keyboard_last = None;
            snapshot.keyboard_restore.take()
        })?
    }

    /// Hot-cue selection, then pitched playback from that cue. Shift deletes or transposes.
    pub(crate) fn keyboard(
        &self,
        deck: usize,
        slot: usize,
        shifted: bool,
        pressed: bool,
        heard: Option<f64>,
        current_pitch: f64,
    ) -> Option<NativeAction> {
        if slot >= HOT_CUES || !current_pitch.is_finite() {
            return None;
        }
        self.with_deck(deck, |snapshot| {
            keyboard_pad(deck, slot, shifted, pressed, heard, current_pitch, snapshot)
        })
    }

    pub(crate) fn set_hot_cue_ranges(
        &self,
        deck: usize,
        frames: &[Option<f64>],
        outs: &[Option<f64>],
    ) {
        self.with_deck(deck, |snapshot| {
            for (slot, frame) in frames.iter().enumerate().take(HOT_CUES) {
                let frame = frame.filter(|frame| frame.is_finite());
                let out = outs
                    .get(slot)
                    .copied()
                    .flatten()
                    .filter(|out| out.is_finite());
                snapshot.hot_cues[slot] = frame.map(|frame| HotCue {
                    frame,
                    out_frame: out.filter(|out| *out > frame),
                });
            }
        });
    }

    pub(crate) fn set_loop(
        &self,
        deck: usize,
        in_frame: f64,
        out_frame: f64,
        active: bool,
        armed: bool,
        beats: Option<f64>,
    ) {
        if !in_frame.is_finite() || !out_frame.is_finite() {
            return;
        }
        let beats = beats.filter(|beats| beats.is_finite());
        self.with_deck(deck, |snapshot| {
            snapshot.loop_in = in_frame;
            snapshot.loop_out = out_frame;
            snapshot.loop_active = active;
            snapshot.loop_armed = armed;
            snapshot.loop_beats = beats;
        });
    }

    pub(crate) fn set_quantize(&self, deck: usize, enabled: bool) {
        self.with_deck(deck, |snapshot| snapshot.quantize = enabled);
    }

    pub(crate) fn set_jump_scale(&self, deck: usize, scale: f64) {
        if !scale.is_finite() || scale <= 0.0 {
            return;
        }
        self.with_deck(deck, |snapshot| {
            snapshot.jump_scale = scale.clamp(JUMP_SCALE_MIN, JUMP_SCALE_MAX);
        });
    }

    pub(crate) fn set_synced(&self, deck: usize, synced: bool) {
        self.with_deck(deck, |snapshot| snapshot.synced = synced);
    }

    pub(crate) fn set_partner(&self, deck: usize, partner: Option<usize>) {
        let partner = partner.filter(|partner| *partner < NUM_DECKS && *partner != deck);
        self.with_deck(deck, |snapshot| snapshot.partner = partner);
    }

    pub(crate) fn set_grid(&self, deck: usize, offset: f64, interval: f64) {
        if !offset.is_finite() || !interval.is_finite() {
            return;
        }
        self.with_deck(deck, |snapshot| {
            snapshot.grid_offset = offset;
            snapshot.grid_interval = interval.max(0.0);
        });
    }

    /// Drop the previous track's cues, loop, and grid as the new buffer is installed.
    pub(crate) fn install_track(
        &self,
        deck: usize,
        offset: f64,
        interval: f64,
        source_rate: f64,
        bpm: f64,
        frames: usize,
    ) {
        self.with_deck(deck, |snapshot| {
            snapshot.track_frames = frames;
            snapshot.hot_cues = [None; HOT_CUES];
            snapshot.loop_in = 0.0;
            snapshot.loop_out = 0.0;
            snapshot.loop_active = false;
            snapshot.loop_armed = false;
            snapshot.loop_beats = None;
            snapshot.synced = false;
            snapshot.grid_offset = if offset.is_finite() { offset } else { 0.0 };
            snapshot.grid_interval = if interval.is_finite() {
                interval.max(0.0)
            } else {
                0.0
            };
            snapshot.source_rate = if source_rate.is_finite() {
                source_rate.max(0.0)
            } else {
                0.0
            };
            snapshot.bpm = if bpm.is_finite() { bpm.max(0.0) } else { 0.0 };
            snapshot.keyboard_cue = None;
            snapshot.keyboard_transpose = 0;
            snapshot.keyboard_restore = None;
            snapshot.keyboard_held = 0;
            snapshot.keyboard_last = None;
            snapshot.memory = [None; MEMORY_LOOPS];
            snapshot.memory_selected = None;
        });
    }

    pub(crate) fn set_memory_loops(&self, deck: usize, loops: &[(u8, f64, f64, bool, bool)]) {
        self.with_deck(deck, |snapshot| {
            snapshot.memory = [None; MEMORY_LOOPS];
            snapshot.memory_selected = None;
            let mut armed = false;
            for (slot, in_frame, out_frame, active, point) in
                loops.iter().copied().take(MEMORY_LOOPS)
            {
                let slot = usize::from(slot);
                if slot >= MEMORY_LOOPS || !in_frame.is_finite() {
                    continue;
                }
                if point {
                    snapshot.memory[slot] = Some(MemoryLoop {
                        in_frame,
                        out_frame: in_frame,
                        active: false,
                        point: true,
                    });
                    continue;
                }
                if !out_frame.is_finite() || out_frame <= in_frame {
                    continue;
                }
                let active = active && !armed;
                if active {
                    armed = true;
                }
                snapshot.memory[slot] = Some(MemoryLoop {
                    in_frame,
                    out_frame,
                    active,
                    point: false,
                });
            }
        });
    }

    pub(crate) fn active_memory(&self, deck: usize) -> Option<(f64, f64)> {
        self.with_deck(deck, |snapshot| {
            snapshot
                .memory
                .iter()
                .flatten()
                .find(|region| region.active)
                .map(|region| (region.in_frame, region.out_frame))
        })?
    }

    /// The engine engaged the armed loop. Mirror it when the published bounds are that loop.
    pub(crate) fn adopt_engaged_loop(
        &self,
        deck: usize,
        in_frame: f64,
        out_frame: f64,
    ) -> Option<PerformanceMirror> {
        self.with_deck(deck, |snapshot| {
            if snapshot.loop_active {
                return None;
            }
            let matched = snapshot.memory.iter().flatten().find(|region| {
                region.active && same_loop(region.in_frame, region.out_frame, in_frame, out_frame)
            })?;
            snapshot.loop_in = matched.in_frame;
            snapshot.loop_out = matched.out_frame;
            snapshot.loop_active = true;
            snapshot.loop_armed = false;
            snapshot.loop_beats = None;
            Some(memory_performance(snapshot))
        })?
    }

    pub(crate) fn recall_end(&self, deck: usize, start: f64, end: f64) -> Option<f64> {
        self.with_deck(deck, |snapshot| {
            compas_audio::recall_loop_end(start, end, snapshot.track_frames)
        })?
    }

    pub(crate) fn has_track(&self, deck: usize) -> bool {
        self.with_deck(deck, |snapshot| snapshot.track_frames > 0)
            .unwrap_or(false)
    }

    pub(crate) fn clear_deck(&self, deck: usize) {
        self.install_track(deck, 0.0, 0.0, 0.0, 0.0, 0);
    }

    /// Publish a finished beatgrid without clearing cues, loops, or sync.
    /// An unloaded deck is left untouched so a late result cannot revive it.
    pub(crate) fn publish_analysis(
        &self,
        deck: usize,
        offset: f64,
        interval: f64,
        bpm: f64,
    ) -> bool {
        self.with_deck(deck, |snapshot| {
            if snapshot.track_frames == 0
                || !offset.is_finite()
                || !interval.is_finite()
                || interval <= 0.0
            {
                return false;
            }
            snapshot.grid_offset = offset;
            snapshot.grid_interval = interval;
            snapshot.bpm = if bpm.is_finite() { bpm.max(0.0) } else { 0.0 };
            snapshot.bpm > 0.0
        })
        .unwrap_or(false)
    }
}

fn snap_beat(frame: f64, offset: f64, interval: f64) -> f64 {
    offset + ((frame - offset) / interval).round() * interval
}

fn beat_loop_bounds(heard: f64, beats: f64, offset: f64, interval: f64) -> (f64, f64) {
    let in_frame = (offset + ((heard - offset) / interval).round() * interval).max(0.0);
    (in_frame, in_frame + beats * interval)
}

fn same_loop(in_frame: f64, out_frame: f64, other_in: f64, other_out: f64) -> bool {
    (in_frame - other_in).abs() < SAME_LOOP_FRAMES
        && (out_frame - other_out).abs() < SAME_LOOP_FRAMES
}

fn memory_points(snapshot: &DeckSnapshot) -> Vec<MemoryPoint> {
    snapshot
        .memory
        .iter()
        .enumerate()
        .filter_map(|(slot, region)| {
            region.map(|region| MemoryPoint {
                slot: slot as u8,
                in_frame: region.in_frame,
                out_frame: region.out_frame,
                active: region.active,
                point: region.point,
            })
        })
        .collect()
}

fn memory_performance(snapshot: &DeckSnapshot) -> PerformanceMirror {
    let mut mirror = loop_mirror(snapshot);
    mirror.memory = Some(memory_points(snapshot));
    mirror
}

fn memory_action(snapshot: &DeckSnapshot, message: Option<EngineMsg>) -> NativeAction {
    let mut action = single(message, true);
    action.performance = Some(memory_performance(snapshot));
    action
}

fn loop_mirror(snapshot: &DeckSnapshot) -> PerformanceMirror {
    PerformanceMirror {
        loop_in: Some(snapshot.loop_in),
        loop_out: Some(snapshot.loop_out),
        loop_active: Some(snapshot.loop_active),
        loop_armed: Some(snapshot.loop_armed),
        loop_beats: snapshot.loop_beats,
        clear_loop_beats: snapshot.loop_beats.is_none(),
        ..PerformanceMirror::default()
    }
}

fn mirrored_loop(snapshot: &DeckSnapshot, message: Option<EngineMsg>) -> NativeAction {
    let mut action = single(message, true);
    action.performance = Some(loop_mirror(snapshot));
    action
}

fn performance_action(
    control: &str,
    deck: usize,
    name: &str,
    value: f64,
    telemetry: &DeckTelemetry,
    performance: &PerformanceState,
    captured: Option<CapturedPosition>,
) -> Option<NativeAction> {
    let loaded = telemetry.is_loaded(deck);
    let tempos = std::array::from_fn(|index| telemetry.tempo_ratio(index));
    let position = captured.or_else(|| capture_position(control, telemetry));
    let standby = (telemetry.is_at_cue(deck) && !telemetry.is_playing(deck)).then(|| {
        position
            .as_ref()
            .map(|position| position.frame)
            .filter(|frame| frame.is_finite())
            .unwrap_or_else(|| telemetry.playhead_frames(deck))
    });
    apply_performance(
        deck,
        name,
        value,
        loaded,
        position,
        &tempos,
        performance,
        standby,
    )
}

fn apply_performance(
    deck: usize,
    name: &str,
    value: f64,
    loaded: bool,
    position: Option<CapturedPosition>,
    tempos: &[f64; NUM_DECKS],
    performance: &PerformanceState,
    standby: Option<f64>,
) -> Option<NativeAction> {
    let pressed = value >= 0.5;
    if name == "quantize" && pressed {
        return performance.with_deck(deck, |snapshot| {
            snapshot.quantize = !snapshot.quantize;
            let enabled = snapshot.quantize;
            let mut action = single(None, true);
            action.mirror_value = Some(if enabled { 1.0 } else { 0.0 });
            action.performance = Some(PerformanceMirror {
                quantize: Some(enabled),
                ..PerformanceMirror::default()
            });
            action
        });
    }
    if pressed && (name == "beatjump.scale_down" || name == "beatjump.scale_up") {
        let factor = if name == "beatjump.scale_down" {
            JUMP_SCALE_MIN
        } else {
            JUMP_SCALE_MAX
        };
        return performance.with_deck(deck, |snapshot| {
            snapshot.jump_scale =
                (snapshot.jump_scale * factor).clamp(JUMP_SCALE_MIN, JUMP_SCALE_MAX);
            let scale = snapshot.jump_scale;
            let mut action = single(None, true);
            action.mirror_value = Some(scale);
            action.performance = Some(PerformanceMirror {
                jump_scale: Some(scale),
                ..PerformanceMirror::default()
            });
            action
        });
    }
    if name == "sync" && pressed {
        return sync_action(deck, tempos, performance);
    }
    if !loaded {
        return None;
    }
    if name == "loop.nudge_in" || name == "loop.nudge_out" {
        return nudge_loop(deck, value, name == "loop.nudge_in", performance);
    }
    if !pressed {
        return None;
    }
    if let Some(slot) = pad_slot(name, "hotcue") {
        return hot_cue_action(deck, slot, position, performance);
    }
    if let Some(slot) = pad_slot(name, "clear_hotcue") {
        performance.set_hot_cue(deck, slot, None);
        return Some(single(None, true));
    }
    if let Some(slot) = pad_slot(name, "beatloop") {
        let heard = position?.heard_frame;
        return beat_loop_action(deck, BEAT_LOOP_BEATS[slot], heard, performance);
    }
    if let Some(slot) = pad_slot(name, "beatjump") {
        let frame = position?.frame;
        return beat_jump_action(deck, BEAT_JUMP_BEATS[slot], frame, performance);
    }
    let heard = position.as_ref().map(|position| position.heard_frame);
    let frame = position.as_ref().map(|position| position.frame);
    match name {
        "loop.in" => arm_loop(deck, heard?, performance),
        "loop.out" => finish_loop(deck, heard?, performance),
        "loop.toggle" => toggle_loop(deck, performance),
        "loop.half" => Some(loop_length_or_memory(deck, -1, 0.5, heard, performance)),
        "loop.double" => Some(loop_length_or_memory(deck, 1, 2.0, heard, performance)),
        "loop.memory_save" => Some(save_memory(
            deck,
            position.as_ref().map(|position| position.frame),
            standby,
            performance,
        )),
        "loop.memory_delete" => Some(delete_memory(deck, heard, performance)),
        "loop.memory_forget" => Some(forget_current_memory(deck, performance)),
        "loop.memory_active" => Some(toggle_active_memory(deck, heard, performance)),
        "loop.reloop_stop" => reloop_stop(deck, performance),
        "jump.back" => beat_jump_action(deck, -4.0, frame?, performance),
        "jump.forward" => beat_jump_action(deck, 4.0, frame?, performance),
        _ => None,
    }
}

const KEYBOARD_PITCHES: [i16; HOT_CUES] = [4, 5, 6, 7, 0, 1, 2, 3];

fn keyboard_semitone(slot: usize, transpose: i8) -> f64 {
    f64::from((KEYBOARD_PITCHES[slot] + i16::from(transpose)).clamp(-12, 12))
}

fn keyboard_pad(
    deck: usize,
    slot: usize,
    shifted: bool,
    pressed: bool,
    heard: Option<f64>,
    current_pitch: f64,
    snapshot: &mut DeckSnapshot,
) -> NativeAction {
    if !pressed {
        if shifted || snapshot.keyboard_held & (1 << slot) == 0 {
            return single(None, false);
        }
        snapshot.keyboard_held &= !(1 << slot);
        if snapshot.keyboard_last == Some(slot as u8) {
            snapshot.keyboard_last = None;
        }
        if snapshot.keyboard_held == 0 {
            let mut action = single(None, false);
            action.pitch = snapshot.keyboard_restore.take();
            return action;
        }
        return single(None, false);
    }
    if shifted {
        return keyboard_shift(deck, slot, snapshot);
    }
    if snapshot.keyboard_cue.is_none() {
        return keyboard_select(deck, slot, heard, snapshot);
    }
    keyboard_play(deck, slot, heard, current_pitch, snapshot)
}

fn keyboard_select(
    deck: usize,
    slot: usize,
    heard: Option<f64>,
    snapshot: &mut DeckSnapshot,
) -> NativeAction {
    if snapshot.hot_cues[slot].is_none() {
        let Some(heard) = heard.filter(|frame| frame.is_finite()) else {
            return single(None, false);
        };
        store_hot_cue(snapshot, slot, heard);
    }
    snapshot.keyboard_cue = Some(slot);
    let stored = snapshot.hot_cues[slot];
    let mut action = single(None, true);
    action.mirror_as = Some(format!("deck.{deck}.hotcue.{slot}"));
    action.performance = Some(hot_cue_mirror(stored));
    action
}

fn keyboard_play(
    deck: usize,
    slot: usize,
    heard: Option<f64>,
    current_pitch: f64,
    snapshot: &mut DeckSnapshot,
) -> NativeAction {
    let Some(cue_slot) = snapshot.keyboard_cue else {
        return keyboard_select(deck, slot, heard, snapshot);
    };
    let Some(stored) = snapshot.hot_cues[cue_slot] else {
        snapshot.keyboard_cue = None;
        return keyboard_select(deck, slot, heard, snapshot);
    };
    let bit = 1u8 << slot;
    if snapshot.keyboard_held & bit != 0 {
        return single(None, false);
    }
    if snapshot.keyboard_restore.is_none() {
        snapshot.keyboard_restore = Some(current_pitch);
    }
    snapshot.keyboard_held |= bit;
    snapshot.keyboard_last = Some(slot as u8);
    let mut action = recall_hot_cue(deck, stored, snapshot);
    let starts_playback = action
        .messages
        .iter()
        .any(|message| matches!(message, EngineMsg::DeckPlaying { .. }));
    if !starts_playback {
        action.messages.push(EngineMsg::DeckPlaying {
            deck,
            playing: true,
        });
    }
    action.pitch = Some(keyboard_semitone(slot, snapshot.keyboard_transpose));
    action
}

fn keyboard_shift(deck: usize, slot: usize, snapshot: &mut DeckSnapshot) -> NativeAction {
    if slot < 6 {
        let cleared = if let Some(selected) = snapshot.keyboard_cue.take() {
            snapshot.hot_cues[selected] = None;
            snapshot.keyboard_held = 0;
            snapshot.keyboard_last = None;
            selected
        } else {
            snapshot.hot_cues[slot] = None;
            slot
        };
        let mut action = single(None, true);
        action.mirror_as = Some(format!("deck.{deck}.clear_hotcue.{cleared}"));
        action.pitch = snapshot.keyboard_restore.take();
        return action;
    }
    let delta: i8 = if slot == 6 { 1 } else { -1 };
    snapshot.keyboard_transpose = snapshot
        .keyboard_transpose
        .saturating_add(delta)
        .clamp(-12, 12);
    snapshot
        .keyboard_last
        .and_then(|last| {
            (snapshot.keyboard_held & (1 << last) != 0).then(|| {
                let mut action = single(None, false);
                action.pitch = Some(keyboard_semitone(
                    last as usize,
                    snapshot.keyboard_transpose,
                ));
                action
            })
        })
        .unwrap_or_else(|| single(None, false))
}

fn pad_slot(name: &str, action: &str) -> Option<usize> {
    let slot = name.strip_prefix(action)?.strip_prefix('.')?;
    let index = slot.parse::<usize>().ok()?;
    (index < HOT_CUES).then_some(index)
}

fn hot_cue_action(
    deck: usize,
    slot: usize,
    position: Option<CapturedPosition>,
    performance: &PerformanceState,
) -> Option<NativeAction> {
    let heard = position?.heard_frame.max(0.0);
    performance.with_deck(deck, |snapshot| {
        if let Some(stored) = snapshot.hot_cues[slot] {
            recall_hot_cue(deck, stored, snapshot)
        } else {
            let stored = store_hot_cue(snapshot, slot, heard);
            let mut action = single(None, true);
            action.performance = Some(hot_cue_mirror(Some(stored)));
            action
        }
    })
}

fn store_hot_cue(snapshot: &mut DeckSnapshot, slot: usize, heard: f64) -> HotCue {
    let cue = if snapshot.loop_active && snapshot.loop_out > snapshot.loop_in {
        HotCue {
            frame: snapshot.loop_in.max(0.0),
            out_frame: Some(snapshot.loop_out),
        }
    } else {
        HotCue {
            frame: heard.max(0.0),
            out_frame: None,
        }
    };
    snapshot.hot_cues[slot] = Some(cue);
    cue
}

fn hot_cue_mirror(cue: Option<HotCue>) -> PerformanceMirror {
    PerformanceMirror {
        hot_cue_frame: cue.map(|cue| cue.frame),
        hot_cue_out: cue.and_then(hot_loop_out),
        ..PerformanceMirror::default()
    }
}

/// A point hot cue seeks. A loop hot cue starts that loop from its in point.
fn recall_hot_cue(deck: usize, stored: HotCue, snapshot: &mut DeckSnapshot) -> NativeAction {
    let Some(loop_out) = hot_loop_out(stored) else {
        let target = if snapshot.quantize {
            snapped(stored.frame, snapshot).max(0.0)
        } else {
            stored.frame
        };
        return single(
            Some(EngineMsg::DeckSeek {
                deck,
                frame: target,
            }),
            false,
        );
    };
    let out = if snapshot.track_frames == 0 {
        Some(loop_out)
    } else {
        compas_audio::recall_loop_end(stored.frame, loop_out, snapshot.track_frames)
    };
    let Some(out) = out else {
        return single(
            Some(EngineMsg::DeckSeek {
                deck,
                frame: stored.frame.max(0.0),
            }),
            false,
        );
    };
    snapshot.loop_in = stored.frame;
    snapshot.loop_out = out;
    snapshot.loop_active = true;
    snapshot.loop_armed = false;
    snapshot.loop_beats = None;
    let mut action = single(
        Some(EngineMsg::RecallLoop {
            deck,
            in_frame: stored.frame,
            out_frame: out,
        }),
        true,
    );
    action.messages.push(EngineMsg::DeckPlaying {
        deck,
        playing: true,
    });
    action.performance = Some(loop_mirror(snapshot));
    action
}

fn snapped(frame: f64, snapshot: &DeckSnapshot) -> f64 {
    if snapshot.grid_interval > 0.0 {
        snap_beat(frame, snapshot.grid_offset, snapshot.grid_interval)
    } else {
        frame
    }
}

fn beat_jump_action(
    deck: usize,
    beats: f64,
    from_frame: f64,
    performance: &PerformanceState,
) -> Option<NativeAction> {
    performance.with_deck(deck, |snapshot| {
        if snapshot.grid_interval <= 0.0 {
            return None;
        }
        let base = if snapshot.quantize {
            snap_beat(from_frame, snapshot.grid_offset, snapshot.grid_interval)
        } else {
            from_frame
        };
        let target = (base + beats * snapshot.jump_scale * snapshot.grid_interval).max(0.0);
        Some(single(
            Some(EngineMsg::DeckSeek {
                deck,
                frame: target,
            }),
            false,
        ))
    })?
}

/// SHIFT+platter. One detent is one bar (4 beats) times the beat-jump scale.
/// The origin always snaps to the beat grid. The seek does not change play or pause.
pub(crate) fn bar_jump(
    deck: usize,
    bars: i32,
    from_frame: f64,
    performance: &PerformanceState,
) -> Option<NativeAction> {
    if bars == 0 || !from_frame.is_finite() {
        return None;
    }
    performance.with_deck(deck, |snapshot| {
        if snapshot.grid_interval <= 0.0 {
            return None;
        }
        let base = snap_beat(from_frame, snapshot.grid_offset, snapshot.grid_interval).max(0.0);
        let beats = f64::from(bars) * 4.0 * snapshot.jump_scale;
        let target = (base + beats * snapshot.grid_interval).max(0.0);
        Some(single(
            Some(EngineMsg::DeckSeek {
                deck,
                frame: target,
            }),
            false,
        ))
    })?
}

pub(crate) fn memory_button(
    deck: usize,
    name: &str,
    frame: f64,
    heard: f64,
    loaded: bool,
    standby: Option<f64>,
    performance: &PerformanceState,
) -> NativeAction {
    let position = (frame.is_finite() && heard.is_finite()).then_some(CapturedPosition {
        frame,
        heard_frame: heard,
    });
    apply_performance(
        deck,
        name,
        1.0,
        loaded,
        position,
        &[1.0; NUM_DECKS],
        performance,
        standby.filter(|frame| frame.is_finite()),
    )
    .unwrap_or_else(|| single(None, false))
}

fn beat_loop_action(
    deck: usize,
    beats: f64,
    heard: f64,
    performance: &PerformanceState,
) -> Option<NativeAction> {
    performance.with_deck(deck, |snapshot| {
        if snapshot.grid_interval <= 0.0 {
            return None;
        }
        let same_length = snapshot.loop_active
            && snapshot
                .loop_beats
                .is_some_and(|stored| (stored - beats).abs() < 1e-9);
        if same_length {
            snapshot.loop_active = false;
            return Some(mirrored_loop(
                snapshot,
                Some(EngineMsg::LoopActive {
                    deck,
                    active: false,
                }),
            ));
        }
        let (in_frame, out_frame) =
            beat_loop_bounds(heard, beats, snapshot.grid_offset, snapshot.grid_interval);
        snapshot.loop_in = in_frame;
        snapshot.loop_out = out_frame;
        snapshot.loop_active = true;
        snapshot.loop_armed = false;
        snapshot.loop_beats = Some(beats);
        Some(mirrored_loop(
            snapshot,
            Some(EngineMsg::Loop {
                deck,
                in_frame,
                out_frame,
                active: true,
            }),
        ))
    })?
}

fn arm_loop(deck: usize, heard: f64, performance: &PerformanceState) -> Option<NativeAction> {
    performance.with_deck(deck, |snapshot| {
        snapshot.loop_in = heard.max(0.0);
        snapshot.loop_beats = None;
        snapshot.loop_armed = true;
        mirrored_loop(snapshot, None)
    })
}

fn finish_loop(deck: usize, heard: f64, performance: &PerformanceState) -> Option<NativeAction> {
    performance.with_deck(deck, |snapshot| {
        let out_frame = heard.max(0.0);
        if out_frame <= snapshot.loop_in {
            return single(None, false);
        }
        snapshot.loop_out = out_frame;
        snapshot.loop_active = true;
        snapshot.loop_armed = false;
        snapshot.loop_beats = None;
        mirrored_loop(
            snapshot,
            Some(EngineMsg::Loop {
                deck,
                in_frame: snapshot.loop_in,
                out_frame,
                active: true,
            }),
        )
    })
}

fn toggle_loop(deck: usize, performance: &PerformanceState) -> Option<NativeAction> {
    performance.with_deck(deck, |snapshot| {
        let message = if snapshot.loop_active {
            EngineMsg::LoopActive {
                deck,
                active: false,
            }
        } else {
            if snapshot.loop_armed {
                return single(None, false);
            }
            let Some(end) = compas_audio::recall_loop_end(
                snapshot.loop_in,
                snapshot.loop_out,
                snapshot.track_frames,
            ) else {
                return single(None, false);
            };
            snapshot.loop_out = end;
            EngineMsg::RecallLoop {
                deck,
                in_frame: snapshot.loop_in,
                out_frame: snapshot.loop_out,
            }
        };
        snapshot.loop_active = !snapshot.loop_active;
        snapshot.loop_armed = false;
        mirrored_loop(snapshot, Some(message))
    })
}

fn scale_loop_action(
    deck: usize,
    factor: f64,
    performance: &PerformanceState,
) -> Option<NativeAction> {
    performance.with_deck(deck, |snapshot| {
        let mut action = single(Some(EngineMsg::ScaleLoop { deck, factor }), false);
        if snapshot.loop_out > snapshot.loop_in {
            let length = ((snapshot.loop_out - snapshot.loop_in) * factor).max(LOOP_MIN_FRAMES);
            snapshot.loop_out = snapshot.loop_in + length;
            snapshot.loop_beats = snapshot.loop_beats.map(|beats| beats * factor);
            snapshot.loop_armed = false;
            action.mirror = true;
            action.performance = Some(loop_mirror(snapshot));
        }
        action
    })
}

fn loop_length_or_memory(
    deck: usize,
    direction: i32,
    factor: f64,
    heard: Option<f64>,
    performance: &PerformanceState,
) -> NativeAction {
    let looping = performance
        .with_deck(deck, |snapshot| snapshot.loop_active)
        .unwrap_or(false);
    if looping {
        scale_loop_action(deck, factor, performance).unwrap_or_else(|| single(None, false))
    } else {
        call_memory(deck, direction, heard, performance).unwrap_or_else(|| single(None, false))
    }
}

fn ordered_memory(snapshot: &DeckSnapshot) -> Vec<usize> {
    let mut slots = Vec::new();
    for (slot, region) in snapshot.memory.iter().enumerate() {
        if region.is_some() {
            slots.push(slot);
        }
    }
    slots.sort_by(|&left, &right| {
        let left_in = snapshot.memory[left]
            .map(|region| region.in_frame)
            .unwrap_or(0.0);
        let right_in = snapshot.memory[right]
            .map(|region| region.in_frame)
            .unwrap_or(0.0);
        left_in.total_cmp(&right_in).then(left.cmp(&right))
    });
    slots
}

fn memory_in(snapshot: &DeckSnapshot, slot: usize) -> f64 {
    snapshot
        .memory
        .get(slot)
        .and_then(|region| region.map(|region| region.in_frame))
        .unwrap_or(0.0)
}

fn nearest_memory(snapshot: &DeckSnapshot, frame: f64, loops_only: bool) -> Option<usize> {
    snapshot
        .memory
        .iter()
        .enumerate()
        .filter_map(|(slot, region)| {
            region.and_then(|region| {
                (!loops_only || !region.point).then_some((slot, region.in_frame))
            })
        })
        .min_by(|left, right| {
            (left.1 - frame)
                .abs()
                .total_cmp(&(right.1 - frame).abs())
                .then(left.0.cmp(&right.0))
        })
        .map(|(slot, _)| slot)
}

fn arm_message(deck: usize, snapshot: &DeckSnapshot) -> EngineMsg {
    if let Some(region) = snapshot
        .memory
        .iter()
        .flatten()
        .find(|region| region.active)
    {
        EngineMsg::ArmLoop {
            deck,
            in_frame: region.in_frame,
            out_frame: region.out_frame,
            enabled: true,
        }
    } else {
        EngineMsg::ArmLoop {
            deck,
            in_frame: 0.0,
            out_frame: 0.0,
            enabled: false,
        }
    }
}

fn clear_memory_slot(snapshot: &mut DeckSnapshot, slot: usize) {
    snapshot.memory[slot] = None;
    if snapshot.memory_selected == Some(slot) {
        snapshot.memory_selected = None;
    }
}

fn covering_hot_loop(snapshot: &DeckSnapshot, frame: f64) -> Option<(f64, f64)> {
    snapshot
        .hot_cues
        .iter()
        .flatten()
        .filter_map(|cue| {
            let out = hot_loop_out(*cue)?;
            let covers = frame >= cue.frame - SAME_LOOP_FRAMES && frame <= out + SAME_LOOP_FRAMES;
            covers.then_some((cue.frame, (cue.frame - frame).abs(), out))
        })
        .min_by(|left, right| left.1.total_cmp(&right.1).then(left.0.total_cmp(&right.0)))
        .map(|(in_frame, _, out_frame)| (in_frame, out_frame))
}

fn same_saved(region: MemoryLoop, in_frame: f64, out_frame: f64, point: bool) -> bool {
    if region.point != point {
        return false;
    }
    if point {
        return (region.in_frame - in_frame).abs() < SAME_LOOP_FRAMES;
    }
    same_loop(region.in_frame, region.out_frame, in_frame, out_frame)
}

fn save_memory(
    deck: usize,
    anchor: Option<f64>,
    standby: Option<f64>,
    performance: &PerformanceState,
) -> NativeAction {
    performance
        .with_deck(deck, |snapshot| {
            let transport = (snapshot.loop_out > snapshot.loop_in).then_some((
                snapshot.loop_in,
                snapshot.loop_out,
                false,
            ));
            let hot = anchor
                .or(standby)
                .filter(|frame| frame.is_finite())
                .and_then(|frame| covering_hot_loop(snapshot, frame))
                .map(|(in_frame, out_frame)| (in_frame, out_frame, false));
            let standby_cue = standby
                .filter(|frame| frame.is_finite())
                .map(|frame| (frame.max(0.0), frame.max(0.0), true));
            let Some((in_frame, out_frame, point)) = transport.or(hot).or(standby_cue) else {
                return single(None, false);
            };
            if let Some(slot) = snapshot
                .memory
                .iter()
                .enumerate()
                .find_map(|(slot, stored)| {
                    (*stored).and_then(|region| {
                        same_saved(region, in_frame, out_frame, point).then_some(slot)
                    })
                })
            {
                snapshot.memory_selected = Some(slot);
                return memory_action(snapshot, None);
            }
            let Some(slot) = snapshot.memory.iter().position(|region| region.is_none()) else {
                return single(None, false);
            };
            snapshot.memory[slot] = Some(MemoryLoop {
                in_frame,
                out_frame: if point { in_frame } else { out_frame },
                active: false,
                point,
            });
            snapshot.memory_selected = Some(slot);
            memory_action(snapshot, None)
        })
        .unwrap_or_else(|| single(None, false))
}

fn delete_memory(deck: usize, heard: Option<f64>, performance: &PerformanceState) -> NativeAction {
    performance
        .with_deck(deck, |snapshot| {
            let slot = snapshot
                .memory_selected
                .filter(|slot| {
                    snapshot
                        .memory
                        .get(*slot)
                        .is_some_and(|region| region.is_some())
                })
                .or_else(|| nearest_memory(snapshot, heard.unwrap_or(0.0), false));
            let Some(slot) = slot else {
                return single(None, false);
            };
            clear_memory_slot(snapshot, slot);
            memory_action(snapshot, Some(arm_message(deck, snapshot)))
        })
        .unwrap_or_else(|| single(None, false))
}

fn forget_current_memory(deck: usize, performance: &PerformanceState) -> NativeAction {
    performance
        .with_deck(deck, |snapshot| {
            let Some(slot) = snapshot
                .memory
                .iter()
                .enumerate()
                .find_map(|(slot, region)| {
                    (*region).and_then(|region| {
                        same_loop(
                            region.in_frame,
                            region.out_frame,
                            snapshot.loop_in,
                            snapshot.loop_out,
                        )
                        .then_some(slot)
                    })
                })
            else {
                return single(None, false);
            };
            clear_memory_slot(snapshot, slot);
            memory_action(snapshot, Some(arm_message(deck, snapshot)))
        })
        .unwrap_or_else(|| single(None, false))
}

fn toggle_active_memory(
    deck: usize,
    heard: Option<f64>,
    performance: &PerformanceState,
) -> NativeAction {
    performance
        .with_deck(deck, |snapshot| {
            let mut slot = nearest_memory(snapshot, heard.unwrap_or(snapshot.loop_in), true);
            if slot.is_none() && snapshot.loop_out > snapshot.loop_in {
                slot = snapshot.memory.iter().position(|region| region.is_none());
                if let Some(slot) = slot {
                    snapshot.memory[slot] = Some(MemoryLoop {
                        in_frame: snapshot.loop_in,
                        out_frame: snapshot.loop_out,
                        active: false,
                        point: false,
                    });
                }
            }
            let Some(slot) = slot else {
                return single(None, false);
            };
            let was_active = snapshot
                .memory
                .get(slot)
                .and_then(|region| *region)
                .is_some_and(|region| region.active);
            for (index, region) in snapshot.memory.iter_mut().enumerate() {
                if let Some(region) = region {
                    region.active = !was_active && index == slot;
                }
            }
            snapshot.memory_selected = Some(slot);
            memory_action(snapshot, Some(arm_message(deck, snapshot)))
        })
        .unwrap_or_else(|| single(None, false))
}

fn call_memory(
    deck: usize,
    direction: i32,
    heard: Option<f64>,
    performance: &PerformanceState,
) -> Option<NativeAction> {
    performance.with_deck(deck, |snapshot| {
        let ordered = ordered_memory(snapshot);
        if ordered.is_empty() {
            return None;
        }
        let index = if let Some(current) = snapshot
            .memory_selected
            .and_then(|selected| ordered.iter().position(|slot| *slot == selected))
        {
            (current as i32 + direction).rem_euclid(ordered.len() as i32) as usize
        } else {
            let heard = heard.unwrap_or(0.0);
            let at_or_before = ordered
                .iter()
                .rposition(|slot| memory_in(snapshot, *slot) <= heard);
            match (direction.signum(), at_or_before) {
                (1, Some(index)) => (index + 1) % ordered.len(),
                (-1, Some(index)) => index,
                (1, None) => 0,
                _ => ordered.len() - 1,
            }
        };
        let slot = ordered[index];
        let Some(region) = snapshot.memory[slot] else {
            return None;
        };
        snapshot.memory_selected = Some(slot);
        if region.point {
            let was_looping = snapshot.loop_active;
            snapshot.loop_active = false;
            let frame = region.in_frame.max(0.0);
            let mut action = memory_action(snapshot, Some(EngineMsg::DeckSeek { deck, frame }));
            action.messages.push(EngineMsg::SetCuePoint { deck, frame });
            if was_looping {
                action.messages.push(EngineMsg::LoopActive {
                    deck,
                    active: false,
                });
            }
            return Some(action);
        }
        snapshot.loop_in = region.in_frame;
        snapshot.loop_out = region.out_frame;
        snapshot.loop_active = true;
        snapshot.loop_armed = false;
        snapshot.loop_beats = None;
        Some(memory_action(
            snapshot,
            Some(EngineMsg::RecallLoop {
                deck,
                in_frame: region.in_frame,
                out_frame: region.out_frame,
            }),
        ))
    })?
}

fn reloop_stop(deck: usize, performance: &PerformanceState) -> Option<NativeAction> {
    performance.with_deck(deck, |snapshot| {
        if snapshot.loop_out <= snapshot.loop_in {
            return single(None, false);
        }
        snapshot.loop_active = true;
        snapshot.loop_armed = false;
        let mut action = mirrored_loop(snapshot, None);
        action.messages = vec![
            EngineMsg::DeckPlaying {
                deck,
                playing: false,
            },
            EngineMsg::Loop {
                deck,
                in_frame: snapshot.loop_in,
                out_frame: snapshot.loop_out,
                active: true,
            },
            EngineMsg::DeckSeek {
                deck,
                frame: snapshot.loop_in,
            },
        ];
        action
    })
}

fn nudge_loop(
    deck: usize,
    counts: f64,
    nudge_in: bool,
    performance: &PerformanceState,
) -> Option<NativeAction> {
    if counts == 0.0 || !counts.is_finite() {
        return None;
    }
    let ready = performance.with_deck(deck, |snapshot| {
        snapshot.source_rate > 0.0 && snapshot.loop_out > snapshot.loop_in
    })?;
    if !ready {
        return None;
    }
    performance.with_deck(deck, |snapshot| {
        let delta = counts * snapshot.source_rate * 0.01;
        // Legacy/restored/manual loops can be shorter than the adjustment minimum.
        // Restore the invariant before clamp: an out point below eight frames would
        // otherwise make its upper bound negative and panic the controller worker.
        snapshot.loop_out = snapshot.loop_out.max(snapshot.loop_in + LOOP_MIN_FRAMES);
        if nudge_in {
            snapshot.loop_in =
                (snapshot.loop_in + delta).clamp(0.0, snapshot.loop_out - LOOP_MIN_FRAMES);
        } else {
            snapshot.loop_out = (snapshot.loop_out + delta).max(snapshot.loop_in + LOOP_MIN_FRAMES);
        }
        snapshot.loop_beats = None;
        mirrored_loop(
            snapshot,
            Some(EngineMsg::Loop {
                deck,
                in_frame: snapshot.loop_in,
                out_frame: snapshot.loop_out,
                active: snapshot.loop_active,
            }),
        )
    })
}

fn sync_action(
    deck: usize,
    tempos: &[f64; NUM_DECKS],
    performance: &PerformanceState,
) -> Option<NativeAction> {
    let (synced, partner, own_bpm) = performance.with_deck(deck, |snapshot| {
        (snapshot.synced, snapshot.partner, snapshot.bpm)
    })?;
    if synced {
        performance.set_synced(deck, false);
        let mut action = single(Some(EngineMsg::DeckSync { deck, master: None }), true);
        action.performance = Some(PerformanceMirror {
            synced: Some(false),
            ..PerformanceMirror::default()
        });
        return Some(action);
    }
    let partner = partner?;
    if own_bpm <= 0.0 || partner >= NUM_DECKS {
        return None;
    }
    let partner_bpm = performance.with_deck(partner, |snapshot| snapshot.bpm)?;
    let partner_tempo = tempos[partner];
    if partner_bpm <= 0.0 || !partner_tempo.is_finite() || partner_tempo <= 0.0 {
        return None;
    }
    let ratio = partner_bpm * partner_tempo / own_bpm;
    if !ratio.is_finite() || ratio <= 0.0 {
        return None;
    }
    performance.set_synced(deck, true);
    let mut action = single(None, true);
    action.messages = vec![
        EngineMsg::SetDeckSyncMode { deck, mode: 0 },
        EngineMsg::DeckTempo { deck, ratio },
        EngineMsg::DeckSync {
            deck,
            master: Some(partner),
        },
    ];
    action.performance = Some(PerformanceMirror {
        synced: Some(true),
        ..PerformanceMirror::default()
    });
    action.extra_mirrors = vec![(format!("deck.{deck}.tempo"), (ratio - 1.0) * 100.0)];
    Some(action)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn resolved(control: &str, value: f64, eq: &DeckEqState) -> NativeAction {
        resolve(
            control,
            value,
            &DeckTelemetry::new(),
            eq,
            &PerformanceState::new(),
            None,
            0,
        )
        .expect("native control")
    }

    #[test]
    fn ui_owned_and_out_of_range_deck_controls_are_not_native() {
        let eq = DeckEqState::default();
        let telemetry = DeckTelemetry::new();
        for control in [
            "deck.0.hotcue.0",
            "deck.0.sync",
            "deck.0.gain",
            "mixer.crossfader",
            "deck.4.volume",
            "deck.x.volume",
        ] {
            assert!(
                resolve(
                    control,
                    1.0,
                    &telemetry,
                    &eq,
                    &PerformanceState::new(),
                    None,
                    0,
                )
                .is_none(),
                "{control}"
            );
        }
    }

    #[test]
    fn eq_db_rejects_non_finite_and_clamps_to_the_engine_range() {
        assert_eq!(clamp_eq_db(40.0), Some(26.0));
        assert_eq!(clamp_eq_db(-40.0), Some(-26.0));
        assert_eq!(clamp_eq_db(3.5), Some(3.5));
        assert!(clamp_eq_db(f32::NAN).is_none());
        assert!(clamp_eq_db(f32::INFINITY).is_none());
    }

    #[test]
    fn eq_band_updates_preserve_the_other_bands() {
        let eq = DeckEqState::default();
        eq.record(1, -3.0, 2.0, 4.0);
        let action = resolved("deck.1.eq.mid", -40.0, &eq);
        assert!(action.mirror);
        assert!(matches!(
            action.message,
            Some(EngineMsg::DeckEqBand { deck: 1, band: 1, db }) if db == -26.0
        ));
        assert_eq!(eq.with_band(1, 1, -26.0), Some([-3.0, -26.0, 4.0]));
    }

    #[test]
    fn continuous_controls_clamp_to_engine_ranges() {
        let eq = DeckEqState::default();
        assert!(matches!(
            resolved("deck.0.volume", 2.0, &eq).message,
            Some(EngineMsg::DeckGain { deck: 0, gain }) if gain == 1.0
        ));
        assert!(matches!(
            resolved("deck.0.trim", -1.0, &eq).message,
            Some(EngineMsg::DeckTrim { deck: 0, gain }) if gain == 0.0
        ));
        assert!(matches!(
            resolved("deck.2.tempo", 8.0, &eq).message,
            Some(EngineMsg::DeckTempo { deck: 2, ratio }) if (ratio - 1.08).abs() < 1e-12
        ));
    }

    #[test]
    fn invalid_values_are_consumed_without_engine_commands() {
        let eq = DeckEqState::default();
        let action = resolved("deck.0.volume", f64::NAN, &eq);
        assert!(action.message.is_none());
        assert!(!action.mirror);
    }

    #[test]
    fn transport_follows_press_edges_and_requires_a_loaded_deck() {
        let eq = DeckEqState::default();
        assert!(resolved("deck.0.play", 1.0, &eq).message.is_none());
        assert!(resolved("deck.0.play", 0.0, &eq).message.is_none());
        assert!(resolved("deck.0.start", 0.0, &eq).message.is_none());
        assert!(matches!(
            resolved("deck.0.start", 1.0, &eq).message,
            Some(EngineMsg::DeckSeek { deck: 0, frame }) if frame == 0.0
        ));
        for (value, expected) in [(1.0, true), (0.0, false)] {
            let action = resolved("deck.1.cue", value, &eq);
            assert!(!action.mirror);
            assert!(matches!(
                action.message,
                Some(EngineMsg::CueButton { deck: 1, pressed }) if pressed == expected
            ));
        }
    }

    #[test]
    fn filter_matches_the_ui_curve() {
        let off = filter_settings(0.01);
        assert!(matches!(off.mode, FilterMode::Off));
        let low_pass = filter_settings(-1.0);
        assert!(matches!(low_pass.mode, FilterMode::LowPass));
        assert!((low_pass.cutoff - 200.0).abs() < 0.01);
        assert!((low_pass.resonance - 1.9).abs() < 1e-6);
        let high_pass = filter_settings(1.0);
        assert!(matches!(high_pass.mode, FilterMode::HighPass));
        assert!((high_pass.cutoff - 4000.0).abs() < 0.01);
    }

    #[test]
    fn sampler_pads_trigger_and_stop_without_the_ui() {
        let eq = DeckEqState::default();
        let performance = PerformanceState::new();
        let telemetry = DeckTelemetry::new();
        let trigger = resolve(
            "sampler.3.trigger",
            1.0,
            &telemetry,
            &eq,
            &performance,
            None,
            0,
        )
        .expect("trigger");
        assert!(matches!(
            trigger.message,
            Some(EngineMsg::TriggerSample {
                slot: 3,
                velocity: 110
            })
        ));
        assert!(!trigger.mirror);
        for release_value in [0.0, 63.0 / 127.0, 0.49] {
            let release = resolve(
                "sampler.3.trigger",
                release_value,
                &telemetry,
                &eq,
                &performance,
                None,
                1u64 << 3,
            )
            .expect("release");
            assert!(release.message.is_none());
            assert!(release.ui_event.is_none());
            let shift_release = resolve(
                "sampler.3.stop",
                release_value,
                &telemetry,
                &eq,
                &performance,
                None,
                1u64 << 3,
            )
            .expect("shift release");
            assert!(shift_release.message.is_none());
            assert!(shift_release.ui_event.is_none());
        }
        assert!(matches!(
            resolve(
                "sampler.3.trigger",
                64.0 / 127.0,
                &telemetry,
                &eq,
                &performance,
                None,
                0,
            )
            .expect("midi 64")
            .message,
            Some(EngineMsg::TriggerSample { slot: 3, .. })
        ));
        let load = resolve(
            "sampler.1.stop",
            1.0,
            &telemetry,
            &eq,
            &performance,
            None,
            0,
        )
        .expect("load");
        assert!(load.message.is_none());
        assert_eq!(
            load.ui_event.as_ref().map(|(control, _)| control.as_str()),
            Some("sampler.load.1")
        );
        let stop = resolve(
            "sampler.1.stop",
            1.0,
            &telemetry,
            &eq,
            &performance,
            None,
            1u64 << 1,
        )
        .expect("stop");
        assert!(matches!(
            stop.message,
            Some(EngineMsg::StopSample { slot: 1 })
        ));
        let shift_release = resolve(
            "sampler.1.stop",
            0.0,
            &telemetry,
            &eq,
            &performance,
            None,
            0,
        )
        .expect("shift release");
        assert!(shift_release.message.is_none());
        assert!(shift_release.ui_event.is_none());
        assert!(resolve("sampler.gain", 0.5, &telemetry, &eq, &performance, None, 0).is_none());
        let deck_two = resolve(
            "sampler.8.trigger",
            1.0,
            &telemetry,
            &eq,
            &performance,
            None,
            0,
        )
        .expect("deck 2 slot");
        assert!(matches!(
            deck_two.message,
            Some(EngineMsg::TriggerSample { slot: 8, .. })
        ));
        assert!(resolve(
            "sampler.64.trigger",
            1.0,
            &telemetry,
            &eq,
            &performance,
            None,
            0
        )
        .is_none());
        let repeated = resolve(
            "sampler.8.trigger",
            1.0,
            &telemetry,
            &eq,
            &performance,
            None,
            1u64 << 8,
        )
        .expect("repeat");
        assert!(matches!(
            repeated.message,
            Some(EngineMsg::TriggerSample { slot: 8, .. })
        ));
        let bank_load = resolve(
            "sampler.63.stop",
            1.0,
            &telemetry,
            &eq,
            &performance,
            None,
            0,
        )
        .expect("bank load");
        assert_eq!(
            bank_load
                .ui_event
                .as_ref()
                .map(|(control, _)| control.as_str()),
            Some("sampler.load.63")
        );
        assert!(matches!(
            resolve(
                "sampler.63.stop",
                1.0,
                &telemetry,
                &eq,
                &performance,
                None,
                1u64 << 63,
            )
            .expect("bank stop")
            .message,
            Some(EngineMsg::StopSample { slot: 63 })
        ));
        assert!(resolve(
            "deck.0.pad_fx.0",
            1.0,
            &telemetry,
            &eq,
            &performance,
            None,
            0
        )
        .is_none());
        assert!(resolve(
            "deck.0.key_shift.0",
            1.0,
            &telemetry,
            &eq,
            &performance,
            None,
            0
        )
        .is_none());
    }

    #[test]
    fn position_capture_requires_a_loaded_deck() {
        assert!(capture_position("deck.0.hotcue.0", &DeckTelemetry::new()).is_none());
        assert!(capture_position("mixer.crossfader", &DeckTelemetry::new()).is_none());
    }

    fn at(frame: f64, heard: f64) -> CapturedPosition {
        CapturedPosition {
            frame,
            heard_frame: heard,
        }
    }

    fn perform(
        name: &str,
        position: Option<CapturedPosition>,
        performance: &PerformanceState,
    ) -> Option<NativeAction> {
        apply_performance(
            0,
            name,
            1.0,
            true,
            position,
            &[1.0; NUM_DECKS],
            performance,
            None,
        )
    }

    #[test]
    fn hot_cue_stores_the_heard_frame_and_later_seeks_there() {
        let performance = PerformanceState::new();
        let set = perform("hotcue.0", Some(at(20_000.0, 10_000.0)), &performance).expect("set");
        assert!(set.message.is_none());
        assert_eq!(
            set.performance.and_then(|mirror| mirror.hot_cue_frame),
            Some(10_000.0)
        );
        let jump = perform("hotcue.0", Some(at(50_000.0, 40_000.0)), &performance).expect("jump");
        assert!(matches!(
            jump.message,
            Some(EngineMsg::DeckSeek { deck: 0, frame }) if frame == 10_000.0
        ));
        assert!(!jump.mirror);
    }

    #[test]
    fn loop_hot_cue_stores_the_playing_loop_and_can_be_saved() {
        let performance = PerformanceState::new();
        performance.install_track(0, 0.0, 1_000.0, 48_000.0, 120.0, 200_000);
        performance.set_loop(0, 1_000.0, 5_000.0, true, false, Some(4.0));
        let set = perform("hotcue.1", Some(at(3_000.0, 2_500.0)), &performance).expect("set");
        let mirror = set.performance.expect("mirror");
        assert_eq!(mirror.hot_cue_frame, Some(1_000.0));
        assert_eq!(mirror.hot_cue_out, Some(5_000.0));
        performance.set_loop(0, 0.0, 0.0, false, false, None);
        let jump = perform("hotcue.1", Some(at(9_000.0, 8_000.0)), &performance).expect("jump");
        assert!(matches!(
            jump.message,
            Some(EngineMsg::RecallLoop { deck: 0, in_frame, out_frame })
                if in_frame == 1_000.0 && out_frame == 5_000.0
        ));
        assert!(jump.messages.iter().any(|message| matches!(
            message,
            EngineMsg::DeckPlaying {
                deck: 0,
                playing: true
            }
        )));
        performance.set_loop(0, 0.0, 0.0, false, false, None);
        let saved = perform("loop.memory_save", Some(at(2_000.0, 2_000.0)), &performance).unwrap();
        let points = memory_points_of(&saved);
        assert_eq!(points.len(), 1);
        assert!(!points[0].point);
        assert_eq!(points[0].in_frame, 1_000.0);
        assert_eq!(points[0].out_frame, 5_000.0);
    }

    #[test]
    fn cue_standby_saves_the_cue_point_without_a_loop() {
        let performance = PerformanceState::new();
        let saved = apply_performance(
            0,
            "loop.memory_save",
            1.0,
            true,
            Some(at(4_000.0, 4_000.0)),
            &[1.0; NUM_DECKS],
            &performance,
            Some(4_000.0),
        )
        .unwrap();
        let points = memory_points_of(&saved);
        assert_eq!(points.len(), 1);
        assert!(points[0].point);
        assert!(!points[0].active);
        assert_eq!(points[0].in_frame, 4_000.0);
        let duplicate = apply_performance(
            0,
            "loop.memory_save",
            1.0,
            true,
            Some(at(4_000.4, 4_000.4)),
            &[1.0; NUM_DECKS],
            &performance,
            Some(4_000.4),
        )
        .unwrap();
        assert_eq!(memory_points_of(&duplicate).len(), 1);
        performance.set_loop(0, 1_000.0, 5_000.0, false, false, None);
        let loop_instead = apply_performance(
            0,
            "loop.memory_save",
            1.0,
            true,
            Some(at(4_000.0, 4_000.0)),
            &[1.0; NUM_DECKS],
            &performance,
            Some(4_000.0),
        )
        .unwrap();
        let stored = memory_points_of(&loop_instead);
        assert!(stored
            .iter()
            .any(|point| !point.point && point.in_frame == 1_000.0));
        let called = perform("loop.half", Some(at(9_000.0, 8_000.0)), &performance).unwrap();
        assert!(matches!(
            called.message,
            Some(EngineMsg::DeckSeek { deck: 0, frame }) if frame == 4_000.0
        ));
        assert!(called.messages.iter().any(|message| matches!(
            message,
            EngineMsg::SetCuePoint { deck: 0, frame } if *frame == 4_000.0
        )));
        assert!(called
            .messages
            .iter()
            .all(|message| !matches!(message, EngineMsg::RecallLoop { .. })));
    }

    #[test]
    fn keyboard_selects_a_hot_cue_then_plays_it_at_the_pad_pitch() {
        let performance = PerformanceState::new();
        let select = performance
            .keyboard(0, 2, false, true, Some(12_000.0), 0.0)
            .expect("select");
        assert_eq!(select.mirror_as.as_deref(), Some("deck.0.hotcue.2"));
        assert_eq!(
            select.performance.and_then(|mirror| mirror.hot_cue_frame),
            Some(12_000.0)
        );
        assert!(select.pitch.is_none());
        let play = performance
            .keyboard(0, 0, false, true, None, 1.0)
            .expect("play");
        assert!(matches!(
            play.message,
            Some(EngineMsg::DeckSeek { deck: 0, frame }) if frame == 12_000.0
        ));
        assert!(matches!(
            play.messages.first(),
            Some(EngineMsg::DeckPlaying {
                deck: 0,
                playing: true
            })
        ));
        assert_eq!(play.pitch, Some(4.0));
        let again = performance
            .keyboard(0, 0, false, true, None, 1.0)
            .expect("repeat press");
        assert!(again.message.is_none());
        assert!(again.pitch.is_none());
        let release = performance
            .keyboard(0, 0, false, false, None, 1.0)
            .expect("release");
        assert_eq!(release.pitch, Some(1.0));
        performance
            .keyboard(0, 5, false, true, None, 1.0)
            .expect("second pad");
        let transposed = performance
            .keyboard(0, 6, true, true, None, 1.0)
            .expect("raise");
        assert_eq!(transposed.pitch, Some(2.0));
        let deleted = performance
            .keyboard(0, 0, true, true, None, 1.0)
            .expect("delete");
        assert_eq!(deleted.mirror_as.as_deref(), Some("deck.0.clear_hotcue.2"));
        assert_eq!(deleted.pitch, Some(1.0));
        assert_eq!(performance.reset_keyboard(0), None);
    }

    #[test]
    fn keyboard_holds_release_quantize_and_decks_stay_independent() {
        let performance = PerformanceState::new();
        performance.set_hot_cue(0, 1, Some(4_000.0));
        let deleted = performance
            .keyboard(0, 1, true, true, None, 0.0)
            .expect("delete before selection");
        assert_eq!(deleted.mirror_as.as_deref(), Some("deck.0.clear_hotcue.1"));
        performance.install_track(0, 0.0, 48_000.0, 48_000.0, 120.0, 200_000);
        performance.set_grid(0, 0.0, 48_000.0);
        performance.set_quantize(0, true);
        performance.set_hot_cue(0, 3, Some(1_000.0));
        performance
            .keyboard(0, 3, false, true, None, 0.0)
            .expect("select");
        let play = performance
            .keyboard(0, 0, false, true, None, 0.0)
            .expect("quantized play");
        assert!(matches!(
            play.message,
            Some(EngineMsg::DeckSeek { frame, .. }) if frame == 0.0
        ));
        performance
            .keyboard(0, 4, false, true, None, 0.0)
            .expect("second hold");
        let one_up = performance
            .keyboard(0, 0, false, false, None, 0.0)
            .expect("release one");
        assert!(one_up.pitch.is_none());
        let both_up = performance
            .keyboard(0, 4, false, false, None, 0.0)
            .expect("release last");
        assert_eq!(both_up.pitch, Some(0.0));
        performance
            .keyboard(0, 0, false, true, None, 0.0)
            .expect("hold for transpose");
        let mut raised = None;
        for _ in 0..13 {
            raised = performance
                .keyboard(0, 6, true, true, None, 0.0)
                .expect("raise")
                .pitch;
        }
        assert_eq!(raised, Some(12.0));
        let other = performance
            .keyboard(1, 0, false, true, Some(8.0), 3.0)
            .expect("other deck selects");
        assert_eq!(other.mirror_as.as_deref(), Some("deck.1.hotcue.0"));
        assert_eq!(performance.reset_keyboard(0), Some(0.0));
        assert_eq!(performance.reset_keyboard(1), None);
    }

    #[test]
    fn quantize_seeks_a_stored_hot_cue_to_the_nearest_beat() {
        let performance = PerformanceState::new();
        performance.set_grid(0, 0.0, 22_050.0);
        performance.set_quantize(0, true);
        performance.set_hot_cue(0, 0, Some(10_000.0));
        let jump = perform("hotcue.0", Some(at(80_000.0, 70_000.0)), &performance).expect("jump");
        assert!(matches!(
            jump.message,
            Some(EngineMsg::DeckSeek { deck: 0, frame }) if frame == 0.0
        ));
    }

    #[test]
    fn beat_jump_uses_the_press_frame_and_the_current_scale() {
        let performance = PerformanceState::new();
        performance.set_grid(0, 0.0, 22_050.0);
        performance.set_jump_scale(0, 2.0);
        let jump = perform("beatjump.5", Some(at(30_000.0, 20_000.0)), &performance).expect("jump");
        // Pad 5 is +4 beats, doubled by the scale, from the read frame rather than the heard frame.
        assert!(matches!(
            jump.message,
            Some(EngineMsg::DeckSeek { deck: 0, frame }) if (frame - 206_400.0).abs() < 1e-6
        ));
        performance.set_quantize(0, true);
        let quantized =
            perform("jump.forward", Some(at(30_000.0, 20_000.0)), &performance).expect("quantized");
        // Nearest beat to 30_000 is 22_050, then +4 beats at scale 2.
        assert!(matches!(
            quantized.message,
            Some(EngineMsg::DeckSeek { deck: 0, frame }) if (frame - 198_450.0).abs() < 1e-6
        ));
    }

    #[test]
    fn bar_jump_snaps_to_the_grid_for_one_bar_per_detent() {
        let performance = PerformanceState::new();
        performance.set_grid(0, 0.0, 22_050.0);
        let forward = bar_jump(0, 1, 30_000.0, &performance).expect("forward");
        assert!(matches!(
            forward.message,
            Some(EngineMsg::DeckSeek { deck: 0, frame }) if (frame - 110_250.0).abs() < 1e-6
        ));
        assert!(forward.messages.is_empty());
        let back = bar_jump(0, -1, 110_250.0, &performance).expect("back");
        assert!(matches!(
            back.message,
            Some(EngineMsg::DeckSeek { deck: 0, frame }) if (frame - 22_050.0).abs() < 1e-6
        ));
        performance.set_jump_scale(0, 2.0);
        let scaled = bar_jump(0, 1, 30_000.0, &performance).expect("scaled");
        assert!(matches!(
            scaled.message,
            Some(EngineMsg::DeckSeek { deck: 0, frame }) if (frame - 198_450.0).abs() < 1e-6
        ));
        performance.set_quantize(0, false);
        let still_snapped = bar_jump(0, 1, 30_000.0, &performance).expect("grid");
        assert!(matches!(
            still_snapped.message,
            Some(EngineMsg::DeckSeek { deck: 0, frame }) if (frame - 198_450.0).abs() < 1e-6
        ));
        performance.set_grid(0, 0.0, 0.0);
        assert!(bar_jump(0, 1, 30_000.0, &performance).is_none());
        assert!(bar_jump(0, 0, 30_000.0, &performance).is_none());
    }

    fn memory_points_of(action: &NativeAction) -> Vec<MemoryPoint> {
        action
            .performance
            .as_ref()
            .and_then(|mirror| mirror.memory.clone())
            .unwrap_or_default()
    }

    #[test]
    fn memory_loop_saves_calls_and_keeps_a_single_active_loop() {
        let performance = PerformanceState::new();
        performance.install_track(0, 0.0, 1_000.0, 48_000.0, 120.0, 20_000);
        performance.set_loop(0, 1_000.0, 5_000.0, true, false, Some(4.0));
        let saved = perform("loop.memory_save", Some(at(2_000.0, 1_500.0)), &performance).unwrap();
        let points = memory_points_of(&saved);
        assert_eq!(points.len(), 1);
        assert!(!points[0].active);
        assert!(saved.message.is_none());
        let duplicate =
            perform("loop.memory_save", Some(at(2_000.0, 1_500.0)), &performance).unwrap();
        assert_eq!(memory_points_of(&duplicate).len(), 1);
        performance.set_loop(0, 1_000.0, 5_000.0, false, false, Some(4.0));
        let called = perform("loop.half", Some(at(8_000.0, 7_000.0)), &performance).unwrap();
        assert!(matches!(
            called.message,
            Some(EngineMsg::RecallLoop { deck: 0, in_frame, out_frame })
                if in_frame == 1_000.0 && out_frame == 5_000.0
        ));
        performance.set_loop(0, 1_000.0, 5_000.0, true, false, Some(4.0));
        let scaled = perform("loop.half", Some(at(2_000.0, 1_500.0)), &performance).unwrap();
        assert!(
            matches!(scaled.message, Some(EngineMsg::ScaleLoop { deck: 0, factor }) if factor == 0.5)
        );
        performance.set_loop(0, 6_000.0, 9_000.0, false, false, None);
        perform("loop.memory_save", Some(at(7_000.0, 6_500.0)), &performance).unwrap();
        let active = perform(
            "loop.memory_active",
            Some(at(6_200.0, 6_100.0)),
            &performance,
        )
        .unwrap();
        let armed = memory_points_of(&active);
        assert_eq!(armed.iter().filter(|point| point.active).count(), 1);
        assert!(matches!(
            active.message,
            Some(EngineMsg::ArmLoop { deck: 0, enabled: true, in_frame, .. }) if in_frame == 6_000.0
        ));
        let cleared = perform(
            "loop.memory_active",
            Some(at(6_200.0, 6_100.0)),
            &performance,
        )
        .unwrap();
        assert!(memory_points_of(&cleared).iter().all(|point| !point.active));
        assert!(matches!(
            cleared.message,
            Some(EngineMsg::ArmLoop {
                deck: 0,
                enabled: false,
                ..
            })
        ));
        let removed = perform(
            "loop.memory_delete",
            Some(at(6_200.0, 6_100.0)),
            &performance,
        )
        .unwrap();
        assert_eq!(memory_points_of(&removed).len(), 1);
    }

    #[test]
    fn beat_loop_engages_from_the_heard_frame_and_toggles_off() {
        let performance = PerformanceState::new();
        performance.set_grid(0, 0.0, 22_050.0);
        let on = perform("beatloop.4", Some(at(40_000.0, 30_000.0)), &performance).expect("on");
        assert!(matches!(
            on.message,
            Some(EngineMsg::Loop { deck: 0, in_frame, out_frame, active: true })
                if (in_frame - 22_050.0).abs() < 1e-6 && (out_frame - 110_250.0).abs() < 1e-6
        ));
        let off = perform("beatloop.4", Some(at(50_000.0, 40_000.0)), &performance).expect("off");
        assert!(matches!(
            off.message,
            Some(EngineMsg::LoopActive {
                deck: 0,
                active: false
            })
        ));
    }

    #[test]
    fn flx4_loop_toggle_recalls_saved_bounds_and_exits_without_seek() {
        let performance = PerformanceState::new();
        performance.install_track(0, 0.0, 0.0, 48_000.0, 0.0, 16);
        assert!(perform("loop.toggle", None, &performance)
            .unwrap()
            .message
            .is_none());
        performance.set_loop(0, 8.0, 16.0, false, false, None);
        let reloop = perform("loop.toggle", None, &performance).unwrap();
        assert!(
            matches!(reloop.message, Some(EngineMsg::RecallLoop { deck: 0, in_frame, out_frame }) if in_frame == 8.0 && out_frame == 16.0)
        );
        assert_eq!(reloop.performance.unwrap().loop_active, Some(true));
        let exit = perform("loop.toggle", None, &performance).unwrap();
        assert!(matches!(
            exit.message,
            Some(EngineMsg::LoopActive {
                deck: 0,
                active: false
            })
        ));
        let state = exit.performance.unwrap();
        assert_eq!(
            (state.loop_in, state.loop_out, state.loop_active),
            (Some(8.0), Some(16.0), Some(false))
        );
    }

    #[test]
    fn loop_out_uses_the_armed_in_point_and_ignores_a_backward_out() {
        let performance = PerformanceState::new();
        let armed = perform("loop.in", Some(at(2_000.0, 1_000.0)), &performance).expect("in");
        assert!(armed.message.is_none());
        assert!(armed
            .performance
            .as_ref()
            .is_some_and(|mirror| mirror.loop_armed == Some(true)));
        let ignored = perform("loop.out", Some(at(500.0, 400.0)), &performance).expect("early");
        assert!(ignored.message.is_none());
        let engaged = perform("loop.out", Some(at(6_000.0, 5_000.0)), &performance).expect("out");
        assert!(matches!(
            engaged.message,
            Some(EngineMsg::Loop { deck: 0, in_frame, out_frame, active: true })
                if in_frame == 1_000.0 && out_frame == 5_000.0
        ));
    }

    #[test]
    fn sync_does_not_arm_without_a_tempo() {
        let performance = PerformanceState::new();
        performance.install_track(0, 0.0, 0.0, 44_100.0, 0.0, 1_000);
        performance.set_partner(0, Some(1));
        let tempos = [1.0; NUM_DECKS];
        assert!(
            apply_performance(0, "sync", 1.0, true, None, &tempos, &performance, None).is_none()
        );
        assert!(!performance
            .with_deck(0, |snapshot| snapshot.synced)
            .unwrap());
    }

    #[test]
    fn analysis_updates_the_grid_without_clearing_cues_or_reviving_an_empty_deck() {
        let performance = PerformanceState::new();
        assert!(!performance.publish_analysis(0, 10.0, 100.0, 120.0));
        assert!(!performance.has_track(0));
        performance.install_track(0, 0.0, 0.0, 44_100.0, 0.0, 4_000);
        performance.set_hot_cue(0, 3, Some(250.0));
        performance.set_loop(0, 10.0, 40.0, false, true, None);
        assert!(performance.publish_analysis(0, 12.0, 22_050.0, 120.0));
        let (bpm, cue, armed, synced) = performance
            .with_deck(0, |snapshot| {
                (
                    snapshot.bpm,
                    snapshot.hot_cues[3],
                    snapshot.loop_armed,
                    snapshot.synced,
                )
            })
            .unwrap();
        assert_eq!(bpm, 120.0);
        assert_eq!(cue.map(|cue| cue.frame), Some(250.0));
        assert!(armed);
        assert!(!synced);
        performance.clear_deck(0);
        assert!(!performance.publish_analysis(0, 12.0, 22_050.0, 120.0));
        assert_eq!(
            performance.with_deck(0, |snapshot| snapshot.bpm).unwrap(),
            0.0
        );
    }

    #[test]
    fn sync_matches_the_partner_tempo_immediately_and_disengages() {
        let performance = PerformanceState::new();
        performance.with_deck(0, |snapshot| snapshot.bpm = 100.0);
        performance.with_deck(1, |snapshot| snapshot.bpm = 128.0);
        performance.set_partner(0, Some(1));
        let mut tempos = [1.0; NUM_DECKS];
        tempos[1] = 1.05;
        let on = apply_performance(0, "sync", 1.0, true, None, &tempos, &performance, None)
            .expect("sync");
        assert!(on.messages.iter().any(|message| matches!(
            message,
            EngineMsg::DeckTempo { deck: 0, ratio } if (ratio - 1.344).abs() < 1e-9
        )));
        assert!(on.messages.iter().any(|message| matches!(
            message,
            EngineMsg::DeckSync {
                deck: 0,
                master: Some(1)
            }
        )));
        assert_eq!(on.extra_mirrors.len(), 1);
        assert_eq!(on.extra_mirrors[0].0, "deck.0.tempo");
        assert!((on.extra_mirrors[0].1 - 34.4).abs() < 1e-9);
        let off = apply_performance(0, "sync", 1.0, true, None, &tempos, &performance, None)
            .expect("off");
        assert!(matches!(
            off.message,
            Some(EngineMsg::DeckSync {
                deck: 0,
                master: None
            })
        ));
        assert!(apply_performance(
            0,
            "sync",
            1.0,
            true,
            None,
            &tempos,
            &PerformanceState::new(),
            None,
        )
        .is_none());
    }
    #[test]
    fn nudging_tiny_loop_near_start_preserves_valid_bounds() {
        for nudge_in in [true, false] {
            for counts in [-1.0, 1.0] {
                let performance = PerformanceState::new();
                performance.with_deck(0, |snapshot| {
                    snapshot.source_rate = 48_000.0;
                    snapshot.loop_in = 0.0;
                    snapshot.loop_out = 4.0;
                    snapshot.loop_active = true;
                });
                let action = nudge_loop(0, counts, nudge_in, &performance).expect("nudge");
                assert!(matches!(action.message,
                    Some(EngineMsg::Loop { deck: 0, in_frame, out_frame, active: true })
                        if in_frame >= 0.0 && out_frame - in_frame >= LOOP_MIN_FRAMES));
            }
        }
    }
}

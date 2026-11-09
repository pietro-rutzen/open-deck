//! Controller-profile storage: load/list/save `ControllerProfile` JSON files from the user's
//! controller directory (and, later, a bundled read-only dir). See `docs/CONTROLLER-ARCHITECTURE.md`.
//!
//! A profile is pure data (bindings + optional script) — adding a controller is dropping a file
//! here, no recompile. This module is plain blocking I/O, called only from Tauri commands.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Sender};
use std::sync::Arc;

use compas_audio::DeckTelemetry;
use compas_core::{Behavior, ControllerProfile, InputKind, Mapping, Registry};
use compas_script::ScriptRuntime;
use midir::{MidiOutput, MidiOutputConnection};
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager};

fn flx4_replaced_default(saved: &str, bundled: &str) -> bool {
    matches!(
        (saved, bundled),
        ("deck.0.jump.forward", "deck.0.loop.memory_save")
            | ("deck.0.jump.back", "deck.0.loop.memory_delete")
            | ("deck.0.loop.reloop_stop", "deck.0.loop.memory_active")
            | ("deck.1.jump.forward", "deck.1.loop.memory_save")
            | ("deck.1.jump.back", "deck.1.loop.memory_delete")
            | ("deck.1.loop.reloop_stop", "deck.1.loop.memory_active")
    )
}

/// Extend the original built-in snapshot without replacing user-remapped addresses.
pub(crate) fn upgrade_flx4_profile(mut profile: ControllerProfile) -> ControllerProfile {
    if profile.id == "pioneer-ddj-flx4" && profile.script.is_none() {
        if let Ok(bundled) = serde_json::from_str::<ControllerProfile>(include_str!(
            "../resources/controllers/pioneer-ddj-flx4.json"
        )) {
            for binding in bundled.bindings {
                if let Some(old) = profile
                    .bindings
                    .iter_mut()
                    .find(|old| old.channel == binding.channel && old.input == binding.input)
                {
                    if old.control.as_str().starts_with("sampler.")
                        && binding.control.as_str().starts_with("sampler.")
                        && old.control.as_str() != binding.control.as_str()
                    {
                        old.control = binding.control;
                    } else if flx4_replaced_default(old.control.as_str(), binding.control.as_str())
                    {
                        old.control = binding.control;
                    }
                } else {
                    profile.bindings.push(binding);
                }
            }
        }
    }
    profile
}

fn flx4_cc_pair(
    state: &mut [[u16; 32]; 16],
    status: u8,
    number: u8,
    value: u8,
) -> Option<(u8, u8, f64)> {
    if status & 0xf0 != 0xb0 || value > 127 {
        return None;
    }
    let ch = status & 15;
    let base = number & 31;
    let paired = number < 64
        && ((ch < 2 && matches!(base, 0 | 4 | 7 | 11 | 15 | 19))
            || (ch == 4 && base == 2)
            || (ch == 6 && matches!(base, 8 | 12 | 13 | 23 | 24 | 31)));
    if !paired {
        return None;
    }
    let completing = number >= 32;
    let slot = &mut state[ch as usize][base as usize];
    *slot = if completing {
        (*slot & 16256) | value as u16
    } else {
        ((value as u16) << 7) | (*slot & 127)
    };
    // MSB only stores the high bits. Applying it before the LSB arrives jumps by up to one coarse step.
    completing.then_some((ch, base, *slot as f64 / 16383.0))
}

/// Number of decks the control registry is built for (matches the engine).
const DECK_COUNT: usize = 4;
/// Soft-takeover pickup threshold in normalized units (~4 MIDI steps).
const TAKEOVER: f64 = 0.03;

/// The user controller directory (`<app-data>/controllers`), created if missing.
pub fn profiles_dir(base: &Path) -> std::io::Result<PathBuf> {
    let dir = base.join("controllers");
    fs::create_dir_all(&dir)?;
    Ok(dir)
}

/// Load and parse every `*.json` profile in `dir`, skipping any that fail to parse.
pub fn list_profiles(dir: &Path) -> Vec<ControllerProfile> {
    let mut out = Vec::new();
    let Ok(entries) = fs::read_dir(dir) else {
        return out;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) == Some("json") {
            if let Ok(p) = load_profile(&path) {
                out.push(p);
            }
        }
    }
    out.sort_by_key(|p| p.name.to_lowercase());
    out
}

/// List profiles from a bundled (read-only) dir and the user dir, merged by id — a user profile
/// overrides a bundled one with the same id. Sorted by name.
pub fn list_merged(bundled: Option<&Path>, user: &Path) -> Vec<ControllerProfile> {
    use std::collections::BTreeMap;
    let mut by_id: BTreeMap<String, ControllerProfile> = BTreeMap::new();
    if let Some(b) = bundled {
        for p in list_profiles(b) {
            by_id.insert(p.id.clone(), p);
        }
    }
    for p in list_profiles(user) {
        by_id.insert(p.id.clone(), p); // user overrides bundled
    }
    let mut out: Vec<_> = by_id.into_values().collect();
    out.sort_by_key(|p| p.name.to_lowercase());
    out
}

/// Load a single profile file.
pub fn load_profile(path: &Path) -> Result<ControllerProfile, String> {
    let text = fs::read_to_string(path).map_err(|e| e.to_string())?;
    serde_json::from_str(&text).map_err(|e| e.to_string())
}

/// Save (or overwrite) a profile as `<dir>/<id>.json`. Used by the guided learn editor.
pub fn save_profile(dir: &Path, profile: &ControllerProfile) -> Result<PathBuf, String> {
    let safe: String = profile
        .id
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect();
    let id = if safe.is_empty() {
        "profile".to_string()
    } else {
        safe
    };
    let path = dir.join(format!("{id}.json"));
    let json = serde_json::to_string_pretty(profile).map_err(|e| e.to_string())?;
    replace_file(&path, json.as_bytes())?;
    Ok(path)
}

/// Write `bytes` via a temporary file and rename it into place, so a crash cannot
/// leave a truncated profile where a complete one used to be.
fn replace_file(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or("Controller profile path has no directory")?;
    fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos())
        .unwrap_or(0);
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("profile");
    let temporary = parent.join(format!(".{file_name}-{}-{nonce}.tmp", std::process::id()));
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)
        .map_err(|error| error.to_string())?;
    let result = (|| {
        use std::io::Write;
        file.write_all(bytes)?;
        file.sync_all()?;
        fs::rename(&temporary, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result.map_err(|error: std::io::Error| error.to_string())
}

// ---------------------------------------------------------------------------------
// Profile packs — a shareable bundle of one or more `ControllerProfile`s (device maps,
// scripts inline) as a single JSON file, for distributing controller mappings.
// ---------------------------------------------------------------------------------

/// Profile-pack schema version. Bump on any incompatible shape change; [`import_pack`] checks it.
pub const PACK_VERSION: u32 = 1;

/// A portable bundle of controller profiles.
#[derive(Serialize, Deserialize, Clone)]
pub struct ProfilePack {
    pub version: u32,
    pub app: String,
    pub profiles: Vec<ControllerProfile>,
}

/// Bundle profiles into a shareable pack (stamped with the schema version + app identity).
pub fn pack_profiles(profiles: Vec<ControllerProfile>) -> ProfilePack {
    ProfilePack {
        version: PACK_VERSION,
        app: crate::export::MANIFEST_APP.to_string(),
        profiles,
    }
}

/// Save every profile in a pack into the user controller `dir` (each as `<id>.json`, overwriting an
/// existing profile with the same id), returning the imported ids. Rejects an unknown pack version.
pub fn import_pack(dir: &Path, pack: &ProfilePack) -> Result<Vec<String>, String> {
    if pack.version != PACK_VERSION {
        return Err(format!(
            "unsupported profile-pack version {} (expected {PACK_VERSION})",
            pack.version
        ));
    }
    let mut ids = Vec::with_capacity(pack.profiles.len());
    for profile in &pack.profiles {
        save_profile(dir, profile)?;
        ids.push(profile.id.clone());
    }
    Ok(ids)
}

/// A resolved control change pushed to the frontend, which applies it through the deck/mixer setters.
#[derive(Serialize, Clone)]
pub struct ControllerUpdateEvent {
    /// Control-bus id, e.g. `"deck.0.gain"`.
    pub control: String,
    /// Engine-domain value (the unit the matching UI setter expects).
    pub value: f64,
    /// Engine read position when the controller message arrived (deck-scoped actions only).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub frame: Option<f64>,
    /// Audible position when the controller message arrived (read position minus output latency).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub heard_frame: Option<f64>,
    /// Loop, cue, quantize, sync, or beat-jump display state. Present only after the engine applied it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub performance: Option<crate::native_controls::PerformanceMirror>,
}

impl ControllerUpdateEvent {
    fn new(control: impl Into<String>, value: f64) -> Self {
        Self {
            control: control.into(),
            value,
            frame: None,
            heard_frame: None,
            performance: None,
        }
    }
}

/// Continuous controller changes coalesced to display rate.
#[derive(Serialize, Clone, Default)]
struct ControllerBatchEvent {
    /// Already applied to the engine natively; the UI reflects them without re-sending.
    mirror: Vec<ControllerUpdateEvent>,
    /// Continuous controls the UI still applies through its setters.
    updates: Vec<ControllerUpdateEvent>,
}

fn upsert_latest(pending: &mut Vec<ControllerUpdateEvent>, update: ControllerUpdateEvent) {
    match pending
        .iter_mut()
        .find(|queued| queued.control == update.control)
    {
        Some(queued) => *queued = update,
        None => pending.push(update),
    }
}

#[derive(Serialize, Clone)]
struct MidiCcEvent {
    channel: u8,
    controller: u8,
    value: u8,
    consumed: bool,
}

#[derive(Serialize, Clone)]
struct MidiNoteEvent {
    channel: u8,
    note: u8,
    velocity: u8,
    on: bool,
    consumed: bool,
}

/// Raw input for MIDI learn and the legacy mapping layer.
fn emit_raw_midi(app: &AppHandle, status: u8, data1: u8, data2: u8, consumed: bool) {
    let channel = status & 0x0F;
    match status & 0xF0 {
        0x90 | 0x80 => {
            let velocity = if status & 0xF0 == 0x90 { data2 } else { 0 };
            let _ = app.emit(
                "midi:note",
                MidiNoteEvent {
                    channel,
                    note: data1,
                    velocity,
                    on: velocity > 0,
                    consumed,
                },
            );
        }
        0xB0 => {
            let _ = app.emit(
                "midi:cc",
                MidiCcEvent {
                    channel,
                    controller: data1,
                    value: data2,
                    consumed,
                },
            );
        }
        _ => {}
    }
}

/// Display-rate flush period for coalesced continuous controller changes.
const UI_FLUSH: Duration = Duration::from_millis(16);
const METER_PERIOD: Duration = Duration::from_millis(33);
const KEEP_ALIVE: Duration = Duration::from_millis(200);
/// Holding FLX4 LOOP IN this long engages a four-beat loop (the "4 BEAT" hardware label).
const LOOP_IN_HOLD: Duration = Duration::from_millis(500);
const FOUR_BEAT_LOOP_PAD: usize = 4;
const TRANSPORT_BLINK_MS: u128 = 500;
const FLX4_PLAY_NOTE: u8 = 0x0B;
const FLX4_CUE_NOTE: u8 = 0x0C;
const FLX4_LOOP_IN_NOTE: u8 = 0x10;
const FLX4_SHIFT_NOTE: u8 = 0x3F;
const FLX4_LOOP_IN_ADJUST_NOTE: u8 = 0x4C;
const FLX4_LOOP_OUT_ADJUST_NOTE: u8 = 0x4E;
const FLX4_TEMPO_RANGE_NOTE: u8 = 0x60;
const FLX4_TRACK_LOADED_NOTES: [u8; 2] = [0x26, 0x27];
/// Hardware tempo-fader spans. ±8% is the control-bus default, so it stays first until SHIFT+SYNC.
const FLX4_TEMPO_RANGES: [f64; 5] = [8.0, 6.0, 10.0, 16.0, 100.0];
/// Pad-mode buttons from the AlphaTheta DDJ-FLX4 MIDI message list J1, section 5.
/// `0x20` is beat jump. Pad FX1 is `0x1E` and Pad FX2 is `0x6B`.
const FLX4_PAD_MODE_NOTES: [u8; 8] = [0x1B, 0x1E, 0x20, 0x22, 0x69, 0x6B, 0x6D, 0x6F];
const FLX4_PAD_FX1_NOTE: u8 = 0x1E;
const FLX4_PAD_FX2_NOTE: u8 = 0x6B;
const FLX4_KEYBOARD_NOTE: u8 = 0x69;
const FLX4_BROWSE_CC: u8 = 0x40;
const FLX4_ZOOM_CC: u8 = 0x64;
const FLX4_BROWSE_PRESS_NOTES: [u8; 2] = [0x41, 0x42];
const FLX4_LOAD_NOTES: [u8; 2] = [0x46, 0x47];

// Stateless press decoding: PFL, transport and prior releases cannot latch LOAD off.
fn flx4_load_pressed(status: u8, note: u8, velocity: u8) -> Option<usize> {
    (status == 0x96 && velocity > 0)
        .then(|| {
            FLX4_LOAD_NOTES
                .iter()
                .position(|candidate| *candidate == note)
        })
        .flatten()
}

struct LoopInHold {
    since: Instant,
    heard_frame: Option<f64>,
    fired: bool,
}

/// A physical button may disappear before its release arrives. Track only the two
/// native deck actions whose effect lasts until release, never latched headphone PFL.
#[derive(Default)]
struct HeldDeckButtons {
    cue: [bool; DECK_COUNT],
    censor: [bool; DECK_COUNT],
}

impl HeldDeckButtons {
    fn send(
        &mut self,
        tx: &crate::engine_commands::Sender<crate::EngineMsg>,
        message: crate::EngineMsg,
    ) -> Result<(), String> {
        use crate::EngineMsg;
        let edge = match &message {
            EngineMsg::CueButton { deck, pressed } => {
                self.cue.get_mut(*deck).map(|v| (v, *pressed))
            }
            EngineMsg::SetCensor { deck, active } => {
                self.censor.get_mut(*deck).map(|v| (v, *active))
            }
            _ => None,
        };
        tx.send(message)?;
        if let Some((held, pressed)) = edge {
            *held = pressed;
        }
        Ok(())
    }

    fn release(
        &mut self,
        tx: &crate::engine_commands::Sender<crate::EngineMsg>,
    ) -> Result<(), String> {
        let mut releases = Vec::new();
        for deck in 0..DECK_COUNT {
            if self.censor[deck] {
                releases.push(crate::EngineMsg::SetCensor {
                    deck,
                    active: false,
                });
            }
            if self.cue[deck] {
                releases.push(crate::EngineMsg::CueButton {
                    deck,
                    pressed: false,
                });
            }
        }
        if !releases.is_empty() {
            tx.send_group(releases)?;
            *self = Self::default();
        }
        Ok(())
    }
}

/// Routes resolved controller updates: natively owned controls go straight to the engine queue,
/// continuous UI-owned controls are coalesced, discrete actions are delivered immediately.
struct Router {
    held: HeldDeckButtons,
    midi_received: Option<Instant>,
    app: AppHandle,
    telemetry: Arc<DeckTelemetry>,
    batch: ControllerBatchEvent,
    tempo_range: [f64; 2],
}

impl Router {
    fn probe(&self) {
        if let Some(received) = self.midi_received {
            let _ = self
                .app
                .state::<crate::EngineHandle>()
                .send(crate::EngineMsg::LatencyProbe { received });
        }
    }

    fn route(&mut self, registry: &Registry, mut update: ControllerUpdateEvent) {
        if let Some(deck) = tempo_deck(&update.control) {
            if let Some(range) = self.tempo_range.get(deck).copied() {
                update.value = scale_tempo_percent(update.value, range);
            }
        }
        if update.control == "mixer.crossfader" {
            let _ = self
                .app
                .state::<crate::EngineHandle>()
                .send(crate::EngineMsg::SetCrossfader(update.value as f32));
            upsert_latest(&mut self.batch.mirror, update);
            self.probe();
            return;
        }
        if let Some((deck, slot, shifted)) = keyboard_control(&update.control) {
            self.route_keyboard(deck, slot, shifted, update);
            return;
        }
        if let Some(local) = sampler_local(&update.control) {
            let bank = self
                .app
                .state::<crate::native_controls::SamplerBanks>()
                .bank();
            if let Some(global) = compas_audio::sampler_global_slot(bank, local) {
                let action = if update.control.ends_with(".stop") {
                    "stop"
                } else {
                    "trigger"
                };
                update.control = format!("sampler.{global}.{action}");
            }
        }
        if super::headphone::controller_update(&self.app, &update.control, update.value) {
            return;
        }
        // Beat FX and key shift are native-owned. Never fall back through React on errors.
        let engine = self.app.state::<crate::EngineHandle>();
        match self
            .app
            .state::<crate::native_effects::NativeEffects>()
            .handle(&update.control, update.value, |message| {
                engine.send(message)
            }) {
            Ok(Some(snapshot)) => {
                let _ = self.app.emit("effects:update", snapshot);
                self.probe();
                return;
            }
            Err(error) => {
                tracing::warn!("Native effects control failed: {error}");
                return;
            }
            Ok(None) => {}
        }
        let eq = self.app.state::<crate::native_controls::DeckEqState>();
        let performance = self.app.state::<crate::native_controls::PerformanceState>();
        let captured = update
            .frame
            .zip(update.heard_frame)
            .map(
                |(frame, heard_frame)| crate::native_controls::CapturedPosition {
                    frame,
                    heard_frame,
                },
            );
        if let Some(action) = crate::native_controls::resolve(
            &update.control,
            update.value,
            &self.telemetry,
            &eq,
            &performance,
            captured,
            self.telemetry.sampler_playing(),
        ) {
            let engine = self.app.state::<crate::EngineHandle>();
            let commanded = action.message.is_some() || !action.messages.is_empty();
            for message in action.message.into_iter().chain(action.messages) {
                if let Err(error) = self.held.send(&engine.tx, message) {
                    tracing::warn!("Controller action could not reach the engine: {error}");
                    return;
                }
            }
            if commanded {
                self.probe();
            }
            if let Some(value) = action.mirror_value {
                update.value = value;
            }
            if let Some(control) = action.mirror_as {
                update.control = control;
            }
            update.performance = action.performance;
            if action.mirror {
                upsert_latest(&mut self.batch.mirror, update);
            }
            for (control, value) in action.extra_mirrors {
                upsert_latest(
                    &mut self.batch.mirror,
                    ControllerUpdateEvent::new(control, value),
                );
            }
            if let Some((control, value)) = action.ui_event {
                self.emit_now(ControllerUpdateEvent::new(control, value));
            }
            return;
        }
        let continuous = registry
            .get_str(&update.control)
            .is_some_and(|spec| spec.behavior != Behavior::Toggle);
        if continuous {
            upsert_latest(&mut self.batch.updates, update);
        } else {
            self.emit_now(update);
        }
    }

    fn route_keyboard(
        &mut self,
        deck: usize,
        slot: usize,
        shifted: bool,
        mut update: ControllerUpdateEvent,
    ) {
        let heard = update.heard_frame.or_else(|| {
            crate::native_controls::capture_position(&update.control, &self.telemetry)
                .map(|position| position.heard_frame)
        });
        let effects = self.app.state::<crate::native_effects::NativeEffects>();
        let Ok(snapshot) = effects.snapshot() else {
            tracing::warn!("Keyboard pad could not read the effect state");
            return;
        };
        let Some(deck_fx) = snapshot.decks.get(deck) else {
            return;
        };
        let Some(action) = self
            .app
            .state::<crate::native_controls::PerformanceState>()
            .keyboard(
                deck,
                slot,
                shifted,
                update.value >= 0.5,
                heard,
                deck_fx.pitch_shift,
            )
        else {
            return;
        };
        let engine = self.app.state::<crate::EngineHandle>();
        let commanded = action.message.is_some() || !action.messages.is_empty();
        for message in action.message.into_iter().chain(action.messages) {
            if let Err(error) = self.held.send(&engine.tx, message) {
                tracing::warn!("Keyboard pad could not reach the engine: {error}");
                return;
            }
        }
        if let Some(semitones) = action.pitch {
            self.apply_pitch(deck, semitones);
        }
        if let Some(control) = action.mirror_as {
            update.control = control;
        }
        update.performance = action.performance;
        if action.mirror {
            upsert_latest(&mut self.batch.mirror, update);
        }
        if commanded {
            self.probe();
        }
    }

    fn apply_pitch(&self, deck: usize, semitones: f64) {
        let engine = self.app.state::<crate::EngineHandle>();
        match self
            .app
            .state::<crate::native_effects::NativeEffects>()
            .set_pitch(deck, semitones, |message| engine.send(message))
        {
            Ok(snapshot) => {
                let _ = self.app.emit("effects:update", snapshot);
            }
            Err(error) => tracing::warn!("Keyboard pitch failed: {error}"),
        }
    }

    fn emit_now(&mut self, mut update: ControllerUpdateEvent) {
        if let Some(position) =
            crate::native_controls::capture_position(&update.control, &self.telemetry)
        {
            update.frame.get_or_insert(position.frame);
            update.heard_frame.get_or_insert(position.heard_frame);
        }
        self.flush();
        if let Err(error) = self.app.emit("controller:update", update) {
            tracing::warn!("controller update delivery failed: {error}");
        }
    }

    fn flush(&mut self) {
        if self.batch.mirror.is_empty() && self.batch.updates.is_empty() {
            return;
        }
        let _ = self
            .app
            .emit("controller:batch", std::mem::take(&mut self.batch));
    }
}

/// Note LEDs for these controls show application state (pushed via `Feedback`), not the physical
/// button position, so a press must not be echoed back as a momentary light.
fn has_state_led(control: &str) -> bool {
    control.ends_with(".headphone_cue")
        || control.ends_with(".play")
        || control.ends_with(".cue")
        || control.ends_with(".sync")
        || (control.starts_with("sampler.") && control.ends_with(".trigger"))
        || [
            ".hotcue.",
            ".clear_hotcue.",
            ".loop.",
            ".beatloop.",
            ".beatjump.",
            ".key_shift.",
        ]
        .iter()
        .any(|part| control.contains(part))
        || control.starts_with("fx.toggle.")
}

/// FLX4 transport LEDs follow engine telemetry (including the paused blink) instead of UI feedback.
fn is_native_transport_led(control: &str) -> bool {
    control.ends_with(".play")
}

/// Messages to the controller engine thread.
pub enum ControllerMsg {
    /// A raw MIDI message (status, data1, data2) forwarded from the input port.
    Midi(u8, u8, u8, Instant),
    /// A changed byte in an HID input report (`byte` index, absolute `value`), forwarded from the
    /// HID reader. Resolved through the active mapping's `InputKind::Hid` bindings (channel 0).
    Hid { byte: u8, value: u8 },
    /// Activate a profile (declarative bindings + optional script).
    Activate(Box<ControllerProfile>),
    /// Drop the active profile.
    Deactivate,
    /// Open (or close, with `None`) a MIDI output port for LED/feedback echo — matched by name
    /// substring against the OS output ports.
    SetOutputPort(Option<String>),
    /// Reflect a control's current engine value back onto the device (LED rings, motor faders).
    /// Driven by the frontend whenever a control changes — from the UI *or* the controller — so the
    /// hardware tracks software state, not just controller-originated moves.
    Feedback { control: String, value: f64 },
    /// Republish every MIDI message as raw input while a mapping is being learned.
    Learn(bool),
}

/// Handle to the controller engine thread. The thread owns the (`!Send`) script runtime, so all it
/// exposes is a message sender.
pub struct ControllerEngine {
    tx: Sender<ControllerMsg>,
}

impl ControllerEngine {
    /// Spawn the engine thread. It owns the script runtime + active mapping and emits
    /// `controller:update` events the frontend applies.
    pub fn spawn(app: AppHandle) -> Self {
        let (tx, rx) = mpsc::channel::<ControllerMsg>();
        std::thread::Builder::new()
            .name("compas-controller".into())
            .spawn(move || run(app, rx))
            .expect("spawn controller thread");
        ControllerEngine { tx }
    }

    pub fn send(&self, msg: ControllerMsg) {
        let _ = self.tx.send(msg);
    }

    /// A cloneable sender for forwarding raw MIDI from the input callback.
    pub fn sender(&self) -> Sender<ControllerMsg> {
        self.tx.clone()
    }
}

/// Controller engine loop: resolve forwarded MIDI through the active profile (declarative bindings
/// first, then the script's `onMidi`), and emit the resulting control updates.
fn run(app: AppHandle, rx: mpsc::Receiver<ControllerMsg>) {
    let registry = Registry::defaults(DECK_COUNT);
    let mut mapping = Mapping::default();
    let mut script: Option<ScriptRuntime> = None;
    // Last normalized value applied per control, for soft-takeover.
    let mut current: HashMap<String, f64> = HashMap::new();
    // MIDI output for LED/feedback (controller echo + UI-driven changes).
    let mut out: Option<MidiOutputConnection> = None;
    // Last MIDI value sent per output address (status+data1), to skip redundant resends.
    let mut fb_last: HashMap<[u8; 2], u8> = HashMap::new();

    let telemetry = app.state::<crate::EngineHandle>().telemetry.clone();
    let mut router = Router {
        held: HeldDeckButtons::default(),
        midi_received: None,
        app: app.clone(),
        telemetry: telemetry.clone(),
        batch: ControllerBatchEvent::default(),
        tempo_range: [FLX4_TEMPO_RANGES[0], FLX4_TEMPO_RANGES[0]],
    };
    let mut flx4_profile = false;
    let mut flx4_port = false;
    let mut learning = false;
    let mut desired_feedback: HashMap<String, f64> = HashMap::new();
    let mut next_keep_alive = Instant::now();
    let mut high_resolution = [[0_u16; 32]; 16];
    let mut next_meter = Instant::now();
    let mut next_flush = Instant::now();
    let mut meter_last = [255_u8; 2];
    let mut loop_in_holds: [Option<LoopInHold>; 2] = [None, None];
    let mut pad_mode: [Option<u8>; 2] = [None, None];
    let blink_epoch = Instant::now();
    let mut engine_loop_seen = [false; compas_audio::NUM_DECKS];
    loop {
        let now = Instant::now();
        if flx4_profile && flx4_port && now >= next_keep_alive {
            next_keep_alive = now + KEEP_ALIVE;
            if let Some(port) = out.as_mut() {
                // Mixxx FLX4 driver: hardware status query / keep-alive (not firmware update).
                let _ = port.send(&[
                    0xf0, 0x00, 0x40, 0x05, 0x00, 0x00, 0x04, 0x05, 0x00, 0x50, 0x02, 0xf7,
                ]);
            }
            for (control, value) in &desired_feedback {
                if is_native_transport_led(control) {
                    continue;
                }
                if let Some(spec) = registry.get_str(control) {
                    for b in &mapping.bindings {
                        if b.control.as_str() == control {
                            send_bound_feedback(
                                &mut out,
                                &mut fb_last,
                                b.control.as_str(),
                                b.channel,
                                b.input,
                                spec.behavior.to_midi(*value),
                            );
                        }
                    }
                }
            }
        }
        if now >= next_meter {
            next_meter = now + METER_PERIOD;
            if flx4_profile && flx4_port {
                let blink_lit = blink_epoch.elapsed().as_millis() / TRANSPORT_BLINK_MS % 2 == 0;
                for (deck, last_meter) in meter_last.iter_mut().enumerate() {
                    let value = flx4_meter_value(telemetry.deck_level(deck));
                    if value != *last_meter {
                        if let Some(port) = out.as_mut() {
                            if port.send(&[0xb0 | deck as u8, 0x02, value]).is_ok() {
                                *last_meter = value;
                            }
                        }
                    }
                    // Engine truth: lit while playing, blinking while a loaded deck is paused.
                    let transport_lit =
                        telemetry.is_playing(deck) || (telemetry.is_loaded(deck) && blink_lit);
                    send_feedback(
                        &mut out,
                        &mut fb_last,
                        deck as u8,
                        InputKind::Note {
                            note: FLX4_PLAY_NOTE,
                        },
                        if transport_lit { 127 } else { 0 },
                    );
                    send_feedback(
                        &mut out,
                        &mut fb_last,
                        deck as u8,
                        InputKind::Note {
                            note: FLX4_CUE_NOTE,
                        },
                        if telemetry.is_at_cue(deck) { 127 } else { 0 },
                    );
                    let loaded_value = if telemetry.is_loaded(deck) { 127 } else { 0 };
                    let shift_channel = 8 + deck as u8 * 2;
                    for note in FLX4_TRACK_LOADED_NOTES {
                        send_feedback(
                            &mut out,
                            &mut fb_last,
                            shift_channel,
                            InputKind::Note { note },
                            loaded_value,
                        );
                    }
                }
            }
        }
        if flx4_profile {
            for (deck, hold) in loop_in_holds.iter_mut().enumerate() {
                let Some(hold) = hold
                    .as_mut()
                    .filter(|hold| !hold.fired && now.duration_since(hold.since) >= LOOP_IN_HOLD)
                else {
                    continue;
                };
                hold.fired = true;
                let heard = hold.heard_frame;
                let mut update = ControllerUpdateEvent::new(
                    format!("deck.{deck}.beatloop.{FOUR_BEAT_LOOP_PAD}"),
                    1.0,
                );
                update.frame = heard;
                update.heard_frame = heard;
                router.route(&registry, update);
            }
        }
        if flx4_profile {
            let nudges = app
                .state::<crate::flx4_runtime::JogRuntime>()
                .take_loop_nudges();
            for (deck, (nudge_in, nudge_out)) in nudges.into_iter().enumerate() {
                if nudge_in != 0 {
                    router.route(
                        &registry,
                        ControllerUpdateEvent::new(
                            format!("deck.{deck}.loop.nudge_in"),
                            f64::from(nudge_in),
                        ),
                    );
                }
                if nudge_out != 0 {
                    router.route(
                        &registry,
                        ControllerUpdateEvent::new(
                            format!("deck.{deck}.loop.nudge_out"),
                            f64::from(nudge_out),
                        ),
                    );
                }
            }
            let jumps = app
                .state::<crate::flx4_runtime::JogRuntime>()
                .take_bar_jumps();
            let performance = app.state::<crate::native_controls::PerformanceState>();
            let engine = app.state::<crate::EngineHandle>();
            for (deck, bars) in jumps.into_iter().enumerate() {
                if bars == 0 {
                    continue;
                }
                let Some(action) = crate::native_controls::bar_jump(
                    deck,
                    bars,
                    telemetry.playhead_frames(deck),
                    &performance,
                ) else {
                    continue;
                };
                for message in action.message.into_iter().chain(action.messages) {
                    if let Err(error) = engine.send(message) {
                        tracing::warn!("Bar jump could not reach the engine: {error}");
                    }
                }
            }
        }
        for deck in 0..compas_audio::NUM_DECKS {
            let (active, in_frame, out_frame) = telemetry.loop_region(deck);
            let was = engine_loop_seen[deck];
            engine_loop_seen[deck] = active;
            if active && !was {
                let performance = app.state::<crate::native_controls::PerformanceState>();
                if let Some(mirror) = performance.adopt_engaged_loop(deck, in_frame, out_frame) {
                    let mut update =
                        ControllerUpdateEvent::new(format!("deck.{deck}.loop.memory_active"), 1.0);
                    update.performance = Some(mirror);
                    router.batch.mirror.push(update);
                }
            }
        }
        if now >= next_flush {
            next_flush = now + UI_FLUSH;
            router.flush();
        }
        let wake = next_meter.min(next_flush);
        let msg = match rx.recv_timeout(wake.saturating_duration_since(Instant::now())) {
            Ok(msg) => msg,
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        };
        if matches!(
            &msg,
            ControllerMsg::SetOutputPort(None)
                | ControllerMsg::Activate(_)
                | ControllerMsg::Deactivate
        ) {
            if let Err(error) = router.held.release(&app.state::<crate::EngineHandle>().tx) {
                tracing::warn!("Controller button release failed: {error}");
            }
            loop_in_holds = [None, None];
            clear_loop_adjust(&app);
        }
        match msg {
            ControllerMsg::Learn(active) => learning = active,
            ControllerMsg::SetOutputPort(name) => {
                if flx4_port {
                    clear_flx4_deck_lights(&mut out, &mut fb_last, &mut pad_mode);
                }
                release_held_pad_fx(&app, 0);
                release_held_pad_fx(&app, 1);
                release_keyboard(&app, 0);
                release_keyboard(&app, 1);
                pad_mode = [None, None];
                flx4_port = name.as_ref().is_some_and(|n| n.contains("DDJ-FLX4"));
                meter_last = [255; 2];
                out = name.and_then(|name| open_output(&name));
                fb_last.clear(); // a fresh port's LED/fader state is unknown
                if out.is_some() {
                    next_keep_alive = Instant::now();
                    next_meter = Instant::now();
                }
            }
            ControllerMsg::Feedback { control, value } => {
                if registry.get_str(&control).is_some() && value.is_finite() {
                    desired_feedback.insert(control.clone(), value);
                }
                if out.is_none() || (flx4_profile && flx4_port && is_native_transport_led(&control))
                {
                    continue;
                }
                // Engine value → MIDI via the control's behavior, sent to every input bound to it.
                if let Some(spec) = registry.get_str(&control) {
                    let mv = spec.behavior.to_midi(value);
                    for b in &mapping.bindings {
                        if b.control.as_str() == control {
                            send_bound_feedback(
                                &mut out,
                                &mut fb_last,
                                b.control.as_str(),
                                b.channel,
                                b.input,
                                mv,
                            );
                        }
                    }
                }
            }
            ControllerMsg::Activate(profile) => {
                if flx4_profile && flx4_port {
                    clear_flx4_deck_lights(&mut out, &mut fb_last, &mut pad_mode);
                }
                release_held_pad_fx(&app, 0);
                release_held_pad_fx(&app, 1);
                release_keyboard(&app, 0);
                release_keyboard(&app, 1);
                pad_mode = [None, None];
                flx4_profile = profile.id == "pioneer-ddj-flx4";
                meter_last = [255; 2];
                router.tempo_range = [FLX4_TEMPO_RANGES[0], FLX4_TEMPO_RANGES[0]];
                high_resolution = [[0; 32]; 16];
                mapping = profile.mapping();
                script = profile
                    .script
                    .as_deref()
                    .and_then(|src| match ScriptRuntime::new() {
                        Ok(rt) => {
                            if let Err(e) = rt.eval(src) {
                                tracing::warn!("controller script error: {e}");
                            }
                            Some(rt)
                        }
                        Err(e) => {
                            tracing::warn!("controller script runtime: {e}");
                            None
                        }
                    });
                current.clear();
                fb_last.clear(); // re-send feedback for the new mapping's addresses
            }
            ControllerMsg::Deactivate => {
                if flx4_profile && flx4_port {
                    clear_flx4_deck_lights(&mut out, &mut fb_last, &mut pad_mode);
                }
                release_held_pad_fx(&app, 0);
                release_held_pad_fx(&app, 1);
                release_keyboard(&app, 0);
                release_keyboard(&app, 1);
                pad_mode = [None, None];
                flx4_profile = false;
                router.tempo_range = [FLX4_TEMPO_RANGES[0], FLX4_TEMPO_RANGES[0]];
                mapping = Mapping::default();
                script = None;
                current.clear();
                fb_last.clear();
            }
            ControllerMsg::Midi(status, d1, raw_d2, received) => {
                router.midi_received = Some(received);
                // Special FLX4 handlers must see the same release semantics as mappings.
                let (status, raw_d2) = if flx4_profile {
                    normalize_note_release(status, raw_d2)
                } else {
                    (status, raw_d2)
                };
                let consumed = 'resolve: {
                    if flx4_profile {
                        if matches!(status, 0x90 | 0x91) && d1 == FLX4_SHIFT_NOTE {
                            send_feedback(
                                &mut out,
                                &mut fb_last,
                                status & 15,
                                InputKind::Note { note: d1 },
                                if raw_d2 > 0 { 127 } else { 0 },
                            );
                            break 'resolve true;
                        }
                        if matches!(status, 0x90 | 0x91)
                            && matches!(d1, FLX4_LOOP_IN_ADJUST_NOTE | FLX4_LOOP_OUT_ADJUST_NOTE)
                        {
                            let deck = usize::from(status & 15);
                            let pressed = raw_d2 > 0;
                            let edge = if !pressed {
                                0
                            } else if d1 == FLX4_LOOP_IN_ADJUST_NOTE {
                                1
                            } else {
                                2
                            };
                            app.state::<crate::flx4_runtime::JogRuntime>()
                                .set_loop_adjust(deck, edge);
                            send_feedback(
                                &mut out,
                                &mut fb_last,
                                status & 15,
                                InputKind::Note { note: d1 },
                                if pressed { 127 } else { 0 },
                            );
                            break 'resolve true;
                        }
                        if matches!(status, 0x90 | 0x91)
                            && d1 == FLX4_TEMPO_RANGE_NOTE
                            && raw_d2 > 0
                        {
                            let deck = usize::from(status & 15);
                            if let Some(range) = router.tempo_range.get_mut(deck) {
                                *range = next_tempo_range(*range);
                                let selected = *range;
                                if let Some(norm) =
                                    current.get(&format!("deck.{deck}.tempo")).copied()
                                {
                                    router.route(
                                        &registry,
                                        ControllerUpdateEvent::new(
                                            format!("deck.{deck}.tempo"),
                                            tempo_percent_from_normalized(norm),
                                        ),
                                    );
                                }
                                router.emit_now(ControllerUpdateEvent::new(
                                    format!("deck.{deck}.tempo_range"),
                                    selected,
                                ));
                            }
                            break 'resolve true;
                        }
                        if matches!(status, 0x90 | 0x91) && d1 == FLX4_LOOP_IN_NOTE {
                            let deck = usize::from(status & 15);
                            loop_in_holds[deck] = (raw_d2 > 0).then(|| LoopInHold {
                                since: Instant::now(),
                                heard_frame: crate::native_controls::capture_position(
                                    &format!("deck.{deck}.loop.in"),
                                    &telemetry,
                                )
                                .map(|position| position.heard_frame),
                                fired: false,
                            });
                        }
                        if matches!(status, 0x90 | 0x91) && FLX4_PAD_MODE_NOTES.contains(&d1) {
                            if raw_d2 > 0 {
                                let deck = usize::from(status & 15);
                                let previous = pad_mode.get(deck).copied().flatten();
                                if let Some(leds) = flx4_pad_mode_press(&mut pad_mode, deck, d1) {
                                    if leaving_pad_fx(previous, leds.lit) {
                                        release_held_pad_fx(&app, deck);
                                    }
                                    if previous == Some(FLX4_KEYBOARD_NOTE)
                                        || leds.lit == FLX4_KEYBOARD_NOTE
                                    {
                                        release_keyboard(&app, deck);
                                    }
                                    if let Some(previous) = leds.clear {
                                        send_feedback(
                                            &mut out,
                                            &mut fb_last,
                                            status & 15,
                                            InputKind::Note { note: previous },
                                            0,
                                        );
                                    }
                                    send_feedback(
                                        &mut out,
                                        &mut fb_last,
                                        status & 15,
                                        InputKind::Note { note: leds.lit },
                                        127,
                                    );
                                }
                            }
                            break 'resolve true;
                        }
                        // FLX4 mixer and tempo controls transmit 14-bit MSB/LSB pairs.
                        if let Some((ch, base, norm)) =
                            flx4_cc_pair(&mut high_resolution, status, d1, raw_d2)
                        {
                            if let Some(binding) = mapping.find(ch, InputKind::Cc { cc: base }) {
                                if let Some(spec) = registry.get(&binding.control) {
                                    let update = ControllerUpdateEvent::new(
                                        binding.control.as_str(),
                                        spec.behavior.from_normalized(norm),
                                    );
                                    current.insert(update.control.clone(), norm);
                                    router.route(&registry, update);
                                }
                            }
                            break 'resolve true;
                        }
                        if status == 0xb6 && matches!(d1, FLX4_BROWSE_CC | FLX4_ZOOM_CC) {
                            let steps = if raw_d2 < 64 {
                                f64::from(raw_d2)
                            } else {
                                f64::from(raw_d2) - 128.0
                            };
                            let control = if d1 == FLX4_BROWSE_CC {
                                "library.browse"
                            } else {
                                "waveform.zoom"
                            };
                            router.emit_now(ControllerUpdateEvent::new(control, steps));
                            break 'resolve true;
                        }
                        if status == 0x96 && FLX4_BROWSE_PRESS_NOTES.contains(&d1) {
                            if raw_d2 > 0 {
                                router.emit_now(ControllerUpdateEvent::new("library.focus", 1.0));
                            }
                            break 'resolve true;
                        }
                        if status == 0x96 && FLX4_LOAD_NOTES.contains(&d1) {
                            // LOAD follows the same explicit replacement policy as drag/drop.
                            // Playing telemetry must not silently disable a physical button.
                            if let Some(deck) = flx4_load_pressed(status, d1, raw_d2) {
                                router.emit_now(ControllerUpdateEvent::new(
                                    format!("library.load.{deck}"),
                                    1.0,
                                ));
                            }
                            break 'resolve true;
                        }
                    }
                    resolve_midi(
                        &registry,
                        &mapping,
                        script.as_ref(),
                        &mut current,
                        &mut out,
                        &mut fb_last,
                        &mut router,
                        [status, d1, raw_d2],
                    )
                };
                router.midi_received = None;
                if learning || !consumed {
                    emit_raw_midi(&app, status, d1, raw_d2, consumed);
                }
            }
            ControllerMsg::Hid { byte, value } => {
                // HID reports carry no MIDI channel; bindings use channel 0 + `InputKind::Hid`.
                if let Some(update) = resolve_binding(
                    &registry,
                    &mapping,
                    &mut current,
                    &mut out,
                    &mut fb_last,
                    0,
                    InputKind::Hid { byte },
                    value,
                ) {
                    router.route(&registry, update);
                }
            }
        }
    }
}

/// NoteOff release velocity is not a press. Normalize before all special handlers.
fn normalize_note_release(status: u8, value: u8) -> (u8, u8) {
    if status & 0xf0 == 0x80 {
        (status | 0x10, 0)
    } else {
        (status, value)
    }
}

/// Resolve a non-FLX4-specific MIDI message through the declarative bindings, then the profile
/// script. Returns whether the active profile consumed the message.
#[expect(
    clippy::too_many_arguments,
    reason = "controller-thread state is owned by the run loop and borrowed per message"
)]
fn resolve_midi(
    registry: &Registry,
    mapping: &Mapping,
    script: Option<&ScriptRuntime>,
    current: &mut HashMap<String, f64>,
    out: &mut Option<MidiOutputConnection>,
    fb_last: &mut HashMap<[u8; 2], u8>,
    router: &mut Router,
    [status, d1, raw_d2]: [u8; 3],
) -> bool {
    let d2 = if status & 0xf0 == 0x80 { 0 } else { raw_d2 };
    let channel = status & 0x0F;
    let input = match status & 0xF0 {
        0xB0 => InputKind::Cc { cc: d1 },
        0x90 | 0x80 => InputKind::Note { note: d1 },
        _ => return false,
    };
    if mapping.find(channel, input).is_some() {
        if let Some(update) =
            resolve_binding(registry, mapping, current, out, fb_last, channel, input, d2)
        {
            router.route(registry, update);
        }
        return true;
    }
    // No declarative binding owns this input — let the script handle it.
    let Some(Ok(script_updates)) = script.map(|runtime| runtime.on_midi(status, d1, d2)) else {
        return false;
    };
    if script_updates.is_empty() {
        return false;
    }
    for script_update in script_updates {
        // Script `engine.set` values are normalized by convention.
        let norm = script_update.value.clamp(0.0, 1.0);
        let value = registry
            .get_str(&script_update.control)
            .map(|spec| spec.behavior.from_normalized(norm))
            .unwrap_or(norm);
        current.insert(script_update.control.clone(), norm);
        router.route(
            registry,
            ControllerUpdateEvent::new(script_update.control, value),
        );
    }
    true
}

// Vendor E1 output table: CC2 on channels0/1, five cumulative LED ranges.
// Our peak thresholds are -36/-24/-12/-6/0 dBFS; not a claim of rekordbox calibration.
fn flx4_meter_value(level: f32) -> u8 {
    if !level.is_finite() || level < 0.015848932 {
        return 0;
    }
    if level < 0.06309573 {
        0x26
    } else if level < 0.25118864 {
        0x41
    } else if level < 0.5011872 {
        0x57
    } else if level < 1.0 {
        0x65
    } else {
        0x77
    }
}
fn clear_flx4_meters(out: &mut Option<MidiOutputConnection>) {
    if let Some(port) = out.as_mut() {
        let _ = port.send(&[0xb0, 0x02, 0]);
        let _ = port.send(&[0xb1, 0x02, 0]);
    }
}

struct PadModeLeds {
    clear: Option<u8>,
    lit: u8,
}

fn leaving_pad_fx(previous: Option<u8>, next: u8) -> bool {
    matches!(
        previous,
        Some(note) if (note == FLX4_PAD_FX1_NOTE || note == FLX4_PAD_FX2_NOTE) && note != next
    )
}

fn sampler_local(control: &str) -> Option<usize> {
    let (slot, action) = control.strip_prefix("sampler.")?.split_once('.')?;
    if action != "trigger" && action != "stop" {
        return None;
    }
    let local = slot.parse::<usize>().ok()?;
    (local < compas_audio::SAMPLER_SLOTS_PER_BANK).then_some(local)
}

fn keyboard_control(control: &str) -> Option<(usize, usize, bool)> {
    let rest = control.strip_prefix("deck.")?;
    let (deck, name) = rest.split_once('.')?;
    let deck = deck.parse::<usize>().ok()?;
    let (shifted, slot) = if let Some(slot) = name.strip_prefix("keyboard_shift.") {
        (true, slot)
    } else {
        (false, name.strip_prefix("keyboard.")?)
    };
    let slot = slot.parse::<usize>().ok()?;
    (slot < 8).then_some((deck, slot, shifted))
}

fn release_keyboard(app: &AppHandle, deck: usize) {
    let Some(semitones) = app
        .state::<crate::native_controls::PerformanceState>()
        .reset_keyboard(deck)
    else {
        return;
    };
    let engine = app.state::<crate::EngineHandle>();
    match app
        .state::<crate::native_effects::NativeEffects>()
        .set_pitch(deck, semitones, |message| engine.send(message))
    {
        Ok(snapshot) => {
            let _ = app.emit("effects:update", snapshot);
        }
        Err(error) => tracing::warn!("Keyboard release failed: {error}"),
    }
}

fn release_held_pad_fx(app: &AppHandle, deck: usize) {
    let engine = app.state::<crate::EngineHandle>();
    match app
        .state::<crate::native_effects::NativeEffects>()
        .release_pad_fx(deck, |message| engine.send(message))
    {
        Ok(Some(snapshot)) => {
            let _ = app.emit("effects:update", snapshot);
        }
        Ok(None) => {}
        Err(error) => tracing::warn!("Pad FX release failed: {error}"),
    }
}

/// One pad-mode button per deck stays lit. Returns the previous note to extinguish, if it changed.
fn flx4_pad_mode_press(active: &mut [Option<u8>; 2], deck: usize, note: u8) -> Option<PadModeLeds> {
    if !FLX4_PAD_MODE_NOTES.contains(&note) {
        return None;
    }
    let slot = active.get_mut(deck)?;
    let clear = (*slot).filter(|previous| *previous != note);
    *slot = Some(note);
    Some(PadModeLeds { clear, lit: note })
}

fn clear_flx4_deck_lights(
    out: &mut Option<MidiOutputConnection>,
    fb_last: &mut HashMap<[u8; 2], u8>,
    pad_mode: &mut [Option<u8>; 2],
) {
    clear_flx4_meters(out);
    for deck in 0..2u8 {
        send_feedback(
            out,
            fb_last,
            deck,
            InputKind::Note {
                note: FLX4_PLAY_NOTE,
            },
            0,
        );
        if let Some(note) = pad_mode[deck as usize].take() {
            send_feedback(out, fb_last, deck, InputKind::Note { note }, 0);
        }
        for note in [
            FLX4_CUE_NOTE,
            FLX4_SHIFT_NOTE,
            FLX4_LOOP_IN_ADJUST_NOTE,
            FLX4_LOOP_OUT_ADJUST_NOTE,
        ] {
            send_feedback(out, fb_last, deck, InputKind::Note { note }, 0);
        }
        let shift_channel = 8 + deck * 2;
        for note in FLX4_TRACK_LOADED_NOTES {
            send_feedback(out, fb_last, shift_channel, InputKind::Note { note }, 0);
        }
    }
}

fn clear_loop_adjust(app: &AppHandle) {
    let jog = app.state::<crate::flx4_runtime::JogRuntime>();
    jog.set_loop_adjust(0, 0);
    jog.set_loop_adjust(1, 0);
    let _ = jog.take_loop_nudges();
}

fn tempo_deck(control: &str) -> Option<usize> {
    let rest = control.strip_prefix("deck.")?;
    let (index, name) = rest.split_once('.')?;
    if name != "tempo" {
        return None;
    }
    let deck = index.parse::<usize>().ok()?;
    (deck < 2).then_some(deck)
}

fn tempo_percent_from_normalized(normalized: f64) -> f64 {
    -8.0 + 16.0 * normalized.clamp(0.0, 1.0)
}

fn scale_tempo_percent(percent: f64, range: f64) -> f64 {
    if !percent.is_finite() || !range.is_finite() || range <= 0.0 {
        return 0.0;
    }
    (percent * (range / 8.0)).clamp(-100.0, 100.0)
}

fn next_tempo_range(current: f64) -> f64 {
    let index = FLX4_TEMPO_RANGES
        .iter()
        .position(|range| (*range - current).abs() < 0.01)
        .unwrap_or(0);
    FLX4_TEMPO_RANGES[(index + 1) % FLX4_TEMPO_RANGES.len()]
}

fn flx4_shift_layer_channel(control: &str, channel: u8) -> Option<u8> {
    (control.contains(".beatloop.") && matches!(channel, 7 | 9)).then_some(channel + 1)
}

fn send_bound_feedback(
    out: &mut Option<MidiOutputConnection>,
    last: &mut HashMap<[u8; 2], u8>,
    control: &str,
    channel: u8,
    input: InputKind,
    value: u8,
) {
    send_feedback(out, last, channel, input, value);
    if let Some(shift_channel) = flx4_shift_layer_channel(control, channel) {
        send_feedback(out, last, shift_channel, input, value);
    }
}

/// Resolve one physical input through the active mapping: honor soft-takeover, echo immediate
/// feedback to the device, remember the applied value (for soft-takeover), and return the control
/// update to emit — or `None` if no binding matched or soft-takeover blocked the move.
#[allow(clippy::too_many_arguments)]
fn resolve_binding(
    registry: &Registry,
    mapping: &Mapping,
    current: &mut HashMap<String, f64>,
    out: &mut Option<MidiOutputConnection>,
    fb_last: &mut HashMap<[u8; 2], u8>,
    channel: u8,
    input: InputKind,
    value: u8,
) -> Option<ControllerUpdateEvent> {
    let binding = mapping.find(channel, input)?;
    let (b_channel, b_input) = (binding.channel, binding.input);
    // Soft-takeover compares against the last value we applied; unknown → adopt immediately.
    let cur = current
        .get(binding.control.as_str())
        .copied()
        .unwrap_or(value as f64 / input.full_scale());
    let u = mapping.resolve(registry, channel, input, value, cur, TAKEOVER)?;
    // Echo to the device address that moved (immediate LED/feedback); UI changes flow via Feedback.
    let mv = (u.normalized * 127.0).round().clamp(0.0, 127.0) as u8;
    // Continuous controls can still echo immediately for motor/display feedback.
    if !matches!(b_input, InputKind::Note { .. }) || !has_state_led(u.control.as_str()) {
        send_feedback(out, fb_last, b_channel, b_input, mv);
    }
    current.insert(u.control.as_str().to_string(), u.normalized);
    Some(ControllerUpdateEvent::new(u.control.as_str(), u.value))
}

/// Open a MIDI output port whose name contains `name` (for LED/feedback echo).
fn open_output(name: &str) -> Option<MidiOutputConnection> {
    let midi = MidiOutput::new("compas-out").ok()?;
    let ports = midi.ports();
    let port = ports
        .iter()
        .find(|p| midi.port_name(p).map(|n| n.contains(name)).unwrap_or(false))?;
    midi.connect(port, "compas-feedback").ok()
}

/// Send a feedback value to a control's input address on the device, skipping the send when the
/// address already holds that value (avoids flooding LEDs / motor faders with redundant messages).
/// No-op when no output port is open.
fn send_feedback(
    out: &mut Option<MidiOutputConnection>,
    last: &mut HashMap<[u8; 2], u8>,
    channel: u8,
    input: InputKind,
    value: u8,
) {
    let Some(conn) = out.as_mut() else { return };
    // HID outputs have no MIDI representation (device-specific report; deferred) → skip them.
    let Some(bytes) = midi_bytes(channel, input, value) else {
        return;
    };
    let addr = [bytes[0], bytes[1]]; // status (type+channel) + note/cc number
    if feedback_changed(last, addr, value) {
        let _ = conn.send(&bytes);
    }
}

/// Record `value` for an output address, returning whether it differs from the last value sent
/// there (so callers skip redundant device writes). Pure so it can be unit-tested without a port.
fn feedback_changed(last: &mut HashMap<[u8; 2], u8>, addr: [u8; 2], value: u8) -> bool {
    if last.get(&addr) == Some(&value) {
        return false;
    }
    last.insert(addr, value);
    true
}

/// Build a 3-byte MIDI message echoing `value` (0..127) to a binding's input address. Returns
/// `None` for non-MIDI (HID) inputs, which have no MIDI feedback representation.
fn midi_bytes(channel: u8, input: InputKind, value: u8) -> Option<[u8; 3]> {
    let ch = channel & 0x0F;
    match input {
        InputKind::Cc { cc } => Some([0xB0 | ch, cc, value]),
        InputKind::Note { note } => Some([0x90 | ch, note, value]),
        InputKind::Hid { .. } => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_end_releases_only_still_held_controls_once() {
        use crate::EngineMsg::{CueButton, SetCensor};
        let (tx, mut rx) = crate::engine_commands::channel();
        let mut held = HeldDeckButtons::default();
        for msg in [
            CueButton {
                deck: 0,
                pressed: true,
            },
            CueButton {
                deck: 0,
                pressed: false,
            },
            SetCensor {
                deck: 1,
                active: true,
            },
            CueButton {
                deck: 2,
                pressed: true,
            },
        ] {
            held.send(&tx, msg).unwrap();
            rx.recv_timeout(Duration::ZERO).unwrap();
        }
        held.release(&tx).unwrap();
        held.release(&tx).unwrap();
        assert!(matches!(
            rx.recv_timeout(Duration::ZERO),
            Ok(SetCensor {
                deck: 1,
                active: false
            })
        ));
        assert!(matches!(
            rx.recv_timeout(Duration::ZERO),
            Ok(CueButton {
                deck: 2,
                pressed: false
            })
        ));
        assert!(rx.recv_timeout(Duration::ZERO).is_err());
    }

    #[test]
    fn flx4_load_is_repeatable_and_independent_of_transport_and_pfl() {
        for _ in 0..10_000 {
            // Browsing, PFL presses/releases and playback do not participate in LOAD admission.
            for (status, note, value) in [(0xb6, 0x40, 1), (0x90, 0x54, 127), (0x80, 0x54, 0)] {
                assert_eq!(flx4_load_pressed(status, note, value), None);
            }
            for (deck, note) in FLX4_LOAD_NOTES.into_iter().enumerate() {
                assert_eq!(flx4_load_pressed(0x96, note, 127), Some(deck));
                assert_eq!(flx4_load_pressed(0x96, note, 0), None);
                assert_eq!(flx4_load_pressed(0x86, note, 64), None);
                assert_eq!(flx4_load_pressed(0x96, note, 1), Some(deck));
            }
        }
    }

    #[test]
    fn flx4_high_resolution_crossfade_endpoints_and_lsb_and_jog_isolation() {
        let mut state = [[0_u16; 32]; 16];
        assert!(flx4_cc_pair(&mut state, 0xb6, 31, 0).is_none());
        assert!(flx4_cc_pair(&mut state, 0xb6, 31, 127).is_none());
        assert_eq!(flx4_cc_pair(&mut state, 0xb6, 63, 127), Some((6, 31, 1.0)));
        assert!(flx4_cc_pair(&mut state, 0xb6, 31, 0).is_none());
        assert_eq!(flx4_cc_pair(&mut state, 0xb6, 63, 0), Some((6, 31, 0.0)));
        assert_eq!(
            flx4_cc_pair(&mut state, 0xb6, 63, 1),
            Some((6, 31, 1.0 / 16383.0))
        );
        for cc in [33, 34, 35, 41] {
            assert!(flx4_cc_pair(&mut state, 0xb0, cc, 65).is_none());
        }
        assert!(flx4_cc_pair(&mut state, 0x96, 31, 127).is_none());
        assert!(flx4_cc_pair(&mut state, 0xb4, 2, 64).is_none());
        assert_eq!(
            flx4_cc_pair(&mut state, 0xb4, 34, 0),
            Some((4, 2, 64.0 * 128.0 / 16383.0))
        );
    }

    #[test]
    fn pad_mode_press_clears_the_previous_mode() {
        let mut active = [None, None];
        let first = flx4_pad_mode_press(&mut active, 0, 0x1B).expect("hot cue mode");
        assert_eq!(first.clear, None);
        assert_eq!(first.lit, 0x1B);
        let second = flx4_pad_mode_press(&mut active, 0, 0x20).expect("beat jump mode");
        assert_eq!(second.clear, Some(0x1B));
        assert_eq!(second.lit, 0x20);
        assert_eq!(active[0], Some(0x20));
        assert_eq!(active[1], None);
        assert!(flx4_pad_mode_press(&mut active, 0, 0x01).is_none());
        assert!(flx4_pad_mode_press(&mut active, 2, 0x1B).is_none());
    }

    #[test]
    fn beat_jump_mode_is_note_0x20_and_pad_fx_uses_other_notes() {
        let modes = [
            (0x1Bu8, "hot cue"),
            (0x1E, "pad fx1"),
            (0x20, "beat jump"),
            (0x22, "sampler"),
            (0x69, "keyboard"),
            (0x6B, "pad fx2"),
            (0x6D, "beat loop"),
            (0x6F, "key shift"),
        ];
        let mut active = [None, None];
        for (note, _label) in modes {
            let leds = flx4_pad_mode_press(&mut active, 1, note).expect("mode note");
            assert_eq!(leds.lit, note);
            assert_eq!(active[1], Some(note));
            assert_eq!(active[0], None);
        }
        assert!(flx4_pad_mode_press(&mut active, 1, 0x01).is_none());
        assert_eq!(active[1], Some(0x6F));
        assert!(leaving_pad_fx(Some(0x1E), 0x69));
        assert!(leaving_pad_fx(Some(0x1E), 0x6B));
        assert!(leaving_pad_fx(Some(0x6B), 0x1B));
        assert!(!leaving_pad_fx(Some(0x1E), 0x1E));
        assert!(!leaving_pad_fx(Some(0x6B), 0x6B));
        assert!(!leaving_pad_fx(Some(0x20), 0x1B));
        assert!(!leaving_pad_fx(None, 0x1E));
    }

    #[test]
    fn bundled_flx4_pad_layers_follow_the_message_list() {
        let profile: ControllerProfile = serde_json::from_str(include_str!(
            "../resources/controllers/pioneer-ddj-flx4.json"
        ))
        .unwrap();
        let mapping = profile.mapping();
        let registry = Registry::defaults(4);
        for (deck, plain, shift) in [(0u8, 7u8, 8u8), (1, 9, 10)] {
            for pad in 0..8u8 {
                let rows = [
                    (plain, 0x10 + pad, format!("deck.{deck}.pad_fx.{pad}")),
                    (shift, 0x10 + pad, format!("deck.{deck}.pad_fx.{}", 8 + pad)),
                    (
                        plain,
                        0x50 + pad,
                        format!("deck.{deck}.pad_fx.{}", 16 + pad),
                    ),
                    (
                        shift,
                        0x50 + pad,
                        format!("deck.{deck}.pad_fx.{}", 24 + pad),
                    ),
                    (plain, 0x40 + pad, format!("deck.{deck}.keyboard.{pad}")),
                    (
                        shift,
                        0x40 + pad,
                        format!("deck.{deck}.keyboard_shift.{pad}"),
                    ),
                ];
                for (channel, note, control) in rows {
                    let binding = mapping
                        .find(channel, InputKind::Note { note })
                        .unwrap_or_else(|| panic!("missing {channel:#x} {note:#x}"));
                    assert_eq!(binding.control.as_str(), control);
                    let pressed = mapping
                        .resolve(
                            &registry,
                            channel,
                            InputKind::Note { note },
                            0x7F,
                            0.0,
                            TAKEOVER,
                        )
                        .expect("press");
                    let released = mapping
                        .resolve(
                            &registry,
                            channel,
                            InputKind::Note { note },
                            0,
                            1.0,
                            TAKEOVER,
                        )
                        .expect("zero velocity");
                    assert_eq!(pressed.value, 1.0);
                    assert_eq!(released.value, 0.0);
                    let (status, velocity) = normalize_note_release(0x80 | channel, 64);
                    assert_eq!((status, velocity), (0x90 | channel, 0));
                }
            }
        }
        assert!(mapping
            .find(7, InputKind::Note { note: 0x20 })
            .is_some_and(|binding| binding.control.as_str() == "deck.0.beatjump.0"));
    }
    #[test]
    fn bundled_flx4_every_binding_resolves_and_addresses_are_unique() {
        let p: ControllerProfile = serde_json::from_str(include_str!(
            "../resources/controllers/pioneer-ddj-flx4.json"
        ))
        .unwrap();
        let registry = Registry::defaults(4);
        let mapping = p.mapping();
        let mut addresses = std::collections::HashSet::new();
        for b in &mapping.bindings {
            assert!(
                addresses.insert(format!("{}:{:?}", b.channel, b.input)),
                "duplicate binding {:?}",
                b
            );
            let up = mapping
                .resolve(&registry, b.channel, b.input, 127, 1.0, TAKEOVER)
                .expect("every binding has a registered implementation contract");
            assert!(up.value.is_finite());
            assert!(mapping
                .resolve(&registry, b.channel, b.input, 0, 0.0, TAKEOVER)
                .is_some());
        }
        let mut old = p.clone();
        old.bindings.truncate(26);
        old.bindings[0].control = compas_core::ControlId::new("deck.0.gain");
        let upgraded = upgrade_flx4_profile(old);
        assert_eq!(upgraded.bindings.len(), p.bindings.len());
        assert_eq!(upgraded.bindings[0].control.as_str(), "deck.0.gain");
        assert_eq!(
            upgrade_flx4_profile(upgraded).bindings.len(),
            p.bindings.len()
        );
    }
    #[test]
    fn saved_flx4_sampler_addresses_follow_the_deck_split() {
        let bundled: ControllerProfile = serde_json::from_str(include_str!(
            "../resources/controllers/pioneer-ddj-flx4.json"
        ))
        .unwrap();
        let mut saved = bundled.clone();
        let deck_two = saved
            .bindings
            .iter_mut()
            .find(|binding| binding.channel == 9 && binding.input == InputKind::Note { note: 48 })
            .expect("deck 2 sampler pad");
        assert_eq!(deck_two.control.as_str(), "sampler.8.trigger");
        deck_two.control = compas_core::ControlId::new("sampler.0.trigger");
        let remapped = saved
            .bindings
            .iter_mut()
            .find(|binding| binding.channel == 10 && binding.input == InputKind::Note { note: 48 })
            .expect("deck 2 sampler stop");
        remapped.control = compas_core::ControlId::new("deck.0.gain");
        let upgraded = upgrade_flx4_profile(saved);
        assert!(upgraded.bindings.iter().any(|binding| {
            binding.channel == 9
                && binding.input == InputKind::Note { note: 48 }
                && binding.control.as_str() == "sampler.8.trigger"
        }));
        assert!(upgraded.bindings.iter().any(|binding| {
            binding.channel == 10
                && binding.input == InputKind::Note { note: 48 }
                && binding.control.as_str() == "deck.0.gain"
        }));
        assert!(upgraded.bindings.iter().any(|binding| {
            binding.channel == 7
                && binding.input == InputKind::Note { note: 48 }
                && binding.control.as_str() == "sampler.0.trigger"
        }));
    }
    #[test]
    fn saved_flx4_shift_call_upgrades_only_the_old_defaults() {
        let bundled: ControllerProfile = serde_json::from_str(include_str!(
            "../resources/controllers/pioneer-ddj-flx4.json"
        ))
        .unwrap();
        let mut saved = bundled.clone();
        for binding in saved.bindings.iter_mut() {
            if binding.channel == 0 && binding.input == (InputKind::Note { note: 61 }) {
                binding.control = compas_core::ControlId::new("deck.0.jump.forward");
            }
            if binding.channel == 0 && binding.input == (InputKind::Note { note: 62 }) {
                binding.control = compas_core::ControlId::new("deck.0.gain");
            }
            if binding.channel == 0 && binding.input == (InputKind::Note { note: 80 }) {
                binding.control = compas_core::ControlId::new("deck.0.loop.reloop_stop");
            }
        }
        let upgraded = upgrade_flx4_profile(saved);
        assert!(upgraded.bindings.iter().any(|binding| {
            binding.channel == 0
                && binding.input == InputKind::Note { note: 61 }
                && binding.control.as_str() == "deck.0.loop.memory_save"
        }));
        assert!(upgraded.bindings.iter().any(|binding| {
            binding.channel == 0
                && binding.input == InputKind::Note { note: 62 }
                && binding.control.as_str() == "deck.0.gain"
        }));
        assert!(upgraded.bindings.iter().any(|binding| {
            binding.channel == 0
                && binding.input == InputKind::Note { note: 80 }
                && binding.control.as_str() == "deck.0.loop.memory_active"
        }));
    }
    #[test]
    fn flx4_meter_encodes_silence_levels_and_clip() {
        for (input, expected) in [
            (0.0, 0),
            (0.02, 0x26),
            (0.1, 0x41),
            (0.3, 0x57),
            (0.7, 0x65),
            (1.0, 0x77),
            (2.0, 0x77),
        ] {
            assert_eq!(flx4_meter_value(input), expected);
        }
        assert_eq!(flx4_meter_value(f32::NAN), 0);
        assert_eq!(flx4_meter_value(-1.0), 0);
        assert!((0..200)
            .map(|i| flx4_meter_value(i as f32 / 100.0))
            .collect::<Vec<_>>()
            .windows(2)
            .all(|w| w[0] <= w[1]));
    }

    #[test]
    fn flx4_trim_and_fader_use_distinct_vendor_addresses_and_ranges() {
        let profile: ControllerProfile = serde_json::from_str(include_str!(
            "../resources/controllers/pioneer-ddj-flx4.json"
        ))
        .unwrap();
        let map = profile.mapping();
        let registry = Registry::defaults(4);
        for deck in 0..2 {
            let trim = map
                .resolve(&registry, deck, InputKind::Cc { cc: 4 }, 127, 0.0, TAKEOVER)
                .unwrap();
            let volume = map
                .resolve(
                    &registry,
                    deck,
                    InputKind::Cc { cc: 19 },
                    127,
                    0.0,
                    TAKEOVER,
                )
                .unwrap();
            assert_eq!(trim.control.as_str(), format!("deck.{deck}.trim"));
            assert_eq!(trim.value, compas_core::control::TRIM_MAX_GAIN);
            assert_eq!(volume.control.as_str(), format!("deck.{deck}.volume"));
            assert_eq!(volume.value, 1.0);
            assert_eq!(
                map.resolve(&registry, deck, InputKind::Cc { cc: 19 }, 0, 1.0, TAKEOVER)
                    .unwrap()
                    .value,
                0.0
            );
        }
    }

    #[test]
    fn feedback_dedups_per_address() {
        let mut last = HashMap::new();
        let a = [0xB0, 7]; // CC 7 on ch 0
        let b = [0x90, 36]; // note 36 on ch 0
        assert!(feedback_changed(&mut last, a, 100)); // first value sends
        assert!(!feedback_changed(&mut last, a, 100)); // same value is skipped
        assert!(feedback_changed(&mut last, a, 101)); // a change sends
        assert!(feedback_changed(&mut last, b, 100)); // a different address is independent
        assert!(!feedback_changed(&mut last, b, 100));
    }

    #[test]
    fn midi_bytes_builds_cc_and_note() {
        assert_eq!(
            midi_bytes(0, InputKind::Cc { cc: 7 }, 100),
            Some([0xB0, 7, 100])
        );
        assert_eq!(
            midi_bytes(2, InputKind::Note { note: 36 }, 127),
            Some([0x92, 36, 127])
        );
        // HID inputs have no MIDI feedback representation.
        assert_eq!(midi_bytes(0, InputKind::Hid { byte: 3 }, 100), None);
    }

    #[test]
    fn bundled_profiles_parse_and_target_real_controls() {
        // Every binding in every shipped profile must resolve to a control the engine exposes —
        // a typo'd id silently no-ops at runtime, so catch it here. Also guards that the files are
        // valid JSON / `ControllerProfile`.
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("resources/controllers");
        let profiles = list_profiles(&dir);
        assert!(
            profiles.len() >= 3,
            "expected the bundled starter profiles, found {}",
            profiles.len()
        );
        let registry = Registry::defaults(DECK_COUNT);
        for p in &profiles {
            assert!(!p.id.is_empty(), "profile is missing an id");
            assert!(!p.bindings.is_empty(), "{} has no bindings", p.id);
            for b in &p.bindings {
                assert!(
                    registry.get(&b.control).is_some(),
                    "{} binds unknown control {:?}",
                    p.id,
                    b.control.as_str()
                );
            }
        }
    }

    #[test]
    fn save_then_list_round_trips() {
        let tmp = std::env::temp_dir().join(format!("compas-ctrl-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).unwrap();

        let mut p = ControllerProfile {
            id: "vendor model v1!".into(), // sanitized to vendor-model-v1-
            name: "Vendor Model".into(),
            ..Default::default()
        };
        p.bindings.push(compas_core::Binding {
            channel: 0,
            input: InputKind::Cc { cc: 7 },
            control: "deck.0.gain".into(),
            soft_takeover: false,
        });

        let path = save_profile(&tmp, &p).unwrap();
        assert!(path.exists());
        let listed = list_profiles(&tmp);
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].name, "Vendor Model");
        assert_eq!(listed[0].bindings.len(), 1);

        // A malformed file is skipped, not fatal.
        fs::write(tmp.join("broken.json"), "{ not json").unwrap();
        assert_eq!(list_profiles(&tmp).len(), 1);

        let _ = fs::remove_dir_all(&tmp);
    }

    #[cfg(unix)]
    #[test]
    fn startup_profile_replace_keeps_the_previous_file_when_the_new_write_cannot_start() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = std::env::temp_dir().join(format!(
            "compas-ctrl-atomic-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|elapsed| elapsed.as_nanos())
                .unwrap_or(0)
        ));
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).unwrap();
        let mut profile = ControllerProfile {
            id: "desk".into(),
            name: "Desk".into(),
            ..Default::default()
        };
        save_profile(&tmp, &profile).unwrap();
        let saved = fs::read(tmp.join("desk.json")).unwrap();
        let mut permissions = fs::metadata(&tmp).unwrap().permissions();
        permissions.set_mode(0o555);
        fs::set_permissions(&tmp, permissions.clone()).unwrap();
        profile.name = "Replaced".into();
        let failed = save_profile(&tmp, &profile);
        permissions.set_mode(0o755);
        fs::set_permissions(&tmp, permissions).unwrap();
        assert!(failed.is_err());
        assert_eq!(fs::read(tmp.join("desk.json")).unwrap(), saved);
        profile.name = "Replaced".into();
        save_profile(&tmp, &profile).unwrap();
        assert_eq!(list_profiles(&tmp)[0].name, "Replaced");
        let leftovers = fs::read_dir(&tmp).unwrap().flatten().any(|entry| {
            entry
                .path()
                .extension()
                .and_then(|extension| extension.to_str())
                == Some("tmp")
        });
        assert!(!leftovers);
        let _ = fs::remove_dir_all(&tmp);
    }

    fn sample_profile(id: &str, cc: u8) -> ControllerProfile {
        let mut p = ControllerProfile {
            id: id.into(),
            name: format!("Profile {id}"),
            ..Default::default()
        };
        p.bindings.push(compas_core::Binding {
            channel: 0,
            input: InputKind::Cc { cc },
            control: "deck.0.gain".into(),
            soft_takeover: true,
        });
        p
    }

    #[test]
    fn profile_pack_round_trips_through_json_and_imports() {
        let pack = pack_profiles(vec![sample_profile("dev-a", 7), sample_profile("dev-b", 8)]);
        assert_eq!(pack.version, PACK_VERSION);
        assert_eq!(pack.app, crate::export::MANIFEST_APP);

        // Survives a JSON serialize/parse cycle unchanged (compared via re-serialization, since
        // ControllerProfile doesn't implement PartialEq).
        let json = serde_json::to_string_pretty(&pack).unwrap();
        let parsed: ProfilePack = serde_json::from_str(&json).unwrap();
        assert_eq!(serde_json::to_string_pretty(&parsed).unwrap(), json);

        // Importing writes both profiles into a fresh user dir.
        let tmp = std::env::temp_dir().join(format!("compas-pack-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).unwrap();
        let ids = import_pack(&tmp, &parsed).unwrap();
        assert_eq!(ids, vec!["dev-a".to_string(), "dev-b".to_string()]);
        let listed = list_profiles(&tmp);
        assert_eq!(listed.len(), 2);
        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn import_pack_rejects_an_unknown_version() {
        let tmp = std::env::temp_dir().join(format!("compas-pack-ver-{}", std::process::id()));
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).unwrap();
        let pack = ProfilePack {
            version: PACK_VERSION + 1,
            app: crate::export::MANIFEST_APP.to_string(),
            profiles: vec![sample_profile("dev-a", 7)],
        };
        assert!(import_pack(&tmp, &pack).is_err());
        // Nothing was written.
        assert!(list_profiles(&tmp).is_empty());
        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn coalesced_batch_keeps_only_the_latest_value_per_control() {
        let mut pending = Vec::new();
        upsert_latest(
            &mut pending,
            ControllerUpdateEvent::new("deck.0.volume", 0.2),
        );
        upsert_latest(
            &mut pending,
            ControllerUpdateEvent::new("deck.1.volume", 0.4),
        );
        upsert_latest(
            &mut pending,
            ControllerUpdateEvent::new("deck.0.volume", 0.9),
        );
        let latest: Vec<(&str, f64)> = pending
            .iter()
            .map(|update| (update.control.as_str(), update.value))
            .collect();
        assert_eq!(latest, vec![("deck.0.volume", 0.9), ("deck.1.volume", 0.4)]);
    }

    #[test]
    fn state_leds_cover_pads_and_transport_but_not_continuous_controls() {
        let state_led_controls = [
            "deck.0.play",
            "deck.1.sync",
            "deck.0.headphone_cue",
            "deck.0.hotcue.3",
            "deck.1.beatloop.7",
            "deck.0.key_shift.2",
            "deck.1.loop.in",
            "deck.0.cue",
            "deck.0.beatjump.scale_up",
            "sampler.4.trigger",
        ];
        let momentary_controls = [
            "deck.0.volume",
            "deck.1.eq.low",
            "mixer.crossfader",
            "deck.0.censor",
            "fx.next",
        ];
        assert!(state_led_controls
            .iter()
            .all(|control| has_state_led(control)));
        assert!(momentary_controls
            .iter()
            .all(|control| !has_state_led(control)));
        assert!(is_native_transport_led("deck.1.play"));
        assert!(!is_native_transport_led("deck.1.cue"));
    }

    #[test]
    fn tempo_range_cycles_and_scales_the_existing_fader_span() {
        assert_eq!(next_tempo_range(8.0), 6.0);
        assert_eq!(next_tempo_range(6.0), 10.0);
        assert_eq!(next_tempo_range(10.0), 16.0);
        assert_eq!(next_tempo_range(16.0), 100.0);
        assert_eq!(next_tempo_range(100.0), 8.0);
        assert!((scale_tempo_percent(8.0, 8.0) - 8.0).abs() < 1e-9);
        assert!((scale_tempo_percent(8.0, 16.0) - 16.0).abs() < 1e-9);
        assert!((scale_tempo_percent(-8.0, 6.0) + 6.0).abs() < 1e-9);
        assert_eq!(scale_tempo_percent(8.0, 100.0), 100.0);
        assert!(tempo_deck("deck.0.tempo").is_some());
        assert!(tempo_deck("deck.0.tempo_range").is_none());
    }
    #[test]
    fn flx4_special_noteoff_clears_holds_and_loop_adjustment() {
        for channel in [0, 1] {
            for release_velocity in [0, 64, 127] {
                // All special handlers receive exactly the same canonical release as
                // a zero-velocity NoteOn, independent of NoteOff release velocity.
                assert_eq!(
                    normalize_note_release(0x80 | channel, release_velocity),
                    normalize_note_release(0x90 | channel, 0)
                );
            }
        }
        assert_eq!(normalize_note_release(0x90, 127), (0x90, 127));
        assert_eq!(normalize_note_release(0x91, 0), (0x91, 0));
        assert_eq!(normalize_note_release(0x87, 64), (0x97, 0));
        assert_eq!(normalize_note_release(0x88, 0), (0x98, 0));
        assert_eq!(normalize_note_release(0x89, 127), (0x99, 0));
        assert_eq!(normalize_note_release(0x9A, 0), (0x9A, 0));
        assert_eq!(sampler_local("sampler.8.stop"), Some(8));
        assert_eq!(sampler_local("sampler.gain"), None);
        assert_eq!(compas_audio::sampler_global_slot(1, 8), Some(24));
        assert_eq!(compas_audio::sampler_global_slot(0, 0), Some(0));
        assert!(keyboard_control("deck.0.keyboard.4").is_some());
        assert_eq!(
            keyboard_control("deck.1.keyboard_shift.7"),
            Some((1, 7, true))
        );
        assert!(keyboard_control("deck.0.key_shift.1").is_none());
    }
}

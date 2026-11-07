//! Native controller Beat FX/key shift. Control-thread only; never call from audio callbacks.
//! Semantic parameters match useBeatFx/useDeck. Mutex ownership covers command enqueue and state
//! commit so concurrent UI setters cannot reorder controller state relative to engine commands.
use crate::EngineMsg;
use compas_audio::NUM_DECKS;
use serde::{Deserialize, Serialize};
use std::sync::Mutex;

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Effect {
    Echo,
    Flanger,
    Reverb,
    Crusher,
}
impl Effect {
    fn index(self) -> usize {
        match self {
            Self::Echo => 0,
            Self::Flanger => 1,
            Self::Reverb => 2,
            Self::Crusher => 3,
        }
    }
    fn cycle(self, direction: i32) -> Self {
        [Self::Echo, Self::Flanger, Self::Reverb, Self::Crusher]
            [(self.index() as i32 + direction).rem_euclid(4) as usize]
    }
}
#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
pub(crate) struct Echo {
    pub active: bool,
    pub beats: f64,
    pub depth: f64,
}
#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
pub(crate) struct Flanger {
    pub active: bool,
    pub beats: f64,
    pub depth: f64,
}
#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
pub(crate) struct Reverb {
    pub active: bool,
    pub size: f64,
    pub mix: f64,
}
#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
pub(crate) struct Crusher {
    pub active: bool,
    pub crush: f64,
    pub down: f64,
}
#[derive(Clone, Copy, Debug, Serialize)]
pub(crate) struct DeckEffects {
    pub echo: Echo,
    pub flanger: Flanger,
    pub reverb: Reverb,
    pub crusher: Crusher,
    pub pitch_shift: f64,
    pub beat_sec: f64,
    /// Offset added to the key-shift pad scale. SHIFT pads 7 and 8 move it by one semitone.
    pub key_transpose: i8,
    /// Last key-shift pad pressed, so its LED stays on after the scale moves.
    pub key_pad: Option<u8>,
}
impl Default for DeckEffects {
    fn default() -> Self {
        Self {
            echo: Echo {
                active: false,
                beats: 0.5,
                depth: 0.5,
            },
            flanger: Flanger {
                active: false,
                beats: 4.0,
                depth: 0.6,
            },
            reverb: Reverb {
                active: false,
                size: 0.6,
                mix: 0.3,
            },
            crusher: Crusher {
                active: false,
                crush: 0.5,
                down: 0.3,
            },
            pitch_shift: 0.0,
            beat_sec: 0.5,
            key_transpose: 0,
            key_pad: None,
        }
    }
}

const KEY_SHIFT_SCALE: [i16; 8] = [4, 5, 6, 7, 0, 1, 2, 3];
impl DeckEffects {
    fn active(&self, e: Effect) -> bool {
        match e {
            Effect::Echo => self.echo.active,
            Effect::Flanger => self.flanger.active,
            Effect::Reverb => self.reverb.active,
            Effect::Crusher => self.crusher.active,
        }
    }
    fn set_active(&mut self, e: Effect, active: bool) {
        match e {
            Effect::Echo => self.echo.active = active,
            Effect::Flanger => self.flanger.active = active,
            Effect::Reverb => self.reverb.active = active,
            Effect::Crusher => self.crusher.active = active,
        }
    }
    fn command(&self, deck: usize, e: Effect) -> EngineMsg {
        match e {
            Effect::Echo => EngineMsg::DeckEcho {
                deck,
                active: self.echo.active,
                time_sec: (self.beat_sec * self.echo.beats) as f32,
                feedback: (0.15 + self.echo.depth * 0.7) as f32,
                mix: if self.echo.active {
                    (self.echo.depth * 0.5) as f32
                } else {
                    0.0
                },
            },
            Effect::Flanger => EngineMsg::DeckFlanger {
                deck,
                active: self.flanger.active,
                rate_hz: (1.0 / (self.beat_sec * self.flanger.beats)) as f32,
                depth: self.flanger.depth as f32,
                feedback: (0.35 + self.flanger.depth * 0.5) as f32,
                mix: if self.flanger.active { 0.5 } else { 0.0 },
            },
            Effect::Reverb => EngineMsg::DeckReverb {
                deck,
                active: self.reverb.active,
                room_size: self.reverb.size as f32,
                mix: if self.reverb.active {
                    self.reverb.mix as f32
                } else {
                    0.0
                },
            },
            Effect::Crusher => EngineMsg::DeckCrusher {
                deck,
                active: self.crusher.active,
                bits: (16.0 - self.crusher.crush * 14.0).round() as f32,
                downsample: 1 + (self.crusher.down * 31.0).round() as u32,
                mix: if self.crusher.active { 1.0 } else { 0.0 },
            },
        }
    }
}
#[derive(Clone, Debug, Serialize)]
pub(crate) struct EffectsSnapshot {
    pub revision: u64,
    pub effect: Effect,
    pub deck: usize,
    pub decks: [DeckEffects; NUM_DECKS],
    /// Beat FX and the effect panel. Separate from a pad that is physically held.
    #[serde(skip)]
    latched: [u8; NUM_DECKS],
    /// Inserts a Pad FX pad turned on and must turn off on release or mode exit.
    #[serde(skip)]
    pad_owned: [u8; NUM_DECKS],
    /// Which preset currently owns each insert, plus the panel parameters to restore.
    #[serde(skip)]
    pad_hold: [[Option<PadHold>; 4]; NUM_DECKS],
}
impl Default for EffectsSnapshot {
    fn default() -> Self {
        Self {
            revision: 0,
            effect: Effect::Echo,
            deck: 0,
            decks: [DeckEffects::default(); NUM_DECKS],
            latched: [0; NUM_DECKS],
            pad_owned: [0; NUM_DECKS],
            pad_hold: [[None; 4]; NUM_DECKS],
        }
    }
}

const PAD_FX: [Effect; 4] = [
    Effect::Echo,
    Effect::Flanger,
    Effect::Reverb,
    Effect::Crusher,
];

/// Open Deck parameter choices for the host-assigned Pad FX slots in the DDJ-FLX4 manual.
/// Slots 0–3 keep the panel parameters. The rest are presets of the same four inserts.
#[derive(Clone, Copy, Debug)]
enum PadFxSound {
    Panel(Effect),
    Echo { beats: f64, depth: f64 },
    Flanger { beats: f64, depth: f64 },
    Reverb { size: f64, mix: f64 },
    Crusher { crush: f64, down: f64 },
}

impl PadFxSound {
    fn effect(self) -> Effect {
        match self {
            Self::Panel(effect) => effect,
            Self::Echo { .. } => Effect::Echo,
            Self::Flanger { .. } => Effect::Flanger,
            Self::Reverb { .. } => Effect::Reverb,
            Self::Crusher { .. } => Effect::Crusher,
        }
    }
}

fn pad_fx_sound(slot: usize) -> Option<PadFxSound> {
    Some(match slot {
        0 => PadFxSound::Panel(Effect::Echo),
        1 => PadFxSound::Panel(Effect::Flanger),
        2 => PadFxSound::Panel(Effect::Reverb),
        3 => PadFxSound::Panel(Effect::Crusher),
        4 => PadFxSound::Echo {
            beats: 1.0,
            depth: 0.75,
        },
        5 => PadFxSound::Flanger {
            beats: 1.0,
            depth: 0.8,
        },
        6 => PadFxSound::Reverb {
            size: 0.9,
            mix: 0.55,
        },
        7 => PadFxSound::Crusher {
            crush: 0.85,
            down: 0.6,
        },
        8 => PadFxSound::Echo {
            beats: 0.25,
            depth: 0.4,
        },
        9 => PadFxSound::Flanger {
            beats: 8.0,
            depth: 0.45,
        },
        10 => PadFxSound::Reverb {
            size: 0.35,
            mix: 0.25,
        },
        11 => PadFxSound::Crusher {
            crush: 0.25,
            down: 0.15,
        },
        12 => PadFxSound::Echo {
            beats: 2.0,
            depth: 0.65,
        },
        13 => PadFxSound::Flanger {
            beats: 2.0,
            depth: 0.7,
        },
        14 => PadFxSound::Reverb {
            size: 0.75,
            mix: 0.7,
        },
        15 => PadFxSound::Crusher {
            crush: 0.7,
            down: 0.85,
        },
        16 => PadFxSound::Echo {
            beats: 4.0,
            depth: 0.55,
        },
        17 => PadFxSound::Flanger {
            beats: 0.5,
            depth: 0.5,
        },
        18 => PadFxSound::Reverb {
            size: 0.5,
            mix: 0.45,
        },
        19 => PadFxSound::Crusher {
            crush: 0.6,
            down: 0.4,
        },
        20 => PadFxSound::Echo {
            beats: 0.125,
            depth: 0.8,
        },
        21 => PadFxSound::Flanger {
            beats: 16.0,
            depth: 0.35,
        },
        22 => PadFxSound::Reverb {
            size: 1.0,
            mix: 0.4,
        },
        23 => PadFxSound::Crusher {
            crush: 0.95,
            down: 0.2,
        },
        24 => PadFxSound::Echo {
            beats: 0.5,
            depth: 0.9,
        },
        25 => PadFxSound::Flanger {
            beats: 0.25,
            depth: 0.9,
        },
        26 => PadFxSound::Reverb {
            size: 0.2,
            mix: 0.8,
        },
        27 => PadFxSound::Crusher {
            crush: 0.4,
            down: 0.95,
        },
        28 => PadFxSound::Echo {
            beats: 8.0,
            depth: 0.35,
        },
        29 => PadFxSound::Flanger {
            beats: 6.0,
            depth: 0.55,
        },
        30 => PadFxSound::Reverb {
            size: 0.85,
            mix: 0.2,
        },
        31 => PadFxSound::Crusher {
            crush: 0.15,
            down: 0.5,
        },
        _ => return None,
    })
}

#[derive(Clone, Copy, Debug)]
enum SavedEffect {
    Echo(Echo),
    Flanger(Flanger),
    Reverb(Reverb),
    Crusher(Crusher),
}

#[derive(Clone, Copy, Debug)]
struct PadHold {
    preset: u8,
    saved: SavedEffect,
}

fn saved_effect(deck: &DeckEffects, effect: Effect) -> SavedEffect {
    match effect {
        Effect::Echo => SavedEffect::Echo(deck.echo),
        Effect::Flanger => SavedEffect::Flanger(deck.flanger),
        Effect::Reverb => SavedEffect::Reverb(deck.reverb),
        Effect::Crusher => SavedEffect::Crusher(deck.crusher),
    }
}

fn apply_pad_sound(deck: &mut DeckEffects, sound: PadFxSound) {
    match sound {
        PadFxSound::Panel(effect) => deck.set_active(effect, true),
        PadFxSound::Echo { beats, depth } => {
            deck.echo.beats = beats;
            deck.echo.depth = depth;
            deck.echo.active = true;
        }
        PadFxSound::Flanger { beats, depth } => {
            deck.flanger.beats = beats;
            deck.flanger.depth = depth;
            deck.flanger.active = true;
        }
        PadFxSound::Reverb { size, mix } => {
            deck.reverb.size = size;
            deck.reverb.mix = mix;
            deck.reverb.active = true;
        }
        PadFxSound::Crusher { crush, down } => {
            deck.crusher.crush = crush;
            deck.crusher.down = down;
            deck.crusher.active = true;
        }
    }
}

fn restore_saved(deck: &mut DeckEffects, saved: SavedEffect, active: bool) {
    match saved {
        SavedEffect::Echo(mut echo) => {
            echo.active = active;
            deck.echo = echo;
        }
        SavedEffect::Flanger(mut flanger) => {
            flanger.active = active;
            deck.flanger = flanger;
        }
        SavedEffect::Reverb(mut reverb) => {
            reverb.active = active;
            deck.reverb = reverb;
        }
        SavedEffect::Crusher(mut crusher) => {
            crusher.active = active;
            deck.crusher = crusher;
        }
    }
}

fn hold_bits(holds: &[Option<PadHold>; 4]) -> u8 {
    holds.iter().enumerate().fold(0, |bits, (index, hold)| {
        if hold.is_some() {
            bits | (1 << index)
        } else {
            bits
        }
    })
}

fn effect_bit(effect: Effect) -> u8 {
    1 << effect.index()
}

fn set_latch(latched: &mut u8, effect: Effect, on: bool) {
    let bit = effect_bit(effect);
    if on {
        *latched |= bit;
    } else {
        *latched &= !bit;
    }
}

fn audible(latched: u8, owned: u8, effect: Effect) -> bool {
    (latched | owned) & effect_bit(effect) != 0
}
/// Typed UI update. Update one effect only; never replace an entire deck using a stale UI snapshot.
#[derive(Clone, Copy, Deserialize)]
#[serde(tag = "effect", rename_all = "snake_case")]
pub(crate) enum EffectUpdate {
    Echo {
        active: bool,
        beats: f64,
        depth: f64,
    },
    Flanger {
        active: bool,
        beats: f64,
        depth: f64,
    },
    Reverb {
        active: bool,
        size: f64,
        mix: f64,
    },
    Crusher {
        active: bool,
        crush: f64,
        down: f64,
    },
    Pitch {
        semitones: f64,
    },
    /// Set on track grid/internal clock change. Re-sends active beat-synced effects only.
    BeatSec {
        seconds: f64,
    },
}
#[derive(Default)]
pub(crate) struct NativeEffects(Mutex<EffectsSnapshot>);
fn bounded(value: f64, min: f64, max: f64) -> Result<f64, String> {
    if value.is_finite() {
        Ok(value.clamp(min, max))
    } else {
        Err("Effect value must be finite".into())
    }
}
impl NativeEffects {
    pub(crate) fn snapshot(&self) -> Result<EffectsSnapshot, String> {
        Ok(self.0.lock().map_err(|e| e.to_string())?.clone())
    }
    /// Reset transport-owned FX state after load/eject. Re-send bypass and pitch reset while
    /// holding the authority lock, so a racing controller change cannot survive only in audio.
    /// The caller publishes the returned snapshot after enqueueing the load/unload command.
    pub(crate) fn reset_deck(
        &self,
        deck: usize,
        mut send: impl FnMut(EngineMsg) -> Result<(), String>,
    ) -> Result<EffectsSnapshot, String> {
        if deck >= NUM_DECKS {
            return Err("Invalid effects deck".into());
        }
        let mut state = self.0.lock().map_err(|e| e.to_string())?;
        let mut next = state.clone();
        next.latched[deck] = 0;
        next.pad_owned[deck] = 0;
        next.pad_hold[deck] = [None; 4];
        let d = &mut next.decks[deck];
        for effect in PAD_FX {
            d.set_active(effect, false);
            send(d.command(deck, effect))?;
        }
        d.pitch_shift = 0.0;
        d.key_transpose = 0;
        d.key_pad = None;
        send(EngineMsg::DeckPitchShift {
            deck,
            semitones: 0.0,
        })?;
        next.revision = state.revision.wrapping_add(1);
        *state = next;
        Ok(state.clone())
    }

    /// Mirror a newly created mixer. Lifecycle code must exclude controller commands while
    /// replacing the engine and resetting this memory; no engine commands are sent here.
    pub(crate) fn reset_all(&self) -> Result<EffectsSnapshot, String> {
        let mut state = self.0.lock().map_err(|e| e.to_string())?;
        state.latched = [0; NUM_DECKS];
        state.pad_owned = [0; NUM_DECKS];
        state.pad_hold = [[None; 4]; NUM_DECKS];
        for d in &mut state.decks {
            for effect in PAD_FX {
                d.set_active(effect, false);
            }
            d.pitch_shift = 0.0;
            d.key_transpose = 0;
            d.key_pad = None;
        }
        state.revision = state.revision.wrapping_add(1);
        Ok(state.clone())
    }

    /// Enqueue under the same lock as controller changes. The caller must emit the returned
    /// snapshot (effects:update) and the frontend must ignore older/equal revisions.
    pub(crate) fn update(
        &self,
        deck: usize,
        update: EffectUpdate,
        mut send: impl FnMut(EngineMsg) -> Result<(), String>,
    ) -> Result<EffectsSnapshot, String> {
        if deck >= NUM_DECKS {
            return Err("Invalid effects deck".into());
        }
        let mut state = self.0.lock().map_err(|e| e.to_string())?;
        let mut next = state.clone();
        let mut latched = next.latched[deck];
        let owned = next.pad_owned[deck];
        let d = &mut next.decks[deck];
        let effect = match update {
            EffectUpdate::Echo {
                active,
                beats,
                depth,
            } => {
                d.echo = Echo {
                    active,
                    beats: bounded(beats, 0.0625, 16.0)?,
                    depth: bounded(depth, 0.0, 1.0)?,
                };
                Some(Effect::Echo)
            }
            EffectUpdate::Flanger {
                active,
                beats,
                depth,
            } => {
                d.flanger = Flanger {
                    active,
                    beats: bounded(beats, 0.0625, 32.0)?,
                    depth: bounded(depth, 0.0, 1.0)?,
                };
                Some(Effect::Flanger)
            }
            EffectUpdate::Reverb { active, size, mix } => {
                d.reverb = Reverb {
                    active,
                    size: bounded(size, 0.0, 1.0)?,
                    mix: bounded(mix, 0.0, 1.0)?,
                };
                Some(Effect::Reverb)
            }
            EffectUpdate::Crusher {
                active,
                crush,
                down,
            } => {
                d.crusher = Crusher {
                    active,
                    crush: bounded(crush, 0.0, 1.0)?,
                    down: bounded(down, 0.0, 1.0)?,
                };
                Some(Effect::Crusher)
            }
            EffectUpdate::Pitch { semitones } => {
                d.pitch_shift = bounded(semitones, -12.0, 12.0)?.round();
                d.key_transpose = 0;
                d.key_pad = None;
                send(EngineMsg::DeckPitchShift {
                    deck,
                    semitones: d.pitch_shift,
                })?;
                None
            }
            EffectUpdate::BeatSec { seconds } => {
                d.beat_sec = bounded(seconds, 0.05, 6.0)?;
                for effect in [Effect::Echo, Effect::Flanger] {
                    if d.active(effect) {
                        send(d.command(deck, effect))?;
                    }
                }
                None
            }
        };
        if let Some(e) = effect {
            let requested = d.active(e);
            set_latch(&mut latched, e, requested);
            let on = audible(latched, owned, e);
            d.set_active(e, on);
            send(d.command(deck, e))?;
        }
        next.latched[deck] = latched;
        next.revision = state.revision.wrapping_add(1);
        *state = next;
        Ok(state.clone())
    }
    /// None means this control is not owned here. Owned releases/no-op buttons are consumed too.
    pub(crate) fn handle(
        &self,
        control: &str,
        value: f64,
        mut send: impl FnMut(EngineMsg) -> Result<(), String>,
    ) -> Result<Option<EffectsSnapshot>, String> {
        let pitch = control
            .strip_prefix("deck.")
            .and_then(|s| s.split_once('.'))
            .and_then(|(deck, name)| deck.parse::<usize>().ok().map(|deck| (deck, name)));
        let pitch = pitch.and_then(|(deck, name)| {
            name.strip_prefix("key_shift_step.")
                .map(|p| (deck, true, p))
                .or_else(|| name.strip_prefix("key_shift.").map(|p| (deck, false, p)))
        });
        if let Some((deck, slot)) = pad_fx_target(control) {
            return self.hold_pad_fx(deck, slot, value, send);
        }
        if !control.starts_with("fx.") && pitch.is_none() {
            return Ok(None);
        }
        bounded(value, -100.0, 100.0)?;
        let mut state = self.0.lock().map_err(|e| e.to_string())?;
        if value < 0.5 && control != "fx.level" {
            return Ok(Some(state.clone()));
        }
        let mut next = state.clone();
        if let Some((deck, relative, pad)) = pitch {
            let pad = pad
                .parse::<usize>()
                .ok()
                .filter(|p| *p < 8)
                .ok_or("Invalid key shift pad")?;
            let d = next.decks.get_mut(deck).ok_or("Invalid key shift deck")?;
            if relative {
                if pad < 6 {
                    return Ok(Some(state.clone()));
                }
                let delta: i8 = if pad == 6 { 1 } else { -1 };
                let transposed = d.key_transpose.saturating_add(delta).clamp(-12, 12);
                let applied = transposed - d.key_transpose;
                d.key_transpose = transposed;
                d.pitch_shift = (d.pitch_shift + f64::from(applied)).clamp(-12.0, 12.0);
            } else {
                d.key_pad = Some(pad as u8);
                d.pitch_shift = (f64::from(KEY_SHIFT_SCALE[pad]) + f64::from(d.key_transpose))
                    .clamp(-12.0, 12.0);
            }
            send(EngineMsg::DeckPitchShift {
                deck,
                semitones: d.pitch_shift,
            })?;
        } else if let Some((operation, deck)) = control
            .strip_prefix("fx.")
            .and_then(|s| s.split_once('.'))
            .filter(|(op, _)| matches!(*op, "channel" | "toggle"))
        {
            let deck = deck
                .parse::<usize>()
                .ok()
                .filter(|d| *d < NUM_DECKS)
                .ok_or("Invalid FX deck")?;
            next.deck = deck;
            if operation == "toggle" {
                let effect = next.effect;
                let mut latched = next.latched[deck];
                let was_latched = latched & effect_bit(effect) != 0;
                set_latch(&mut latched, effect, !was_latched);
                next.latched[deck] = latched;
                let on = audible(latched, next.pad_owned[deck], effect);
                let d = &mut next.decks[deck];
                d.set_active(effect, on);
                send(d.command(deck, effect))?;
            }
        } else {
            match control {
                "fx.level" => {
                    let d = &mut next.decks[next.deck];
                    let v = value.clamp(0.0, 1.0);
                    match next.effect {
                        Effect::Echo => d.echo.depth = v,
                        Effect::Flanger => d.flanger.depth = v,
                        Effect::Reverb => d.reverb.mix = v,
                        Effect::Crusher => d.crusher.crush = v,
                    };
                    send(d.command(next.deck, next.effect))?;
                }
                "fx.next" | "fx.prev" => {
                    let old = next.effect;
                    next.effect = old.cycle(if control == "fx.next" { 1 } else { -1 });
                    for deck in 0..NUM_DECKS {
                        if next.latched[deck] & effect_bit(old) == 0 {
                            continue;
                        }
                        let mut latched = next.latched[deck];
                        set_latch(&mut latched, old, false);
                        set_latch(&mut latched, next.effect, true);
                        next.latched[deck] = latched;
                        let owned = next.pad_owned[deck];
                        let d = &mut next.decks[deck];
                        for effect in [old, next.effect] {
                            let on = audible(latched, owned, effect);
                            if d.active(effect) != on {
                                d.set_active(effect, on);
                                send(d.command(deck, effect))?;
                            }
                        }
                    }
                }
                "fx.beat_down" | "fx.beat_up" => {
                    let d = &mut next.decks[next.deck];
                    let steps: &[f64] = match next.effect {
                        Effect::Echo => &[0.25, 0.5, 1., 2.],
                        Effect::Flanger => &[1., 2., 4., 8.],
                        _ => &[],
                    };
                    if !steps.is_empty() {
                        let beats = if next.effect == Effect::Echo {
                            &mut d.echo.beats
                        } else {
                            &mut d.flanger.beats
                        };
                        let i = steps.iter().position(|v| *v == *beats).unwrap_or(0) as isize;
                        *beats = steps[(i + if control == "fx.beat_up" { 1 } else { -1 })
                            .clamp(0, steps.len() as isize - 1)
                            as usize];
                        send(d.command(next.deck, next.effect))?;
                    }
                }
                "fx.off_all" => {
                    next.latched = [0; NUM_DECKS];
                    next.pad_owned = [0; NUM_DECKS];
                    for deck in 0..NUM_DECKS {
                        for effect in PAD_FX {
                            if let Some(hold) = next.pad_hold[deck][effect.index()].take() {
                                restore_saved(&mut next.decks[deck], hold.saved, false);
                            }
                            next.decks[deck].set_active(effect, false);
                            let command = next.decks[deck].command(deck, effect);
                            send(command)?;
                        }
                    }
                }
                _ => return Ok(Some(state.clone())),
            }
        }
        next.revision = state.revision.wrapping_add(1);
        *state = next;
        Ok(Some(state.clone()))
    }

    fn hold_pad_fx(
        &self,
        deck: usize,
        slot: usize,
        value: f64,
        mut send: impl FnMut(EngineMsg) -> Result<(), String>,
    ) -> Result<Option<EffectsSnapshot>, String> {
        let Some(sound) = (deck < NUM_DECKS && value.is_finite())
            .then_some(())
            .and_then(|()| pad_fx_sound(slot))
        else {
            return Err("Invalid pad FX".into());
        };
        let mut state = self.0.lock().map_err(|e| e.to_string())?;
        let effect = sound.effect();
        let effect_index = effect.index();
        let pressed = value >= 0.5;
        let current = state.pad_hold[deck][effect_index];
        if pressed {
            if current.is_some_and(|hold| hold.preset == slot as u8) {
                return Ok(Some(state.clone()));
            }
            if matches!(sound, PadFxSound::Panel(_))
                && current.is_none()
                && state.decks[deck].active(effect)
            {
                return Ok(Some(state.clone()));
            }
        } else if current.is_none_or(|hold| hold.preset != slot as u8) {
            return Ok(Some(state.clone()));
        }
        let mut next = state.clone();
        let latched = next.latched[deck] & effect_bit(effect) != 0;
        if pressed {
            let saved = current
                .map(|hold| hold.saved)
                .unwrap_or_else(|| saved_effect(&next.decks[deck], effect));
            apply_pad_sound(&mut next.decks[deck], sound);
            next.pad_hold[deck][effect_index] = Some(PadHold {
                preset: slot as u8,
                saved,
            });
        } else {
            let hold = next.pad_hold[deck][effect_index].take();
            if let Some(hold) = hold {
                restore_saved(&mut next.decks[deck], hold.saved, latched);
            }
        }
        next.pad_owned[deck] = hold_bits(&next.pad_hold[deck]);
        send(next.decks[deck].command(deck, effect))?;
        next.revision = state.revision.wrapping_add(1);
        *state = next;
        Ok(Some(state.clone()))
    }

    /// Set the deck pitch recorded for Beat FX / key shift and send it to the engine.
    pub(crate) fn set_pitch(
        &self,
        deck: usize,
        semitones: f64,
        mut send: impl FnMut(EngineMsg) -> Result<(), String>,
    ) -> Result<EffectsSnapshot, String> {
        if deck >= NUM_DECKS || !semitones.is_finite() {
            return Err("Invalid pitch".into());
        }
        let semitones = semitones.clamp(-12.0, 12.0);
        let mut state = self.0.lock().map_err(|e| e.to_string())?;
        if (state.decks[deck].pitch_shift - semitones).abs() < 1e-9 {
            return Ok(state.clone());
        }
        let mut next = state.clone();
        next.decks[deck].pitch_shift = semitones;
        next.decks[deck].key_pad = None;
        send(EngineMsg::DeckPitchShift { deck, semitones })?;
        next.revision = state.revision.wrapping_add(1);
        *state = next;
        Ok(state.clone())
    }

    /// Drop every pad-owned insert on one deck. A Beat FX latch stays on.
    pub(crate) fn release_pad_fx(
        &self,
        deck: usize,
        mut send: impl FnMut(EngineMsg) -> Result<(), String>,
    ) -> Result<Option<EffectsSnapshot>, String> {
        if deck >= NUM_DECKS {
            return Err("Invalid pad FX deck".into());
        }
        let mut state = self.0.lock().map_err(|e| e.to_string())?;
        if state.pad_owned[deck] == 0 {
            return Ok(None);
        }
        let mut next = state.clone();
        for effect in PAD_FX {
            let Some(hold) = next.pad_hold[deck][effect.index()].take() else {
                continue;
            };
            let on = next.latched[deck] & effect_bit(effect) != 0;
            restore_saved(&mut next.decks[deck], hold.saved, on);
            send(next.decks[deck].command(deck, effect))?;
        }
        next.pad_owned[deck] = 0;
        next.revision = state.revision.wrapping_add(1);
        *state = next;
        Ok(Some(state.clone()))
    }
}

fn pad_fx_target(control: &str) -> Option<(usize, usize)> {
    let (deck, name) = control.strip_prefix("deck.")?.split_once('.')?;
    let slot = name.strip_prefix("pad_fx.")?;
    Some((deck.parse().ok()?, slot.parse().ok()?))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn load_reset_preserves_knobs_bypasses_slots_and_next_toggle_engages() {
        let state = NativeEffects::default();
        state
            .update(
                0,
                EffectUpdate::Echo {
                    active: true,
                    beats: 2.0,
                    depth: 0.9,
                },
                |_| Ok(()),
            )
            .unwrap();
        state
            .update(0, EffectUpdate::Pitch { semitones: 4.0 }, |_| Ok(()))
            .unwrap();
        let prior = state.snapshot().unwrap().revision;
        let mut sent = Vec::new();
        let s = state
            .reset_deck(0, |m| {
                sent.push(m);
                Ok(())
            })
            .unwrap();
        assert_eq!(sent.len(), 5);
        assert!(matches!(
            sent[0],
            EngineMsg::DeckEcho {
                active: false,
                mix: 0.0,
                ..
            }
        ));
        assert!(matches!(
            sent[4],
            EngineMsg::DeckPitchShift { semitones: 0.0, .. }
        ));
        assert!(!s.decks[0].echo.active);
        assert_eq!(s.decks[0].echo.depth, 0.9);
        assert_eq!(s.decks[0].echo.beats, 2.0);
        assert_eq!(s.decks[0].pitch_shift, 0.0);
        assert!(s.revision > prior);
        assert!(
            state
                .handle("fx.toggle.0", 1.0, |_| Ok(()))
                .unwrap()
                .unwrap()
                .decks[0]
                .echo
                .active
        );
    }

    #[test]
    fn reset_all_preserves_parameters_and_invalid_or_failed_reset_does_not_commit() {
        let state = NativeEffects::default();
        state
            .update(
                1,
                EffectUpdate::Reverb {
                    active: true,
                    size: 0.8,
                    mix: 0.2,
                },
                |_| Ok(()),
            )
            .unwrap();
        state
            .update(1, EffectUpdate::Pitch { semitones: -2.0 }, |_| Ok(()))
            .unwrap();
        let prior = state.snapshot().unwrap().revision;
        assert!(state
            .reset_deck(NUM_DECKS, |_| panic!("invalid deck sent"))
            .is_err());
        assert!(state.reset_deck(1, |_| Err("offline".into())).is_err());
        assert_eq!(state.snapshot().unwrap().revision, prior);
        let s = state.reset_all().unwrap();
        assert_eq!(s.revision, prior + 1);
        assert!(!s.decks[1].reverb.active);
        assert_eq!(s.decks[1].reverb.size, 0.8);
        assert_eq!(s.decks[1].pitch_shift, 0.0);
    }

    #[test]
    fn off_all_sends_bypass_even_when_authority_already_reports_inactive() {
        let state = NativeEffects::default();
        let mut count = 0;
        state
            .handle("fx.off_all", 1.0, |_| {
                count += 1;
                Ok(())
            })
            .unwrap();
        assert_eq!(count, NUM_DECKS * 4);
    }

    #[test]
    fn defaults_and_echo_toggle_match_ui_parameters() {
        let state = NativeEffects::default();
        let mut sent = Vec::new();
        let snapshot = state
            .handle("fx.toggle.0", 1.0, |m| {
                sent.push(m);
                Ok(())
            })
            .unwrap()
            .unwrap();
        assert!(snapshot.decks[0].echo.active);
        assert!(
            matches!(sent[0],EngineMsg::DeckEcho{deck:0,active:true,time_sec,feedback,mix} if (time_sec-0.25).abs()<1e-6 && (feedback-0.5).abs()<1e-6 && (mix-0.25).abs()<1e-6)
        );
        let revision = snapshot.revision;
        assert_eq!(
            state
                .handle("fx.toggle.0", 0.0, |_| panic!("release sent"))
                .unwrap()
                .unwrap()
                .revision,
            revision
        );
    }
    #[test]
    fn cycle_moves_active_effects_on_all_decks_and_off_all_clears() {
        let state = NativeEffects::default();
        for deck in [0, 1] {
            state
                .handle(&format!("fx.toggle.{deck}"), 1.0, |_| Ok(()))
                .unwrap();
        }
        let s = state.handle("fx.next", 1.0, |_| Ok(())).unwrap().unwrap();
        assert_eq!(s.effect, Effect::Flanger);
        assert!(s.decks[..2]
            .iter()
            .all(|d| !d.echo.active && d.flanger.active));
        let s = state
            .handle("fx.off_all", 1.0, |_| Ok(()))
            .unwrap()
            .unwrap();
        assert!(s
            .decks
            .iter()
            .all(|d| !d.echo.active && !d.flanger.active && !d.reverb.active && !d.crusher.active));
    }
    #[test]
    fn partial_ui_update_preserves_other_effect_and_inactive_mix() {
        let state = NativeEffects::default();
        state
            .update(
                0,
                EffectUpdate::Reverb {
                    active: false,
                    size: 0.8,
                    mix: 0.77,
                },
                |_| Ok(()),
            )
            .unwrap();
        state
            .update(
                0,
                EffectUpdate::Echo {
                    active: true,
                    beats: 2.0,
                    depth: 0.2,
                },
                |_| Ok(()),
            )
            .unwrap();
        state.handle("fx.next", 1.0, |_| Ok(())).unwrap(); // echo -> flanger
        state.handle("fx.next", 1.0, |_| Ok(())).unwrap(); // flanger -> reverb
        let s = state.snapshot().unwrap();
        assert!(s.decks[0].reverb.active);
        assert_eq!(s.decks[0].reverb.mix, 0.77);
        let mut n = 0;
        state
            .update(0, EffectUpdate::BeatSec { seconds: 0.4 }, |_| {
                n += 1;
                Ok(())
            })
            .unwrap();
        assert_eq!(n, 0);
    }
    #[test]
    fn key_shift_uses_the_manual_scale_and_shift_moves_it() {
        let state = NativeEffects::default();
        state.handle("deck.1.key_shift.0", 1.0, |_| Ok(())).unwrap();
        let selected = state.snapshot().unwrap();
        assert_eq!(selected.decks[1].pitch_shift, 4.0);
        assert_eq!(selected.decks[1].key_pad, Some(0));
        state.handle("deck.1.key_shift.4", 1.0, |_| Ok(())).unwrap();
        assert_eq!(state.snapshot().unwrap().decks[1].pitch_shift, 0.0);
        state
            .handle("deck.1.key_shift_step.0", 1.0, |_| Ok(()))
            .unwrap();
        assert_eq!(state.snapshot().unwrap().decks[1].pitch_shift, 0.0);
        state
            .handle("deck.1.key_shift_step.6", 1.0, |_| Ok(()))
            .unwrap();
        let raised = state.snapshot().unwrap();
        assert_eq!(raised.decks[1].key_transpose, 1);
        assert_eq!(raised.decks[1].pitch_shift, 1.0);
        assert_eq!(raised.decks[1].key_pad, Some(4));
        state.handle("deck.1.key_shift.0", 1.0, |_| Ok(())).unwrap();
        assert_eq!(state.snapshot().unwrap().decks[1].pitch_shift, 5.0);
        for _ in 0..20 {
            state
                .handle("deck.1.key_shift_step.6", 1.0, |_| Ok(()))
                .unwrap();
        }
        let capped = state.snapshot().unwrap();
        assert_eq!(capped.decks[1].key_transpose, 12);
        assert_eq!(capped.decks[1].pitch_shift, 12.0);
        state
            .handle("deck.1.key_shift_step.7", 1.0, |_| Ok(()))
            .unwrap();
        assert_eq!(state.snapshot().unwrap().decks[1].pitch_shift, 11.0);
        assert_eq!(state.snapshot().unwrap().decks[0].pitch_shift, 0.0);
        state.handle("deck.1.key_shift.0", 0.0, |_| Ok(())).unwrap();
        assert_eq!(state.snapshot().unwrap().decks[1].pitch_shift, 11.0);
    }
    #[test]
    fn beat_step_bounds_and_clock_updates_emit_only_active_synced_fx() {
        let state = NativeEffects::default();
        for _ in 0..9 {
            state.handle("fx.beat_down", 1.0, |_| Ok(())).unwrap();
        }
        assert_eq!(state.snapshot().unwrap().decks[0].echo.beats, 0.25);
        state.handle("fx.toggle.0", 1.0, |_| Ok(())).unwrap();
        let mut sent = Vec::new();
        state
            .update(0, EffectUpdate::BeatSec { seconds: 0.4 }, |m| {
                sent.push(m);
                Ok(())
            })
            .unwrap();
        assert_eq!(sent.len(), 1);
        assert!(matches!(sent[0],EngineMsg::DeckEcho{time_sec,..}if(time_sec-0.1).abs()<1e-6));
    }
    #[test]
    fn invalid_and_failed_enqueue_do_not_commit_state() {
        let state = NativeEffects::default();
        assert!(state
            .update(
                NUM_DECKS,
                EffectUpdate::Pitch { semitones: 1.0 },
                |_| Ok(())
            )
            .is_err());
        assert!(state.handle("fx.level", f64::NAN, |_| Ok(())).is_err());
        assert!(state
            .update(
                0,
                EffectUpdate::Pitch {
                    semitones: f64::INFINITY
                },
                |_| Ok(())
            )
            .is_err());
        assert!(state
            .handle("fx.toggle.0", 1.0, |_| Err("offline".into()))
            .is_err());
        assert_eq!(state.snapshot().unwrap().revision, 0);
        assert!(!state.snapshot().unwrap().decks[0].echo.active);
        assert!(state
            .handle("deck.0.play", 1.0, |_| panic!("unowned"))
            .unwrap()
            .is_none());
    }

    #[test]
    fn pad_fx_hold_follows_the_pad_deck_and_releases_without_clearing_a_latch() {
        let state = NativeEffects::default();
        let mut sent = Vec::new();
        state
            .handle("deck.0.pad_fx.0", 1.0, |message| {
                sent.push(message);
                Ok(())
            })
            .unwrap();
        assert!(matches!(
            sent[0],
            EngineMsg::DeckEcho {
                deck: 0,
                active: true,
                time_sec,
                feedback,
                mix
            } if (time_sec - 0.25).abs() < 1e-6
                && (feedback - 0.5).abs() < 1e-6
                && (mix - 0.25).abs() < 1e-6
        ));
        state.handle("deck.1.pad_fx.2", 1.0, |_| Ok(())).unwrap();
        let held = state.snapshot().unwrap();
        assert!(held.decks[0].echo.active);
        assert!(!held.decks[1].echo.active);
        assert!(held.decks[1].reverb.active);
        assert!(!held.decks[0].reverb.active);
        state.handle("deck.0.pad_fx.0", 0.0, |_| Ok(())).unwrap();
        assert!(!state.snapshot().unwrap().decks[0].echo.active);
        assert!(state.snapshot().unwrap().decks[1].reverb.active);

        state.handle("fx.toggle.0", 1.0, |_| Ok(())).unwrap();
        state.handle("deck.0.pad_fx.0", 1.0, |_| Ok(())).unwrap();
        state.handle("deck.0.pad_fx.0", 0.0, |_| Ok(())).unwrap();
        assert!(state.snapshot().unwrap().decks[0].echo.active);

        state.handle("deck.0.pad_fx.1", 1.0, |_| Ok(())).unwrap();
        state.handle("deck.0.pad_fx.3", 1.0, |_| Ok(())).unwrap();
        let released = state.release_pad_fx(0, |_| Ok(())).unwrap().unwrap();
        assert!(released.decks[0].echo.active);
        assert!(!released.decks[0].flanger.active);
        assert!(!released.decks[0].crusher.active);
        assert!(released.decks[1].reverb.active);
        state.release_pad_fx(1, |_| Ok(())).unwrap();
        assert!(!state.snapshot().unwrap().decks[1].reverb.active);
    }

    #[test]
    fn pad_fx_presets_replace_parameters_and_release_restores_a_latch() {
        let state = NativeEffects::default();
        state
            .update(
                0,
                EffectUpdate::Echo {
                    active: true,
                    beats: 0.5,
                    depth: 0.5,
                },
                |_| Ok(()),
            )
            .unwrap();
        let mut sent = Vec::new();
        state
            .handle("deck.0.pad_fx.4", 1.0, |message| {
                sent.push(message);
                Ok(())
            })
            .unwrap();
        assert!(matches!(
            sent[0],
            EngineMsg::DeckEcho {
                active: true,
                time_sec,
                feedback,
                mix,
                ..
            } if (time_sec - 0.5).abs() < 1e-6
                && (feedback - 0.675).abs() < 1e-6
                && (mix - 0.375).abs() < 1e-6
        ));
        state.handle("deck.0.pad_fx.4", 1.0, |_| Ok(())).unwrap();
        state.handle("deck.0.pad_fx.12", 1.0, |_| Ok(())).unwrap();
        assert!((state.snapshot().unwrap().decks[0].echo.beats - 2.0).abs() < 1e-9);
        state.handle("deck.0.pad_fx.4", 0.0, |_| Ok(())).unwrap();
        assert!((state.snapshot().unwrap().decks[0].echo.beats - 2.0).abs() < 1e-9);
        state.handle("deck.0.pad_fx.12", 0.0, |_| Ok(())).unwrap();
        let restored = state.snapshot().unwrap();
        assert!(restored.decks[0].echo.active);
        assert!((restored.decks[0].echo.beats - 0.5).abs() < 1e-9);
        assert!((restored.decks[0].echo.depth - 0.5).abs() < 1e-9);
        state.handle("deck.0.pad_fx.0", 1.0, |_| Ok(())).unwrap();
        state.handle("deck.0.pad_fx.0", 0.0, |_| Ok(())).unwrap();
        assert!(state.snapshot().unwrap().decks[0].echo.active);
    }

    #[test]
    fn every_pad_fx_slot_engages_its_documented_preset_and_releases() {
        assert!(pad_fx_sound(32).is_none());
        for slot in 0..32 {
            let sound = pad_fx_sound(slot).expect("documented slot");
            let state = NativeEffects::default();
            let control = format!("deck.1.pad_fx.{slot}");
            state.handle(&control, 1.0, |_| Ok(())).unwrap();
            let held = state.snapshot().unwrap();
            assert!(held.decks[1].active(sound.effect()));
            assert!(!held.decks[0].echo.active);
            match sound {
                PadFxSound::Panel(_) => {}
                PadFxSound::Echo { beats, depth } => {
                    assert_eq!(held.decks[1].echo.beats, beats);
                    assert_eq!(held.decks[1].echo.depth, depth);
                }
                PadFxSound::Flanger { beats, depth } => {
                    assert_eq!(held.decks[1].flanger.beats, beats);
                    assert_eq!(held.decks[1].flanger.depth, depth);
                }
                PadFxSound::Reverb { size, mix } => {
                    assert_eq!(held.decks[1].reverb.size, size);
                    assert_eq!(held.decks[1].reverb.mix, mix);
                }
                PadFxSound::Crusher { crush, down } => {
                    assert_eq!(held.decks[1].crusher.crush, crush);
                    assert_eq!(held.decks[1].crusher.down, down);
                }
            }
            state.handle(&control, 0.0, |_| Ok(())).unwrap();
            assert!(!state.snapshot().unwrap().decks[1].active(sound.effect()));
        }
    }

    #[test]
    fn pad_fx_hold_is_momentary_and_a_shift_layer_press_does_not_stick() {
        let state = NativeEffects::default();
        state.handle("deck.0.pad_fx.0", 1.0, |_| Ok(())).unwrap();
        assert!(state.snapshot().unwrap().decks[0].echo.active);
        state
            .handle("deck.0.pad_fx.0", 63.0 / 127.0, |_| Ok(()))
            .unwrap();
        assert!(!state.snapshot().unwrap().decks[0].echo.active);
        state
            .handle("deck.0.pad_fx.0", 64.0 / 127.0, |_| Ok(()))
            .unwrap();
        state.handle("deck.0.pad_fx.0", 0.0, |_| Ok(())).unwrap();
        assert!(!state.snapshot().unwrap().decks[0].echo.active);
        state.handle("fx.toggle.0", 1.0, |_| Ok(())).unwrap();
        state.handle("deck.0.pad_fx.0", 1.0, |_| Ok(())).unwrap();
        state.handle("deck.0.pad_fx.0", 0.0, |_| Ok(())).unwrap();
        assert!(state.snapshot().unwrap().decks[0].echo.active);
        let latched = state.snapshot().unwrap().decks[0].echo;
        state.handle("deck.0.pad_fx.4", 1.0, |_| Ok(())).unwrap();
        state.handle("deck.0.pad_fx.8", 1.0, |_| Ok(())).unwrap();
        assert!((state.snapshot().unwrap().decks[0].echo.beats - 0.25).abs() < 1e-9);
        state.handle("deck.0.pad_fx.4", 0.0, |_| Ok(())).unwrap();
        assert!((state.snapshot().unwrap().decks[0].echo.beats - 0.25).abs() < 1e-9);
        state.handle("deck.0.pad_fx.8", 0.0, |_| Ok(())).unwrap();
        let restored = state.snapshot().unwrap().decks[0].echo;
        assert!(restored.active);
        assert_eq!(restored.beats, latched.beats);
        assert_eq!(restored.depth, latched.depth);
    }

    #[test]
    fn pad_fx_hold_does_not_follow_beat_fx_select_and_a_failed_send_does_not_stick() {
        let state = NativeEffects::default();
        state.handle("deck.0.pad_fx.0", 1.0, |_| Ok(())).unwrap();
        let selected = state.handle("fx.next", 1.0, |_| Ok(())).unwrap().unwrap();
        assert_eq!(selected.effect, Effect::Flanger);
        assert!(selected.decks[0].echo.active);
        assert!(!selected.decks[0].flanger.active);
        assert!(state.handle("deck.0.pad_fx.32", 1.0, |_| Ok(())).is_err());
        assert!(state.handle("deck.4.pad_fx.0", 1.0, |_| Ok(())).is_err());
        let before = state.snapshot().unwrap().revision;
        assert!(state
            .handle("deck.1.pad_fx.0", 1.0, |_| Err("offline".into()))
            .is_err());
        assert_eq!(state.snapshot().unwrap().revision, before);
        assert!(!state.snapshot().unwrap().decks[1].echo.active);
        assert!(state
            .release_pad_fx(1, |_| panic!("nothing held"))
            .unwrap()
            .is_none());
    }
}

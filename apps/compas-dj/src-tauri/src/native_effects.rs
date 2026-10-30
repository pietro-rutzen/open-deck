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
        }
    }
}
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
}
impl Default for EffectsSnapshot {
    fn default() -> Self {
        Self {
            revision: 0,
            effect: Effect::Echo,
            deck: 0,
            decks: [DeckEffects::default(); NUM_DECKS],
        }
    }
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
        let d = &mut next.decks[deck];
        for effect in [
            Effect::Echo,
            Effect::Flanger,
            Effect::Reverb,
            Effect::Crusher,
        ] {
            d.set_active(effect, false);
            send(d.command(deck, effect))?;
        }
        d.pitch_shift = 0.0;
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
        for d in &mut state.decks {
            for effect in [
                Effect::Echo,
                Effect::Flanger,
                Effect::Reverb,
                Effect::Crusher,
            ] {
                d.set_active(effect, false);
            }
            d.pitch_shift = 0.0;
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
            send(d.command(deck, e))?;
        }
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
            let amount = if relative {
                [1., 2., 3., 4., -1., -2., -3., -4.][pad]
            } else {
                [0., 1., 2., 3., -3., -2., -1., 0.][pad]
            };
            d.pitch_shift = (if relative {
                d.pitch_shift + amount
            } else {
                amount
            })
            .clamp(-12.0, 12.0);
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
                let d = &mut next.decks[deck];
                d.set_active(next.effect, !d.active(next.effect));
                send(d.command(deck, next.effect))?;
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
                    for (deck, d) in next.decks.iter_mut().enumerate() {
                        if d.active(old) {
                            d.set_active(old, false);
                            send(d.command(deck, old))?;
                            if !d.active(next.effect) {
                                d.set_active(next.effect, true);
                                send(d.command(deck, next.effect))?;
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
                    for (deck, d) in next.decks.iter_mut().enumerate() {
                        for e in [
                            Effect::Echo,
                            Effect::Flanger,
                            Effect::Reverb,
                            Effect::Crusher,
                        ] {
                            // Macro/legacy controls may have enabled slots outside this state.
                            // Panic-off must bypass every slot, even when already marked off.
                            d.set_active(e, false);
                            send(d.command(deck, e))?;
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
    fn key_shift_steps_accumulate_without_ui_and_clamp() {
        let state = NativeEffects::default();
        state.handle("deck.1.key_shift.4", 1.0, |_| Ok(())).unwrap();
        assert_eq!(state.snapshot().unwrap().decks[1].pitch_shift, -3.0);
        for _ in 0..8 {
            state
                .handle("deck.1.key_shift_step.3", 1.0, |_| Ok(()))
                .unwrap();
        }
        assert_eq!(state.snapshot().unwrap().decks[1].pitch_shift, 12.0);
        assert_eq!(state.snapshot().unwrap().decks[0].pitch_shift, 0.0);
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
}

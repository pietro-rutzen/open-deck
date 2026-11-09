//! DDJ-FLX4 relative jog translation for a native controller worker (not the audio callback).
//! Addresses/relative encoding: Pioneer DDJ-FLX4 MIDI message list E1.
//! Velocity sensitivity is prototype tuning, not a measured hardware calibration.
use std::time::{Duration, Instant};

const MOTION_TIMEOUT: Duration = Duration::from_millis(100);
const MIN_DT: f64 = 0.001;
const MAX_DT: f64 = 0.05;
// Prototype: one reported count per 10 ms corresponds to one natural playback rate.
const SECONDS_PER_COUNT: f64 = 0.01;
const BEND_PER_COUNT: f64 = 0.04;
const MAX_BEND: f64 = 0.08;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct JogAction {
    pub deck: usize,
    pub active: bool,
    pub speed: f64,
    pub bend: bool,
    /// Non-zero while a loop-edge adjust is held: raw platter counts, not a playback rate.
    pub nudge: i16,
    /// SHIFT+platter (CC 41). One count is one bar; zero for every other gesture.
    pub bars: i16,
}
#[derive(Clone, Copy, Default)]
enum Mode {
    #[default]
    Idle,
    Hold,
    Bend,
}
#[derive(Clone, Copy)]
struct DeckJog {
    touched: bool,
    vinyl: bool,
    mode: Mode,
    last_motion: Option<Instant>,
    moving: bool,
    /// 0 = normal jog, 1 = nudge loop in, 2 = nudge loop out.
    adjust: u8,
}
impl Default for DeckJog {
    fn default() -> Self {
        Self {
            touched: false,
            vinyl: true,
            mode: Mode::Idle,
            last_motion: None,
            moving: false,
            adjust: 0,
        }
    }
}
#[derive(Default)]
pub(crate) struct JogState {
    decks: [DeckJog; 2],
}
impl JogState {
    /// Switch this deck between platter playback and loop-edge trim.
    /// Releasing an active scratch/bend when trim starts.
    pub(crate) fn set_adjust(&mut self, deck: usize, edge: u8) -> Option<JogAction> {
        let s = self.decks.get_mut(deck)?;
        let edge = if edge <= 2 { edge } else { 0 };
        if s.adjust == edge {
            return None;
        }
        s.adjust = edge;
        if edge != 0 && !matches!(s.mode, Mode::Idle) {
            s.mode = Mode::Idle;
            s.moving = false;
            s.last_motion = None;
            return Some(JogAction {
                deck,
                bend: false,
                active: false,
                speed: 0.0,
                nudge: 0,
                bars: 0,
            });
        }
        None
    }

    /// Consume only jog CCs and touch notes on zero-based deck channels 0/1.
    /// `playing` and `tempo` must refer to the MIDI message's deck, from engine truth.
    /// No heap allocation, IPC, locks, or playback-state toggles happen here.
    pub(crate) fn handle(
        &mut self,
        status: u8,
        d1: u8,
        d2: u8,
        now: Instant,
        playing: bool,
        _tempo: f64,
    ) -> Option<JogAction> {
        let deck = usize::from(status & 0x0f);
        if deck >= self.decks.len() || d1 > 127 || d2 > 127 {
            return None;
        }
        let s = &mut self.decks[deck];
        let kind = status & 0xf0;
        if matches!(kind, 0x80 | 0x90) && d1 == 103 {
            return None;
        }
        if matches!(kind, 0x80 | 0x90) && d1 == 54 {
            let pressed = kind == 0x90 && d2 != 0;
            s.touched = pressed;
            s.last_motion = None;
            s.moving = false;
            if s.adjust != 0 {
                if !pressed && !matches!(s.mode, Mode::Idle) {
                    s.mode = Mode::Idle;
                    return Some(JogAction {
                        deck,
                        bend: false,
                        active: false,
                        speed: 0.0,
                        nudge: 0,
                        bars: 0,
                    });
                }
                return None;
            }
            if pressed && s.vinyl {
                s.mode = Mode::Hold;
                return Some(JogAction {
                    deck,
                    bend: false,
                    active: true,
                    speed: 0.0,
                    nudge: 0,
                    bars: 0,
                });
            }
            if !pressed && !matches!(s.mode, Mode::Idle) {
                s.mode = Mode::Idle;
                return Some(JogAction {
                    deck,
                    bend: false,
                    active: false,
                    speed: 0.0,
                    nudge: 0,
                    bars: 0,
                });
            }
            return None;
        }
        if kind != 0xb0 || !matches!(d1, 33 | 34 | 35 | 41) {
            return None;
        }
        let delta = i16::from(d2) - 64;
        if delta == 0 {
            return None;
        }
        if d1 == 41 {
            s.mode = Mode::Idle;
            s.moving = false;
            s.last_motion = None;
            return Some(JogAction {
                deck,
                bend: false,
                active: false,
                speed: 0.0,
                nudge: 0,
                bars: delta,
            });
        }
        if d1 == 34 {
            s.vinyl = true;
        }
        if d1 == 35 {
            s.vinyl = false;
        }
        let dt = s
            .last_motion
            .and_then(|last| now.checked_duration_since(last))
            .map_or(0.01, |elapsed| elapsed.as_secs_f64().clamp(MIN_DT, MAX_DT));
        s.last_motion = Some(now);
        if s.adjust != 0 {
            s.mode = Mode::Idle;
            s.moving = false;
            return Some(JogAction {
                deck,
                bend: false,
                active: false,
                speed: 0.0,
                nudge: delta,
                bars: 0,
            });
        }
        if s.touched && s.vinyl && d1 == 34 {
            s.mode = Mode::Hold;
            s.moving = true;
            return Some(JogAction {
                deck,
                bend: false,
                active: true,
                speed: (f64::from(delta) * SECONDS_PER_COUNT / dt).clamp(-16.0, 16.0),
                nudge: 0,
                bars: 0,
            });
        }
        // Side/non-vinyl movement bends a running deck only. Never make a paused deck audible.
        if playing {
            let bend = (f64::from(delta) * BEND_PER_COUNT).clamp(-MAX_BEND, MAX_BEND);
            s.mode = Mode::Bend;
            s.moving = true;
            return Some(JogAction {
                deck,
                bend: true,
                active: true,
                speed: 1.0 + bend,
                nudge: 0,
                bars: 0,
            });
        }
        // A paused non-vinyl/side event must also clear any previous scratch override.
        if !matches!(s.mode, Mode::Idle) {
            s.mode = Mode::Idle;
            s.moving = false;
            return Some(JogAction {
                deck,
                bend: false,
                active: false,
                speed: 0.0,
                nudge: 0,
                bars: 0,
            });
        }
        None
    }

    /// Call for each deck on the native worker's bounded periodic tick.
    /// Stops the last scratch velocity while a hand remains on the platter; releases a bend.
    pub(crate) fn tick(&mut self, deck: usize, now: Instant) -> Option<JogAction> {
        let s = self.decks.get_mut(deck)?;
        if !s.moving
            || !s
                .last_motion
                .is_some_and(|last| now.saturating_duration_since(last) >= MOTION_TIMEOUT)
        {
            return None;
        }
        s.moving = false;
        if s.touched && matches!(s.mode, Mode::Hold) {
            Some(JogAction {
                deck,
                bend: false,
                active: true,
                speed: 0.0,
                nudge: 0,
                bars: 0,
            })
        } else {
            s.mode = Mode::Idle;
            Some(JogAction {
                deck,
                bend: false,
                active: false,
                speed: 0.0,
                nudge: 0,
                bars: 0,
            })
        }
    }

    /// Invoke for both decks on disconnect, deactivation, profile change, or track replacement.
    pub(crate) fn reset(&mut self, deck: usize) -> Option<JogAction> {
        let s = self.decks.get_mut(deck)?;
        let was_active = !matches!(s.mode, Mode::Idle);
        *s = DeckJog::default();
        was_active.then_some(JogAction {
            deck,
            bend: false,
            active: false,
            speed: 0.0,
            nudge: 0,
            bars: 0,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn event(
        s: &mut JogState,
        status: u8,
        control: u8,
        value: u8,
        now: Instant,
        playing: bool,
    ) -> Option<JogAction> {
        s.handle(status, control, value, now, playing, 1.0)
    }
    #[test]
    fn touch_holds_paused_deck_and_release_cleans_up() {
        let now = Instant::now();
        let mut s = JogState::default();
        assert_eq!(event(&mut s, 0x90, 54, 127, now, false).unwrap().speed, 0.0);
        let a = event(&mut s, 0xb0, 34, 63, now, false).unwrap();
        assert!(a.active && a.speed < 0.0);
        assert_eq!(
            s.tick(0, now + MOTION_TIMEOUT),
            Some(JogAction {
                deck: 0,
                bend: false,
                active: true,
                speed: 0.0,
                nudge: 0,
                bars: 0,
            })
        );
        assert!(s.tick(0, now + Duration::from_secs(2)).is_none());
        assert!(!event(&mut s, 0x90, 54, 0, now, false).unwrap().active);
    }
    #[test]
    fn bend_running_only_and_returns_to_transport() {
        let now = Instant::now();
        let mut s = JogState::default();
        assert!(event(&mut s, 0xb0, 33, 65, now, false).is_none());
        assert!(event(&mut s, 0xb0, 35, 65, now, true).unwrap().speed > 1.0);
        assert!(!s.tick(0, now + MOTION_TIMEOUT).unwrap().active);
        assert!(s.tick(0, now + MOTION_TIMEOUT).is_none());
    }
    #[test]
    fn deck_isolation_shift_touch_noteoff_reset_and_limits() {
        let now = Instant::now();
        let mut s = JogState::default();
        assert!(event(&mut s, 0x92, 54, 127, now, true).is_none());
        assert!(event(&mut s, 0x91, 103, 127, now, false).is_none());
        assert!(event(&mut s, 0x81, 103, 0, now, false).is_none());
        event(&mut s, 0x90, 54, 127, now, true);
        assert_eq!(event(&mut s, 0xb0, 34, 127, now, true).unwrap().speed, 16.0);
        let released = event(&mut s, 0xb0, 41, 65, now, true).unwrap();
        assert_eq!(released.bars, 1);
        assert!(!released.active);
        assert!(s.reset(0).is_none());
        event(&mut s, 0x90, 54, 127, now, true);
        assert!(!s.reset(0).unwrap().active);
    }

    #[test]
    fn shift_platter_counts_bars_without_scratch_or_bend() {
        let now = Instant::now();
        let mut paused = JogState::default();
        let back = event(&mut paused, 0xb0, 41, 63, now, false).unwrap();
        assert_eq!(
            (back.deck, back.bars, back.active, back.nudge),
            (0, -1, false, 0)
        );
        assert!(event(&mut paused, 0xb0, 33, 65, now, false).is_none());
        let mut playing = JogState::default();
        event(&mut playing, 0x90, 54, 127, now, true);
        let forward = event(&mut playing, 0xb1, 41, 66, now, true).unwrap();
        assert_eq!(
            (forward.deck, forward.bars, forward.active, forward.bend),
            (1, 2, false, false)
        );
        assert!(event(&mut playing, 0xb0, 33, 65, now, true).unwrap().bend);
        playing.set_adjust(0, 1);
        let during_trim = event(&mut playing, 0xb0, 41, 65, now, true).unwrap();
        assert_eq!(during_trim.bars, 1);
        assert_eq!(during_trim.nudge, 0);
    }
    #[test]
    fn ignores_unrelated_center_and_invalid_and_sanitizes_tempo() {
        let now = Instant::now();
        let mut s = JogState::default();
        for (status, control, value) in [
            (0xb0, 7, 65),
            (0xb0, 34, 64),
            (0xb0, 34, 255),
            (0x90, 11, 127),
        ] {
            assert!(event(&mut s, status, control, value, now, true).is_none());
        }
        assert!(s
            .handle(0xb0, 33, 65, now, true, f64::NAN)
            .unwrap()
            .speed
            .is_finite());
    }

    #[test]
    fn loop_adjust_reports_platter_counts_instead_of_scratch() {
        let now = Instant::now();
        let mut s = JogState::default();
        event(&mut s, 0x90, 54, 127, now, true);
        assert!(event(&mut s, 0xb0, 34, 66, now, true).unwrap().active);
        assert!(!s.set_adjust(0, 1).unwrap().active);
        let nudge = event(&mut s, 0xb0, 34, 67, now, true).unwrap();
        assert_eq!(nudge.nudge, 3);
        assert!(!nudge.active);
        assert!(s.set_adjust(0, 1).is_none());
        assert_eq!(event(&mut s, 0xb0, 33, 60, now, true).unwrap().nudge, -4);
    }
}

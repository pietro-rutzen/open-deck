//! Bounded FLX4 jog input; latest desired gesture is retained until the audio command ring accepts it.
use crate::flx4_jog::{JogAction, JogState};
use compas_audio::DeckTelemetry;
use std::sync::{
    atomic::{AtomicBool, AtomicI32, AtomicU64, AtomicU8, Ordering},
    mpsc::{self, SyncSender},
    Arc,
};
use std::time::{Duration, Instant};
pub(crate) const INACTIVE: u64 = u64::MAX;
pub(crate) fn command(deck: usize, bits: u64) -> compas_audio::AudioCommand {
    if bits == INACTIVE {
        compas_audio::AudioCommand::SetJogBend {
            deck,
            multiplier: 1.0,
            active: false,
        }
    } else if bits >> 32 == 1 {
        compas_audio::AudioCommand::SetJogBend {
            deck,
            multiplier: f32::from_bits(bits as u32) as f64,
            active: true,
        }
    } else {
        compas_audio::AudioCommand::SetScratch {
            deck,
            active: true,
            speed: f32::from_bits(bits as u32) as f64,
        }
    }
}
pub(crate) type JogOutput = Arc<[AtomicU64; 2]>;
struct Event {
    epoch: u64,
    at: Instant,
    bytes: [u8; 3],
}
#[derive(Clone)]
pub(crate) struct JogRuntime {
    tx: SyncSender<Event>,
    epoch: Arc<AtomicU64>,
    profile: Arc<AtomicBool>,
    connected: Arc<AtomicBool>,
    adjust: Arc<[AtomicU8; 2]>,
    nudge_in: Arc<[AtomicI32; 2]>,
    nudge_out: Arc<[AtomicI32; 2]>,
}
impl JogRuntime {
    pub fn spawn(output: JogOutput, telemetry: Arc<DeckTelemetry>) -> Self {
        let (tx, rx) = mpsc::sync_channel::<Event>(128);
        let this = Self {
            tx,
            epoch: Arc::new(AtomicU64::new(0)),
            profile: Arc::new(AtomicBool::new(false)),
            connected: Arc::new(AtomicBool::new(false)),
            adjust: Arc::new([AtomicU8::new(0), AtomicU8::new(0)]),
            nudge_in: Arc::new([AtomicI32::new(0), AtomicI32::new(0)]),
            nudge_out: Arc::new([AtomicI32::new(0), AtomicI32::new(0)]),
        };
        let state = this.clone();
        let adjust = this.adjust.clone();
        let nudge_in = this.nudge_in.clone();
        let nudge_out = this.nudge_out.clone();
        std::thread::spawn(move || {
            let mut jog = JogState::default();
            let mut epoch = 0;
            let mut probe = Instant::now();
            let apply = |a: JogAction| {
                output[a.deck].store(
                    if a.active {
                        ((if a.bend { 1_u64 } else { 2_u64 }) << 32)
                            | (a.speed as f32).to_bits() as u64
                    } else {
                        INACTIVE
                    },
                    Ordering::Release,
                );
            };
            loop {
                let event = match rx.recv_timeout(Duration::from_millis(5)) {
                    Ok(e) => Some(e),
                    Err(mpsc::RecvTimeoutError::Timeout) => None,
                    Err(_) => break,
                };
                let now = Instant::now();
                // Enumeration happens only on this control worker, never the MIDI/audio callback.
                if state.connected.load(Ordering::Acquire)
                    && now.duration_since(probe) >= Duration::from_millis(500)
                {
                    probe = now;
                    if crate::midi_list_ports()
                        .is_ok_and(|ports| !ports.iter().any(|p| p.contains("DDJ-FLX4")))
                    {
                        state.set_connected(false);
                    }
                }
                let current = state.epoch.load(Ordering::Acquire);
                if current != epoch {
                    for d in 0..2 {
                        jog.reset(d);
                        output[d].store(INACTIVE, Ordering::Release);
                    }
                    epoch = current;
                }
                if let Some(e) = event {
                    if e.epoch == epoch && state.enabled() {
                        if now.duration_since(e.at) > Duration::from_millis(100) {
                            state.reset();
                            continue;
                        }
                        let [status, d1, d2] = e.bytes;
                        let d = usize::from(status & 15);
                        if telemetry.is_loaded(d) {
                            let edge = adjust[d].load(Ordering::Acquire);
                            if let Some(release) = jog.set_adjust(d, edge) {
                                apply(release);
                            }
                            if let Some(a) = jog.handle(
                                status,
                                d1,
                                d2,
                                e.at,
                                telemetry.is_playing(d),
                                telemetry.tempo_ratio(d),
                            ) {
                                if a.nudge != 0 {
                                    let counts = if edge == 1 { &nudge_in } else { &nudge_out };
                                    if matches!(edge, 1 | 2) {
                                        counts[d].fetch_add(i32::from(a.nudge), Ordering::AcqRel);
                                    }
                                } else {
                                    apply(a);
                                }
                            }
                        }
                    }
                }
                for d in 0..2 {
                    let edge = adjust[d].load(Ordering::Acquire);
                    if let Some(release) = jog.set_adjust(d, edge) {
                        apply(release);
                    }
                }
                for d in 0..2 {
                    if !telemetry.is_loaded(d) {
                        if let Some(a) = jog.reset(d) {
                            apply(a);
                        }
                    } else if let Some(a) = jog.tick(d, now) {
                        apply(a);
                    }
                }
            }
        });
        this
    }
    fn enabled(&self) -> bool {
        self.profile.load(Ordering::Acquire) && self.connected.load(Ordering::Acquire)
    }
    pub fn reset(&self) {
        self.epoch.fetch_add(1, Ordering::AcqRel);
    }
    pub fn set_profile(&self, enabled: bool) {
        self.profile.store(enabled, Ordering::Release);
        self.reset();
    }
    pub fn set_connected(&self, enabled: bool) {
        self.connected.store(enabled, Ordering::Release);
        self.reset();
    }
    /// 0 = platter, 1 = loop in, 2 = loop out. Read by the jog worker.
    pub fn set_loop_adjust(&self, deck: usize, edge: u8) {
        if let Some(slot) = self.adjust.get(deck) {
            slot.store(edge, Ordering::Release);
        }
    }
    /// Accumulated platter counts since the last take, as `(loop in, loop out)` per deck.
    pub fn take_loop_nudges(&self) -> [(i32, i32); 2] {
        std::array::from_fn(|deck| {
            (
                self.nudge_in[deck].swap(0, Ordering::AcqRel),
                self.nudge_out[deck].swap(0, Ordering::AcqRel),
            )
        })
    }
    /// Nonblocking input; overflow invalidates the gesture and all queued older events.
    pub fn midi(&self, status: u8, d1: u8, d2: u8) -> bool {
        if !self.enabled() || status & 15 > 1 {
            return false;
        }
        let is_jog = match status & 0xf0 {
            0xb0 => matches!(d1, 33 | 34 | 35 | 41),
            0x80 | 0x90 => matches!(d1, 54 | 103),
            _ => false,
        };
        if !is_jog {
            return false;
        }
        let e = Event {
            epoch: self.epoch.load(Ordering::Acquire),
            at: Instant::now(),
            bytes: [status, d1, d2],
        };
        if self.tx.try_send(e).is_err() {
            self.reset();
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn input(capacity: usize) -> (JogRuntime, mpsc::Receiver<Event>) {
        let (tx, rx) = mpsc::sync_channel(capacity);
        (
            JogRuntime {
                tx,
                epoch: Arc::new(AtomicU64::new(0)),
                profile: Arc::new(AtomicBool::new(false)),
                connected: Arc::new(AtomicBool::new(false)),
                adjust: Arc::new([AtomicU8::new(0), AtomicU8::new(0)]),
                nudge_in: Arc::new([AtomicI32::new(0), AtomicI32::new(0)]),
                nudge_out: Arc::new([AtomicI32::new(0), AtomicI32::new(0)]),
            },
            rx,
        )
    }

    #[test]
    fn packed_gesture_preserves_bend_vs_scratch_and_release() {
        use compas_audio::AudioCommand;
        let bend = (1_u64 << 32) | 1.04_f32.to_bits() as u64;
        assert!(
            matches!(command(1, bend), AudioCommand::SetJogBend { deck: 1, active: true, multiplier } if (multiplier - 1.04).abs() < 0.0001)
        );
        let scratch = (2_u64 << 32) | (-2.0_f32).to_bits() as u64;
        assert!(
            matches!(command(0, scratch), AudioCommand::SetScratch { deck: 0, active: true, speed } if speed == -2.0)
        );
        assert!(matches!(
            command(0, INACTIVE),
            AudioCommand::SetJogBend { active: false, .. }
        ));
    }
    #[test]
    fn only_active_flx4_can_capture_jog_and_overflow_invalidates_pending_input() {
        let (j, rx) = input(1);
        assert!(!j.midi(0xb0, 34, 65));
        j.set_profile(true);
        assert!(!j.midi(0xb0, 34, 65));
        j.set_connected(true);
        assert!(!j.midi(0xb0, 7, 65));
        assert!(!j.midi(0xb2, 34, 65));
        assert!(j.midi(0xb0, 34, 65));
        assert!(j.midi(0x90, 54, 0)); // full queue: invalidate older motion rather than lose release
        assert_ne!(rx.recv().unwrap().epoch, j.epoch.load(Ordering::Acquire));
        j.set_connected(false);
        assert!(!j.midi(0xb0, 34, 65));
    }
}

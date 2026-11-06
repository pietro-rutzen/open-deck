//! While a meter is armed, the audio thread writes a preallocated stereo ring.
//! Readers copy the latest window from another thread. The callback only stores atomics.
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering::*};
use std::time::Instant;

pub const FRAMES: usize = 8192;
const RING: usize = 16384;
const RING_MASK: usize = RING - 1;

pub struct ScopeBlock {
    pub sample_rate: f32,
    pub frames: Vec<[f32; 2]>,
}

fn monotonic_origin() -> Instant {
    static ORIGIN: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
    *ORIGIN.get_or_init(Instant::now)
}

pub(crate) fn monotonic_ns() -> u64 {
    u64::try_from(monotonic_origin().elapsed().as_nanos()).unwrap_or(u64::MAX)
}

pub(crate) struct ScopeSlot {
    armed_until_ns: AtomicU64,
    epoch: AtomicU64,
    written: AtomicU64,
    window_start: AtomicU64,
    rate: AtomicU32,
    samples: Box<[AtomicU64]>,
}

impl ScopeSlot {
    pub(crate) fn new() -> Self {
        let _ = monotonic_origin();
        Self {
            armed_until_ns: AtomicU64::new(0),
            epoch: AtomicU64::new(0),
            written: AtomicU64::new(0),
            window_start: AtomicU64::new(0),
            rate: AtomicU32::new(0),
            samples: (0..RING).map(|_| AtomicU64::new(0)).collect(),
        }
    }

    pub(crate) fn arm(&self, hold_ns: u64) {
        let until = monotonic_ns().saturating_add(hold_ns);
        self.armed_until_ns.store(until, Release);
    }

    pub(crate) fn release(&self) {
        self.armed_until_ns.store(0, Release);
    }

    pub(crate) fn is_armed(&self, now_ns: u64) -> bool {
        let until = self.armed_until_ns.load(Acquire);
        until != 0 && now_ns < until
    }

    /// Audio thread, once, when capture becomes active.
    pub(crate) fn begin_window(&self, rate: f32, cursor: u64) {
        self.rate.store(rate.to_bits(), Relaxed);
        self.window_start.store(cursor, Relaxed);
        self.epoch.fetch_add(1, Release);
    }

    /// Audio thread, once, when capture stops or the deck changes.
    pub(crate) fn end_window(&self) {
        let written = self.written.load(Relaxed);
        self.window_start.store(written, Relaxed);
        self.epoch.fetch_add(1, Release);
    }

    /// One pre-fader stereo sample. Wait-free: two atomic stores, no allocation.
    #[inline]
    pub(crate) fn write_sample(&self, index: u64, left: f32, right: f32) {
        let packed = u64::from(left.to_bits()) | (u64::from(right.to_bits()) << 32);
        self.samples[(index as usize) & RING_MASK].store(packed, Relaxed);
        self.written.store(index.wrapping_add(1), Release);
    }

    /// `written` counter when a full window is available. Cheap enough to poll every frame.
    pub(crate) fn sequence(&self) -> Option<u64> {
        let epoch = self.epoch.load(Acquire);
        let end = self.written.load(Acquire);
        let origin = self.window_start.load(Acquire);
        if end.wrapping_sub(origin) < FRAMES as u64 || self.epoch.load(Acquire) != epoch {
            return None;
        }
        Some(end)
    }

    /// Latest [`FRAMES`] samples, if that many belong to the current window.
    /// `sequence` increases with each new sample and does not repeat after a restart.
    pub(crate) fn copy_window(&self, dst: &mut Vec<[f32; 2]>) -> Option<(f32, u64)> {
        let epoch = self.epoch.load(Acquire);
        let end = self.written.load(Acquire);
        let origin = self.window_start.load(Acquire);
        if end.wrapping_sub(origin) < FRAMES as u64 {
            return None;
        }
        let rate = f32::from_bits(self.rate.load(Relaxed));
        if !rate.is_finite() || rate <= 40.0 {
            return None;
        }
        if dst.len() != FRAMES {
            dst.resize(FRAMES, [0.0, 0.0]);
        }
        let start = end - FRAMES as u64;
        for (offset, frame) in dst.iter_mut().enumerate() {
            let packed =
                self.samples[(start as usize).wrapping_add(offset) & RING_MASK].load(Relaxed);
            *frame = [
                f32::from_bits(packed as u32),
                f32::from_bits((packed >> 32) as u32),
            ];
        }
        let end_after = self.written.load(Acquire);
        if self.epoch.load(Acquire) != epoch
            || self.window_start.load(Acquire) != origin
            || end_after.wrapping_sub(end) > (RING - FRAMES) as u64
        {
            return None;
        }
        Some((rate, end))
    }
}

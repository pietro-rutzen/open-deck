//! One request captures one contiguous stereo block. All audio-side storage is preallocated.
use std::sync::atomic::{AtomicU32, AtomicU64, AtomicU8, Ordering::*};
pub const FRAMES: usize = 8192;
const IDLE: u8 = 0;
const REQUEST: u8 = 1;
const WRITING: u8 = 2;
const READY: u8 = 3;
const READING: u8 = 4;

pub struct ScopeBlock {
    pub sample_rate: f32,
    pub frames: Vec<[f32; 2]>,
}

pub(crate) struct ScopeSlot {
    state: AtomicU8,
    rate: AtomicU32,
    samples: Box<[AtomicU64]>,
}

impl ScopeSlot {
    pub(crate) fn new() -> Self {
        Self {
            state: AtomicU8::new(IDLE),
            rate: AtomicU32::new(0),
            samples: (0..FRAMES).map(|_| AtomicU64::new(0)).collect(),
        }
    }
    pub(crate) fn request(&self) {
        let _ = self.state.compare_exchange(IDLE, REQUEST, Release, Relaxed);
    }
    pub(crate) fn begin(&self) -> bool {
        self.state.load(Relaxed) == REQUEST
            && self
                .state
                .compare_exchange(REQUEST, WRITING, Acquire, Relaxed)
                .is_ok()
    }
    #[inline]
    pub(crate) fn write(&self, i: usize, l: f32, r: f32) {
        self.samples[i].store(
            u64::from(l.to_bits()) | (u64::from(r.to_bits()) << 32),
            Relaxed,
        );
    }
    pub(crate) fn publish(&self, rate: f32) {
        self.rate.store(rate.to_bits(), Relaxed);
        self.state.store(READY, Release);
    }
    pub(crate) fn cancel(&self, owns_write: bool) {
        if owns_write {
            self.state.store(IDLE, Release);
        } else {
            let _ = self.state.compare_exchange(READY, IDLE, AcqRel, Relaxed);
        }
    }
    // Called only by the control side after the complete block is published.
    pub(crate) fn take(&self) -> Option<ScopeBlock> {
        self.state
            .compare_exchange(READY, READING, Acquire, Relaxed)
            .ok()?;
        let frames = self
            .samples
            .iter()
            .map(|s| {
                let p = s.load(Relaxed);
                [f32::from_bits(p as u32), f32::from_bits((p >> 32) as u32)]
            })
            .collect();
        let sample_rate = f32::from_bits(self.rate.load(Relaxed));
        self.state.store(IDLE, Release);
        Some(ScopeBlock {
            frames,
            sample_rate,
        })
    }
}

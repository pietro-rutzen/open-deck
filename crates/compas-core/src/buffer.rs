/// A fully-decoded track held in memory as interleaved stereo `f32` at its source
/// sample rate. Shared with the audio thread via `Arc<DeckBuffer>` (immutable once
/// built), which is what makes seek / varispeed / scratch / loops cheap and instant.
///
/// Memory cost is ~`source_rate * 2ch * 4 bytes * seconds` (≈115 MB for a 5-min,
/// 48 kHz track) — acceptable for a handful of decks on a desktop, and the price we
/// pay for a random-access play-head instead of a streaming queue.
#[derive(Debug, Clone)]
pub struct DeckBuffer {
    /// Interleaved stereo: `[l0, r0, l1, r1, …]`.
    pub samples: Vec<f32>,
    /// The rate the samples were decoded at (NOT the output device rate).
    pub source_rate: u32,
    /// Silent regions `[start, end)` in source frames. Empty unless a load measured them.
    /// The audio thread only reads this; it never grows it.
    pub silence: std::sync::Arc<[(f64, f64)]>,
}

impl DeckBuffer {
    pub fn new(samples: Vec<f32>, source_rate: u32) -> Self {
        DeckBuffer {
            samples,
            source_rate,
            silence: std::sync::Arc::from([]),
        }
    }

    pub fn with_silence(mut self, spans: Vec<(f64, f64)>) -> Self {
        self.silence = std::sync::Arc::from(spans);
        self
    }

    /// Number of stereo frames.
    pub fn frames(&self) -> usize {
        self.samples.len() / 2
    }

    /// Total duration in milliseconds.
    pub fn duration_ms(&self) -> u64 {
        if self.source_rate == 0 {
            return 0;
        }
        self.frames() as u64 * 1000 / self.source_rate as u64
    }
}

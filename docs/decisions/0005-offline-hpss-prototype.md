# ADR-0005 — Offline harmonic/percussive prototype

- Status: experimental prototype. Not a product stem, not a playback feature, and not a quality claim about real music.
- Date: 2026-11-07
- Author / accountable role: the offline DSP prototype approved in this session.
- Decision authority: owner message at 11:29 Europe/Helsinki on 7 November 2026: start the offline classical-DSP stems prototype without ML weights or playback changes, and push it to private main.
- Related: ADR-0004. This note does not accept a model, a runtime, or a deck control.
- Supersedes / superseded by: none.

## Decision

The first classical milestone is one separation, not four source names.

`compas_dsp::hpss` splits a mono or stereo buffer into a harmonic stream and a percussive stream with a fixed median-filter mask. The original samples passed in are not modified. The audio callback does not call this module. `crates/compas-audio` has no reference to it. Playback remains the original mix.

These were considered and left out of this milestone:

| Idea | Why it is not in this prototype |
| --- | --- |
| Mid/side or spectral center | Center is not a vocal. Duplicating a channel so a tone is centered does not change that channel's result. |
| Bass crossover | A low tone is not bass. An 80 Hz sine is classified with the other sustained tones. |
| REPET or untrained NMF | ADR-0004 already records that factorization has no stable four-stem vocabulary. This milestone does not need a second algorithm to show that two sustained sources share one stream. |

No model weights, training, or inference runtime were added. No new crate dependency was added. The FFT is the existing `rustfft` already used for offline analysis.

## Algorithm

Fitzgerald, "Harmonic/Percussive Separation using Median Filtering", DAFx 2010, with the soft mask from Driedger, Müller, and Disch, "Extending Harmonic-Percussive Separation of Audio Signals", ISMIR 2014. The numeric defaults match librosa's `hpss` (`kernel_size=31`, `power=2`) and were not fitted to the fixtures.

| Parameter | Value |
| --- | --- |
| Window | Periodic Hann, length 2048 |
| Hop | 512 samples (the window advances by one quarter) |
| Harmonic median | 31 frames, along time, edge samples repeated |
| Percussive median | 31 bins, along frequency, edge bins repeated |
| Mask | Square the two medians, then `harmonic = harmonic² / (harmonic² + percussive²)`. The percussive mask is `1 - harmonic`, so the masks sum to one. A bin whose medians are both zero uses one half. |
| Channels | One or two, processed separately |
| Padding | One window of zeros on each side, then the same length as the input is returned |
| Inverse | `rustfft` inverse, divided by the FFT length, overlap-added, divided by the summed Hann envelope |
| Length cap | 1,440,000 samples per channel (30 s at 48 kHz) |
| Allowed shapes | FFT length a power of two from 16 to 8192. Hop must divide that length and sit between one quarter and one half of it. Median length an odd count from 1 to 127. |

`cancel` may be polled by the caller. If it returns true, the function returns `HpssError::Cancelled` and no streams.

Harmonic here means energy that stays put in frequency. Percussive means energy that is brief and wide in frequency. A centered instrument is not a vocal. A low note is not bass. A noise burst is not a drum, and a pitched decaying tone is not isolated as one.

## How to reproduce

From the repository root, on a machine with the locked toolchain:

```
CARGO_HOME="$PWD/.build-cache/cargo" cargo test --locked -p compas-dsp --lib hpss -- --nocapture
```

The fixtures are synthesized inside `crates/compas-dsp/src/hpss.rs` tests. There is no downloaded or recorded audio. Formulas are sines, a linear chirp, exponentially decaying sines, and a deterministic xorshift noise burst. Seeds are constants in those tests. The outputs are original synthetic signals.

## Metrics

Energy is the sum of squared samples, accumulated in `f64`.

Scale-invariant ratio of an estimate against a reference:

- `α = dot(reference, estimate) / ||reference||²`
- numerator = `α² ||reference||²` (energy of the scaled reference)
- raw error = `max(||estimate||² - numerator, 0)`
- denominator = `max(raw error, 1e-8 * numerator, 1e-20)`
- decibels = `10 * log10(numerator / denominator)`

`1e-8` caps a reported ratio at 80 dB. `1e-20` is the absolute energy floor. When either floor replaces the raw error, or the numerator itself is at the absolute floor, `capped` is set. A silent estimate of a real reference is reported at −80 dB with the cap set. A reference whose energy is at or below `1e-20` is not scored. A half-gain copy of the reference scores as a match, so this ratio alone does not prove that a stream isolated an instrument.

Projected fraction: the same numerator divided by `max(||estimate||², 1e-20)`, clamped to 0..=1. One means the estimate is a scaled copy of that reference.

Energy share: `||stream||² / ||input||²`. Because the streams overlap in time, the shares do not have to sum to one.

Mixture residual: `10 * log10(max(||mix - (harmonic + percussive)||², 1e-20) / max(||mix||², 1e-20))`. The masks sum to one, so this residual is the short-time transform round trip. It is not a separation score.

The mixture baseline is the same scale-invariant ratio with the mix used as the estimate. An equal-energy pair of nearly orthogonal sources sits near 0 dB. Improvement is the stream's ratio minus that baseline.

## Measured synthetic results

Collected 7 November 2026 (Europe/Helsinki) with `cargo test --locked -p compas-dsp --lib`, rustc 1.96.0 host `aarch64-apple-darwin`, on the Apple M4 Pro with 24 GB. The `compas-dsp` dev profile compiles the library at opt-level 2. These are assertions that passed, plus the values the tests printed. They are not a listening test and not a real-song result.

| Case | What was synthesized | Result |
| --- | --- | --- |
| Chirp round trip | 1 s sweep, 80 Hz upward | Mixture residual −139.78 dB |
| Tone plus noise bursts | 440 Hz sine and equal-energy 180-sample bursts every 3,528 samples, overlapping the whole second | Tone 21.66 dB versus mixture −0.05 dB (gain 21.70 dB). Bursts 21.98 dB versus mixture −0.05 dB (gain 22.02 dB). Burst fraction inside the harmonic stream 0.001. Tone fraction inside the percussive stream 0.000. Residual −139.37 dB. |
| Low sine | 80 Hz sine plus equal-energy bursts | Harmonic stream 22.55 dB, percussive stream −47.76 dB. The low tone follows the harmonic stream. |
| Center versus one side | The same tone-plus-burst mix on both channels, then on the left only | Both harmonic readings are 23.41 dB and the samples match bit for bit. A tone that exists only on the right scores 29.93 dB there. Its fraction inside the other channel's harmonic stream stays under 0.05. |
| Steady noise | One second of full-length noise, no second source | Energy shares 0.226 harmonic and 0.310 percussive. Both are above 0.1 and below 0.5. The noise is divided, not isolated. |
| Pitched decay alone | 140 Hz, 0.18 s decay, hit twice per second | Projected fraction 0.987 harmonic and 0.036 percussive. A steady 140 Hz sine's percussive fraction is 0.005. The decay is still a harmonic classification. |
| Three overlapping sources | Equal-energy three-note pad (220, 277, 330 Hz), noise bursts, and a 110 Hz decay | Pad fraction 0.504 harmonic and 0.000 percussive. Burst fraction 0.967 percussive and 0.001 harmonic. Decay fraction 0.479 harmonic and 0.002 percussive. The two pitched sources share the harmonic stream (their fractions sum above 0.90) and neither exceeds 0.75. |
| Short buffer | 300 samples of a 440 Hz sine | Residual −142.16 dB. A repeat call matches bit for bit. Silence of 4,410 samples returns exact zeros. |

## Where this fails

The easy tone-versus-burst case is about 22 dB better than listening to the mix. That number does not travel to the other fixtures.

A pitched decay, including one that resembles a tom body, is placed with the sustained tones. In the three-source mix its percussive fraction is 0.002. Taking the percussive output would keep the noise bursts and drop the pitched ring. Taking the harmonic output would keep the pad and the ring together. Neither output is a drum stem, a bass stem, or a vocal stem.

Continuous noise is spread across both streams. Reporting a scale-invariant ratio there would be misleading, because a scaled copy of the same noise scores as a match. The energy shares are the relevant figure, and they show a split rather than an instrument.

Mono, stereo, silence, a buffer shorter than the window, a repeated call, a cancelled call, non-finite samples, a third channel, and a length above the cap are defined. They do not add a musical identity.

## Time and memory

One test separates 88,200 samples on each of two channels (2 s at 44.1 kHz). Three observed wall times of that test during this session were 177.8 ms, 159.8 ms, and 168.3 ms. That is one test body, not a benchmark series, and it is not deck latency.

`planned_bytes` for that call is 7,288,212 bytes. For two channels at the 1,440,000-sample cap it is 115,457,172 bytes. The count includes the padded copy, the Hann envelope, one channel's complex spectrogram, three magnitude buffers, the FFT scratch queried from `rustfft`, and the retained outputs. Resident memory was not measured. The caller's mix buffer is not included.

## Next decision

Stop here. Do not connect this function to a deck, a pad, or the UI, and do not label either stream as vocals, drums, bass, or other. A later classical experiment would need its own owner decision. The model gate in ADR-0004 stays closed.

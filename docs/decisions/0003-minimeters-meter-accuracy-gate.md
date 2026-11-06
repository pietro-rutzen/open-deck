# ADR-0003 — Live MiniMeters accuracy gate for deck meters

- Status: proposed
- Date: 2026-11-06
- Author / accountable role: Architect
- Decision authority / review evidence: Dev Manager T-045 REVIEW / design gate; awaiting orchestrator acceptance after live evidence
- Related task / milestone: T-045 (optional live deck meters)
- Supersedes / superseded by: none

## Context and constraints

OpenDeck ships demand-driven deck meters (stereo image, oscilloscope, spectrogram) fed by an 8,192-frame pre-fader stereo tap (`compas-audio` `ScopeSlot`), with Blackman-Harris / zero-padded FFT analysis off the audio thread (`deck_meters.rs`). Offline desktop tests claim tone frequency/level accuracy; **a live comparison against MiniMeters was not run** (T-045).

MiniMeters is the external reference for this gate only. Hardware/Core Audio long-session dropout work remains a separate open checkbox.

Paint paths (`DeckMeters.tsx`) map PCM and dBFS into CSS/canvas geometry. Frontend workspace sizing (`useElementSize`, container-query strip heights) can change pixel scale without changing engine contracts.

## Options

1. **Ship on offline tests alone.** Fast; leaves live scaling/windowing/tap mismatches undiscovered against a DJ-facing meter app.
2. **Ad-hoc eyeball vs MiniMeters.** Cheap; no tolerances, no pass/fail, FE/DSP regressions stay argumentative.
3. **Explicit live comparison protocol with numeric tolerances and contract freeze.** Small doc cost; unblocks FE/DSP without reopening analysis math casually.

## Decision and rationale

Adopt option 3. Treat MiniMeters as the live reference for stereometer, oscilloscope, and spectrum **only after** aligning the signal path (same tone source, same device rate, OpenDeck pre-fader vs MiniMeters input of that same stream). Offline unit tolerances remain the floor; live checks may loosen slightly for device/IPC jitter but must still be recorded.

### What OpenDeck already claims (verified offline, not live)

| Meter | Claim (from T-045 + `deck_meters` tests) |
| --- | --- |
| Tap | Contiguous 8,192 stereo frames; post EQ/FX, **before channel fader**; one-shot mailbox; cueable with fader at 0 |
| Spectrum | 4-term Blackman-Harris on capture window; zero-pad to 16,384; coherent-gain amplitude so full-scale sine → 0 dBFS; max of L/R; parabolic peak; 384 log rows 20 Hz→Nyquist (cap 20 kHz); floor −72 dBFS |
| Spectrum accuracy | 65.36 Hz tone within **0.5 Hz** and **0.25 dB** at 44.1 / 48 / 96 kHz; half-scale → −6 dBFS; left-only half-scale ≈ −6.02 dBFS; antiphase retains level |
| Oscilloscope | Rising-edge trigger on louder channel; ~20 ms span; 192 display points; samples clamped to ±1 |
| Stereo image | Mid/side plot of same triggered L/R points (no separate analysis path) |
| FE poll | ~15 Hz when selected, visible, active; hidden deck ≤1 leftover capture |

### Live checklist (must run before calling meters “MiniMeters-accurate”)

Shared setup: known tones (or looped WAV) into one deck; note macOS version, device, firmware if controller involved, sample rate, buffer, OpenDeck revision; route MiniMeters to the **same** monitored stream OpenDeck taps (pre-fader / PFL-equivalent — not post-master unless explicitly testing master).

1. **Spectrum — frequency:** 65.36 Hz, 1 kHz, 8 kHz tones; compare OpenDeck peak Hz label vs MiniMeters peak/cursor. Pass: ≤ **1 Hz** or ≤ **0.5%** relative (whichever larger) at 48 kHz; record outliers at 44.1/96.
2. **Spectrum — level:** 0 dBFS and −6 / −12 dBFS sines; compare OpenDeck dBFS label vs MiniMeters. Pass: ≤ **0.5 dB** (live) with windowing/averaging differences noted; offline suite stays 0.25 dB.
3. **Spectrum — windowing/leakage:** single tone; confirm no false side peaks above −36 dBFS that MiniMeters does not show (Blackman-Harris contract).
4. **Oscilloscope:** same sine; period and amplitude envelope match MiniMeters visually; OpenDeck full-scale fills ±plot radius (not auto-gained). Pass: period error ≤ **1 sample** at trigger; amplitude within **5%** of full-scale mapping.
5. **Stereo / stereometer:** mono in-phase → vertical energy; antiphase → horizontal; L-only / R-only quadrants. Pass: qualitative match to MiniMeters goniometer; no collapse when fader is down if cue/PFL path still feeds MiniMeters equivalently — **document if MiniMeters cannot see pre-fader**.
6. **Pre-fader contract:** channel fader at 0, deck playing + cued: OpenDeck meters still show signal; master may be silent. If MiniMeters is on master only, this check is OpenDeck-internal + headphone bus, and is **not** a MiniMeters fail.
7. **Rate matrix:** at least 48 kHz required; 44.1 and 96 preferred for spectrum items 1–2.

### Contract risks

- **Scaling:** Analysis uses coherent gain (`window_sum`) and reports dBFS clamped to [−72, 0] on rows (peaks allow up to +12 before clamp). FE color map is `(db + 72) / 72`. Changing either without updating the other breaks “0 dBFS = full-scale sine.”
- **Windowing:** BH4 + pad length are load-bearing for 0.5 Hz bass localization. Shorter windows or Hann swaps need a new ADR + re-run of this gate.
- **Pre-fader tap:** Scope writes `rendered.pre_fader` in `next_buses`. Moving the tap post-fader or post-crossfader changes DJ meaning (inspect cued tracks) and invalidates MiniMeters comparisons taken from a different point.
- **Trigger / pair integrity:** Oscilloscope and stereo share one triggered L/R slice; resampling display points must keep pairs aligned or the goniometer lies.
- **FE sizing vs DSP scaling (flag):** **Conflict risk is real for visual meters, low for numeric spectrum readout.** `DeckMeters` paints with `clientWidth`/`clientHeight` and DPR-backed canvas; stereo/scope map ±1.0 into `radius` / half-width. A FE “sizing fix” that auto-normalizes amplitude to fill the canvas, stretches without updating DPR, or changes spectrum CSS height while reinterpreting dBFS would **look** like a DSP error against MiniMeters. `useElementSize` today drives waveforms/SVG, not these canvases, but strip `clamp(...)` heights already change geometry. **Rule:** FE sizing work may change layout pixels only; it must not rescale PCM, dBFS, or invent display auto-gain. Live stereo/scope judgments should prefer geometry-independent checks (phase orientation, period) over absolute Lissajous size.

## Consequences and validation

- T-045 stays incomplete for the MiniMeters checkbox until the live table is filled (pass/fail + setup notes).
- DSP/analysis changes that touch window, FFT length, gain, tap point, or trigger require re-running this gate.
- FE-only sizing/CSS changes do **not** require re-running spectrum numeric checks if peak Hz/dBFS labels are unchanged; they **do** require a quick stereo/scope visual smoke if plot radius mapping changed.
- Mark this ADR accepted only after at least one recorded live session meets the tolerances above; documentation alone is not acceptance.

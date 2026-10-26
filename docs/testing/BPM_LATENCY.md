# T-003 library analysis handoff

Changed source: staged lib.rs + new library_analysis.rs (root integrated); deck-analysis persistence now emits library:changed after a successful cache update. No database schema, callbacks or deck loading semantics changed.

Background worker streams only first 90 s of source-rate stereo, stores mono (max 69.12 MB at 192 kHz; typical 15.9 MB at 44.1 kHz), runs unchanged estimators, saves BPM/key then emits library:changed. Selected track priority is one retained path, applied after active track; queue queries skip cached BPM. Pending kick flag closes import/exit race. DB read/write errors terminate instead of looping indefinitely. Existing 0-BPM failure sentinel preserved. Analysis supports up to 192 kHz/eight source channels; playback unaffected.

`prioritize_analysis(path: string) -> Result<(), String>` validates imported absolute library paths; Tauri AppHandle is injected (no frontend argument).

Checks:
- `rustfmt --edition 2021 --config skip_children=true` on staged Rust files: passed.
- Standalone module tests linked against real workspace deps: 2 passed (first-90-second mono exact sample equality to old full decode; missing file returns error). Integrated native library suite passed all 35 tests.
- `python3 docs/testing/run-bpm-benchmark.py` compiles rustc harness against most-recent modified real workspace compas_sources/compas_dsp rlibs; uses generated 120 BPM WAV + three already imported tracks from a read-only SQLite query. It never modifies track metadata or files.
- Actual root-built optimized artifacts in first integrated benchmark: compas_sources-89cc88d6b34a61aa, compas_dsp-56abf7175e6b625d. Unoptimized harness matches shell-side worker profile.

Measurements, milliseconds (single warm runs; generated WAV then three imported tracks):
| Configuration | Full decode + existing analysis | Bounded-window analysis |
|---|---:|---:|
| Original debug, generated WAV | 1701 | 1841 |
| Original debug, imported 1 | 5409 | 5017 |
| Original debug, imported 2 | 5730 | 5006 |
| Original debug, imported 3 | 5305 | 5057 |
| Root optimized workspace, generated WAV | 72 | 82 |
| Root optimized workspace, imported 1 | 386 | 332 |
| Root optimized workspace, imported 2 | 560 | 299 |
| Root optimized workspace, imported 3 | 383 | 301 |

Each run asserts exact baseline/window equality of BPM, BPM confidence, beat interval, first-beat phase, key and key confidence; all passed. Windowing alone helps long-track decode, but workspace DSP dev optimization is the major improvement (roughly 5 s to 0.3 s per tested imported track combined). Short files gain nothing from windowing, and timing fluctuates with CPU/build activity. Microbenchmark timing excludes IPC/UI refresh and is not a native end-to-end claim.

NOT RUN by specialist: full native UI interaction, hardware listening/dropout qualification, controller tests, very large library stress, deterministic worker-exit concurrency stress. Root owns integration builds/review and tracker/handoff records. Ready for review; do not mark hardware milestones complete.

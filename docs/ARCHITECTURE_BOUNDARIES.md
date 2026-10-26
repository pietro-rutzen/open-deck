# Proposed architecture and ownership

Status: planning baseline, pending Compás audit. Names below are proposed paths, not verified upstream APIs. Prefer adapting/extracting existing functionality over rewriting DSP; choose fork versus dependency only after M1. Preserve upstream notices and track local patches.

```text
AGENTS.md
agents/                         role instructions
apps/desktop/src/               React/TypeScript presentation
apps/desktop/src-tauri/         Tauri commands, lifecycle, capabilities
crates/contracts/               versioned commands/state/errors
crates/engine-adapter/          narrow OpenDeck facade over pinned Compás
crates/audio/                   real-time mixer integration and DSP glue
crates/platform-macos/          audio device lifecycle and routing
crates/controllers/             MIDI parsing, mappings and feedback
crates/library/                 local-file metadata, analysis workers, persistence
mappings/ddj-flx4/              versioned mapping and captured fixtures
tests/                         verification suites and fixtures
docs/                          plans, tasks, decisions, handoffs, test evidence
```

`tests/` will contain offline audio fixtures, IPC integration, UI and hardware procedures. Exact crates/layout may change after audit; record the change rather than duplicating upstream subsystems.

## Data flow

React → typed Tauri command → validated control service → bounded engine command queue → Rust engine callback.
CoreMIDI input → parser/mapping → same control service/command model → engine queue. High-rate controller processing never depends on React. Platform APIs may route through worker threads; never through the callback.
Engine → preallocated state/meter snapshots → control service → rate-limited/coalesced Tauri events → React. MIDI LED feedback is a bounded control-side consumer of engine state.
Local-file decode, waveform analysis and persistence run on workers; prepared audio reaches playback through preallocated buffers. Master and headphone cue buses route independently through the selected output device.

## Ownership and interfaces

| Area | Accountable specialist | Required collaborator/reviewer |
|---|---|---|
| Architecture and cross-layer contracts | architect-planner | backend-rust, audio-dsp |
| React implementation | frontend | ui-ux, QA |
| Interaction specification and visual assets | ui-ux | frontend, macOS-MIDI for controller semantics |
| Tauri shell, library, IPC, engine facade | backend-rust | architect, audio-dsp for callback reachability |
| Mixer, transport timing, DSP glue | audio-dsp | backend-rust, QA |
| Device lifecycle, routing, CoreMIDI, mappings | macos-midi | audio-dsp, QA |
| Automated verification and test evidence | qa-test | relevant domain reviewer |
| Contributor docs, packaging and release evidence | docs-release | QA, macos-midi |

Role ownership is not a permanent exclusive write permission: every task specifies exact paths. The orchestrator reserves shared manifests, lockfiles and generated contracts for one writer at a time.

## Contract to establish before parallel implementation

Define deck identifiers, sample-frame time, command sequence IDs, load generations, legal transport transitions, parameter ranges/units, errors and acknowledgement semantics. Reject NaN/infinite/out-of-range values. Specify queue capacity and overflow behavior by command type: coalescing can suit continuous controls; transport/load commands need explicit rejection or acknowledgement, never silent loss. UI must reconcile stale/out-of-order events.

DSP owns sample-accurate timing. Define smoothing, clipping/headroom policy, buffer ownership and resource reclamation off the audio thread. Audit atomics and queue implementations on target architectures. No callback-owned object may perform blocking or unbounded destruction. Set performance budgets using a pinned build and hardware; proposed targets are in the tracker, not compatibility claims.

Device selection must expose actual channel capabilities. Validate master/cue separation physically before claiming FLX4 audio support. Disconnection, default-device change, sleep/wake and rate/buffer changes require explicit lifecycle states, safe silence and recovery behavior. MIDI mappings need provenance and captured messages; never guess controller byte sequences.

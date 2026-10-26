# Real-time audio / DSP specialist

## Mission

Protect callback safety, sample-clock timing and audible correctness of the Rust audio path.

## Required context

Read root AGENTS.md, workflow, progress, assigned task and relevant ADRs. Follow the task's exact writable paths over the typical scope below. AI is used to develop OpenDeck only; no AI product features.

## Typical scope

crates/audio/ and assigned engine-adapter/upstream patches, offline DSP tests and benchmark fixtures.

## Work instructions

- Audit the full callback call graph including dependencies, lazy initialization, buffer growth and destructors.
- No allocation/deallocation, blocking, locks, I/O, logs or unbounded work in the callback. Prepare and reclaim resources on control threads.
- Specify sample-accurate transport, smoothing, cue/master buses, gain/headroom, resampling and underrun behavior.
- Use deterministic fixtures for DSP properties and target-hardware measurements for callback duration/dropouts; report tolerances and conditions.

## Completion contract

A callback audit, offline correctness evidence, performance measurements or explicit pending hardware gates, and documented queue/buffer invariants. Use `docs/templates/HANDOFF.md` at every task end or interruption. Include changed files, revision, commands/results, NOT RUN checks, risks and next action. Report ready-for-review; do not self-certify milestone completion or edit shared progress without an exclusive claim.

## Review and escalation

Backend reviews lifetimes and queue ownership; QA independently checks fixtures and measurements; macOS/MIDI reviews routing. Stop and report overlapping writes, missing prerequisites, unsafe boundaries or scope changes to the orchestrator. Do not spawn subagents or silently expand the task. Never claim hardware validation from mocks.

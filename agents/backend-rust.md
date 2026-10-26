# Backend / Rust specialist

## Mission

Implement Tauri control services, engine facade, file library and worker lifecycles.

## Required context

Read root AGENTS.md, workflow, progress, assigned task and relevant ADRs. Follow the task's exact writable paths over the typical scope below. AI is used to develop OpenDeck only; no AI product features.

## Typical scope

apps/desktop/src-tauri/, crates/contracts/, crates/engine-adapter/, crates/library/ and related tests within task claims.

## Work instructions

- Inspect and pin upstream interfaces before adapting them; avoid claiming a stable standalone engine API.
- Validate IPC arguments, file access, deck IDs and finite parameter ranges; keep Tauri capabilities narrow.
- Separate decoding/analysis/persistence from callbacks; specify cancellation, load generations and shutdown.
- Use typed bounded command/state paths with explicit errors; coordinate contract generation with frontend.

## Completion contract

Rust changes with contract/error tests, exact check results and lifecycle/IPC integration evidence. Use `docs/templates/HANDOFF.md` at every task end or interruption. Include changed files, revision, commands/results, NOT RUN checks, risks and next action. Report ready-for-review; do not self-certify milestone completion or edit shared progress without an exclusive claim.

## Review and escalation

Audio-DSP reviews anything reachable from the callback; architect reviews contract changes. Stop and report overlapping writes, missing prerequisites, unsafe boundaries or scope changes to the orchestrator. Do not spawn subagents or silently expand the task. Never claim hardware validation from mocks.

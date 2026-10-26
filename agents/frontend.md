# Frontend specialist

## Mission

Implement React/TypeScript presentation and interactions against the agreed engine contract.

## Required context

Read root AGENTS.md, workflow, progress, assigned task and relevant ADRs. Follow the task's exact writable paths over the typical scope below. AI is used to develop OpenDeck only; no AI product features.

## Typical scope

apps/desktop/src/ and frontend tests; shared manifests and generated contracts only with an exclusive claim.

## Work instructions

- Render authoritative engine state; handle pending commands, errors, stale snapshots and deck identity explicitly.
- Keep audio timing, decode and controller processing out of JavaScript. Rate-limit meters and bound waveform work.
- Follow the UX interaction spec, keyboard behavior and accessible labels/focus.
- Use mock engine adapters only with clear test labels; verify the real desktop command/event path before completion.

## Completion contract

Typed UI changes, interaction tests, screenshots and evidence of real engine integration where required. Use `docs/templates/HANDOFF.md` at every task end or interruption. Include changed files, revision, commands/results, NOT RUN checks, risks and next action. Report ready-for-review; do not self-certify milestone completion or edit shared progress without an exclusive claim.

## Review and escalation

UI/UX reviews interactions; QA reviews behavior; backend reviews IPC use. Stop and report overlapping writes, missing prerequisites, unsafe boundaries or scope changes to the orchestrator. Do not spawn subagents or silently expand the task. Never claim hardware validation from mocks.

# ADR-0001 — Portable roles with one orchestrator

- Status: proposed
- Date: 2026-10-25
- Author: workflow design delivery
- Decision authority: maintainer adoption pending
- Related milestone: M0
- Supersedes: none

## Context

OpenDeck needs eight specialist disciplines coordinated through one orchestrator, plus durable progress and handoffs. Rust/Compás, Tauri 2, React/TypeScript, macOS, DDJ-FLX4 and no AI product features are user-supplied constraints. No product source or build evidence was provided in this workspace.

## Options

1. Portable Markdown roles in an existing coding harness: small setup, reviewable instructions and Git-tracked state; orchestration relies on task discipline and harness capabilities.
2. Custom LangGraph orchestrator: durable programmable state and recovery; requires runner, sandbox, tool, review and operations implementation.
3. CrewAI Crews/Flows: explicit role/task workflows; requires coding-tool and repository integration and adds Python infrastructure.
4. OpenHands platform: self-hosted agent control plane; additional deployment/operations surface and still needs local macOS validation.

## Recommendation

Adopt option 1, initially using Codex CLI where available, while keeping all roles portable. Use agency-agents and wshobson/agents selectively as design references. Do not vendor or execute entire libraries. See [research](../AGENT_RESEARCH.md).

## Consequences and validation

One orchestrator writes shared state and integrates changes. Specialists have bounded assignments, separate review and explicit evidence. Markdown enables resumption but is not an automatic scheduler or permission system. Run a pilot task with read-only specialists, then one isolated implementation/review cycle. Mark this ADR accepted only when the maintainer adopts the approach; documentation delivery alone does not prove runtime operation. Reconsider a durable framework if actual task-queue/recovery needs justify it.

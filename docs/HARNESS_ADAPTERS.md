# Harness adapters

Canonical behavior lives in AGENTS.md and `agents/*.md`. The roles are not executable agents by themselves. This kit does not install a runtime, enable tools or change permissions.

## Portable adapter (recommended first)

Start an existing coding harness in the repository root and give it:

```text
Read AGENTS.md and agents/orchestrator.md. Continue from
docs/PROJECT_PROGRESS.md using docs/AGENT_WORKFLOW.md.
Delegate bounded assignments to the specialist role files when supported;
otherwise execute assignments sequentially. Record task claims, independent
review, evidence and handoffs. Begin with the earliest unblocked milestone.
Do not add AI functionality to the DJ application.
```

No agent-registration syntax is needed. The orchestrator must explicitly give each worker its assigned role file and task; arbitrary Markdown role files are not automatically discovered by every harness.

## Optional Codex project agents

[Official documentation](https://learn.chatgpt.com/docs/agent-configuration/subagents) describes standalone project files under `.codex/agents/` with `name`, `description` and `developer_instructions`. Verify the installed client's supported format before adopting it. Keep default model/effort and inherited permissions unless the team has a measured reason to change them.

Example optional `.codex/agents/audio-dsp.toml` (create in the real repository after checking client support):

```toml
name = "audio-dsp"
description = "OpenDeck real-time audio and DSP implementation/review"
developer_instructions = """
Read AGENTS.md, docs/AGENT_WORKFLOW.md and agents/audio-dsp.md before work.
Follow the orchestrator's task contract and exact writable paths.
Return verification evidence and a durable handoff. Do not spawn subagents.
"""
```

Repeat the wrapper for the other eight role names if native registration is useful. The root session remains orchestrator; specialists do not invoke an additional orchestrator. Keep wrappers thin so canonical role instructions do not drift. Validate one read-only delegation and handoff before enabling concurrent writes.

[Official AGENTS.md guidance](https://learn.chatgpt.com/docs/agent-configuration/agents-md) documents discovery from project root toward the working directory. Launch at the repository root and explicitly load role/context files. A Markdown instruction cannot restrict OS permissions: use the harness's actual sandbox/permission controls and review diffs.

## Other harnesses

Pass canonical files directly in the assignment. If a client needs a native agent file or project instructions file, create a minimal adapter referencing AGENTS.md and the role; verify syntax against that client's current documentation. Do not copy tool permissions or install scripts from role libraries blindly. Sequential role assignments remain valid when native subagents are unavailable.

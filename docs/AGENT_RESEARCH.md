# Coding-agent research and recommendation

Researched 2026-10-25 from primary repository documentation. Capabilities below are source descriptions; fit and adoption judgments are OpenDeck recommendations, not benchmark results. Open source harnesses do not imply free model inference or unrestricted hosted services.

## Practical options

| Option | Kind / source license shown | Useful capability | OpenDeck cost or limitation | Recommendation |
|---|---|---|---|---|
| [Codex CLI](https://github.com/openai/codex) | Local coding harness; Apache-2.0 | Repository instruction entry point and delegated coding work | Model access and client capabilities vary; shared files still need ownership | Preferred starting harness with portable Markdown roles |
| [OpenHands](https://github.com/OpenHands/OpenHands) | Coding-agent platform; repository currently presents Agent Canvas, MIT | Self-hosted control center for coding agents and automations | Larger service/operations surface; remote environments cannot prove macOS hardware behavior | Alternative if a self-hosted team control plane becomes necessary |
| [LangGraph](https://github.com/langchain-ai/langgraph) | General orchestration framework; MIT | Stateful graphs, durable execution and human oversight | Must build coding tools, isolation, task runner and recovery behavior | Later option for a custom durable orchestrator; unnecessary for MVP |
| [CrewAI](https://github.com/crewAIInc/crewAI) | General multi-agent framework; MIT | Role-based Crews and event-driven Flows | Additional Python runtime and coding-tool integration to maintain | Useful for workflow experiments; defer for core development |
| [agency-agents](https://github.com/msitarzewski/agency-agents) | MIT role/prompt library | Engineering, design and testing role patterns | Broad agency scope; prompts do not enforce audio safety or execute tasks by themselves | Selective inspiration for responsibilities and deliverables |
| [wshobson/agents](https://github.com/wshobson/agents) | MIT core multi-harness role/plugin catalog; external plugins have separate licenses | Reusable engineering, review and testing components | Installation differs by harness; importing everything expands instructions and tool surface | Select only relevant components after review; keep local roles authoritative |

## Decision

Use a thin orchestrator in an existing coding harness, not a new framework inside OpenDeck. Start with Codex CLI and canonical `AGENTS.md` plus the nine role files. Keep task and handoff state in Git-tracked Markdown. Use harness-native subagents for independent bounded tasks and sequential assignments when delegation is unavailable. Durable state is the repository record, not an agent's conversation history.

Do not install entire role collections. Their role vocabulary is useful, but OpenDeck's DSP and macOS/MIDI roles must specify callback behavior, channel routing, actual hardware evidence and clock ownership. Original local role instructions avoid dependence on third-party personalities, tool names or blanket permissions. No third-party prompt text is copied in this kit.

Adopt LangGraph only if measured needs emerge for unattended multi-session queues, checkpoint recovery and externally scheduled workflows. First define an idempotent job runner, sandbox/worktree boundaries, approval policy and cost limits. CrewAI's role abstraction alone does not solve coding integration or hardware testing. OpenHands is a platform alternative rather than a required layer on top of another harness.

## Role-library mapping

| OpenDeck role | Relevant reusable patterns | Local additions required |
|---|---|---|
| Orchestrator | Task decomposition, coordination, review routing | Single writer for shared state; acceptance gates |
| Architect/planner | Architecture and implementation planning | Compás extraction audit, real-time boundary ADRs |
| Frontend | Frontend development roles | Engine-truth display; bounded waveform rendering |
| UI/UX | UX research and interface design roles | Deck focus, cue/master clarity, performance workflows |
| Backend/Rust | Backend and language specialist patterns | Typed IPC, engine adapter, worker lifetimes |
| Audio/DSP | Custom specialist | Callback audit, DSP correctness, timing measurements |
| macOS/MIDI | Custom specialist | Core Audio routing, CoreMIDI, FLX4 evidence |
| QA | Test engineering and review patterns | Offline DSP + physical controller test matrix |
| Documentation/release | Technical writing and release patterns | License provenance, signing/notarization, honest support matrix |

## Sources and reuse policy

- [Codex repository](https://github.com/openai/codex): local harness and Apache-2.0 licensing.
- [Official AGENTS.md guidance](https://learn.chatgpt.com/docs/agent-configuration/agents-md): instruction discovery; role files elsewhere are not automatically loaded just because they are Markdown.
- [Official subagent guidance](https://learn.chatgpt.com/docs/agent-configuration/subagents): delegation and project-scoped custom agents.
- The five other option names above link directly to their primary repositories. Inspect each exact revision's LICENSE and any component licenses before importing; retain required notices.
- [Compás repository](https://github.com/sergiogallegos/compas): describes a Rust core and Tauri 2/TypeScript DJ app. Its README reports features; OpenDeck has not verified library extraction, APIs, licenses of transitive assets or hardware support. Pin a revision and audit in M1.

No frameworks, accounts, model services, plugins or external automations were installed or activated by this documentation delivery.

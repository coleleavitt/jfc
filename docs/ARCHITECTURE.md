# JFC Architecture

This document provides a high-level overview of jfc's architecture, design principles, and how the pieces fit together.

## Active Architecture Reset

JFC is in the middle of a Pi/opencode-style kernel reset. The target is a short
ownership-root layout, not another round of `jfc-*` crate sprawl. The destination
roots are `kernel`, `protocol`, `runtime`, `session`, `plugin`, `context`,
`policy`, `tools`, `providers`, `orchestration`, `daemon`, `ui-model`, `tui`,
and `cli`.

The current codebase still contains compatibility crates and adapters, but the
completed reset slices have already landed these boundaries:

- Architecture guardrails for target roots and the engine root-file freeze.
- `jfc-session` ownership of typed append entries, compatibility transcript
  fixtures, and the session store seam.
- `RuntimeServices` seams for provider lookup, tool dispatch, diagnostics,
  sessions, and frontend-neutral directives.
- `jfc-plugin-sdk` and `jfc-plugin-host` descriptor rails for tools, providers,
  resources, runtime extensions, runtime actions, UI slots, widgets, panels,
  metrics, agent launches, process-bridge frames, and diagnostics.
- Descriptor-backed first-party filesystem tool and OpenAI-compatible provider
  packs, while legacy routes remain as compatibility.
- Short-root `crates/context` and `crates/orchestration` skeletons, plus a
  daemon-owned scheduled-task service seam.

Read the root `ARCHITECTURE.md` progressive refactor ledger and `.omo/evidence/`
task files for exact landed slices before claiming a migration is complete.

## Core Systems

### 1. Code Graph (`jfc-graph`)
**1.2M LOC, 15K nodes, 29K edges**

A production-grade, polyglot code indexing engine built on tree-sitter. Extracts and analyzes source code structure from 12+ languages (Rust, TypeScript, Python, Go, Java, C/C++, C#, Kotlin, Swift, PHP, Ruby).

**Key capabilities:**
- Language-agnostic AST extraction via `LanguageAdapter` trait
- Incremental indexing with revision tracking
- Dataflow analysis, taint tracking, dominators
- Query DSL with set algebra, path patterns, preconditions
- CSR graph snapshots for parallel traversal
- Coverage-aware type propagation

**Entry point:** `crates/jfc-graph/src/lib.rs`

### 2. LLM Provider Abstraction (`jfc-provider`, `jfc-providers`)
Unified interface for multiple LLM backends (Claude, Bedrock, Vertex, Ollama, OpenWebUI, etc.).

**Key abstractions:**
- `ModelId`, `Usage`, `ToolResult` — standardized types
- `Provider` trait — implement once, work with all backends
- Retry logic, token budgets, cost tracking
- OAuth flows for workspace identity
- Batch API support (Anthropic)

**Entry point:** `crates/jfc-provider/src/lib.rs`

### 3. Tool Execution (`jfc-tools`, `jfc-mcp`, descriptor packs)
Agents request tool use (read files, grep, run bash, etc.). Tools are dispatched via:
- **MCP (Model Context Protocol)**: SSE-based bidirectional RPC
- **Descriptor packs**: built-in and plugin-owned tool descriptors registered
  through `jfc-plugin-host`
- **Direct dispatch compatibility**: legacy internal execution routes kept while
  tool families migrate behind descriptors

Supports streaming tool outputs, cancellation, resource cleanup.

**Entry point:** `crates/jfc-mcp/src/lib.rs`

### 4. Agent Lifecycle (`jfc-agents`, `jfc-core`)
Manages agent spawn, execution, task delegation, and inter-agent communication.

**Key concepts:**
- `AgentDef` — template for agents (subagent type, model, effort level)
- `TaskInput` — parameters for spawning: description, prompt, schema
- `ExecutionResult` — outcome: success/failure, tokens, final output
- Delegation chain: parent agent → subagent → sub-subagent

**Entry point:** `crates/jfc-core/src/agent_def.rs`

### 5. Daemon & Persistence (`jfc-daemon`)
Background service managing:
- **Cron jobs** — scheduled tasks (research, audits, cache cleanup)
- **Wakeups** — one-shot delayed triggers (user: "remind me in 2h")
- **Background agents** — long-running tasks with PID tracking
- **Session state** — agent execution logs, metadata

Uses DO-178B test convention for robustness (see `docs/DO-178B-TESTING.md`).

**Entry point:** `crates/jfc-daemon/src/lib.rs`

### 6. Context and Cross-Session Memory (`crates/context`, `jfc-memory`, `jfc-learn`)
Persistent memory system with:
- **User memories** — global preferences, feedback
- **Project memories** — codebase conventions, architecture notes
- **Team memories** — shared via `.jfc/memory/team/` (committed to repo)
- **Historian** — session summaries for dreamer agent
- **Dreamer** — suggests context from prior sessions
- **Key files** — pinned files for cross-session recall

**Magic Context parity target:** after JFC is reshaped around Pi/opencode-style
runtime primitives, evolve this from memory recall into a full cache-stable
context subsystem: stable baseline + volatile delta provider messages, typed
historian compartments with deterministic decay rendering, validated dreamer
maintenance jobs, `ctx_reduce`/`ctx_expand`-style replay-safe drop and expansion
tools, unified recall across memories/session/git/codegraph, and visible context
health in TUI/dashboard/doctor surfaces.

**Entry point:** `crates/jfc-learn/src/dreamer.rs`

**Current reset slice:** `crates/context` now contains layout, contributors,
health, memory, history, reduce, search, and doctor DTO modules. Its first live
integration is a non-hot doctor data report for `ContextHealth`.

### 7. TUI and CLI Shell (`jfc`, target `tui` / `cli`)
Full async event loop with:
- Character-boundary-safe cursor navigation
- Streaming response rendering
- Inline tool execution (bash, edit, grep, read)
- Agent delegation panel
- State machine for handler lifecycle

**Entry point:** `crates/jfc/src/runtime/event_loop/mod.rs`

The destination keeps ratatui drawing and terminal input in the frontend shell,
while status rows, widgets, plugin UI state, runtime actions, and domain command
logic move behind view-model, plugin-host, runtime-service, or domain-service
seams. The current migration has first seams for status rows, plugin UI refresh,
and one CLI command path.

### 8. Agent Economy (`jfc-economy`)
**Experimental multi-agent market:**
- Bounty posting (parent agent posts work)
- Solver auction (competing agents bid)
- Validator adversary (challenge each solution)
- Settlement (only validated solutions pay out)

Used for high-stakes tasks where you want cross-validated, competing solutions.

**Entry point:** `crates/jfc-economy/src/lib.rs`

## Design Principles

### 1. **Trait-First Abstraction**
- `LanguageAdapter` for all language support
- `LanguageAdapter` for all language support
- `Provider` for LLM backends
- Tool execution via trait objects and dispatch

Enables extensibility without recompilation.

### 2. **Type Safety Over Strings**
- `NodeId`, `EdgeKind`, `ModelId` are newtypes, not strings
- `Visibility::Public` not `"public"`
- Compile-time guarantees on valid states

### 3. **Incremental Computation**
- Graph revisions track who changed what
- Incremental indexing on file changes
- CSR snapshots for batched queries
- Session caching of graph queries

### 4. **Async-First**
- All I/O is async (`tokio`)
- Tool execution is concurrent (bash spawning, file I/O, HTTP)
- Event loop drives all interactions

### 5. **Testing Discipline**
- DO-178B `_normal`/`_robust` test split
- Happy path + boundary + error cases
- 723+ tests in jfc-graph alone
- No silent failures

### 6. **Modular Workspace**
Workspace crates are being reshaped around short ownership roots:
- `jfc-core` — types only (no runtime)
- `jfc-provider` — abstract interface
- `jfc-providers` — concrete implementations
- `jfc-graph` — standalone, works offline
- `jfc` — runtime, async, TUI
- `crates/context` — target context root for layout, health, memory, history, reduce, and search
- `crates/orchestration` — target orchestration root for agents, swarm, council, workflows, and goals

Allows library usage without the full runtime.

## Data Flow: Typical Session

```
User → TUI → Prompt + Context
        ↓
    Runtime (jfc)
        ↓
    Select Agent (Explore, Plan, Build, etc.)
        ↓
    Spawn Subagent via Task Tool
        ↓
    Subagent reads codebase
        ├→ Graph Query (jfc-graph)
        ├→ File Read (via tool)
        └→ Grep (via tool)
        ↓
    Subagent calls LLM (jfc-provider)
        ├→ Anthropic / Bedrock / etc.
        └→ Streaming response
        ↓
    Tool use: Edit, Bash, Read, etc.
        └→ MCP dispatch (jfc-mcp)
        ↓
    Result → Parent Agent
        ↓
    Persist to Memory (jfc-memory)
        ↓
    Display to TUI
        ↓
    Save Session (jfc-daemon)
```

## Crate Dependency Graph (High Level)

```
jfc (TUI Runtime)
  ├→ jfc-core (types)
  ├→ jfc-provider (LLM abstraction)
  ├→ jfc-providers (impl: Anthropic, Bedrock, etc.)
  ├→ jfc-graph (code indexing)
  ├→ jfc-mcp (tool dispatch)
  ├→ jfc-memory (cross-session)
  ├→ jfc-learn (historian, dreamer)
  ├→ jfc-daemon (background tasks)
  └→ jfc-economy (optional: task auctions)

jfc-agents (agent lifecycle)
  ├→ jfc-core
  ├→ jfc-daemon
  └→ jfc-provider

jfc-graph (standalone)
  ├→ tree-sitter
  ├→ petgraph
  └→ no external agent deps
```

## Testing Strategy

- **Unit tests** in each crate
- **Integration tests** for agent spawning, tool dispatch
- **DO-178B split** (see `docs/DO-178B-TESTING.md`)
- **Fixture-based** tests for graph (real code samples)

Run with:
```bash
cargo test --workspace
cargo test -p jfc-graph -- --skip xdg_cache_home  # long test
cargo clippy --all-targets  # 0 errors, <5 warnings
```

## Future Directions

1. **Runtime architecture reset**: reduce JFC to a bare runnable kernel and
   replace `EngineState`-centric growth plus `jfc-*` crate sprawl with a
   Pi/opencode-shaped runtime: service graph, session runtime factory,
   extension runner, descriptor registries, typed append entries, and curated
   runtime actions.
2. **Magic Context parity**: native cache-stable context layout, historian
   compartments, dreamer maintenance, semantic recall health, and replay-safe
   context reduction (tracked in root `PLAN.md`).
3. **Graph storage**: Move beyond in-memory indexing → RocksDB.
4. **Remote agents**: Ship jfc to serverless platforms.
5. **Distributed economy**: Multi-machine task settlement.
6. **IDE integration**: LSP server, VS Code extension.
7. **Framework awareness**: Django/FastAPI/Rails route detection.

## See Also

- `docs/DO-178B-TESTING.md` — testing discipline
- `docs/RTCA-DO-178B.pdf` — full RTCA DO-178B standard (local copy)
- `PLAN.md` — active roadmap, including Magic Context parity follow-on work
- `ARCHITECTURE.md` — root architecture reset map comparing JFC with Pi and opencode
- `.research/magic-context/ARCHITECTURE.md` — source architecture being translated into JFC
- `CLAUDE.md` — system prompt, agent definitions
- `.jfc/skills/` — predefined agent capabilities
- `crates/jfc-graph/research/` — papers on incremental, dataflow, CPG

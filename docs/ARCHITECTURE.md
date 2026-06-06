# JFC Architecture

This document provides a high-level overview of jfc's architecture, design principles, and how the pieces fit together.

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

### 3. Tool Execution (`jfc-tools`, `jfc-mcp`)
Agents request tool use (read files, grep, run bash, etc.). Tools are dispatched via:
- **MCP (Model Context Protocol)**: SSE-based bidirectional RPC
- **Direct dispatch**: Internal tool execution in the runtime

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

### 6. Cross-Session Memory (`jfc-memory`, `jfc-learn`)
Persistent memory system with:
- **User memories** — global preferences, feedback
- **Project memories** — codebase conventions, architecture notes
- **Team memories** — shared via `.jfc/memory/team/` (committed to repo)
- **Historian** — session summaries for dreamer agent
- **Dreamer** — suggests context from prior sessions
- **Key files** — pinned files for cross-session recall

**Entry point:** `crates/jfc-learn/src/dreamer.rs`

### 7. TUI Runtime (`jfc`)
Full async event loop with:
- Character-boundary-safe cursor navigation
- Streaming response rendering
- Inline tool execution (bash, edit, grep, read)
- Agent delegation panel
- State machine for handler lifecycle

**Entry point:** `crates/jfc/src/runtime/event_loop/mod.rs`

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
21 crates with clear dependencies:
- `jfc-core` — types only (no runtime)
- `jfc-provider` — abstract interface
- `jfc-providers` — concrete implementations
- `jfc-graph` — standalone, works offline
- `jfc` — runtime, async, TUI

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

1. **Graph storage**: Move beyond in-memory indexing → RocksDB
2. **Remote agents**: Ship jfc to serverless platforms
3. **Distributed economy**: Multi-machine task settlement
4. **IDE integration**: LSP server, VS Code extension
5. **Framework awareness**: Django/FastAPI/Rails route detection

## See Also

- `docs/DO-178B-TESTING.md` — testing discipline
- `docs/RTCA-DO-178B.pdf` — full RTCA DO-178B standard (local copy)
- `CLAUDE.md` — system prompt, agent definitions
- `.jfc/skills/` — predefined agent capabilities
- `crates/jfc-graph/research/` — papers on incremental, dataflow, CPG

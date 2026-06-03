# jfc

A high-performance AI coding agent for the terminal. It combines a Rust ratatui UI, durable background workers, persistent swarms, code-graph intelligence, provider OAuth, MCP tools, and a competit[ive bounty marketplace.

![Rust](https://img.shields.io/badge/rust-nightly-orange)
![License](https://img.shields.io/badge/license-AGPL--3.0-blue)

<p align="center">
  <img src="resources/image.png" alt="jfc" width="600"/>
</p>

## Quick Start

```bash
# Install from source
git clone https://github.com/coleleavitt/jfc.git
cd jfc
cargo install --path crates/jfc-ui

# Or run dev build
cargo build -p jfc-ui --bin jfc
./target/debug/jfc
```

**Usage:**
```bash
jfc                                  # Interactive TUI
jfc --continue                        # Resume most recent session
jfc -p "explain this codebase"        # One-shot prompt (print mode)
jfc --model claude-sonnet-4-6         # Specify model
jfc daemon start                      # Run background daemon
```

## Highlights

- **Full agentic loop** — streaming responses, tool calls, approval modes, task tracking, auto-compaction, and cancellation.
- **Foreground + detached background agents** — `Task` can run inline or fork a durable `jfc daemon` process that survives the TUI.
- **Persistent swarms** — spawn named teammates, send mailbox messages, share task lists/memory, and gate teammate tools through leader approval.
- **Code graph intelligence** — tree-sitter graph queries, callers/callees, path search, taint/preconditions, coverage metadata, and semantic `symbol_edit` cascades.
- **Bounty marketplace** — post coding bounties, run competing solver agents, adversarial validators, settlement, trust, and token ledger accounting.
- **Multi-provider auth** — Anthropic API/OAuth, OpenAI, Codex/ChatGPT OAuth, OpenWebUI/LiteLLM, Bedrock, and Vertex provider foundations.
- **MCP + Skills** — local and remote MCP tools, skill files, memories, web fetch/search, notebooks, cron, wakeups, LSP, notifications, and webhooks.

---

## Architecture

```text
jfc/
├── crates/
│   ├── jfc-ui/             # Main binary: TUI, event loop, tools, providers, swarm, daemon
│   ├── jfc-graph/          # Code graph, DSL, symbol table, coverage, semantic edit validation
│   ├── jfc-economy/        # Bounty lifecycle, solvers, validators, trust, ledger, settlement
│   ├── jfc-anthropic-sdk/  # Anthropic managed-session / SDK foundations
│   ├── jfc-providers/      # Multi-provider backends (Anthropic, Bedrock, Gemini, OpenAI, OpenWebUI, Codex)
│   ├── jfc-provider/       # Core provider trait, ModelSpec, StreamOptions, cost, retry
│   ├── jfc-agents/         # Agent lifecycle, registry, and state management
│   ├── jfc-audit/          # Security audit: taint, reachability, suspicious-point enumeration
│   ├── jfc-learn/          # Learning subsystem: historian, dreamer, key-files, auto-hints, verifier
│   ├── jfc-daemon/         # Background daemon: cron, PID, worker pool, state reconciliation
│   ├── jfc-core/           # Shared types: tasks, tool inputs, IDs, diffs, execution results
│   ├── jfc-config/         # Config management, feature flags, atomic writes
│   ├── jfc-auth/           # OAuth core + credential vault
│   ├── jfc-mcp/            # MCP server: tool dispatch, transport, protocol, registry
│   ├── jfc-memory/         # Memory recall & persistence
│   ├── jfc-session/        # Session + task store persistence, task status types
│   ├── jfc-tools/          # Shared tool definitions and execution contracts
│   ├── jfc-web/            # Web search (Google CSE, arXiv, Semantic Scholar)
│   ├── jfc-remote/         # Remote control: wire protocol, HMAC auth, WS transport
│   ├── jfc-markdown/       # Markdown rendering utilities, fence detection
│   └── jfc-theme/          # Terminal themes, palette validation, ANSI color management
├── .claude/skills/         # Declarative skill files
├── .claude/agents/         # Optional project agent definitions
├── .claude/workflows/      # Workflow scripts (multi-agent orchestration)
└── .jfc/memory/            # Persistent project memories
```

**Central runtime:** `AppEvent` → `BackgroundTask` — foreground subagents, detached workers, swarm teammates, and bounty solver/validator agents all stream progress into the same fan/task display.

---

## Features

### Core Agent + TUI

| Feature | Description |
| --- | --- |
| **Multi-provider** | Anthropic API/OAuth, OpenAI, Codex OAuth, OpenWebUI/LiteLLM, Bedrock, Vertex. |
| **Streaming tool loop** | Models emit tool calls, jfc executes tools, returns results, and continues until completion. |
| **Approval modes** | `plan`, `default`, `acceptEdits`, `auto`, and `bypass` modes; Shift+Tab cycles. |
| **Auto-mode classifier** | ML classifier auto-approves safe tool calls when `/auto-mode on` is active. |
| **Session persistence** | Auto-save, `--continue`, `/continue`, `/resume`, session picker/sidebar, cwd mismatch warnings. |
| **Context management** | Token gauge, auto-compaction, forced `/compact`, subagent history compaction, byte-budget tool-result caps. |
| **Sandbox** | bubblewrap (bwrap) + Landlock LSM kernel sandboxing for Bash tool execution. |
| **Slop guard** | Post-response quality checks: duplication, dead code, churn, coherence, complexity, and test quality. |
| **File checkpoints** | Automatic pre-edit snapshots with `/undo` restore and configurable pruning. |
| **Sprint budgets** | Token budget tracking with pressure warnings and handoff thresholds for long sessions. |
| **Goal loop** | `/goal <condition>` sets an autonomous objective; the agent loops until the condition is met. |

### Agents + Background Workers + Swarms

| Mode | What it does |
| --- | --- |
| **Foreground `Task`** | Runs an in-process one-shot subagent, streams `AgentChunk` and `TaskProgress` live. |
| **Detached background `Task`** | `run_in_background=true` forks `jfc daemon worker --launch <json>`. |
| **Worktree-isolated `Task`** | `isolation="worktree"` creates `.jfc-worktrees/<name>` and runs tools from that checkout. |
| **Teammate spawn** | `Task` with `name` + `team_name` creates a persistent teammate addressable with `SendMessage`. |
| **Swarm task claiming** | Teammates can claim unowned team tasks from the shared `TaskStore`. |
| **Permission sync** | Plan-mode teammates write permission requests; leader resolves with `/swarm-approve` or `/swarm-deny`. |

### Daemon / Durable Jobs

| Command | Description |
| --- | --- |
| `jfc daemon start` | Run the daemon loop in the foreground. |
| `jfc daemon stop` | Stop the running daemon via PID file. |
| `jfc daemon status` | Show daemon health and counts. |
| `jfc daemon attach <id>` | Follow a detached agent log until terminal state. |

### Code Graph (`jfc-graph`)

The graph subsystem builds a symbol/call/type graph from the workspace and exposes it through the `graph_query` tool.

| Capability | Example |
| --- | --- |
| Entrypoints | `entrypoints`, `entrypoints kind=PublicApi` |
| Function/type search | `fn("execute_tool")`, `type("Config")` |
| Traversal | `fn("execute_task") \| callees \| depth 3` |
| Set algebra | `fn("spawn") union fn("worker")` |
| Taint | `fn("parse") \| taint "input" \| depth 5` |
| Preconditions | `fn("dangerous_op") \| callers \| preconditions` |

Supported languages: Rust, TypeScript/JavaScript, Python, Go, Java, Kotlin, C, C++, C#, PHP, Ruby, Swift.

**Advanced analysis:** CFG, dataflow, taint propagation, module clustering, complexity metrics, co-change analysis, CSR, incremental re-indexing, pointer analysis, generic instantiation, polyglot resolution.

### Bounty Market (`jfc-economy`)

| Feature | Description |
| --- | --- |
| **Post** | `post_bounty` registers a task with a token budget and acceptance criteria. |
| **Solve** | `run_bounty` spawns 1-5 solver agents; each produces a patch/FILE blocks. |
| **Validate** | Validator agents inspect solutions in sealed sessions and propose flaws/tests. |
| **Settle** | Market ranks solutions, pays winners, updates trust, and records ledger usage. |

### Tools

**Filesystem/Shell:** `Bash`, `Read`, `Write`, `Edit`, `MultiEdit`, `Glob`, `Grep`

**Tasks/Teams:** `Task`, `TaskCreate`, `TaskList`, `TaskDone`, `SendMessage`, `TeamCreate`

**Code:** `GraphQuery`, `GraphContext`, `SymbolEdit`, `CodeIndex`, `RunCoverage`

**Memory/Web:** `MemoryCreate`, `WebFetch`, `WebSearch`

**Daemon:** `CronCreate`, `CronList`, `CronDelete`, `ScheduleWakeup`

**Advanced:** `PostBounty`, `RunBounty`, `Workflow`, `Advisor`, `MCP`, `LSP`, `Monitor`, `Notebook`

---

## Slash Commands

| Command | Description |
| --- | --- |
| `/help` | Show command/key help. |
| `/clear` | Clear conversation and start fresh. |
| `/continue [all]` / `/c` | Continue most recent session. |
| `/resume <id>` | Resume a saved session. |
| `/sessions` | List saved sessions. |
| `/compact` | Queue manual context compaction. |
| `/undo` | Undo recent tool edit where supported. |
| `/goal <condition>` | Keep working toward a goal condition. |
| `/workflow` / `/wf` | Workflow helper. |
| `/market` | Show bounty-market status. |
| `/graph-history` | Show recent graph queries. |
| `/tasks` | Show task list. |
| `/cost` / `/stats` | Show usage/cost stats. |
| `/mode <name>` | Show/switch permission mode. |
| `/auto-mode on\|off\|status` | Toggle classifier-based auto-approval. |
| `/theme [name]` | Pick or persist a theme. |
| `/login [provider]` | Provider login chooser. |
| `/mcp [list\|restart\|logs]` | Inspect/restart MCP servers. |

**More:** `/check`, `/config`, `/batch`, `/diff`, `/export`, `/verbose`, `/timeline`, `/doctor`, `/effort`, `/feature`, `/memory`, `/skills`, `/agents`, `/cascade`, `/worktree`, `/teleport`, `/init`, `/bug`, `/rewind`, `/pr <num>`, `/swarm-approve`

---

## Keybindings

| Key | Action |
| --- | --- |
| `Enter` | Send message. |
| `Shift+Enter` | Newline in input. |
| `Shift+Tab` | Cycle permission mode. |
| `Ctrl+P` | Command palette. |
| `Ctrl+B` | Toggle sessions sidebar. |
| `Ctrl+S` | Toggle info sidebar. |
| `Ctrl+M` | Open model picker. |
| `Ctrl+O` | Expand reasoning / diagnostic panel. |
| `Ctrl+Y` | Yank last assistant message to clipboard. |
| `Ctrl+C` | Cancel streaming / exit. |
| `Esc` | Dismiss popup / close panel. |
| `@` | Autocomplete file paths. |
| `↑/↓` | Scroll or recall input history. |

---

## Configuration

**Config path:**
```text
~/.config/jfc/config.toml
```

**Example:**
```toml
[model]
default = "claude-sonnet-4-6-20250514"

[compact]
auto_pct = 80

[permissions]
mode = "default"  # plan | default | accept | auto | bypass

[daemon]
max_sessions = 5
cleanup_after_hours = 24

[agents.Explore]
model = "litellm/qwen/qwen3.6-35b-a3b:coding"

[agents.Plan]
model = "claude-opus-4-7"
```

**Project feature config** (`.jfc/features.toml`):
```toml
[permissions]
enabled = true

[background]
max_concurrent = 10
```

**Built-in subagents:**

| Subagent | Default Model | Role |
| --- | --- | --- |
| `Explore` | `haiku` | Read-only codebase search and grep |
| `general-purpose` | inherit | Multi-step tasks with full tool access |
| `Plan` | inherit | Architecture design, read-only planning |
| `verification` | inherit | Post-edit testing |
| `orchestrator` | inherit | Decomposes broad requests into subtask plans |

**Environment variables:**

| Env var | Effect |
| --- | --- |
| `JFC_LITELLM_API_KEY` | LiteLLM proxy API key. |
| `JFC_LITELLM_API` | LiteLLM proxy base URL. |
| `JFC_DISABLE_BELL=1` | Silence terminal bell. |
| `JFC_DISABLE_AUTO_COMPACT=1` | Disable auto-compaction. |
| `JFC_ADVISOR_ENABLED=1` | Enable `/advisor`. |
| `JFC_WORKER_BIN=/abs/path/jfc` | Force detached worker executable path. |

---

## Skills & Agents

**Skills** are Markdown files in `.claude/skills/` with optional YAML frontmatter:

```markdown
---
name: my-skill
description: Domain-specific instructions
---
Instructions for the agent when this skill is invoked.
```

**Agents** are Markdown files in `.claude/agents/` with YAML frontmatter:

```markdown
---
name: Explore
model: openai/gpt-5.1
permissionMode: default
allowedTools: [Read, Glob, Grep, graph_query]
disallowedTools: [Write, Edit]
skills: [ripgrep]
---
You explore codebases and report concise, cited findings.
```

**Built-in skills:** `do-178b`, `vuln-researcher`, `git-master`, `rust-style`, `tracing`, `snafu`, `thiserror`, `ripgrep`.

---

## MCP (Model Context Protocol)

- Stdio and SSE transports
- Tool registry with schema validation
- Dynamic tool dispatch to registered handlers
- Server lifecycle management (`/mcp list`, `/mcp restart`, `/mcp logs`)
- Client-side: discovers and calls remote MCP tools as `mcp__<server>__<tool>`

---

## Performance

- Markdown/tool rendering cached by content hash and viewport width.
- Virtual scrolling with cached tool heights.
- Read-only tools dispatched in parallel.
- Detached background workers don't block the TUI event loop.
- Graph sessions memoize queries and invalidate after file edits.
- Subagents auto-compact or elide history before oversized requests.
- Web results cached in LRU.
- Incremental graph re-indexing via file watcher.

---

## Development

```bash
cargo fmt --all --check
cargo check -p jfc-ui
cargo test -p jfc-ui app::tests::
cargo test -p jfc-graph
cargo test -p jfc-economy
cargo clippy --workspace
```

See `.claude/` for skills, agents, and workflows.

---

## License

[AGPL-3.0](LICENSE)

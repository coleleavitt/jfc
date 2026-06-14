# jfc

A high-performance AI coding agent for the terminal. Built in Rust with a ratatui TUI, persistent swarms, code-graph intelligence, multi-provider auth, and a bounty marketplace for competitive code solving.

![Rust](https://img.shields.io/badge/rust-nightly-orange)
![License](https://img.shields.io/badge/license-AGPL--3.0-blue)
[![CodSpeed](https://img.shields.io/endpoint?url=https://codspeed.io/badge.json)](https://app.codspeed.io/coleleavitt/jfc?utm_source=badge)

<p align="center">
  <img src="resources/image.png" alt="jfc" width="600"/>
</p>

## Quick Start

```bash
# Install
git clone https://github.com/coleleavitt/jfc.git
cd jfc
cargo install --path crates/jfc

# Run
jfc                           # Interactive TUI
jfc --continue                # Resume last session
jfc -p "explain this code"    # One-shot print mode
jfc daemon start              # Run background daemon
```

## What Makes jfc Different

- **Foreground + detached background agents** — Tasks run inline or fork durable daemon processes that survive the TUI.
- **Persistent swarms** — Named teammates with mailbox messaging, shared task lists, and permission gating.
- **Code graph DSL** — Tree-sitter powered queries: `fn("name") | callees | depth 3 | taint "var"` with multi-language support.
- **Bounty marketplace** — Post tasks, spawn 1-5 competing solvers, adversarial validators, and settlement with trust scoring.
- **MCP + Skills** — Model Context Protocol servers, declarative skill files, learning/memory subsystem, and remote control.
- **Multi-provider** — Anthropic API/OAuth, OpenAI, Codex, OpenWebUI/LiteLLM, Bedrock, Vertex with unified auth layer.

---

## Crates Overview

| Crate | Purpose |
| --- | --- |
| **jfc** | Main binary: TUI event loop, streaming tool calls, approval modes, session persistence |
| **jfc-graph** | Code graph builder + DSL query engine (tree-sitter, SCC, dataflow, taint, preconditions) |
| **jfc-economy** | Bounty lifecycle: solvers, validators, trust, ledger, settlement |
| **jfc-anthropic-sdk** | Anthropic managed-session SDK foundations and model streaming |
| **jfc-providers** | Multi-provider backends (Anthropic, Bedrock, Gemini, OpenAI, OpenWebUI, Codex) |
| **jfc-provider** | Core provider trait, ModelSpec, StreamOptions, cost, retry logic |
| **jfc-agents** | Agent lifecycle, registry, state management, subagent spawning |
| **jfc-audit** | Security: taint analysis, reachability, vulnerability enumeration, suspicious-point detection |
| **jfc-learn** | Learning subsystem: historian, dreamer, key-files, auto-hints, memory verification |
| **jfc-daemon** | Background daemon: cron, PID, worker pool, state reconciliation, durable job tracking |
| **jfc-core** | Shared types: tasks, tool inputs, IDs, diffs, execution results |
| **jfc-config** | Config management, feature flags, atomic writes, TOML parsing |
| **jfc-auth** | OAuth core, credential vault, account management, provider flows |
| **jfc-mcp** | MCP server: tool registry, dispatch, Stdio/SSE transports, lifecycle management |
| **jfc-memory** | Memory recall, persistence, deduplication, user/project memory promotion |
| **jfc-session** | Session + task store, persistence layer, task status types, hydration |
| **jfc-tools** | Shared tool definitions: Bash, Read, Write, Edit, Graph, Market, Web, Notebook |
| **jfc-web** | Web search (Google CSE, arXiv, Semantic Scholar), fetch, caching |
| **jfc-remote** | Remote control: wire protocol, HMAC auth, WebSocket transport, managed sessions |
| **jfc-markdown** | Markdown rendering, fence detection, syntax highlighting utilities |
| **jfc-theme** | Terminal themes, palette validation, ANSI color management |
| **jfc-bridge** | *New* — OAuth bridge, request attestation, credential exchange |
| **jfc-changeset** | *New* — Diff tracking, file changes, cascade planning for symbol edits |

---

## Architecture

**Core runtime:** `AppEvent` → `BackgroundTask`

```
┌─────────────────────────────────────┐
│        jfc (Main Binary)         │
│  TUI + Event Loop + Tool Dispatch   │
└──┬──────────────────────────────────┘
   │
   ├─→ jfc-graph        (Code analysis)
   ├─→ jfc-agents       (Subagent spawn/registry)
   ├─→ jfc-session      (Persistence)
   ├─→ jfc-providers    (Model backends)
   ├─→ jfc-tools        (Bash, Edit, etc.)
   ├─→ jfc-daemon       (Background workers)
   │
   └─→ Foreground/Detached/Swarm Tasks
       ├─ Streaming subagents
       ├─ Durable daemon workers
       ├─ Swarm teammates
       └─ Bounty solvers/validators (jfc-economy)
```

All modes stream progress into the same `TaskPanel` for unified visibility.

---

## Key Features

### Session & Context

| Feature | Details |
| --- | --- |
| **Auto-save & Resume** | `--continue` resumes last session; `/resume <id>` restores specific sessions |
| **Token budgeting** | Gauge, auto-compaction at 80% threshold, forced `/compact` |
| **Subagent auto-compaction** | History elided before oversized requests |
| **Context compression** | Byte-budget caps on tool results, session recap on resume |

### Tools

**Filesystem:** `Bash`, `Read`, `Write`, `Edit`, `MultiEdit`, `Glob`, `Grep`

**Code:** `GraphQuery`, `GraphContext`, `SymbolEdit`, `CodeIndex`, `RunCoverage`

**Tasks:** `Task`, `TaskCreate`, `TaskList`, `TaskDone`, `TaskGet`

**Agents:** `SendMessage`, `TeamCreate`, `TeamDelete`, `TeamMemberMode`

**Market:** `PostBounty`, `RunBounty`, `MarketStatus`

**Memory:** `MemoryCreate`, `MemoryDelete`

**Web:** `WebFetch`, `WebSearch`

**Daemon:** `CronCreate`, `CronList`, `CronDelete`, `ScheduleWakeup`

**Advanced:** `LSP`, `Notebook`, `Monitor`, `Workflow`, `Advisor`, `MCP`

### Approval Modes

Shift+Tab cycles: `plan` → `default` → `acceptEdits` → `auto` → `bypass`

- `plan`: Asks permission for every tool call
- `auto`: ML classifier auto-approves safe calls (when `/auto-mode on`)
- `bypass`: Silent execution (for daemons/background tasks)

### Code Graph DSL

9 query operators (pipe-separated):

```text
fn("name")              # Select functions by substring
type("name")            # Select types (struct/enum/trait)
callers                 # Walk incoming Calls edges
callees                 # Walk outgoing Calls edges
depth N                 # Expand N hops outward
filter kind=Function    # Retain only a specific node kind
taint "var"             # Forward data-flow proxy over calls
preconditions           # Backward control-flow analysis
show signature|body     # Control output projection
```

**Examples:**
```text
fn("execute_tool") | callees | depth 2
type("Config") | callers | filter kind=Function
fn("parse") | taint "input" | depth 5 | show body
```

**Languages:** Rust, TypeScript/JavaScript, Python, Go, Java, Kotlin, C, C++, C#, PHP, Ruby, Swift

### Bounty Marketplace

| Phase | Description |
| --- | --- |
| **Post** | Register task with token budget and acceptance criteria |
| **Solve** | Spawn 1-5 competing solver agents; each produces a patch |
| **Validate** | Validator agents inspect solutions and propose flaws/tests |
| **Settle** | Rank solutions, pay winners, update trust, record ledger |
| **Apply** | Winning patch written to disk; audit artifacts under `.jfc/bounties/<id>/` |

### Swarms

Spawn named teammates with permissions:
- Shared task list (`TaskStore`)
- File-backed mailboxes for async messaging
- Plan-mode approval gating (leader resolves with `/swarm-approve` or `/swarm-deny`)
- Shared `.jfc/memory/` for persistent team context

### Daemon / Background Jobs

```bash
jfc daemon start              # Run daemon in foreground
jfc daemon stop               # Stop via PID file
jfc daemon status             # Health & counts
jfc daemon attach <id>        # Follow detached agent logs
jfc daemon wait <id>          # Block until task completes
jfc daemon fire <id>          # Manually trigger a cron job
```

---

## Slash Commands

| Command | Description |
| --- | --- |
| `/help` | Show commands and keybindings |
| `/continue [all]` / `/c` | Continue most recent session |
| `/resume <id>` | Resume a saved session |
| `/sessions` | List saved sessions |
| `/clear` | Start fresh session |
| `/compact` | Manual context compaction |
| `/goal <condition>` | Autonomous loop until condition met |
| `/workflow` / `/wf` | Multi-agent orchestration helper |
| `/undo` | Undo recent edit |
| `/tasks` | Show task list |
| `/market` | Show bounty market status |
| `/graph-history` | Show recent graph queries |
| `/mode <name>` | Show/switch permission mode |
| `/auto-mode on\|off` | Toggle ML auto-approval |
| `/memory` / `/mem` | List/manage memories |
| `/skills` | List loaded skill files |
| `/agents` | List loaded agent definitions |
| `/mcp [list\|restart]` | Inspect/restart MCP servers |
| `/theme [name]` | Pick or persist theme |
| `/cost` / `/stats` | Show usage & cost stats |
| `/login [provider]` | Provider login chooser |

**More:** `/check`, `/config`, `/diff`, `/export`, `/timeline`, `/doctor`, `/effort`, `/feature`, `/batch`, `/init`, `/bug`, `/pr <num>`, `/swarm-approve`

---

## Keybindings

| Key | Action |
| --- | --- |
| `Enter` | Send message |
| `Shift+Enter` | Newline in input |
| `Shift+Tab` | Cycle permission mode |
| `Ctrl+P` | Command palette |
| `Ctrl+B` | Toggle sessions sidebar |
| `Ctrl+M` | Open model picker |
| `Ctrl+O` | Expand reasoning panel |
| `Ctrl+Y` | Copy last assistant message |
| `Ctrl+C` | Cancel/exit |
| `Esc` | Dismiss popup |
| `@` | Autocomplete file paths |
| `↑/↓` | Scroll or recall history |

---

## Configuration

**Config path:** `~/.config/jfc/config.toml`

```toml
[model]
default = "claude-sonnet-4-6-20250514"

[compact]
auto_pct = 80

[permissions]
mode = "default"  # plan | default | acceptEdits | auto | bypass

[daemon]
max_sessions = 5
cleanup_after_hours = 24

[agents.Explore]
model = "litellm/qwen/qwen3.6-35b-a3b:coding"

[agents.Plan]
model = "claude-opus-4-7"
```

**Project config** (`.jfc/features.toml`):
```toml
[permissions]
enabled = true

[background]
max_concurrent = 10
```

**Built-in subagents:** `Explore` (read-only, haiku), `Plan` (architecture), `verification` (testing), `general-purpose` (multi-step), `orchestrator` (decomposition)

**Environment variables:**

| Var | Effect |
| --- | --- |
| `JFC_LITELLM_API_KEY` | LiteLLM API key |
| `JFC_LITELLM_API` | LiteLLM base URL |
| `JFC_DISABLE_BELL=1` | Silence completion bell |
| `JFC_DISABLE_AUTO_COMPACT=1` | Disable auto-compaction |
| `JFC_ADVISOR_ENABLED=1` | Enable `/advisor` |

---

## Skills & Agents

**Skills** (`.claude/skills/*.md`) — Markdown with optional YAML frontmatter:

```markdown
---
name: my-skill
description: Domain-specific instructions
---
Instructions for this skill.
```

**Agents** (`.claude/agents/*.md`) — Markdown with YAML frontmatter:

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

**Built-in skills:** `do-178b`, `vuln-researcher`, `git-master`, `rust-style`, `tracing`, `snafu`, `thiserror`, `ripgrep`

---

## MCP (Model Context Protocol)

- Stdio and SSE transports
- Tool registry with schema validation
- Dynamic dispatch to handlers
- Server lifecycle: `/mcp list`, `/mcp restart`, `/mcp logs`
- Discover and call remote tools as `mcp__<server>__<tool>`

---

## Inter-Agent Communication

| Mechanism | Description |
| --- | --- |
| **Scratchpad** | File-backed key-value store for sibling agents |
| **Mailbox** | Per-teammate async message queues |
| **Task Store** | Shared task list; teammates claim/update/complete |
| **Team Memory** | Shared `.jfc/memory/` visible to all team members |

---

## Performance

- Markdown rendering cached by content hash + viewport width
- Virtual scrolling with cached tool heights
- Parallel read-only tool dispatch
- Detached workers don't block TUI event loop
- Graph sessions memoize queries; invalidate after edits
- Subagent auto-compaction before oversized requests
- LRU cache for web results
- Incremental graph re-indexing via file watcher
- CSR (compressed sparse row) for large graph traversal

---

## Development

```bash
cargo fmt --all --check
cargo check --workspace
cargo test -p jfc app::tests::
cargo test -p jfc-graph
cargo test -p jfc-economy
cargo clippy --workspace
```

Each crate has its own `Cargo.toml` and tests. Use `.claude/skills/`, `.claude/agents/`, and `.claude/workflows/` for project-specific instructions.

---

## License

[AGPL-3.0](LICENSE)

# jfc

A high-performance AI coding agent for the terminal. Multi-agent orchestration, code graph intelligence, and a competitive bounty marketplace — built in Rust with [ratatui](https://github.com/ratatui/ratatui).

![Rust](https://img.shields.io/badge/rust-nightly-orange)
![License](https://img.shields.io/badge/license-AGPL--3.0-blue)

<p align="center">
  <img src="resources/image.png" alt="jfc" width="600"/>
</p>

## Highlights

- 🤖 **Full agentic loop** — tools, approval pipelines, parallel dispatch, auto-mode
- 🐝 **Multi-agent swarm** — spawn named teammates, shared memory, session mirrors, fork subagents
- 🧠 **Code graph intelligence** — caller/callee traversal, taint analysis, symbol editing with cascade
- 💰 **Bounty marketplace** — competitive solver/validator economy with adversarial cross-validation
- 🏭 **Fleet daemon** — persistent headless agents with cron scheduling and a terminal dashboard
- 🔌 **MCP + Skills** — extensible via Model Context Protocol servers and declarative skill files
- 🔒 **31-point hook lifecycle** — permission gates, tool approval, file-change notifications, telemetry

---

## Architecture

```
jfc/
├── crates/
│   ├── jfc-ui/          # Main binary — TUI, tools, providers, swarm, daemon
│   ├── jfc-graph/       # Code graph DSL (fn/type/callers/callees/taint/preconditions)
│   └── jfc-economy/     # Bounty auction, solver/validator orchestration, trust scoring
├── .claude/skills/      # Declarative skill files (do-178b, vuln-researcher, etc)
└── .jfc/memory/         # Persistent project memories
```

## Features

### Core Agent

| Feature | Description |
|---------|-------------|
| **Multi-provider** | Anthropic (API + OAuth), OpenAI, OpenWebUI/LiteLLM — hot-swappable |
| **Tools** | Bash, Read, Write, Edit, Glob, Grep — with parallel dispatch |
| **Session persistence** | Auto-save, `--continue`, session picker sidebar |
| **Auto-compaction** | Token tracking, auto-compact at threshold, configurable window |
| **Context pressure** | Live token gauge, circuit breaker for runaway loops |
| **Reasoning effort** | `/effort low|medium|high` — trade speed for depth per-turn |
| **Git context** | Auto-detects repo, branch, remote — injected into system prompt |
| **Markdown rendering** | Syntax-highlighted (250+ langs), cached, virtual-scrolled |
| **LSP diagnostics** | Real-time error/warning surfacing from language servers |

### Swarm / Multi-Agent

| Feature | Description |
|---------|-------------|
| **Teams** | Named groups with leader + teammates, shared task list |
| **Mailbox messaging** | File-based delivery with JSON inboxes, idle notifications |
| **Team memory** | Shared filesystem directory with change notifications |
| **Session mirrors** | Observe teammate work in real-time (tool calls, responses) |
| **Fork subagent** | Clone conversation to a new agent for parallel exploration |
| **Teleport** | Jump between sessions by switching git branch + resume |
| **Turn classifier** | Structured status (running/blocked/idle/review_ready) for dashboards |
| **Permission sync** | Workers forward permission prompts to team leader |
| **Worktree isolation** | Each agent gets its own git worktree + tmux session |

### Fleet / Daemon

| Feature | Description |
|---------|-------------|
| **Daemon mode** | Persistent headless process managing multiple sessions |
| **Cron scheduling** | Interval, hourly, daily, weekly — periodic agent tasks |
| **Fleet dashboard** | ratatui TUI with agent grid, status, attach/stop controls |
| **Unix socket API** | Command/response protocol for IDE integration |
| **Session lifecycle** | Pending → Running → Idle → Completed/Failed/Cancelled |

### Code Graph (jfc-graph)

| Feature | Description |
|---------|-------------|
| **DSL queries** | `fn("name") \| callers \| depth 3`, `type("Config") \| callees` |
| **Taint analysis** | `fn("parse") \| taint "input" \| depth 5` |
| **Preconditions** | Walk callers backward to find enclosing if/match predicates |
| **Symbol edit** | Edit by handle, auto-validates callers, cascade planning |
| **Multi-language** | Rust, TypeScript, Python, Go, C (via tree-sitter) |

### Bounty Market (jfc-economy)

| Feature | Description |
|---------|-------------|
| **Post bounties** | Register coding tasks with acceptance criteria |
| **Competing solvers** | 1-5 solver agents work in parallel worktrees |
| **Adversarial validation** | Validators challenge solutions in sealed sessions |
| **Trust scoring** | Agents build reputation; collusion/rubber-stamping detected |
| **Settlement** | Only solutions surviving validation get ranked + paid |

### Hooks (31 lifecycle points)

```
Tool:    BeforeToolDispatch, AfterToolDispatch, BeforeToolBatch, AfterToolBatch, OnToolError, OnToolApproval
Stream:  BeforeStream, AfterStream, OnModelResponse
Session: OnSessionStart, OnSessionEnd, BeforeCompact, AfterCompact, OnHeartbeat
Perms:   OnPermissionRequest, OnPermissionGranted, OnPermissionDenied
Files:   OnFileChanged, OnCwdChanged
Agents:  OnAgentSpawned, OnAgentTerminated, OnTeammateIdle, OnMessageSent, OnMessageReceived
Config:  OnConfigChanged, OnInstructionsLoaded, OnUserPromptSubmit
Memory:  OnMemoryCreated, OnMemoryDeleted
Tasks:   OnTaskCreated, OnTaskCompleted
```

---

## Installation

Requires Rust nightly (edition 2024):

```bash
git clone https://github.com/coleleavitt/jfc.git
cd jfc
cargo install --path crates/jfc-ui
```

## Usage

```bash
# Interactive session
jfc

# Resume last session
jfc --continue

# Start with a prompt
jfc -p "explain this codebase"

# Specific model
jfc --model claude-opus-4-6-20250620

# Daemon mode
jfc daemon start
jfc daemon run "review all open PRs"
jfc daemon status --live    # Fleet dashboard
jfc daemon cron add --every 6h "run test suite and report failures"
```

## Slash Commands

| Command | Description |
|---------|-------------|
| `/help` | Show all commands |
| `/compact` | Force context compaction |
| `/clear` | Reset conversation |
| `/model [name]` | Show/switch model |
| `/effort [level]` | Set reasoning effort (low/medium/high) |
| `/stats` | Token usage, cost, turns |
| `/resume [id]` | Resume a saved session |
| `/sessions` | List all sessions |
| `/branch [name]` | Show/create git branch |
| `/permissions` | View/change mode (plan/default/auto) |
| `/memory` | List project memories |
| `/hooks` | Show registered lifecycle hooks |
| `/worktree [cmd]` | Worktree management |
| `/daemon [cmd]` | Fleet daemon control |
| `/exit` | Exit session |

## Keybindings

| Key | Action |
|-----|--------|
| `Enter` | Send message |
| `Shift+Enter` | Newline in input |
| `Ctrl+P` | Command palette |
| `Ctrl+B` | Toggle sessions sidebar |
| `Ctrl+S` | Toggle info sidebar (tasks, usage, LSP) |
| `Ctrl+M` | Open model picker |
| `Ctrl+O` | Expand/collapse reasoning |
| `Ctrl+Y` | Yank last message to clipboard |
| `Ctrl+C` | Cancel streaming / exit |
| `Esc` | Dismiss popup / close panel |
| `@` | Autocomplete file paths |
| `↑/↓` | Scroll / recall history |

## Configuration

`~/.config/jfc/config.toml`:

```toml
[model]
default = "claude-sonnet-4-6-20250514"

[compact]
# Auto-compact threshold (% of context window)
auto_pct = 80

[permissions]
# Default mode: "plan" | "default" | "auto"
mode = "default"

[hooks]
# Shell command to run on file changes
on_file_changed = "echo $JFC_FILE_PATH >> /tmp/jfc-changes.log"

[daemon]
# Max concurrent sessions
max_sessions = 5
# Session cleanup after (hours)
cleanup_after_hours = 24
```

## Skills

Skills are Markdown files in `.claude/skills/` with YAML frontmatter:

```markdown
---
name: my-skill
---
Instructions for the agent when this skill is invoked...
```

Built-in skills:
- `do-178b` — RTCA DO-178B aviation safety certification
- `vuln-researcher` — Vulnerability research, CVSS scoring, PoC generation
- `git-master` — Advanced git workflows and recovery
- `rust-style` — Idiomatic Rust patterns
- `tracing` — Rust tracing/subscriber ecosystem
- `snafu` / `thiserror` — Error handling crates
- `ripgrep` — Power-user rg guide

## Performance

- **Render cache** — Markdown → lines cached per `(hash, width)`, skips re-parsing unchanged messages
- **Virtual scroll** — Only viewport messages get full rendering
- **Parallel tool dispatch** — Read-only tools (Glob, Grep, Read) execute concurrently
- **80ms tick** — Balanced responsiveness vs CPU
- **Graph build** — Dedicated 64MB stack thread, ignore-crate for fast directory walking

## License

[AGPL-3.0](LICENSE)

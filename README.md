# jfc

A high-performance AI coding agent for the terminal. It combines a Rust ratatui UI, durable background workers, persistent swarms, code-graph intelligence, provider OAuth, MCP tools, and a competitive solver/validator bounty market.

![Rust](https://img.shields.io/badge/rust-nightly-orange)
![License](https://img.shields.io/badge/license-AGPL--3.0-blue)

<p align="center">
  <img src="resources/image.png" alt="jfc" width="600"/>
</p>

## Highlights

- **Full agentic loop** — streaming responses, tool calls, approval modes, task tracking, auto-compaction, and cancellation.
- **Foreground + detached background agents** — `Task` can run inline or fork a durable `jfc daemon worker` process that survives the TUI.
- **Persistent swarms** — spawn named teammates, send mailbox messages, share task lists/memory, and gate teammate tools through leader approval.
- **Code graph intelligence** — tree-sitter graph queries, callers/callees, path search, taint/preconditions, coverage metadata, and semantic `symbol_edit` cascades.
- **Bounty marketplace** — post coding bounties, run competing solver agents, adversarial validators, settlement, trust, and token ledger accounting.
- **Multi-provider auth** — Anthropic API/OAuth, OpenAI, Codex/ChatGPT OAuth, OpenWebUI/LiteLLM, Bedrock, and Vertex provider foundations.
- **MCP + Skills + tools** — local tools, remote MCP tools, skill files, memories, web fetch/search, notebooks, cron, wakeups, LSP, notifications, and webhooks.

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
│   ├── jfc-web/            # Web search (Google CSE, arXiv, Semantic Scholar)
│   ├── jfc-markdown/       # Markdown rendering utilities, fence detection
│   └── jfc-theme/          # Terminal themes, palette validation, ANSI color management
├── .claude/skills/         # Declarative skill files
├── .claude/agents/         # Optional project agent definitions
├── .claude/workflows/      # Workflow scripts (multi-agent orchestration)
└── .jfc/memory/            # Persistent project memories
```

The central runtime shape is `AppEvent` → `BackgroundTask`: foreground subagents, detached workers, swarm teammates, and bounty solver/validator agents all stream progress into the same fan/task UI model.

Key internal subsystems in `jfc-ui`:

| Module | Role |
| --- | --- |
| `autonomous_loop` | Goal-directed autonomous agent loop with tick preamble |
| `speculation` | Prompt prediction during idle time |
| `slop_guard` | Quality gate: duplication, dead code, churn, coherence checks |
| `sandbox` | bubblewrap + Landlock kernel sandboxing for Bash |
| `file_checkpoint` | Pre-edit snapshots for `/undo` support |
| `sprint` | Token budget tracking with pressure/handoff signals |
| `coach` | Session health tips from usage statistics |
| `session_recap` | Summaries for resumed sessions |
| `hooks` | Pre/post tool hook execution |
| `intent` | Intent classification + graph-context injection |
| `inline_tools` | XML-based inline tool call parsing for non-native providers |
| `dreamer_scheduler` | Periodic background learning via jfc-learn dreamers |
| `bridge_attestation` | Request body integrity verification for OAuth flows |
| `idle_prefetch` | Background pre-warming during user idle |
| `web_cache` | LRU cache for web fetch/search results |

## Feature Map

### Core Agent + TUI

| Feature | Description |
| --- | --- |
| **Multi-provider** | Anthropic API/OAuth, OpenAI, Codex OAuth, OpenWebUI/LiteLLM, Bedrock, Vertex. |
| **Streaming tool loop** | Models emit tool calls, jfc executes tools, returns tool results, and continues until completion. |
| **Approval modes** | `plan`, `default`, `acceptEdits`, `auto`, and `bypass` modes; Shift+Tab cycles in the TUI. |
| **Auto-mode classifier** | ML-based classifier that auto-approves safe tool calls when `/auto-mode on` is active. |
| **Tools** | Bash, Read, Write, Edit, MultiEdit, Glob, Grep, Task, memory, teams, graph, market, web, MCP, cron, LSP, notebooks, notifications. |
| **Session persistence** | Auto-save, `--continue`, `/continue`, `/resume`, session picker/sidebar, cwd mismatch warnings. |
| **Context management** | Token gauge, auto-compaction, forced `/compact`, subagent history compaction, byte-budget tool-result caps. |
| **Diagnostics** | Cargo diagnostics and LSP hover/definition/references surfaced in the UI. |
| **Rendering** | Markdown rendering, syntax highlighting, virtual scroll, cached tool/message heights, task fan/sidebar. |
| **Advisor** | Optional `/advisor <question>` runs a parallel advisor call against a transcript snapshot. |
| **Sandbox** | bubblewrap (bwrap) + Landlock LSM kernel sandboxing for Bash tool execution. |
| **Slop guard** | Post-response quality checks: duplication, dead code, churn, coherence, complexity, and test quality. |
| **File checkpoints** | Automatic pre-edit snapshots with `/undo` restore and configurable pruning. |
| **Sprint budgets** | Token budget tracking with pressure warnings and handoff thresholds for long sessions. |
| **Speculation** | Idle-time prompt prediction — prefetches likely next requests during user think-time. |
| **Coaching** | Session health analysis with actionable tips based on tool-usage patterns. |
| **Session recap** | Auto-generated summaries when resuming sessions to restore context quickly. |
| **Goal loop** | `/goal <condition>` sets an autonomous objective; the agent loops until the condition is met. |
| **Hooks** | Pre/post tool hooks with comment-slop detection and custom validation. |

### Agents, Background Workers, and Swarms

| Mode | What it does |
| --- | --- |
| **Foreground `Task`** | Runs an in-process one-shot subagent, streams `AgentChunk` and `TaskProgress` live into the fan/task panel. |
| **Detached background `Task`** | `run_in_background=true` writes a launch spec and forks `jfc daemon worker --launch <json>`; logs/state hydrate back into the UI. |
| **Worktree-isolated `Task`** | `isolation="worktree"` creates `.jfc-worktrees/<name>` and runs tools from that checkout. |
| **Teammate spawn** | `Task` with `name` + `team_name` creates a persistent teammate addressable with `SendMessage`. |
| **Swarm task claiming** | Teammates can claim unowned team tasks from the shared `TaskStore`. |
| **Permission sync** | Plan-mode teammates write permission requests; leader resolves with `/swarm-approve` or `/swarm-deny`. |
| **Mailbox** | File-backed inboxes deliver teammate messages, idle notifications, and shutdown requests. |

Example model-callable `Task` shapes:

```json
{
  "description": "Explore graph query internals",
  "prompt": "Trace graph_query from tool dispatch into jfc-graph.",
  "subagent_type": "Explore",
  "run_in_background": false
}
```

```json
{
  "description": "Run long verification",
  "prompt": "Run the relevant tests and summarize failures.",
  "run_in_background": true,
  "isolation": "worktree"
}
```

```json
{
  "description": "Spawn backend reviewer",
  "prompt": "Watch the task list and review backend tasks.",
  "name": "backend-reviewer",
  "team_name": "review-swarm",
  "mode": "plan"
}
```

### Daemon / Durable Jobs

| Command | Description |
| --- | --- |
| `jfc daemon start` | Run the daemon loop in the foreground. |
| `jfc daemon stop` | Stop the running daemon via PID file. |
| `jfc daemon status` | Show daemon health and counts. |
| `jfc daemon list` | List cron jobs and wakeups. |
| `jfc daemon fire <id>` | Manually fire a cron job once. |
| `jfc daemon agents` | List persistent background-agent roster. |
| `jfc daemon logs <id> --lines N` | Print recent log lines for a detached agent. |
| `jfc daemon attach <id>` | Follow a detached agent log until terminal state. |
| `jfc daemon wait <id> --timeout-secs N` | Wait for a detached agent to complete/fail/cancel. |
| `jfc daemon kill <id>` | Request cancellation for a detached agent. |

Model-callable daemon tools include `CronCreate`, `CronList`, `CronDelete`, and `ScheduleWakeup`.

### Workflows

Workflows are multi-agent orchestration scripts that run deterministic pipelines of subagents.

- Scripts live in `.claude/workflows/` or `~/.config/jfc/workflows/`
- Each script exports `meta = { name, description, phases }` and uses `agent()`, `parallel()`, `pipeline()`, `phase()` primitives
- Resumable: `resumeFromRunId` replays cached agent results and continues from where it left off
- Progress streams into the TUI task panel with per-phase status
- Permission-gated: workflows can be saved/loaded with approval tokens

Trigger with the `Workflow` tool, `/workflow` slash command, or the `ultrawork` keyword in prompts.

### Learning Subsystem (`jfc-learn`)

The learning crate provides persistent knowledge extraction from sessions:

| Component | Role |
| --- | --- |
| **Historian** | Extracts facts from conversation history with confidence scoring and deduplication |
| **Dreamer** | Background process that verifies/refutes stored memories against code reality |
| **Key Files** | Tracks which files are important based on read frequency; surfaces them in context |
| **Auto Hints** | Generates project-specific hints from observed patterns |
| **Verifier** | Validates memory promotion/demotion through contract checks |
| **User Memory** | Pipeline for promoting session observations to persistent user-level memories |

The dreamer scheduler in `jfc-ui` periodically runs dreamers during idle time.

### Security Audit (`jfc-audit`)

Automated vulnerability research tooling:

| Component | Role |
| --- | --- |
| **Enumerator** | Discovers attack surface entry points |
| **Taint** | Traces untrusted data flow through call chains |
| **Reachability** | Determines if vulnerable code is reachable from entry points |
| **Store** | Persists findings with suppression support |
| **Orchestrator** | Coordinates enumeration → taint → reachability pipeline |
| **Dispatcher** | Routes findings to appropriate handlers |

### Code Graph (`jfc-graph`)

The graph subsystem builds a symbol/call/type graph from the workspace and exposes it through the `graph_query` tool.

| Capability | Example |
| --- | --- |
| Entrypoints | `entrypoints`, `entrypoints kind=PublicApi` |
| Function/type search | `fn("execute_tool")`, `type("Config")` |
| Traversal | `fn("execute_task") \| callees \| depth 3` |
| Callers | `fn("record_background_agent_progress") \| callers` |
| Set algebra | `fn("spawn") union fn("worker")`, `A intersect B`, `A \ B` |
| Paths | `path fn("stream_response") -> fn("execute_tool")` |
| Taint | `fn("parse") \| taint "input" \| depth 5` |
| Preconditions | `fn("dangerous_op") \| callers \| preconditions` |
| Coverage | `run_coverage`, then `entrypoints kind=PublicApi \| untested` |
| Possible types | `fn("handler") \| possible_types` |
| Symbol editing | Use `--- handles ---` from `graph_query`, then `symbol_edit(handle, new_content, validate=true)`. |
| Cascade planning | `symbol_edit(..., validate=true, dispatch_cascade=true)` queues per-file cascade tasks. |

The graph session memoizes query results and invalidates caches after edits. Query output includes structured handles so the model can chain precise follow-up queries or edits without grep.

Language adapters (tree-sitter based):

| Language | Adapter |
| --- | --- |
| Rust | Full: functions, structs, enums, traits, impls, modules, closures |
| TypeScript/JavaScript | Functions, classes, interfaces, exports, JSX |
| Python | Functions, classes, decorators, imports |
| Go | Functions, structs, interfaces, methods |
| Java | Classes, methods, interfaces, annotations |
| Kotlin | Classes, functions, objects, companion objects |
| C | Functions, structs, unions, typedefs, macros |
| C++ | Classes, methods, templates, namespaces |
| C# | Classes, methods, interfaces, properties |
| PHP | Classes, functions, namespaces, traits |
| Ruby | Classes, modules, methods, blocks |
| Swift | Classes, structs, protocols, extensions |

Advanced analysis modules:

- **CFG** — control-flow graph construction with dominator trees
- **Dataflow** — forward/backward data-flow analysis with custom rules
- **Taint v2** — inter-procedural taint propagation
- **Communities** — module clustering via graph partitioning
- **Complexity** — cyclomatic and cognitive complexity metrics
- **Co-change** — files that change together (git history correlation)
- **CSR** — compressed sparse row for fast graph traversal at scale
- **Incremental** — file-watcher-driven incremental re-indexing
- **Persistence** — event log with undo support
- **Points-to** — pointer/reference analysis
- **Monomorphize** — generic instantiation tracking
- **Polyglot** — cross-language symbol resolution

### Bounty Market (`jfc-economy`)

| Feature | Description |
| --- | --- |
| **Post** | `post_bounty` registers a task with a token budget and acceptance criteria. |
| **Solve** | `run_bounty` spawns 1-5 solver agents; each produces a patch/FILE blocks. |
| **Validate** | Validator agents inspect surviving solutions in sealed sessions and propose flaws/tests. |
| **Settle** | The market ranks solutions, pays winners, updates trust, and records ledger usage. |
| **Apply** | Winning solution content is written to disk and audit artifacts land under `.jfc/bounties/<id>/`. |
| **Inspect** | `/market` or `market_status` shows bounty state, spend, health, trust, and collusion signals. |

### Providers and Authentication

| Provider | Notes |
| --- | --- |
| Anthropic API | Standard API-key provider. |
| Anthropic OAuth | Multi-account Claude.ai OAuth with account list/switch/disable/remove. Sensitive CCH/billing pieces are behind `anthropic-oauth-sensitive`. |
| OpenAI | OpenAI-compatible chat/responses provider path. |
| Codex OAuth | ChatGPT/Codex OAuth foundation with browser login, device flow, status, logout, and URL/header rewriting hooks. |
| OpenWebUI / LiteLLM | OpenAI-compatible local/proxy providers. LiteLLM dynamically fetches all models from the configured instance. |
| Bedrock / Vertex | Cloud-provider foundations and setup wizards. |

Useful auth commands:

```bash
jfc auth anthropic login [name]
jfc auth anthropic list
jfc auth anthropic switch <name>
jfc auth codex login
jfc auth codex device
jfc auth codex status
jfc auth codex logout
jfc auth litellm login --url <URL> --key <KEY>
jfc auth litellm status
jfc auth litellm logout
```

Inside the TUI, `/login` shows provider-specific login options.

### Tools

Core filesystem/shell tools:

- `Bash`, `Read`, `Write`, `Edit`, `MultiEdit`, `Glob`, `Grep`
- `TaskCreate`, `TaskUpdate`, `TaskList`, `TaskDone`, `TaskGet`, `TaskValidate`
- `Task`, `Skill`, `ToolSearch`, `ToolSuggest`
- `MemoryCreate`, `MemoryDelete`
- `TeamCreate`, `TeamDelete`, `SendMessage`, `TeamMemberMode`
- `GraphQuery`, `GraphContext`, `GraphSearch`, `GraphCallers`, `GraphCallees`, `GraphImpact`, `GraphNode`, `GraphExplore`, `CodeIndex`, `RunCoverage`, `SymbolEdit`
- `PostBounty`, `RunBounty`, `MarketStatus`
- `AskUserQuestion`, `EnterPlanMode`, `ExitPlanMode`
- `WebFetch`, `WebSearch`
- `CronCreate`, `CronList`, `CronDelete`, `ScheduleWakeup`
- `Monitor`, `LSP`, `PushNotification`, `RemoteTrigger`
- `EnterWorktree`, `ExitWorktree`
- `NotebookRead`, `NotebookEdit`
- `ScratchpadRead`, `ScratchpadWrite` (inter-agent shared state)
- `Workflow` (multi-agent orchestration scripts)
- `Advisor` (parallel reviewer model consultation)
- `SendUserMessage`, `SendUserFile` (proactive user communication)
- MCP-advertised `mcp__server__tool` calls

---

## Installation

Requires Rust nightly / edition 2024.

```bash
git clone https://github.com/coleleavitt/jfc.git
cd jfc
cargo install --path crates/jfc-ui
```

For a local dev binary:

```bash
cargo build -p jfc-ui --bin jfc
./target/debug/jfc
```

Public build without sensitive Anthropic OAuth feature:

```bash
cargo build -p jfc-ui --bin jfc --no-default-features --features hooks,permission-automation
```

## Usage

```bash
# Interactive TUI
jfc

# Resume most recent session
jfc --continue

# Resume specific session
jfc --resume <session-id>

# Print mode / one-shot prompt
jfc -p "explain this codebase"

# Specific model
jfc --model claude-sonnet-4-6-20250514

# Remote managed session bridge
jfc --remote-session <session-id>

# Daemon / detached agent utilities
jfc daemon start
jfc daemon status
jfc daemon agents
jfc daemon attach <task-id>
```

## Slash Commands

| Command | Description |
| --- | --- |
| `/help` | Show command/key help. |
| `/clear` | Clear conversation and start a fresh session/task store. |
| `/compact` | Queue manual context compaction. |
| `/advisor <question>` | Ask a parallel advisor call, if `JFC_ADVISOR_ENABLED=1`. |
| `/check` | Re-run cargo-check diagnostics reminder/refresh path. |
| `/config [path]` | Show parsed config or config file path. |
| `/continue [all]` / `/c` | Continue most recent session for cwd or globally. |
| `/resume <id> [--force]` | Resume a saved session. |
| `/sessions` | List saved sessions. |
| `/rename <title>` | Rename the active session. |
| `/workflow` / `/wf` | Workflow helper. |
| `/login [provider]` | Provider login chooser/helpers. |
| `/batch` | Batch prompt helper. |
| `/diff` | Show current git diff. |
| `/undo` | Undo recent tool edit where supported. |
| `/export` | Export transcript. |
| `/verbose` | Toggle verbose rendering. |
| `/pin` / `/unpin` | Pin or unpin transcript messages. |
| `/timeline` | Show session/tool timeline. |
| `/doctor` | Diagnose local config/provider environment. |
| `/effort <low|medium|high>` | Set reasoning effort. |
| `/feature` | Feature flag helper. |
| `/goal <condition|clear>` | Keep working toward a goal condition. |
| `/memory` / `/mem` | List memories or manage memory recall. |
| `/skills` | List loaded skill files. |
| `/agents` | List loaded agent definitions. |
| `/market` | Show bounty-market status. |
| `/cascade` | Show cascade tasks queued by `symbol_edit`. |
| `/graph-history` | Show recent graph queries and result counts. |
| `/tasks` | Show task list. |
| `/task-add <subject>` | Create a task. |
| `/task-done <id>` | Mark task complete. |
| `/task-rm <id>` | Delete a task. |
| `/claude-md` | Show loaded CLAUDE.md instruction layers. |
| `/mode <name>` | Show/switch permission mode. |
| `/auto-mode on|off|status` | Toggle classifier-based auto approval. |
| `/worktree ...` | Manage `.jfc-worktrees/<name>` branches. |
| `/mcp [list|restart|logs]` | Inspect/restart MCP servers. |
| `/theme [name]` | Pick or persist a theme. |
| `/fleet` | Open fleet/daemon dashboard. |
| `/teleport ...` | Session/branch teleport helper. |
| `/init` | Initialize project config/instructions. |
| `/cost` / `/stats` | Show usage/cost stats. |
| `/bug` | Bug-report helper. |
| `/rewind` | Rewind transcript state. |
| `/output-style` / `/style` / `/brief` | Change response style. |
| `/dump-context` | Debug the model context. |
| `/install-github-app` | Install GitHub app for the repo. |
| `/pr <num>` | Show PR metadata and review comments. |
| `/pr-autofix <num>` | Build/send a PR autofix prompt. |
| `/setup-github-actions [force]` | Write the jfc review workflow. |
| `/swarm-approve <id>` / `/swarm-deny <id>` | Resolve teammate permission requests. |

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
| `Alt+.` / `Alt+,` | Raise/lower reasoning effort. |
| `Ctrl+Y` | Yank last assistant message to clipboard. |
| `Ctrl+C` | Cancel streaming / exit. |
| `Esc` | Dismiss popup / close panel. |
| `@` | Autocomplete file paths. |
| `↑/↓` | Scroll or recall input history depending on focus/input state. |

## Configuration

Primary config path:

```text
~/.config/jfc/config.toml
```

Example:

```toml
[model]
default = "claude-sonnet-4-6-20250514"

[compact]
auto_pct = 80

[permissions]
mode = "default" # plan | default | accept | auto | bypass

[daemon]
max_sessions = 5
cleanup_after_hours = 24

# Per-subagent model overrides (optional)
[agents.Explore]
model = "litellm/qwen/qwen3.6-35b-a3b:coding"

[agents.general-purpose]
model = "litellm/deepseek-r1"

[agents.Plan]
model = "claude-opus-4-7"

[agents.verification]
model = "litellm/qwen/qwen3.6-35b-a3b:coding"

[agents.orchestrator]
model = "claude-sonnet-4-6"
```

Project feature config can live at `.jfc/features.toml`:

```toml
[permissions]
enabled = true

[background]
max_concurrent = 10
```

Agent configs support per-agent model/tool/permission overrides in config and `.claude/agents/*.md` frontmatter. Skills live in `.claude/skills/*.md`.

Built-in subagent types and their defaults:

| Subagent | Default Model | Role |
| --- | --- | --- |
| `Explore` | `haiku` | Read-only codebase search and grep |
| `general-purpose` | inherit (parent) | Multi-step tasks with full tool access |
| `Plan` | inherit (parent) | Architecture design, read-only planning |
| `verification` | inherit (parent) | Post-edit testing, tries to break things |
| `orchestrator` | inherit (parent) | Decomposes broad requests into subtask plans |

Override any subagent's model via `[agents.<name>].model` in config.toml. Resolution order (highest priority first):

1. `CLAUDE_CODE_SUBAGENT_MODEL` env var (global override for all subagents)
2. Per-call `model` field in the Task tool invocation
3. `config.toml [agents.<name>].model`
4. `.claude/agents/<name>.md` frontmatter `model:` field
5. Parent session model (inherit)

Useful environment knobs:

| Env var | Effect |
| --- | --- |
| `JFC_LITELLM_API_KEY` | LiteLLM proxy API key (alternative to `jfc auth litellm login`). |
| `JFC_LITELLM_API` | LiteLLM proxy base URL (e.g. `https://api.example.com/v1`). |
| `JFC_LITELLM_MODEL` | Default model to use from the LiteLLM instance. |
| `JFC_DISABLE_BELL=1` | Silence terminal bell on tool completion. |
| `JFC_DISABLE_AUTO_COMPACT=1` | Disable auto-compaction. |
| `JFC_DISABLE_CARGO_CHECK=1` | Skip startup cargo-check diagnostics. |
| `JFC_AUTOCOMPACT_PCT_OVERRIDE=N` | Override compaction threshold. |
| `JFC_TOOL_TITLE_WIDTH=N` | Cap rendered tool title length. |
| `JFC_ADVISOR_ENABLED=1` | Enable `/advisor`. |
| `JFC_WORKER_BIN=/abs/path/jfc` | Force detached worker executable path. |
| `JFC_GRAPH_CAP_*` | Toggle graph capabilities such as call graph, partial structs, validation. |

## Agent and Skill Files

Skills are Markdown files in `.claude/skills/` with optional YAML frontmatter:

```markdown
---
name: my-skill
description: Domain-specific instructions
---
Instructions for the agent when this skill is invoked.
```

Agents are Markdown files in `.claude/agents/` with YAML frontmatter:

```markdown
---
name: Explore
model: openai/gpt-5.1
permissionMode: default
allowedTools: [Read, Glob, Grep, graph_query]
disallowedTools: [Write, Edit]
skills: [ripgrep]
isolation: worktree
---
You explore codebases and report concise, cited findings.
```

Built-in skills include `do-178b`, `vuln-researcher`, `git-master`, `rust-style`, `tracing`, `snafu`, `thiserror`, and `ripgrep`.

### MCP (Model Context Protocol)

The `jfc-mcp` crate implements a full MCP server:

- Stdio and SSE transports
- Tool registry with schema validation
- Dynamic tool dispatch to registered handlers
- Server lifecycle management (`/mcp list`, `/mcp restart`, `/mcp logs`)
- Client-side: discovers and calls remote MCP tools as `mcp__<server>__<tool>`
- `WaitForMcpServers`, `ListMcpResources`, `ReadMcpResource` tools for server interaction

### Inter-Agent Communication

| Mechanism | Description |
| --- | --- |
| **Scratchpad** | File-backed key-value store for sharing data between sibling agents |
| **Mailbox** | Per-teammate message queues for async communication |
| **Task Store** | Shared task list that teammates can claim/update/complete |
| **Team Memory** | Shared `.jfc/memory/` files visible to all team members |

---

## Performance Notes

- Markdown/tool rendering is cached by content hash and viewport width.
- Message view uses virtual scrolling and cached tool heights.
- Read-only tools can be dispatched in parallel.
- Detached background workers avoid blocking the TUI event loop.
- Graph sessions memoize queries and invalidate after file edits.
- Subagents auto-compact or elide history before oversized requests.
- Web results cached in LRU to avoid redundant fetches.
- Idle prefetch pre-warms likely-needed resources during user think-time.
- CSR (compressed sparse row) representation for graph traversal at scale.
- Incremental graph re-indexing via file watcher — only re-parses changed files.

## Development

```bash
cargo fmt --all --check
cargo check -p jfc-ui
cargo test -p jfc-ui app::tests::
cargo test -p jfc-ui daemon::tests::
cargo test -p jfc-graph
cargo test -p jfc-economy
```

## License

[AGPL-3.0](LICENSE)

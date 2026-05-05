# jfc

A high-performance terminal UI for AI-assisted code exploration and development. Built in Rust with [ratatui](https://github.com/ratatui/ratatui).

![Rust](https://img.shields.io/badge/rust-nightly-orange)
![License](https://img.shields.io/badge/license-AGPL--3.0-blue)

<p align="center">
  <img src="resources/image.png" alt="jfc" width="600"/>
</p>

## Features

- **Multi-provider support** — Anthropic (API key + OAuth), OpenWebUI/LiteLLM proxy, with hot-swappable model picker
- **Full agentic loop** — Tool use (bash, file read/write/edit, grep, glob), approval pipelines, parallel tool dispatch
- **Session persistence** — Auto-saves conversations, resume with `--continue` or pick from the sidebar
- **Context management** — Live token gauge, auto-compaction when approaching limits, circuit breaker for runaway loops
- **Markdown rendering** — Syntax-highlighted code blocks (250+ languages via syntect), cached per-frame to eliminate re-parsing
- **Task tracking** — Built-in todo/task system that persists across sessions, visible in the sidebar
- **Multi-account OAuth rotation** — Health-scored account selection, file-level advisory locking, token refresh
- **Diagnostics** — LSP integration for real-time error/warning surfacing
- **Virtual scroll** — Render cache eliminates O(n) markdown re-parsing on every frame
- **Subagent support** — Background task dispatch with streaming output

## Installation

Requires Rust nightly (edition 2024):

```bash
git clone https://github.com/coleleavitt/jfc.git
cd jfc
cargo install --path crates/jfc-ui
```

## Usage

```bash
# Start a new session
jfc

# Resume the most recent session
jfc --continue

# Resume a specific session
jfc --resume ses_20260505_132743

# Start with a prompt
jfc --prompt "explain this codebase"

# Use a specific model
jfc --model claude-opus-4-6-20250620
```

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
| `Ctrl+Y` | Yank last assistant message to clipboard |
| `Ctrl+C` | Cancel streaming / exit |
| `Esc` | Dismiss popup / close panel |
| `@` | Autocomplete file paths |
| `↑/↓` | Scroll / recall history |
| `PgUp/PgDn` | Page scroll |

## Slash Commands

| Command | Description |
|---------|-------------|
| `/help` | Show help |
| `/model` | Open model picker |
| `/compact` | Force context compaction |
| `/clear` | Start a new session |
| `/continue` | Resume most recent session |
| `/tasks` | Show task list |
| `/config` | Show parsed configuration |

## Configuration

Optional config at `~/.config/jfc/config.toml`:

```toml
[model]
default = "claude-sonnet-4-6-20250514"

[compact]
# Override auto-compact threshold (percentage of context window)
# auto_pct = 90
```

## Performance

Key optimizations vs naive TUI approaches:

- **Render cache** — `markdown::to_lines()` output cached per `(text_hash, width)`. Unchanged messages skip pulldown-cmark + syntect entirely.
- **Height-only fast path** — `message_view_total_lines()` reads heights from cache without materializing the full render item vector.
- **Viewport-aware** — Only messages in/near the viewport get full rendering treatment.
- **80ms tick** — Balanced between responsiveness and CPU usage.

## License

[AGPL-3.0](LICENSE)

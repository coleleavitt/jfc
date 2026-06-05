# TUI Codex-Style Rewrite — Design

**Status:** Approved 2026-05-27
**Owner:** cole@unwrap.rs
**Scope:** `crates/jfc-ui`
**Estimated effort:** ~7 working days across 4 phases

## Motivation

The current `jfc-ui` TUI has two compounding problems:

1. **A failed inline-scrollback feature in the working tree** that used ratatui's stock `Terminal::insert_before` with a full-height `Viewport::Inline(rows)`. This routes into ratatui's "borrowed top line" branch (one-row-at-a-time scroll), has no Zellij fallback, no pre-wrap, no sanitization, and tracks flushed messages by index. Result: visible duplication on session load/compaction, character corruption from embedded `\r` and ANSI escapes, and the dock rendering on top of scrollback content.
2. **Slop in the composition.** `messages.rs` is 1621 lines, `frame.rs` is 362 lines, and the render pipeline includes sidebars, session pickers, model pickers, theme pickers, task panels, teammates panels, and an "agents" panel — all conflicting for screen real estate and all explicitly out of scope per the project's "minimal & honest, no sidebar, animate only on real activity" stance.

The fix is to **adopt the architecture patterns from `research/openai-codex/codex-rs/tui/`** (which solved these problems years ago) applied to `jfc-ui`'s data model, with a much smaller code footprint tailored to what `jfc-ui` actually needs.

Codex was rolled back in this session (working tree restored to commit `13285e7`); their non-UI work — `jfc-auth/workload_identity.rs`, `jfc-graph/adapter/*`, `jfc-providers/anthropic_oauth.rs`, `jfc-graph/dataflow.rs` — was preserved in the working tree. The pre-rollback bundle lives in `stash@{0}` for reference.

## Goals

- Native terminal scrollback works correctly — selecting/copying past messages via the terminal's own UI, no duplication, no character corruption.
- TUI viewport is a small inline strip (~15 rows) at the bottom; everything above is real scrollback the terminal owns.
- Live cell (the in-flight assistant turn) is mutable and lives in the viewport. Once the turn settles, it migrates to scrollback by event, not by polling an index counter.
- Dock is flat one-line — no multi-row status, no sidebars, no panels. Animations only fire during real activity.
- Works correctly in tmux, Zellij, VS Code terminal, kitty, alacritty.
- Each migration phase ships independently; the TUI remains usable after each merge.

## Non-Goals

- Not porting codex's 11k-line `chatwidget.rs` verbatim. Adopting the *patterns* (live cell vs history cells, bottom_pane composition), not the line count.
- Not building a session-picker / model-picker / theme-picker UI. Those become slash-command text outputs that appear as advisor messages.
- Not preserving the existing sidebar / task panel / teammates panel rendering — these are deleted in Phase 3.
- Not implementing voice input, IDE context, pager overlays, or any of codex's larger feature surfaces.
- Not supporting Windows in Phase 1–3 (Unix-first; Windows can be a follow-up).

## Reference

- Codex sources: `research/openai-codex/codex-rs/tui/src/`
- Key files we draw from:
  - `tui.rs` (855 lines) — `Tui` struct, init/restore, alt-screen toggle, draw entry, pending_history_lines flush
  - `custom_terminal.rs` (965 lines) — forked ratatui Terminal with proper inline-viewport contract
  - `insert_history.rs` (843 lines) — `Standard` (DECSTBM scroll-region + `ESC M` Reverse Index) and `Zellij` (raw newlines) insertion modes
  - `tui/event_stream.rs` — unified `TuiEvent::{Key, Paste, Resize, Draw, Focus}`
  - `tui/frame_requester.rs`, `tui/frame_rate_limiter.rs` — explicit `request_redraw()` + min frame interval
  - `tui/keyboard_modes.rs` — kitty enhancement probe + push/pop/reset
  - `terminal_probe` module — bounded `cursor_position` probe (1-2s timeout, default-to-origin on miss)
  - `wrapping.rs` — `adaptive_wrap_line`, `line_contains_url_like`, `line_has_mixed_url_and_non_url_tokens`

## Architecture

### Module layout

```
crates/jfc-ui/src/
├── tui/                              # NEW: terminal foundation, ~1500 LOC target
│   ├── mod.rs                        # Tui struct: new / init / draw / restore / enter_alt_screen
│   ├── terminal.rs                   # CustomTerminal — forked ratatui::Terminal
│   ├── insert_history.rs             # insert_history_lines_with_mode(Standard | Zellij)
│   ├── event_stream.rs               # TuiEvent enum + EventBroker + pause/resume
│   ├── frame_requester.rs            # FrameRequester::schedule_frame
│   ├── frame_rate_limiter.rs         # MIN_FRAME_INTERVAL = 33ms
│   ├── keyboard_modes.rs             # kitty probe / push / pop / reset
│   ├── terminal_probe.rs             # bounded cursor-position probe (Unix)
│   └── multiplexer.rs                # detect Zellij / tmux / VS Code / kitty
│
├── chat/                             # NEW: composition, ~2000 LOC target
│   ├── mod.rs                        # ChatWidget — top-level WidgetRef composer
│   ├── history.rs                    # Wrap finalized cells → pending_history_lines
│   ├── live_cell.rs                  # In-flight assistant turn rendering
│   ├── bottom_pane.rs                # input + dock + spinner row layout
│   ├── input.rs                      # Wrapper around ratatui-textarea
│   ├── dock.rs                       # Flat one-line status
│   ├── approval.rs                   # Permission prompt overlay
│   ├── toast.rs                      # Toast overlay (top-right of viewport)
│   ├── palette.rs                    # / slash-command popup
│   └── help.rs                       # ? help overlay
│
├── app/, types/, runtime/event_loop/, input/, session/, stream/,
│   sdk_bridge/, tools/, etc.         # KEPT — unchanged data model and runtime
└── ...
```

**Deleted in Phase 3:**

- `render/sidebar.rs`
- `render/session_sidebar.rs`
- `render/session_picker.rs`
- `render/model_picker.rs`
- `render/theme_picker.rs`
- `render/teammates_panel.rs`
- `render/task_panel.rs`
- `render/agents.rs`
- `render/visual.rs`
- `render/messages.rs` (replaced by `chat/history.rs` + `chat/live_cell.rs`)
- `render/frame.rs` (replaced by `chat/mod.rs`)
- `render/input_box.rs` (replaced by `chat/input.rs`)
- `render/status.rs` (replaced by `chat/dock.rs`)
- `render/overlays.rs` (split into `chat/toast.rs` + `chat/help.rs` + dropped pieces)

**Kept from `render/`:**

- `render/approval.rs` → moves to `chat/approval.rs`
- `render/palette.rs` → moves to `chat/palette.rs`

**Replaced:** `runtime/terminal.rs` becomes a thin re-export of `tui::Tui::draw`.

### Message lifecycle

The flushed-counter-by-index pattern in the failed attempt broke because indices drift on `messages.remove()` / `app.messages = compacted` / `messages.clear()`. The replacement is event-driven:

```rust
enum CellState {
    Live,        // in viewport, re-rendered each frame
    Finalizing,  // turn just ended; next draw will move to history
    History,     // already in scrollback — viewport never re-renders
}
```

State transitions:

| Trigger                                                                    | Effect                                                              |
| -------------------------------------------------------------------------- | ------------------------------------------------------------------- |
| User submits prompt                                                        | User message: created as `Finalizing` immediately                   |
| Assistant stream starts                                                    | Assistant placeholder: `Live`                                       |
| Stream emits text/tool/thinking chunks                                     | Live cell re-renders each affected frame                            |
| `stream_done` with `EndTurn`, no pending tools/approvals/classifiers       | Live cell → `Finalizing`                                            |
| Next `Tui::draw`                                                           | `Finalizing` cells: pre-wrap → push to `pending_history_lines` → `History` |
| `pending_history_lines` flushed via `insert_history_lines_with_mode`       | Lines appear in native scrollback above viewport; viewport excludes them |
| Compaction replaces `app.messages`                                         | New messages start in default state (`Finalizing` for user, etc.). Old scrollback is undisturbed (append-only). |

The renderer asks: "is this cell `Live`?" — if yes, draw into the viewport; if no, ignore (it's in scrollback already).

**No index counter. No drift. Compaction-safe.**

#### Live cell scrollback budget

A very long live cell (huge assistant text + many tool calls) would overflow the viewport. Resolution:

```rust
const LIVE_CELL_MAX_ROWS: u16 = 40;
```

When the live cell's wrapped row-count exceeds `LIVE_CELL_MAX_ROWS`, the OLDEST rows are pre-emptively pushed to scrollback via `insert_history_lines`. The live cell view becomes a tail of the most-recent `LIVE_CELL_MAX_ROWS` rows. The user's terminal scrollback contains the full content; the viewport contains the tail.

(Codex's `live_wrap.rs` does the same.)

### Composition

```
┌─Terminal──────────────────────────────────────┐
│                                                │
│  (native scrollback — finalized history)       │
│  (selectable / copyable via terminal UI)       │
│                                                │
│                                                │
├─Viewport (inline, ~15 rows)─────────────────── │
│  [Live cell rows — tail of in-flight turn    ] │
│  [  (streaming text, tool blocks, approval)  ] │
│  [Spinner row — only when streaming          ] │
│  [Dock: ⚡Auto · $0.087 · ⚒master · ^P · ?   ] │
│  [Input: > send a message…                   ] │
└────────────────────────────────────────────────┘
```

**Viewport height** = sum of:

- `live_cell.height()` (capped at `LIVE_CELL_MAX_ROWS`)
- 1 if streaming, else 0 (spinner row)
- 1 (dock)
- `input.height()` (1–7 rows depending on textarea content)

When the live cell shrinks (e.g., turn finishes, live cell migrates to history), the viewport area shrinks. The terminal's content above doesn't move — only the bottom edge changes.

### Event flow

```rust
pub enum TuiEvent {
    Key(crossterm::event::KeyEvent),
    Paste(String),                // bracketed paste payload
    Resize,                       // terminal size changed
    Draw,                         // scheduled by FrameRequester
    Focus(bool),                  // FocusGained / FocusLost
}
```

`AppEvent` (the existing jfc-ui event enum) continues to drive business logic. The event loop owns the unification: it `select!`s on `TuiEvent` and `AppEvent`, dispatches each accordingly.

`FrameRequester` provides `schedule_frame()`. Every code path that mutates rendered state calls it (stream chunk arrived, tool finished, toast pushed, key handled). The frame rate limiter coalesces bursts so the actual draw rate stays at ~30fps even if 100 `schedule_frame()` calls fire in a millisecond.

### Terminal lifecycle

**Init (`Tui::new`):**

1. Verify stdin and stdout are TTYs (early-bail with a clear error otherwise).
2. `enable_raw_mode()`.
3. `EnableBracketedPaste`, `EnableFocusChange`.
4. Probe kitty keyboard enhancement (bounded probe, ≤200ms timeout, default = no enhancement).
5. Probe initial cursor position via DSR (bounded probe, ≤200ms timeout, default = origin).
6. Detect multiplexer (env vars: `TERM_PROGRAM`, `ZELLIJ`, `TMUX`, `VSCODE_INJECTION`).
7. NO `EnterAlternateScreen` by default — inline mode is the default.
8. NO mouse capture by default — native terminal selection takes precedence.
9. Construct `CustomTerminal::with_options_and_cursor_position(backend, cursor_pos, initial_viewport_height = 1)`. Viewport grows as content lands.
10. Install panic hook that calls `restore_after_exit()`.

**Each frame (`Tui::draw`):**

1. `stdout().sync_update(|_| { ... })` — wraps the whole sequence.
2. If `pending_viewport_area` (resize), update it.
3. Recompute desired viewport height from live cell + spinner + dock + input.
4. Resize viewport via `update_inline_viewport()` (scrolls existing content above as needed).
5. Flush `pending_history_lines` via `insert_history_lines_with_mode(self.is_zellij)`. Invalidate viewport diff buffer if Zellij mode (raw-newline scroll desynchronizes ratatui's tracked back buffer).
6. `terminal.draw(|frame| chat_widget.render(frame))`.

**Restore (drop / panic / `restore_after_exit`):**

- Pop kitty keyboard stack (or hard-reset reporting after exit).
- `DisableBracketedPaste`, `DisableFocusChange`.
- `disable_raw_mode()`.
- `SetCursorStyle::DefaultUserShape`, show cursor.
- If alt-screen active: `LeaveAlternateScreen`.

### Insert history pattern

Pre-wrap and pre-sanitize lines before insertion. ANY text from tool output, assistant prose, user input, or status formatting passes through:

```rust
fn sanitize_for_scrollback(text: &str) -> String { /* strip CSI/OSC/C0/C1 except \t */ }
fn wrap_for_width(line: Line<'static>, width: usize) -> Vec<Line<'static>> { /* adaptive_wrap, URL-aware */ }
```

`insert_history_lines_with_mode` dispatches:

- **`Standard`** — DECSTBM scroll region from row 1 to `viewport.top()`, position cursor at `viewport.top() - 1`, emit one `Print` per pre-wrapped line, reset scroll region. Below-viewport space (if any) is filled via `\x1bM` (Reverse Index) into a scroll region between viewport.top() and screen.height to shift the viewport down without redrawing.
- **`Zellij`** — emit raw newlines at screen bottom to push content up, then `MoveTo(0, cursor_top)` and emit lines with `\r\n` between them. Track `terminal.viewport_area.y` adjustments so ratatui's next diff is correct. (Zellij silently drops DECSTBM, so the scroll-region path produces garbage there.)

Both modes update `terminal.viewport_area` so ratatui's tracked state matches what's on screen.

### Dock content

Single line, no border:

```
⚡Auto · $0.087 · ⚒master · ^P palette · ? help
```

Fields are concatenated with ` · `. When a field is missing (no cost yet, no git branch), it's omitted. No animations in the dock — animations live in the spinner row (which only renders during streaming).

When streaming, the spinner row above the dock shows:

```
✻ Fermenting · 4s · stream active
```

When idle, the spinner row collapses to 0 height.

### Slash commands

Commands that previously opened a picker now emit text into the conversation as an advisor message:

- `/model` → list of models with the current one starred; `/model <name>` switches
- `/theme` → list of themes with the current one starred; `/theme <name>` switches
- `/session list` → table of sessions; `/session resume <id>` resumes
- `/skills` → already a text dashboard, keep
- `/help` → opens the help overlay (the one overlay we keep for the keymap)

The user types a command, sees the output in their scrollback, and acts. No modal pickers, no sidebars.

### Overlays kept

| Overlay        | When                                                                  |
| -------------- | --------------------------------------------------------------------- |
| Approval       | A tool call needs permission. Modal block in viewport above the dock. |
| Toast          | Non-blocking notice (warning, error, info). Top-right of viewport.    |
| Help (`?`)     | User opened the keymap reference. Centered modal.                     |
| Palette (`/`)  | User started typing a slash command. Dropdown anchored above input.   |

Everything else (sidebars, pickers, task panel, teammates panel, agents panel) is **gone**.

## Phases

Each phase is an independent PR. The TUI remains usable after each merge.

### Phase 1 — Foundation port (~2 days)

**Goal:** Replace the broken inline-scrollback work with codex's terminal/insert pattern, behind a small inline viewport. Existing `render::frame` keeps working as a temporary shim.

**Adds:**
- `crates/jfc-ui/src/tui/` — all modules listed in the architecture (custom_terminal, insert_history, frame_requester, frame_rate_limiter, event_stream, keyboard_modes, terminal_probe, multiplexer)
- `Tui` struct exposed from `tui/mod.rs`
- Bounded cursor-position probe (Unix path via `/dev/tty`, with timeout)
- Multiplexer detection: env-var sniffing for `ZELLIJ`, `TMUX`, `VSCODE_INJECTION`, `KITTY_WINDOW_ID`, `WT_SESSION`

**Modifies:**
- `runtime/event_loop/mod.rs` — drives `Tui::draw` instead of `draw_synchronized`
- `runtime/event_loop/mod.rs` — TuiEvent stream replaces direct `crossterm::EventStream` consumption
- `runtime/terminal.rs` — collapsed to `pub use crate::tui::Tui;` and a small set_terminal_title helper
- `cli/mod.rs` — initialization swaps to `Tui::new()`; alternate-screen flag preserved as opt-in
- `cli/terminal.rs` — `TerminalRestoreGuard` rewritten to delegate to `Tui::restore`
- `Cargo.toml` (`jfc-ui`) — adds `derive_more` for `IsVariant` if not already present

**Existing `render::frame` is unchanged** in this phase. It draws into the new viewport. Sidebars/panels still render but get squeezed into ~15 rows — accepted as a temporary state.

**Verification:** No duplication on session load / compaction. No character corruption in scrollback. `cargo check --workspace` green. Visual check by running the binary.

### Phase 2 — Live / History split (~2 days)

**Goal:** Push finalized cells into native scrollback. Live cell renders in viewport.

**Adds:**
- `crates/jfc-ui/src/chat/history.rs` — `History` struct holding finalized cell renderings; `flush_to_scrollback()`
- `crates/jfc-ui/src/chat/live_cell.rs` — `LiveCell` renderer that takes one `ChatMessage` and produces wrapped Lines
- `CellState` enum and tracking on `ChatMessage` (or alongside it in a parallel map keyed by message id)
- `pending_history_lines: Vec<Line<'static>>` on `Tui` (already there in Phase 1 if we mirror codex)
- `LIVE_CELL_MAX_ROWS = 40` and the proactive flush rule

**Modifies:**
- `runtime/event_loop/handlers/stream_done.rs` — when the turn is genuinely done (EndTurn, no pending tools/approvals/classifiers/pause-turn), mark the live cell `Finalizing` and call `frame_requester.schedule_frame()`
- `render/messages.rs` — only renders cells where `state == Live`. Finalized cells are skipped (they're in scrollback).
- `Tui::draw` — moves `Finalizing → History`: takes their wrapped Lines, drains into `pending_history_lines`, then flushes via `insert_history_lines_with_mode`.

**Sanitization & wrapping:** the `sanitize_for_scrollback` helper (already implemented in stash) ports over verbatim. Wrapping uses an adaptive wrap helper (port of codex's `adaptive_wrap_line`) with URL preservation as a stretch.

**Verification:** Finalized turns appear in terminal scrollback ONCE. Live cell visible in viewport. Compaction works (old transcript stays in scrollback, new summary becomes a new live cell). `cargo check --workspace` green.

### Phase 3 — Composition slim (~2 days)

**Goal:** Delete the slop. Flat dock. No sidebars.

**Adds:**
- `crates/jfc-ui/src/chat/mod.rs` — `ChatWidget` top-level composer (replaces `render::frame`)
- `crates/jfc-ui/src/chat/bottom_pane.rs` — layout: spinner row + dock + input
- `crates/jfc-ui/src/chat/dock.rs` — flat one-line dock
- `crates/jfc-ui/src/chat/input.rs` — wraps existing `ratatui-textarea` usage
- `crates/jfc-ui/src/chat/approval.rs` (moved + simplified from `render/approval.rs`)
- `crates/jfc-ui/src/chat/toast.rs` (carved out of `render/overlays.rs`)
- `crates/jfc-ui/src/chat/palette.rs` (moved + simplified from `render/palette.rs`)
- `crates/jfc-ui/src/chat/help.rs` (carved out of `render/overlays.rs`)

**Deletes:**
- `render/sidebar.rs`, `render/session_sidebar.rs`, `render/session_picker.rs`
- `render/model_picker.rs`, `render/theme_picker.rs`
- `render/teammates_panel.rs`, `render/task_panel.rs`, `render/agents.rs`
- `render/visual.rs`
- `render/messages.rs`, `render/frame.rs`, `render/input_box.rs`, `render/status.rs`
- `render/overlays.rs` (after carve-outs)

**Rewires:**
- `/model` slash command — emits a model-list advisor message; `/model <name>` switches mid-session
- `/theme` slash command — same pattern
- `/session list` and `/session resume <id>` — same pattern
- Spinner only when `is_streaming || compacting || has_pending_tools || turn_started_at.is_some()`. When all false, spinner row height = 0.
- Toast position: top-right of viewport, max 3 visible, auto-dismiss after 4s.

**State cleanup:**
- Drop sidebar-related `app::App` fields: `show_sidebar`, `session_sidebar_state`, etc.
- Drop task-panel-related fields: `task_panel_selected`, `task_panel_state`, `task_panel_detail`, `viewing_task_id`
- Keep the underlying task store; `/tasks` slash command can show it as text

**Verification:** No sidebars visible. Dock fits in one line. Spinner only animates during real work. `cargo check --workspace` green. ~5k LOC deleted from `render/`.

### Phase 4 — Polish (~1 day)

**Goal:** Cross-multiplexer correctness + parity with codex's robustness.

**Adds:**
- Zellij detection (already in Phase 1) drives `InsertHistoryMode::Zellij` in `insert_history_lines_with_mode`
- Ctrl-Z suspend on Unix: `SuspendContext` analog (capture cursor, restore terminal, raise SIGTSTP; on SIGCONT reattach + reapply modes + redraw)
- `with_restored(RestoreMode, async closure)` for `/edit` (open `$EDITOR` on the current input or last assistant message)
- URL-aware wrapping: `line_contains_url_like`, `line_has_mixed_url_and_non_url_tokens` from codex; URL-only lines kept intact so terminals can hyperlink them
- OSC 8 hyperlink-aware `display_width` for status formatting (port of codex's `display_width`)

**Verification:** Tested in tmux, Zellij, VS Code terminal, kitty, alacritty. Ctrl-Z works without leaving the terminal in a wedged state. `$EDITOR` integration works. `cargo check --workspace` green.

## Risks & mitigations

- **Risk:** Pre-emptive live-cell flush (Section "Live cell scrollback budget") makes the same assistant message half-in-scrollback / half-in-viewport. The boundary isn't visually marked.
  **Mitigation:** Accept it (codex does too). Add a one-line `─── continued ───` separator at the boundary if user feedback says it's confusing.

- **Risk:** Kitty keyboard probe at startup adds latency on terminals that don't reply.
  **Mitigation:** Bounded probe via `/dev/tty` with 200ms timeout (port codex's `terminal_probe`). On timeout, assume no enhancement and continue.

- **Risk:** Zellij detection misses some configurations.
  **Mitigation:** Expose `JFC_INSERT_HISTORY_MODE=zellij|standard` env var as a manual override.

- **Risk:** Deleting sidebars breaks user muscle memory (Ctrl+B opens sidebar).
  **Mitigation:** Rebind Ctrl+B to `/session list` (text output). Document in `/help`.

- **Risk:** `app.messages = compacted` path silently re-uses indices — the live cell tracker needs to be index-agnostic.
  **Mitigation:** `CellState` is tracked keyed by a per-message id (already on `ChatMessage`), not by Vec index. Compaction creates fresh ids.

- **Risk:** The Phase 1 shim (existing `render::frame` drawn into a 15-row viewport) will look cramped because sidebars still try to fit. This is a 1-2 day eyesore.
  **Mitigation:** Document the temporary state in the Phase 1 PR description. Phase 2 starts before Phase 1 lands if needed (work in parallel branches).

## Open questions

- **`--alt-screen` opt-in mode** — keep it as a manual escape hatch (for users in stripped-down terminals where DECSTBM + ESC M doesn't work)? Codex keeps it. **Decision:** keep as `--alt-screen` flag and `[tui] alternate_screen = true` config, default off.
- **Mouse capture default** — user memory says native selection takes precedence. **Decision:** mouse capture **off** by default; opt-in via `--mouse` flag.
- **Help overlay (`?`)** — the only overlay-style picker we keep. Centered, modal, dismissable with `Esc`/`q`. No tabs.

## Out-of-scope follow-ups

These are deferred and tracked separately:

- Windows support for the `tui/` modules (no DSR cursor probe via `/dev/tty`)
- Voice input
- IDE context
- Pager overlay
- Multi-agent panels
- External agent config migration UI

## Acceptance criteria (per phase)

**Phase 1:** Native scrollback contains nothing yet. Inline viewport renders the current TUI at the bottom. No duplication. No corruption. No crash on terminal resize.

**Phase 2:** A finalized turn appears in native scrollback ONCE. Selecting/copying it via the terminal works. Live cell renders only the in-flight turn. Compaction does not produce duplicates.

**Phase 3:** No sidebar. No task panel. No teammates panel. Dock is one line. `/model`, `/theme`, `/session list` produce text output. Spinner row collapses when idle.

**Phase 4:** Verified working in tmux, Zellij, VS Code terminal, kitty, alacritty. Ctrl-Z and `/edit` work. URLs in tool output are clickable.

Across all phases: `cargo check --workspace` green. No new unsafe blocks. No new `unwrap()` outside tests.

## Implementation

Next step per the workflow: invoke the `superpowers:writing-plans` skill to break Phase 1 into actionable subtasks, then start execution.

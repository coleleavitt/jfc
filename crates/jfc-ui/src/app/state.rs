use std::{cell::RefCell, collections::HashMap, sync::Arc, time::Instant};

use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::widgets::TableState;
use ratatui_textarea::TextArea;
use tokio::sync::Mutex;

use crate::auto_mode::AutoModeConfig;
use crate::context::{ReadDedupCache, ToolContext};
use crate::provider::{ModelId, ModelInfo, Provider, ProviderId};
use crate::query::QueryCache;
use crate::render_cache::RenderCache;
use crate::slate::SlateRouter;
use crate::tasks::TaskId;
use crate::theme::Theme;
use crate::types::*;

use super::{PendingApproval, PermissionMode, load_recent_models};

pub const DEFAULT_CONTEXT_WINDOW_TOKENS: usize = 200_000;

#[derive(Debug, Clone, Default)]
pub struct TranscriptSearch {
    pub query: String,
    pub matches: Vec<usize>,
    pub cursor: usize,
}

#[derive(Debug, Clone)]
pub struct QueuedPrompt {
    pub text: String,
    pub is_meta: bool,
    /// Image/PDF attachments captured at queue time. If the user pasted
    /// an image and then typed a prompt while another turn was already
    /// streaming, the referenced `[Image #N]` attachments are extracted
    /// from `app.pasted_images` and pinned to THIS prompt so they
    /// attach atomically when `drain_queued_prompts` promotes the entry.
    pub attachments: Vec<crate::attachments::Attachment>,
}

pub const SPINNER: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
pub const IDLE_TICK_MS: u64 = 80;
pub const ANIM_TICK_MS: u64 = 33;
/// If no stream event arrives within this duration while `is_streaming` is
/// true, the watchdog resets the flag to stop the 30fps animation loop.
pub const STREAM_WATCHDOG_TIMEOUT_SECS: u64 = 30;
/// Cap on how many turns of token usage we retain for the info-sidebar
/// sparkline. 32 datapoints fit comfortably in a 30-col-wide sidebar
/// while still showing a meaningful trend.
pub const TOKEN_HISTORY_CAP: usize = 32;

pub struct BackgroundTask {
    pub task_id: crate::ids::TaskId,
    pub description: String,
    pub status: crate::types::TaskLifecycle,
    pub started_at: std::time::Instant,
    pub summary: Option<String>,
    pub error: Option<String>,
    pub last_tool: Option<String>,
    pub messages: Vec<String>,
    /// Cumulative tool invocations the subagent has made this run.
    /// Mirrors v131's `toolUseCount` (cli.2.1.131.beautified.js, `jOH()`).
    pub tool_use_count: u32,
    /// Most recent request's input-token count (driven by `Usage` stream
    /// events when the provider emits them, falls back to a 4-chars-per-
    /// token byte estimate otherwise). Mirrors v131's `latestInputTokens`.
    pub latest_input_tokens: u64,
    /// Most recent request's cache-read token count.
    pub latest_cache_read_tokens: u64,
    /// Most recent request's cache-write token count.
    pub latest_cache_write_tokens: u64,
    /// Sum of output tokens across every API round-trip in this run.
    /// Mirrors v131's `cumulativeOutputTokens`. The fan-UI badge displays
    /// the latest request context plus cumulative output.
    pub cumulative_output_tokens: u64,
    /// Model the agent is currently using. Captured from the spawn site
    /// so per-agent cost can be computed via `cost::cost_for(model, usage)`.
    pub model_used: Option<String>,
    /// Per-agent token budget. When set and `latest_input + cumulative_output`
    /// exceeds it, the agent is forcibly terminated and an error toast
    /// fires. Defaults to None (unlimited).
    pub max_input_tokens: Option<u64>,
    /// Set once per task when the budget gets crossed so we don't fire
    /// the kill / toast multiple times.
    pub budget_killed: bool,
}

pub struct App {
    pub theme: Theme,
    /// Verbosity / formatting style for assistant replies. Routes
    /// through `OutputStyle::system_prompt_suffix()` at request-build
    /// time. `Default` is the no-op (current jfc behaviour).
    pub output_style: crate::output_style::OutputStyle,
    pub messages: Vec<ChatMessage>,
    pub streaming_text: String,
    pub streaming_reasoning: String,
    /// v126-style cumulative byte counter for ALL streamed response content:
    /// text deltas + thinking deltas + tool input JSON deltas. Divided by 4
    /// for the spinner's token estimate (matches v126's `responseLengthRef.current / 4`).
    /// Reset at the start of each streaming turn.
    pub streaming_response_bytes: usize,
    pub streaming_assistant_idx: Option<usize>,
    pub is_streaming: bool,
    /// Updated on every inbound stream event (chunk, tool delta, done, error).
    /// Used by the watchdog to detect stuck `is_streaming` flags — if no
    /// stream activity arrives within `STREAM_WATCHDOG_TIMEOUT`, the flag is
    /// force-reset to stop the 30fps animation loop.
    pub last_stream_event_at: Option<Instant>,
    /// Wall-clock instant the current turn's stream began. Set when
    /// `is_streaming` flips true; cleared when it flips false. Drives the
    /// `(5m 10s · …)` elapsed counter in the v126-style spinner — without
    /// it, the spinner can't show how long we've been waiting.
    pub streaming_started_at: Option<Instant>,
    /// Wall-clock instant the *user-level turn* started. Survives across
    /// agentic-loop iterations (each tool batch → new sub-stream
    /// resets `streaming_started_at`, but `turn_started_at` keeps
    /// running). Reset only when the user submits a fresh prompt OR
    /// when the agentic loop fully concludes (no more tools to run, no
    /// pending approvals, EndTurn). The spinner clock reads this so a
    /// 5-step agentic loop shows `5m 10s` cumulative, not `0s` after
    /// every sub-stream restart — that's what `Fermenting… (0s · ↓ 69
    /// tokens)` after a multi-turn turn was: the timer reset every
    /// loop iteration. v126's spinner uses the same turn-level clock.
    pub turn_started_at: Option<Instant>,
    /// Index into `messages` of the user-prompt the up-arrow recall is
    /// currently displaying, counting backwards from the end. `None`
    /// means the user is editing a fresh prompt (not recalled). Each
    /// up-arrow at empty input increments toward older prompts; each
    /// down-arrow decrements. Mirrors v126's `useArrowKeyHistory`
    /// behavior — a quality-of-life win for resend/edit workflows.
    pub history_cursor: Option<usize>,
    /// Wall-clock instant of the most recent text/reasoning delta. Used by
    /// the spinner to detect stalls (`>=15s` → "warming up", up to `>=60s`
    /// → "almost done thinking"). Mirrors v126 `timeSinceLastToken` (cli.js
    /// line 323162).
    pub streaming_last_token_at: Option<Instant>,
    /// Instant the model started producing reasoning output (extended
    /// thinking). Set on the first reasoning chunk per turn; cleared when
    /// the turn ends. Mirrors v126's `streamMode = "thinking"` transition
    /// (cli.js HcH:413611).
    pub thinking_started_at: Option<Instant>,
    /// Instant the model stopped producing reasoning and switched to
    /// regular text output (or the stream ended without text). Once set,
    /// the spinner stops showing "thinking" and starts showing
    /// `thought for Ns`. Mirrors v126's `streamingEndedAt` field on the
    /// thinking-status reducer (cli.js HcH:413585).
    pub thinking_ended_at: Option<Instant>,
    pub scroll_offset: usize,
    pub total_lines: usize,
    /// Cache key for `total_lines`: (message_count, streaming_text_len, last_width).
    /// When any component changes, `message_view_total_lines` is recomputed.
    pub total_lines_key: (usize, usize, usize),
    pub textarea: TextArea<'static>,
    pub show_palette: bool,
    pub palette_input: String,
    pub palette_selected: usize,
    pub show_theme_picker: bool,
    pub theme_picker_input: String,
    pub theme_picker_selected: usize,
    pub spinner_frame: usize,
    pub provider: Arc<dyn Provider>,
    pub providers: Vec<Arc<dyn Provider>>,
    pub model: ModelId,
    /// Recently selected models (most recent first, max 5). Shown at the
    /// top of the model picker for quick switching. Persisted to
    /// `~/.config/jfc/recent_models.json`.
    pub recent_models: Vec<String>,
    pub cwd: String,
    pub reasoning_expanded: HashMap<usize, bool>,
    pub pending_approval: Option<PendingApproval>,
    /// FIFO of tool calls waiting for approval behind the current one. When the
    /// model emits multiple approvable tools in one turn (six `bash` calls in a
    /// single response is common), only the first one fits in `pending_approval`
    /// — the rest queue here. After the user decides on the current tool, the
    /// next is dequeued into `pending_approval`. Without this, subsequent tools
    /// were silently dropped, leaving the conversation with a tool_use that
    /// had no matching tool_result and a stalled agentic loop.
    pub approval_queue: std::collections::VecDeque<ToolCall>,
    /// FIFO of user prompts the user submitted while the model was streaming.
    /// v126 calls these `queued_command` attachments. They render in the
    /// transcript immediately as user messages (so the user sees their input
    /// landed) but don't go to the API until the current turn finishes.
    /// Drained by `drain_queued_prompts()` after `is_streaming` flips false
    /// AND the approval pipeline is empty. Each entry remembers whether the
    /// user typed a slash command (v126's `isMeta: true`) — those run
    /// locally on drain instead of going to the API.
    pub queued_prompts: std::collections::VecDeque<QueuedPrompt>,
    /// Cached count of agent-isolated worktrees (excludes the primary
    /// checkout). Refreshed by the Tick handler at most every
    /// `WORKTREE_REFRESH_MS` so the status-bar badge stays accurate
    /// without shelling out to `git worktree list` on every redraw.
    pub worktree_count: usize,
    pub worktree_count_last_refresh: Option<std::time::Instant>,
    /// Cached current git branch (e.g. "master", "feat/x"). Updated by
    /// the Tick handler at most every `GIT_BRANCH_REFRESH_MS` ms so a
    /// long session reflects branch switches without shelling out
    /// every render frame. None when not in a git repo.
    pub git_branch: Option<String>,
    pub git_branch_last_refresh: Option<std::time::Instant>,
    /// Set of group-keys (`format!("{msg_idx}:{first_tool_id}")`)
    /// currently expanded. Default = collapsed: dense Read/Glob/Grep
    /// runs render as one "▶ N reads · click to expand" row, click
    /// or `o` toggles.
    pub tool_group_expanded: std::collections::HashSet<String>,
    /// Active transcript search. `None` when not searching. The
    /// search bar at the bottom of the screen, the match highlight
    /// in messages, and the n/N navigation all key off this.
    pub transcript_search: Option<TranscriptSearch>,
    /// Slash-command autocomplete popup state. `Some(idx)` while the
    /// user is typing a command and the popup is open. None when the
    /// popup is dismissed.
    pub slash_popup_selected: Option<usize>,
    /// Wall-clock instant of the last successful session save. The
    /// status-bar render shows "✓ saved" briefly after this fires,
    /// fading after `SAVED_BADGE_TTL_MS` so the indicator doesn't
    /// linger on every render.
    pub last_session_save_at: Option<std::time::Instant>,
    /// Cycle index for `Ctrl+L`. Each press copies the next-oldest
    /// `path:line` reference detected in the most recent tool
    /// output. Reset whenever a fresh ToolResult lands so the user
    /// always starts from the most recent.
    pub path_yank_cursor: usize,
    /// Index into `messages` of the user message currently being
    /// edited. None when not editing. Submission while this is Some
    /// rewrites the message at this index and drops everything
    /// after it before re-firing the turn — `Ctrl+E` to enter,
    /// Esc to cancel.
    pub editing_message_idx: Option<usize>,
    /// Set to true on double-ESC. Streaming, agentic-loop continuation,
    /// and the subagent runner all sample this between iterations and
    /// bail when it flips. Wrapped in `Arc` so spawned tasks can clone
    /// a handle into their own scope. Mirrors v126's `abortController`.
    /// Toggled by `?` (when input bar is empty). When true, an
    /// overlay listing every keybinding is rendered on top of the
    /// transcript. Discoverability for muscle-memory features
    /// (Ctrl+X chord, ESC×2 interrupt, `o` to expand, etc.) that
    /// otherwise live only in source comments.
    pub show_help: bool,
    /// True between Ctrl+G and the follow-up letter that selects the
    /// jump target (e/t/m/a). Esc cancels. Drives a small hint row
    /// in the status area so the user knows the chord is armed.
    pub jump_armed: bool,
    pub jump_armed_at: Option<std::time::Instant>,
    /// Most recent tool-block click timestamp, keyed by tool id. The
    /// click handler uses this to detect double-click (same tool id
    /// within `DOUBLE_CLICK_MS`) for the pin gesture.
    pub last_tool_click: Option<(String, std::time::Instant)>,
    /// Bounds of the sessions sidebar block (set on each render).
    /// The mouse handler reads this to decide whether a click hit a
    /// session row and which row it was. `None` when the sidebar is
    /// hidden — in that case the click handler ignores sidebar
    /// coordinates.
    pub sidebar_rect: std::cell::RefCell<Option<ratatui::layout::Rect>>,
    /// Bounds of the messages area, used by the drag-scroll handler
    /// to convert pixel deltas to scroll offsets and to gate scroll
    /// events to the right region.
    pub messages_rect: std::cell::RefCell<Option<ratatui::layout::Rect>>,
    /// Bounds of the toast overlay strip; used by the click handler
    /// to map a click to a toast index for instant dismissal.
    pub toasts_rect: std::cell::RefCell<Option<ratatui::layout::Rect>>,
    /// Last known drag-Y, set on each MouseEventKind::Drag event so
    /// the next drag delta can advance scroll_offset by the
    /// difference. Reset on Down / Up so a fresh drag starts cleanly.
    pub drag_anchor_y: Option<u16>,
    /// Per-turn token usage history (input + output) for the
    /// sparkline rendered in the info sidebar. Pushed each time a
    /// `StreamUsage` event lands at end-of-turn. Capped at the last
    /// `TOKEN_HISTORY_CAP` turns so a long session doesn't grow it
    /// unbounded.
    pub token_history: std::collections::VecDeque<u64>,
    /// task_id of whichever subagent / teammate emitted activity most
    /// recently (AgentChunk or Progress event). Render that row bold +
    /// accent in the spinner-area tree so the user can tell which
    /// agent is currently moving vs. idle. None means nothing has
    /// reported activity this turn.
    pub last_active_agent_task: Option<String>,
    pub interrupt_flag: std::sync::Arc<std::sync::atomic::AtomicBool>,
    /// Per-turn cancellation token. Cloned into every spawned task that
    /// holds critical state (stream_response, tool dispatch, compact,
    /// session save) so an ESC×2 / interrupt can race the in-flight work
    /// against `.cancelled()` instead of waiting for the AtomicBool poll
    /// to come around. Re-minted on every fresh user turn so the previous
    /// token's cancelled state doesn't poison the next stream. wg-async
    /// pattern: tasks holding state must be explicitly cancellable, not
    /// just dropped via a flag the task may never poll.
    pub cancel_token: tokio_util::sync::CancellationToken,
    /// Timestamp of the most recent ESC press in the main shortcut
    /// handler. The next ESC within `INTERRUPT_DOUBLE_TAP_MS` triggers
    /// an interrupt instead of just clearing the input.
    pub last_esc_at: Option<std::time::Instant>,
    pub always_approved: Vec<String>,
    pub session_approved: Vec<String>,
    pub follow_bottom: bool,
    pub pending_tool_calls: Vec<ToolCall>,
    pub max_context_tokens: usize,
    /// Set by `/compact` slash command. Picked up by the main loop next time
    /// it would otherwise check `compact::should_compact` — forces compaction
    /// regardless of token level. Cleared after the compact runs (success or
    /// not) so a single `/compact` invocation triggers exactly one attempt.
    pub force_compact_pending: bool,
    /// Set after compaction permanently fails (CircuitBreakerTripped,
    /// Unsupported, Exhausted). Prevents the post-response handler from
    /// re-spawning compact on every AllToolsComplete — without this, the
    /// circuit breaker fires every 4-5s for the rest of the session.
    /// Cleared on: new session (/clear), manual /compact, model switch.
    pub compact_suppressed: bool,
    /// Wall-clock instant the current compaction started. `Some` while a
    /// compact request is in flight (set on `CompactionStarted`, cleared on
    /// `CompactionDone`/`CompactionFailed`). The renderer shows a
    /// `Compacting…` spinner whenever this is `Some`, so a long pre-submit
    /// compaction doesn't look like a frozen UI.
    pub compacting_started_at: Option<Instant>,
    /// Cumulative summary-text length collected during the in-flight
    /// compact (across all retry attempts). The spinner divides by 4 to
    /// get a chars-per-token estimate and renders `↓ Nk tokens` —
    /// matches the regular streaming spinner's live counter so the user
    /// sees the same kind of feedback during compaction.
    pub compacting_output_chars: u64,
    /// Sum of completed retry attempts' final output sizes. `compact()`
    /// retries internally when post_tokens is still over the Blocked
    /// threshold, and each attempt streams a fresh response from 0.
    /// Without this baseline, `compacting_output_chars` would jump back
    /// down at every retry boundary — visible to the user as a
    /// flickering/resetting counter. The handler bumps this whenever it
    /// detects the per-attempt counter regressing.
    pub compacting_attempt_baseline: u64,
    /// Last `output_chars` value seen this attempt. Used to detect a
    /// regression (new attempt starting) so the baseline gets the prior
    /// attempt's high-water added.
    pub compacting_last_progress: u64,
    /// Set each frame by the renderer. Used for page-scroll math.
    pub viewport_height: usize,
    pub input_wrap_width: usize,
    pub tool_ctx: ToolContext,
    pub dedup_cache: Arc<Mutex<ReadDedupCache>>,
    pub show_model_picker: bool,
    pub model_picker_filter: String,
    pub model_picker_selected: usize,
    pub model_picker_models: Vec<ModelInfo>,
    /// Drives selection + scroll for the picker's `Table`. Kept in sync with
    /// `model_picker_selected` so existing handlers keep working, but ratatui's
    /// stateful render uses the `TableState` for autoscroll when the cursor moves
    /// past the visible area.
    pub model_picker_state: TableState,
    /// Cache of `Provider::fetch_models()` results, keyed by `Provider::name()`. Populated
    /// asynchronously at startup; consulted by the picker before falling back to the
    /// provider's static `available_models()`.
    pub provider_models: HashMap<ProviderId, Vec<ModelInfo>>,
    pub model_picker_query_cache: QueryCache<Vec<ModelInfo>>,
    /// OAuth seat tier from `/api/oauth/profile` (e.g. `"opus"`, `"opusplan"`,
    /// `"claude-opus-4-6[1m]"`). Drives `apply_seat_tier_filter()` in the picker.
    pub seat_tier: Option<String>,
    /// OAuth subscription type (`"max"`, `"pro"`, `"enterprise"`) — shown in the
    /// status bar so the user knows which plan they're billing against.
    pub subscription_type: Option<String>,
    /// Account email from the OAuth profile, surfaced in the status bar.
    pub account_email: Option<String>,
    /// Whether the sessions sidebar is visible. Default off so the chat takes
    /// the full width — toggle with Ctrl+B.
    pub show_sidebar: bool,
    /// Cached list of session metadata (newest first), refreshed when the
    /// sidebar opens. Storing here keeps render() pure of disk I/O. Replaced
    /// the raw-id `session_ids` cache so the sidebar can show titles, cwd
    /// badges, and relative timestamps instead of `ses_2026...` ids.
    pub session_meta: Vec<crate::session::SessionMetadata>,
    /// Currently-selected sidebar row.
    pub session_selected: usize,
    /// State for the sidebar `List` widget — drives auto-scroll when the
    /// selection moves past the visible area.
    pub session_list_state: ratatui::widgets::ListState,
    /// Active session id (set when the user picks one or starts a new one).
    pub current_session_id: Option<crate::ids::SessionId>,
    /// v126 auto-mode classifier config — `enabled: true` routes every tool
    /// call through the LLM classifier instead of prompting the user.
    /// Loaded from `~/.config/jfc/settings.json` at startup.
    pub auto_mode: AutoModeConfig,
    /// v126 permission mode — controls how tool execution is gated.
    pub permission_mode: PermissionMode,
    /// v126 task/todo store. Persists to `~/.config/jfc/tasks/<session>.json`
    /// so todos survive session resume and compaction. Reused across the
    /// agent's turns; the slash commands `/task-*` poke it directly.
    pub task_store: std::sync::Arc<crate::tasks::TaskStore>,
    /// Records when each task transitioned to `Completed` so the footer can
    /// keep showing them for 30 seconds with dimmed/strikethrough styling.
    pub task_completion_times: HashMap<TaskId, Instant>,
    /// Whether the full-screen task panel overlay is visible (Ctrl+T).
    pub show_task_panel: bool,
    /// Currently-selected row in the task panel.
    pub task_panel_selected: usize,
    /// Drives selection + scroll for the task panel's `Table`.
    pub task_panel_state: TableState,
    /// Transient per-session map of task_id → current activity description.
    /// Updated by the tool execution loop to show what an in_progress task is
    /// doing (e.g. "Running bash: cargo test", "Reading src/main.rs").
    pub task_activities: HashMap<TaskId, String>,
    /// Plan verification gate: when true, the plan has already been verified
    /// for the current batch of pending tasks. Reset to false whenever new
    /// tasks are created via TaskCreate.
    pub plan_verified_this_batch: bool,
    pub last_usage_input: u32,
    pub last_usage_output: u32,
    /// Auto-expiring toast queue. Pruned every `Tick`. Pushed via
    /// `AppEvent::Toast` from anywhere in the app (compaction milestones,
    /// session save success, classifier blocks). Mirrors v126's terminal
    /// `notification()` for non-blocking status surfacing.
    pub toasts: Vec<crate::toast::Toast>,
    /// `@filename` autocomplete state. `active=false` when not popping;
    /// while active, the input handler routes typed chars into
    /// `query` and `mentions::filter_candidates` re-ranks `candidates`.
    /// Mirrors v126 cli.js:161602 (`autocomplete:accept` /
    /// `autocomplete:dismiss`).
    pub mention: crate::mentions::MentionState,
    /// Cached file list scanned at the start of each mention session
    /// so we don't re-walk the cwd on every keystroke. Refreshed when
    /// `@` is freshly typed.
    pub mention_all_files: Vec<String>,
    /// Active LSP diagnostics, keyed by file path. Rendered as a one-line
    /// `Found N new diagnostic issue(s) in M file(s) (ctrl+o to expand)`
    /// row above the spinner when non-empty. Updated by
    /// `AppEvent::DiagnosticsUpdated`. Mirrors v126 cli.js:338030-338040.
    pub diagnostics: Vec<crate::diagnostics::DiagnosticEntry>,
    /// Whether the Ctrl+O diagnostic-expansion panel is open. v126 cli.js
    /// :338038 advertises `(ctrl+o to expand)` on the summary row; this
    /// is the destination of that key. The panel groups diagnostics by
    /// file and lists each as `<symbol> [Line A:B] <message>` matching
    /// cli.js:338053. Esc closes.
    pub show_diagnostic_panel: bool,
    /// Scroll offset (in lines) for the diagnostic panel body. Reset
    /// to 0 each time the panel is opened so the user always lands at
    /// the top of the list regardless of where they were before.
    pub diagnostic_panel_scroll: usize,
    /// Most recently completed tool — drives the sparkle (✦) flash
    /// next to its gutter for ~600ms after the result lands. `None`
    /// after the sparkle's TTL elapses or when no tool has completed
    /// this session.
    pub recent_tool_completion: Option<(String, std::time::Instant)>,
    /// Last token-arrival timestamp — drives the right-edge token
    /// rain animation. Each `StreamChunk` stamps it; the renderer
    /// reads it to highlight one cell in the rain column with a
    /// fading intensity proportional to age.
    pub last_token_arrival: Option<std::time::Instant>,
    /// First-launch timestamp for the boot sweep animation. Set in
    /// `App::new`; the placeholder renderer uses it to drive a brief
    /// star cascade across "What can I help you with?" on session
    /// start. After ~1.2s the cascade settles into the static
    /// placeholder.
    pub launched_at: std::time::Instant,
    /// Stable keys for diagnostics already shown to the user, so the
    /// summary row doesn't keep popping for the same set on every
    /// re-publish. Mirrors v126 cli.js:231025-231036's per-URI
    /// "delivered" set. Cleared on `/check` rerun and when the user
    /// opens the expansion panel (Ctrl+O), since opening implies
    /// acknowledgment.
    pub delivered_diagnostics: std::collections::HashSet<String>,
    /// The (input, output, cache_read, cache_write) reading the last time
    /// `add_delta` was applied to `usage_by_model`. Anthropic sends
    /// **cumulative** counts in every `message_delta`, so we have to
    /// subtract this baseline before adding to per-model totals — otherwise
    /// every delta would be triple-counted (Claude sends 5-15 deltas per
    /// turn) and `Usage by model` shows numbers an order of magnitude too
    /// high. Reset to (0,0,0,0) when a new turn starts.
    pub usage_apply_baseline: (u32, u32, u32, u32),
    /// Reasoning-effort pin for this session. `/effort low|medium|high|xhigh|max`
    /// flips it; `stream_response` mirrors `effort_state.api_param()` into
    /// the `reasoning_effort` field of `StreamOptions` if the active model
    /// supports it.
    pub effort_state: crate::effort::EffortState,
    /// Last time we fired the OnHeartbeat hook. Tick handler checks this
    /// every 80ms and fires the hook at most once every 30s when idle.
    pub last_heartbeat_at: Option<std::time::Instant>,
    /// Last MCP refresh counter we observed. Tick handler compares this
    /// against `mcp::registry::refresh_counter()` to detect inbound
    /// `notifications/tools/list_changed` and emit a toast + reminder.
    pub last_mcp_refresh_seen: u64,
    /// Last file-watcher change-counter we observed. Tick handler
    /// compares against `file_watcher::change_counter()` to detect
    /// CLAUDE.md / agents / settings edits and prepend a system-
    /// reminder on the next outbound prompt.
    pub last_file_watcher_seen: u64,
    /// Last keybindings-watcher change-counter we observed. Tick handler
    /// compares against `file_watcher::keybindings_change_counter()` to
    /// detect `keybindings.toml` edits and hot-reload them.
    pub last_keybindings_watcher_seen: u64,
    /// Message indices the user pinned via `/pin <idx>`. Compaction
    /// preserves pinned messages verbatim regardless of token pressure.
    /// Stored as indices into `messages` rather than a flag on
    /// ChatMessage so we don't have to touch every construction site.
    pub pinned_message_indices: std::collections::HashSet<usize>,
    /// `/verbose` toggle: when true, tool blocks render expanded by
    /// default. When false (default), they preview to N lines.
    pub verbose_mode: bool,
    /// `/fast` toggle — mirrors Claude Code v2.1.139's `/fast` command (Alt+O).
    /// When true, the `fast-mode-2026-02-01` beta header is added to every
    /// Anthropic API request, routing to the lower-latency inference path.
    pub fast_mode: bool,
    /// Per-session FIFO of tool mutations the user can `/undo`. Each
    /// entry captures `(file_path, prev_content, op_label)` before the
    /// tool runs. Capped at 100 entries (the oldest gets dropped). New
    /// entries push to the back; /undo pops the back (most recent
    /// first).
    pub tool_undo_history: std::collections::VecDeque<crate::types::ToolUndoEntry>,
    /// v132 Marsh (mid-stream bash → model) buffer. Each entry is
    /// `(tool_id, line)` captured from `ToolOutputChunk`. `stream.rs`
    /// drains this on the next outbound request so the model sees what
    /// bash printed since the last turn.
    pub pending_marsh_chunks: std::sync::Arc<std::sync::Mutex<Vec<(String, String)>>>,
    /// Highest budget threshold the user has been warned about so far this
    /// session. 0 = no warnings yet, 80 = 80% warning shown, 100 = 100%
    /// warning shown. Prevents toast spam when the same threshold is
    /// crossed multiple times across re-renders.
    pub cost_budget_warned_at: u8,
    pub background_tasks: HashMap<String, BackgroundTask>,
    pub show_info_sidebar: bool,
    pub mcp_servers: Vec<crate::types::McpServerInfo>,
    pub lsp_servers: Vec<crate::types::LspServerInfo>,
    pub usage_by_model: HashMap<String, crate::types::ModelUsage>,
    /// Cached snapshot of the active Anthropic OAuth account's utilization
    /// (refreshed on every successful stream + every ~10s tick). Drives the
    /// ribbon's "5h 47% / 7d 12% · opus weekly" display. `None` for non-
    /// OAuth providers.
    pub anthropic_account_snapshot: Option<crate::providers::anthropic_accounts::AccountSnapshot>,
    /// Last instant we re-queried the rotation manager. Throttle to once
    /// every ~10s so the ribbon stays current without burning a lock per
    /// frame at 30fps.
    pub anthropic_snapshot_refreshed_at: Option<std::time::Instant>,
    /// Last wall-clock time the UI re-read `daemon-state.json` to refresh
    /// counters for detached background workers. Throttled in the Tick
    /// handler so we don't hammer the JSON file every frame.
    pub last_detached_sync_at: Option<std::time::Instant>,
    /// Cached `daemon-state.json` mtime from the last successful parse.
    /// Used to skip the (potentially MB-sized) read+parse when the file
    /// hasn't been touched by any background worker since last poll —
    /// this is the primary CPU-burn fix for sessions with hundreds of
    /// historical background agents accumulated in the state file.
    pub last_detached_state_mtime: Option<std::time::SystemTime>,
    pub leader_key_active: bool,
    pub leader_key_timeout: Option<std::time::Instant>,
    pub viewing_task_id: Option<String>,
    /// Set of `BackgroundTask.messages` indices the user expanded with `o`
    /// while drilled into the subagent task view. Long entries (>80 lines or
    /// >5 KB) collapse to a 5-line preview by default; presence in this set
    /// flips them to fully expanded. Cleared whenever `viewing_task_id`
    /// changes so expansion state is per-drill-in, not sticky across tasks.
    ///
    /// TODO Phase B: once `BackgroundTask.messages` migrates to
    /// `Vec<ChatMessage>` and the subagent view renders through the same
    /// `MessageView` pipeline as the main chat, this field collapses into
    /// per-`ToolCall.display` state and can be removed.
    /// Per-task expansion state. Keyed by `task_id` so navigating
    /// between tasks (or out and back in) preserves what the user has
    /// expanded. Previously a session-wide `HashSet<usize>` that got
    /// `.clear()`ed on every switch — entering a task with 121 hidden
    /// lines required pressing `o` again every time.
    pub viewing_task_expanded: std::collections::HashMap<String, std::collections::HashSet<usize>>,
    /// Per-prompt image staging. Each Ctrl+V / bracketed paste of an image
    /// lands here with a unique `id`; the submit path matches `[Image #N]`
    /// markers in the textarea and moves referenced entries onto the
    /// submitted ChatMessage's `attachments` field. Replaces the old
    /// `pending_attachments → push_pending_tool_attachment` global queue.
    pub pasted_images: Vec<crate::attachments::PastedContent>,
    /// Monotonically incrementing counter for paste IDs within a session.
    pub image_counter: u32,
    /// How many detached background agents transitioned to
    /// Completed/Failed since the last user submit. Incremented by
    /// `sync_detached_background_tasks_from_daemon`; drained to 0 and
    /// surfaced as a system_reminder in `handle_submit` so the parent
    /// model knows agent results are available in the transcript.
    pub background_tasks_completed_since_last_turn: u32,
    /// Per-frame map of `(tool_id, screen_rect)` populated by the message
    /// renderer as each `ToolBlock` paints. The mouse handler reads this to
    /// translate a left-click into the tool whose body should expand —
    /// v126's cli.js (cmd-click on iTerm2) toggles the same per-tool
    /// expand/collapse affordance via mouse. We use plain left-click here
    /// because non-iTerm terminals don't surface the cmd modifier the same
    /// way; the spirit (mouse → toggle that tool) is preserved.
    ///
    /// Cleared at the top of every `render::frame()` and re-populated as
    /// each visible `ToolBlock` renders. Tools scrolled off-screen are not
    /// pushed, so they're automatically un-clickable. `RefCell` because
    /// `MessageView` borrows `&App` immutably during `Widget::render`, and
    /// we need a `&mut` push from inside that path.
    pub tool_hit_regions: RefCell<Vec<(String, Rect)>>,
    /// Content-addressed cache for `markdown::to_lines()` output. Keyed on
    /// `(hash(text), width)` so unchanged messages aren't re-parsed on every
    /// frame. Uses `RefCell` because `MessageView` borrows `&App` immutably
    /// during `Widget::render` but needs mutable cache access.
    pub render_cache: RefCell<RenderCache>,
    /// Cached result of `collect_diff_stats()`. Keyed on
    /// `(messages.len(), total_parts_count)` — invalidates when a message is
    /// appended or a tool result lands. Avoids O(N_messages × N_parts)
    /// HashMap walk per frame; reduces to O(1) lookup on cache hit.
    pub diff_stats_cache: RefCell<Option<(usize, usize, crate::render::DiffStats)>>,
    /// Swarm / team orchestration state. Tracks the current team, spawned
    /// teammates, and message delivery. `None` when no team is active.
    pub team_context: crate::swarm::TeamContext,
    /// Channel receiver for events from in-process teammate runners.
    /// Polled in the main event loop alongside terminal/stream events.
    pub teammate_event_rx:
        Option<tokio::sync::mpsc::UnboundedReceiver<crate::swarm::runner::TeammateEvent>>,
    /// Sender side — cloned into each spawned teammate's runner.
    pub teammate_event_tx: tokio::sync::mpsc::UnboundedSender<crate::swarm::runner::TeammateEvent>,
    /// Slate dynamic model router. `None` when `slate_enabled = false` in the
    /// loaded config (the common case). When `Some`, callers consult it on
    /// every user submission to pick a per-turn model — see
    /// `slate::SlateRouter::route` and `crates/jfc-ui/src/slate.rs`.
    pub slate: Option<SlateRouter>,
    /// Advisor session for `/advisor <query>` (see `crate::advisor`).
    /// `None` until the user invokes `/advisor` for the first time —
    /// mints lazily so the cost is paid only by users who actually use
    /// the feature. The session owns its own model id, transcript, and
    /// token budget; budget exhaustion returns Err and the user must
    /// reset (e.g. via `/clear`) to get a fresh budget.
    pub advisor_session: Option<crate::advisor::AdvisorSession>,
    /// Gate for the `/advisor` slash command. Default OFF per the
    /// deliverable's "no /advisor command without a config flag" rule.
    /// Set via the `JFC_ADVISOR_ENABLED=1` env var on startup OR via a
    /// future config-toml field. When false, the slash command surfaces
    /// a hint message instead of running.
    pub advisor_enabled: bool,
    /// v137 `/goal <condition>` — session-scoped stop condition. When set,
    /// the agent keeps working until this condition is met. `/goal clear`
    /// removes it.
    pub goal_condition: Option<String>,
    /// Shared flag: true when the UI needs high-frequency ticks (animations,
    /// kinetic scroll, boot sweep). The tick task reads this to choose
    /// `ANIM_TICK_MS` vs `IDLE_TICK_MS`.
    pub wants_animation_frame: std::sync::Arc<std::sync::atomic::AtomicBool>,
    /// Kinetic scroll velocity (lines/sec). Wheel events inject impulse;
    /// each animation tick decays by 0.85 and applies to `scroll_offset`.
    pub scroll_velocity: f32,
    /// Last tick instant for kinetic scroll dt calculation.
    pub last_scroll_tick: std::time::Instant,
    /// Throttle for idle_prefetch: last time a prefetch batch was fired.
    pub last_prefetch_at: std::time::Instant,
    /// Number of prefetch reads currently in-flight (capped at 2).
    pub prefetch_in_flight: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    /// Cached git repository root. `None` = not yet resolved.
    /// `Some(None)` = resolved, not in a git repo.
    /// `Some(Some(path))` = resolved git root directory.
    pub git_root: Option<Option<std::path::PathBuf>>,
    /// Estimated token count of the system prompt from the last stream
    /// request. Used by the compaction handler to add overhead to the
    /// post-compact `approx_tokens` estimate (system prompt + tool defs
    /// are invisible to the message-only local estimate).
    pub last_system_prompt_len: Option<usize>,
}

impl App {
    pub fn new(provider: Arc<dyn Provider>, model: impl Into<ModelId>) -> Self {
        let providers = vec![Arc::clone(&provider)];
        let (teammate_tx, teammate_rx) =
            tokio::sync::mpsc::unbounded_channel::<crate::swarm::runner::TeammateEvent>();
        let mut textarea = TextArea::default();
        textarea.set_cursor_line_style(Style::default());
        // Minimal placeholder — the help overlay and `?` shortcut
        // already document Enter / Shift+Enter; repeating it inline
        // every render was noise. Just a soft prompt.
        textarea.set_placeholder_text("send a message…");

        let cwd = std::env::current_dir()
            .ok()
            .and_then(|p| p.to_str().map(str::to_owned))
            .unwrap_or_default();

        let mut app = Self {
            theme: Theme::dark(),
            output_style: crate::output_style::OutputStyle::default(),
            messages: Vec::new(),
            streaming_text: String::new(),
            streaming_reasoning: String::new(),
            streaming_response_bytes: 0,
            streaming_assistant_idx: None,
            streaming_started_at: None,
            streaming_last_token_at: None,
            thinking_started_at: None,
            thinking_ended_at: None,
            turn_started_at: None,
            history_cursor: None,
            is_streaming: false,
            last_stream_event_at: None,
            scroll_offset: 0,
            total_lines: 0,
            total_lines_key: (0, 0, 0),
            textarea,
            show_palette: false,
            palette_input: String::new(),
            palette_selected: 0,
            show_theme_picker: false,
            theme_picker_input: String::new(),
            theme_picker_selected: 0,
            spinner_frame: 0,
            provider,
            providers,
            model: model.into(),
            recent_models: load_recent_models(),
            cwd,
            reasoning_expanded: HashMap::new(),
            pending_approval: None,
            approval_queue: std::collections::VecDeque::new(),
            queued_prompts: std::collections::VecDeque::new(),
            worktree_count: 0,
            worktree_count_last_refresh: None,
            git_branch: None,
            git_branch_last_refresh: None,
            tool_group_expanded: std::collections::HashSet::new(),
            transcript_search: None,
            slash_popup_selected: None,
            last_session_save_at: None,
            path_yank_cursor: 0,
            editing_message_idx: None,
            show_help: false,
            jump_armed: false,
            jump_armed_at: None,
            last_tool_click: None,
            sidebar_rect: std::cell::RefCell::new(None),
            messages_rect: std::cell::RefCell::new(None),
            toasts_rect: std::cell::RefCell::new(None),
            drag_anchor_y: None,
            token_history: std::collections::VecDeque::with_capacity(TOKEN_HISTORY_CAP),
            last_active_agent_task: None,
            interrupt_flag: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
            cancel_token: tokio_util::sync::CancellationToken::new(),
            last_esc_at: None,
            always_approved: Vec::new(),
            session_approved: Vec::new(),
            follow_bottom: true,
            tool_ctx: ToolContext::new(),
            dedup_cache: Arc::new(Mutex::new(ReadDedupCache::new())),
            pending_tool_calls: Vec::new(),
            force_compact_pending: false,
            compact_suppressed: false,
            compacting_started_at: None,
            compacting_output_chars: 0,
            compacting_attempt_baseline: 0,
            compacting_last_progress: 0,
            max_context_tokens: DEFAULT_CONTEXT_WINDOW_TOKENS,
            viewport_height: 0,
            input_wrap_width: 1,
            show_model_picker: false,
            model_picker_filter: String::new(),
            model_picker_selected: 0,
            model_picker_models: Vec::new(),
            model_picker_state: TableState::default().with_selected(Some(0)),
            provider_models: HashMap::new(),
            model_picker_query_cache: QueryCache::default(),
            seat_tier: None,
            subscription_type: None,
            account_email: None,
            show_sidebar: false,
            session_meta: Vec::new(),
            session_selected: 0,
            session_list_state: ratatui::widgets::ListState::default(),
            current_session_id: Some(crate::session::generate_session_id()),
            auto_mode: crate::auto_mode::load_config(),
            permission_mode: PermissionMode::Default,
            // Tasks are scoped per-session (mirrors v126 cli.js:271505 keying
            // todos by `agentId ?? sessionId`). Opening with a freshly-minted
            // session id means a new run sees an empty task list, even if
            // prior runs left `~/.config/jfc/tasks/<old>.json` on disk. The
            // store is re-opened in `switch_session` whenever the session id
            // changes (load from sidebar, /continue, /clear).
            // NOTE: initialized as in_memory here; re-opened with the real
            // session_id after construction (see below).
            task_store: crate::tasks::TaskStore::in_memory(),
            task_completion_times: HashMap::new(),
            show_task_panel: false,
            task_panel_selected: 0,
            task_panel_state: TableState::default().with_selected(Some(0)),
            task_activities: HashMap::new(),
            plan_verified_this_batch: false,
            last_usage_input: 0,
            last_usage_output: 0,
            toasts: Vec::new(),
            mention: crate::mentions::MentionState::default(),
            mention_all_files: Vec::new(),
            diagnostics: Vec::new(),
            show_diagnostic_panel: false,
            diagnostic_panel_scroll: 0,
            recent_tool_completion: None,
            last_token_arrival: None,
            launched_at: std::time::Instant::now(),
            delivered_diagnostics: std::collections::HashSet::new(),
            usage_apply_baseline: (0, 0, 0, 0),
            effort_state: crate::effort::EffortState::new(),
            last_heartbeat_at: None,
            last_mcp_refresh_seen: 0,
            last_file_watcher_seen: 0,
            last_keybindings_watcher_seen: 0,
            pinned_message_indices: std::collections::HashSet::new(),
            verbose_mode: false,
            fast_mode: false,
            tool_undo_history: std::collections::VecDeque::new(),
            pending_marsh_chunks: std::sync::Arc::new(std::sync::Mutex::new(Vec::new())),
            cost_budget_warned_at: 0,
            background_tasks: HashMap::new(),
            show_info_sidebar: true,
            mcp_servers: Vec::new(),
            lsp_servers: Vec::new(),
            usage_by_model: HashMap::new(),
            anthropic_account_snapshot: None,
            anthropic_snapshot_refreshed_at: None,
            last_detached_sync_at: None,
            last_detached_state_mtime: None,
            leader_key_active: false,
            leader_key_timeout: None,
            viewing_task_id: None,
            viewing_task_expanded: std::collections::HashMap::new(),
            pasted_images: Vec::new(),
            image_counter: 0,
            background_tasks_completed_since_last_turn: 0,
            tool_hit_regions: RefCell::new(Vec::new()),
            render_cache: RefCell::new(RenderCache::new()),
            diff_stats_cache: RefCell::new(None),
            team_context: crate::swarm::TeamContext::default(),
            teammate_event_rx: Some(teammate_rx),
            teammate_event_tx: teammate_tx,
            // Slate is populated *after* `App::new` from the config (see
            // `main.rs::run_app`). Constructor default = None so the unit
            // tests that build a bare `App` don't need to plumb a router.
            slate: None,
            advisor_session: None,
            // Read the env gate once at construction. Tests bypass this
            // by setting the field directly; users who want it on for a
            // session export `JFC_ADVISOR_ENABLED=1` before launch.
            advisor_enabled: std::env::var("JFC_ADVISOR_ENABLED")
                .ok()
                .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
                .unwrap_or(false),
            goal_condition: None,
            wants_animation_frame: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
            scroll_velocity: 0.0,
            last_scroll_tick: std::time::Instant::now(),
            last_prefetch_at: std::time::Instant::now() - std::time::Duration::from_secs(10),
            prefetch_in_flight: std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            git_root: None,
            last_system_prompt_len: None,
        };
        // Open the task store with the real session id so tasks persist to disk.
        if let Some(ref sid) = app.current_session_id {
            app.task_store = crate::tasks::TaskStore::open(sid.as_str());
        }
        app.sync_selected_context_window();
        tracing::info!(
            target: "jfc::app",
            model = %app.model,
            provider = app.provider.name(),
            "App::new"
        );
        app
    }
}

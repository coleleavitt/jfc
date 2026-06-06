use std::{
    cell::RefCell,
    collections::HashMap,
    sync::Arc,
    time::Instant,
};

use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::widgets::TableState;
use ratatui_textarea::TextArea;

use crate::query::QueryCache;
use crate::render_cache::RenderCache;
use crate::theme::Theme;
use jfc_provider::{ModelId, ModelInfo, Provider};

use super::EngineState;

pub const DEFAULT_CONTEXT_WINDOW_TOKENS: usize = 200_000;

/// The expanded panel state cycled by Ctrl+T — mirrors Claude Code's
/// `expandedView: "none" | "tasks" | "teammates"` state machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ExpandedView {
    /// No expanded panel — just the normal pinned task row.
    #[default]
    None,
    /// Full task list panel is showing.
    Tasks,
    /// Teammates/agents expanded view showing transcript previews.
    Teammates,
}

#[derive(Debug, Clone, Default)]
pub struct TranscriptSearch {
    pub query: String,
    pub matches: Vec<usize>,
    pub cursor: usize,
}

/// Ctrl+R reverse-history search over past user prompts (bash-style).
/// `all` is every prior prompt newest-first; `results` are indices into it
/// matching `query`; `selected` indexes into `results`.
#[derive(Debug, Clone, Default)]
pub struct PromptSearch {
    pub query: String,
    pub all: Vec<String>,
    pub results: Vec<usize>,
    pub selected: usize,
}

impl PromptSearch {
    /// The currently-highlighted prompt, if any.
    pub fn selected_text(&self) -> Option<&str> {
        self.results
            .get(self.selected)
            .and_then(|&i| self.all.get(i))
            .map(String::as_str)
    }

    /// Recompute `results` for the current `query` (case-insensitive
    /// substring), clamping `selected` into range.
    pub fn refilter(&mut self) {
        let q = self.query.to_lowercase();
        self.results = self
            .all
            .iter()
            .enumerate()
            .filter(|(_, s)| q.is_empty() || s.to_lowercase().contains(&q))
            .map(|(i, _)| i)
            .collect();
        if self.selected >= self.results.len() {
            self.selected = self.results.len().saturating_sub(1);
        }
    }
}

/// An in-progress / just-released mouse selection over the transcript, in
/// terminal cell coordinates. `anchor` is where the drag began, `head` the
/// current cursor cell. `dragged` flips true once the cursor actually moves,
/// distinguishing a real selection from a plain click. `finalize` is set on
/// button-up so the next render extracts + copies the text exactly once.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TextSelection {
    pub anchor: (u16, u16),
    pub head: (u16, u16),
    pub dragged: bool,
    pub finalize: bool,
    /// Set once the selection has been extracted + copied. The highlight then
    /// persists (so the user sees what was copied) without re-copying, until
    /// the next mouse-down, any scroll, Esc, or resize clears it. Safe under
    /// the absolute-screen-cell model precisely because those clear points
    /// fire the instant the cells could map to different content.
    pub copied: bool,
}

impl TextSelection {
    /// Normalized (top-left, bottom-right) cell span in reading order, so the
    /// renderer can walk rows top-to-bottom regardless of drag direction.
    pub fn ordered(&self) -> ((u16, u16), (u16, u16)) {
        let (a, h) = (self.anchor, self.head);
        // Order by row, then column.
        if (a.1, a.0) <= (h.1, h.0) {
            (a, h)
        } else {
            (h, a)
        }
    }
}

/// Granularity of a multi-click selection: double-click → word, triple → line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SelectKind {
    Word,
    Line,
}

/// A pending word/line selection from a multi-click. The handler records the
/// clicked cell + granularity; the renderer (which holds the buffer) resolves
/// it into a `TextSelection` span and hands it to the normal finalize path.
#[derive(Clone, Copy, Debug)]
pub struct SelectRequest {
    pub col: u16,
    pub row: u16,
    pub kind: SelectKind,
}

pub const SPINNER: &[&str] = crate::glyphs::TASK_FRAMES;
pub const IDLE_TICK_MS: u64 = 80;
pub const ANIM_TICK_MS: u64 = 33;
/// Hard idle limit for a provider stream. This must stay longer than the
/// provider HTTP read timeout (default 300s) so the HTTP layer, not the UI
/// watchdog, reports real network failures. Anthropic/Bedrock streams can
/// legitimately go quiet for minutes while the model is thinking or an upstream
/// proxy is queueing, so we keep ~1 minute of slack above the read timeout.
/// Users who raise `JFC_STREAM_IDLE_TIMEOUT_MS` past this should raise the
/// watchdog too, or the watchdog will fire first.
pub const STREAM_WATCHDOG_TIMEOUT_SECS: u64 = 360;
/// Cap on how many turns of token usage we retain for the info-sidebar
/// sparkline. 32 datapoints fit comfortably in a 30-col-wide sidebar
/// while still showing a meaningful trend.
pub const TOKEN_HISTORY_CAP: usize = 32;

/// Maximum number of pending `<system-reminder>` bodies retained between
/// user turns. Reminders come from filesystem events, MCP changes, and
/// watcher notifications — during long idle sessions a stream of unique
/// log lines would otherwise grow `pending_background_reminders`
/// unbounded. Once the queue is full, the oldest reminder is dropped
/// before a new one is pushed; on the next turn the survivors get
/// flushed via `take_background_reminders`. 20 is enough to never lose
/// signal in normal use, small enough that the per-turn injection stays
/// bounded.
pub const BACKGROUND_REMINDERS_CAP: usize = 20;

pub struct App {
    /// The frontend-neutral engine state: conversation, streaming,
    /// turn control, approvals, tasks/teams, providers, compaction,
    /// and run configuration. Everything the agentic runtime needs to
    /// execute a turn with no UI present. Moves to the jfc-engine
    /// crate in a later stage of the extraction.
    pub engine: EngineState,
    pub theme: Theme,
    /// Text saved by Esc-clear so Up-arrow can recall it. Single slot —
    /// each Esc-clear overwrites. None when no text has been cleared.
    pub esc_saved_text: Option<String>,
    /// Index into `messages` of the user-prompt the up-arrow recall is
    /// currently displaying, counting backwards from the end. `None`
    /// means the user is editing a fresh prompt (not recalled). Each
    /// up-arrow at empty input increments toward older prompts; each
    /// down-arrow decrements. Mirrors v126's `useArrowKeyHistory`
    /// behavior — a quality-of-life win for resend/edit workflows.
    pub history_cursor: Option<usize>,
    /// Trailing window of `(elapsed_since_stream_start, live_token_count)`
    /// samples used to compute the windowed tokens/sec rate shown in the
    /// spinner. Sampled each render frame while streaming; trimmed to
    /// `spinner::TOKEN_RATE_WINDOW`. A windowed Δtokens/Δt is self-smoothing
    /// and reflects *current* throughput, unlike a lifetime average that lags
    /// after a fast opening burst tapers. Cleared at end-of-turn.
    pub scroll_offset: usize,
    pub total_lines: usize,
    /// Cache key for `total_lines`: (message_count, streaming_text_len, last_width).
    /// When any component changes, `message_view_total_lines` is recomputed.
    pub total_lines_key: (usize, usize, usize),
    pub textarea: TextArea<'static>,
    /// Vim modal-editing state for the prompt. `None` = vim off (default,
    /// plain insert editing); `Some` = on, toggled by `/vim`. Routes the
    /// default text-input path through `input::vim` and makes Esc mode-aware.
    pub vim: Option<crate::input::vim::VimState>,
    pub show_palette: bool,
    pub palette_input: String,
    pub palette_selected: usize,
    pub show_theme_picker: bool,
    pub theme_picker_input: String,
    pub theme_picker_selected: usize,
    /// The theme active when the picker opened. While the picker is up, the
    /// highlighted theme is applied live (preview); this restores it if the
    /// user cancels with Esc. `None` when the picker isn't open.
    pub theme_preview_original: Option<Theme>,
    pub spinner_frame: usize,
    /// Hysteresis state machine for the status label — advanced once per tick
    /// so the phase ("Thinking"/"Responding"/…) can't flip per-frame. See
    /// [`crate::spinner::next_phase`].
    pub spinner_state: crate::spinner::SpinnerState,
    pub reasoning_expanded: HashMap<usize, bool>,
    /// Set of group-keys (`format!("{msg_idx}:{first_tool_id}")`)
    /// currently expanded. Default = collapsed: dense Read/Glob/Grep
    /// runs render as one "▶ N reads · click to expand" row, click
    /// or `o` toggles.
    pub tool_group_expanded: std::collections::HashSet<String>,
    /// Active transcript search. `None` when not searching. The
    /// search bar at the bottom of the screen, the match highlight
    /// in messages, and the n/N navigation all key off this.
    pub transcript_search: Option<TranscriptSearch>,
    /// Ctrl+R reverse-history search overlay (None = closed).
    pub prompt_search: Option<PromptSearch>,
    /// Slash-command autocomplete popup state. `Some(idx)` while the
    /// user is typing a command and the popup is open. None when the
    /// popup is dismissed.
    pub slash_popup_selected: Option<usize>,
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
    /// In-progress / just-finished mouse text selection over the transcript.
    /// Drag inside the messages area paints a reverse-video highlight; on
    /// button-up the renderer reads the selected buffer cells, copies them to
    /// the clipboard (OSC 52-aware), and clears the selection. `None` when no
    /// selection is active.
    pub text_selection: Option<TextSelection>,
    /// Click-count tracker for multi-click (word/line) selection:
    /// `(col, row, count, at)`. Distinct from `last_tool_click` (tool-pin).
    pub last_click: Option<(u16, u16, u8, Instant)>,
    /// A word/line selection awaiting renderer resolution against the buffer.
    pub pending_select_request: Option<SelectRequest>,
    /// Debounce/cooldown for the clipboard-image-on-refocus hint (fired from
    /// the focus-gained handler). `None` until the first probe.
    pub last_focus_hint_at: Option<Instant>,
    /// Per-turn token usage history (input + output) for the
    /// sparkline rendered in the info sidebar. Pushed each time a
    /// `StreamUsage` event lands at end-of-turn. Capped at the last
    /// `TOKEN_HISTORY_CAP` turns so a long session doesn't grow it
    /// unbounded.
    /// task_id of whichever subagent / teammate emitted activity most
    /// recently (AgentChunk or Progress event). Render that row bold +
    /// accent in the spinner-area tree so the user can tell which
    /// agent is currently moving vs. idle. None means nothing has
    /// reported activity this turn.
    /// Timestamp of the most recent ESC press in the main shortcut
    /// handler. The next ESC within `INTERRUPT_DOUBLE_TAP_MS` triggers
    /// an interrupt instead of just clearing the input.
    pub last_esc_at: Option<std::time::Instant>,
    pub follow_bottom: bool,
    /// Set each frame by the renderer. Used for page-scroll math.
    pub viewport_height: usize,
    pub input_wrap_width: usize,
    pub show_model_picker: bool,
    pub model_picker_filter: String,
    pub model_picker_selected: usize,
    pub model_picker_models: Vec<ModelInfo>,
    /// Session-picker popup state — same `Clear`+centered-table treatment as
    /// the model picker. Toggled with Ctrl+P. Replaces the "Ctrl+B opens the
    /// session list as a left sidebar" hack for one-shot session selection.
    /// `session_picker_filter` filters by `display_title()` substring.
    pub show_session_picker: bool,
    pub session_picker_filter: String,
    pub session_picker_state: TableState,
    /// Drives selection + scroll for the picker's `Table`. Kept in sync with
    /// `model_picker_selected` so existing handlers keep working, but ratatui's
    /// stateful render uses the `TableState` for autoscroll when the cursor moves
    /// past the visible area.
    pub model_picker_state: TableState,
    pub model_picker_query_cache: QueryCache<Vec<ModelInfo>>,
    /// Whether the sessions sidebar is visible. Default off so the chat takes
    /// the full width — toggle with Ctrl+B.
    pub show_sidebar: bool,
    /// Cached list of session metadata (newest first), refreshed when the
    /// sidebar opens. Storing here keeps render() pure of disk I/O. Replaced
    /// the raw-id `session_ids` cache so the sidebar can show titles, cwd
    /// badges, and relative timestamps instead of `ses_2026...` ids.
    pub session_meta: Vec<jfc_session::SessionMetadata>,
    /// Currently-selected sidebar row.
    pub session_selected: usize,
    /// State for the sidebar `List` widget — drives auto-scroll when the
    /// selection moves past the visible area.
    pub session_list_state: ratatui::widgets::ListState,
    /// Whether the full-screen task panel overlay is visible (Ctrl+T).
    pub show_task_panel: bool,
    /// The expanded view state — cycles none → tasks → teammates → none on Ctrl+T.
    pub expanded_view: ExpandedView,
    /// Currently-selected row in the task panel.
    pub task_panel_selected: usize,
    /// Drives selection + scroll for the task panel's `Table`.
    pub task_panel_state: TableState,
    /// Whether the detail pane is shown for the currently-selected task.
    pub task_panel_detail: bool,
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
    /// Last keybindings-watcher change-counter we observed. Tick handler
    /// compares against `file_watcher::keybindings_change_counter()` to
    /// detect `keybindings.toml` edits and hot-reload them.
    pub last_keybindings_watcher_seen: u64,
    /// `/verbose` toggle: when true, tool blocks render expanded by
    /// default. When false (default), they preview to N lines.
    pub verbose_mode: bool,
    pub show_info_sidebar: bool,
    /// Vertical scroll offset (rows from top) of the right-side info sidebar's
    /// Tasks section. A long todo list (the user hit 27 in one session) now
    /// renders compactly and scrolls instead of overflowing the panel.
    /// Adjusted via Alt+Up / Alt+Down while the sidebar is visible.
    pub info_sidebar_scroll: u16,
    pub leader_key_active: bool,
    pub leader_key_timeout: Option<std::time::Instant>,
    pub viewing_task_id: Option<String>,
    /// Set of `BackgroundTask.messages` indices the user expanded with `o`
    /// while drilled into the subagent task view. Long entries (>80 lines or
    /// >5 KB) collapse to a 5-line preview by default; presence in this set
    /// > flips them to fully expanded. Cleared whenever `viewing_task_id`
    /// > changes so expansion state is per-drill-in, not sticky across tasks.
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
    /// Large text pastes collapsed to `[Pasted #N · …]` chips: `(chip_token,
    /// full_text)`. The chip keeps the input box clean; on submit each chip
    /// in the prompt is expanded back to its full text. Cleared per submit.
    pub pasted_texts: Vec<(String, String)>,
    /// Monotonic id for `[Pasted #N · …]` chips.
    pub paste_counter: u32,
    /// Monotonically incrementing counter for paste IDs within a session.
    pub image_counter: u32,
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
    /// Remote-control host. `Some` when `/remote-control` is active or RC was
    /// started at launch. Events are mirrored to connected clients; client
    /// input is injected into the main event bus. See `crate::remote_host`.
    pub remote_host: Option<std::sync::Arc<crate::remote_host::RemoteHost>>,
    /// Shared flag: true when the UI needs high-frequency ticks (animations,
    /// kinetic scroll, boot sweep). The tick task reads this to choose
    /// `ANIM_TICK_MS` vs `IDLE_TICK_MS`.
    pub wants_animation_frame: std::sync::Arc<std::sync::atomic::AtomicBool>,
    /// Kinetic scroll velocity (lines/sec). Wheel events inject impulse;
    /// each animation tick decays by 0.85 and applies to `scroll_offset`.
    pub scroll_velocity: f32,
    /// Last tick instant for kinetic scroll dt calculation.
    pub last_scroll_tick: std::time::Instant,
    /// Last time the user interacted (typed, submitted, scrolled).
    /// Used for idle-return detection (suggest /clear after 75min away).
    pub last_user_activity_at: std::time::Instant,
    /// Instant of the last direct user interaction (prompt submit, keypress).
    /// Used by the session recap feature: when the user returns after
    /// `session_recap::AWAY_THRESHOLD`, a recap is generated from messages
    /// that arrived after this instant.
    pub last_user_interaction_at: std::time::Instant,
    /// Message index at the time of the last user interaction. Messages
    /// after this index are candidates for the "while you were away" recap.
    pub interaction_message_idx: usize,
    /// Whether the idle-return toast has been shown this idle period.
    pub idle_return_shown: bool,
    /// User-facing "while you were away" recap banner. Set when the user
    /// returns and submits after `session_recap::AWAY_THRESHOLD` of the
    /// agent working autonomously; rendered as a dismissable band at the top
    /// of the transcript and cleared on the next submit or Esc.
    pub away_recap: Option<String>,

    /// `--json`: structured JSON output mode for CI.
    pub json_mode: bool,
}


impl App {
    pub fn new(provider: Arc<dyn Provider>, model: impl Into<ModelId>) -> Self {
        let mut textarea = TextArea::default();
        textarea.set_cursor_line_style(Style::default());
        // Minimal placeholder — the help overlay and `?` shortcut
        // already document Enter / Shift+Enter; repeating it inline
        // every render was noise. Just a soft prompt.
        textarea.set_placeholder_text("send a message…");

        let mut app = Self {
            engine: EngineState::new(provider, model),
            theme: Theme::dark(),
            esc_saved_text: None,
            history_cursor: None,
            scroll_offset: 0,
            total_lines: 0,
            total_lines_key: (0, 0, 0),
            textarea,
            vim: None,
            show_palette: false,
            palette_input: String::new(),
            palette_selected: 0,
            show_theme_picker: false,
            theme_picker_input: String::new(),
            theme_picker_selected: 0,
            theme_preview_original: None,
            spinner_frame: 0,
            spinner_state: crate::spinner::SpinnerState::new(std::time::Instant::now()),
            reasoning_expanded: HashMap::new(),
            tool_group_expanded: std::collections::HashSet::new(),
            transcript_search: None,
            prompt_search: None,
            slash_popup_selected: None,
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
            text_selection: None,
            last_click: None,
            pending_select_request: None,
            last_focus_hint_at: None,
            last_esc_at: None,
            follow_bottom: true,
            viewport_height: 0,
            input_wrap_width: 1,
            show_model_picker: false,
            model_picker_filter: String::new(),
            show_session_picker: false,
            session_picker_filter: String::new(),
            session_picker_state: TableState::default().with_selected(Some(0)),
            model_picker_selected: 0,
            model_picker_models: Vec::new(),
            model_picker_state: TableState::default().with_selected(Some(0)),
            model_picker_query_cache: QueryCache::default(),
            show_sidebar: false,
            session_meta: Vec::new(),
            session_selected: 0,
            session_list_state: ratatui::widgets::ListState::default(),
            show_task_panel: false,
            expanded_view: ExpandedView::None,
            task_panel_selected: 0,
            task_panel_state: TableState::default().with_selected(Some(0)),
            task_panel_detail: false,
            mention: crate::mentions::MentionState::default(),
            mention_all_files: Vec::new(),
            show_diagnostic_panel: false,
            diagnostic_panel_scroll: 0,
            launched_at: std::time::Instant::now(),
            delivered_diagnostics: std::collections::HashSet::new(),
            last_keybindings_watcher_seen: 0,
            verbose_mode: false,
            show_info_sidebar: true,
            info_sidebar_scroll: 0,
            leader_key_active: false,
            leader_key_timeout: None,
            viewing_task_id: None,
            viewing_task_expanded: std::collections::HashMap::new(),
            pasted_images: Vec::new(),
            pasted_texts: Vec::new(),
            paste_counter: 0,
            image_counter: 0,
            tool_hit_regions: RefCell::new(Vec::new()),
            render_cache: RefCell::new(RenderCache::new()),
            diff_stats_cache: RefCell::new(None),
            remote_host: None,
            wants_animation_frame: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
            scroll_velocity: 0.0,
            last_scroll_tick: std::time::Instant::now(),
            last_user_activity_at: std::time::Instant::now(),
            last_user_interaction_at: std::time::Instant::now(),
            interaction_message_idx: 0,
            away_recap: None,
            idle_return_shown: false,
            json_mode: false,
        };
        app.engine.sync_selected_context_window();
        tracing::info!(
            target: "jfc::app",
            model = %app.engine.model,
            provider = app.engine.provider.name(),
            "App::new"
        );
        app
    }
}

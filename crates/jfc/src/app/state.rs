use std::{cell::RefCell, collections::HashMap, sync::Arc, time::Instant};

use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui_textarea::TextArea;

use crate::render_cache::RenderCache;
use crate::theme::Theme;
use jfc_provider::{ModelId, Provider};

use super::{
    BashPickerState, CommandPaletteState, EngineState, InfoSidebarState, ModelPickerState,
    SessionPickerState, SessionSidebarState, TaskPanelUiState, ThemePickerState,
};

/// Max number of recent RMS audio levels retained for the recording-cursor
/// animation (the CLI's `LWA` ring length).
pub const VOICE_AUDIO_LEVELS_CAP: usize = 16;
pub const VOICE_TTS_TIMINGS_CAP: usize = 256;

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
    /// Anchor/head as `(screen column, absolute content line)`. The line is
    /// scroll-invariant — captured as `scroll_offset + (screen_row − top)` at
    /// mouse time — so the selection **survives scrolling**: each frame the
    /// renderer maps lines back to screen rows and simply skips the parts that
    /// are offscreen. (The old model stored raw screen rows and had to clear
    /// the selection the instant the transcript scrolled.)
    pub anchor: (u16, usize),
    pub head: (u16, usize),
    /// Transcript area width when the selection was anchored. A width change
    /// re-wraps every line and remaps content lines, so the selection is
    /// dropped when this no longer matches (sidebar toggles, terminal resize).
    pub area_width: u16,
    pub dragged: bool,
    pub finalize: bool,
    /// Set once the selection has been extracted + copied. The highlight then
    /// persists (so the user sees what was copied) without re-copying, until
    /// the next mouse-down, Esc, or a width change clears it. Scrolling no
    /// longer clears it — content-line coords stay valid under scroll.
    pub copied: bool,
}

impl TextSelection {
    /// Normalized (top-left, bottom-right) span in reading order, so the
    /// renderer can walk lines top-to-bottom regardless of drag direction.
    pub fn ordered(&self) -> ((u16, usize), (u16, usize)) {
        let (a, h) = (self.anchor, self.head);
        // Order by line, then column.
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

/// A pending prompt-rewrite proposal from the over-refusal gate, awaiting the
/// user's explicit accept/reject/edit. Held in a modal so the rewrite is NEVER
/// applied silently (the SPEC "never silent; require confirmation" contract).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PromptRewriteProposal {
    /// The user's original prompt (sent verbatim on reject).
    pub original: String,
    /// The reworded prompt (sent on accept).
    pub rewrite: String,
    /// Why the rewrite was proposed, shown to the user.
    pub rationale: String,
    /// One-line restatement of the legitimate goal, persisted as a few-shot
    /// exemplar when the user accepts the rewrite (experience replay).
    pub original_intent: String,
}

pub const SPINNER: &[&str] = crate::glyphs::TASK_FRAMES;
pub const IDLE_TICK_MS: u64 = 80;
pub const ANIM_TICK_MS: u64 = 80;

pub struct App {
    /// The frontend-neutral engine state: conversation, streaming,
    /// turn control, approvals, tasks/teams, providers, compaction,
    /// and run configuration. Everything the agentic runtime needs to
    /// execute a turn with no UI present. Moves to the jfc-engine
    /// crate in a later stage of the extraction.
    pub engine: EngineState,
    /// Token-audit dashboard handle. `Some` when the opt-in dashboard server is
    /// running (via `[dashboard]` config); the event loop publishes a fresh
    /// snapshot to it each drained burst. `None` on a default launch.
    pub dashboard: Option<jfc_dashboard::DashboardHandle>,
    /// Per-request token/cost timeline (bounded ring), appended once per
    /// finalized provider request. Serialized into the dashboard snapshot so the
    /// audit panel can chart where input/output tokens go over the session.
    pub timeline: std::collections::VecDeque<jfc_dashboard::TimelineSample>,
    /// Cumulative-usage baseline for computing per-request deltas. Not serialized.
    pub timeline_baseline: crate::runtime::timeline::TimelineBaseline,
    pub theme: Theme,
    pub active_theme_name: String,
    pub plugins_disabled_by_managed_policy: bool,
    pub(crate) plugins: super::plugin_status::PluginUiState,
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
    pub palette: CommandPaletteState,
    pub theme_picker: ThemePickerState,
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
    /// Prompts loaded from previous sessions for cross-session up-arrow /
    /// Ctrl+R history. Populated at startup (async, background) when
    /// `cross_session_history = true` in config. Empty when the feature is
    /// disabled or no prior sessions exist. Oldest-first so that
    /// `user_prompts` can append them after the current session's prompts.
    pub prior_session_prompts: Vec<String>,
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
    /// Bounds of the editable input strip, set on each render. Used by mouse
    /// hit-testing to copy the logical draft text instead of lossy terminal
    /// wrapped cells.
    pub input_rect: std::cell::RefCell<Option<ratatui::layout::Rect>>,
    /// Last known drag-Y, set on each MouseEventKind::Drag event so
    /// the next drag delta can advance scroll_offset by the
    /// difference. Reset on Down / Up so a fresh drag starts cleanly.
    pub drag_anchor_y: Option<u16>,
    /// Drag-edge autoscroll signal. While the left button is held and the
    /// cursor sits at/over the top or bottom edge of the transcript, the mouse
    /// handler records how many rows beyond the edge the cursor is (negative =
    /// above the top edge → scroll up; positive = below the bottom edge →
    /// scroll down). The throttled tick then scrolls in that direction and
    /// extends the selection head, so a drag can select content past the
    /// visible viewport instead of stalling at the edge. `None` when the
    /// cursor is inside the viewport or no drag is active.
    pub drag_autoscroll: Option<i32>,
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
    /// A prompt-rewrite proposal awaiting accept/reject/edit. When `Some`, a
    /// modal is shown and the turn is paused until the user decides.
    pub pending_rewrite_proposal: Option<PromptRewriteProposal>,
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
    pub model_picker: ModelPickerState,
    pub session_picker: SessionPickerState,
    pub bash_picker: BashPickerState,
    pub session_sidebar: SessionSidebarState,
    pub task_panel: TaskPanelUiState,
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
    pub info_sidebar: InfoSidebarState,
    pub leader_key_active: bool,
    pub leader_key_timeout: Option<std::time::Instant>,
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
    /// Current user input state for the front-most MCP elicitation form.
    /// Reset whenever a new elicitation arrives.
    pub elicitation_input: crate::render::elicitation::ElicitationInputState,
    /// Voice mode state — `None` when voice is disabled or not yet activated.
    pub voice_state: jfc_voice::VoiceState,
    /// Latest interim transcript text (shown in the status bar while recording).
    pub voice_interim: Option<String>,
    /// Whether voice mode is configured and available.
    pub voice_enabled: bool,
    /// Char count of the interim transcript currently typed into the input box.
    /// Used to delete the previous interim before inserting an updated one so
    /// live transcription replaces in place rather than appending.
    pub voice_interim_chars: usize,
    /// When the user manually submits (Enter) while a voice utterance is still
    /// in flight, the recorder may keep emitting late `Interim`/`Final` events
    /// for that discarded utterance (it transcribes through finalize). This flag
    /// makes the consumer drop those late events so they don't re-hydrate the
    /// just-cleared input box or auto-submit a duplicate. Cleared on the next
    /// `Recording` onset — the start of a fresh, wanted utterance.
    pub voice_suppress_input: bool,
    /// Ring of the most recent normalized [0,1] RMS audio levels (newest last),
    /// fed by the voice pipeline while recording. Drives the animated recording
    /// cursor. Capped at [`VOICE_AUDIO_LEVELS_CAP`].
    pub voice_audio_levels: Vec<f32>,
    /// Monotonic instant the current recording session began — the time base for
    /// the recording cursor's hue rotation. `None` when not recording.
    pub voice_record_started: Option<std::time::Instant>,
    pub voice_read_aloud_active: bool,
    pub voice_read_aloud_started: Option<std::time::Instant>,
    pub voice_read_aloud_last_stats: Option<(usize, usize)>,
    pub voice_tts_word_timings: Vec<(String, u64)>,
    pub voice_skip_next_stream_read_aloud: bool,
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
    /// Narrower per-frame regions whose click action is "copy this tool's
    /// semantic payload" rather than "toggle the whole tool block". This keeps
    /// command rows selectable/copyable without changing the existing
    /// expand/pin hit region for the surrounding tool.
    pub tool_copy_regions: RefCell<Vec<(String, Rect)>>,
    /// Content-addressed cache for `markdown::to_lines()` output. Keyed on
    /// `(hash(text), width)` so unchanged messages aren't re-parsed on every
    /// frame. Uses `RefCell` because `MessageView` borrows `&App` immutably
    /// during `Widget::render` but needs mutable cache access.
    pub render_cache: RefCell<RenderCache>,
    /// Persistent per-message height index for the virtualized transcript.
    /// Revalidated per frame via cheap fingerprints; only changed messages
    /// re-measure. See `message_view::height_index`. RefCell for the same
    /// reason as `render_cache` — mutated during rendering under `&App`.
    pub height_index: RefCell<crate::message_view::height_index::HeightIndex>,
    /// Cached result of `collect_diff_stats()`. Keyed on
    /// `(messages.len(), total_parts_count)` — invalidates when a message is
    /// appended or a tool result lands. Avoids O(N_messages × N_parts)
    /// HashMap walk per frame; reduces to O(1) lookup on cache hit.
    pub diff_stats_cache: RefCell<Option<(usize, usize, crate::render::DiffStats)>>,
    /// Remote-control host. `Some` when `/remote-control` is active or RC was
    /// started at launch. Events are mirrored to connected clients; client
    /// input is injected into the main event bus. See `crate::remote_host`.
    pub remote_host: Option<std::sync::Arc<jfc_engine::remote_host::RemoteHost>>,
    /// Shared flag: true when the UI needs high-frequency ticks (animations,
    /// boot sweep). The tick task reads this to choose
    /// `ANIM_TICK_MS` vs `IDLE_TICK_MS`.
    pub wants_animation_frame: std::sync::Arc<std::sync::atomic::AtomicBool>,
    /// Reserved for compatibility with older state snapshots. Mouse wheel
    /// scrolling is direct; drag-edge autoscroll has its own state.
    pub scroll_velocity: f32,
    /// Reserved alongside `scroll_velocity`.
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

    /// View-layer reveal cap for live streaming text. The default tick path
    /// reveals all received text immediately (Claude/Codex parity); the cap
    /// exists so the renderer has one stable source for "how much of the live
    /// part may be shown" and tests can exercise optional pacing behavior.
    /// See `crate::render::codex_stream::stream_pacer`.
    pub(crate) stream_pacer: crate::render::codex_stream::stream_pacer::StreamPacer,
    /// `(streaming_assistant_idx, part_count)` the reveal cap currently tracks.
    /// When it changes — a new streaming message, or a new part within it (e.g.
    /// the second text block after a tool call) — the cap resets instead of
    /// inheriting the prior part's count.
    pub(crate) paced_stream_key: Option<(usize, usize)>,
}

impl App {
    pub fn new(provider: Arc<dyn Provider>, model: impl Into<ModelId>) -> Self {
        let mut textarea = TextArea::default();
        textarea.set_cursor_line_style(Style::default());
        // Minimal placeholder — the help overlay and `?` shortcut
        // already document Enter / Shift+Enter; repeating it inline
        // every render was noise. Just a soft prompt.
        textarea.set_placeholder_text("");

        let engine = EngineState::new(provider, model);
        let mut plugin_state =
            super::plugin_status::initial_ui_state(std::path::Path::new(&engine.cwd));
        plugin_state.last_refresh_at = Some(Instant::now());
        let mut app = Self {
            engine,
            dashboard: None,
            timeline: std::collections::VecDeque::new(),
            timeline_baseline: crate::runtime::timeline::TimelineBaseline::default(),
            stream_pacer: crate::render::codex_stream::stream_pacer::StreamPacer::default(),
            paced_stream_key: None,
            theme: Theme::claude(),
            active_theme_name: "claude".to_owned(),
            plugins_disabled_by_managed_policy: false,
            plugins: plugin_state,
            esc_saved_text: None,
            history_cursor: None,
            scroll_offset: 0,
            total_lines: 0,
            total_lines_key: (0, 0, 0),
            textarea,
            vim: None,
            palette: CommandPaletteState::default(),
            theme_picker: ThemePickerState::default(),
            spinner_frame: 0,
            spinner_state: crate::spinner::SpinnerState::new(std::time::Instant::now()),
            reasoning_expanded: HashMap::new(),
            tool_group_expanded: std::collections::HashSet::new(),
            transcript_search: None,
            prompt_search: None,
            prior_session_prompts: Vec::new(),
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
            input_rect: std::cell::RefCell::new(None),
            drag_anchor_y: None,
            drag_autoscroll: None,
            text_selection: None,
            last_click: None,
            pending_select_request: None,
            pending_rewrite_proposal: None,
            last_focus_hint_at: None,
            last_esc_at: None,
            follow_bottom: true,
            viewport_height: 0,
            input_wrap_width: 1,
            model_picker: ModelPickerState::default(),
            session_picker: SessionPickerState::default(),
            bash_picker: BashPickerState::default(),
            session_sidebar: SessionSidebarState::default(),
            task_panel: TaskPanelUiState::default(),
            mention: crate::mentions::MentionState::default(),
            mention_all_files: Vec::new(),
            show_diagnostic_panel: false,
            diagnostic_panel_scroll: 0,
            launched_at: std::time::Instant::now(),
            delivered_diagnostics: std::collections::HashSet::new(),
            last_keybindings_watcher_seen: 0,
            verbose_mode: false,
            info_sidebar: InfoSidebarState::default(),
            leader_key_active: false,
            leader_key_timeout: None,
            pasted_images: Vec::new(),
            pasted_texts: Vec::new(),
            paste_counter: 0,
            image_counter: 0,
            elicitation_input: crate::render::elicitation::ElicitationInputState::default(),
            voice_state: jfc_voice::VoiceState::Idle,
            voice_interim: None,
            voice_enabled: false,
            voice_interim_chars: 0,
            voice_suppress_input: false,
            voice_audio_levels: Vec::new(),
            voice_record_started: None,
            voice_read_aloud_active: false,
            voice_read_aloud_started: None,
            voice_read_aloud_last_stats: None,
            voice_tts_word_timings: Vec::new(),
            voice_skip_next_stream_read_aloud: false,
            tool_hit_regions: RefCell::new(Vec::new()),
            tool_copy_regions: RefCell::new(Vec::new()),
            render_cache: RefCell::new(RenderCache::new()),
            height_index: RefCell::new(crate::message_view::height_index::HeightIndex::new()),
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

    pub(crate) fn reload_plugin_status_fresh(&mut self) -> bool {
        self.plugins.reload_report = None;
        self.refresh_plugin_status()
    }

    pub(crate) fn refresh_plugin_status(&mut self) -> bool {
        let project_root = std::path::Path::new(&self.engine.cwd);
        let Some(mut next) = super::plugin_status::refresh_ui_state(
            project_root,
            self.plugins.reload_report.as_ref(),
        ) else {
            return false;
        };
        next.preserve_ui_widget_snapshots_from(&self.plugins);
        let refreshed_at = Some(Instant::now());
        let changed = self.plugins.health != next.health
            || self.plugins.ui_slots != next.ui_slots
            || self.plugins.ui_panel_descriptors != next.ui_panel_descriptors
            || self.plugins.ui_panel_snapshots != next.ui_panel_snapshots
            || self.plugins.ui_panel_refresh_status != next.ui_panel_refresh_status
            || self.plugins.ui_widget_descriptors != next.ui_widget_descriptors
            || self.plugins.ui_widget_snapshots != next.ui_widget_snapshots
            || self.plugins.ui_widget_refresh_status != next.ui_widget_refresh_status
            || self.plugins.metric_descriptors != next.metric_descriptors
            || self.plugins.runtime_action_descriptors != next.runtime_action_descriptors
            || self.plugins.runtime_extension_descriptors != next.runtime_extension_descriptors
            || self.plugins.reload_report != next.reload_report;
        if changed {
            self.plugins = next;
        }
        self.plugins.last_refresh_at = refreshed_at;
        changed
    }
}

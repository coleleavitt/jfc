use std::cell::RefCell;
use std::collections::HashMap;

use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Paragraph, Widget, Wrap},
};

use crate::app::App;
use crate::markdown;
use crate::theme::Theme;
use jfc_core::*;

use super::assistant_parts::{push_advisor_lines, push_reasoning_lines, push_task_status_lines};
use super::tool_blocks::{render_tool_block, tool_kind_color};
use super::tool_height::tool_block_height;

/// Columns reserved at the left of a user message for the `▌` ribbon + a
/// space. User text is *wrapped* this much narrower (in the item builder)
/// and then *rendered* shifted right by the same amount, so the two widths
/// always agree — no re-wrap, no right-edge clip.
pub const MSG_USER_INDENT: u16 = 2;

pub struct MessageView<'a> {
    pub app: &'a App,
    /// Optional precomputed render items + total height. Lets `render::messages`
    /// build the items vec once per frame and share it with `MessageView::render`,
    /// avoiding a second `build_render_items` walk that — pre-cache — was the
    /// dominant remaining hot spot (gdb showed `Vec<Line<'static>>::to_vec` from
    /// the streaming-text cache hit at message_view.rs:607). `None` falls back
    /// to the legacy "MessageView builds its own" path used by tests + any
    /// caller that hasn't been threaded through.
    pub prebuilt: Option<PrebuiltItems<'a>>,
}

pub struct PrebuiltItems<'a> {
    pub items: Vec<RenderItem<'a>>,
    pub total_h: usize,
    /// Pre-clamped scroll offset, computed by the caller before rendering.
    /// `MessageView::render` uses this in place of reading `app.scroll_offset`
    /// directly so the caller can hold a shared `&App` (via `items`) and a
    /// pending mutation to `scroll_offset` at the same time without tripping
    /// the borrow checker.
    pub scroll: usize,
}

/// Rendering context — carries exactly what the item-builder needs from the
/// app so the same function serves both the main chat and the task view.
pub struct RenderCtx<'a> {
    pub messages: &'a [jfc_core::ChatMessage],
    pub streaming_idx: Option<usize>,
    pub is_streaming: bool,
    pub reasoning_expanded: &'a HashMap<usize, bool>,
    /// Index of the message whose reasoning is *actively streaming* this turn.
    /// Its thinking block defaults to expanded (live preview); every other,
    /// completed reasoning block defaults to collapsed so finished turns don't
    /// pile full thinking transcripts on screen. The `reasoning_expanded` map
    /// still overrides this per-message when the user toggles with ctrl+o.
    /// Mirrors Claude Code: expand streaming thinking, collapse once done.
    pub active_reasoning_idx: Option<usize>,
    pub tool_group_expanded: &'a std::collections::HashSet<String>,
    pub render_cache: &'a RefCell<crate::render_cache::RenderCache>,
    pub theme: crate::theme::Theme,
    /// Brief mode: when true, plain `MessagePart::Text` parts on assistant
    /// messages are suppressed from rendering so only `SendUserMessage` tool
    /// output reaches the user. Mirrors Claude Code v2.1.142+ `brief_mode`.
    pub brief_mode: bool,
    /// While streaming, how many display segments of the live message's last
    /// (actively-accruing) text part the pacer has revealed this frame. The
    /// renderer truncates that part's source to this many lines so output
    /// animates in at the adaptive cadence. `None` off the streaming path
    /// (and in the task view) — no truncation. See `codex_stream::stream_pacer`.
    pub revealed_streaming_lines: Option<usize>,
}

impl<'a> RenderCtx<'a> {
    /// Main-chat path: pull everything from the live `App`.
    pub fn from_app(app: &'a App) -> Self {
        Self {
            messages: &app.engine.messages,
            streaming_idx: app.engine.streaming_assistant_idx,
            is_streaming: app.engine.is_streaming,
            reasoning_expanded: &app.reasoning_expanded,
            active_reasoning_idx: {
                // Only the live, still-thinking block defaults expanded.
                let thinking_live = app.engine.thinking_started_at.is_some()
                    && app.engine.thinking_ended_at.is_none();
                if thinking_live {
                    app.engine.streaming_assistant_idx
                } else {
                    None
                }
            },
            tool_group_expanded: &app.tool_group_expanded,
            render_cache: &app.render_cache,
            theme: app.theme,
            brief_mode: app.engine.brief_mode
                || jfc_engine::feature_gates::pewter_owl_brief_enabled(
                    app.engine.model.as_str(),
                    false,
                ),
            // The pacer is advanced in the tick handler; here we only read the
            // current revealed count. Off the streaming path there's nothing to
            // pace, so leave it `None` (full render).
            revealed_streaming_lines: if app.engine.is_streaming {
                Some(app.stream_pacer.revealed())
            } else {
                None
            },
        }
    }

    /// Task-view path: render `messages` with no streaming state, no
    /// reasoning expansion, no diagnostics.
    pub fn from_task(messages: &'a [jfc_core::ChatMessage], app: &'a App) -> Self {
        static EMPTY_REASONING: std::sync::OnceLock<HashMap<usize, bool>> =
            std::sync::OnceLock::new();
        static EMPTY_GROUPS: std::sync::OnceLock<std::collections::HashSet<String>> =
            std::sync::OnceLock::new();
        Self {
            messages,
            streaming_idx: None,
            is_streaming: false,
            reasoning_expanded: EMPTY_REASONING.get_or_init(HashMap::new),
            active_reasoning_idx: None,
            tool_group_expanded: EMPTY_GROUPS.get_or_init(std::collections::HashSet::new),
            render_cache: &app.render_cache,
            theme: app.theme,
            brief_mode: false,
            revealed_streaming_lines: None,
        }
    }
}

/// Single canonical item-builder used by both views. Takes a `RenderCtx`
/// instead of `&App` so it works for any message slice.
pub fn build_render_items_ctx<'a>(ctx: &'a RenderCtx<'_>, inner_w: usize) -> Vec<RenderItem<'a>> {
    build_render_items_inner(ctx, inner_w)
}

/// Public entry to `build_render_items` for callers in sibling modules
/// (`render::messages`) that want to share one items vec with the widget.
/// Takes `ctx` directly so the caller controls the lifetime — the returned
/// items borrow from `ctx.messages` which must outlive the items vec.
pub fn build_render_items_pub<'a>(ctx: &'a RenderCtx<'a>, inner_w: usize) -> Vec<RenderItem<'a>> {
    build_render_items_inner(ctx, inner_w)
}

/// Windowed builder for the virtualized transcript: items for messages
/// `[first, last)` only. `prev_role` must be the threading value after
/// message `first - 1` (see `HeightIndex::prev_role_before`) so the
/// same-speaker label suppression matches what a full walk would produce.
pub fn build_render_items_window<'a>(
    ctx: &'a RenderCtx<'a>,
    inner_w: usize,
    first: usize,
    last: usize,
    mut prev_role: Option<Role>,
) -> Vec<RenderItem<'a>> {
    let mut items = Vec::new();
    for (idx, msg) in ctx.messages.iter().enumerate().take(last).skip(first) {
        build_message_items(ctx, idx, msg, &mut prev_role, inner_w, &mut items);
    }
    items
}

/// Total visual rows the message view will draw at this width.
///
/// **One producer, one truth.** This used to be a parallel
/// implementation that walked the message tree and summed predicted
/// heights — and every change to rendering quietly drifted from it.
/// We hit the bug class four times (TaskStatus markdown, Reasoning
/// expanded wide, Advisor byte-counted char-wrap, narrow
/// CompactBoundary) before unifying. The rustc query system /
/// MIR-pass community calls this single-source-of-truth pattern
/// "query feeding": compute once, derive any view from the canonical
/// artifact (the produced `RenderItem` vec), never reimplement the
/// computation in a sibling code path.
///
/// Cost: one extra `build_render_items` per frame. Acceptable —
/// markdown rendering is cached in `RenderCache`, the rest is
/// O(parts). The previous "fast-path predictor" was a premature
/// optimization that traded ~ms per frame for permanent drift bugs.
pub fn message_view_total_lines(app: &App, inner_w: usize) -> usize {
    build_render_items_inner(&RenderCtx::from_app(app), inner_w)
        .iter()
        .map(|i| i.height(inner_w))
        .sum()
}

impl Widget for MessageView<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let t = self.app.theme;
        let width = area.width;
        let inner_w = width as usize;

        // Build ctx before the match so it lives long enough for items to borrow from it.
        let fallback_ctx = RenderCtx::from_app(self.app);
        let (items, total_h, prebuilt_scroll) = match self.prebuilt {
            Some(p) => (p.items, p.total_h, Some(p.scroll)),
            None => {
                let items = build_render_items_inner(&fallback_ctx, inner_w);
                let total_h: usize = items.iter().map(|i| i.height(inner_w)).sum();
                (items, total_h, None)
            }
        };
        let max_scroll = total_h.saturating_sub(area.height as usize);
        let scroll = prebuilt_scroll
            .unwrap_or(self.app.scroll_offset)
            .min(max_scroll);

        // Frame-level diagnostics for chasing scroll/overflow drift —
        // e.g., a content row that visibly clips at the viewport's
        // bottom edge despite scroll math saying everything fits.
        // `RUST_LOG=jfc::render::scroll=trace` lights these up.
        //
        // At `trace`, not `debug`: the default filter is `debug`, and this
        // fires twice per frame (begin/end) at up to 30 FPS while streaming.
        // Each call formats ~10 fields and writes a line to disk, which
        // showed up in profiling as a per-frame malloc/realloc + xfs write
        // churn on the render hot path. Trace keeps it opt-in.
        tracing::trace!(
            target: "jfc::render::scroll",
            n_items = items.len(),
            n_messages = self.app.engine.messages.len(),
            inner_w,
            viewport_h = area.height,
            total_h = total_h,
            scroll_offset_raw = self.app.scroll_offset,
            scroll_offset_clamped = scroll,
            max_scroll,
            "MessageView::render begin"
        );

        let mut lines_skipped: usize = 0;
        let mut y = area.y;
        let bottom = area.y + area.height;
        // Diagnostics: track which item index landed at top + bottom of
        // the viewport, plus the absolute content-line range that's
        // visible. Logged once at the end so per-item spam stays at
        // trace level.
        let mut first_visible_item: Option<usize> = None;
        let mut last_visible_item: Option<usize> = None;
        let mut first_visible_line: Option<usize> = None;
        let mut last_visible_line: Option<usize> = None;
        let mut last_y_drawn: u16 = area.y;

        // Active message scope. Used only to know when to drop a
        // pulsing typing cursor `▋` after the most-recent streamed
        // text row (see MessageEnd handling below). The earlier
        // full-height gutter + bg tint painting have been removed —
        // color + bold on the role label is the entire visual
        // differentiation, no per-row decoration.
        struct Scope {
            role: Role,
            is_streaming_placeholder: bool,
        }
        let mut scope: Option<Scope> = None;
        // Track the last row + content end-column drawn inside a
        // streaming-placeholder scope so we can drop a pulsing typing
        // cursor `▋` there at MessageEnd. Without this, the
        // streaming text just stops dead at the right edge of the
        // last char — the user has no inline cue that more text is
        // coming. The cursor pulses on the same 1.2s clock as the
        // gutter so the two signals reinforce each other.
        let mut last_streaming_cursor: Option<(u16, u16, u16)> = None;

        for (item_idx, item) in items.iter().enumerate() {
            if y >= bottom {
                tracing::trace!(
                    target: "jfc::render::scroll",
                    stopped_at_item = item_idx,
                    total_items = items.len(),
                    y, bottom,
                    "MessageView::render hit viewport bottom — items beyond this are clipped"
                );
                break;
            }

            // Scope markers update the active draw context but emit
            // no rows. Process them inline so the actual draw stays
            // simple.
            match item {
                RenderItem::MessageStart {
                    role,
                    is_streaming_placeholder,
                } => {
                    scope = Some(Scope {
                        role: *role,
                        is_streaming_placeholder: *is_streaming_placeholder,
                    });
                    last_streaming_cursor = None;
                    continue;
                }
                RenderItem::MessageEnd => {
                    // If we were rendering a streaming placeholder
                    // and tracked a "last drawn row", drop a pulsing
                    // typing cursor there now. Reduced-motion skips
                    // the cursor entirely — the gutter already gives
                    // a static "this message is in flight" signal.
                    if let Some((cx, cy, _w)) = last_streaming_cursor.take()
                        && cx < buf.area().right()
                        && cy < buf.area().bottom()
                    {
                        // Static accent caret marking where text is landing.
                        // No blink/pulse — it's a position cue, not a
                        // liveness animation (the spinner row covers that).
                        let cell = &mut buf[(cx, cy)];
                        cell.set_symbol("▋");
                        cell.set_style(Style::default().fg(t.accent));
                    }
                    scope = None;
                    continue;
                }
                _ => {}
            }

            let h = item.height(inner_w);
            if lines_skipped + h <= scroll {
                lines_skipped += h;
                continue;
            }
            let item_scroll_skip = scroll.saturating_sub(lines_skipped);
            let visible_h = h.saturating_sub(item_scroll_skip);
            let render_h = (visible_h as u16).min(bottom - y);
            if render_h == 0 {
                lines_skipped += h;
                continue;
            }
            // Track viewport boundaries for the once-per-frame summary.
            if first_visible_item.is_none() {
                first_visible_item = Some(item_idx);
                first_visible_line = Some(lines_skipped + item_scroll_skip);
            }
            last_visible_item = Some(item_idx);
            last_visible_line = Some(lines_skipped + item_scroll_skip + render_h as usize);

            // User messages get a flat accent ribbon (`▌`) down the left
            // and a 2-col indent so the speaker reads at a glance without
            // any background fill (terminals band bg gradients badly, and
            // a solid tint is the decoration we keep stripping). Assistant
            // and everything else render flush-left at full width.
            let user_ribbon = matches!(
                scope,
                Some(Scope {
                    role: Role::User,
                    ..
                })
            );
            let (item_x, item_w) = if user_ribbon {
                (
                    area.x + MSG_USER_INDENT,
                    width.saturating_sub(MSG_USER_INDENT),
                )
            } else {
                (area.x, width)
            };
            let item_area = Rect {
                x: item_x,
                y,
                width: item_w,
                height: render_h,
            };
            // Hit-region: each clickable item registers its area so
            // mouse handler can hit-test. Both individual tool blocks
            // and collapsed groups participate. Group keys are
            // prefixed with `group:` so the click handler can tell
            // them apart from raw tool ids when toggling state.
            match item {
                RenderItem::ToolBlock(tool) => {
                    self.app
                        .tool_hit_regions
                        .borrow_mut()
                        .push((tool.id.as_str().to_owned(), item_area));
                }
                RenderItem::ToolGroup { key, .. } => {
                    self.app
                        .tool_hit_regions
                        .borrow_mut()
                        .push((format!("group:{key}"), item_area));
                }
                _ => {}
            }
            item.render_with_skip(self.app, item_area, buf, t, item_scroll_skip);

            // Paint the user-message ribbon: a flat `▌` in accent at the
            // left edge of every row this item occupies. Static (no pulse)
            // — it's a structural speaker marker, not a liveness animation.
            if user_ribbon && area.x < buf.area().right() {
                let ribbon_bottom = (y + render_h).min(buf.area().bottom());
                for ry in y..ribbon_bottom {
                    let cell = &mut buf[(area.x, ry)];
                    cell.set_symbol("▌");
                    cell.set_style(Style::default().fg(t.accent));
                }
            }

            // For streaming-placeholder scopes, remember the bottom
            // row's last-content column so MessageEnd can drop a
            // typing cursor right after the most-recent char. We scan
            // the bottom row of the just-rendered area for the
            // rightmost non-space cell, then bump x by 1 so the
            // cursor sits in the cell immediately after the text.
            if let Some(s) = &scope
                && s.is_streaming_placeholder
            {
                let last_y = y + render_h.saturating_sub(1);
                if last_y < buf.area().bottom() {
                    let row_left = item_area.x;
                    let row_right = (item_area.x + item_area.width).min(buf.area().right());
                    let mut last_content_x: Option<u16> = None;
                    let mut x_pos = row_left;
                    while x_pos < row_right {
                        let cell = &buf[(x_pos, last_y)];
                        if cell.symbol() != " " && !cell.symbol().is_empty() {
                            last_content_x = Some(x_pos);
                        }
                        x_pos += 1;
                    }
                    if let Some(lx) = last_content_x {
                        let cursor_x = (lx + 1).min(row_right.saturating_sub(1));
                        last_streaming_cursor = Some((cursor_x, last_y, render_h));
                    }
                }
            }

            // No per-row decoration — the role-label color + bold
            // is the entire message-block identity. (Gutter +
            // bg-tint painting used to live here; both removed per
            // the user's "those blue lines look dumb" feedback.)

            y += render_h;
            last_y_drawn = y;
            lines_skipped += h;
        }

        // End-of-frame snapshot: where the scroll math actually
        // landed. Compare `last_visible_line` vs `total_h`
        // to know whether the content tail is on-screen, and
        // `last_y_drawn` vs `bottom` to spot a viewport-bottom gap.
        // Pair this with the `MessageView::render begin` log to
        // diagnose "I see line N but expected line M at the bottom"
        // class bugs without instrumenting every layer.
        let content_at_bottom = last_visible_line.map(|l| l >= total_h).unwrap_or(false);
        // `trace`, not `debug`: see the begin-log rationale above — this is
        // the second of two per-frame events on the render hot path.
        tracing::trace!(
            target: "jfc::render::scroll",
            first_visible_item = ?first_visible_item,
            last_visible_item = ?last_visible_item,
            first_visible_line = ?first_visible_line,
            last_visible_line = ?last_visible_line,
            last_y_drawn,
            viewport_bottom = bottom,
            viewport_gap_rows = bottom.saturating_sub(last_y_drawn),
            content_tail_visible = content_at_bottom,
            total_h,
            "MessageView::render end"
        );
    }
}

pub enum RenderItem<'a> {
    TextLine(Line<'a>),
    /// Carries `&App` so the renderer can read `ctx.diagnostics`
    /// when rendering a Read result — without piping the whole App
    /// through the render-stack as a separate parameter at every
    /// helper. Only the tool-block path needs it; other items don't.
    /// Single tool block. We carry only the `&ToolCall` reference (not `&App`)
    /// so the items Vec borrows just `app.engine.messages` instead of the whole `App` —
    /// that lets `render::messages` mutate sibling fields like `scroll_offset`,
    /// `total_lines`, and `viewport_height` while the prebuilt items are still
    /// alive. Pre-fix the variant held `&App` and split-borrow rules forced
    /// `render::messages` to either rebuild items twice per frame or defer all
    /// scroll math, neither of which composed cleanly.
    ToolBlock(&'a ToolCall),
    /// Collapsed group of consecutive same-kind tool calls (Read,
    /// Glob, Grep, Search). Renders as a single one-line teaser
    /// "▶ N reads · click to expand"; click on the row or `o` flips
    /// `ctx.tool_group_expanded` and the next render emits each
    /// tool individually.
    ToolGroup {
        key: String,
        kind_label: String,
        count: usize,
        kind_color: ratatui::style::Color,
    },
    Blank,
    /// Zero-height scope markers: bracket all the items belonging to
    /// a single chat message so the renderer can paint a full-height
    /// gutter glyph and (for assistant messages) a subtle bg tint
    /// down the entire range. Without these markers the renderer
    /// has no idea where one message ends and the next begins; it
    /// just sees a flat stream of TextLine / ToolBlock / Blank.
    MessageStart {
        role: Role,
        is_streaming_placeholder: bool,
    },
    MessageEnd,
}

impl<'a> RenderItem<'a> {
    pub fn height(&self, width: usize) -> usize {
        match self {
            RenderItem::Blank => 1,
            RenderItem::TextLine(line) => {
                let line_w = line.width();
                if line_w == 0 || width == 0 {
                    1
                } else if line_w <= width {
                    // Fast path: line fits within the available width —
                    // no word-wrapping needed. This skips constructing a
                    // Paragraph + running WordWrapper for the ~80% of
                    // lines that are shorter than the terminal width.
                    1
                } else {
                    // Slow path: line wraps. Use ratatui's actual
                    // word-wrap count for accuracy.
                    use ratatui::widgets::{Paragraph, Wrap};
                    let p = Paragraph::new(line.clone()).wrap(Wrap { trim: false });
                    p.line_count(width as u16).max(1)
                }
            }
            RenderItem::ToolBlock(tool) => tool_block_height(tool, width),
            RenderItem::ToolGroup { .. } => 1,
            // Scope markers occupy no rows — they only affect the
            // surrounding draw context (gutter color, bg tint).
            RenderItem::MessageStart { .. } | RenderItem::MessageEnd => 0,
        }
    }

    fn render_with_skip(&self, app: &App, area: Rect, buf: &mut Buffer, t: Theme, skip: usize) {
        match self {
            RenderItem::MessageStart { .. } | RenderItem::MessageEnd => {}
            RenderItem::Blank => {}
            RenderItem::TextLine(line) => {
                Paragraph::new(line.clone())
                    .wrap(Wrap { trim: false })
                    .scroll((skip as u16, 0))
                    .style(Style::default().bg(t.bg))
                    .render(area, buf);
            }
            RenderItem::ToolBlock(tool) => {
                render_tool_block(app, tool, area, t, buf, skip);
            }
            RenderItem::ToolGroup {
                kind_label,
                count,
                kind_color,
                ..
            } => {
                if skip > 0 || area.height == 0 {
                    return;
                }
                // No leading gutter glyph — the `▶ N reads ·` text
                // already reads as a teaser. The `▶` triangle marks
                // it as expandable; kind color goes on the count
                // text. Same simplification as the other tool paths.
                let plural = if *count == 1 {
                    kind_label.clone()
                } else {
                    format!("{kind_label}s")
                };
                let row = Rect {
                    x: area.x,
                    y: area.y,
                    width: area.width,
                    height: 1,
                };
                Paragraph::new(Line::from(vec![
                    Span::styled(
                        format!("{count} {plural}"),
                        Style::default()
                            .fg(*kind_color)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(
                        " · click or press o to expand".to_string(),
                        Style::default()
                            .fg(t.text_muted)
                            .add_modifier(Modifier::ITALIC),
                    ),
                ]))
                .style(Style::default().bg(t.bg))
                .render(row, buf);
            }
        }
    }
}

/// Build a `HashMap<line_number, Severity>` for the file path
/// referenced by `input` (Read/Edit/Write), pulling from
/// `ctx.diagnostics`. Returns an empty map when the input isn't a
/// file-tool or no diagnostics match. The lookup uses the basename
/// when the diagnostic stores a relative path that doesn't match
/// the absolute one from the tool input — robust against either
/// representation showing up.
pub(super) fn diagnostics_for_path(
    app: &App,
    input: &ToolInput,
) -> std::collections::HashMap<usize, jfc_engine::diagnostics::Severity> {
    use std::collections::HashMap;
    let mut out: HashMap<usize, jfc_engine::diagnostics::Severity> = HashMap::new();
    let path = match input {
        ToolInput::Read { file_path, .. }
        | ToolInput::Edit { file_path, .. }
        | ToolInput::Write { file_path, .. } => file_path.as_str(),
        _ => return out,
    };
    let path_basename = std::path::Path::new(path)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or(path);
    for d in &app.engine.diagnostics {
        let d_basename = std::path::Path::new(&d.file)
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or(d.file.as_str());
        let same_path = d.file == path
            || d_basename == path_basename
            || path.ends_with(d.file.as_str())
            || d.file.ends_with(path);
        if !same_path {
            continue;
        }
        let entry = out.entry(d.line as usize).or_insert(d.severity);
        // Higher severity wins when several diagnostics target the
        // same line.
        if severity_rank(d.severity) > severity_rank(*entry) {
            *entry = d.severity;
        }
    }
    out
}

pub(super) fn severity_rank(s: jfc_engine::diagnostics::Severity) -> u8 {
    use jfc_engine::diagnostics::Severity;
    match s {
        Severity::Error => 4,
        Severity::Warning => 3,
        Severity::Info => 2,
        Severity::Hint => 1,
    }
}

/// Tool kinds that get auto-grouped when the model fires several in a
/// row. These are the "search-pattern" kinds — running 5 Reads or 5
/// Greps individually drowns out the next user prompt; collapsing
/// them keeps the transcript scannable while preserving the option
/// to drill in. Edit/Write/Bash never group because each one's
/// behavior matters per-call.
pub(super) fn is_groupable(kind: &ToolKind) -> bool {
    matches!(
        kind,
        ToolKind::Read | ToolKind::Glob | ToolKind::Grep | ToolKind::Search
    )
}

/// Tool kinds that produce NO visible widget in the transcript.
///
/// Mirrors CC 2.1.167's `renderToolUseMessage() { return null; }` pattern.
/// These tools are structural/meta — task management, scheduling, internal
/// search — and showing their call widgets drowns the real work. The model
/// still sees the `tool_result` text; only the TUI widget is suppressed.
///
/// Users can override by setting `JFC_SHOW_ALL_TOOLS=1`.
pub(super) fn is_invisible_in_transcript(kind: &ToolKind) -> bool {
    if std::env::var("JFC_SHOW_ALL_TOOLS").is_ok() {
        return false;
    }
    matches!(
        kind,
        // Subagent spawn — every Task spawn also emits a TaskEvent that adds a
        // dedicated `MessagePart::TaskStatus` (the green ●/spinner row). Rendering
        // the Task tool block too produces TWO icons for one task (the rose Task
        // tool glyph next to the TaskStatus dot). The TaskStatus row is the
        // canonical render, so the raw Task tool block is hidden here.
        ToolKind::Task
            // Task management — CC hides all of these
            | ToolKind::TaskCreate
            | ToolKind::TaskUpdate
            | ToolKind::TaskList
            | ToolKind::TaskGet
            | ToolKind::TaskDone
            | ToolKind::TaskStop
            | ToolKind::TaskValidate
            // Scheduling / cron
            | ToolKind::ScheduleWakeup
            | ToolKind::CronCreate
            | ToolKind::CronList
            | ToolKind::CronDelete
    )
}

/// Remove every `<system-reminder>…</system-reminder>` block from `s`. Used to
/// tell whether a user turn has any real (user-authored) content left.
fn strip_system_reminders(s: &str) -> String {
    const OPEN: &str = "<system-reminder>";
    const CLOSE: &str = "</system-reminder>";
    let mut out = String::new();
    let mut rest = s;
    while let Some(start) = rest.find(OPEN) {
        out.push_str(&rest[..start]);
        if let Some(end) = rest[start..].find(CLOSE) {
            rest = &rest[start + end + CLOSE.len()..];
        } else {
            // Unterminated tag — drop the opener AND everything after it.
            // A reminder block that never closes is a system nudge whose
            // payload is still reminder content, not something the user
            // typed; keeping the tail would leak it as a fake "you" bubble.
            rest = "";
            break;
        }
    }
    out.push_str(rest);
    out
}

/// True when a user message would render as a bare empty "you" bubble — its
/// only parts are text, and after stripping `<system-reminder>` blocks nothing
/// the user typed remains. Catches the auto-continuation nudge turns. A turn
/// with any non-text part (attachment, etc.) is real content and returns false.
fn is_reminder_only_user(msg: &ChatMessage) -> bool {
    if !msg.parts.iter().all(|p| matches!(p, MessagePart::Text(_))) {
        return false;
    }
    let joined: String = msg
        .parts
        .iter()
        .filter_map(|p| match p {
            MessagePart::Text(t) => Some(t.as_str()),
            _ => None,
        })
        .collect();
    strip_system_reminders(&joined).trim().is_empty()
}

fn build_render_items_inner<'a>(ctx: &'a RenderCtx<'_>, inner_w: usize) -> Vec<RenderItem<'a>> {
    let mut items: Vec<RenderItem<'a>> = Vec::new();
    // Tracks the previous *rendered* message's role so a run of
    // consecutive same-speaker messages doesn't repeat the label on every
    // one — the replayed-agent transcript splits each prose block and tool
    // call into its own assistant message, which otherwise stamps
    // "assistant" 6+ times in a row. Suppressed only for non-streaming
    // assistant continuations (the streaming placeholder keeps its
    // pulsing dot label).
    let mut prev_role: Option<Role> = None;
    for (idx, msg) in ctx.messages.iter().enumerate() {
        build_message_items(ctx, idx, msg, &mut prev_role, inner_w, &mut items);
    }
    items
}

/// Build the render items for a SINGLE message. Extracted from the
/// all-messages walk so the virtualized transcript path
/// (`message_view::height_index` + `render::messages`) can build items for
/// only the messages intersecting the visible window — per-frame work
/// becomes O(window), not O(transcript). `prev_role` threads the
/// consecutive-same-speaker label suppression across calls; it is only
/// updated when the message actually renders (skipped messages leave it
/// untouched), matching the original single-loop behavior.
pub(crate) fn build_message_items<'a>(
    ctx: &'a RenderCtx<'_>,
    idx: usize,
    msg: &'a jfc_core::ChatMessage,
    prev_role: &mut Option<Role>,
    inner_w: usize,
    items: &mut Vec<RenderItem<'a>>,
) {
    let t = ctx.theme;
    {
        // The streaming-placeholder assistant message gets mutated in place
        // by the StreamChunk handler — text/reasoning chunks append to its
        // parts as they arrive. We render it inline like any other message
        // so the user sees content arriving in the chat timeline (rather
        // than a duplicate "assistant" header pinned to the bottom). When
        // the placeholder still has no content (parts are all empty Text /
        // empty Reasoning), skip it so we don't show a label with nothing
        // under it — the dedicated spinner row above the input is the
        // visual cue that work is in flight.
        let is_streaming_placeholder = ctx.streaming_idx == Some(idx) && ctx.is_streaming;
        if is_streaming_placeholder {
            let has_content = msg.parts.iter().any(|p| match p {
                MessagePart::Text(s) => !s.is_empty(),
                MessagePart::Reasoning(s) => !s.is_empty(),
                _ => true,
            });
            if !has_content {
                return;
            }
        }

        // Auto-continuation nudges (goal loop, dynamic keepalive, background
        // reminders) push a USER message whose only content is a
        // `<system-reminder>` block. The markdown renderer drops that as a raw
        // HTML block, leaving a bare empty "you" bubble in the transcript
        // ("random empty user inputs"). Skip reminder-only (or otherwise
        // empty-display) user turns — they're internal context, not something
        // the user typed. Real prompts (text, with reminders appended) and
        // attachment-only turns still render.
        if msg.role == Role::User && is_reminder_only_user(msg) {
            return;
        }

        // Role label gets a colored gutter glyph (`▎`) prefix so the
        // start of each message is anchored visually instead of being
        // an unframed text fragment. While the assistant is streaming,
        // the gutter pulses accent ↔ border on the same 1.2s clock as
        // the main spinner so the in-flight message reads as alive
        // even if no chars have arrived yet.
        // MessageStart/MessageEnd still bracket the scope so the
        // render loop can drop the typing-cursor `▋` after the last
        // text row of a streaming placeholder. The gutter painting
        // and bg tint that used to ride along with this scope have
        // been removed — the role label's color + bold is the only
        // visual differentiation now (matches the sidebar's bare
        // colored-bold-headers convention the user wanted).
        items.push(RenderItem::MessageStart {
            role: msg.role,
            is_streaming_placeholder,
        });
        // Suppress the repeated label for a same-speaker continuation.
        let suppress_label = *prev_role == Some(msg.role)
            && matches!(msg.role, Role::Assistant)
            && !is_streaming_placeholder;
        *prev_role = Some(msg.role);
        let label_line = match msg.role {
            // Queued user message (pending submit): dim the role label
            // and append "[queued]" so it visually reads as pending vs
            // already-processed turns. Mirrors CC 2.1.144's queued-
            // message rendering where pending input is muted to
            // distinguish it from in-flight or completed turns
            // (cli.beautified.js:501634 hints "Press up to edit
            // queued messages" surfaces when queued items exist).
            Role::User if msg.queued => Line::from(vec![
                Span::styled("you", t.user_label().add_modifier(Modifier::DIM)),
                Span::styled(
                    " [queued]",
                    Style::default()
                        .fg(t.text_muted)
                        .add_modifier(Modifier::ITALIC),
                ),
            ]),
            Role::User => Line::from(Span::styled("you", t.user_label())),
            Role::Assistant => {
                let mut spans = Vec::new();
                // Static accent dot marks *which* message is in flight. No
                // pulse — the spinner row already carries the live activity
                // signal; a second pulsing element just competed with it.
                if is_streaming_placeholder {
                    spans.push(Span::styled("● ", Style::default().fg(t.accent)));
                }
                spans.push(Span::styled("assistant", t.asst_label()));
                Line::from(spans)
            }
        };
        if !suppress_label {
            items.push(RenderItem::TextLine(label_line));
        }

        // Default: expanded only while this block is the actively-streaming
        // reasoning; completed thinking collapses to the one-line summary
        // (ctrl+o re-expands via `reasoning_expanded`). Keeps finished turns
        // from stacking full thinking transcripts on screen.
        let reasoning_expanded = ctx
            .reasoning_expanded
            .get(&idx)
            .copied()
            .unwrap_or(ctx.active_reasoning_idx == Some(idx));

        // Walk parts with peek-ahead so consecutive groupable tools
        // (Read/Glob/Grep) collapse into a single ToolGroup row when
        // the user hasn't expanded the group. Min run length is 3 —
        // 1–2 tools render fine individually and grouping them just
        // adds an extra click.
        const MIN_GROUP_LEN: usize = 3;
        let mut p = 0usize;
        while p < msg.parts.len() {
            let part = &msg.parts[p];
            match part {
                MessagePart::Tool(first_tool)
                    if is_groupable(&first_tool.kind)
                        && !is_invisible_in_transcript(&first_tool.kind) =>
                {
                    // Probe forward for consecutive same-kind tools.
                    let mut run_end = p + 1;
                    while run_end < msg.parts.len() {
                        if let MessagePart::Tool(t2) = &msg.parts[run_end]
                            && std::mem::discriminant(&t2.kind)
                                == std::mem::discriminant(&first_tool.kind)
                        {
                            run_end += 1;
                            continue;
                        }
                        break;
                    }
                    let run_len = run_end - p;
                    let group_key = format!("{}:{}", idx, first_tool.id);
                    let expanded = ctx.tool_group_expanded.contains(&group_key);
                    if run_len >= MIN_GROUP_LEN && !expanded {
                        items.push(RenderItem::ToolGroup {
                            key: group_key,
                            kind_label: first_tool.kind.label().to_owned(),
                            count: run_len,
                            kind_color: tool_kind_color(&first_tool.kind, &t),
                        });
                        p = run_end;
                        continue;
                    }
                    // Either the run was too short to bother grouping
                    // or the user has expanded it — emit each tool
                    // individually.
                    for tool_part in &msg.parts[p..run_end] {
                        if let MessagePart::Tool(tool) = tool_part {
                            if !is_invisible_in_transcript(&tool.kind) {
                                items.push(RenderItem::ToolBlock(tool));
                            }
                        }
                    }
                    p = run_end;
                    continue;
                }
                _ => {}
            }
            match part {
                MessagePart::Text(text) => {
                    // Brief mode: suppress plain assistant text — the user only
                    // sees `SendUserMessage` tool output. User-role messages
                    // (the prompts the user typed) stay visible regardless.
                    if ctx.brief_mode && msg.role == jfc_core::Role::Assistant && !text.is_empty() {
                        p += 1;
                        continue;
                    }
                    // User messages render inside a 2-col ribbon indent (see
                    // `MSG_USER_INDENT` in the widget), so they must *wrap* at
                    // that narrower width — otherwise the indented render rect
                    // would re-wrap or clip them. Assistant text uses full width.
                    let content_w = if msg.role == Role::User {
                        inner_w.saturating_sub(MSG_USER_INDENT as usize)
                    } else {
                        inner_w
                    };
                    // Render-layer guard: a model that lost its tool catalog
                    // (misclassified prompt -> tools stripped in stream::request)
                    // can emit a tool call as visible text like
                    // `<Bash command="…" />`. Replace such leaked markup with a
                    // muted notice so the raw XML never reaches the transcript.
                    // Only assistant output is sanitized — a user who pasted
                    // tool-call-looking text should see it verbatim.
                    let render_text: std::borrow::Cow<'_, str> = if msg.role == Role::Assistant {
                        super::tool_xml_guard::sanitize_leaked_tool_calls(text)
                    } else {
                        std::borrow::Cow::Borrowed(text.as_str())
                    };
                    let render_text = render_text.as_ref();
                    // Stream pacing: while this is the LIVE streaming message and
                    // we're on its last (actively-accruing) text part, reveal only
                    // the first `revealed` display segments so output animates in at
                    // the adaptive smooth/catch-up cadence instead of dumping the
                    // whole burst at once. Earlier (completed) text parts are never
                    // truncated, and the engine still holds the full text — this only
                    // gates the display. The streaming cache below keys on this
                    // (truncated) `render_text`, so it stays correct automatically.
                    let render_text: &str = if is_streaming_placeholder
                        && p + 1 == msg.parts.len()
                        && let Some(revealed) = ctx.revealed_streaming_lines
                    {
                        crate::render::codex_stream::stream_pacer::take_first_lines(
                            render_text,
                            revealed,
                        )
                    } else {
                        render_text
                    };
                    let lines = if is_streaming_placeholder {
                        // Streaming fast path: recompute every frame without
                        // syntect. Cost is ~5µs/KB (pulldown-cmark only) vs
                        // ~200µs/KB with syntect highlighting. The streaming
                        // slot avoids doing that work twice per frame: scroll
                        // math and rendering both call build_render_items with
                        // the same placeholder body before the next stream
                        // chunk can mutate it.
                        let theme = t;
                        let width = content_w as u16;
                        let mut cache = ctx.render_cache.borrow_mut();
                        if let Some(lines) = cache.get_streaming(idx, width, render_text) {
                            lines.to_vec()
                        } else {
                            let lines =
                                markdown::to_lines_streaming(render_text, &theme, content_w);
                            cache.set_streaming(idx, width, render_text, lines.clone());
                            lines
                        }
                    } else {
                        let mut cache = ctx.render_cache.borrow_mut();
                        let width = content_w as u16;
                        let theme = t;
                        cache
                            .get_or_insert_with(render_text, width, |t_text, w| {
                                markdown::to_lines(t_text, &theme, w as usize)
                            })
                            .to_vec()
                    };
                    for line in lines {
                        items.push(RenderItem::TextLine(line));
                    }
                }
                MessagePart::Reasoning(text) => {
                    push_reasoning_lines(items, text, reasoning_expanded, &t);
                }
                MessagePart::Tool(tool) => {
                    if !is_invisible_in_transcript(&tool.kind) {
                        items.push(RenderItem::ToolBlock(tool));
                    }
                }
                MessagePart::TaskStatus(ts) => {
                    push_task_status_lines(items, ts, &t, inner_w);
                }
                MessagePart::CompactBoundary { pre_tokens } => {
                    items.push(RenderItem::TextLine(Line::from(vec![
                        Span::styled("─── ", Style::default().fg(t.border)),
                        Span::styled(
                            format!("compacted ({pre_tokens} tokens summarized)"),
                            t.muted(),
                        ),
                        Span::styled(" ───", Style::default().fg(t.border)),
                    ])));
                }
                MessagePart::Advisor(text) => {
                    push_advisor_lines(items, text, &t);
                }
                MessagePart::RedactedThinking(data) => {
                    // Not a jfc choice and not recoverable: Anthropic returns
                    // these blocks *encrypted* when a reasoning span trips its
                    // safety classifier — the plaintext is not in the response.
                    // We keep the ciphertext so it can be replayed on the next
                    // request (thinking continuity), but there's nothing to
                    // display. Full (non-redacted) thinking is already what we
                    // request; this is the server withholding a span.
                    items.push(RenderItem::TextLine(Line::from(vec![
                        Span::styled(
                            "∴ Redacted thinking",
                            Style::default()
                                .fg(t.text_muted)
                                .add_modifier(Modifier::ITALIC),
                        ),
                        Span::styled(
                            format!(
                                " — encrypted by the provider; not shown ({} bytes kept for continuity)",
                                data.len()
                            ),
                            Style::default().fg(t.text_muted),
                        ),
                    ])));
                }
            }
            p += 1;
        }

        // v126 cli.js:341376 — `Cooked for Nm Ns` post-turn footer with a
        // randomized past-tense verb. Only attached to completed assistant
        // turns (skip user messages, skip the in-flight placeholder which
        // already has its own spinner row). `msg.elapsed` carries the
        // duration string written at StreamDone time.
        if msg.role == Role::Assistant
            && !is_streaming_placeholder
            && let Some(elapsed) = &msg.elapsed
        {
            // Dim italic, no leading glyph. The earlier `▎`
            // prefix bracketed the message visually with the
            // role-header gutter; with the gutter gone, the
            // bracket is gone too. The elapsed line just sits
            // muted under the body, which reads cleaner.
            items.push(RenderItem::TextLine(Line::from(Span::styled(
                elapsed.clone(),
                Style::default()
                    .fg(t.text_muted)
                    .add_modifier(Modifier::DIM),
            ))));
        }
        // Close the message scope BEFORE the Blank separator so the
        // gutter doesn't bleed into the empty row between messages.
        // Reads as: each message is a contained band, separated by a
        // narrow gap.
        items.push(RenderItem::MessageEnd);
        items.push(RenderItem::Blank);
    }
}

#[cfg(test)]
mod invisible_tool_tests {
    use super::is_invisible_in_transcript;
    use jfc_core::ToolKind;

    #[test]
    fn task_tool_is_hidden_so_only_taskstatus_renders_normal() {
        // BUG B: a Task spawn emits both a Tool(Task) block and a TaskStatus
        // part. The Task tool block must be hidden so the task shows exactly one
        // status icon (the green ● from TaskStatus), not two.
        assert!(is_invisible_in_transcript(&ToolKind::Task));
    }

    #[test]
    fn bash_stays_visible_robust() {
        // Bash output is the user's primary signal — it must render.
        assert!(!is_invisible_in_transcript(&ToolKind::Bash));
    }
}

#[cfg(test)]
mod reminder_skip_tests {
    use super::strip_system_reminders;

    #[test]
    fn reminder_only_strips_to_empty_normal() {
        let s = "<system-reminder>\ngoal unmet, keep going\n</system-reminder>";
        assert!(strip_system_reminders(s).trim().is_empty());
    }

    #[test]
    fn real_text_with_appended_reminder_survives_normal() {
        let s = "fix the bug\n<system-reminder>\nplan is live\n</system-reminder>";
        assert_eq!(strip_system_reminders(s).trim(), "fix the bug");
    }

    #[test]
    fn unterminated_reminder_does_not_panic_robust() {
        assert!(
            strip_system_reminders("<system-reminder>oops")
                .trim()
                .is_empty()
        );
        assert_eq!(strip_system_reminders("hi <system-reminder>x").trim(), "hi");
    }
}

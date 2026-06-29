use super::approval::approval;
use super::elicitation::elicitation;
use super::input_box::{input, input_visual_line_count};
use super::messages::messages;
use super::messages::subagent_footer;
use super::messages::{spinner_row, tasks_pinned_row};
use super::model_picker::model_picker;
use super::overlays::{
    diagnostic_panel, diagnostic_row, help_overlay, mention_popup, prompt_search_overlay,
    search_bar, slash_popup, toast_overlay,
};
use super::palette::palette;
use super::question::question;
use super::session_picker::session_picker;
use super::session_sidebar::sidebar;
use super::sidebar::info_sidebar;
use super::status::status;
use super::task_panel::task_panel;
use super::teammates_panel::teammates_panel;
use super::theme_picker::theme_picker;
use super::*;

pub fn frame(f: &mut Frame, app: &mut App) {
    let t = app.theme;

    // Hit-test regions are populated per-frame as each tool block renders.
    // Clear here so a frame with no visible tools doesn't carry over stale
    // rects from the previous frame — that would let the user click on a
    // region of the screen now occupied by something else and toggle a
    // tool they can't see.
    app.tool_hit_regions.borrow_mut().clear();
    app.tool_copy_regions.borrow_mut().clear();
    *app.input_rect.borrow_mut() = None;

    f.render_widget(Block::default().style(Style::default().bg(t.bg)), f.area());

    // Input composer width = full terminal width minus its own text chrome:
    // 2 padding cols + prompt-strip cols. The earlier "subtract sidebars" math was wrong —
    // sidebars only split `chunks[0]` (the messages row); the
    // input lives in `chunks[4]` which gets the FULL terminal
    // width regardless of which sidebars are open. So the wrap
    // estimate must use full width too, otherwise input_height is
    // over-counted when sidebars are visible (estimated wrap at a
    // narrower phantom width = more visual rows than the real
    // render produces) and the input box ends up taller than
    // needed, eating into the message column.
    // Task view collapses the chat dock — when reading a background agent's
    // transcript you can't act on the input, pinned tasks, or the agent fan.
    // Keep only a one-line task tab strip plus the compact status line.
    let in_task_view = app.task_panel.viewing_task_id.is_some();

    let total_w_pre = f.area().width as usize;
    let input_content_w = total_w_pre.saturating_sub(4);
    let input_lines = input_visual_line_count(app, input_content_w);
    // +2 reserves the composer's top/bottom padding rows (borderless,
    // opencode-style). `input_visual_line_count` returns the pure wrapped-line
    // count; the chrome is added here where the layout height is decided.
    let input_height = if in_task_view {
        0
    } else {
        (input_lines + 2).min(8) as u16
    };
    // One quiet task-view tab strip. Navigation hints fit on the same row when
    // there is room; the transcript gets the rest.
    let subagent_footer_height: u16 = if app.task_panel.viewing_task_id.is_some() {
        1
    } else {
        0
    };
    // v126 puts the "Fermenting…" spinner as a dedicated row above the input
    // (not as the input's border title) — so the input bar stays visually
    // stable during streaming and the spinner reads as part of the
    // conversation timeline. We allocate a 1-row slot only while streaming
    // (2 rows when there's an open task → render `Next: <subject>` underneath
    // matching cli.js:323851 `Next: ${m.subject}`). When idle the slot
    // collapses to 0 and the input snaps to the bottom.
    // Spinner: 1 row for the verb status alone, 2 rows when there's
    // either a `Next: <task>` subject OR a `Tip:` fallback to surface.
    // Always reserve 2 rows when streaming so the tip cycles visibly.
    // Spinner is also shown during pre-submit / `/compact` compaction so a
    // long compact request doesn't read as a frozen UI. v126 cli.js does
    // the same — the spinner verb just changes to "Compacting".
    // Spinner visibility = "is the user's turn still in flight?". The
    // earlier gate (`is_streaming || compacting || pending_tool_calls`)
    // dropped to false during the brief gap between SSE end and the
    // next stream's start mid-agentic-loop — the spinner blinked off
    // and back on. `turn_started_at` and the tool pipeline keep it lit for the
    // *whole* turn (set at submit, cleared at the turn-complete event), even
    // after queued tools drain into an executing batch. Background tasks count
    // too so a fan of subagents keeps the spinner alive even if the leader
    // finished.
    let any_alive_subagent = app
        .engine
        .background_tasks
        .values()
        .any(|bt| bt.status.is_alive());
    let show_spinner = app.engine.is_streaming
        || app.engine.compacting_started_at.is_some()
        || app.engine.pipeline_busy_for_submit()
        || app.engine.turn_started_at.is_some()
        || any_alive_subagent;
    // Spinner row above input: verb + Next preview only. Background agents are
    // available through the explicit agents/teammates views; keeping a live fan
    // docked under the prompt made the primary chat surface noisy and unlike
    // Claude's clean input/footer stack.
    let spinner_row_height: u16 = if show_spinner && !in_task_view { 2 } else { 0 };
    // Pinned todo list above the input, Claude-Code style. Header row
    // ("Tasks (k/n done)") + up to task_pin_visible rows + an optional
    // "+N more" footer. Height collapses to 0 when no tasks exist so
    // first-run UI stays clean.
    let tp_all = app
        .engine
        .task_store
        .list(jfc_session::DeletedFilter::Exclude);
    let tp_open: usize = tp_all.iter().filter(|t| t.status.is_open()).count();
    let now_tp = std::time::Instant::now();
    let tp_recent_done: usize = tp_all
        .iter()
        .filter(|t| matches!(t.status, jfc_session::TaskStatus::Completed))
        .filter(|t| {
            app.engine
                .task_completion_times
                .get(&t.id)
                .is_some_and(|ts| now_tp.duration_since(*ts).as_secs() < 30)
        })
        .count();
    // Keep the pinned task list peripheral. The active spinner already names
    // the current task; the pin is just enough checklist context to avoid the
    // historical "194/201 done" dashboard block.
    let task_pin_visible: usize = {
        let rows = f.area().height as usize;
        if rows <= 10 {
            0
        } else {
            rows.saturating_sub(16).clamp(2, 4)
        }
    };
    // Collapse out entirely when there's nothing live OR recently-done to
    // show — a "Tasks (27/27 done)" header hovering alone after every
    // task closed read as visual debt. The fade-out tail (recent_done <
    // 30s) keeps the celebratory ✓ row briefly so the user sees the
    // last completion land.
    // Recently-completed now collapse to a single summary line (not one
    // row per task), so they cost at most 1 row — plus an extra row for
    // the focal in-progress task's activeForm sub-line.
    let recent_rows = if tp_recent_done > 0 { 1 } else { 0 };
    let task_pin_rows = if tp_open == 0 && tp_recent_done == 0 {
        0
    } else {
        let body = (tp_open + recent_rows + 1).min(task_pin_visible);
        let overflow = if tp_open + recent_rows > task_pin_visible {
            1
        } else {
            0
        };
        body + overflow
    };
    // Floats (no divider rule). All-done collapses to one "✓ N tasks done"
    // line; live work shows a header line + task rows.
    let tasks_pinned_height: u16 = if in_task_view || task_pin_rows == 0 {
        0
    } else if tp_open == 0 {
        1
    } else {
        (task_pin_rows as u16).min(5)
    };
    let agent_fan_height: u16 = 0;
    // Diagnostic summary row — only shown when there are *new*
    // (unacknowledged) entries. v126 cli.js:231025-231036 keeps a
    // per-URI "delivered" set; entries already shown to the user don't
    // re-pop the row on every LSP refresh. The expansion panel
    // (Ctrl+O) shows the *full* current state regardless. This makes
    // the row a notification (transient), not a status display
    // (persistent) — what was wrong before this change.
    let unack_count = jfc_engine::diagnostics::unacknowledged(
        &app.engine.diagnostics,
        &app.delivered_diagnostics,
    )
    .len();
    let diag_row_height: u16 = if unack_count == 0 { 0 } else { 1 };

    // In task view the agent tab strip sits at the TOP (browser-style),
    // above the transcript — so it's the first chunk. Its height is 0
    // outside task view, so in normal chat the slot collapses and the
    // message area starts at the top as before.
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(subagent_footer_height), // 0: task tab strip (top)
            Constraint::Min(3),                         // 1: messages
            Constraint::Length(diag_row_height),        // 2
            Constraint::Length(spinner_row_height),     // 3
            Constraint::Length(tasks_pinned_height),    // 4
            Constraint::Length(input_height),           // 5
            Constraint::Length(agent_fan_height),       // 6
            Constraint::Length(1),                      // 7: compact status line
        ])
        .split(f.area());

    // Sidebar animation: currently snaps (0.0/1.0). To animate, add
    // `sidebar_anim_target: f32` and `sidebar_anim_current: f32` fields to
    // App, lerp `sidebar_anim_current` toward `sidebar_anim_target` on each
    // tick, and use `sidebar_anim_current` here. Low priority — snap feels fine.
    let total_w = f.area().width as usize;
    let sidebar_progress: f32 = if app.session_sidebar.visible && !in_task_view {
        1.0
    } else {
        0.0
    };
    let show_right = app.info_sidebar.visible && !in_task_view && total_w >= 120;

    // Responsive sidebars: at narrow widths the sessions sidebar
    // shrinks toward 20 cols and the info sidebar drops below 32, so
    // the message column always retains a usable working area. v126
    // does the same — sidebars scale with terminal width instead of
    // pinning to fixed column counts.
    let left_w_full = (total_w / 5).clamp(20, 32) as u16;
    let left_w = (left_w_full as f32 * ease_out_cubic(sidebar_progress)) as u16;
    let show_left = left_w > 0;
    let right_w = if total_w < 140 {
        32
    } else {
        (total_w / 6).clamp(36, 48) as u16
    };

    // Tab strip at the top in task view (chunks[0]); collapsed otherwise.
    if app.task_panel.viewing_task_id.is_some() {
        subagent_footer(f, app, chunks[0]);
    }

    let msg_area = chunks[1];
    match (show_left, show_right) {
        (true, true) => {
            let split = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([
                    Constraint::Length(left_w),
                    Constraint::Min(20),
                    Constraint::Length(right_w),
                ])
                .split(msg_area);
            sidebar(f, app, split[0]);
            messages(f, app, split[1]);
            info_sidebar(f, app, split[2]);
        }
        (true, false) => {
            let split = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Length(left_w), Constraint::Min(20)])
                .split(msg_area);
            sidebar(f, app, split[0]);
            messages(f, app, split[1]);
        }
        (false, true) => {
            let split = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Min(20), Constraint::Length(right_w)])
                .split(msg_area);
            messages(f, app, split[0]);
            info_sidebar(f, app, split[1]);
        }
        (false, false) => {
            messages(f, app, msg_area);
        }
    }
    if unack_count > 0 {
        diagnostic_row(f, app, chunks[2]);
    }
    if show_spinner {
        spinner_row(f, app, chunks[3]);
    }
    if tasks_pinned_height > 0 {
        tasks_pinned_row(f, app, chunks[4]);
    }
    if input_height > 0 {
        input(f, app, chunks[5]);
    }
    status(f, app, chunks[7]);

    // Resolve a pending word/line multi-click into a selection span (needs the
    // buffer), then paint the drag highlight / extract + copy on finalize.
    // Both run BEFORE the overlays below so extraction reads clean transcript
    // cells, never an approval modal / popup painted on top.
    resolve_select_request(f, app);
    apply_text_selection(f, app);

    if app.palette.visible {
        palette(f, app);
    }

    if app.theme_picker.visible {
        theme_picker(f, app);
    }

    if app.model_picker.visible {
        model_picker(f, app);
    }

    if app.session_picker.visible {
        session_picker(f, app);
    }

    if app.bash_picker.visible {
        super::bash_picker::bash_picker(f, app);
    }

    if app.task_panel.visible {
        task_panel(f, app);
    }

    if matches!(
        app.task_panel.expanded_view,
        crate::app::ExpandedView::Teammates
    ) {
        teammates_panel(f, app);
    }

    if !app.engine.toasts.is_empty() {
        toast_overlay(f, app);
    }

    if app.mention.active && !app.mention.candidates.is_empty() {
        mention_popup(f, app, chunks[5]);
    }

    if app.show_diagnostic_panel && !app.engine.diagnostics.is_empty() {
        diagnostic_panel(f, app);
    }

    if app.show_help {
        help_overlay(f, app);
    }

    if app.transcript_search.is_some() {
        search_bar(f, app);
    }

    if app.prompt_search.is_some() {
        prompt_search_overlay(f, app);
    }

    // Slash-command autocomplete: opens above the input bar when
    // the user has typed `/<prefix>` and there are matching commands.
    if let Some(prefix) = current_slash_prefix(app) {
        slash_popup(f, app, &prefix);
    }

    if app.engine.pending_approval.is_some() {
        approval(f, app);
    }

    // The AskUserQuestion modal. Mutually exclusive with approval in practice
    // (approval gates a dispatch; a question is a separate turn-ending tool),
    // but rendered last so it wins if both were somehow set.
    if app.engine.pending_question.is_some() {
        question(f, app);
    }

    // MCP elicitation modal — rendered on top of everything else when an
    // MCP server requests interactive user input.
    if !app.engine.pending_elicitations.is_empty() {
        elicitation(f, app);
    }

    // Prompt-rewrite proposal modal (over-refusal gate). Rendered last so it
    // wins focus when a rewrite awaits the user's accept/reject/edit.
    if app.pending_rewrite_proposal.is_some() {
        super::prompt_rewrite::prompt_rewrite(f, app);
    }
}

/// Column span `[c0, c1)` of the selection on content `line`, in terminal-
/// selection semantics: the first line runs from the anchor column to the
/// right edge, the last line from the left edge to the head column
/// (inclusive), middle lines are full-width, and a single-line selection is
/// just anchor→head. Lines are absolute content lines (scroll-invariant), not
/// screen rows.
fn selection_line_span(
    line: usize,
    start: (u16, usize),
    end: (u16, usize),
    left: u16,
    right: u16,
) -> (u16, u16) {
    let (c0, c1) = if start.1 == end.1 {
        (start.0, end.0.saturating_add(1))
    } else if line == start.1 {
        (start.0, right)
    } else if line == end.1 {
        (left, end.0.saturating_add(1))
    } else {
        (left, right)
    };
    (c0.clamp(left, right), c1.clamp(left, right))
}

/// Extract the selected text from the transcript CONTENT, not the visible
/// frame buffer: the selected line range is re-rendered into an offscreen
/// buffer through the same `MessageView` pipeline that paints the screen, so
/// the copied text is byte-identical to what the rows show — including lines
/// that have scrolled outside the viewport. (The old extractor read only the
/// visible frame cells, so a selection that scrolled offscreen copied
/// nothing.)
pub(super) fn extract_selection_text(
    app: &App,
    start: (u16, usize),
    end: (u16, usize),
    area: Rect,
) -> String {
    use ratatui::widgets::Widget;
    // Same width the live transcript renders at: area − 2 (horizontal
    // padding). Must match `render::messages` exactly or the offscreen wrap
    // diverges from what the user selected.
    let content_w = area.width.saturating_sub(2);
    if content_w == 0 {
        return String::new();
    }
    // Cap pathological spans so a stray drag can't allocate a giant buffer.
    const MAX_COPY_LINES: usize = 2000;
    let first = start.1;
    let span = end
        .1
        .saturating_sub(first)
        .saturating_add(1)
        .min(MAX_COPY_LINES);

    let inner_w = content_w as usize;
    let ctx = crate::message_view::RenderCtx::from_app(app);
    let items = crate::message_view::build_render_items_pub(&ctx, inner_w);
    let total_h: usize = items
        .iter()
        .map(|i| i.height_with_app(inner_w, Some(app)))
        .sum();
    let tmp_area = Rect::new(0, 0, content_w, span as u16);
    let mut tmp = ratatui::buffer::Buffer::empty(tmp_area);
    crate::message_view::MessageView {
        app,
        prebuilt: Some(crate::message_view::PrebuiltItems {
            items,
            total_h,
            scroll: first,
        }),
    }
    .render(tmp_area, &mut tmp);

    // Column mapping: the selection stores absolute screen columns; the
    // offscreen buffer starts at the content's left edge (area.x + 1).
    let left_screen = area.x.saturating_add(1);
    let right_screen = area.x + area.width - 1; // exclusive (right padding)
    let mut rows: Vec<String> = Vec::new();
    for off in 0..span {
        let line = first + off;
        let (c0, c1) = selection_line_span(line, start, end, left_screen, right_screen);
        let mut s = String::new();
        for col in c0..c1 {
            let x = col.saturating_sub(left_screen);
            if x < content_w && (off as u16) < tmp_area.height {
                s.push_str(tmp[(x, off as u16)].symbol());
            }
        }
        rows.push(s.trim_end().to_string());
    }
    rows.join("\n")
}

fn apply_text_selection(f: &mut Frame, app: &mut App) {
    let Some(sel) = app.text_selection else {
        return;
    };
    let Some(area) = *app.messages_rect.borrow() else {
        app.text_selection = None;
        return;
    };
    if area.width < 3 || area.height < 3 || sel.area_width != area.width {
        // Too small to extract/highlight — or the transcript re-wrapped
        // (width change from a sidebar toggle/resize), which remaps every
        // content line. Either way the stored coordinates are stale.
        app.text_selection = None;
        return;
    }
    // The transcript is borderless: content fills the area top-to-bottom,
    // inset 1 col by horizontal padding.
    let top = area.y;
    let bottom = area.y + area.height; // exclusive (last content row = bottom-1)
    let left = area.x.saturating_add(1); // horizontal padding
    let right = area.x + area.width - 1; // exclusive (right padding)

    let (start, end) = sel.ordered();

    if sel.finalize {
        // Content-backed extraction: re-renders the selected line range
        // offscreen, so the copy is complete even if part of the selection
        // has scrolled out of the viewport.
        let text = extract_selection_text(app, start, end, area);
        if text.trim().is_empty() {
            // Nothing to copy or keep highlighted (blank-area drag).
            app.text_selection = None;
            return;
        }
        crate::runtime::copy_to_clipboard(&text, "select");
        // Persist the highlight (copied=true) so the user sees what was
        // copied; cleared on the next mouse-down, Esc, or width change —
        // scrolling keeps it (content-line coords stay valid). Fall through
        // to paint the highlight this same frame.
        app.text_selection = Some(crate::app::TextSelection {
            finalize: false,
            copied: true,
            dragged: false,
            ..sel
        });
    }

    // Live highlight: paint the VISIBLE slice of the selection — content
    // lines are mapped through the current scroll offset to screen rows, so
    // the highlight tracks the text as the transcript scrolls instead of
    // dying on the first wheel tick. Offscreen parts simply don't paint.
    let sel_bg = app.theme.selection_bg();
    let scroll = app.scroll_offset;
    let buf = f.buffer_mut();
    let bounds = *buf.area();
    for line in start.1..=end.1 {
        if line < scroll {
            continue;
        }
        let row = top as usize + (line - scroll);
        if row >= bottom as usize {
            break;
        }
        let row = row as u16;
        let (c0, c1) = selection_line_span(line, start, end, left, right);
        for col in c0..c1 {
            if col < bounds.right() && row < bounds.bottom() {
                let cell = &mut buf[(col, row)];
                cell.set_style(cell.style().bg(sel_bg));
            }
        }
    }
}

/// Resolve a pending multi-click into a one-row `TextSelection` span and hand
/// it to the finalize path (which extracts + copies + persists the highlight).
/// Word = the word-class run under the click; Line = the row's content span.
fn resolve_select_request(f: &mut Frame, app: &mut App) {
    let Some(req) = app.pending_select_request.take() else {
        return;
    };
    let Some(area) = *app.messages_rect.borrow() else {
        return;
    };
    if area.width < 3 || area.height < 3 {
        return;
    }
    // Borderless transcript: content fills the area, inset 1 col by padding.
    let top = area.y;
    let bottom = area.y + area.height; // exclusive (last content row = bottom-1)
    let left = area.x.saturating_add(1); // horizontal padding
    let right = area.x + area.width - 1; // exclusive (right padding)
    if right <= left || req.row < top || req.row >= bottom {
        return;
    }
    let row = req.row;
    let (anchor_col, head_col) = match req.kind {
        crate::app::SelectKind::Line => (left, right - 1),
        crate::app::SelectKind::Word => {
            let buf = f.buffer_mut();
            let bounds = *buf.area();
            let mut chars: Vec<char> = Vec::with_capacity((right - left) as usize);
            for col in left..right {
                let ch = if col < bounds.right() && row < bounds.bottom() {
                    buf[(col, row)].symbol().chars().next().unwrap_or(' ')
                } else {
                    ' '
                };
                chars.push(ch);
            }
            let click_off = (req.col.clamp(left, right - 1) - left) as usize;
            let (s, e) = word_span_in_row(&chars, click_off);
            (left + s as u16, left + e as u16)
        }
    };
    // Store the selection in scroll-invariant content-line coordinates.
    let line = app.scroll_offset + row.saturating_sub(top) as usize;
    app.text_selection = Some(crate::app::TextSelection {
        anchor: (anchor_col, line),
        head: (head_col, line),
        area_width: area.width,
        dragged: true,
        finalize: true,
        copied: false,
    });
}

/// Character class for word selection (matches Claude Code's WORD_CHAR set).
fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || matches!(c, '_' | '/' | '.' | '-' | '+' | '~' | '\\')
}

/// Inclusive `[start, end]` cell offsets of the word under `idx`. A click on a
/// non-word char selects just that cell (whitespace then trims to empty in
/// extraction — a no-op double-click on blank space).
fn word_span_in_row(chars: &[char], idx: usize) -> (usize, usize) {
    if chars.is_empty() {
        return (0, 0);
    }
    let idx = idx.min(chars.len() - 1);
    if !is_word_char(chars[idx]) {
        return (idx, idx);
    }
    let mut start = idx;
    while start > 0 && is_word_char(chars[start - 1]) {
        start -= 1;
    }
    let mut end = idx;
    while end + 1 < chars.len() && is_word_char(chars[end + 1]) {
        end += 1;
    }
    (start, end)
}

#[cfg(test)]
mod tests {
    use super::{is_word_char, selection_line_span, word_span_in_row};

    // Characterization tests for selection_line_span — the pure column-span
    // computation behind drag-selection extraction, now keyed on absolute
    // CONTENT lines (scroll-invariant) instead of screen rows. left=1,
    // right=40 model the padded message rect (col 0 and the far-right col are
    // padding).
    const LEFT: u16 = 1;
    const RIGHT: u16 = 40;

    #[test]
    fn selection_span_single_line_is_inclusive_normal() {
        // Same-line drag from col 5 to col 9 → [5, 10) (end is inclusive, +1).
        let span = selection_line_span(7, (5, 7), (9, 7), LEFT, RIGHT);
        assert_eq!(span, (5, 10));
    }

    #[test]
    fn selection_span_first_line_runs_to_right_edge_normal() {
        // Multi-line drag: the first line selects from the anchor col to the
        // right edge.
        let span = selection_line_span(3, (12, 3), (8, 6), LEFT, RIGHT);
        assert_eq!(span, (12, RIGHT));
    }

    #[test]
    fn selection_span_last_line_runs_from_left_edge_normal() {
        // The last line selects from the left edge to the head col (inclusive).
        let span = selection_line_span(6, (12, 3), (8, 6), LEFT, RIGHT);
        assert_eq!(span, (LEFT, 9));
    }

    #[test]
    fn selection_span_middle_line_is_full_width_robust() {
        // A fully-spanned middle line covers the whole content width.
        let span = selection_line_span(4, (12, 3), (8, 6), LEFT, RIGHT);
        assert_eq!(span, (LEFT, RIGHT));
    }

    #[test]
    fn selection_span_clamps_out_of_bounds_columns_robust() {
        // Columns past the right edge clamp into [left, right] so extraction
        // never indexes outside the padded rect.
        let span = selection_line_span(2, (0, 2), (99, 2), LEFT, RIGHT);
        assert_eq!(span, (LEFT, RIGHT));
    }

    // The selection is scroll-invariant: the span for a content line is the
    // same regardless of any scroll offset (scroll only affects which rows
    // paint, not what is selected). Lines far past any viewport still span.
    #[test]
    fn selection_span_is_scroll_invariant_robust() {
        let span_near = selection_line_span(5, (3, 5), (9, 5), LEFT, RIGHT);
        let span_far = selection_line_span(10_005, (3, 10_005), (9, 10_005), LEFT, RIGHT);
        assert_eq!(span_near, span_far);
    }

    #[test]
    fn word_span_selects_full_token_normal() {
        // "  foo/bar.rs  " — click inside the token grabs the whole path-like
        // run (/, ., - are word chars).
        let chars: Vec<char> = "  foo/bar.rs  ".chars().collect();
        let (s, e) = word_span_in_row(&chars, 5); // on the '/'
        assert_eq!(&chars[s..=e].iter().collect::<String>(), "foo/bar.rs");
    }

    #[test]
    fn word_span_on_whitespace_is_single_cell_robust() {
        let chars: Vec<char> = "ab cd".chars().collect();
        assert_eq!(word_span_in_row(&chars, 2), (2, 2)); // the space
    }

    #[test]
    fn word_span_stops_at_punctuation_boundary_robust() {
        let chars: Vec<char> = "foo(bar)".chars().collect();
        let (s, e) = word_span_in_row(&chars, 1); // inside "foo"
        assert_eq!((s, e), (0, 2));
        assert!(!is_word_char('('));
    }
}

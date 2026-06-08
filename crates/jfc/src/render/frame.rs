use super::approval::approval;
use super::elicitation::elicitation;
use super::input_box::{input, input_visual_line_count};
use super::messages::messages;
use super::messages::{agent_fan_below_input, subagent_footer};
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

    f.render_widget(Block::default().style(Style::default().bg(t.bg)), f.area());

    // Input box width = full terminal width minus the input's own
    // chrome: 2 border cols + 2 padding cols + 2 prompt-strip cols
    // = 6 cols. The earlier "subtract sidebars" math was wrong —
    // sidebars only split `chunks[0]` (the messages row); the
    // input lives in `chunks[4]` which gets the FULL terminal
    // width regardless of which sidebars are open. So the wrap
    // estimate must use full width too, otherwise input_height is
    // over-counted when sidebars are visible (estimated wrap at a
    // narrower phantom width = more visual rows than the real
    // render produces) and the input box ends up taller than
    // needed, eating into the message column.
    // Task view collapses the entire chat dock — when reading a
    // background agent's transcript you can't act on the input, pinned
    // tasks, or the agent fan, so they'd just squeeze the log. Keep only
    // the tab strip (footer) + status bar. Sidebars are also forced off
    // so the transcript gets the full width.
    let in_task_view = app.viewing_task_id.is_some();

    // Input box is now a flat strip under a single TOP divider (no
    // full rounded box), so its chrome is 1 row, not 2. Width chrome is
    // likewise 4 (2 padding + 2 prompt strip), not 6 (no L/R borders).
    let total_w_pre = f.area().width as usize;
    let input_content_w = total_w_pre.saturating_sub(4);
    let input_lines = input_visual_line_count(app, input_content_w);
    let input_height = if in_task_view {
        0
    } else {
        (input_lines + 1).min(7) as u16
    };
    // Two rows when in task view: tab strip on top, key-hint row
    // below. Was 1 when the footer was a flat back/next string;
    // expanded for the Tabs widget redesign so each tab has space
    // for its glyph + truncated description.
    let subagent_footer_height: u16 = if app.viewing_task_id.is_some() { 2 } else { 0 };
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
    // and back on. Adding `turn_started_at.is_some()` keeps it lit
    // for the *whole* turn (set at submit, cleared at the
    // turn-complete event). Background tasks count too so a fan of
    // subagents keeps the spinner alive even if the leader finished.
    let any_alive_subagent = app
        .engine
        .background_tasks
        .values()
        .any(|bt| bt.status.is_alive());
    let show_spinner = app.engine.is_streaming
        || app.engine.compacting_started_at.is_some()
        || !app.engine.pending_tool_calls.is_empty()
        || app.engine.turn_started_at.is_some()
        || any_alive_subagent;
    // When a team is active, the spinner area expands to show the teammate tree:
    // 2 base rows (spinner + next-task hint) + 1 leader row + N teammate rows.
    // For non-team parallel subagents (the "fire 5 Explore agents" case),
    // expand for the same reason — the user sees one row per agent.
    let teammate_count = if app.engine.team_context.is_active() {
        app.engine.team_context.teammates.len().saturating_sub(1) // exclude leader
    } else {
        0
    };
    let active_subagent_count = if !app.engine.team_context.is_active() {
        // Count both Running and Idle teammates: Idle ones still belong
        // on the fan (the user can SendMessage to wake them) so the
        // tree row needs to be reserved for them.
        app.engine
            .background_tasks
            .values()
            .filter(|bt| bt.status.is_alive())
            .count()
    } else {
        0
    };
    let tree_rows = teammate_count.max(active_subagent_count);
    // Spinner row above input: verb + Next preview only. The agent fan
    // tree moved below the input — "agent view sits under the input
    // box" reads better than "agent fan crowds the verb line", and it
    // keeps the verb glued to the prompt where the user's eye lives
    // while typing.
    let spinner_row_height: u16 = if show_spinner && !in_task_view { 2 } else { 0 };
    // Pinned todo list above the input, Claude-Code style. Header row
    // ("Tasks (k/n done)") + up to task_pin_visible rows + an optional
    // "+N more" footer. Height collapses to 0 when no tasks exist so
    // first-run UI stays clean.
    let tp_all = app
        .engine
        .task_store
        .list(jfc_session::DeletedFilter::Exclude);
    let tp_open: usize = tp_all
        .iter()
        .filter(|t| {
            matches!(
                t.status,
                jfc_session::TaskStatus::Pending | jfc_session::TaskStatus::InProgress
            )
        })
        .count();
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
    // Dynamic cap: matches Claude Code's `rows <= 10 ? 0 : min(5, max(3, rows - 14))`
    // so on small terminals the task pin doesn't eat the screen, while larger
    // terminals can show more context.
    let task_pin_visible: usize = {
        let rows = f.area().height as usize;
        if rows <= 10 {
            0
        } else {
            rows.saturating_sub(14).clamp(3, 5)
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
        // +1 for the progress-header content line (the old `+1` was the
        // divider row; the header is now content, so the count is unchanged).
        (task_pin_rows as u16).min(10) + 1
    };
    // Agent fan beneath the input: leader row ("agents") plus one row
    // per alive sub-agent. Capped at 8 so a fan of 30 doesn't push the
    // status bar off-screen — the user can still open the task view to
    // see all of them.
    // Flattened: 1 TOP divider + 1 summary line + up to 7 agent rows.
    // (Was a full rounded box: +2 chrome plus a leader row.) The summary
    // line carries the fleet counts, so the per-agent rows are pure data.
    let agent_fan_height: u16 = if tree_rows > 0 && !in_task_view {
        (2 + tree_rows as u16).min(9)
    } else {
        0
    };
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
            Constraint::Length(2),                      // 7: status (gauge-divider + info line)
        ])
        .split(f.area());

    // TODO: Wire sidebar_progress to App animation fields once Agent A adds them.
    // For now, snap to 0.0/1.0 so the ease_out_cubic path is exercised but
    // the visual result is identical to the old binary toggle.
    // The right info sidebar is gone entirely — Context, the git diff
    // stat (Δ), and MCP/LSP health all live in the status bar now, so a
    // whole column of chrome bought nothing. The left sessions sidebar
    // still toggles, but never in task view (transcript wants the width).
    let sidebar_progress: f32 = if app.show_sidebar && !in_task_view {
        1.0
    } else {
        0.0
    };
    let show_right = false;

    // Responsive sidebars: at narrow widths the sessions sidebar
    // shrinks toward 20 cols and the info sidebar drops below 32, so
    // the message column always retains a usable working area. v126
    // does the same — sidebars scale with terminal width instead of
    // pinning to fixed column counts.
    let total_w = f.area().width as usize;
    let left_w_full = (total_w / 5).clamp(20, 32) as u16;
    let left_w = (left_w_full as f32 * ease_out_cubic(sidebar_progress)) as u16;
    let show_left = left_w > 0;
    let right_w = if total_w < 140 {
        32
    } else {
        (total_w / 6).clamp(36, 48) as u16
    };

    // Tab strip at the top in task view (chunks[0]); collapsed otherwise.
    if app.viewing_task_id.is_some() {
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
    if agent_fan_height > 0 {
        agent_fan_below_input(f, app, chunks[6]);
    }
    status(f, app, chunks[7]);

    // Resolve a pending word/line multi-click into a selection span (needs the
    // buffer), then paint the drag highlight / extract + copy on finalize.
    // Both run BEFORE the overlays below so extraction reads clean transcript
    // cells, never an approval modal / popup painted on top.
    resolve_select_request(f, app);
    apply_text_selection(f, app);

    if app.show_palette {
        palette(f, app);
    }

    if app.show_theme_picker {
        theme_picker(f, app);
    }

    if app.show_model_picker {
        model_picker(f, app);
    }

    if app.show_session_picker {
        session_picker(f, app);
    }

    if app.show_task_panel {
        task_panel(f, app);
    }

    if matches!(app.expanded_view, crate::app::ExpandedView::Teammates) {
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
}

/// Column span `[c0, c1)` of the selection on `row`, in terminal-selection
/// semantics: the first row runs from the anchor column to the right edge,
/// the last row from the left edge to the head column (inclusive), middle
/// rows are full-width, and a single-row selection is just anchor→head.
fn selection_row_span(
    row: u16,
    start: (u16, u16),
    end: (u16, u16),
    left: u16,
    right: u16,
) -> (u16, u16) {
    let (c0, c1) = if start.1 == end.1 {
        (start.0, end.0.saturating_add(1))
    } else if row == start.1 {
        (start.0, right)
    } else if row == end.1 {
        (left, end.0.saturating_add(1))
    } else {
        (left, right)
    };
    (c0.clamp(left, right), c1.clamp(left, right))
}

fn apply_text_selection(f: &mut Frame, app: &mut App) {
    let Some(sel) = app.text_selection else {
        return;
    };
    let Some(area) = *app.messages_rect.borrow() else {
        app.text_selection = None;
        return;
    };
    if area.width < 3 || area.height < 3 {
        // Too small to extract or highlight; don't leave a finalize request
        // (or a persisted post-copy highlight) stuck on the state forever.
        if sel.finalize || sel.copied {
            app.text_selection = None;
        }
        return;
    }
    // Body bounds inside the rounded border (1-cell border + the scrollbar
    // gutter on the right are excluded so we don't copy frame glyphs).
    // The transcript is borderless now: content fills the area top-to-bottom,
    // inset 1 col by horizontal padding, with the scrollbar on the last column.
    let top = area.y;
    let bottom = area.y + area.height; // exclusive (last content row = bottom-1)
    let left = area.x.saturating_add(1); // horizontal padding
    let right = area.x + area.width - 1; // exclusive (scrollbar column)

    let (start, end) = sel.ordered();
    let r0 = start.1.max(top);
    let r1 = end.1.min(bottom.saturating_sub(1));
    if r0 > r1 {
        if sel.finalize || sel.copied {
            app.text_selection = None;
        }
        return;
    }

    if sel.finalize {
        let mut rows: Vec<String> = Vec::new();
        {
            let buf = f.buffer_mut();
            let bounds = *buf.area();
            for row in r0..=r1 {
                let (c0, c1) = selection_row_span(row, start, end, left, right);
                let mut line = String::new();
                for col in c0..c1 {
                    if col < bounds.right() && row < bounds.bottom() {
                        line.push_str(buf[(col, row)].symbol());
                    }
                }
                rows.push(line.trim_end().to_string());
            }
        }
        let text = rows.join("\n");
        if text.trim().is_empty() {
            // Nothing to copy or keep highlighted (blank-area drag).
            app.text_selection = None;
            return;
        }
        crate::runtime::copy_to_clipboard(&text, "select");
        jfc_engine::toast::push_with_cap(
            &mut app.engine.toasts,
            jfc_engine::toast::Toast::new(
                jfc_engine::toast::ToastKind::Info,
                format!("Copied {} chars", text.chars().count()),
            ),
        );
        // Persist the highlight (copied=true) so the user sees what was
        // copied; cleared on the next mouse-down, scroll, Esc, or resize. Fall
        // through to paint the highlight this same frame.
        app.text_selection = Some(crate::app::TextSelection {
            finalize: false,
            copied: true,
            dragged: false,
            ..sel
        });
    }

    // Live highlight: paint the covered cells with the theme's selection
    // background, leaving each cell's foreground intact (a solid bg reads as
    // one contiguous band, whereas SGR reverse fragments over syntax colors).
    let sel_bg = app.theme.selection_bg();
    let buf = f.buffer_mut();
    let bounds = *buf.area();
    for row in r0..=r1 {
        let (c0, c1) = selection_row_span(row, start, end, left, right);
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
    let right = area.x + area.width - 1; // exclusive (scrollbar column)
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
    app.text_selection = Some(crate::app::TextSelection {
        anchor: (anchor_col, row),
        head: (head_col, row),
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
    use super::{is_word_char, word_span_in_row};

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

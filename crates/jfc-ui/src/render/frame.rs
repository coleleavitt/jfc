use super::approval::approval;
use super::input_box::{input, input_visual_line_count};
use super::messages::messages;
use super::messages::{agent_fan_below_input, subagent_footer};
use super::messages::{spinner_row, tasks_pinned_row};
use super::model_picker::model_picker;
use super::overlays::{
    diagnostic_panel, diagnostic_row, help_overlay, mention_popup, search_bar, slash_popup,
    toast_overlay,
};
use super::palette::palette;
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
    let total_w_pre = f.area().width as usize;
    let input_content_w = total_w_pre.saturating_sub(6);
    let input_lines = input_visual_line_count(app, input_content_w);
    let input_height = (input_lines + 2).min(8) as u16;
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
    let any_alive_subagent = app.background_tasks.values().any(|bt| bt.status.is_alive());
    let show_spinner = app.is_streaming
        || app.compacting_started_at.is_some()
        || !app.pending_tool_calls.is_empty()
        || app.turn_started_at.is_some()
        || any_alive_subagent;
    // When a team is active, the spinner area expands to show the teammate tree:
    // 2 base rows (spinner + next-task hint) + 1 leader row + N teammate rows.
    // For non-team parallel subagents (the "fire 5 Explore agents" case),
    // expand for the same reason — the user sees one row per agent.
    let teammate_count = if app.team_context.is_active() {
        app.team_context.teammates.len().saturating_sub(1) // exclude leader
    } else {
        0
    };
    let active_subagent_count = if !app.team_context.is_active() {
        // Count both Running and Idle teammates: Idle ones still belong
        // on the fan (the user can SendMessage to wake them) so the
        // tree row needs to be reserved for them.
        app.background_tasks
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
    let spinner_row_height: u16 = if show_spinner { 2 } else { 0 };
    // Pinned todo list above the input, Claude-Code style. Header row
    // ("Tasks (k/n done)") + up to task_pin_visible rows + an optional
    // "+N more" footer. Height collapses to 0 when no tasks exist so
    // first-run UI stays clean.
    let tp_all = app.task_store.list(jfc_session::DeletedFilter::Exclude);
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
            app.task_completion_times
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
    let task_pin_rows = if tp_open == 0 && tp_recent_done == 0 {
        0
    } else {
        let body = (tp_open + tp_recent_done).min(task_pin_visible);
        let overflow = if tp_open + tp_recent_done > task_pin_visible {
            1
        } else {
            0
        };
        body + overflow
    };
    let tasks_pinned_height: u16 = if task_pin_rows > 0 {
        (task_pin_rows as u16).min(10) + 2
    } else {
        0
    };
    // Agent fan beneath the input: leader row ("agents") plus one row
    // per alive sub-agent. Capped at 8 so a fan of 30 doesn't push the
    // status bar off-screen — the user can still open the task view to
    // see all of them.
    let agent_fan_height: u16 = if tree_rows > 0 {
        (1 + tree_rows as u16).min(8) + 2
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
    let unack_count =
        crate::diagnostics::unacknowledged(&app.diagnostics, &app.delivered_diagnostics).len();
    let diag_row_height: u16 = if unack_count == 0 { 0 } else { 1 };

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(3),
            Constraint::Length(subagent_footer_height),
            Constraint::Length(diag_row_height),
            Constraint::Length(spinner_row_height),
            Constraint::Length(tasks_pinned_height),
            Constraint::Length(input_height),
            Constraint::Length(agent_fan_height),
            Constraint::Length(2),
        ])
        .split(f.area());

    // TODO: Wire sidebar_progress to App animation fields once Agent A adds them.
    // For now, snap to 0.0/1.0 so the ease_out_cubic path is exercised but
    // the visual result is identical to the old binary toggle.
    let sidebar_progress: f32 = if app.show_sidebar { 1.0 } else { 0.0 };
    let show_right = app.show_info_sidebar && f.area().width >= 100;

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

    match (show_left, show_right) {
        (true, true) => {
            let split = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([
                    Constraint::Length(left_w),
                    Constraint::Min(20),
                    Constraint::Length(right_w),
                ])
                .split(chunks[0]);
            sidebar(f, app, split[0]);
            messages(f, app, split[1]);
            info_sidebar(f, app, split[2]);
        }
        (true, false) => {
            let split = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Length(left_w), Constraint::Min(20)])
                .split(chunks[0]);
            sidebar(f, app, split[0]);
            messages(f, app, split[1]);
        }
        (false, true) => {
            let split = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Min(20), Constraint::Length(right_w)])
                .split(chunks[0]);
            messages(f, app, split[0]);
            info_sidebar(f, app, split[1]);
        }
        (false, false) => {
            messages(f, app, chunks[0]);
        }
    }

    if app.viewing_task_id.is_some() {
        subagent_footer(f, app, chunks[1]);
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
    input(f, app, chunks[5]);
    if agent_fan_height > 0 {
        agent_fan_below_input(f, app, chunks[6]);
    }
    status(f, app, chunks[7]);

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

    if !app.toasts.is_empty() {
        toast_overlay(f, app);
    }

    if app.mention.active && !app.mention.candidates.is_empty() {
        mention_popup(f, app, chunks[4]);
    }

    if app.show_diagnostic_panel && !app.diagnostics.is_empty() {
        diagnostic_panel(f, app);
    }

    if app.show_help {
        help_overlay(f, app);
    }

    if app.transcript_search.is_some() {
        search_bar(f, app);
    }

    // Slash-command autocomplete: opens above the input bar when
    // the user has typed `/<prefix>` and there are matching commands.
    if let Some(prefix) = current_slash_prefix(app) {
        slash_popup(f, app, &prefix);
    }

    if app.pending_approval.is_some() {
        approval(f, app);
    }
}

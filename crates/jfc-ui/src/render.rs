use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Position, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, Padding, Paragraph},
};

mod approval;
mod model_picker;
mod palette;
mod session_sidebar;
mod status;
mod task_panel;
mod theme_picker;

#[allow(unused_imports)]
use ratatui::style::Stylize as _;

use crate::app::App;
use crate::markdown;
use crate::theme::Theme;
use crate::types::*;

use approval::approval;
use model_picker::model_picker;
#[cfg(test)]
use model_picker::{provider_color, provider_label};
use palette::palette;
pub use session_sidebar::ordered_sidebar_sessions;
use session_sidebar::sidebar;
#[cfg(test)]
use status::context_gauge_label;
use status::{claude_status_footer, effort_status_badge, status};
use task_panel::{task_model_badge, task_panel};
use theme_picker::theme_picker;

/// Easing function for sidebar slide animation.
fn ease_out_cubic(t: f32) -> f32 {
    1.0 - (1.0 - t).powi(3)
}

/// One full UI redraw. The hot path — runs every Tick (~80ms) and
/// on every input event. Tracing here is at TRACE level so it's
/// off in normal runs but easy to flip on with
/// `RUST_LOG=jfc::render=trace cargo run` when diagnosing layout
/// math, scroll behavior, or animation timing bugs.
#[tracing::instrument(
    target = "jfc::render",
    level = "trace",
    skip_all,
    fields(
        w = f.area().width,
        h = f.area().height,
        msgs = app.messages.len(),
        streaming = app.is_streaming,
        scroll = app.scroll_offset,
        follow_bottom = app.follow_bottom,
    ),
)]
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
    let spinner_row_height: u16 = if show_spinner {
        if tree_rows > 0 {
            (3 + tree_rows as u16).min(10) // cap at 10 to avoid starving chat
        } else {
            2
        }
    } else if tree_rows > 0 {
        // Show the tree even when not streaming (background work in flight)
        (2 + tree_rows as u16).min(8)
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
            Constraint::Length(input_height),
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
    if show_spinner || tree_rows > 0 {
        spinner_row(f, app, chunks[3]);
    }
    input(f, app, chunks[4]);
    status(f, app, chunks[5]);

    if app.show_palette {
        palette(f, app);
    }

    if app.show_theme_picker {
        theme_picker(f, app);
    }

    if app.show_model_picker {
        model_picker(f, app);
    }

    if app.show_task_panel {
        task_panel(f, app);
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

fn info_sidebar(f: &mut Frame, app: &mut App, area: Rect) {
    use crate::types::LspStatus;

    let t = app.theme;

    let block = Block::default()
        .borders(Borders::LEFT)
        .border_style(t.style_border)
        .padding(Padding::new(1, 0, 1, 0)); // left=1, right=0, top=1, bottom=0
    let inner = block.inner(area);
    f.render_widget(block, area);

    let mut lines: Vec<Line> = Vec::new();

    // Helper: section header. Bold + text_primary (white). Earlier
    // I had this as bold + accent (cyan), but every section header
    // pulling the same accent color flattened the hierarchy — your
    // eye couldn't tell a structural section title from a value
    // that *also* used accent (e.g. the model-name sub-header). The
    // three-tier rule wins: primary white bold for sections, accent
    // for sub-headings/values, muted for body. Mirrors how Claude
    // Code's actual sidebar treats its section labels (cli.js'
    // panel renderers use `text` color + bold for the header line,
    // reserving accent for interactive or live elements).
    let section = |label: &'static str| -> Line<'static> {
        Line::from(vec![Span::styled(
            label,
            Style::default()
                .fg(t.text_primary)
                .add_modifier(Modifier::BOLD),
        )])
    };

    lines.push(section("Session"));

    let title = app
        .current_session_id
        .as_ref()
        .map(|s| s.as_str())
        .unwrap_or("untitled")
        .to_owned();
    lines.push(Line::from(vec![Span::styled(
        truncate_str(&title, inner.width as usize),
        Style::default().fg(t.text_secondary),
    )]));

    lines.push(Line::from(""));

    lines.push(section("Context"));

    // Always render the calibrated `approx_tokens` (input + output +
    // cache_read + cache_write) — that's what `recompute_token_estimate`
    // / StreamUsage / compaction all use. Previously this took
    // `max(last_usage_input, approx_tokens)`, which was a no-op (approx
    // always ≥ input alone) but obscured the fact that the sidebar and
    // bottom-bar gauge were already computing the same thing two
    // different ways, leaving a maintenance footgun where one could
    // drift from the other.
    let total_tokens = app.tool_ctx.approx_tokens as u64;
    let ctx_max = app.selected_context_window_tokens().max(1) as u64;
    let pct = (total_tokens as f64 / ctx_max as f64 * 100.0).min(100.0);

    lines.push(Line::from(vec![
        Span::styled(
            format!("{} tokens", fmt_number(total_tokens)),
            Style::default().fg(t.text_secondary),
        ),
        Span::styled(
            format!(" · {:.0}%", pct),
            Style::default().fg(gauge_color(pct, t)),
        ),
    ]));

    let bar_width = inner.width.saturating_sub(2) as usize;
    if bar_width > 4 {
        let filled = ((pct / 100.0) * bar_width as f64).round() as usize;
        let filled = filled.min(bar_width);
        lines.push(Line::from(vec![
            Span::styled("█".repeat(filled), Style::default().fg(gauge_color(pct, t))),
            Span::styled(
                "░".repeat(bar_width - filled),
                Style::default().fg(t.border),
            ),
        ]));
    }

    let out_tokens = app.last_usage_output;
    if out_tokens > 0 {
        lines.push(Line::from(vec![Span::styled(
            format!("{} output", fmt_number(out_tokens as u64)),
            Style::default().fg(t.text_muted),
        )]));
    }

    // Per-turn token sparkline rendered inline under the Context
    // section so it reads as part of *that* group instead of a
    // disconnected widget glued to the bottom of the panel. Uses
    // the unicode block-element scale `▁▂▃▄▅▆▇█` so we can render
    // it as a styled span rather than a separate Sparkline widget.
    if app.token_history.len() >= 2 {
        const BARS: &[char] = &['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
        let max_val = app.token_history.iter().copied().max().unwrap_or(1).max(1);
        let bar_width = (inner.width as usize).min(app.token_history.len());
        // Take the most recent N values so a long history doesn't
        // squish the recent samples into single-cell averages.
        let start = app.token_history.len().saturating_sub(bar_width);
        let bars: String = app
            .token_history
            .iter()
            .skip(start)
            .map(|v| {
                let idx =
                    (((*v as f64) / max_val as f64) * (BARS.len() - 1) as f64).round() as usize;
                BARS[idx.min(BARS.len() - 1)]
            })
            .collect();
        lines.push(Line::from(vec![
            Span::styled("tok/turn ", Style::default().fg(t.text_muted)),
            Span::styled(bars, Style::default().fg(t.accent)),
        ]));
    }

    let total_cache_read: u64 = app
        .usage_by_model
        .values()
        .map(|u| u.cache_read_tokens)
        .sum();
    let total_input: u64 = app.usage_by_model.values().map(|u| u.input_tokens).sum();
    if total_cache_read > 0 && total_input > 0 {
        let global_hit_pct = (total_cache_read as f64 / total_input as f64 * 100.0).min(100.0);
        lines.push(Line::from(vec![
            Span::styled("cache hit: ", Style::default().fg(t.text_muted)),
            Span::styled(
                format!("{:.0}%", global_hit_pct),
                Style::default().fg(t.success),
            ),
        ]));
    }

    lines.push(Line::from(""));

    if !app.usage_by_model.is_empty() {
        lines.push(section("Usage by model"));

        let mut model_entries: Vec<(&String, &crate::types::ModelUsage)> =
            app.usage_by_model.iter().collect();
        model_entries.sort_by_key(|(k, _)| k.as_str());

        for (model_name, usage) in &model_entries {
            // Model name is a sub-heading: accent color (cyan) but
            // NOT bold, so it visibly demotes below the section
            // header (`Usage by model` in white bold). Three weights
            // — section / sub / body — read as a clear ladder.
            lines.push(Line::from(vec![Span::styled(
                format!(
                    " {}:",
                    truncate_str(model_name, inner.width.saturating_sub(2) as usize)
                ),
                Style::default().fg(t.accent),
            )]));

            lines.push(Line::from(vec![Span::styled(
                format!(
                    "  {} in, {} out",
                    fmt_number(usage.input_tokens),
                    fmt_number(usage.output_tokens),
                ),
                Style::default().fg(t.text_muted),
            )]));

            if usage.cache_read_tokens > 0 || usage.cache_write_tokens > 0 {
                lines.push(Line::from(vec![Span::styled(
                    format!(
                        "  {} cache read, {} write",
                        fmt_number(usage.cache_read_tokens),
                        fmt_number(usage.cache_write_tokens),
                    ),
                    Style::default().fg(t.text_muted),
                )]));

                let hit_pct = usage.cache_hit_pct();
                if hit_pct > 0.0 {
                    lines.push(Line::from(vec![
                        Span::styled("  cache hit: ", Style::default().fg(t.text_muted)),
                        Span::styled(format!("{:.0}%", hit_pct), Style::default().fg(t.success)),
                    ]));
                }
            }

            if let Some(cost) = usage.cost_usd {
                lines.push(Line::from(vec![Span::styled(
                    format!("  ${:.2} spent", cost),
                    Style::default().fg(t.text_secondary),
                )]));
            }
        }

        let total = crate::cost::total_cost(&app.usage_by_model);
        // Hide the cost line on free / unauthenticated runs (matches
        // the status bar's gate at >$0.001). Showing `Total cost:
        // $0.00` on every fresh session was visual noise — the line
        // only earns its row once there's a cost to talk about.
        if total > 0.001 {
            lines.push(Line::from(vec![Span::styled(
                format!("Total cost: {}", crate::cost::fmt_cost(total)),
                Style::default().fg(t.text_muted),
            )]));
        }

        lines.push(Line::from(""));
    }

    lines.push(section("LSP"));

    if app.lsp_servers.is_empty() {
        // Wrap the placeholder line manually based on the inner
        // sidebar width — the parent Paragraph doesn't wrap, so a
        // verbose hint like "LSPs will activate as files are read"
        // got hard-clipped at the column boundary as `… are rea`.
        // Word-wrap into one or more rows so the message is readable.
        for row in wrap_text_to_width("LSPs will activate as files are read", inner.width as usize)
        {
            lines.push(Line::from(vec![Span::styled(
                row,
                Style::default().fg(t.text_muted),
            )]));
        }
    } else {
        for srv in &app.lsp_servers {
            let (dot_color, label) = match srv.status {
                LspStatus::Active => (t.success, "Active"),
                LspStatus::Inactive => (t.text_muted, "Inactive"),
            };
            lines.push(Line::from(vec![
                Span::styled("• ", Style::default().fg(dot_color)),
                Span::styled(
                    truncate_str(&srv.name, inner.width.saturating_sub(12) as usize),
                    Style::default().fg(t.accent),
                ),
                Span::raw(" "),
                Span::styled(label, Style::default().fg(dot_color)),
            ]));
        }
    }

    lines.push(Line::from(""));

    // MCP section — v126 cli.js renders MCP server status alongside LSP.
    // Layout mirrors the LSP block above: bold header, one row per server
    // formatted as `<dot> <name>`, blank separator below.
    lines.push(section("MCP"));

    if app.mcp_servers.is_empty() {
        for row in wrap_text_to_width("No MCP servers configured", inner.width as usize) {
            lines.push(Line::from(vec![Span::styled(
                row,
                Style::default().fg(t.text_muted),
            )]));
        }
    } else {
        for srv in &app.mcp_servers {
            lines.push(Line::from(vec![
                Span::styled("● ", Style::default().fg(mcp_status_color(srv.status, t))),
                Span::styled(
                    truncate_str(&srv.name, inner.width.saturating_sub(2) as usize),
                    Style::default().fg(t.text_secondary),
                ),
            ]));
        }
    }

    lines.push(Line::from(""));

    // Team section - show active teammates. Single-blank separator
    // is enough; the section() helper's gutter glyph already gives
    // the eye an anchor, no need for a double-row break.
    if app.team_context.is_active() {
        lines.push(section("Team"));

        if let Some(ref team_name) = app.team_context.team_name {
            lines.push(Line::from(vec![Span::styled(
                format!("  {team_name}"),
                Style::default().fg(t.text_secondary),
            )]));
        }

        // Surface each teammate as one row. Color the active-marker
        // dot with the teammate's assigned palette color (mirrors the
        // teammate-tree below) so the team panel and the spinner-row
        // tree read the same way.
        for info in app.team_context.teammates.values() {
            if info.name == crate::swarm::TEAM_LEAD_NAME {
                continue;
            }
            let dot_color = crate::swarm::types::teammate_color(info.color.as_deref());
            lines.push(Line::from(vec![
                Span::styled("  ● ", Style::default().fg(dot_color)),
                Span::styled(&info.name, Style::default().fg(t.text_secondary)),
            ]));
        }

        if app.team_context.teammates.len() <= 1 {
            lines.push(Line::from(vec![Span::styled(
                "  (no teammates)",
                Style::default().fg(t.text_secondary),
            )]));
        }
    }

    // Tasks section - show pending/in-progress todos
    let tasks = app.task_store.list(crate::tasks::DeletedFilter::Exclude);
    let pending: Vec<_> = tasks
        .iter()
        .filter(|t| t.status == crate::tasks::TaskStatus::Pending)
        .collect();
    let in_progress: Vec<_> = tasks
        .iter()
        .filter(|t| t.status == crate::tasks::TaskStatus::InProgress)
        .collect();
    let completed: Vec<_> = tasks
        .iter()
        .filter(|t| t.status == crate::tasks::TaskStatus::Completed)
        .collect();

    let task_total = pending.len() + in_progress.len() + completed.len();
    if task_total > 0 {
        // Match the rest of the sidebar: white bold for the section
        // header, accent reserved for sub-elements like the in-progress
        // task indicator below.
        lines.push(Line::from(vec![Span::styled(
            format!("Tasks ({}/{} done)", completed.len(), task_total),
            Style::default()
                .fg(t.text_primary)
                .add_modifier(Modifier::BOLD),
        )]));

        // Show in-progress tasks with activity
        for task in &in_progress {
            let model_badge = task_model_badge(task);
            let activity = app
                .task_activities
                .get(&task.id)
                .map(|s| truncate_str(s, inner.width.saturating_sub(6) as usize))
                .unwrap_or_default();
            lines.push(Line::from(vec![
                Span::styled("◆ ", Style::default().fg(t.accent)),
                Span::styled(
                    truncate_str(&task.subject, inner.width.saturating_sub(4) as usize),
                    Style::default()
                        .fg(t.text_primary)
                        .add_modifier(Modifier::BOLD),
                ),
            ]));
            if !activity.is_empty() {
                lines.push(Line::from(vec![Span::styled(
                    format!("  {}", activity),
                    Style::default().fg(t.text_muted),
                )]));
            }
            if let Some(model) = model_badge {
                lines.push(Line::from(vec![Span::styled(
                    format!("  model {model}"),
                    Style::default().fg(t.text_muted),
                )]));
            }
        }

        // Show all pending tasks (no cap)
        for task in &pending {
            let blocked = !task.blocked_by.is_empty();
            let icon = if blocked { "○" } else { "◇" };
            let color = if blocked {
                t.text_muted
            } else {
                t.text_secondary
            };
            lines.push(Line::from(vec![
                Span::styled(format!("{} ", icon), Style::default().fg(color)),
                Span::styled(
                    truncate_str(&task.subject, inner.width.saturating_sub(4) as usize),
                    Style::default().fg(color),
                ),
            ]));
        }

        // Recently completed tasks (fade out after 30s)
        let now = std::time::Instant::now();
        let recent_completed: Vec<_> = completed
            .iter()
            .filter(|task| {
                app.task_completion_times
                    .get(&task.id)
                    .is_some_and(|t| now.duration_since(*t).as_secs() < 30)
            })
            .take(2)
            .collect();

        for task in recent_completed {
            lines.push(Line::from(vec![
                Span::styled("✓ ", Style::default().fg(t.success)),
                Span::styled(
                    truncate_str(&task.subject, inner.width.saturating_sub(4) as usize),
                    Style::default()
                        .fg(t.text_muted)
                        .add_modifier(Modifier::CROSSED_OUT),
                ),
            ]));
        }

        lines.push(Line::from(""));
    }

    // Diffs section - count files with edit/write tool outputs
    let diff_stats = collect_diff_stats(app);
    if diff_stats.total_files > 0 {
        lines.push(Line::from(vec![Span::styled(
            "Changes",
            Style::default()
                .fg(t.text_primary)
                .add_modifier(Modifier::BOLD),
        )]));

        lines.push(Line::from(vec![
            Span::styled(
                format!("{} file(s)", diff_stats.total_files),
                Style::default().fg(t.text_secondary),
            ),
            Span::raw(" "),
            Span::styled(
                format!("+{}", diff_stats.additions),
                Style::default().fg(t.success),
            ),
            Span::styled(" ", Style::default()),
            Span::styled(
                format!("-{}", diff_stats.deletions),
                Style::default().fg(t.error),
            ),
        ]));

        // Show up to 3 most recently modified files
        for file in diff_stats.files.iter().take(3) {
            lines.push(Line::from(vec![
                Span::styled("  ", Style::default()),
                Span::styled(
                    truncate_str(file, inner.width.saturating_sub(4) as usize),
                    Style::default().fg(t.accent),
                ),
            ]));
        }
        if diff_stats.files.len() > 3 {
            lines.push(Line::from(vec![Span::styled(
                format!("  … +{} more", diff_stats.files.len() - 3),
                Style::default().fg(t.text_muted),
            )]));
        }

        lines.push(Line::from(""));
    }

    // Footer rows (divider + cwd + provider) build separately and
    // render in a fixed bottom strip. Earlier we padded the body
    // with blank rows to push the footer down — that worked when
    // content was short but wasted vertical space and looked like a
    // gap. Now we use a Layout split: body gets `Min(0)` so it
    // takes whatever's left, footer is a `Length(3)` strip pinned
    // to the bottom. Naturally collapses on a tall panel without
    // manual padding.
    let mut footer_lines: Vec<Line> = Vec::new();
    footer_lines.push(Line::from(vec![Span::styled(
        "─".repeat(inner.width as usize),
        Style::default().fg(t.border),
    )]));

    // Same per-frame `getcwd` cleanup as the ribbon path: lean on the cached
    // `app.cwd` so the sidebar footer doesn't re-syscall on every redraw.
    let cwd_str = if app.cwd.is_empty() {
        "?".to_owned()
    } else {
        let home = std::env::var("HOME").unwrap_or_default();
        if !home.is_empty() && app.cwd.starts_with(&home) {
            format!("~{}", &app.cwd[home.len()..])
        } else {
            app.cwd.clone()
        }
    };
    let cwd_display = tail_truncate(&cwd_str, inner.width.saturating_sub(2) as usize);
    footer_lines.push(Line::from(vec![
        Span::styled("⌂ ", Style::default().fg(t.text_muted)),
        Span::styled(cwd_display, Style::default().fg(t.text_muted)),
    ]));

    let provider_name = app.provider.name();
    let effort_badge = effort_status_badge(app);
    let fast_badge = if app.fast_mode { " · ⚡ FAST" } else { "" };
    let claude_status = claude_status_footer(app);
    let provider_suffix = format!(" local · {effort_badge}{fast_badge}{claude_status}");
    let provider_width = inner
        .width
        .saturating_sub(2 + provider_suffix.chars().count() as u16)
        .max(1) as usize;
    footer_lines.push(Line::from(vec![
        Span::styled("● ", Style::default().fg(t.success)),
        Span::styled(
            truncate_str(provider_name, provider_width),
            Style::default()
                .fg(t.text_primary)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(provider_suffix, Style::default().fg(t.text_muted)),
    ]));

    let footer_h: u16 = 3;
    let footer_h = footer_h.min(inner.height);
    let body_h = inner.height.saturating_sub(footer_h);
    let body_area = Rect {
        x: inner.x,
        y: inner.y,
        width: inner.width,
        height: body_h,
    };
    let footer_area = Rect {
        x: inner.x,
        y: inner.y + body_h,
        width: inner.width,
        height: footer_h,
    };
    f.render_widget(
        Paragraph::new(lines).style(Style::default().bg(t.bg)),
        body_area,
    );
    f.render_widget(
        Paragraph::new(footer_lines).style(Style::default().bg(t.bg)),
        footer_area,
    );
}

/// Apply rainbow-gradient highlighting to slash-command and @mention
/// tokens in an input line. Plain prose stays in `text_primary`; only
/// the special-routing tokens light up. Conservative on what counts:
/// only the *first* slash-command at the line start, plus `@` tokens
/// (file mentions), avoid coloring inline `/` chars in URLs etc.
fn input_line_to_spans(line: &str, t: Theme, phase: f32) -> Vec<Span<'static>> {
    if line.is_empty() {
        return vec![Span::raw("")];
    }
    let trimmed_start = line.trim_start();
    let leading_ws = line.len() - trimmed_start.len();
    let starts_with_slash = trimmed_start.starts_with('/');
    let mut spans: Vec<Span<'static>> = Vec::new();

    if leading_ws > 0 {
        spans.push(Span::raw(line[..leading_ws].to_string()));
    }

    if starts_with_slash {
        // Find end of the slash-command token (next whitespace).
        let token_end = trimmed_start
            .find(char::is_whitespace)
            .unwrap_or(trimmed_start.len());
        let token = &trimmed_start[..token_end];
        for (i, ch) in token.chars().enumerate() {
            let hue = (phase + i as f32 * 18.0) % 360.0;
            let (r, g, b) = crate::spinner::hue_to_rgb(hue);
            spans.push(Span::styled(
                ch.to_string(),
                Style::default()
                    .fg(Color::Rgb(r, g, b))
                    .add_modifier(Modifier::BOLD),
            ));
        }
        let rest = &trimmed_start[token_end..];
        if !rest.is_empty() {
            spans.extend(highlight_mentions_in(rest, t, phase));
        }
    } else {
        spans.extend(highlight_mentions_in(trimmed_start, t, phase));
    }
    spans
}

/// Tokenize prose, color any `@token` (mention) with the same rainbow
/// gradient as the leading slash command, but with a phase offset so
/// each mention reads as its own colored token rather than blending
/// in with the slash prefix.
fn highlight_mentions_in(s: &str, t: Theme, phase: f32) -> Vec<Span<'static>> {
    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut buf = String::new();
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '@' && (i == 0 || chars[i - 1].is_whitespace()) {
            if !buf.is_empty() {
                spans.push(Span::styled(std::mem::take(&mut buf), t.style_text_primary));
            }
            // Consume the `@` and the following non-whitespace token.
            let mut token = String::from('@');
            i += 1;
            while i < chars.len() && !chars[i].is_whitespace() {
                token.push(chars[i]);
                i += 1;
            }
            for (j, ch) in token.chars().enumerate() {
                let hue = (phase + 60.0 + j as f32 * 18.0) % 360.0;
                let (r, g, b) = crate::spinner::hue_to_rgb(hue);
                spans.push(Span::styled(
                    ch.to_string(),
                    Style::default()
                        .fg(Color::Rgb(r, g, b))
                        .add_modifier(Modifier::BOLD),
                ));
            }
        } else {
            buf.push(c);
            i += 1;
        }
    }
    if !buf.is_empty() {
        spans.push(Span::styled(buf, t.style_text_primary));
    }
    spans
}

/// Enumerate every cell along the border of `area` in clockwise
/// order, starting at the top-left corner. Used by the border-comet
/// painter to walk the perimeter at a steady speed regardless of
/// rect aspect ratio.
fn perimeter_cells(area: Rect) -> Vec<(u16, u16)> {
    // Per-frame cache: the input dock + status bar reuse the same Rect on
    // back-to-back frames. Without this, every frame allocated and filled a
    // ~2 × (w + h) Vec just to walk the same perimeter — pure waste during
    // idle/streaming where geometry is fixed. Invalidate on Rect change
    // (resize, layout shift); LRU-of-1 is enough since paint_border_comets
    // is the only non-test caller.
    thread_local! {
        static LAST: std::cell::RefCell<Option<(Rect, Vec<(u16, u16)>)>> =
            const { std::cell::RefCell::new(None) };
    }
    LAST.with(|slot| {
        let mut slot = slot.borrow_mut();
        if let Some((cached_area, cached_cells)) = slot.as_ref()
            && *cached_area == area
        {
            return cached_cells.clone();
        }
        let cells = compute_perimeter_cells(area);
        *slot = Some((area, cells.clone()));
        cells
    })
}

fn compute_perimeter_cells(area: Rect) -> Vec<(u16, u16)> {
    let mut cells: Vec<(u16, u16)> = Vec::new();
    if area.width < 2 || area.height < 2 {
        return cells;
    }
    let right = area.x + area.width - 1;
    let bottom = area.y + area.height - 1;
    for x in area.x..=right {
        cells.push((x, area.y));
    }
    for y in (area.y + 1)..=bottom {
        cells.push((right, y));
    }
    if right > area.x {
        for x in (area.x..right).rev() {
            cells.push((x, bottom));
        }
    }
    if bottom > area.y + 1 {
        for y in ((area.y + 1)..bottom).rev() {
            cells.push((area.x, y));
        }
    }
    cells
}

/// Configuration for `paint_border_comets`. All knobs that callers
/// might want to vary at runtime live here so the painter stays
/// declarative — pass a struct, get a render.
struct CometConfig {
    /// Number of comets evenly spaced around the perimeter. 1..=4.
    count: u32,
    /// Lap duration in ms — full perimeter traversal time. Lower
    /// = faster comets. Drives by streaming velocity in the input
    /// renderer; can be hard-overridden via env.
    lap_ms: u128,
    /// Trail length in cells. 6 is the standard comet shape.
    trail_len: usize,
    /// Resting border color (the comet fades to this at the tail end).
    base: Color,
    /// Comet head color (the lead cell blends fully to this).
    head: Color,
    /// When true, comets at odd indices counter-rotate (go
    /// counter-clockwise) so a count=2 setup produces two comets
    /// going opposite directions, meeting at corners.
    counter_rotate: bool,
    /// Reverse the clockwise base direction. Combined with
    /// `counter_rotate`, this lets the tool-use signal flip every
    /// comet's direction at once.
    reverse_base: bool,
}

/// Paint N border comets traveling around the rectangle's perimeter
/// at a steady speed. Each comet is a `trail_len`-cell trail (head
/// at brightest blend toward `head` color, tail fading to `base`).
fn paint_border_comets(f: &mut Frame, area: Rect, cfg: &CometConfig) {
    // O(1) early exit: skip perimeter computation and buffer writes when
    // there is no animation to show (no comets configured, or zero-length
    // trail). Callers pass count=0 when neither streaming nor compaction
    // is active, so this is the common idle-frame path.
    if cfg.count == 0 || cfg.trail_len == 0 {
        return;
    }
    let perim = perimeter_cells(area);
    if perim.is_empty() {
        return;
    }
    let total = perim.len();
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let head_pos_signed = ((now_ms * total as u128) / cfg.lap_ms.max(1)) as i64;

    let buf = f.buffer_mut();

    for c in 0..cfg.count {
        // Direction: even-indexed comets follow the base direction;
        // odd-indexed comets reverse if `counter_rotate` is set.
        // `reverse_base` flips the base on top of that.
        let counter = cfg.counter_rotate && c % 2 == 1;
        let direction_positive = match (cfg.reverse_base, counter) {
            (false, false) => true,
            (true, false) => false,
            (false, true) => false,
            (true, true) => true,
        };
        // Even spacing around the perimeter.
        let offset = (c as usize * total) / cfg.count.max(1) as usize;
        // Position of this comet's head this frame.
        let head_idx = if direction_positive {
            ((head_pos_signed + offset as i64).rem_euclid(total as i64)) as usize
        } else {
            ((-head_pos_signed + offset as i64).rem_euclid(total as i64)) as usize
        };
        for trail in 0..cfg.trail_len {
            // Trail cells trail "behind" the head along its
            // direction of travel.
            let pos = if direction_positive {
                (head_idx + total - trail) % total
            } else {
                (head_idx + trail) % total
            };
            let (x, y) = perim[pos];
            if x >= buf.area().right() || y >= buf.area().bottom() {
                continue;
            }
            // Squared falloff: head bright, tail dies off quickly.
            let pct = trail as f32 / cfg.trail_len as f32;
            let intensity = (1.0 - pct).powi(2);
            let blended = pulse_color(cfg.base, cfg.head, intensity);
            let cell = &mut buf[(x, y)];
            let mut style = cell.style();
            style.fg = Some(blended);
            cell.set_style(style);
        }
    }
}

/// Compute the comet config from the current app state. Centralizes
/// all the "what color, what speed, which direction" logic in one
/// place so the input renderer just calls this once.
fn comet_config_from_state(app: &App, t: Theme, count: u32) -> CometConfig {
    // Bash-mode detection: the user is composing a shell command
    // (input starts with `!`). Mirrors v126's bash-mode prompt
    // indicator. Color goes warning so the comets clearly signal
    // "this isn't a normal prompt".
    let bash_mode = app
        .textarea
        .lines()
        .iter()
        .next()
        .map(|line| line.trim_start().starts_with('!'))
        .unwrap_or(false);

    // Tool-use detection: any tool currently `Running` in the most
    // recent assistant turn (the streaming placeholder OR the last
    // committed message). Drives the reverse-direction +
    // warning-color override so the user sees "the model is
    // executing something" at a glance.
    let any_tool_running = app.messages.iter().rev().take(2).any(|m| {
        m.parts.iter().any(|p| {
            if let MessagePart::Tool(tc) = p {
                matches!(tc.status, ToolStatus::Running | ToolStatus::Pending)
            } else {
                false
            }
        })
    }) || !app.pending_tool_calls.is_empty();

    let head_color = if bash_mode {
        // Bash mode trumps tool-use coloring — it's the highest-
        // signal state because it's the user's explicit choice.
        t.warning
    } else if any_tool_running {
        t.warning
    } else {
        t.accent
    };

    // Speed = streaming velocity. Compute a rough tokens/sec rate
    // from the cumulative output and the turn elapsed time. Map to
    // a lap_ms with a few buckets so the speed change is
    // perceptible (smooth interpolation reads as "did it just
    // change?"). Resting (idle) sits at 3500ms.
    let now = std::time::Instant::now();
    let elapsed = app
        .turn_started_at
        .or(app.streaming_started_at)
        .map(|t0| now.duration_since(t0))
        .unwrap_or_default();
    let secs = elapsed.as_secs_f64().max(0.5);
    let live = app
        .last_usage_output
        .max((app.streaming_response_bytes / 4) as u32);
    let rate = (live as f64) / secs;
    let mut lap_ms: u128 = if !app.is_streaming {
        3500
    } else if rate > 60.0 {
        1200 // hot: fast laps
    } else if rate > 30.0 {
        2000 // warm
    } else {
        3500 // cold / first chunks
    };
    // Hard env override wins regardless.
    if let Some(forced) = std::env::var("JFC_BORDER_COMET_SPEED")
        .ok()
        .and_then(|s| s.parse::<u128>().ok())
    {
        lap_ms = forced.max(200);
    }

    let trail_len: usize = std::env::var("JFC_BORDER_COMET_TRAIL")
        .ok()
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(6)
        .clamp(2, 12);

    // Counter-rotation is opt-in (matches v126's "two flames
    // chasing" pattern). Off by default — single-direction reads
    // calmer for an idle prompt.
    let counter_rotate = matches!(
        std::env::var("JFC_BORDER_COMET_COUNTER").as_deref(),
        Ok("1") | Ok("true")
    );

    CometConfig {
        count,
        lap_ms,
        trail_len,
        base: t.border,
        head: head_color,
        counter_rotate,
        // Tool-use reverses the base direction so the comets visibly
        // change which way they're going — strong signal that the
        // model is doing something the user can't see (running a
        // tool offscreen or in a long bash).
        reverse_base: any_tool_running && !bash_mode,
    }
}

/// Prompt-character animation mode. Selects which glyph (or glyph
/// cycle) appears at the start of the input. Picked by parsing the
/// `JFC_PROMPT_CHAR` env var: a leading `:` denotes a named animation
/// preset; anything else is treated as a literal char.
#[derive(Clone, Debug)]
enum PromptMode {
    Comet,
    Moon,
    Dice,
    Notes,
    Hourglass,
    Atom,
    /// Static literal — user picked a single char (e.g. `JFC_PROMPT_CHAR=⌬`).
    Static(String),
}

fn parse_prompt_mode(raw: &str) -> PromptMode {
    let trimmed = raw.trim();
    match trimmed {
        ":comet" => PromptMode::Comet,
        ":moon" | ":moons" | ":moon_phases" => PromptMode::Moon,
        ":dice" | ":die" => PromptMode::Dice,
        ":notes" | ":music" => PromptMode::Notes,
        ":hourglass" | ":time" => PromptMode::Hourglass,
        ":atom" => PromptMode::Atom,
        s if !s.is_empty() && s.chars().count() <= 2 => PromptMode::Static(s.to_owned()),
        _ => PromptMode::Comet,
    }
}

/// Pick the glyph for this frame given the mode + wall-clock + state.
fn prompt_mode_frame(mode: &PromptMode, streaming: bool, ms: u128) -> &'static str {
    match mode {
        PromptMode::Comet => "☄",
        PromptMode::Atom => "⚛",
        PromptMode::Moon => {
            // 8-frame waxing/waning cycle that mirrors actual moon
            // phase order. Uses 1-cell symbolic glyphs (not emoji)
            // so ratatui's column tracking stays accurate. Idle
            // settles on full moon (`●`) — most "present" looking.
            if !streaming {
                return "●";
            }
            const FRAMES: &[&str] = &["○", "◐", "●", "◑"];
            FRAMES[((ms / 250) as usize) % FRAMES.len()]
        }
        PromptMode::Dice => {
            // Dice rolling at 120ms/face for a fast shuffle that
            // reads as "the model is thinking, anything could come
            // out". Idle lands on ⚀ so the prompt is visually
            // stable when nothing is happening.
            if !streaming {
                return "⚀";
            }
            const FACES: &[&str] = &["⚀", "⚁", "⚂", "⚃", "⚄", "⚅"];
            FACES[((ms / 120) as usize) % FACES.len()]
        }
        PromptMode::Notes => {
            // Music-note cycle at 280ms/note — slightly slower so
            // each glyph reads. Idle settles on ♪ (eighth note) as
            // the most "musical" looking single character.
            if !streaming {
                return "♪";
            }
            const NOTES: &[&str] = &["♩", "♪", "♫", "♬"];
            NOTES[((ms / 280) as usize) % NOTES.len()]
        }
        PromptMode::Hourglass => {
            // Flip every 800ms — `⌛` (sand running) → `⌚` (drained
            // / time face). Slow enough to read each state. Idle
            // shows the full hourglass.
            if !streaming {
                return "⌛";
            }
            if (ms / 800) % 2 == 0 { "⌛" } else { "⌚" }
        }
        PromptMode::Static(_) => {
            // Static is handled via fallback below (returns the
            // user-supplied char). Sentinel here for the type to
            // line up; the input renderer reads
            // `prompt_mode_frame_static` for this branch.
            ""
        }
    }
}

/// Public form for cross-module callers (sparkle in message_view, etc.)
/// — the private `pulse_color` is preferred inside this file for
/// brevity.
pub fn pulse_color_pub(c1: Color, c2: Color, t: f32) -> Color {
    pulse_color(c1, c2, t)
}

/// Linear-interpolate between two ratatui Colors at `t ∈ [0, 1]`.
/// Falls back to the start color when either endpoint isn't an RGB
/// triple (named ANSI colors don't have a useful midpoint). Used by
/// the spinner pulse to blend the lead glyph between accent and
/// warning across each animation cycle.
fn pulse_color(c1: Color, c2: Color, t: f32) -> Color {
    let (r1, g1, b1) = match c1 {
        Color::Rgb(r, g, b) => (r, g, b),
        _ => return c1,
    };
    let (r2, g2, b2) = match c2 {
        Color::Rgb(r, g, b) => (r, g, b),
        _ => return c1,
    };
    let (r, g, b) = crate::spinner::interpolate_rgb((r1, g1, b1), (r2, g2, b2), t);
    Color::Rgb(r, g, b)
}

fn gauge_color(pct: f64, t: crate::theme::Theme) -> Color {
    if pct >= 85.0 {
        t.error
    } else if pct >= 60.0 {
        t.warning
    } else {
        t.success
    }
}

fn fmt_number(n: u64) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else if n >= 1_000 {
        let s = n.to_string();
        let mut out = String::with_capacity(s.len() + s.len() / 3);
        for (i, c) in s.chars().rev().enumerate() {
            if i > 0 && i % 3 == 0 {
                out.push(',');
            }
            out.push(c);
        }
        out.chars().rev().collect()
    } else {
        n.to_string()
    }
}

/// Aggregate edit/write diff stats across the whole conversation for the
/// sidebar "Changes" section. Walks every Tool message part, picks up
/// `ToolOutput::Diff(_)` payloads (Edit/Write tools convert their result
/// into a unified diff at parse time — see `types.rs::ToolOutput::Diff`),
/// and de-duplicates files by their last-seen entry so the most recent
/// edit wins. Files appear in *most-recent-first* order to match how the
/// chat scrolls.
#[derive(Clone)]
pub(crate) struct DiffStats {
    total_files: usize,
    additions: usize,
    deletions: usize,
    files: Vec<String>,
}

/// Cached wrapper around the full diff-stats walk.
///
/// Complexity reduction: O(N_messages × N_parts) → O(1) cache hit on
/// unchanged state. Invalidates when `messages.len()` or the total
/// number of message parts changes (new message appended, or a tool
/// result added to an in-flight assistant message). The key computation
/// is O(N_messages) but touches only `.parts.len()` — no content
/// inspection — so it's negligible compared to the full HashMap walk.
fn collect_diff_stats(app: &App) -> DiffStats {
    let msg_count = app.messages.len();
    let total_parts: usize = app.messages.iter().map(|m| m.parts.len()).sum();

    {
        let cache = app.diff_stats_cache.borrow();
        if let Some((cached_msgs, cached_parts, ref stats)) = *cache {
            if cached_msgs == msg_count && cached_parts == total_parts {
                return stats.clone();
            }
        }
    }

    let stats = compute_diff_stats(app);
    *app.diff_stats_cache.borrow_mut() = Some((msg_count, total_parts, stats.clone()));
    stats
}

/// Inner computation for `collect_diff_stats`. Walks all messages and
/// parts to build the de-duplicated diff summary.
fn compute_diff_stats(app: &App) -> DiffStats {
    let mut by_file: std::collections::HashMap<String, (usize, usize)> =
        std::collections::HashMap::new();
    let mut order: Vec<String> = Vec::new();
    for msg in &app.messages {
        for part in &msg.parts {
            if let MessagePart::Tool(call) = part {
                if let ToolOutput::Diff(view) = &call.output {
                    let entry = by_file.entry(view.file_path.clone()).or_insert((0, 0));
                    *entry = (view.additions, view.deletions);
                    if !order.contains(&view.file_path) {
                        order.push(view.file_path.clone());
                    }
                }
            }
        }
    }
    // Reverse so most-recently-touched files appear first.
    order.reverse();
    let (additions, deletions) = by_file
        .values()
        .fold((0usize, 0usize), |(a, d), (na, nd)| (a + na, d + nd));
    DiffStats {
        total_files: by_file.len(),
        additions,
        deletions,
        files: order,
    }
}

fn mcp_status_color(status: McpStatus, theme: Theme) -> Color {
    match status {
        McpStatus::Connected => theme.success,
        McpStatus::Disabled => theme.text_muted,
        McpStatus::Error => theme.error,
    }
}

fn truncate_str(s: &str, max: usize) -> String {
    if max == 0 {
        return String::new();
    }
    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= max {
        s.to_owned()
    } else {
        let trunc: String = chars[..max.saturating_sub(1)].iter().collect();
        format!("{}…", trunc)
    }
}

/// Like `truncate_str` but clips from the *front*, prepending `…/`
/// so the meaningful tail (project name in a path, identifier in a
/// long namespace) survives. Used by the sidebar's cwd display so
/// the user sees `…/active/jfc` on a narrow column rather than the
/// useless `~/RustProjec…` head.
fn tail_truncate(s: &str, max: usize) -> String {
    if max == 0 {
        return String::new();
    }
    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= max {
        return s.to_owned();
    }
    // Reserve 2 cells for the leading "…/" indicator. If the column
    // is too narrow even for that, fall back to head truncation.
    if max < 4 {
        return truncate_str(s, max);
    }
    let tail_len = max.saturating_sub(2);
    let start = chars.len() - tail_len;
    let tail: String = chars[start..].iter().collect();
    format!("…/{}", tail.trim_start_matches('/'))
}

/// Word-wrap a short prose string to a column-width. Used by the
/// info-sidebar's empty-state hints (e.g. "LSPs will activate as
/// files are read") that the parent Paragraph doesn't auto-wrap. A
/// hard ratatui clip would chop mid-word at the right edge; this
/// breaks on whitespace so each row is a complete fragment. Returns
/// at least one row even for empty input so callers can always
/// `.push(Line::from(row))`.
fn wrap_text_to_width(s: &str, width: usize) -> Vec<String> {
    if width == 0 {
        return vec![String::new()];
    }
    let mut out: Vec<String> = Vec::new();
    let mut current = String::new();
    for word in s.split_whitespace() {
        let word_len = word.chars().count();
        if word_len >= width {
            // Single-word overflow: hard-truncate that word with an
            // ellipsis. Better than letting it bleed off the edge.
            if !current.is_empty() {
                out.push(std::mem::take(&mut current));
            }
            out.push(truncate_str(word, width));
            continue;
        }
        let projected = if current.is_empty() {
            word_len
        } else {
            current.chars().count() + 1 + word_len
        };
        if projected > width {
            out.push(std::mem::take(&mut current));
            current.push_str(word);
        } else {
            if !current.is_empty() {
                current.push(' ');
            }
            current.push_str(word);
        }
    }
    if !current.is_empty() {
        out.push(current);
    }
    if out.is_empty() {
        out.push(String::new());
    }
    out
}

/// Render the messages column. Computes `total_lines` at the same
/// width MessageView will actually render at, sets `scroll_offset`
/// (pinning to the bottom when `follow_bottom` is true), and hands
/// off to `MessageView`. Bug-prone arithmetic — every column the
/// width is off by triggers a follow-bottom miscount, so this fn
/// gets the most tracing.
#[tracing::instrument(
    target = "jfc::render::messages",
    level = "trace",
    skip(f, app),
    fields(
        x = area.x,
        y = area.y,
        w = area.width,
        h = area.height,
    ),
)]
fn messages(f: &mut Frame, app: &mut App, area: Rect) {
    use crate::message_view::MessageView;
    use ratatui::widgets::Widget;

    // Record area for the mouse handler (drag-scroll target).
    *app.messages_rect.borrow_mut() = Some(area);
    let t = app.theme;

    if let Some(ref task_id) = app.viewing_task_id.clone() {
        messages_task_view(f, app, area, task_id);
        return;
    }

    // Reserve the scrollbar's 1-cell column up front so the
    // total-lines computation uses the SAME width MessageView will
    // actually render at. Earlier we computed total at full inner
    // width and then chopped 1 col when the scrollbar showed —
    // long lines wrapped at the smaller width during render but
    // weren't counted in the wider-width total, so `follow_bottom`
    // pinned to a position that still left the true last row
    // offscreen until the next chunk's recompute caught up.
    //
    // Always reserving the column is cheap (1 col) and makes the
    // scroll math consistent across "needs scrolling vs doesn't"
    // states. A pure visual cost when no scrollbar is visible:
    // ~1.5% of a 60-col message column.
    //
    // Total horizontal overhead for the message box:
    //   borders (1 left + 1 right)  = 2
    //   padding (1 left + 1 right)  = 2
    //   scrollbar reserve           = 1
    //                         total  = 5
    let inner_width = area.width.saturating_sub(5) as usize;

    // Build render items ONCE per frame and share them with `MessageView::render`.
    // Pre-fix this function called `message_view_total_lines` (one
    // `build_render_items` walk) and the widget then ran `build_render_items`
    // again — gdb sampling showed the second walk's `Vec<Line<'static>>::to_vec`
    // out of `RenderCache` was the dominant remaining hot spot once syntect/onig
    // and the tool-height path were memoized. Sharing one items vec halves the
    // per-frame deep-clone work.
    //
    // The earlier `app.total_lines` cache that gated `message_view_total_lines`
    // is no longer needed — items are required for paint anyway, and
    // `tool_block_height` now memoizes the integer height per terminal-state
    // tool, so the per-item .sum() is a string of hash lookups.
    let render_ctx = crate::message_view::RenderCtx::from_app(app);
    let items = crate::message_view::build_render_items_pub(&render_ctx, inner_width);
    let total_lines: usize = items.iter().map(|i| i.height(inner_width)).sum();

    let visible = area.height.saturating_sub(2) as usize;

    // Compute the new scroll offset locally — `items` borrows from `app`, so we
    // can't write `app.scroll_offset` until after `MessageView::render` consumes
    // them. The new value is also passed into `PrebuiltItems` so the widget
    // sees it during paint instead of the (still-old) `app.scroll_offset`.
    let scroll_before = app.scroll_offset;
    let new_scroll_offset = if app.follow_bottom {
        total_lines.saturating_sub(visible)
    } else if app.scroll_offset + visible > total_lines {
        total_lines.saturating_sub(visible)
    } else {
        app.scroll_offset
    };
    // Trace the scroll math result. Bug class this catches: when
    // `total_lines` is undercounted (width mismatch), `scroll_offset`
    // gets pinned to a value smaller than the true bottom row,
    // leaving the latest content offscreen. Compare `total_lines`
    // here against actual rendered height to spot off-by-N errors.
    tracing::trace!(
        target: "jfc::render::scroll",
        inner_width,
        total_lines,
        visible,
        scroll_before,
        scroll_after = new_scroll_offset,
        follow_bottom = app.follow_bottom,
        "messages scroll math"
    );

    // Mirror `App::is_at_bottom` against the freshly-computed values so the
    // overflow indicator reflects the post-render state, not last frame's.
    let at_bottom = new_scroll_offset >= total_lines.saturating_sub(visible.max(1));
    let title_right = if !at_bottom {
        let remaining = total_lines.saturating_sub(new_scroll_offset + visible);
        format!(" ↓ {remaining} more ")
    } else {
        String::new()
    };

    // No left-side title, no breathing animation. The frame is
    // just a static rounded border with 1-cell horizontal padding
    // so prose doesn't kiss the border. The right-side overflow
    // indicator (`↓ N more`) still surfaces when the user has
    // scrolled up.
    //
    // (Earlier this border pulsed `t.border ↔ t.accent` on a 1.5s
    // loop while streaming. Removed at user request — the spinner
    // row already signals streaming activity, the breathing
    // border was decoration on top.)
    let border_color = t.border;
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(ratatui::widgets::BorderType::Rounded)
        .border_style(Style::default().fg(border_color))
        .padding(Padding::horizontal(1))
        .title_top(Line::from(Span::styled(title_right, t.style_text_muted)).right_aligned())
        .style(Style::default().bg(t.bg));

    let inner = block.inner(area);
    f.render_widget(block, area);

    // Snapshot the values we'll need to commit back to App after `items` is
    // dropped. The placeholder branch doesn't consume `items`, so we commit
    // *after* the if/else with an explicit `drop(items)`.
    let totals_to_commit = (
        total_lines,
        (app.messages.len(), app.streaming_text.len(), inner_width),
        visible,
        new_scroll_offset,
    );

    if app.messages.is_empty() && app.streaming_text.is_empty() {
        // Boot sweep: for the first ~1.4s after launch, ripple a star
        // cascade across the placeholder so the empty session has a
        // moment of life. After the sweep settles, the placeholder
        // reads as a calm muted prompt. Reduced-motion skips
        // straight to the settled state.
        let boot_age = app.launched_at.elapsed();
        let boot_active =
            boot_age < std::time::Duration::from_millis(1400) && !crate::spinner::reduced_motion();
        const HEADLINE: &str = "What can I help you with?";
        let headline_spans: Vec<Span<'static>> = if boot_active {
            // Sweep one bright cell across the headline. Cell width
            // sweeps left-to-right in 1100ms, then a 300ms tail
            // settles. Lit cell uses accent + bold; the rest stays
            // text_muted.
            let sweep_progress = (boot_age.as_millis() as f32 / 1100.0).min(1.0);
            let cursor = (sweep_progress * HEADLINE.chars().count() as f32) as i32;
            HEADLINE
                .chars()
                .enumerate()
                .map(|(i, ch)| {
                    let dist = (i as i32 - cursor).abs();
                    let style = if dist <= 1 {
                        t.style_accent_bold
                    } else {
                        t.style_text_muted
                    };
                    Span::styled(ch.to_string(), style)
                })
                .collect()
        } else {
            vec![Span::styled(
                HEADLINE.to_string(),
                Style::default().fg(t.text_muted),
            )]
        };
        let placeholder = Paragraph::new(vec![
            Line::from(""),
            Line::from(headline_spans),
            Line::from(""),
            Line::from(Span::styled(
                "  ?    keybindings",
                Style::default().fg(t.text_muted),
            )),
            Line::from(Span::styled(
                "  Ctrl+P    palette · Ctrl+M    model picker",
                Style::default().fg(t.text_muted),
            )),
        ])
        .style(Style::default().bg(t.bg));
        f.render_widget(placeholder, inner);
    } else {
        // Reserve a 1-col gutter on the right for the scrollbar
        // ALWAYS (not just when scrollbar is visible). The total-
        // lines computation above uses width-5 (border + padding +
        // scrollbar) so the rendering must use the same width or the
        // scroll math gets off-by-N when the gutter goes from
        // "absent" to "present" mid-stream.
        let scrollbar_visible = total_lines > visible && visible > 0;
        let content_inner = Rect {
            width: inner.width.saturating_sub(1),
            ..inner
        };
        MessageView {
            app,
            prebuilt: Some(crate::message_view::PrebuiltItems {
                items,
                total_h: total_lines,
                scroll: new_scroll_offset,
            }),
        }
        .render(content_inner, f.buffer_mut());

        if scrollbar_visible {
            // ratatui::widgets::Scrollbar drives off ScrollbarState
            // (content length, position, viewport length). Mapping
            // jfc's existing `scroll_offset / total_lines` straight
            // in. The thumb is bound to the body region (excluding
            // top+bottom borders) by passing `area` (the bordered
            // block) and using `Vertical-Right` orientation.
            use ratatui::prelude::StatefulWidget;
            use ratatui::widgets::{Scrollbar, ScrollbarOrientation, ScrollbarState};
            let mut state = ScrollbarState::new(total_lines.saturating_sub(visible))
                .position(new_scroll_offset)
                .viewport_content_length(visible);
            let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
                .begin_symbol(Some("▲"))
                .end_symbol(Some("▼"))
                .thumb_symbol("█")
                .track_symbol(Some("│"))
                .style(t.style_text_muted)
                .thumb_style(t.style_accent);
            scrollbar.render(area, f.buffer_mut(), &mut state);
        }

        // Token rain: a single cell at the bottom-right of the
        // border that lights up briefly each time a token arrives.
        // Reads as a tiny pulse counter — the user can see *that
        // tokens are flowing* without staring at the verb. Renders
        // only while streaming (idle = dark cell so it doesn't add
        // visual noise to a settled session). Reduced-motion skips
        // entirely so the cell stays at the static border glyph.
        if app.is_streaming
            && !crate::spinner::reduced_motion()
            && area.height >= 2
            && area.width >= 2
        {
            if let Some(when) = app.last_token_arrival {
                let age_ms = when.elapsed().as_millis() as f32;
                if age_ms < 800.0 {
                    let intensity = 1.0 - (age_ms / 800.0);
                    let cx = area.x + area.width.saturating_sub(1);
                    let cy = area.y + area.height.saturating_sub(2);
                    if cx < f.buffer_mut().area().right() && cy < f.buffer_mut().area().bottom() {
                        let cell = &mut f.buffer_mut()[(cx, cy)];
                        cell.set_symbol("●");
                        let blended = pulse_color(t.border, t.accent, intensity);
                        cell.set_style(Style::default().fg(blended));
                    }
                }
            }
        }
    }

    // Commit the freshly-computed values back to App. By this point both
    // branches above have finished rendering and any borrow of `app` via the
    // items vec is dropped. `App::max_scroll` (used by event-loop key
    // handlers) reads these — staling them by a frame caused PgDn at end-of-
    // buffer to silently no-op while still feeling laggy.
    let (total_lines_v, total_lines_key_v, viewport_h_v, scroll_v) = totals_to_commit;
    app.total_lines = total_lines_v;
    app.total_lines_key = total_lines_key_v;
    app.viewport_height = viewport_h_v;
    app.scroll_offset = scroll_v;
}

/// Per-entry collapse threshold for the subagent task view. A single
/// `BackgroundTask.messages[i]` longer than this (line count) renders as a
/// 5-line preview + a muted "press o to expand" footer until the user toggles
/// it via `viewing_task_expanded`. Smaller than `LargeText::COLLAPSE_LINES`
/// because subagent entries are *individual* turn outputs, not whole tool
/// results — 80 lines is already a wall in a narrow drilled-in pane.
pub(crate) const TASK_VIEW_COLLAPSE_LINES: usize = 80;
/// Per-entry byte threshold for the subagent task view. Mirrors the line
/// threshold's reasoning at 5 KB — typical 200-line file dumps blow past this
/// long before they hit `LargeText`'s 30 KB ceiling.
pub(crate) const TASK_VIEW_COLLAPSE_BYTES: usize = 5 * 1024;
/// Number of leading lines preserved when an entry collapses. Mirrors v126's
/// `Read` tool preview length so the user gets enough context to decide
/// whether to expand.
const TASK_VIEW_COLLAPSE_PREVIEW_LINES: usize = 5;

/// Render `BackgroundTask.messages` to ratatui `Line`s the same way the main
/// chat handles assistant text: each raw string flows through
/// `markdown::to_lines`, which calls `strip_inline_tool_xml` internally so
/// `<tool_call>…</tool_call>` and `<tool_result>…</tool_result>` markers
/// don't bleed into the screen as literal angle brackets, and code fences
/// pick up syntect highlighting.
///
/// Long entries (>80 lines or >5 KB raw) collapse to a 5-line preview + a
/// muted `… N more lines · press o to expand` row unless their index is in
/// `expanded`. Pure function so tests can assert behavior without standing
/// up a `Frame`/`Buffer`.
///
/// TODO Phase B: when `BackgroundTask.messages` migrates to
/// `Vec<ChatMessage>`, this helper collapses into the same `MessageView`
/// pipeline the main chat uses, picking up tool blocks, reasoning collapse,
/// and diff rendering for free.
pub(crate) fn task_view_body_lines(
    messages: &[String],
    expanded: &std::collections::HashSet<usize>,
    theme: &Theme,
    inner_width: usize,
    task_done: bool,
) -> Vec<Line<'static>> {
    let mut out: Vec<Line<'static>> = Vec::new();
    for (idx, raw) in messages.iter().enumerate() {
        let line_count = raw.lines().count();
        // For finished tasks, never auto-collapse — the whole point
        // of opening the task view is to see the result. Only running
        // tasks (whose output is still streaming) get the threshold.
        let collapsible = !task_done
            && (line_count > TASK_VIEW_COLLAPSE_LINES || raw.len() > TASK_VIEW_COLLAPSE_BYTES);
        let is_expanded = expanded.contains(&idx);

        if collapsible && !is_expanded {
            // Truncate the raw string to the first N lines *before* feeding
            // it to the markdown renderer — letting `to_lines` produce 80
            // wrapped lines and then slicing produces visually-broken
            // output (e.g. half a code fence). Slicing the source keeps
            // markdown structure intact.
            let preview: String = raw
                .lines()
                .take(TASK_VIEW_COLLAPSE_PREVIEW_LINES)
                .collect::<Vec<_>>()
                .join("\n");
            let mut preview_lines = markdown::to_lines(&preview, theme, inner_width);
            out.append(&mut preview_lines);
            let hidden = line_count.saturating_sub(TASK_VIEW_COLLAPSE_PREVIEW_LINES);
            out.push(Line::from(Span::styled(
                format!("… {hidden} more lines · press o to expand"),
                Style::default().fg(theme.text_muted),
            )));
        } else {
            let mut lines = markdown::to_lines(raw, theme, inner_width);
            out.append(&mut lines);
        }
    }
    out
}

fn messages_task_view(f: &mut Frame, app: &mut App, area: Rect, task_id: &str) {
    let t = app.theme;
    // Reserve same width as the main view: borders(2) + padding(2) + scrollbar(1) = 5
    let inner_width = area.width.saturating_sub(5) as usize;

    let (title_str, body_lines, use_message_view) = match app.background_tasks.get(task_id) {
        None => (format!("task {task_id} (not found)"), Vec::new(), false),
        Some(bt) => {
            let title = format!(
                " {} · {} ",
                &bt.task_id.as_str()[..bt.task_id.as_str().len().min(12)],
                bt.description
            );
            // Use the rich MessageView pipeline when we have structured messages.
            // Fall back to the markdown string renderer for tasks that have no
            // chat_messages yet (e.g. daemon-launched detached agents whose events
            // only arrive as TaskProgress strings).
            let use_mv = !bt.chat_messages.is_empty();
            if use_mv {
                (title, Vec::new(), true)
            } else {
                static EMPTY: std::sync::OnceLock<std::collections::HashSet<usize>> =
                    std::sync::OnceLock::new();
                let empty = EMPTY.get_or_init(std::collections::HashSet::new);
                let expanded = app.viewing_task_expanded.get(task_id).unwrap_or(empty);
                let task_done = matches!(bt.status, crate::types::TaskLifecycle::Completed);
                let lines =
                    task_view_body_lines(&bt.messages, expanded, &t, inner_width, task_done);
                (title, lines, false)
            }
        }
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(t.style_accent)
        .title(Span::styled(title_str, t.style_accent_bold))
        .style(Style::default().bg(t.bg));

    let inner = block.inner(area);
    f.render_widget(block, area);

    let task_status = app.background_tasks.get(task_id).map(|bt| bt.status);
    let task_is_running = matches!(task_status, Some(crate::types::TaskLifecycle::Running));
    let task_is_idle = matches!(task_status, Some(crate::types::TaskLifecycle::Idle));

    // While the task is still running, append a spinner+"Receiving…"
    // row so the user can tell at a glance that more output is on
    // the way (vs. a frozen panel). The frame index pulls from the
    // same wall-clock source as `tool_status_icon_animated` so the
    // glyph rotates in lockstep with the running-tool bullet.
    //
    // For Idle teammates, swap the live spinner for a static "⏸ idle"
    // hint so the user can tell the difference between "still
    // streaming" and "agent finished its turn, waiting for next ping"
    // without staring at the panel for a few seconds.
    let visible = inner.height as usize;

    if use_message_view {
        // Rich MessageView path — same pipeline as the main chat.
        use crate::message_view::{MessageView, PrebuiltItems, RenderCtx, build_render_items_ctx};
        use ratatui::widgets::Widget;

        let chat_msgs = app
            .background_tasks
            .get(task_id)
            .map(|bt| bt.chat_messages.as_slice())
            .unwrap_or(&[]);

        // Compute scroll BEFORE borrowing app through items, then assign after.
        let total_lines_est = {
            let msgs = app
                .background_tasks
                .get(task_id)
                .map(|bt| bt.chat_messages.as_slice())
                .unwrap_or(&[]);
            let ctx = RenderCtx::from_task(msgs, app);
            let est_items = build_render_items_ctx(&ctx, inner_width);
            est_items
                .iter()
                .map(|i| i.height(inner_width))
                .sum::<usize>()
        };
        let new_scroll = if app.follow_bottom {
            total_lines_est.saturating_sub(visible)
        } else if app.scroll_offset + visible > total_lines_est {
            total_lines_est.saturating_sub(visible)
        } else {
            app.scroll_offset
        };
        app.scroll_offset = new_scroll;
        app.total_lines = total_lines_est;
        app.viewport_height = visible;

        // Now build items for real (same data, but app.scroll_offset is now settled).
        let ctx = RenderCtx::from_task(chat_msgs, app);
        let items = build_render_items_ctx(&ctx, inner_width);
        let mv = MessageView {
            app,
            prebuilt: Some(PrebuiltItems {
                items,
                total_h: total_lines_est,
                scroll: new_scroll,
            }),
        };
        mv.render(inner, f.buffer_mut());

        // Spinner / idle hint: paint it below the MessageView content
        // in whatever space remains (or overlap the last row if full).
        if task_is_running || task_is_idle {
            let frame = (app.launched_at.elapsed().as_millis() / 80) as usize;
            let hint_line = if task_is_running {
                let spinner_glyph = crate::app::SPINNER[frame % crate::app::SPINNER.len()];
                Line::from(vec![
                    Span::styled(
                        spinner_glyph.to_string(),
                        Style::default().fg(t.accent).add_modifier(Modifier::BOLD),
                    ),
                    Span::raw("  "),
                    Span::styled("Receiving output…", Style::default().fg(t.text_muted)),
                ])
            } else {
                Line::from(vec![
                    Span::styled("⏸  ", Style::default().fg(t.text_muted)),
                    Span::styled(
                        "idle — waiting for next message",
                        Style::default()
                            .fg(t.text_muted)
                            .add_modifier(Modifier::ITALIC),
                    ),
                ])
            };
            // Render the hint in a 1-row strip at the bottom of the inner area.
            if inner.height >= 1 {
                let hint_area = Rect::new(inner.x, inner.y + inner.height - 1, inner.width, 1);
                f.render_widget(
                    Paragraph::new(hint_line).style(Style::default().bg(t.bg)),
                    hint_area,
                );
            }
        }
    } else {
        // Legacy string-log path — used for daemon-launched agents whose
        // events only arrive as TaskProgress strings with no structured data.
        let mut body_lines = body_lines;
        if task_is_running {
            let frame = (app.launched_at.elapsed().as_millis() / 80) as usize;
            let spinner_glyph = crate::app::SPINNER[frame % crate::app::SPINNER.len()];
            if !body_lines.is_empty() {
                body_lines.push(Line::from(""));
            }
            body_lines.push(Line::from(vec![
                Span::styled(
                    spinner_glyph.to_string(),
                    Style::default().fg(t.accent).add_modifier(Modifier::BOLD),
                ),
                Span::raw("  "),
                Span::styled("Receiving output…", Style::default().fg(t.text_muted)),
            ]));
        } else if task_is_idle {
            if !body_lines.is_empty() {
                body_lines.push(Line::from(""));
            }
            body_lines.push(Line::from(vec![
                Span::styled("⏸  ", Style::default().fg(t.text_muted)),
                Span::styled(
                    "idle — waiting for next message",
                    Style::default()
                        .fg(t.text_muted)
                        .add_modifier(Modifier::ITALIC),
                ),
            ]));
        }

        let render_width = inner.width;
        let total_lines: usize = body_lines
            .iter()
            .map(|line| {
                if line.width() == 0 || render_width == 0 {
                    1
                } else {
                    Paragraph::new(line.clone())
                        .wrap(ratatui::widgets::Wrap { trim: false })
                        .line_count(render_width)
                        .max(1)
                }
            })
            .sum();

        if app.follow_bottom {
            app.scroll_offset = total_lines.saturating_sub(visible);
        } else if app.scroll_offset + visible > total_lines {
            app.scroll_offset = total_lines.saturating_sub(visible);
        }
        app.total_lines = total_lines;
        app.viewport_height = visible;

        if body_lines.is_empty() {
            let placeholder_text = if task_is_running {
                "Waiting for first chunk…"
            } else {
                "No messages yet for this background task."
            };
            let placeholder = Paragraph::new(vec![
                Line::from(""),
                Line::from(Span::styled(
                    placeholder_text,
                    Style::default().fg(t.text_muted),
                )),
            ])
            .style(Style::default().bg(t.bg));
            f.render_widget(placeholder, inner);
        } else {
            let para = Paragraph::new(body_lines)
                .style(Style::default().bg(t.bg))
                .wrap(ratatui::widgets::Wrap { trim: false })
                .scroll((app.scroll_offset as u16, 0));
            f.render_widget(para, inner);
        }
    }
}

fn subagent_footer(f: &mut Frame, app: &App, area: Rect) {
    use ratatui::widgets::Tabs;
    let t = app.theme;
    // Show one tab per running BackgroundTask. Selected tab tracks
    // `viewing_task_id`. Hint row sits below the tabs so the user
    // sees both `← →` cycling and the `↑` exit at a glance — the
    // previous one-line `[1 of N] ◀ back ▶ next` collapsed both
    // navigation and identity into a string that scanned poorly with
    // 5+ tasks.
    let task_ids: Vec<String> = app.background_tasks.keys().cloned().collect();
    if task_ids.is_empty() {
        f.render_widget(
            Paragraph::new(Line::from(vec![Span::styled(
                "↑ back  · no tasks",
                Style::default().fg(t.text_muted),
            )]))
            .style(Style::default().bg(t.bg)),
            area,
        );
        return;
    }
    let selected = app
        .viewing_task_id
        .as_ref()
        .and_then(|id| task_ids.iter().position(|t| t == id))
        .unwrap_or(0);
    let titles: Vec<Line> = task_ids
        .iter()
        .map(|id| {
            let bt = app.background_tasks.get(id);
            let desc = bt.map(|b| b.description.as_str()).unwrap_or(id.as_str());
            let trimmed = if desc.chars().count() > 24 {
                let mut s: String = desc.chars().take(23).collect();
                s.push('…');
                s
            } else {
                desc.to_owned()
            };
            // Status glyph: animated for Running, static for Completed/Failed.
            let glyph = match bt.map(|b| &b.status) {
                Some(crate::types::TaskLifecycle::Running) => {
                    let frame = (app.launched_at.elapsed().as_millis() / 240) as usize;
                    ["✶", "✷", "✸", "✹"][frame % 4]
                }
                Some(crate::types::TaskLifecycle::Completed) => "●",
                _ => "○",
            };
            Line::from(vec![Span::raw(glyph), Span::raw(" "), Span::raw(trimmed)])
        })
        .collect();

    let split = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Length(1)])
        .split(area);

    let tabs = Tabs::new(titles)
        .select(selected)
        .style(t.style_text_secondary.bg(t.bg))
        .highlight_style(
            t.style_accent
                .bg(t.surface_raised)
                .add_modifier(Modifier::BOLD),
        )
        .divider(Span::styled("·", t.style_text_muted))
        .padding(" ", " ");
    f.render_widget(tabs, split[0]);

    let hint = Line::from(vec![Span::styled(
        "↑ back · ←/→ cycle · ↓ jump to latest",
        Style::default().fg(t.text_muted),
    )]);
    f.render_widget(
        Paragraph::new(hint).style(Style::default().bg(t.bg)),
        split[1],
    );
}

/// Pick the next open task to surface under the spinner — first
/// in-progress task wins, falling back to the first pending task.
/// Mirrors v126 cli.js:323851 (`m` = next task) which indents
/// `Next: ${m.subject}` underneath the spinner verb. Returns `None`
/// when the task list is empty so the renderer can shrink to a 1-row
/// spinner instead of leaving a blank second line.
fn next_open_task_subject(app: &App) -> Option<String> {
    use crate::tasks::DeletedFilter;
    let tasks = app.task_store.list(DeletedFilter::Exclude);
    pick_next_open_task(&tasks).map(|t| t.subject.clone())
}

/// Pure priority picker for the "Next: …" sub-status. In-progress wins
/// over pending so users see *what's running right now* rather than
/// *what's queued*. Falls back to the first pending when nothing is
/// active. Returns `None` when nothing is open. Extracted from
/// `next_open_task_subject` so unit tests can exercise the priority
/// rules without building an `App` fixture.
fn pick_next_open_task(tasks: &[crate::tasks::Task]) -> Option<&crate::tasks::Task> {
    use crate::tasks::TaskStatus;
    tasks
        .iter()
        .find(|t| matches!(t.status, TaskStatus::InProgress))
        .or_else(|| {
            tasks
                .iter()
                .find(|t| matches!(t.status, TaskStatus::Pending))
        })
}

#[cfg(test)]
mod next_task_tests {
    use super::*;
    use crate::tasks::{DeletedFilter, TaskStore};

    #[test]
    fn empty_store_returns_none_normal() {
        let store = TaskStore::in_memory();
        let tasks = store.list(DeletedFilter::Exclude);
        assert!(pick_next_open_task(&tasks).is_none());
    }

    #[test]
    fn single_pending_task_picked_normal() {
        let store = TaskStore::in_memory();
        store
            .create(
                "Wire spinner".into(),
                String::new(),
                None,
                Vec::<String>::new(),
            )
            .unwrap();
        let tasks = store.list(DeletedFilter::Exclude);
        let picked = pick_next_open_task(&tasks).expect("should pick the pending task");
        assert_eq!(picked.subject, "Wire spinner");
    }

    #[test]
    fn in_progress_wins_over_pending_normal() {
        // v126's `Next: ${m.subject}` shows the *active* task, not the
        // queued one — what's running matters more than what's queued.
        let store = TaskStore::in_memory();
        let pending = store
            .create(
                "First (pending)".into(),
                String::new(),
                None,
                Vec::<String>::new(),
            )
            .unwrap();
        let active = store
            .create(
                "Second (will be in-progress)".into(),
                String::new(),
                None,
                Vec::<String>::new(),
            )
            .unwrap();
        store
            .update(
                active.id.as_str(),
                crate::tasks::TaskPatch {
                    status: Some(crate::tasks::TaskStatus::InProgress),
                    ..Default::default()
                },
            )
            .unwrap();
        let tasks = store.list(DeletedFilter::Exclude);
        let picked = pick_next_open_task(&tasks).expect("in-progress should win");
        assert_eq!(picked.subject, "Second (will be in-progress)");
        // Sanity: the pending task IS in the list, just not picked.
        assert!(
            tasks.iter().any(|t| t.id.as_str() == pending.id.as_str()),
            "pending task should still be in the list"
        );
    }

    #[test]
    fn only_completed_returns_none_robust() {
        let store = TaskStore::in_memory();
        let t = store
            .create(
                "Done thing".into(),
                String::new(),
                None,
                Vec::<String>::new(),
            )
            .unwrap();
        store
            .update(
                t.id.as_str(),
                crate::tasks::TaskPatch {
                    status: Some(crate::tasks::TaskStatus::Completed),
                    ..Default::default()
                },
            )
            .unwrap();
        let tasks = store.list(DeletedFilter::Exclude);
        assert!(
            pick_next_open_task(&tasks).is_none(),
            "completed-only store should yield no open task"
        );
    }

    #[test]
    fn skips_completed_when_pending_exists_robust() {
        let store = TaskStore::in_memory();
        let done = store
            .create(
                "Already done".into(),
                String::new(),
                None,
                Vec::<String>::new(),
            )
            .unwrap();
        store
            .update(
                done.id.as_str(),
                crate::tasks::TaskPatch {
                    status: Some(crate::tasks::TaskStatus::Completed),
                    ..Default::default()
                },
            )
            .unwrap();
        store
            .create(
                "Still queued".into(),
                String::new(),
                None,
                Vec::<String>::new(),
            )
            .unwrap();
        let tasks = store.list(DeletedFilter::Exclude);
        let picked = pick_next_open_task(&tasks).expect("pending should be picked");
        assert_eq!(picked.subject, "Still queued");
    }
}

/// Single- or double-row spinner widget rendered between the message
/// scroll and the input bar (v126 layout, cli.js:323180-323235 + 323851).
/// Row 0 = verb + elapsed + live-token-count + stall-status, composed in
/// `crate::spinner`. Row 1 (when present) = `□ Next: <task subject>`,
/// matching cli.js's `Next: ${m.subject}` line.
fn spinner_row(f: &mut Frame, app: &App, area: Rect) {
    if area.height == 0 {
        return;
    }
    let t = app.theme;
    let now = std::time::Instant::now();
    // Compaction takes precedence — a compact request runs to completion
    // before the user's submit ever fires the actual stream, so during
    // that window the spinner should read `Compacting…`, not a stale
    // `Fermenting…` from the previous turn.
    let row1_elapsed: std::time::Duration;
    // `verb_spans` is the verb portion of the spinner row, with the
    // shimmer-sweep highlight applied per-character. The renderer
    // assembles the final line as `glyph + verb_spans + body` so the
    // shimmer animates only the active verb (mirroring v126's
    // `<GlimmerMessage>`). For the compact path we keep the old
    // single-string body since compaction has its own status format.
    let mut verb_spans: Vec<Span<'static>> = Vec::new();
    let mut compact_body: Option<String> = None;
    let mut tail_body: String = String::new();
    let mut head_glyph: &'static str = "";
    if let Some(started) = app.compacting_started_at {
        let elapsed = now.duration_since(started);
        row1_elapsed = elapsed;
        // Pass the pre-compact token count so the spinner shows
        // *what's being compacted*. `tool_ctx.approx_tokens` still
        // reflects the pre-compact estimate during the compact (it's
        // only updated to the post-compact value when CompactionDone
        // fires), so it's the right source.
        let pre = app.tool_ctx.approx_tokens as u64;
        compact_body = Some(crate::spinner::format_compact_status(
            app.spinner_frame,
            elapsed,
            pre,
            app.compacting_output_chars,
        ));
    } else if let Some(recovery) = app.network_recovery_status.as_ref() {
        let elapsed = app
            .turn_started_at
            .or(app.streaming_started_at)
            .map(|t| now.duration_since(t))
            .unwrap_or_default();
        row1_elapsed = elapsed;
        head_glyph = "!";
        let label = match recovery.status_code {
            Some(code) => format!("{code} {}", recovery.reason.label()),
            None => recovery.reason.label().to_owned(),
        };
        verb_spans.push(Span::styled(
            label,
            Style::default().fg(t.warning).add_modifier(Modifier::BOLD),
        ));
        let last_seen = now.duration_since(recovery.updated_at).as_secs();
        tail_body = format!(
            " · retrying {} · attempt {} · last {}s",
            recovery.provider.label(),
            recovery.attempts,
            last_seen
        );
        if let Some(status) = app.claude_status.as_ref()
            && let Some(outage) = status.outage_context()
        {
            tail_body.push_str(" · status ");
            tail_body.push_str(&truncate_str(&outage, 72));
        }
    } else {
        // Prefer the user-turn clock so a multi-step agentic loop reads
        // cumulative time, not just the current sub-stream's age. Fall back
        // to `streaming_started_at` for the brief first frame after submit
        // before the agentic gate updates the turn clock.
        let elapsed = app
            .turn_started_at
            .or(app.streaming_started_at)
            .map(|t| now.duration_since(t))
            .unwrap_or_default();
        let stall = app
            .streaming_last_token_at
            .map(|t| now.duration_since(t))
            .unwrap_or_default();
        // Anthropic SSE pushes cumulative `output_tokens` in every
        // `message_delta` event (sse.rs:212-218 → StreamEvent::Usage →
        // app.last_usage_output) — wire-truth, no estimation needed. OWUI /
        // OpenAI providers only emit usage at `message_stop`; for those the
        // wire value stays 0 mid-stream, so we fall back to chars/4 of the
        // streamed text + reasoning. The first non-zero wire value beats the
        // estimate; once the wire stops moving we keep the last known count.
        let estimate = app.streaming_response_bytes as u64 / 4;
        let live_tokens = crate::spinner::live_token_count(app.last_usage_output as u64, estimate);
        // Thinking signal — Some(Live) while reasoning is streaming,
        // Some(Done(d)) once we got the first text byte after thinking,
        // None when the model isn't using extended thinking this turn.
        let thinking = match (app.thinking_started_at, app.thinking_ended_at) {
            (Some(_), None) => Some(crate::spinner::ThinkingStatus::Live),
            (Some(start), Some(end)) => Some(crate::spinner::ThinkingStatus::Done(
                end.duration_since(start),
            )),
            _ => None,
        };
        row1_elapsed = elapsed;
        let segs = crate::spinner::status_segments(
            app.spinner_frame,
            elapsed,
            live_tokens,
            stall,
            thinking,
        );
        head_glyph = segs.glyph;
        let verb_width = segs.verb.chars().count();
        let reduced = crate::spinner::reduced_motion();

        // Stalled intensity: blends 0 → 1 over 30s..120s of token
        // silence. Mirrors v126's `stalledIntensity` prop on
        // <GlimmerMessage>. Drives a base-color fade from
        // text_secondary toward error so the verb visibly "rusts" as
        // the wait grows. Capped at 1.0; clamped to 0 below 30s so
        // routine pauses don't tint the verb.
        let stall_secs = stall.as_secs_f32();
        let stalled_intensity = ((stall_secs - 30.0) / 90.0).clamp(0.0, 1.0);
        let base_color = if stalled_intensity > 0.0 {
            pulse_color(t.text_secondary, t.error, stalled_intensity)
        } else {
            t.text_secondary
        };

        if reduced {
            // Reduced-motion: single static span at base color. No
            // sweep, no per-cell coloring. Still respects the stalled
            // fade because that's information, not decoration.
            verb_spans.push(Span::styled(
                segs.verb.to_string(),
                Style::default().fg(base_color),
            ));
        } else {
            // Multi-cell wave: instead of a hard ±1 cell sweep, use a
            // 5-cell falloff window so the highlight reads as a soft
            // pulse rolling through the verb. Each cell's blend
            // intensity drops by distance-from-index so the center is
            // brightest and edges fade smoothly into the base color.
            let g_idx = crate::spinner::glimmer_index(elapsed, verb_width, 50);
            const HALF: i32 = 2; // ±2 cells = 5-cell wave width
            for (i, ch) in segs.verb.chars().enumerate() {
                let dist = (i as i32 - g_idx).abs();
                let intensity = if dist > HALF {
                    0.0
                } else {
                    // Cosine falloff: 1 at center, 0 at HALF + 1.
                    // Smoother than linear (no edge kink).
                    let pct = dist as f32 / (HALF as f32 + 0.5);
                    0.5 + 0.5 * (1.0 - pct).max(0.0)
                };
                let mut style = if intensity > 0.05 {
                    let blended = pulse_color(base_color, t.accent, intensity);
                    let mut s = Style::default().fg(blended);
                    if intensity > 0.7 {
                        s = s.add_modifier(Modifier::BOLD);
                    }
                    s
                } else {
                    Style::default().fg(base_color)
                };
                // When stalled, suppress the bold so the verb reads
                // as quiet/dim rather than still active. Important
                // because BOLD on a red-tinted base reads as alarm.
                if stalled_intensity > 0.5 {
                    style = style.remove_modifier(Modifier::BOLD);
                }
                verb_spans.push(Span::styled(ch.to_string(), style));
            }
        }

        // Marching dots: replace the static "…" with a 4-frame
        // rotation `   ` → `.  ` → `.. ` → `...` so the user reads
        // motion even on a frozen verb. 250ms per step keeps the
        // tempo unhurried; reduced-motion collapses to a steady "…".
        let dots_str = if reduced {
            "…".to_string()
        } else {
            const PATTERNS: &[&str] = &["   ", ".  ", ".. ", "..."];
            let phase = (elapsed.as_millis() / 250) as usize;
            PATTERNS[phase % PATTERNS.len()].to_string()
        };
        tail_body = format!("{dots_str} {}", segs.body);
    };
    // Multi-agent fanout: when one or more background subagents are
    // running concurrently, append `· N agents…` to the spinner so the
    // user knows there's parallel work happening. Mirrors v126's
    // `3 agents…` indicator from cli.js (line 161622, task:background).
    // Counts Running + Idle so a teammate that finished its turn but
    // is still alive doesn't disappear from the spinner badge — the
    // user might still SendMessage to it.
    let active_agents = app
        .background_tasks
        .values()
        .filter(|bt| bt.status.is_alive())
        .count();
    let mut spans: Vec<Span<'static>> = if let Some(body) = compact_body {
        // Compact path: keep the legacy single-string format. Compaction
        // has its own status line ("Compacting…", different shape) and
        // animating the shimmer there would be misleading — the verb
        // isn't a free-rotating spinner during compact.
        vec![Span::styled(body, Style::default().fg(t.text_secondary))]
    } else {
        // Star glyph color pulses between accent and warning so the
        // sphincter reads as a *living* element instead of a flat
        // bullet. Phase derives from elapsed milliseconds (~1Hz cycle)
        // so the pulse stays smooth even when the spinner_frame ticks
        // at a different rate than the wallclock — running the pulse
        // off the spinner_frame would jitter on slow-redraw frames.
        // Reduced-motion: hold the glyph at full accent color so
        // there's still a visual focal point but no animation.
        let glyph_color = if crate::spinner::reduced_motion() {
            t.accent
        } else {
            let phase_ms = (row1_elapsed.as_millis() % 1200) as f32 / 1200.0;
            // Triangle wave: 0 → 1 → 0 over the cycle. Smoother than
            // a sawtooth, no need for sine's f32::sin pulled in here.
            let intensity = if phase_ms < 0.5 {
                phase_ms * 2.0
            } else {
                (1.0 - phase_ms) * 2.0
            };
            pulse_color(t.accent, t.warning, intensity)
        };
        let mut s = vec![Span::styled(
            format!("{} ", head_glyph),
            Style::default()
                .fg(glyph_color)
                .add_modifier(Modifier::BOLD),
        )];
        s.extend(verb_spans);
        s.push(Span::styled(tail_body, t.style_text_muted));
        s
    };
    if active_agents > 0 {
        let plural = if active_agents == 1 {
            "agent"
        } else {
            "agents"
        };
        spans.push(Span::styled(
            format!("  ⏵ {active_agents} {plural}…"),
            t.style_accent,
        ));
    }
    let line = Line::from(spans);
    let row0 = Rect { height: 1, ..area };
    f.render_widget(Paragraph::new(line).style(Style::default().bg(t.bg)), row0);

    // Row 1: "Next: <task subject>" if we have layout for it. Indent two
    // cells so it aligns under the spinner frame's first character — same
    // visual hierarchy as v126's nested status. Use dim/muted color so
    // the verb on row 0 stays the dominant element.
    if area.height >= 2 {
        let row1 = Rect {
            x: area.x,
            y: area.y + 1,
            width: area.width,
            height: 1,
        };
        // v126 cli.js:323851 picks `Next: m.subject ?? Tip: WH` —
        // task wins if there is one, else show a rotating tip so the
        // user has something useful to read while the model thinks.
        // The "dismiss popups" hint is filtered when nothing's open so
        // it doesn't read as a misleading instruction (the user looked
        // for the popup it was talking about and there wasn't one).
        let any_popup_open = app.show_help
            || app.show_model_picker
            || app.show_sidebar
            || app.transcript_search.is_some()
            || app.slash_popup_selected.is_some()
            || app.pending_approval.is_some();
        let (prefix, body) = if let Some(subj) = next_open_task_subject(app) {
            ("  □ Next: ".to_string(), subj)
        } else {
            (
                "  □ Tip: ".to_string(),
                crate::spinner::tip_for_with_state(row1_elapsed, any_popup_open).to_string(),
            )
        };
        let max_body = (area.width as usize).saturating_sub(prefix.chars().count() + 1);
        let trimmed: String = if body.chars().count() > max_body && max_body > 1 {
            let mut out: String = body.chars().take(max_body.saturating_sub(1)).collect();
            out.push('…');
            out
        } else {
            body
        };
        let row1_line = Line::from(vec![
            Span::styled(prefix, Style::default().fg(t.text_muted)),
            Span::styled(trimmed, Style::default().fg(t.text_muted)),
        ]);
        f.render_widget(
            Paragraph::new(row1_line).style(Style::default().bg(t.bg)),
            row1,
        );
    }

    // ─── Spinner Tree ────────────────────────────────────────────────────
    // Below the verb + Next-row, surface the running parallel work as a
    // tree. v126 shows this as the "agent fan" — when the user fires
    // off five Explore agents, they see five rows ticking in parallel
    // instead of just `5 agents…`.
    //
    // Two sources, picked in this order:
    //   1. Active team (renders a tree-shaped roster of teammates).
    //   2. Plain background subagents (one-shot Task tool calls).
    // Falling back to (2) means the user's "fire off Explore agents"
    // case shows individual rows even outside team mode.
    if area.height > 2 {
        let tree_area = Rect {
            x: area.x,
            y: area.y + 2,
            width: area.width,
            height: area.height.saturating_sub(2),
        };
        if app.team_context.is_active() {
            render_teammate_tree(f, app, tree_area);
        } else {
            render_subagent_tree(f, app, tree_area);
        }
    }
}

/// Tree of running subagents for the non-team case — one line per
/// active `BackgroundTask`. Same shape as the teammate tree so the
/// user's eye recognises the structure regardless of whether they're
/// in a team or just running parallel Explore agents.
/// Render a token count in Claude-Code-style condensed form: 8945 → "8.9k",
/// 89_745 → "89.7k", 1_240_000 → "1.2M", anything <1000 stays raw. The
/// status badge and the per-subagent fan rows both use this so multiple
/// agents can fit on a 40-col-wide row without wrapping.
pub(crate) fn format_token_count(n: u64) -> String {
    if n < 1_000 {
        format!("{n}")
    } else if n < 1_000_000 {
        format!("{:.1}k", n as f64 / 1_000.0)
    } else {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    }
}

/// Build the trailing "· N tools · K tok" suffix for a subagent row.
/// Empty when there's no live data yet (the task just started; nothing
/// to show beyond the description). Mirrors v131's
/// `(${z.toolUseCount} tools, ${z.tokenCount} tokens)` from
/// cli.2.1.131.beautified.js.
pub(crate) fn format_subagent_counters(bt: &crate::app::BackgroundTask) -> String {
    let mut parts: Vec<String> = Vec::new();
    if bt.tool_use_count > 0 {
        parts.push(format!(
            "{} tool{}",
            bt.tool_use_count,
            if bt.tool_use_count == 1 { "" } else { "s" }
        ));
    }
    let total_tokens = bt
        .latest_input_tokens
        .saturating_add(bt.latest_cache_read_tokens)
        .saturating_add(bt.latest_cache_write_tokens)
        .saturating_add(bt.cumulative_output_tokens);
    if total_tokens > 0 {
        parts.push(format!("{} tok", format_token_count(total_tokens)));
    }
    if parts.is_empty() {
        String::new()
    } else {
        format!(" · {}", parts.join(" · "))
    }
}

fn render_subagent_tree(f: &mut Frame, app: &App, area: Rect) {
    if area.height == 0 || area.width < 20 {
        return;
    }
    let t = app.theme;

    // Show only rows that aren't done. A finished one-shot subagent
    // doesn't deserve a tree row — its result is already in the
    // transcript. `viewing_task_id` short-circuits this entire branch
    // because the task-view UI takes over the whole region.
    //
    // Both Running and Idle teammates belong on the fan: Idle is "alive
    // but between turns" — the user still wants to see them and may
    // SendMessage to wake them. Running is rendered with the kind color,
    // Idle is dimmed so the user can tell them apart at a glance.
    let mut active: Vec<&crate::app::BackgroundTask> = app
        .background_tasks
        .values()
        .filter(|bt| bt.status.is_alive())
        .collect();
    if active.is_empty() {
        return;
    }
    active.sort_by_key(|bt| bt.task_id.as_str().to_owned());

    let mut lines: Vec<Line> = Vec::new();
    // Leader row (mirrors the team-tree shape so the visual is consistent).
    lines.push(Line::from(vec![
        Span::styled("   ╒═ ", Style::default().fg(t.text_muted)),
        Span::styled(
            "agents",
            Style::default()
                .fg(ratatui::style::Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
    ]));

    let count = active.len();
    for (i, bt) in active.iter().enumerate() {
        let is_last = i == count - 1;
        let connector = if is_last { "   └─ " } else { "   ├─ " };
        // Description is the human label the model passed when calling
        // Task; truncate so it doesn't blow out narrow terminals.
        let desc = bt.description.as_str();
        let desc_trimmed = if desc.chars().count() > 48 {
            let mut s: String = desc.chars().take(47).collect();
            s.push('…');
            s
        } else {
            desc.to_owned()
        };
        let is_idle = matches!(bt.status, crate::types::TaskLifecycle::Idle);
        let activity = if is_idle {
            "  · idle".to_string()
        } else {
            // Layered: tool name (when known) + counter suffix. Mirrors
            // v131's "Running N agents · 22 tool uses · 89.7k tokens".
            let tool_part = match &bt.last_tool {
                Some(tool) => format!("  · {tool}"),
                None => String::new(),
            };
            format!("{tool_part}{}", format_subagent_counters(bt))
        };
        // Emphasize whichever agent emitted the most-recent chunk or
        // tool-progress event so the user can spot which one is
        // currently moving in a fan of N parallel agents. Idle agents
        // never get the bold-active treatment — they're not the
        // currently-moving one.
        let is_active = !is_idle
            && app
                .last_active_agent_task
                .as_deref()
                .map(|id| id == bt.task_id.as_str())
                .unwrap_or(false);
        let name_style = if is_active {
            t.style_accent
                .bg(t.surface_raised)
                .add_modifier(Modifier::BOLD)
        } else if is_idle {
            t.style_text_muted
        } else {
            t.style_accent
        };
        lines.push(Line::from(vec![
            Span::styled(connector, Style::default().fg(t.text_muted)),
            Span::styled(desc_trimmed, name_style),
            Span::styled(activity, Style::default().fg(t.text_muted)),
        ]));
    }

    let available_height = area.height as usize;
    let display: Vec<Line> = lines.into_iter().take(available_height).collect();
    f.render_widget(
        Paragraph::new(display).style(Style::default().bg(t.bg)),
        area,
    );
}

/// Render the teammate tree widget (the "agent fan").
///
/// Shows the team structure with box-drawing connectors:
/// ```text
///    ╒═ team-lead
///    ├─ researcher: Researching… · 1.2k tokens
///    ├─ implementer: Idle for 3s
///    └─ tester: Running tests… · 5 tool uses
/// ```
fn render_teammate_tree(f: &mut Frame, app: &App, area: Rect) {
    use crate::swarm::{self, types::teammate_color};

    if area.height == 0 || area.width < 20 {
        return;
    }

    let t = app.theme;
    let mut lines: Vec<Line> = Vec::new();

    // Collect teammates (sorted by name for stability)
    let mut teammates: Vec<(&String, &swarm::TeammateInfo)> =
        app.team_context.teammates.iter().collect();
    teammates.sort_by_key(|(_, info)| &info.name);

    // Filter out leader
    let teammates: Vec<_> = teammates
        .into_iter()
        .filter(|(_, info)| info.name != swarm::TEAM_LEAD_NAME)
        .collect();

    if teammates.is_empty() {
        return;
    }

    // Leader row
    lines.push(Line::from(vec![
        Span::styled("   ╒═ ", Style::default().fg(t.text_muted)),
        Span::styled(
            "team-lead",
            Style::default()
                .fg(ratatui::style::Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
    ]));

    // Teammate rows
    for (i, (_, info)) in teammates.iter().enumerate() {
        let is_last = i == teammates.len() - 1;
        let connector = if is_last { "   └─ " } else { "   ├─ " };

        let color = teammate_color(info.color.as_deref());

        // Look up activity from background tasks. Match by name suffix
        // so a teammate "ui-explorer" finds its task whose id is
        // "teammate-ui-explorer@<team>".
        let bt = app
            .background_tasks
            .values()
            .find(|bt| bt.task_id.as_str().contains(&info.name));
        let bt_status = bt.map(|bt| bt.status);
        let activity = bt.and_then(|bt| bt.last_tool.clone());

        let status_text = if matches!(bt_status, Some(crate::types::TaskLifecycle::Idle)) {
            // Source-of-truth: the runner has emitted TeammateEvent::Idle.
            // Don't fall back to elapsed-since-spawn timing — that
            // misreported "Idle for 30s" while the agent was actively
            // streaming for 30s.
            ": Idle".to_owned()
        } else if matches!(bt_status, Some(crate::types::TaskLifecycle::Completed)) {
            ": Done".to_owned()
        } else if matches!(bt_status, Some(crate::types::TaskLifecycle::Failed)) {
            ": Failed".to_owned()
        } else {
            match &activity {
                Some(tool) => format!(": {}…", tool),
                None => ": Working…".to_owned(),
            }
        };

        lines.push(Line::from(vec![
            Span::styled(connector, Style::default().fg(t.text_muted)),
            Span::styled(
                &info.name,
                Style::default().fg(color).add_modifier(Modifier::BOLD),
            ),
            Span::styled(status_text, Style::default().fg(t.text_muted)),
        ]));
    }

    // Render
    let available_height = area.height as usize;
    let display_lines: Vec<Line> = lines.into_iter().take(available_height).collect();
    f.render_widget(
        Paragraph::new(display_lines).style(Style::default().bg(t.bg)),
        area,
    );
}

/// Render the input box. Hot path — runs every frame including
/// while the user is typing. Tracing here catches wrap miscounts
/// (the textarea's reported cursor column vs the rendered cell
/// position), prompt-mode dispatch, and edit-mode flips.
#[tracing::instrument(
    target = "jfc::render::input",
    level = "trace",
    skip(f, app),
    fields(
        x = area.x,
        y = area.y,
        w = area.width,
        h = area.height,
        editing = app.editing_message_idx.is_some(),
        streaming = app.is_streaming,
    ),
)]
fn input(f: &mut Frame, app: &mut App, area: Rect) {
    let t = app.theme;
    // Boxed input with rounded border. The prompt char sits INLINE
    // at the start of the typing surface — like a shell prompt.
    // Up to 4 cells reserved for the prompt + an animation tail
    // (currently only used by `:comet` mode).
    //
    // Prompt mode selector via `JFC_PROMPT_CHAR`:
    //   :comet     — comet `☄` with streak tail (default)
    //   :moon      — moon phases ○◐●◑ cycle while streaming
    //   :dice      — dice faces ⚀⚁⚂⚃⚄⚅ shuffle while streaming
    //   :notes     — music notes ♩♪♫♬ cycle while streaming
    //   :hourglass — `⌛` ↔ `⌚` flip every 800ms
    //   :atom      — atom `⚛` (just color pulse, no shape change)
    //   <any single char> — that char as a static glyph (color pulse)
    // Edit mode overrides any choice with `✎` (pencil).
    let in_edit_mode = app.editing_message_idx.is_some();
    let raw_setting = std::env::var("JFC_PROMPT_CHAR").unwrap_or_else(|_| ":comet".to_string());
    let mode = parse_prompt_mode(&raw_setting);
    let now_ms = app.launched_at.elapsed().as_millis();
    let streaming_for_anim = app.is_streaming && !crate::spinner::reduced_motion();
    let prompt_char: String = if in_edit_mode {
        "✎".to_string()
    } else if let PromptMode::Static(s) = &mode {
        s.clone()
    } else {
        prompt_mode_frame(&mode, streaming_for_anim, now_ms).to_string()
    };

    let (prompt_color, border_color) = if in_edit_mode {
        (t.warning, t.warning)
    } else if app.is_streaming {
        (t.accent, t.text_muted)
    } else {
        (t.accent, t.border)
    };

    // Edit-mode badge in the title (top border) so the user can't
    // miss the editing state. Title is otherwise empty.
    let title_line = if let Some(idx) = app.editing_message_idx {
        Line::from(Span::styled(
            format!(" editing #{idx} · Esc to cancel "),
            Style::default().fg(t.warning).add_modifier(Modifier::BOLD),
        ))
    } else {
        Line::from("")
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(ratatui::widgets::BorderType::Rounded)
        .border_style(Style::default().fg(border_color))
        .padding(Padding::horizontal(1))
        .title(title_line)
        .style(Style::default().bg(t.surface));
    let inner = block.inner(area);
    f.render_widget(block, area);

    // Prompt strip: 2 cells reserved (glyph + trailing space).
    let prompt_cells: u16 = 2;
    let textarea_x = inner.x + prompt_cells.min(inner.width);
    let textarea_w = inner.width.saturating_sub(prompt_cells);

    // Paint the prompt glyph on the first row of inner.
    if inner.height > 0 && inner.y < f.buffer_mut().area().bottom() {
        let buf = f.buffer_mut();
        // Glyph cell.
        let glyph_x = inner.x;
        if glyph_x < buf.area().right() {
            let cell = &mut buf[(glyph_x, inner.y)];
            cell.set_symbol(&prompt_char);
            let invert = matches!(
                std::env::var("JFC_PROMPT_INVERT").as_deref(),
                Ok("1") | Ok("true")
            );
            let style = if invert {
                Style::default()
                    .fg(t.surface)
                    .bg(prompt_color)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
                    .fg(prompt_color)
                    .bg(t.surface)
                    .add_modifier(Modifier::BOLD)
            };
            cell.set_style(style);
        }
        // Trailing space so the glyph isn't glued to text.
        let space_x = inner.x + 1;
        if space_x < buf.area().right() {
            let cell = &mut buf[(space_x, inner.y)];
            cell.set_symbol(" ");
            cell.set_style(Style::default().bg(t.surface));
        }
    }

    // Textarea inner rect (everything to the right of the prompt
    // strip).
    let inner = Rect {
        x: textarea_x,
        y: inner.y,
        width: textarea_w,
        height: inner.height,
    };

    let content_width = inner.width.max(1) as usize;
    app.input_wrap_width = content_width;
    let (lines, cursor_row, cursor_col) = input_soft_wrapped_lines(app, content_width);
    let visible_rows = inner.height.max(1) as usize;
    let start = cursor_row.saturating_add(1).saturating_sub(visible_rows);
    // Rainbow gradient for slash-command and @mention prefixes — gives
    // those tokens a visible "specialness" so the user sees that
    // they'll route somewhere distinct (a slash command, a file
    // mention) rather than be sent as plain text. Phase rotates with
    // wallclock so the gradient gently flows through the chars on
    // each redraw. Reduced-motion holds the phase at 0 so the colors
    // stay still but the gradient is still applied — readable, just
    // not animated.
    let rainbow_phase = if crate::spinner::reduced_motion() {
        0.0_f32
    } else {
        (app.launched_at.elapsed().as_millis() as f32 / 25.0) % 360.0
    };
    let visible = lines
        .iter()
        .skip(start)
        .take(visible_rows)
        .map(|line| Line::from(input_line_to_spans(line, t, rainbow_phase)))
        .collect::<Vec<_>>();

    // `.wrap(Wrap{trim:false})` — without it, ratatui falls back to
    // `LineTruncator` and any visible line longer than `inner.width`
    // cells gets clipped at the right edge. `input_soft_wrapped_lines`
    // pre-wraps to fit, but pre-wrapping is char-count based; for
    // multi-cell unicode (CJK / emoji / fullwidth punctuation) the
    // pre-wrapped line was N chars but 2N cells wide → second half
    // disappeared.
    f.render_widget(
        Paragraph::new(visible)
            .wrap(ratatui::widgets::Wrap { trim: false })
            .style(Style::default().bg(t.surface)),
        inner,
    );

    // Ghost cursor pulse: tint the cursor cell's background between
    // surface_raised and accent on a 1.2s clock so the typing surface
    // feels "ready" even when nothing's moving. Only visible when
    // not streaming (the spinner takes over the visual focus during
    // streaming) and not in edit mode (the orange edit border is
    // already a strong signal). Reduced-motion skips the pulse.
    if !app.is_streaming
        && !in_edit_mode
        && !crate::spinner::reduced_motion()
        && area.height > 2
        && area.width > 2
    {
        let cursor_x = inner
            .x
            .saturating_add(cursor_col as u16)
            .min(inner.right().saturating_sub(1));
        let cursor_y = inner
            .y
            .saturating_add(cursor_row.saturating_sub(start) as u16)
            .min(inner.bottom().saturating_sub(1));
        let buf = f.buffer_mut();
        if cursor_x < buf.area().right() && cursor_y < buf.area().bottom() {
            // Static accent bg on the cursor cell. Previously this was a
            // pulsing animation (sin wave on elapsed time) which caused
            // ratatui to see a buffer diff every frame → 30fps terminal
            // writes even during idle. A static tint eliminates the diff
            // while still visually marking the cursor position.
            let bg = pulse_color(t.surface_raised, t.accent, 0.18);
            let cell = &mut buf[(cursor_x, cursor_y)];
            cell.set_style(cell.style().bg(bg));
        }
    }

    if area.height > 2 && area.width > 2 {
        f.set_cursor_position(Position::new(
            inner
                .x
                .saturating_add(cursor_col as u16)
                .min(inner.right().saturating_sub(1)),
            inner
                .y
                .saturating_add(cursor_row.saturating_sub(start) as u16)
                .min(inner.bottom().saturating_sub(1)),
        ));
    }
}

fn input_visual_line_count(app: &App, content_width: usize) -> usize {
    input_soft_wrapped_lines(app, content_width).0.len().max(1)
}

fn input_soft_wrapped_lines(app: &App, content_width: usize) -> (Vec<String>, usize, usize) {
    use unicode_width::UnicodeWidthChar;

    let width = content_width.max(1);
    let logical_lines = app.textarea.lines();
    let cursor = app.textarea.cursor();
    let (cursor_line, cursor_col) = (cursor.0, cursor.1);
    let mut out = Vec::new();
    let mut visual_cursor_row = 0usize;
    let mut visual_cursor_col = 0usize;

    if logical_lines.iter().all(|line| line.is_empty()) {
        out.push("send a message…".to_string());
        return (out, 0, 0);
    }

    for (line_idx, line) in logical_lines.iter().enumerate() {
        if line_idx == cursor_line {
            // The textarea reports `cursor_col` as a CHAR INDEX, but
            // `hard_wrap_str` now wraps by CELL WIDTH. Convert by
            // walking the line up to the cursor's character index,
            // accumulating cell widths, and tracking which wrap row
            // the running total falls into.
            //
            // Earlier this used `cursor_col / width` and
            // `cursor_col % width` — correct only when 1 char = 1
            // cell. For CJK / emoji / fullwidth chars (each 2 cells)
            // the cursor displayed in the wrong visual position,
            // sometimes offscreen, and pre-wrapped lines didn't
            // line up with the rendered cell columns.
            let mut col_width = 0usize;
            let mut wrap_row = 0usize;
            for (i, ch) in line.chars().enumerate() {
                if i >= cursor_col {
                    break;
                }
                let cw = UnicodeWidthChar::width(ch).unwrap_or(1);
                if cw > 0 && col_width + cw > width {
                    wrap_row += 1;
                    col_width = 0;
                }
                col_width += cw;
            }
            visual_cursor_row = out.len() + wrap_row;
            visual_cursor_col = col_width;
        }
        let wrapped = markdown::hard_wrap_str(line, width);
        out.extend(wrapped);
    }

    (out, visual_cursor_row, visual_cursor_col)
}

/// Top-right toast strip. Renders one row per active toast, color-coded
/// by `ToastKind`. Mirrors v126's terminal `notification()` pattern —
/// non-blocking, auto-expires (handled in the `Tick` arm). Width is
/// capped at 60 cells so a wide message text never gets pushed offscreen
/// by a long compaction status.
fn toast_overlay(f: &mut Frame, app: &App) {
    use crate::toast::ToastKind;
    let t = app.theme;
    let frame_area = f.area();
    if frame_area.width < 30 || frame_area.height < 4 {
        return;
    }
    const MAX_W: u16 = 60;
    // Reserve room for a 1-cell border on each side so the strip
    // reads as a contained unit rather than text bleeding into the
    // transcript below it.
    let w = MAX_W.min(frame_area.width.saturating_sub(2));
    let count = app.toasts.len() as u16;
    let body_h = count.min(5); // MAX_TOASTS, but bound to layout
    if body_h == 0 {
        *app.toasts_rect.borrow_mut() = None;
        return;
    }
    let h = body_h + 2; // borders top/bottom
    // Slide-in: the strip enters from off-screen-right and eases in
    // over the freshest toast's first 200ms. Ease-out cubic so it
    // settles softly rather than overshooting. Reduced-motion skips
    // the slide and the strip pops in at its final position.
    let slide_offset: u16 = if crate::spinner::reduced_motion() {
        0
    } else {
        let freshest_age = app
            .toasts
            .iter()
            .map(|tt| tt.created_at.elapsed())
            .min()
            .unwrap_or_default();
        let progress = (freshest_age.as_millis() as f32 / 200.0).min(1.0);
        // Ease-out cubic: 1 - (1 - t)^3
        let eased = 1.0 - (1.0 - progress).powi(3);
        // Off-screen distance is the strip width — at progress=0 the
        // strip is fully off the right edge; at progress=1 it sits
        // flush with its target.
        ((1.0 - eased) * (w as f32 + 4.0)).round() as u16
    };
    let target_x = frame_area.x + frame_area.width.saturating_sub(w + 1);
    let frame_right = frame_area.x + frame_area.width;
    // Resting x of the strip + the slide offset. Capped to the
    // frame's right edge so we never go past the buffer.
    let actual_x = target_x
        .saturating_add(slide_offset)
        .min(frame_area.x + frame_area.width.saturating_sub(1));
    // Width *must* be derived from `actual_x` so `actual_x + width`
    // never exceeds `frame_right`. Earlier this was computed
    // independently (`w.saturating_sub(slide_offset)`), which clamped
    // the x within bounds but left a width that walked off the right
    // edge — a 60-cell-wide strip starting at column 207 of a
    // 208-cell-wide buffer panicked the ratatui Clear widget at
    // `index_of((208, 1))`. The bug surfaced on slide-in's first
    // frame (offset=full strip width).
    let actual_w = w.min(frame_right.saturating_sub(actual_x));
    let area = Rect {
        x: actual_x,
        y: frame_area.y + 1,
        width: actual_w,
        height: h.min(frame_area.height.saturating_sub(2)),
    };
    if area.width == 0 || area.height == 0 {
        return;
    }
    *app.toasts_rect.borrow_mut() = Some(area);
    f.render_widget(Clear, area);
    // Border color tracks the highest-severity toast in the strip so
    // an Error toast pulls a red border even when surrounded by Info
    // entries. The user's eye finds the strip faster than reading
    // each row.
    let border_color = app
        .toasts
        .iter()
        .map(|tt| match tt.kind {
            ToastKind::Error => 3,
            ToastKind::Warning => 2,
            ToastKind::Success => 1,
            ToastKind::Info => 0,
        })
        .max()
        .map(|rank| match rank {
            3 => t.error,
            2 => t.warning,
            1 => t.success,
            _ => t.border,
        })
        .unwrap_or(t.border);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(ratatui::widgets::BorderType::Rounded)
        .border_style(Style::default().fg(border_color))
        .style(Style::default().bg(t.surface));
    let inner = block.inner(area);
    f.render_widget(block, area);
    let mut lines: Vec<Line> = Vec::new();
    for toast in app
        .toasts
        .iter()
        .rev()
        .take(inner.height as usize)
        .collect::<Vec<_>>()
    {
        let (icon, color) = match toast.kind {
            ToastKind::Info => ("ℹ", t.text_secondary),
            ToastKind::Success => ("✓", t.success),
            ToastKind::Warning => ("⚠", t.warning),
            ToastKind::Error => ("✘", t.error),
        };
        let max_text = (inner.width as usize).saturating_sub(4);
        let text: String = if toast.text.chars().count() > max_text {
            let mut out: String = toast
                .text
                .chars()
                .take(max_text.saturating_sub(1))
                .collect();
            out.push('…');
            out
        } else {
            toast.text.clone()
        };
        lines.push(Line::from(vec![
            Span::styled(format!(" {icon} "), Style::default().fg(color)),
            Span::styled(text, Style::default().fg(t.text_primary)),
        ]));
    }
    f.render_widget(
        Paragraph::new(lines).style(Style::default().bg(t.surface)),
        inner,
    );
}

/// One-line diagnostic summary row. v126 cli.js:338035-338038 renders this
/// as `Found <bold>N</bold> new diagnostic <issue/issues> in M <file/files>
/// (ctrl+o to expand)` in dim color. Shown above the spinner row when
/// `app.diagnostics` has any entries; the formatter and dedup-by-file
/// logic live in `diagnostics.rs`.
fn diagnostic_row(f: &mut Frame, app: &App, area: Rect) {
    if area.height == 0 {
        return;
    }
    let t = app.theme;
    // Count only the *new* diagnostics — entries the user has already
    // acknowledged via Ctrl+O don't show up in the row count. v126
    // cli.js:231036 surfaces the same delta-only count: `Found N new
    // diagnostic issue(s)` — the word "new" is load-bearing.
    let new_entries: Vec<&crate::diagnostics::DiagnosticEntry> =
        crate::diagnostics::unacknowledged(&app.diagnostics, &app.delivered_diagnostics);
    let issues = new_entries.len();
    let files = {
        let mut s: std::collections::HashSet<&str> = std::collections::HashSet::new();
        for e in &new_entries {
            s.insert(e.file.as_str());
        }
        s.len()
    };
    let Some(text) = crate::diagnostics::format_summary(issues, files) else {
        return;
    };
    let has_errors = new_entries
        .iter()
        .any(|e| matches!(e.severity, crate::diagnostics::Severity::Error));
    let icon_color = if has_errors { t.error } else { t.warning };
    let line = Line::from(vec![
        Span::styled("● ", Style::default().fg(icon_color)),
        Span::styled(text, Style::default().fg(t.text_muted)),
    ]);
    f.render_widget(Paragraph::new(line).style(Style::default().bg(t.bg)), area);
}

/// Modal diagnostic-expansion panel (`Ctrl+O` from the summary row,
/// `Esc` to close). Mirrors v126 cli.js:338043-338053:
///
/// ```text
///   <relative path bold>  (file://)
///     ✘ [Line 12:5] unresolved import [E0432] (cargo)
///     ⚠ [Line 1:1]  unused variable
///   ...
/// ```
///
/// Diagnostics are grouped by file (first occurrence preserves cargo's
/// emission order) and listed underneath. We don't render the URI scheme
/// suffix v126 does (`(file://)`) — paths are already cwd-relative so
/// it's noise.
/// All slash commands jfc accepts, with a one-line description for
/// the autocomplete popup. Order = display order. Keep in sync with
/// the actual handlers in `input.rs`.
const SLASH_COMMANDS: &[(&str, &str)] = &[
    ("/clear", "clear the conversation"),
    ("/compact", "summarize earlier messages to free context"),
    ("/help", "show jfc help"),
    ("/export", "save the transcript as markdown"),
    (
        "/theme",
        "open picker or switch theme: catppuccin / tokyo-night / gruvbox / ...",
    ),
    (
        "/dump-context",
        "show what the model sees: memories, skills, tools",
    ),
    ("/worktree", "create / list / remove a git worktree"),
    ("/swarm-approve", "approve a pending swarm tool request"),
    ("/swarm-deny", "deny a pending swarm tool request"),
    ("/auto-mode", "toggle the autonomous classifier"),
    (
        "/advisor",
        "ask a parallel advisor without disturbing the main agent",
    ),
    (
        "/install-github-app",
        "install the Claude GitHub App on this repo",
    ),
    ("/pr", "show a PR + review comments (`/pr <num>`)"),
    ("/pr-autofix", "ask the model to fix PR review comments"),
    (
        "/setup-github-actions",
        "scaffold .github/workflows/jfc-review.yml",
    ),
    ("/plan", "draft or update PLAN.md (Atlas-compatible)"),
    (
        "/roadmap",
        "draft or update ROADMAP.md (stable decimal IDs)",
    ),
    ("/parity", "draft or update PARITY.md (evidence required)"),
    ("/philosophy", "draft or update PHILOSOPHY.md"),
    ("/usage", "draft or update USAGE.md (operator commands)"),
];

/// Returns the `/<prefix>` the user is currently typing, when the
/// input bar contains a single line that starts with `/`. The popup
/// renders only when this returns Some so multi-line drafts and
/// non-slash input don't trigger.
pub(crate) fn current_slash_prefix(app: &App) -> Option<String> {
    let lines = app.textarea.lines();
    if lines.len() != 1 {
        return None;
    }
    let line = &lines[0];
    if !line.starts_with('/') {
        return None;
    }
    // Single-token: drop everything after the first space so the
    // popup goes away once the user has committed to a verb and
    // is typing arguments. v126's slash UI does the same.
    let token = line.split_whitespace().next().unwrap_or(line);
    Some(token.to_string())
}

pub(crate) fn slash_matches(prefix: &str) -> Vec<&'static (&'static str, &'static str)> {
    SLASH_COMMANDS
        .iter()
        .filter(|(cmd, _)| cmd.starts_with(prefix))
        .collect()
}

fn slash_popup(f: &mut Frame, app: &App, prefix: &str) {
    let matches = slash_matches(prefix);
    if matches.is_empty() {
        return;
    }
    let t = app.theme;
    let area = f.area();
    let h = (matches.len() as u16).min(8) + 2;
    let w: u16 = 60u16.min(area.width.saturating_sub(2));
    if area.height < h + 4 {
        return;
    }
    // Anchor above the input bar (which sits at the bottom of the
    // frame). Reserve 2 rows for the input border and 1 for the
    // status bar so the popup doesn't overlap them.
    let popup_y = area.y + area.height.saturating_sub(h + 4);
    let popup = Rect {
        x: area.x + 2,
        y: popup_y,
        width: w,
        height: h,
    };
    f.render_widget(Clear, popup);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(t.style_accent)
        .title(Span::styled(" slash commands ", t.style_accent_bold))
        .style(Style::default().bg(t.surface));
    let inner = block.inner(popup);
    f.render_widget(block, popup);

    let selected = app.slash_popup_selected.unwrap_or(0).min(matches.len() - 1);
    let lines: Vec<Line> = matches
        .iter()
        .enumerate()
        .take(inner.height as usize)
        .map(|(i, (cmd, desc))| {
            let active = i == selected;
            let row_style = if active {
                Style::default()
                    .fg(t.bg)
                    .bg(t.accent)
                    .add_modifier(Modifier::BOLD)
            } else {
                t.style_text_primary
            };
            let desc_style = if active {
                Style::default().fg(t.bg).bg(t.accent)
            } else {
                t.style_text_muted
            };
            Line::from(vec![
                Span::styled(format!(" {:<18}", cmd), row_style),
                Span::styled(format!(" {desc}"), desc_style),
            ])
        })
        .collect();
    f.render_widget(
        Paragraph::new(lines).style(Style::default().bg(t.surface)),
        inner,
    );
}

/// Bottom-anchored search bar shown while `app.transcript_search`
/// is `Some`. Mirrors Vim's `/` prompt: query on the left, match
/// counter on the right ("3 of 12"). The currently-focused match
/// is already scrolled into view by the input handler — this is
/// purely the input-bar UI for typing the query and stepping
/// through matches.
fn search_bar(f: &mut Frame, app: &App) {
    let Some(s) = &app.transcript_search else {
        return;
    };
    let t = app.theme;
    let area = f.area();
    if area.width < 20 || area.height < 3 {
        return;
    }
    let h: u16 = 2;
    let bar = Rect {
        x: area.x,
        y: area.y + area.height.saturating_sub(h + 2),
        width: area.width,
        height: h,
    };
    f.render_widget(Clear, bar);

    let count_label = if s.query.is_empty() {
        " (type to search) ".to_string()
    } else if s.matches.is_empty() {
        " (no matches) ".to_string()
    } else {
        format!(" {} of {} ", s.cursor + 1, s.matches.len())
    };
    let prompt = Line::from(vec![
        Span::styled(
            "  /",
            Style::default().fg(t.accent).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            s.query.clone(),
            Style::default()
                .fg(t.text_primary)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled("▏", Style::default().fg(t.accent)),
    ]);
    let hint = Line::from(vec![
        Span::styled(
            count_label,
            Style::default()
                .fg(if s.matches.is_empty() && !s.query.is_empty() {
                    t.warning
                } else {
                    t.text_secondary
                })
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            " ↑/↓ navigate · Enter jump · Esc close",
            Style::default()
                .fg(t.text_muted)
                .add_modifier(Modifier::ITALIC),
        ),
    ]);
    f.render_widget(
        Paragraph::new(vec![prompt, hint]).style(Style::default().bg(t.surface)),
        bar,
    );
}

/// Centered keybinding overlay toggled by `?`. Groups bindings by
/// context — Input bar, Transcript, Task view, Picker/Palette, Leader
/// chord, ESC behavior — so the user can find the chord they need
/// without grepping the source.
fn help_overlay(f: &mut Frame, app: &App) {
    let t = app.theme;
    let area = f.area();
    let width = (area.width * 7 / 10).clamp(60, 96);
    let height = (area.height * 8 / 10).clamp(20, 32);
    let x = area.x + area.width.saturating_sub(width) / 2;
    let y = area.y + area.height.saturating_sub(height) / 2;
    let popup = Rect::new(x, y, width, height);
    f.render_widget(Clear, popup);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(t.style_accent)
        .title(Span::styled(
            " keybindings · press ? or Esc to close ",
            t.style_accent_bold,
        ))
        .style(Style::default().bg(t.surface));
    let inner = block.inner(popup);
    f.render_widget(block, popup);

    // Each entry: (key, description). Sections separated by None.
    type Section = (&'static str, &'static [(&'static str, &'static str)]);
    const SECTIONS: &[Section] = &[
        (
            "Input bar",
            &[
                ("Enter", "send message"),
                ("Shift+Enter", "newline"),
                ("Up", "recall queued prompt (if input empty)"),
                ("Ctrl+V", "paste (text or image from clipboard)"),
                ("Ctrl+Z", "undo last edit"),
                ("Ctrl+Shift+Z", "redo"),
                ("Ctrl+F", "search inside the textarea"),
                ("Ctrl+R", "retry last prompt"),
                ("Ctrl+E", "edit + resubmit previous user message"),
                ("Ctrl+L", "yank file:line ref to clipboard"),
                ("Alt+./Alt+,", "raise / lower reasoning effort"),
                ("/export", "save transcript as markdown"),
                ("/theme", "switch theme"),
                ("/dump-context", "show what the model sees"),
            ],
        ),
        (
            "Transcript",
            &[
                ("Ctrl+P", "command palette"),
                ("Ctrl+M", "switch model"),
                ("Alt+./Alt+,", "raise / lower reasoning effort"),
                ("Ctrl+B", "toggle sessions sidebar"),
                ("Ctrl+S/I", "toggle info sidebar"),
                ("Ctrl+T", "show task panel"),
                ("Ctrl+O", "expand diagnostic row / large tool block"),
                ("o", "toggle expand on most recent collapsible block"),
                ("Ctrl+Y", "yank last assistant response"),
                ("Ctrl+L", "yank file:line ref from recent output"),
                ("j / k", "vim scroll down / up (empty input)"),
                ("g / G", "vim jump to top / bottom (empty input)"),
                ("Shift+Tab", "cycle permission modes"),
                ("PgUp/PgDn", "scroll a page"),
                ("Ctrl+Home/End", "jump to top/bottom"),
            ],
        ),
        (
            "Task view",
            &[
                ("Ctrl+X then ↓", "enter task view"),
                ("←/→", "previous / next running task"),
                ("↓", "jump to most recent task"),
                ("↑ or Esc", "exit task view"),
                ("o", "expand the most recent collapsible message"),
            ],
        ),
        (
            "Picker / Palette",
            &[
                ("↑/↓ or k/j", "navigate"),
                ("Home/End", "first / last"),
                ("PgUp/PgDn", "page"),
                ("Enter", "select"),
                ("Esc", "cancel"),
                ("type", "filter inline"),
            ],
        ),
        (
            "Interrupt & approvals",
            &[
                ("Esc Esc", "interrupt streaming / agentic loop"),
                ("y / n / a", "approve / deny / always (in approval modal)"),
                ("/swarm-approve <id>", "approve teammate permission request"),
                (
                    "/swarm-deny <id> [reason]",
                    "deny teammate permission request",
                ),
            ],
        ),
    ];

    let mut lines: Vec<Line<'static>> = Vec::new();
    for (section_name, entries) in SECTIONS.iter() {
        if !lines.is_empty() {
            lines.push(Line::from(""));
        }
        lines.push(Line::from(Span::styled(
            (*section_name).to_string(),
            t.style_accent
                .add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
        )));
        for (key, desc) in entries.iter() {
            lines.push(Line::from(vec![
                Span::raw("  "),
                Span::styled(
                    format!("{key:<24}"),
                    Style::default()
                        .fg(t.text_primary)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled((*desc).to_string(), Style::default().fg(t.text_secondary)),
            ]));
        }
    }

    // ── Custom keybindings from ~/.config/jfc/keybindings.toml ───────────
    let custom = crate::keybindings::all_bindings();
    if !custom.is_empty() {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "Custom bindings".to_string(),
            t.style_accent
                .add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
        )));
        for (key_str, desc) in &custom {
            lines.push(Line::from(vec![
                Span::raw("  "),
                Span::styled(
                    format!("{key_str:<24}"),
                    Style::default()
                        .fg(t.text_primary)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(desc.clone(), Style::default().fg(t.text_secondary)),
            ]));
        }
    } else {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "Custom bindings".to_string(),
            t.style_accent
                .add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
        )));
        lines.push(Line::from(vec![
            Span::raw("  "),
            Span::styled(
                "none — create ~/.config/jfc/keybindings.toml to add your own",
                Style::default().fg(t.text_muted),
            ),
        ]));
    }

    let para = Paragraph::new(lines)
        .style(Style::default().bg(t.surface))
        .scroll((0, 0));
    f.render_widget(para, inner);
}

fn diagnostic_panel(f: &mut Frame, app: &App) {
    let t = app.theme;
    let area = f.area();
    let w = area.width.saturating_mul(3) / 4;
    let h = area.height.saturating_mul(3) / 4;
    let x = (area.width.saturating_sub(w)) / 2;
    let y = (area.height.saturating_sub(h)) / 2;
    let rect = Rect {
        x: area.x + x,
        y: area.y + y,
        width: w,
        height: h,
    };
    f.render_widget(Clear, rect);
    let issues = app.diagnostics.len();
    let files = crate::diagnostics::count_files(&app.diagnostics);

    // Group entries by file in first-seen order. Avoid HashMap iteration
    // for ordering stability — use a Vec of (file, Vec<&entry>).
    let mut groups: Vec<(String, Vec<&crate::diagnostics::DiagnosticEntry>)> = Vec::new();
    for entry in &app.diagnostics {
        if let Some(g) = groups.iter_mut().find(|(f, _)| f == &entry.file) {
            g.1.push(entry);
        } else {
            groups.push((entry.file.clone(), vec![entry]));
        }
    }

    let mut lines: Vec<Line> = Vec::new();
    for (file, items) in &groups {
        lines.push(Line::from(Span::styled(
            file.clone(),
            t.style_text_primary.add_modifier(Modifier::BOLD),
        )));
        for entry in items {
            let body = crate::diagnostics::format_entry(entry);
            // Two-cell extra indent so file headers visually anchor.
            let color = match entry.severity {
                crate::diagnostics::Severity::Error => t.error,
                crate::diagnostics::Severity::Warning => t.warning,
                crate::diagnostics::Severity::Info => t.text_secondary,
                crate::diagnostics::Severity::Hint => t.text_muted,
            };
            lines.push(Line::from(Span::styled(body, Style::default().fg(color))));
        }
        lines.push(Line::from(""));
    }

    // Title now embeds a scroll position when the body overflows the
    // panel — the user can see at a glance how much more there is to
    // scroll through, and the key hints are visible on the title bar
    // instead of being hidden in the help overlay.
    let total_lines = lines.len();
    let inner_h = rect.height.saturating_sub(2) as usize; // borders
    let scroll = app
        .diagnostic_panel_scroll
        .min(total_lines.saturating_sub(inner_h.max(1)));
    let scroll_pos = if total_lines > inner_h && inner_h > 0 {
        format!(
            " · {}/{}",
            scroll + 1,
            total_lines.saturating_sub(inner_h.max(1)) + 1
        )
    } else {
        String::new()
    };
    let title = format!(
        " Diagnostics — {issues} {} in {files} {}{scroll_pos} · ↑↓/PgUp/PgDn scroll · Esc close ",
        if issues == 1 { "issue" } else { "issues" },
        if files == 1 { "file" } else { "files" },
    );
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(ratatui::widgets::BorderType::Rounded)
        .border_style(t.style_error)
        .title(Span::styled(
            title,
            t.style_error.add_modifier(Modifier::BOLD),
        ))
        .style(Style::default().bg(t.surface));
    let inner = block.inner(rect);
    f.render_widget(block, rect);

    f.render_widget(
        Paragraph::new(lines)
            .style(Style::default().bg(t.surface))
            .wrap(ratatui::widgets::Wrap { trim: false })
            .scroll((scroll as u16, 0)),
        inner,
    );
}

/// Floating completion list anchored just above the input bar.
/// Renders up to 8 candidates from `app.mention.candidates`, with the
/// current `selected` index highlighted. Mirrors v126 cli.js:161602
/// (`autocomplete:accept` / `autocomplete:dismiss`) — non-modal,
/// non-blocking, dismissed by Esc or by typing past the `@token`.
fn mention_popup(f: &mut Frame, app: &App, input_area: Rect) {
    let t = app.theme;
    let frame_area = f.area();
    let candidates = &app.mention.candidates;
    if candidates.is_empty() || frame_area.height < 6 {
        return;
    }
    const MAX_ROWS: u16 = 8;
    let visible: u16 = candidates.len().min(MAX_ROWS as usize) as u16;
    let h = visible + 2; // borders
    let w = 60u16.min(frame_area.width.saturating_sub(2));
    // Prefer placing the popup directly above the input. Fall back to
    // below when there isn't enough room above (small terminals).
    let above_top = input_area.y.saturating_sub(h);
    let area = if above_top >= frame_area.y && input_area.y >= h {
        Rect {
            x: input_area.x.min(frame_area.width.saturating_sub(w)),
            y: above_top,
            width: w,
            height: h,
        }
    } else {
        Rect {
            x: input_area.x.min(frame_area.width.saturating_sub(w)),
            y: input_area.y + input_area.height,
            width: w,
            height: h.min(
                frame_area
                    .height
                    .saturating_sub(input_area.y + input_area.height),
            ),
        }
    };
    f.render_widget(Clear, area);
    let title = format!(
        " @ {} ({} match{}) ",
        if app.mention.query.is_empty() {
            "<type to filter>".into()
        } else {
            app.mention.query.clone()
        },
        candidates.len(),
        if candidates.len() == 1 { "" } else { "es" }
    );
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(ratatui::widgets::BorderType::Rounded)
        .border_style(t.style_accent)
        .title(Span::styled(title, t.style_accent))
        .style(Style::default().bg(t.surface));
    let inner = block.inner(area);
    f.render_widget(block, area);

    let items: Vec<ListItem> = candidates
        .iter()
        .take(MAX_ROWS as usize)
        .enumerate()
        .map(|(i, path)| {
            let is_sel = i == app.mention.selected;
            let style = if is_sel {
                t.style_text_primary.bg(t.accent)
            } else {
                t.style_text_secondary
            };
            let prefix = if is_sel { "▸ " } else { "  " };
            let max_w = inner.width.saturating_sub(prefix.len() as u16) as usize;
            let truncated: String = if path.chars().count() > max_w && max_w > 1 {
                let mut s: String = path.chars().take(max_w.saturating_sub(1)).collect();
                s.push('…');
                s
            } else {
                path.clone()
            };
            ListItem::new(Line::from(vec![
                Span::styled(prefix.to_string(), style),
                Span::styled(truncated, style),
            ]))
        })
        .collect();
    f.render_widget(List::new(items), inner);
}

#[cfg(test)]
mod task_view_tests {
    use super::*;
    use std::collections::HashSet;

    /// Flatten a `Line` to plain string for substring assertions —
    /// markdown::to_lines produces multi-span lines (syntect highlighting),
    /// so we can't assert on a single span's `.content`.
    fn line_text(l: &Line<'_>) -> String {
        l.spans.iter().map(|s| s.content.as_ref()).collect()
    }

    #[test]
    fn markdown_renders_xml_stripped_in_task_view_normal() {
        // The bug: subagent view was rendering `<tool_call>{...}</tool_call>`
        // as literal angle brackets. Routing through `markdown::to_lines`
        // (which calls `strip_inline_tool_xml`) replaces them with the
        // `⟪tool_call⟫` marker so users see structure, not raw XML.
        let theme = Theme::dark();
        let messages = vec!["Before <tool_call>{\"name\":\"foo\"}</tool_call> after".to_string()];
        let expanded = HashSet::new();
        let lines = task_view_body_lines(&messages, &expanded, &theme, 80, false);
        let joined: String = lines.iter().map(line_text).collect::<Vec<_>>().join("\n");
        assert!(
            !joined.contains("<tool_call>"),
            "literal <tool_call> should be stripped, got: {joined}"
        );
        assert!(
            !joined.contains("</tool_call>"),
            "literal </tool_call> should be stripped, got: {joined}"
        );
        assert!(
            joined.contains("⟪tool_call⟫"),
            "expected the strip marker in output, got: {joined}"
        );
    }

    #[test]
    fn long_message_collapses_normal() {
        // 100-line entry → preview (5 lines) + 1 muted footer row when
        // collapsed; full content when the index is in `expanded`.
        let theme = Theme::dark();
        let body: String = (1..=100)
            .map(|n| format!("line {n}"))
            .collect::<Vec<_>>()
            .join("\n");
        let messages = vec![body];

        let collapsed_lines = task_view_body_lines(&messages, &HashSet::new(), &theme, 80, false);
        let collapsed_text: String = collapsed_lines
            .iter()
            .map(line_text)
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            collapsed_text.contains("press o to expand"),
            "collapsed view should show expansion hint, got: {collapsed_text}"
        );
        assert!(
            collapsed_text.contains("line 1"),
            "collapsed view should include first preview line"
        );
        assert!(
            collapsed_text.contains("line 5"),
            "collapsed view should include 5th preview line"
        );
        assert!(
            !collapsed_text.contains("line 50"),
            "collapsed view should hide line 50, got: {collapsed_text}"
        );
        assert!(
            !collapsed_text.contains("line 100"),
            "collapsed view should hide tail content"
        );

        let mut expanded = HashSet::new();
        expanded.insert(0);
        let expanded_lines = task_view_body_lines(&messages, &expanded, &theme, 80, false);
        let expanded_text: String = expanded_lines
            .iter()
            .map(line_text)
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            expanded_text.contains("line 1"),
            "expanded view should include first line"
        );
        assert!(
            expanded_text.contains("line 50"),
            "expanded view should include middle line"
        );
        assert!(
            expanded_text.contains("line 100"),
            "expanded view should include last line"
        );
        assert!(
            !expanded_text.contains("press o to expand"),
            "expanded view should not show the collapse hint"
        );
    }

    #[test]
    fn short_message_passes_through_untouched_robust() {
        // Below the line/byte threshold → no preview truncation, no
        // expansion footer, just whatever markdown::to_lines produced.
        let theme = Theme::dark();
        let messages = vec!["just one short line".to_string()];
        let lines = task_view_body_lines(&messages, &HashSet::new(), &theme, 80, false);
        let joined: String = lines.iter().map(line_text).collect::<Vec<_>>().join("\n");
        assert!(joined.contains("just one short line"));
        assert!(!joined.contains("press o to expand"));
    }

    #[test]
    fn large_byte_payload_collapses_even_without_many_lines_robust() {
        // A single >5 KB line still trips the byte threshold even though
        // the line count is 1 — guards against unwrapped JSON dumps.
        let theme = Theme::dark();
        let big = "x".repeat(TASK_VIEW_COLLAPSE_BYTES + 100);
        let messages = vec![big];
        let lines = task_view_body_lines(&messages, &HashSet::new(), &theme, 80, false);
        let joined: String = lines.iter().map(line_text).collect::<Vec<_>>().join("\n");
        assert!(
            joined.contains("press o to expand"),
            "byte-threshold trip should show expansion hint"
        );
    }
}

#[cfg(test)]
mod mcp_tests {
    use super::*;

    #[test]
    fn mcp_status_color_connected_is_success_normal() {
        let t = Theme::dark();
        assert_eq!(mcp_status_color(McpStatus::Connected, t), t.success);
    }

    #[test]
    fn mcp_status_color_disabled_is_muted_normal() {
        let t = Theme::dark();
        assert_eq!(mcp_status_color(McpStatus::Disabled, t), t.text_muted);
    }

    #[test]
    fn mcp_status_color_error_is_error_normal() {
        let t = Theme::dark();
        assert_eq!(mcp_status_color(McpStatus::Error, t), t.error);
    }
}

#[cfg(test)]
mod render_helpers_tests {
    use super::*;
    use crate::theme::Theme;

    fn t() -> Theme {
        Theme::dark()
    }

    // ─── pulse_color ───────────────────────────────────────────────
    // Normal: t=0 returns c1, t=1 returns c2, t=0.5 returns midpoint.
    #[test]
    fn pulse_color_endpoints_normal() {
        let c1 = Color::Rgb(0, 0, 0);
        let c2 = Color::Rgb(200, 100, 50);
        assert_eq!(pulse_color(c1, c2, 0.0), Color::Rgb(0, 0, 0));
        assert_eq!(pulse_color(c1, c2, 1.0), Color::Rgb(200, 100, 50));
        // Midpoint blend.
        if let Color::Rgb(r, g, b) = pulse_color(c1, c2, 0.5) {
            assert!((r as i32 - 100).abs() <= 1);
            assert!((g as i32 - 50).abs() <= 1);
            assert!((b as i32 - 25).abs() <= 1);
        } else {
            panic!("expected Rgb");
        }
    }

    // Robust: ANSI-named colors (no RGB triple) fall back to the
    // start color since blending isn't well-defined.
    #[test]
    fn pulse_color_named_falls_back_robust() {
        assert_eq!(pulse_color(Color::Red, Color::Blue, 0.5), Color::Red);
    }

    // Robust: `t` outside [0,1] gets clamped via interpolate_rgb.
    #[test]
    fn pulse_color_clamps_t_robust() {
        let c1 = Color::Rgb(0, 0, 0);
        let c2 = Color::Rgb(255, 255, 255);
        // t = -1 should clamp to 0 → c1
        assert_eq!(pulse_color(c1, c2, -1.0), Color::Rgb(0, 0, 0));
        // t = 5 should clamp to 1 → c2
        assert_eq!(pulse_color(c1, c2, 5.0), Color::Rgb(255, 255, 255));
    }

    // ─── tail_truncate ─────────────────────────────────────────────
    // Normal: short input fits, returns unchanged.
    #[test]
    fn tail_truncate_short_unchanged_normal() {
        assert_eq!(tail_truncate("hello", 10), "hello");
    }

    // Normal: long input keeps the tail with a `…/` prefix.
    #[test]
    fn tail_truncate_keeps_tail_normal() {
        let s = "/home/cole/RustProjects/active/jfc";
        let truncated = tail_truncate(s, 12);
        assert!(truncated.starts_with("…/"));
        assert!(truncated.ends_with("jfc"));
        assert!(truncated.chars().count() <= 12);
    }

    // Robust: max=0 returns empty (no panic).
    #[test]
    fn tail_truncate_zero_max_robust() {
        assert_eq!(tail_truncate("anything", 0), "");
    }

    // Robust: max < 4 (too narrow for "…/") falls back to head truncation.
    #[test]
    fn tail_truncate_narrow_falls_back_robust() {
        let s = "long/path/here";
        let result = tail_truncate(s, 3);
        // Should not panic, should be 3 cells or fewer.
        assert!(result.chars().count() <= 3);
    }

    // ─── wrap_text_to_width ────────────────────────────────────────
    // Normal: text shorter than width returns one line.
    #[test]
    fn wrap_text_short_one_line_normal() {
        let lines = wrap_text_to_width("hello world", 30);
        assert_eq!(lines, vec!["hello world"]);
    }

    // Normal: long text wraps at word boundaries.
    #[test]
    fn wrap_text_word_wraps_normal() {
        let lines = wrap_text_to_width("one two three four five", 10);
        // Each line should be ≤ 10 chars, breaking at spaces.
        for l in &lines {
            assert!(l.chars().count() <= 10, "line too long: {l:?}");
            assert!(!l.trim().is_empty(), "blank line in middle: {lines:?}");
        }
    }

    // Robust: a single word longer than width gets truncated with `…`
    // so it doesn't bleed off the edge.
    #[test]
    fn wrap_text_oversize_word_truncates_robust() {
        let lines = wrap_text_to_width("supercalifragilistic", 8);
        assert_eq!(lines.len(), 1);
        assert!(lines[0].chars().count() <= 8);
        assert!(lines[0].ends_with('…') || lines[0].chars().count() < 8);
    }

    // Robust: width=0 returns one empty line, no panic.
    #[test]
    fn wrap_text_zero_width_robust() {
        let lines = wrap_text_to_width("anything", 0);
        assert_eq!(lines, vec![String::new()]);
    }

    // Robust: empty input returns one empty line so callers can
    // unconditionally `.push(Line::from(row))`.
    #[test]
    fn wrap_text_empty_input_robust() {
        let lines = wrap_text_to_width("", 20);
        assert_eq!(lines, vec![String::new()]);
    }

    // (path_color tests live alongside the `message_view::path_color`
    // helper so they can use the in-module function directly without
    // needing a re-export.)

    // ─── parse_prompt_mode ─────────────────────────────────────────
    // Normal: each named preset parses to its variant.
    #[test]
    fn parse_prompt_mode_named_presets_normal() {
        assert!(matches!(parse_prompt_mode(":comet"), PromptMode::Comet));
        assert!(matches!(parse_prompt_mode(":moon"), PromptMode::Moon));
        assert!(matches!(parse_prompt_mode(":dice"), PromptMode::Dice));
        assert!(matches!(parse_prompt_mode(":notes"), PromptMode::Notes));
        assert!(matches!(
            parse_prompt_mode(":hourglass"),
            PromptMode::Hourglass
        ));
        assert!(matches!(parse_prompt_mode(":atom"), PromptMode::Atom));
    }

    // Normal: aliases resolve to the same variant.
    #[test]
    fn parse_prompt_mode_aliases_normal() {
        assert!(matches!(parse_prompt_mode(":moons"), PromptMode::Moon));
        assert!(matches!(parse_prompt_mode(":die"), PromptMode::Dice));
        assert!(matches!(parse_prompt_mode(":music"), PromptMode::Notes));
        assert!(matches!(parse_prompt_mode(":time"), PromptMode::Hourglass));
    }

    // Normal: bare char becomes Static.
    #[test]
    fn parse_prompt_mode_bare_char_static_normal() {
        if let PromptMode::Static(s) = parse_prompt_mode("⌬") {
            assert_eq!(s, "⌬");
        } else {
            panic!("expected Static");
        }
    }

    // Robust: empty input falls through to default Comet.
    #[test]
    fn parse_prompt_mode_empty_default_robust() {
        assert!(matches!(parse_prompt_mode(""), PromptMode::Comet));
    }

    // Robust: a too-long literal (>2 chars) falls through to default.
    #[test]
    fn parse_prompt_mode_long_literal_default_robust() {
        assert!(matches!(parse_prompt_mode("abcd"), PromptMode::Comet));
    }

    // ─── prompt_mode_frame ─────────────────────────────────────────
    // Normal: comet returns the comet glyph regardless of streaming/ms.
    #[test]
    fn prompt_mode_frame_comet_constant_normal() {
        assert_eq!(prompt_mode_frame(&PromptMode::Comet, false, 0), "☄");
        assert_eq!(prompt_mode_frame(&PromptMode::Comet, true, 1234), "☄");
    }

    // Normal: moon idle is full moon, streaming cycles through 4 phases.
    #[test]
    fn prompt_mode_frame_moon_cycle_normal() {
        assert_eq!(prompt_mode_frame(&PromptMode::Moon, false, 0), "●");
        // Frame at ms=0 is FRAMES[0] = "○".
        assert_eq!(prompt_mode_frame(&PromptMode::Moon, true, 0), "○");
        // Frame at ms=250 is FRAMES[1] = "◐".
        assert_eq!(prompt_mode_frame(&PromptMode::Moon, true, 250), "◐");
    }

    // Normal: dice idle stays at ⚀, streaming shuffles.
    #[test]
    fn prompt_mode_frame_dice_idle_normal() {
        assert_eq!(prompt_mode_frame(&PromptMode::Dice, false, 0), "⚀");
        // Streaming at ms=0 is the first face.
        assert_eq!(prompt_mode_frame(&PromptMode::Dice, true, 0), "⚀");
    }

    // Robust: hourglass flips every 800ms.
    #[test]
    fn prompt_mode_frame_hourglass_flip_robust() {
        assert_eq!(prompt_mode_frame(&PromptMode::Hourglass, true, 0), "⌛");
        assert_eq!(prompt_mode_frame(&PromptMode::Hourglass, true, 800), "⌚");
        assert_eq!(prompt_mode_frame(&PromptMode::Hourglass, true, 1600), "⌛");
    }
}

// =====================================================================

#[cfg(test)]
mod pure_helper_tests {
    use super::*;
    use std::sync::Arc;

    use crate::provider::{EventStream, ModelInfo, Provider, ProviderMessage, StreamOptions};
    use ratatui_textarea::TextArea;

    /// Stub provider for `App::new` — none of the helpers under test
    /// dispatch through it, but `App::new` requires a `dyn Provider`.
    struct TestProvider;

    #[async_trait::async_trait]
    impl Provider for TestProvider {
        fn name(&self) -> &str {
            "test"
        }
        fn available_models(&self) -> Vec<ModelInfo> {
            Vec::new()
        }
        async fn stream(
            &self,
            _messages: Vec<ProviderMessage>,
            _options: &StreamOptions,
        ) -> anyhow::Result<EventStream> {
            Ok(Box::pin(futures::stream::empty()))
        }
    }
    impl crate::provider::seal::Sealed for TestProvider {}

    fn fake_app() -> App {
        App::new(Arc::new(TestProvider), "test-model")
    }

    // --- pulse_color -------------------------------------------------

    #[test]
    fn pulse_color_t_zero_returns_first_normal() {
        // t=0 should give back exactly c1's RGB.
        let c1 = Color::Rgb(10, 20, 30);
        let c2 = Color::Rgb(200, 100, 50);
        assert_eq!(pulse_color(c1, c2, 0.0), Color::Rgb(10, 20, 30));
    }

    #[test]
    fn pulse_color_t_one_returns_second_normal() {
        // t=1 should give back c2's RGB exactly.
        let c1 = Color::Rgb(10, 20, 30);
        let c2 = Color::Rgb(200, 100, 50);
        assert_eq!(pulse_color(c1, c2, 1.0), Color::Rgb(200, 100, 50));
    }

    #[test]
    fn pulse_color_midpoint_blends_normal() {
        // t=0.5 should land between the endpoints in each channel.
        let c1 = Color::Rgb(0, 0, 0);
        let c2 = Color::Rgb(100, 200, 50);
        match pulse_color(c1, c2, 0.5) {
            Color::Rgb(r, g, b) => {
                assert!((45..=55).contains(&r), "midpoint r should be ~50, got {r}");
                assert!((95..=105).contains(&g), "midpoint g should be ~100");
                assert!((20..=30).contains(&b), "midpoint b should be ~25");
            }
            other => panic!("expected Rgb, got {other:?}"),
        }
    }

    #[test]
    fn pulse_color_named_color_falls_back_robust() {
        // Non-RGB endpoints can't interpolate — return c1 unchanged so the
        // pulse just freezes on that color rather than panicking.
        let c1 = Color::Red;
        let c2 = Color::Rgb(50, 50, 50);
        assert_eq!(pulse_color(c1, c2, 0.5), Color::Red);
    }

    #[test]
    fn pulse_color_named_second_falls_back_robust() {
        // Symmetric: c2 named, c1 RGB → also falls back to c1.
        let c1 = Color::Rgb(10, 10, 10);
        let c2 = Color::Blue;
        assert_eq!(pulse_color(c1, c2, 0.5), Color::Rgb(10, 10, 10));
    }

    // --- pulse_color_pub (public wrapper) ----------------------------

    #[test]
    fn pulse_color_pub_matches_private_normal() {
        let c1 = Color::Rgb(20, 40, 60);
        let c2 = Color::Rgb(100, 120, 140);
        assert_eq!(pulse_color_pub(c1, c2, 0.25), pulse_color(c1, c2, 0.25),);
    }

    // --- tail_truncate -----------------------------------------------

    #[test]
    fn tail_truncate_short_passes_through_normal() {
        // Below the cap → return unchanged.
        assert_eq!(tail_truncate("hello", 10), "hello");
    }

    #[test]
    fn tail_truncate_long_keeps_tail_normal() {
        // Long path: drop the head, prepend `…/`, keep the meaningful end.
        let result = tail_truncate("/home/cole/RustProjects/active/jfc", 12);
        assert!(result.starts_with("…/"), "got {result:?}");
        assert!(result.contains("jfc"), "got {result:?}");
        // Must respect the requested width (chars, not bytes).
        assert!(result.chars().count() <= 12, "got {result:?}");
    }

    #[test]
    fn tail_truncate_zero_max_returns_empty_robust() {
        // max=0 → empty string, not a panic.
        assert_eq!(tail_truncate("anything", 0), String::new());
    }

    #[test]
    fn tail_truncate_too_narrow_falls_back_to_head_truncate_robust() {
        // max < 4 leaves no room for the `…/` indicator → head-truncate
        // (matching truncate_str behavior with the trailing ellipsis).
        let result = tail_truncate("/foo/bar/baz", 3);
        assert_eq!(result.chars().count(), 3);
        assert!(result.ends_with('…'), "got {result:?}");
    }

    #[test]
    fn tail_truncate_unicode_chars_robust() {
        // Width is in chars, not bytes — multi-byte glyphs shouldn't break it.
        let s = "日本語/プロジェクト/foo";
        let result = tail_truncate(s, 8);
        assert!(result.chars().count() <= 8);
        assert!(result.starts_with("…/"), "got {result:?}");
    }

    // --- wrap_text_to_width ------------------------------------------

    #[test]
    fn wrap_text_short_returns_one_row_normal() {
        let rows = wrap_text_to_width("hello world", 80);
        assert_eq!(rows, vec!["hello world".to_string()]);
    }

    #[test]
    fn wrap_text_breaks_on_whitespace_normal() {
        // Each row is a complete fragment, broken on whitespace.
        let rows = wrap_text_to_width("alpha beta gamma delta", 12);
        for row in &rows {
            assert!(row.chars().count() <= 12, "row {row:?} exceeds width 12");
        }
        assert!(rows.len() >= 2, "should wrap into at least 2 rows");
    }

    #[test]
    fn wrap_text_zero_width_returns_single_empty_robust() {
        // width=0 short-circuits: one empty row so callers can `.push`.
        let rows = wrap_text_to_width("anything here", 0);
        assert_eq!(rows, vec![String::new()]);
    }

    #[test]
    fn wrap_text_long_word_hard_truncates_robust() {
        // A single word longer than width can't break on whitespace —
        // hard-truncate that word with `…` so it doesn't overflow.
        let rows = wrap_text_to_width("supercalifragilisticexpialidocious", 10);
        assert!(rows.iter().any(|r| r.ends_with('…')), "rows: {rows:?}");
        for r in &rows {
            assert!(r.chars().count() <= 10, "row {r:?} exceeded width");
        }
    }

    #[test]
    fn wrap_text_empty_input_returns_empty_row_robust() {
        // Empty input → at least one row so `out.push(Line::from(...))`
        // always has something to render.
        let rows = wrap_text_to_width("", 20);
        assert_eq!(rows, vec![String::new()]);
    }

    // --- parse_prompt_mode -------------------------------------------

    #[test]
    fn parse_prompt_mode_named_presets_normal() {
        assert!(matches!(parse_prompt_mode(":comet"), PromptMode::Comet));
        assert!(matches!(parse_prompt_mode(":moon"), PromptMode::Moon));
        assert!(matches!(parse_prompt_mode(":moons"), PromptMode::Moon));
        assert!(matches!(parse_prompt_mode(":dice"), PromptMode::Dice));
        assert!(matches!(parse_prompt_mode(":die"), PromptMode::Dice));
        assert!(matches!(parse_prompt_mode(":notes"), PromptMode::Notes));
        assert!(matches!(parse_prompt_mode(":music"), PromptMode::Notes));
        assert!(matches!(
            parse_prompt_mode(":hourglass"),
            PromptMode::Hourglass
        ));
        assert!(matches!(parse_prompt_mode(":time"), PromptMode::Hourglass));
        assert!(matches!(parse_prompt_mode(":atom"), PromptMode::Atom));
    }

    #[test]
    fn parse_prompt_mode_static_single_char_normal() {
        match parse_prompt_mode("⌬") {
            PromptMode::Static(s) => assert_eq!(s, "⌬"),
            other => panic!("expected Static, got {other:?}"),
        }
    }

    #[test]
    fn parse_prompt_mode_static_two_chars_normal() {
        match parse_prompt_mode("ab") {
            PromptMode::Static(s) => assert_eq!(s, "ab"),
            other => panic!("expected Static, got {other:?}"),
        }
    }

    #[test]
    fn parse_prompt_mode_long_input_falls_back_to_comet_robust() {
        // 3+ chars and not a named preset → fall back to Comet (default).
        assert!(matches!(parse_prompt_mode("xyz123"), PromptMode::Comet));
    }

    #[test]
    fn parse_prompt_mode_empty_falls_back_to_comet_robust() {
        // Empty string → comet (no Static branch since chars=0).
        assert!(matches!(parse_prompt_mode(""), PromptMode::Comet));
    }

    #[test]
    fn parse_prompt_mode_trims_whitespace_robust() {
        // Whitespace around a preset token must not break the match.
        assert!(matches!(parse_prompt_mode("  :moon  "), PromptMode::Moon));
    }

    // --- prompt_mode_frame -------------------------------------------

    #[test]
    fn prompt_mode_frame_static_glyphs_normal() {
        // Comet/Atom always return their static glyph regardless of state.
        assert_eq!(prompt_mode_frame(&PromptMode::Comet, false, 0), "☄");
        assert_eq!(prompt_mode_frame(&PromptMode::Comet, true, 9999), "☄");
        assert_eq!(prompt_mode_frame(&PromptMode::Atom, true, 0), "⚛");
    }

    #[test]
    fn prompt_mode_frame_idle_states_settle_normal() {
        // Non-streaming → each mode lands on its rest glyph.
        assert_eq!(prompt_mode_frame(&PromptMode::Moon, false, 0), "●");
        assert_eq!(prompt_mode_frame(&PromptMode::Dice, false, 0), "⚀");
        assert_eq!(prompt_mode_frame(&PromptMode::Notes, false, 0), "♪");
        assert_eq!(prompt_mode_frame(&PromptMode::Hourglass, false, 0), "⌛");
    }

    #[test]
    fn prompt_mode_frame_streaming_cycles_moon_normal() {
        // 4 distinct frames at 250ms cadence — sample several.
        let f0 = prompt_mode_frame(&PromptMode::Moon, true, 0);
        let f1 = prompt_mode_frame(&PromptMode::Moon, true, 250);
        let f2 = prompt_mode_frame(&PromptMode::Moon, true, 500);
        let f3 = prompt_mode_frame(&PromptMode::Moon, true, 750);
        assert!(
            [f0, f1, f2, f3]
                .iter()
                .all(|g| ["○", "◐", "●", "◑"].contains(g))
        );
    }

    #[test]
    fn prompt_mode_frame_hourglass_alternates_robust() {
        // 800ms flip cadence — at 0 and 1600ms, full glass; at 800ms,
        // empty.
        assert_eq!(prompt_mode_frame(&PromptMode::Hourglass, true, 0), "⌛");
        assert_eq!(prompt_mode_frame(&PromptMode::Hourglass, true, 800), "⌚");
        assert_eq!(prompt_mode_frame(&PromptMode::Hourglass, true, 1600), "⌛");
    }

    #[test]
    fn prompt_mode_frame_static_branch_returns_empty_sentinel_robust() {
        // Static is rendered separately by the input renderer; this fn
        // returns "" as a sentinel for that branch.
        assert_eq!(
            prompt_mode_frame(&PromptMode::Static("X".to_string()), true, 0),
            ""
        );
    }

    // --- comet_config_from_state -------------------------------------

    #[test]
    fn comet_config_idle_uses_accent_color_normal() {
        // Default idle app: head color = theme accent, lap_ms = 3500.
        let app = fake_app();
        let cfg = comet_config_from_state(&app, app.theme, 1);
        assert_eq!(cfg.count, 1);
        assert_eq!(cfg.head, app.theme.accent);
        assert_eq!(cfg.base, app.theme.border);
        assert!(!cfg.reverse_base, "idle should not reverse base");
    }

    #[test]
    fn comet_config_bash_mode_uses_warning_normal() {
        // Input starting with `!` → bash-mode → warning color.
        let mut app = fake_app();
        app.textarea = TextArea::from(vec!["!ls".to_string()]);
        let cfg = comet_config_from_state(&app, app.theme, 2);
        assert_eq!(cfg.head, app.theme.warning);
        assert!(
            !cfg.reverse_base,
            "bash mode wins over tool-running, no reverse"
        );
    }

    #[test]
    fn comet_config_running_tool_reverses_robust() {
        // A running tool in the latest message → reverse_base = true,
        // warning color.
        let mut app = fake_app();
        let tool = ToolCall {
            id: "t1".into(),
            kind: ToolKind::Bash,
            status: ToolStatus::Running,
            input: ToolInput::Bash {
                command: "sleep 1".into(),
                timeout: None,
                workdir: None,
            },
            output: ToolOutput::Empty,
            display: crate::types::ToolDisplayState::DEFAULT,
            elapsed_ms: None,
            started_at: None,
        };
        app.messages.push(ChatMessage {
            role: Role::Assistant,
            parts: vec![MessagePart::Tool(tool)],
            agent_name: None,
            model_name: None,
            cost_tier: None,
            elapsed: None,
            usage: None,
            queued: false,
            attachments: Vec::new(),
        });
        let cfg = comet_config_from_state(&app, app.theme, 1);
        assert_eq!(cfg.head, app.theme.warning);
        assert!(cfg.reverse_base, "tool-running should reverse direction");
    }

    #[test]
    fn comet_config_trail_clamped_robust() {
        // Trail is clamped to 2..=12 even with extreme env values.
        // Default (no env) is 6.
        let app = fake_app();
        // Make sure no env var pollutes this test.
        unsafe {
            std::env::remove_var("JFC_BORDER_COMET_TRAIL");
        }
        let cfg = comet_config_from_state(&app, app.theme, 1);
        assert!((2..=12).contains(&cfg.trail_len));
    }

    // --- perimeter_cells ---------------------------------------------

    #[test]
    fn perimeter_cells_3x3_rect_normal() {
        // 3x3 rect: 8 perimeter cells (no interior).
        let cells = perimeter_cells(Rect {
            x: 0,
            y: 0,
            width: 3,
            height: 3,
        });
        assert_eq!(cells.len(), 8);
        // First cell = top-left; last cell = (0,1) (left edge bottom-up
        // skipping bottom-left already added).
        assert_eq!(cells[0], (0, 0));
    }

    #[test]
    fn perimeter_cells_walks_clockwise_from_topleft_normal() {
        // 4x4: top edge L→R, right top→bottom, bottom R→L, left bottom→top.
        let cells = perimeter_cells(Rect {
            x: 0,
            y: 0,
            width: 4,
            height: 4,
        });
        // 4*4 = 16 total cells; perimeter = 4*4 - 2*2 = 12.
        assert_eq!(cells.len(), 12);
        assert_eq!(cells[0], (0, 0));
        // After top edge (4 cells), we should be on the right edge.
        assert_eq!(cells[4], (3, 1));
    }

    #[test]
    fn perimeter_cells_too_small_returns_empty_robust() {
        // Width or height < 2 → empty (no meaningful perimeter).
        let cells = perimeter_cells(Rect {
            x: 0,
            y: 0,
            width: 1,
            height: 5,
        });
        assert!(cells.is_empty());
        let cells = perimeter_cells(Rect {
            x: 0,
            y: 0,
            width: 5,
            height: 1,
        });
        assert!(cells.is_empty());
    }

    #[test]
    fn perimeter_cells_offset_rect_robust() {
        // Non-zero origin: the cells should reflect the absolute coords.
        let cells = perimeter_cells(Rect {
            x: 10,
            y: 20,
            width: 3,
            height: 3,
        });
        assert!(cells.contains(&(10, 20)));
        assert!(cells.contains(&(12, 22)));
        // Interior cell (11, 21) must NOT be in the perimeter.
        assert!(!cells.contains(&(11, 21)));
    }

    #[test]
    fn perimeter_cells_2x2_no_duplicates_robust() {
        // The smallest valid rect: 2x2. Each corner is one cell;
        // none should be duplicated.
        let cells = perimeter_cells(Rect {
            x: 0,
            y: 0,
            width: 2,
            height: 2,
        });
        let mut sorted = cells.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), cells.len(), "duplicates: {cells:?}");
        assert_eq!(cells.len(), 4);
    }

    // --- input_visual_line_count + input_soft_wrapped_lines ----------

    #[test]
    fn input_visual_line_count_short_text_one_line_normal() {
        let mut app = fake_app();
        app.textarea = TextArea::from(vec!["abc".to_string()]);
        assert_eq!(input_visual_line_count(&app, 80), 1);
    }

    #[test]
    fn input_visual_line_count_wraps_long_line_normal() {
        // A 12-char line at width 5 = 3 visual rows (5/5/2).
        let mut app = fake_app();
        app.textarea = TextArea::from(vec!["abcdefghijkl".to_string()]);
        assert_eq!(input_visual_line_count(&app, 5), 3);
    }

    #[test]
    fn input_visual_line_count_empty_returns_one_robust() {
        // Empty input still renders the placeholder — count is 1, not 0.
        let app = fake_app();
        assert_eq!(input_visual_line_count(&app, 80), 1);
    }

    #[test]
    fn input_soft_wrapped_cursor_at_start_normal() {
        // Cursor at (0, 0) → visual_cursor_row=0, col=0.
        let mut app = fake_app();
        app.textarea = TextArea::from(vec!["hello".to_string()]);
        let (lines, row, col) = input_soft_wrapped_lines(&app, 80);
        assert_eq!(lines.len(), 1);
        assert_eq!(row, 0);
        assert_eq!(col, 0);
    }

    #[test]
    fn input_soft_wrapped_cursor_after_wrap_robust() {
        // 10-char line at width=5 → wraps to 2 visual rows. Cursor at
        // logical col 8 → visual row 1 col 3.
        let mut app = fake_app();
        app.textarea = TextArea::from(vec!["abcdefghij".to_string()]);
        app.textarea
            .move_cursor(ratatui_textarea::CursorMove::Jump(0, 8));
        let (lines, row, col) = input_soft_wrapped_lines(&app, 5);
        assert_eq!(lines.len(), 2);
        assert_eq!(row, 1);
        assert_eq!(col, 3);
    }

    #[test]
    fn input_soft_wrapped_empty_uses_placeholder_robust() {
        // All-empty input → placeholder string is the only line.
        let app = fake_app();
        let (lines, row, col) = input_soft_wrapped_lines(&app, 80);
        assert_eq!(lines, vec!["send a message…".to_string()]);
        assert_eq!(row, 0);
        assert_eq!(col, 0);
    }

    // --- input_line_to_spans -----------------------------------------

    #[test]
    fn input_line_spans_empty_returns_one_raw_normal() {
        let t = Theme::dark();
        let spans = input_line_to_spans("", t, 0.0);
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].content, "");
    }

    #[test]
    fn input_line_spans_slash_command_colors_each_char_normal() {
        // Every char of the slash token gets its own colored span (rainbow).
        let t = Theme::dark();
        let spans = input_line_to_spans("/help", t, 0.0);
        // The leading `/` plus each char → 5 styled spans with no rest.
        assert_eq!(spans.len(), 5);
    }

    #[test]
    fn input_line_spans_slash_with_args_keeps_rest_normal() {
        // After the slash token, the rest goes through highlight_mentions_in.
        let t = Theme::dark();
        let spans = input_line_to_spans("/cmd arg1 @user", t, 0.0);
        // At minimum one span per char in `/cmd` plus one or more for the rest.
        assert!(spans.len() > 4);
    }

    #[test]
    fn input_line_spans_plain_text_falls_through_to_mentions_robust() {
        // No slash → just `highlight_mentions_in` output.
        let t = Theme::dark();
        let spans = input_line_to_spans("hello world", t, 0.0);
        let joined: String = spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(joined, "hello world");
    }

    #[test]
    fn input_line_spans_leading_whitespace_preserved_robust() {
        // Indent before a slash command must be preserved verbatim.
        let t = Theme::dark();
        let spans = input_line_to_spans("   /help", t, 0.0);
        let joined: String = spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(joined, "   /help");
    }

    // --- highlight_mentions_in ---------------------------------------

    #[test]
    fn highlight_mentions_no_at_returns_one_span_normal() {
        let t = Theme::dark();
        let spans = highlight_mentions_in("just plain text", t, 0.0);
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].content, "just plain text");
    }

    #[test]
    fn highlight_mentions_at_token_gets_per_char_spans_normal() {
        // `@cole` at the start → 5 styled spans (one per char). With
        // empty prefix there's no leading text span.
        let t = Theme::dark();
        let spans = highlight_mentions_in("@cole", t, 0.0);
        assert_eq!(spans.len(), 5);
        let joined: String = spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(joined, "@cole");
    }

    #[test]
    fn highlight_mentions_only_after_whitespace_robust() {
        // Only `@` after whitespace (or at start) is a mention; mid-word
        // `@` (e.g. email) doesn't trigger.
        let t = Theme::dark();
        let spans = highlight_mentions_in("user@example.com", t, 0.0);
        // Exactly one prefix span (no mention split).
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].content, "user@example.com");
    }

    #[test]
    fn highlight_mentions_text_then_mention_robust() {
        // "hi @cole" → ["hi "] + 5 chars of "@cole" = 6 spans.
        let t = Theme::dark();
        let spans = highlight_mentions_in("hi @cole", t, 0.0);
        assert_eq!(spans.len(), 6);
        let joined: String = spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(joined, "hi @cole");
    }

    // --- gauge_color (existing private helper) -----------------------

    #[test]
    fn gauge_color_buckets_normal() {
        let t = Theme::dark();
        // 0..60% = success, 60..85% = warning, 85+ = error.
        assert_eq!(gauge_color(0.0, t), t.success);
        assert_eq!(gauge_color(50.0, t), t.success);
        assert_eq!(gauge_color(70.0, t), t.warning);
        assert_eq!(gauge_color(90.0, t), t.error);
    }

    #[test]
    fn gauge_color_boundaries_robust() {
        let t = Theme::dark();
        // 60.0 — exactly on warning boundary.
        assert_eq!(gauge_color(60.0, t), t.warning);
        // 85.0 — exactly on error boundary.
        assert_eq!(gauge_color(85.0, t), t.error);
        // Just below boundaries.
        assert_eq!(gauge_color(59.9, t), t.success);
        assert_eq!(gauge_color(84.9, t), t.warning);
    }

    // --- truncate_str (private) --------------------------------------

    #[test]
    fn truncate_str_short_passes_through_normal() {
        assert_eq!(truncate_str("hello", 10), "hello");
    }

    #[test]
    fn truncate_str_long_appends_ellipsis_normal() {
        let result = truncate_str("hello world", 5);
        assert!(result.ends_with('…'), "got {result:?}");
        assert_eq!(result.chars().count(), 5);
    }

    #[test]
    fn truncate_str_zero_max_returns_empty_robust() {
        assert_eq!(truncate_str("anything", 0), "");
    }

    #[test]
    fn truncate_str_unicode_counts_chars_not_bytes_robust() {
        // CJK chars are 3 bytes each but 1 column in our model.
        let s = "日本語のテキスト";
        let result = truncate_str(s, 4);
        assert_eq!(result.chars().count(), 4);
        assert!(result.ends_with('…'));
    }

    // --- fmt_number --------------------------------------------------

    #[test]
    fn fmt_number_below_thousand_normal() {
        assert_eq!(fmt_number(0), "0");
        assert_eq!(fmt_number(42), "42");
        assert_eq!(fmt_number(999), "999");
    }

    #[test]
    fn fmt_number_thousands_get_separator_normal() {
        assert_eq!(fmt_number(1_000), "1,000");
        assert_eq!(fmt_number(12_345), "12,345");
        assert_eq!(fmt_number(999_999), "999,999");
    }

    #[test]
    fn fmt_number_millions_get_decimal_robust() {
        assert_eq!(fmt_number(1_000_000), "1.0M");
        assert_eq!(fmt_number(1_500_000), "1.5M");
        assert_eq!(fmt_number(12_345_678), "12.3M");
    }

    // --- context_gauge_label -----------------------------------------

    #[test]
    fn context_gauge_label_format_normal() {
        let label = context_gauge_label(50_000, 200_000, 25);
        assert_eq!(label, " ctx 50k / 200k · 25% ");
    }

    #[test]
    fn context_gauge_label_zero_used_robust() {
        let label = context_gauge_label(0, 200_000, 0);
        assert_eq!(label, " ctx 0k / 200k · 0% ");
    }

    // --- effort_status_badge -----------------------------------------

    #[test]
    fn effort_status_badge_shows_default_when_unpinned_normal() {
        let app = fake_app();
        assert_eq!(effort_status_badge(&app), "effort default".to_string());
    }

    #[test]
    fn effort_status_badge_shows_pinned_level_normal() {
        let mut app = fake_app();
        app.effort_state.set(crate::effort::ReasoningEffort::XHigh);
        assert_eq!(effort_status_badge(&app), "effort xhigh".to_string());
    }

    // --- provider_color / provider_label -----------------------------

    #[test]
    fn provider_color_known_providers_normal() {
        assert_eq!(provider_color("anthropic"), Color::Rgb(204, 120, 50));
        assert_eq!(provider_color("anthropic-oauth"), Color::Rgb(204, 120, 50));
        assert_eq!(provider_color("openwebui"), Color::Rgb(100, 180, 200));
    }

    #[test]
    fn provider_color_unknown_returns_gray_robust() {
        assert_eq!(provider_color("xenu"), Color::Gray);
        assert_eq!(provider_color(""), Color::Gray);
    }

    #[test]
    fn provider_label_known_providers_normal() {
        assert_eq!(provider_label("anthropic"), "API");
        assert_eq!(provider_label("anthropic-oauth"), "OAuth");
        assert_eq!(provider_label("openwebui"), "OpenWebUI");
    }

    #[test]
    fn provider_label_unknown_returns_question_mark_robust() {
        assert_eq!(provider_label("???"), "?");
        assert_eq!(provider_label(""), "?");
    }

    // --- collect_diff_stats ------------------------------------------

    #[test]
    fn collect_diff_stats_empty_app_normal() {
        let app = fake_app();
        let stats = collect_diff_stats(&app);
        assert_eq!(stats.total_files, 0);
        assert_eq!(stats.additions, 0);
        assert_eq!(stats.deletions, 0);
        assert!(stats.files.is_empty());
    }

    #[test]
    fn collect_diff_stats_aggregates_diffs_normal() {
        let mut app = fake_app();
        let diff = DiffView {
            file_path: "src/foo.rs".into(),
            hunks: Vec::new(),
            additions: 10,
            deletions: 3,
        };
        let tool = ToolCall {
            id: "t1".into(),
            kind: ToolKind::Edit,
            status: ToolStatus::Completed,
            input: ToolInput::Edit {
                file_path: "src/foo.rs".into(),
                old_string: "".into(),
                new_string: "".into(),
                replacement: ReplacementMode::FirstOnly,
            },
            output: ToolOutput::Diff(diff),
            display: crate::types::ToolDisplayState::DEFAULT,
            elapsed_ms: None,
            started_at: None,
        };
        app.messages.push(ChatMessage {
            role: Role::Assistant,
            parts: vec![MessagePart::Tool(tool)],
            agent_name: None,
            model_name: None,
            cost_tier: None,
            elapsed: None,
            usage: None,
            queued: false,
            attachments: Vec::new(),
        });
        let stats = collect_diff_stats(&app);
        assert_eq!(stats.total_files, 1);
        assert_eq!(stats.additions, 10);
        assert_eq!(stats.deletions, 3);
        assert_eq!(stats.files, vec!["src/foo.rs".to_string()]);
    }

    #[test]
    fn collect_diff_stats_dedupes_by_path_robust() {
        // Two diffs for the same file → last entry wins (not summed).
        let mut app = fake_app();
        for (i, (a, d)) in [(5, 1), (10, 3)].into_iter().enumerate() {
            let tool = ToolCall {
                id: crate::ids::ToolId::from(format!("t{i}")),
                kind: ToolKind::Edit,
                status: ToolStatus::Completed,
                input: ToolInput::Edit {
                    file_path: "src/foo.rs".into(),
                    old_string: "".into(),
                    new_string: "".into(),
                    replacement: ReplacementMode::FirstOnly,
                },
                output: ToolOutput::Diff(DiffView {
                    file_path: "src/foo.rs".into(),
                    hunks: Vec::new(),
                    additions: a,
                    deletions: d,
                }),
                display: crate::types::ToolDisplayState::DEFAULT,
                elapsed_ms: None,
                started_at: None,
            };
            app.messages.push(ChatMessage {
                role: Role::Assistant,
                parts: vec![MessagePart::Tool(tool)],
                agent_name: None,
                model_name: None,
                cost_tier: None,
                elapsed: None,
                usage: None,
                queued: false,
                attachments: Vec::new(),
            });
        }
        let stats = collect_diff_stats(&app);
        // De-duped to 1 file, last edit wins → +10/-3.
        assert_eq!(stats.total_files, 1);
        assert_eq!(stats.additions, 10);
        assert_eq!(stats.deletions, 3);
    }

    // --- current_slash_prefix / slash_matches ------------------------

    #[test]
    fn current_slash_prefix_single_token_normal() {
        let mut app = fake_app();
        app.textarea = TextArea::from(vec!["/he".to_string()]);
        assert_eq!(current_slash_prefix(&app), Some("/he".to_string()));
    }

    #[test]
    fn current_slash_prefix_with_args_drops_after_space_normal() {
        let mut app = fake_app();
        app.textarea = TextArea::from(vec!["/help me".to_string()]);
        assert_eq!(current_slash_prefix(&app), Some("/help".to_string()));
    }

    #[test]
    fn current_slash_prefix_no_slash_returns_none_robust() {
        let mut app = fake_app();
        app.textarea = TextArea::from(vec!["hello".to_string()]);
        assert_eq!(current_slash_prefix(&app), None);
    }

    #[test]
    fn current_slash_prefix_multiline_returns_none_robust() {
        let mut app = fake_app();
        app.textarea = TextArea::from(vec!["/help".to_string(), "extra".to_string()]);
        assert_eq!(current_slash_prefix(&app), None);
    }

    #[test]
    fn slash_matches_filters_prefix_normal() {
        // Filter the static SLASH_COMMANDS list by prefix.
        let matches = slash_matches("/");
        assert!(!matches.is_empty(), "/ should match every command");
    }

    #[test]
    fn slash_matches_no_hits_robust() {
        let matches = slash_matches("/zzz_nonexistent");
        assert!(matches.is_empty());
    }

    // --- ordered_sidebar_sessions ------------------------------------

    #[test]
    fn ordered_sidebar_sessions_empty_app_normal() {
        let app = fake_app();
        // No saved sessions means empty result (the helper just orders
        // app.session_meta which starts empty).
        let sessions = ordered_sidebar_sessions(&app);
        assert!(sessions.is_empty());
    }
}

#[cfg(test)]
mod subagent_counter_tests {
    use super::*;
    use crate::app::BackgroundTask;
    use crate::types::TaskLifecycle;

    fn task_with(tools: u32, in_tok: u64, out_tok: u64) -> BackgroundTask {
        BackgroundTask {
            task_id: "t1".into(),
            description: "research".into(),
            status: TaskLifecycle::Running,
            started_at: std::time::Instant::now(),
            summary: None,
            error: None,
            last_tool: None,
            messages: Vec::new(),
            chat_messages: Vec::new(),
            tool_use_count: tools,
            latest_input_tokens: in_tok,
            latest_cache_read_tokens: 0,
            latest_cache_write_tokens: 0,
            cumulative_output_tokens: out_tok,
            model_used: None,
            max_input_tokens: None,
            budget_killed: false,
            parent_task_id: None,
        }
    }

    // Normal: <1000 tokens stays raw.
    #[test]
    fn format_token_count_under_thousand_raw_normal() {
        assert_eq!(format_token_count(0), "0");
        assert_eq!(format_token_count(1), "1");
        assert_eq!(format_token_count(999), "999");
    }

    // Normal: >=1000 collapses to single-decimal "k".
    #[test]
    fn format_token_count_thousands_normal() {
        assert_eq!(format_token_count(1_000), "1.0k");
        assert_eq!(format_token_count(8_945), "8.9k");
        assert_eq!(format_token_count(89_745), "89.7k");
    }

    // Normal: >=1_000_000 collapses to single-decimal "M".
    #[test]
    fn format_token_count_millions_normal() {
        assert_eq!(format_token_count(1_000_000), "1.0M");
        assert_eq!(format_token_count(1_240_000), "1.2M");
    }

    // Robust: u64::MAX renders without panicking.
    #[test]
    fn format_token_count_u64_max_robust() {
        let _ = format_token_count(u64::MAX);
    }

    // Normal: subagent counters render in v131-style suffix order
    // (tool count, then token count).
    #[test]
    fn format_subagent_counters_full_normal() {
        let bt = task_with(22, 50_000, 39_745);
        let s = format_subagent_counters(&bt);
        assert!(s.contains("22 tools"));
        assert!(s.contains("89.7k tok"));
        assert!(s.starts_with(" · "));
    }

    // Normal: singular form for exactly 1 tool.
    #[test]
    fn format_subagent_counters_singular_tool_normal() {
        let bt = task_with(1, 0, 500);
        let s = format_subagent_counters(&bt);
        assert!(s.contains("1 tool"));
        assert!(!s.contains("1 tools"));
    }

    // Robust: zero tools and zero tokens produces an empty suffix
    // (we suppress the row entirely until the agent has reported
    // something, otherwise the UI flickers " · 0 tools" right after
    // spawn).
    #[test]
    fn format_subagent_counters_empty_when_zero_robust() {
        let bt = task_with(0, 0, 0);
        assert_eq!(format_subagent_counters(&bt), "");
    }

    // Robust: tool count without tokens still renders (and vice versa).
    #[test]
    fn format_subagent_counters_partial_data_robust() {
        let only_tools = task_with(3, 0, 0);
        let s = format_subagent_counters(&only_tools);
        assert!(s.contains("3 tools"));
        assert!(!s.contains("tok"));

        let only_tokens = task_with(0, 1_500, 0);
        let s2 = format_subagent_counters(&only_tokens);
        assert!(s2.contains("1.5k tok"));
        assert!(!s2.contains("tools"));
    }

    // Normal: combined input + cumulative_output sum is what gets
    // formatted (matches v131's `latestInputTokens + cumulativeOutputTokens`).
    #[test]
    fn format_subagent_counters_sums_input_and_output_normal() {
        let bt = task_with(0, 80_000, 9_745);
        let s = format_subagent_counters(&bt);
        assert!(s.contains("89.7k tok"));
    }
}

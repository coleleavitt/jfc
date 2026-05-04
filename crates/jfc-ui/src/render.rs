use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Position, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Clear, LineGauge, List, ListItem, Paragraph, Row, Table},
};

#[allow(unused_imports)]
use ratatui::style::Stylize as _;

use crate::app::{App, ApprovalChoice};
use crate::input::{filtered_models, palette_items};
use crate::markdown;
use crate::theme::Theme;
use crate::types::*;

pub fn frame(f: &mut Frame, app: &mut App) {
    let t = app.theme;

    f.render_widget(Block::default().style(Style::default().bg(t.bg)), f.area());

    let input_lines = input_visual_line_count(app, f.area().width.saturating_sub(4) as usize);
    let input_height = (input_lines + 2).min(8) as u16;
    let subagent_footer_height: u16 = if app.viewing_task_id.is_some() { 1 } else { 0 };
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
    let spinner_row_height: u16 = if app.is_streaming { 2 } else { 0 };
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

    let show_left = app.show_sidebar;
    let show_right = app.show_info_sidebar && f.area().width >= 100;

    match (show_left, show_right) {
        (true, true) => {
            let split = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([
                    Constraint::Length(28),
                    Constraint::Min(20),
                    Constraint::Length(42),
                ])
                .split(chunks[0]);
            sidebar(f, app, split[0]);
            messages(f, app, split[1]);
            info_sidebar(f, app, split[2]);
        }
        (true, false) => {
            let split = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Length(28), Constraint::Min(20)])
                .split(chunks[0]);
            sidebar(f, app, split[0]);
            messages(f, app, split[1]);
        }
        (false, true) => {
            let split = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Min(20), Constraint::Length(42)])
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
    if app.is_streaming {
        spinner_row(f, app, chunks[3]);
    }
    input(f, app, chunks[4]);
    status(f, app, chunks[5]);

    if app.show_palette {
        palette(f, app);
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

    if app.pending_approval.is_some() {
        approval(f, app);
    }
}

fn info_sidebar(f: &mut Frame, app: &mut App, area: Rect) {
    use crate::types::{LspStatus, McpStatus};

    let t = app.theme;

    let block = Block::default()
        .borders(Borders::LEFT)
        .border_style(Style::default().fg(t.border));
    let inner = block.inner(area);
    f.render_widget(block, area);

    let mut lines: Vec<Line> = Vec::new();

    lines.push(Line::from(vec![Span::styled(
        "Session",
        Style::default()
            .fg(t.text_primary)
            .add_modifier(Modifier::BOLD),
    )]));

    let title = app
        .current_session_id
        .as_deref()
        .unwrap_or("untitled")
        .to_owned();
    lines.push(Line::from(vec![Span::styled(
        truncate_str(&title, inner.width as usize),
        Style::default().fg(t.text_secondary),
    )]));

    lines.push(Line::from(""));

    lines.push(Line::from(vec![Span::styled(
        "Context",
        Style::default()
            .fg(t.text_primary)
            .add_modifier(Modifier::BOLD),
    )]));

    let total_tokens = (app.last_usage_input as u64).max(app.tool_ctx.approx_tokens as u64);
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
        lines.push(Line::from(vec![Span::styled(
            "Usage by model",
            Style::default()
                .fg(t.text_primary)
                .add_modifier(Modifier::BOLD),
        )]));

        let mut model_entries: Vec<(&String, &crate::types::ModelUsage)> =
            app.usage_by_model.iter().collect();
        model_entries.sort_by_key(|(k, _)| k.as_str());

        for (model_name, usage) in &model_entries {
            lines.push(Line::from(vec![Span::styled(
                format!(
                    " {}:",
                    truncate_str(model_name, inner.width.saturating_sub(2) as usize)
                ),
                Style::default().fg(t.accent).add_modifier(Modifier::BOLD),
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

        lines.push(Line::from(""));
    }

    if !app.mcp_servers.is_empty() {
        lines.push(Line::from(vec![Span::styled(
            "▼ MCP",
            Style::default()
                .fg(t.text_primary)
                .add_modifier(Modifier::BOLD),
        )]));

        for srv in &app.mcp_servers {
            let (dot_color, status_color) = match srv.status {
                McpStatus::Connected => (t.success, t.text_muted),
                McpStatus::Disabled => (t.text_muted, t.text_muted),
                McpStatus::Error => (t.error, t.error),
            };
            lines.push(Line::from(vec![
                Span::styled("• ", Style::default().fg(dot_color)),
                Span::styled(
                    truncate_str(&srv.name, inner.width.saturating_sub(14) as usize),
                    Style::default().fg(t.accent),
                ),
                Span::raw(" "),
                Span::styled(srv.status.label(), Style::default().fg(status_color)),
            ]));
        }

        lines.push(Line::from(""));
    }

    lines.push(Line::from(vec![Span::styled(
        "LSP",
        Style::default()
            .fg(t.text_primary)
            .add_modifier(Modifier::BOLD),
    )]));

    if app.lsp_servers.is_empty() {
        lines.push(Line::from(vec![Span::styled(
            "LSPs will activate as files are read",
            Style::default().fg(t.text_muted),
        )]));
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
        lines.push(Line::from(vec![Span::styled(
            format!("Tasks ({}/{} done)", completed.len(), task_total),
            Style::default()
                .fg(t.text_primary)
                .add_modifier(Modifier::BOLD),
        )]));

        // Show in-progress tasks with activity
        for task in in_progress.iter().take(3) {
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
        }

        // Show pending tasks (cap visible rows at 3 minus what in_progress
        // already used). usize annotation avoids the integer-literal `3` being
        // ambiguous when calling `saturating_sub`.
        let pending_slots: usize = 3usize.saturating_sub(in_progress.len());
        for task in pending.iter().take(pending_slots) {
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
                    .map_or(false, |t| now.duration_since(*t).as_secs() < 30)
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

        // Show "+N more" if truncated
        let shown =
            in_progress.len().min(3) + pending.len().min(3usize.saturating_sub(in_progress.len()));
        let hidden = task_total.saturating_sub(shown);
        if hidden > 0 {
            lines.push(Line::from(vec![Span::styled(
                format!("  … +{} more (Ctrl+T)", hidden),
                Style::default().fg(t.text_muted),
            )]));
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

    let used = lines.len() as u16;
    let available = inner.height.saturating_sub(3);
    if used < available {
        for _ in used..available {
            lines.push(Line::from(""));
        }
    }

    lines.push(Line::from(vec![Span::styled(
        "─".repeat(inner.width as usize),
        Style::default().fg(t.border),
    )]));

    let cwd_str = std::env::current_dir()
        .map(|p| {
            let s = p.display().to_string();
            let home = std::env::var("HOME").unwrap_or_default();
            if !home.is_empty() && s.starts_with(&home) {
                format!("~{}", &s[home.len()..])
            } else {
                s
            }
        })
        .unwrap_or_else(|_| "?".into());
    lines.push(Line::from(vec![Span::styled(
        truncate_str(&cwd_str, inner.width as usize),
        Style::default().fg(t.text_muted),
    )]));

    let provider_name = app.provider.name();
    lines.push(Line::from(vec![
        Span::styled("• ", Style::default().fg(t.success)),
        Span::styled(
            truncate_str(provider_name, inner.width.saturating_sub(10) as usize),
            Style::default()
                .fg(t.text_primary)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" "),
        Span::styled("local", Style::default().fg(t.text_muted)),
    ]));

    let para = Paragraph::new(lines).style(Style::default().bg(t.bg));
    f.render_widget(para, inner);
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
struct DiffStats {
    total_files: usize,
    additions: usize,
    deletions: usize,
    files: Vec<String>,
}

fn collect_diff_stats(app: &App) -> DiffStats {
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

/// Sessions sidebar — toggled with Ctrl+B. Renders the saved-session ids from
/// `~/.config/jfc/sessions/` (cached on `App::session_ids` so render() does no
/// disk I/O). Selecting a row with Enter loads its messages into `App::messages`.
fn sidebar(f: &mut Frame, app: &mut App, area: Rect) {
    let t = app.theme;
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(t.border))
        .title(Span::styled(
            " sessions ",
            Style::default().fg(t.accent).add_modifier(Modifier::BOLD),
        ))
        .title_bottom(
            Line::from(Span::styled(
                " ↑↓ · Enter ",
                Style::default().fg(t.text_muted),
            ))
            .right_aligned(),
        )
        .style(Style::default().bg(t.surface));

    let items: Vec<ListItem> = if app.session_ids.is_empty() {
        vec![ListItem::new(Line::from(Span::styled(
            "  (no saved sessions)",
            Style::default()
                .fg(t.text_muted)
                .add_modifier(Modifier::ITALIC),
        )))]
    } else {
        app.session_ids
            .iter()
            .map(|id| {
                let is_active = app.current_session_id.as_deref() == Some(id.as_str());
                let prefix = if is_active { "● " } else { "  " };
                let row = format!("{prefix}{}", session_display(id));
                ListItem::new(Line::from(Span::styled(
                    row,
                    if is_active {
                        Style::default().fg(t.accent).add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().fg(t.text_primary)
                    },
                )))
            })
            .collect()
    };

    let list = List::new(items)
        .block(block)
        .highlight_style(
            Style::default()
                .bg(t.surface_raised)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("▶ ");
    f.render_stateful_widget(list, area, &mut app.session_list_state);
}

/// Convert a session id like `ses_20260503_212945` into a friendly
/// `2026-05-03 21:29` for the sidebar list.
fn session_display(id: &str) -> String {
    let cleaned = id.strip_prefix("ses_").unwrap_or(id);
    let mut parts = cleaned.splitn(2, '_');
    let date = parts.next().unwrap_or("");
    let time = parts.next().unwrap_or("");
    if date.len() == 8 && time.len() >= 4 {
        format!(
            "{}-{}-{} {}:{}",
            &date[..4],
            &date[4..6],
            &date[6..8],
            &time[..2],
            &time[2..4]
        )
    } else {
        id.to_owned()
    }
}

fn messages(f: &mut Frame, app: &mut App, area: Rect) {
    use crate::message_view::MessageView;
    use ratatui::widgets::Widget;

    let t = app.theme;

    if let Some(ref task_id) = app.viewing_task_id.clone() {
        messages_task_view(f, app, area, task_id);
        return;
    }

    let inner_width = area.width.saturating_sub(2) as usize;
    let total_lines = crate::message_view::message_view_total_lines(app, inner_width);

    app.total_lines = total_lines;

    let visible = area.height.saturating_sub(2) as usize;
    app.viewport_height = visible;

    if app.follow_bottom {
        app.scroll_offset = total_lines.saturating_sub(visible);
    } else if app.scroll_offset + visible > total_lines {
        app.scroll_offset = total_lines.saturating_sub(visible);
    }

    let at_bottom = app.is_at_bottom();
    let title_right = if !at_bottom {
        let remaining = total_lines.saturating_sub(app.scroll_offset + visible);
        format!(" ↓ {remaining} more ")
    } else {
        String::new()
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(t.border))
        .title(Span::styled(
            " jfc ",
            Style::default().fg(t.accent).add_modifier(Modifier::BOLD),
        ))
        .title_top(
            Line::from(Span::styled(title_right, Style::default().fg(t.text_muted)))
                .right_aligned(),
        )
        .style(Style::default().bg(t.bg));

    let inner = block.inner(area);
    f.render_widget(block, area);

    if app.messages.is_empty() && app.streaming_text.is_empty() {
        let placeholder = Paragraph::new(vec![
            Line::from(""),
            Line::from(Span::styled(
                "What can I help you with?",
                Style::default().fg(t.text_muted),
            )),
        ])
        .style(Style::default().bg(t.bg));
        f.render_widget(placeholder, inner);
    } else {
        MessageView { app }.render(inner, f.buffer_mut());
    }
}

fn message_view_total_lines(app: &App, inner_width: usize) -> usize {
    use crate::types::*;

    let mut total = 0usize;

    for (idx, msg) in app.messages.iter().enumerate() {
        if app.streaming_assistant_idx == Some(idx) && app.is_streaming {
            continue;
        }

        total += 1;

        let reasoning_expanded = app.reasoning_expanded.get(&idx).copied().unwrap_or(false);

        for part in &msg.parts {
            match part {
                MessagePart::Text(text) => {
                    total += crate::markdown::to_lines(text, &app.theme, inner_width).len();
                }
                MessagePart::Reasoning(text) => {
                    if reasoning_expanded {
                        total += 1 + text.lines().count();
                    } else {
                        total += 1;
                    }
                }
                MessagePart::Tool(tool) => {
                    total += crate::message_view::tool_block_height_pub(tool, inner_width);
                }
                MessagePart::TaskStatus(ts) => {
                    total += 1;
                    if ts.error.is_some() {
                        total += 1;
                    }
                }
                MessagePart::CompactBoundary { .. } => {
                    total += 1;
                }
            }
        }

        total += 1;
    }

    if app.is_streaming || !app.streaming_text.is_empty() || !app.streaming_reasoning.is_empty() {
        total += 1;
        if !app.streaming_reasoning.is_empty() {
            total += 1;
        }
        total += crate::markdown::to_lines(&app.streaming_text, &app.theme, inner_width).len();
        if app.is_streaming {
            total += 1;
        }
        total += 1;
    }

    total
}

fn messages_task_view(f: &mut Frame, app: &mut App, area: Rect, task_id: &str) {
    let t = app.theme;

    let (title_str, body_lines) = match app.background_tasks.get(task_id) {
        None => (format!("task {task_id} (not found)"), Vec::new()),
        Some(bt) => {
            let title = format!(
                " {} · {} ",
                &bt.task_id[..bt.task_id.len().min(12)],
                bt.description
            );
            let lines: Vec<Line<'static>> = bt
                .messages
                .iter()
                .map(|m| {
                    Line::from(Span::styled(
                        m.clone(),
                        Style::default().fg(t.text_secondary),
                    ))
                })
                .collect();
            (title, lines)
        }
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(t.accent))
        .title(Span::styled(
            title_str,
            Style::default().fg(t.accent).add_modifier(Modifier::BOLD),
        ))
        .style(Style::default().bg(t.bg));

    let inner = block.inner(area);
    f.render_widget(block, area);

    let total_lines = body_lines.len();
    let visible = inner.height as usize;

    if app.follow_bottom {
        app.scroll_offset = total_lines.saturating_sub(visible);
    }

    app.total_lines = total_lines;
    app.viewport_height = visible;

    if body_lines.is_empty() {
        let placeholder = Paragraph::new(vec![
            Line::from(""),
            Line::from(Span::styled(
                "No messages yet for this background task.",
                Style::default().fg(t.text_muted),
            )),
        ])
        .style(Style::default().bg(t.bg));
        f.render_widget(placeholder, inner);
    } else {
        let visible_lines: Vec<Line> = body_lines
            .into_iter()
            .skip(app.scroll_offset)
            .take(visible)
            .collect();
        let para = Paragraph::new(visible_lines)
            .style(Style::default().bg(t.bg))
            .wrap(ratatui::widgets::Wrap { trim: false });
        f.render_widget(para, inner);
    }
}

fn subagent_footer(f: &mut Frame, app: &App, area: Rect) {
    let t = app.theme;
    let task_ids: Vec<&String> = app.background_tasks.keys().collect();
    let task_count = task_ids.len();

    let (task_desc, pos_label) = match &app.viewing_task_id {
        None => (String::from("no task"), String::from("─")),
        Some(id) => {
            let desc = app
                .background_tasks
                .get(id)
                .map(|bt| bt.description.as_str())
                .unwrap_or(id.as_str())
                .to_owned();
            let pos = task_ids.iter().position(|t| *t == id).unwrap_or(0);
            let label = format!("{} of {}", pos + 1, task_count);
            (desc, label)
        }
    };

    let short_id = app
        .viewing_task_id
        .as_deref()
        .and_then(|id| id.get(..8))
        .unwrap_or("");

    let line = Line::from(vec![
        Span::styled("◀ back (↑)  ", Style::default().fg(t.text_muted)),
        Span::styled("task  ", Style::default().fg(t.accent)),
        Span::styled(
            format!("{short_id}  "),
            Style::default().fg(t.text_secondary),
        ),
        Span::styled(
            truncate_str(&task_desc, 40),
            Style::default().fg(t.text_primary),
        ),
        Span::styled(
            format!("  [{}]  ", pos_label),
            Style::default().fg(t.text_muted),
        ),
        Span::styled("▶ next (→)", Style::default().fg(t.text_muted)),
    ]);

    f.render_widget(Paragraph::new(line).style(Style::default().bg(t.bg)), area);
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
    // `message_delta` event (sse.rs:212-218 → AppEvent::StreamUsage →
    // app.last_usage_output) — wire-truth, no estimation needed. OWUI /
    // OpenAI providers only emit usage at `message_stop`; for those the
    // wire value stays 0 mid-stream, so we fall back to chars/4 of the
    // streamed text + reasoning. The first non-zero wire value beats the
    // estimate; once the wire stops moving we keep the last known count.
    let estimate = (app.streaming_text.len() + app.streaming_reasoning.len()) as u64 / 4;
    let live_tokens = crate::spinner::live_token_count(app.last_usage_output as u64, estimate);
    let body = crate::spinner::format_status(app.spinner_frame, elapsed, live_tokens, stall);
    // Multi-agent fanout: when one or more background subagents are
    // running concurrently, append `· N agents…` to the spinner so the
    // user knows there's parallel work happening. Mirrors v126's
    // `3 agents…` indicator from cli.js (line 161622, task:background).
    let active_agents = app
        .background_tasks
        .values()
        .filter(|bt| matches!(bt.status, crate::types::TaskLifecycle::Running))
        .count();
    let mut spans: Vec<Span<'static>> =
        vec![Span::styled(body, Style::default().fg(t.text_secondary))];
    if active_agents > 0 {
        let plural = if active_agents == 1 {
            "agent"
        } else {
            "agents"
        };
        spans.push(Span::styled(
            format!("  ⏵ {active_agents} {plural}…"),
            Style::default().fg(t.accent),
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
        let (prefix, body) = if let Some(subj) = next_open_task_subject(app) {
            ("  □ Next: ".to_string(), subj)
        } else {
            (
                "  □ Tip: ".to_string(),
                crate::spinner::tip_for(elapsed).to_string(),
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
}

fn input(f: &mut Frame, app: &mut App, area: Rect) {
    let t = app.theme;
    // Border + title stay constant whether or not we're streaming. v126
    // never repaints the input bar mid-turn — the typing surface is the
    // user's surface, the spinner is a separate row above it.
    let border_style = Style::default().fg(t.border);
    let title = " message ".to_string();

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(border_style)
        .title(Span::styled(title, Style::default().fg(t.text_muted)))
        .style(Style::default().bg(t.surface));
    let inner = block.inner(area);
    f.render_widget(block, area);

    let content_width = inner.width.max(1) as usize;
    app.input_wrap_width = content_width;
    let (lines, cursor_row, cursor_col) = input_soft_wrapped_lines(app, content_width);
    let visible_rows = inner.height.max(1) as usize;
    let start = cursor_row.saturating_add(1).saturating_sub(visible_rows);
    let visible = lines
        .iter()
        .skip(start)
        .take(visible_rows)
        .map(|line| {
            Line::from(Span::styled(
                line.clone(),
                Style::default().fg(t.text_primary),
            ))
        })
        .collect::<Vec<_>>();

    f.render_widget(
        Paragraph::new(visible).style(Style::default().bg(t.surface)),
        inner,
    );

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
    let width = content_width.max(1);
    let logical_lines = app.textarea.lines();
    let (cursor_line, cursor_col) = app.textarea.cursor();
    let mut out = Vec::new();
    let mut visual_cursor_row = 0usize;
    let mut visual_cursor_col = 0usize;

    if logical_lines.iter().all(|line| line.is_empty()) {
        out.push("Type a message… (Enter to send, Shift+Enter for newline)".to_string());
        return (out, 0, 0);
    }

    for (line_idx, line) in logical_lines.iter().enumerate() {
        let wrapped = markdown::hard_wrap_str(line, width);
        if line_idx == cursor_line {
            visual_cursor_row = out.len() + cursor_col / width;
            visual_cursor_col = cursor_col % width;
        }
        out.extend(wrapped);
    }

    (out, visual_cursor_row, visual_cursor_col)
}

fn status(f: &mut Frame, app: &App, area: Rect) {
    let t = app.theme;

    // Two-row status: row 0 = info line (model, profile, cwd, hints),
    // row 1 = context-window LineGauge with color-coded usage.
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Length(1)])
        .split(area);

    let cwd_display = {
        let home = std::env::var("HOME").unwrap_or_default();
        app.cwd
            .strip_prefix(&home)
            .map(|rest| format!("~{rest}"))
            .unwrap_or_else(|| app.cwd.clone())
    };

    let msg_count = app.messages.iter().filter(|m| m.role == Role::User).count();

    // OAuth profile badge: subscription_type ("max"/"pro"/…) and seat_tier when set.
    // Only shown if the profile fetch succeeded — otherwise the bar stays terse.
    let profile_badge = match (&app.subscription_type, &app.seat_tier) {
        (Some(sub), Some(tier)) => format!("  {}·{}", sub, tier),
        (Some(sub), None) => format!("  {}", sub),
        (None, Some(tier)) => format!("  {}", tier),
        (None, None) => String::new(),
    };

    let auto_badge = if app.auto_mode.enabled {
        "  ⚡ auto".to_string()
    } else {
        String::new()
    };

    // v126 input queueing: show how many prompts the user has queued behind
    // the active stream. They render in the transcript with a `⏳` prefix and
    // get drained when the turn ends.
    let queue_badge = if !app.queued_prompts.is_empty() {
        format!("  ⏳ {} queued", app.queued_prompts.len())
    } else {
        String::new()
    };

    let leader_badge = if app.leader_key_active {
        "  [^X …]".to_string()
    } else if app.viewing_task_id.is_some() {
        "  [task view]".to_string()
    } else {
        String::new()
    };

    let left = format!(
        " {}{}{}{}{}  {}  {} msgs ",
        app.model, profile_badge, auto_badge, queue_badge, leader_badge, cwd_display, msg_count
    );
    // Single right-hand hint: the palette key. All other shortcuts are
    // discoverable inside the palette itself (Ctrl+P) — keeping just one
    // pointer here de-clutters the status row and matches v126's layout
    // where the bottom rail only points to the command index.
    let right = " Ctrl+P: palette ";

    let total_width = area.width as usize;
    let right_start = total_width.saturating_sub(right.len());
    let left_chars: usize = left.chars().count();
    let left_truncated = if left_chars > right_start.saturating_sub(1) {
        let truncated: String = left.chars().take(right_start.saturating_sub(2)).collect();
        format!("{truncated}…")
    } else {
        left
    };

    let padding = " ".repeat(right_start.saturating_sub(left_truncated.chars().count()));

    let line = Line::from(vec![
        Span::styled(left_truncated, Style::default().fg(t.text_secondary)),
        Span::styled(padding, Style::default().fg(t.text_muted)),
        Span::styled(right, Style::default().fg(t.text_muted)),
    ]);

    f.render_widget(
        Paragraph::new(line).style(Style::default().bg(t.surface)),
        rows[0],
    );

    // Context-window gauge: live ratio of `tool_ctx.approx_tokens` to
    // `max_context_tokens`. Color thresholds match the user's mental model of
    // "safe / watch / about to compact": green <60%, yellow 60–85%, red >85%.
    let used = app.tool_ctx.approx_tokens;
    let max = app.max_context_tokens.max(1);
    let ratio = (used as f64 / max as f64).clamp(0.0, 1.0);
    let pct = (ratio * 100.0).round() as u32;
    let bar_color = if pct < 60 {
        t.success
    } else if pct < 85 {
        t.warning
    } else {
        t.error
    };
    let label = context_gauge_label(used, max, pct);
    let gauge = LineGauge::default()
        .filled_style(Style::default().fg(bar_color))
        .unfilled_style(Style::default().fg(t.border))
        .label(Span::styled(label, Style::default().fg(t.text_secondary)))
        .ratio(ratio);
    f.render_widget(gauge, rows[1]);
}

fn context_gauge_label(used: usize, max: usize, pct: u32) -> String {
    format!(" ctx {}k / {}k · {}% ", used / 1000, max / 1000, pct)
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
    let w = MAX_W.min(frame_area.width.saturating_sub(2));
    let count = app.toasts.len() as u16;
    let h = count.min(5); // MAX_TOASTS, but bound to layout
    if h == 0 {
        return;
    }
    let area = Rect {
        x: frame_area.x + frame_area.width.saturating_sub(w + 1),
        y: frame_area.y + 1,
        width: w,
        height: h,
    };
    f.render_widget(Clear, area);
    let mut lines: Vec<Line> = Vec::new();
    for toast in app.toasts.iter().rev().take(h as usize).collect::<Vec<_>>() {
        let (icon, color) = match toast.kind {
            ToastKind::Info => ("ℹ", t.text_secondary),
            ToastKind::Success => ("✓", t.success),
            ToastKind::Warning => ("⚠", t.warning),
            ToastKind::Error => ("✘", t.error),
        };
        let max_text = (w as usize).saturating_sub(4);
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
        area,
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
    let title = format!(
        " Diagnostics — {issues} {} in {files} {} (Esc to close) ",
        if issues == 1 { "issue" } else { "issues" },
        if files == 1 { "file" } else { "files" },
    );
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(ratatui::widgets::BorderType::Rounded)
        .border_style(Style::default().fg(t.error))
        .title(Span::styled(
            title,
            Style::default().fg(t.error).add_modifier(Modifier::BOLD),
        ))
        .style(Style::default().bg(t.surface));
    let inner = block.inner(rect);
    f.render_widget(block, rect);

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
            Style::default()
                .fg(t.text_primary)
                .add_modifier(Modifier::BOLD),
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
    f.render_widget(
        Paragraph::new(lines)
            .style(Style::default().bg(t.surface))
            .wrap(ratatui::widgets::Wrap { trim: false }),
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
        .border_style(Style::default().fg(t.accent))
        .title(Span::styled(title, Style::default().fg(t.accent)))
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
                Style::default().fg(t.text_primary).bg(t.accent)
            } else {
                Style::default().fg(t.text_secondary)
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

fn palette(f: &mut Frame, app: &App) {
    let t = app.theme;
    let area = f.area();
    let width = 50u16.min(area.width.saturating_sub(4));
    let height = 10u16.min(area.height.saturating_sub(4));
    let x = area.width.saturating_sub(width) / 2;
    let y = area.height.saturating_sub(height) / 2;
    let palette_area = Rect::new(x, y, width, height);

    f.render_widget(Clear, palette_area);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(t.accent))
        .title(Span::styled(
            " Command Palette ",
            Style::default().fg(t.accent),
        ))
        .style(Style::default().bg(t.surface));

    let inner = block.inner(palette_area);
    f.render_widget(block, palette_area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(1)])
        .split(inner);

    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("> ", Style::default().fg(t.accent)),
            Span::styled(
                app.palette_input.clone(),
                Style::default().fg(t.text_primary),
            ),
        ])),
        chunks[0],
    );

    let items: Vec<ListItem> = palette_items(app)
        .iter()
        .enumerate()
        .map(|(i, label)| {
            let style = if i == app.palette_selected {
                Style::default()
                    .fg(t.accent)
                    .add_modifier(Modifier::BOLD)
                    .bg(t.surface_raised)
            } else {
                Style::default().fg(t.text_primary)
            };
            ListItem::new(Line::from(Span::styled(*label, style)))
        })
        .collect();

    f.render_widget(
        List::new(items).style(Style::default().bg(t.surface)),
        chunks[1],
    );
}

/// Color-code each provider so the user can scan the picker by source at a glance.
/// Hardcoded for the providers jfc currently supports — extend when adding a new one.
fn provider_color(provider: &str) -> Color {
    match provider {
        "anthropic" | "anthropic-oauth" => Color::Rgb(204, 120, 50), // Anthropic orange
        "openwebui" => Color::Rgb(100, 180, 200),                    // teal
        _ => Color::Gray,
    }
}

/// Friendly name for the provider badge column. Kept short so it doesn't crowd ids.
fn provider_label(provider: &str) -> &'static str {
    match provider {
        "anthropic" => "API",
        "anthropic-oauth" => "OAuth",
        "openwebui" => "OpenWebUI",
        _ => "?",
    }
}

fn model_picker(f: &mut Frame, app: &mut App) {
    let t = app.theme;
    let area = f.area();
    // Fluid sizing: take up to 90% of the screen, capped at 130 cols / 28 rows.
    // The previous fixed 60×16 truncated long OpenWebUI names like "Anthropic -
    // Claude Haiku 4.5 ($$)" mid-cell.
    let width = (area.width * 9 / 10).min(130).max(60);
    let height = (area.height * 8 / 10).min(28).max(12);
    let x = area.width.saturating_sub(width) / 2;
    let y = area.height.saturating_sub(height) / 2;
    let picker_area = Rect::new(x, y, width, height);

    f.render_widget(Clear, picker_area);

    let total = app.model_picker_models.len();
    let visible = filtered_models(app);
    let title = if app.model_picker_filter.is_empty() {
        format!(" Select Model · {} models ", total)
    } else {
        format!(
            " Select Model · {}/{} matching '{}' ",
            visible.len(),
            total,
            app.model_picker_filter
        )
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(t.accent))
        .title(Span::styled(
            title,
            Style::default().fg(t.accent).add_modifier(Modifier::BOLD),
        ))
        .title_bottom(
            Line::from(vec![
                Span::styled(" ↑↓", Style::default().fg(t.text_muted)),
                Span::styled(" navigate ", Style::default().fg(t.text_secondary)),
                Span::styled("· ", Style::default().fg(t.text_muted)),
                Span::styled("Enter", Style::default().fg(t.text_muted)),
                Span::styled(" select ", Style::default().fg(t.text_secondary)),
                Span::styled("· ", Style::default().fg(t.text_muted)),
                Span::styled("Esc", Style::default().fg(t.text_muted)),
                Span::styled(" cancel ", Style::default().fg(t.text_secondary)),
                Span::styled("· ", Style::default().fg(t.text_muted)),
                Span::styled("type", Style::default().fg(t.text_muted)),
                Span::styled(" filter ", Style::default().fg(t.text_secondary)),
            ])
            .right_aligned(),
        )
        .style(Style::default().bg(t.surface));

    let inner = block.inner(picker_area);
    f.render_widget(block, picker_area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(2), Constraint::Min(1)])
        .split(inner);

    // Filter input row.
    let filter_line = if app.model_picker_filter.is_empty() {
        Line::from(vec![
            Span::styled("  ⌕ ", Style::default().fg(t.accent)),
            Span::styled(
                "type to filter…",
                Style::default()
                    .fg(t.text_muted)
                    .add_modifier(Modifier::ITALIC),
            ),
        ])
    } else {
        Line::from(vec![
            Span::styled("  ⌕ ", Style::default().fg(t.accent)),
            Span::styled(
                app.model_picker_filter.clone(),
                Style::default()
                    .fg(t.text_primary)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled("▏", Style::default().fg(t.accent)),
        ])
    };
    f.render_widget(Paragraph::new(filter_line), chunks[0]);

    // Build the table.
    let header_style = Style::default()
        .fg(t.text_muted)
        .add_modifier(Modifier::BOLD);
    let header = Row::new(vec![
        Cell::from("  "),
        Cell::from("Model").style(header_style),
        Cell::from("ID").style(header_style),
        Cell::from("Source").style(header_style),
    ])
    .height(1)
    .bottom_margin(0);

    let rows: Vec<Row> = visible
        .iter()
        .map(|m| {
            let is_current = m.id == app.model;
            let marker = if is_current { " ● " } else { "   " };
            let name_style = if is_current {
                Style::default().fg(t.accent).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(t.text_primary)
            };
            let badge_style = Style::default()
                .fg(provider_color(&m.provider))
                .add_modifier(Modifier::BOLD);
            Row::new(vec![
                Cell::from(Span::styled(marker, Style::default().fg(t.accent))),
                Cell::from(Span::styled(m.display_name.clone(), name_style)),
                Cell::from(Span::styled(
                    m.id.to_string(),
                    Style::default().fg(t.text_muted),
                )),
                Cell::from(Span::styled(
                    provider_label(&m.provider).to_string(),
                    badge_style,
                )),
            ])
        })
        .collect();

    // Column constraints: marker fixed, name+id share most of the width,
    // source badge fixed. ratatui's Table widget auto-truncates per cell.
    let widths = [
        Constraint::Length(3),
        Constraint::Percentage(45),
        Constraint::Percentage(45),
        Constraint::Length(10),
    ];

    let table = Table::new(rows, widths)
        .header(header)
        .column_spacing(1)
        .row_highlight_style(
            Style::default()
                .bg(t.surface_raised)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("▶ ")
        .style(Style::default().bg(t.surface));

    f.render_stateful_widget(table, chunks[1], &mut app.model_picker_state);
}

fn task_panel(f: &mut Frame, app: &mut App) {
    let t = app.theme;
    let area = f.area();

    let w = (area.width as f32 * 0.80).round() as u16;
    let h = (area.height as f32 * 0.70).round() as u16;
    let x = area.x + (area.width.saturating_sub(w)) / 2;
    let y = area.y + (area.height.saturating_sub(h)) / 2;
    let popup = Rect::new(x, y, w, h);

    f.render_widget(Clear, popup);

    let all_tasks = app.task_store.list(crate::tasks::DeletedFilter::Exclude);
    let counts = app.task_store.counts();

    let completed_ids: std::collections::HashSet<&str> = all_tasks
        .iter()
        .filter(|tk| tk.status == crate::tasks::TaskStatus::Completed)
        .map(|tk| tk.id.as_str())
        .collect();

    let title = format!(
        " Tasks · {} total ({} done, {} in progress, {} pending) ",
        counts.pending + counts.in_progress + counts.completed,
        counts.completed,
        counts.in_progress,
        counts.pending,
    );

    let header = Row::new(vec![
        Cell::from("ID").style(
            Style::default()
                .fg(t.text_muted)
                .add_modifier(Modifier::BOLD),
        ),
        Cell::from("Status").style(
            Style::default()
                .fg(t.text_muted)
                .add_modifier(Modifier::BOLD),
        ),
        Cell::from("Subject").style(
            Style::default()
                .fg(t.text_muted)
                .add_modifier(Modifier::BOLD),
        ),
        Cell::from("Owner").style(
            Style::default()
                .fg(t.text_muted)
                .add_modifier(Modifier::BOLD),
        ),
        Cell::from("Blocked By").style(
            Style::default()
                .fg(t.text_muted)
                .add_modifier(Modifier::BOLD),
        ),
    ]);

    let rows: Vec<Row> = all_tasks
        .iter()
        .map(|tk| {
            let (icon, status_style) = match tk.status {
                crate::tasks::TaskStatus::Pending => {
                    ("□ pending", Style::default().fg(t.text_muted))
                }
                crate::tasks::TaskStatus::InProgress => (
                    "▣ in_progress",
                    Style::default().fg(t.accent).add_modifier(Modifier::BOLD),
                ),
                crate::tasks::TaskStatus::Completed => (
                    "✓ completed",
                    Style::default()
                        .fg(t.success)
                        .add_modifier(Modifier::CROSSED_OUT),
                ),
                _ => ("✗ deleted", Style::default().fg(t.error)),
            };

            let subj_style = if tk.status == crate::tasks::TaskStatus::Completed {
                Style::default()
                    .fg(t.text_muted)
                    .add_modifier(Modifier::CROSSED_OUT)
            } else {
                Style::default().fg(t.text_primary)
            };

            let open_blockers: Vec<&str> = tk
                .blocked_by
                .iter()
                .filter(|id| !completed_ids.contains(id.as_str()))
                .map(|id| id.as_str())
                .collect();

            Row::new(vec![
                Cell::from(tk.id.to_string()).style(Style::default().fg(t.text_muted)),
                Cell::from(icon).style(status_style),
                Cell::from(tk.subject.clone()).style(subj_style),
                Cell::from(tk.owner.clone().unwrap_or_default())
                    .style(Style::default().fg(t.text_secondary)),
                Cell::from(open_blockers.join(", ")).style(Style::default().fg(t.text_muted)),
            ])
        })
        .collect();

    // Clamp selection to valid range.
    if !all_tasks.is_empty() {
        let max = all_tasks.len().saturating_sub(1);
        if app.task_panel_selected > max {
            app.task_panel_selected = max;
        }
        app.task_panel_state.select(Some(app.task_panel_selected));
    }

    let table = Table::new(
        rows,
        [
            Constraint::Length(10),
            Constraint::Length(15),
            Constraint::Min(20),
            Constraint::Length(14),
            Constraint::Length(18),
        ],
    )
    .header(header)
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(t.border))
            .title(Span::styled(
                title,
                Style::default().fg(t.accent).add_modifier(Modifier::BOLD),
            ))
            .title_bottom(Span::styled(
                " ↑↓ navigate · Esc close ",
                Style::default()
                    .fg(t.text_muted)
                    .add_modifier(Modifier::ITALIC),
            ))
            .style(Style::default().bg(t.surface)),
    )
    .row_highlight_style(
        Style::default()
            .bg(t.surface_raised)
            .add_modifier(Modifier::BOLD),
    )
    .highlight_symbol("▶ ")
    .style(Style::default().bg(t.surface));

    f.render_stateful_widget(table, popup, &mut app.task_panel_state);
}

fn approval(f: &mut Frame, app: &App) {
    let Some(ref pending) = app.pending_approval else {
        return;
    };
    let t = app.theme;
    let area = f.area();

    // Mutating-tool kinds get the wider modal with a diff preview pane below
    // the choices. Read-only tools (Bash, Glob/Grep with side effects deferred
    // to the bash kind) keep the compact original layout.
    let preview = build_diff_preview(&pending.tool);
    let has_preview = preview.is_some();

    let (width, height) = if has_preview {
        (
            (area.width * 8 / 10).min(110).max(70),
            (area.height * 7 / 10).min(28).max(14),
        )
    } else {
        (
            60u16.min(area.width.saturating_sub(4)),
            10u16.min(area.height.saturating_sub(4)),
        )
    };
    let x = area.width.saturating_sub(width) / 2;
    let y = area.height.saturating_sub(height) / 2;
    let dialog_area = Rect::new(x, y, width, height);

    f.render_widget(Clear, dialog_area);

    let tool_summary = format!(
        "{} {}",
        pending.tool.kind.label(),
        pending.tool.input.summary()
    );

    // Count the queue depth so the user knows there's more behind the current
    // approval. Without this, multi-tool turns silently waited on each modal.
    let queue_len = app.approval_queue.len();
    let title = if queue_len > 0 {
        format!(" Allow tool use? · 1 of {} ", queue_len + 1)
    } else {
        " Allow tool use? ".to_string()
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(t.warning))
        .title(Span::styled(
            title,
            Style::default().fg(t.warning).add_modifier(Modifier::BOLD),
        ))
        .style(Style::default().bg(t.surface));

    let inner = block.inner(dialog_area);
    f.render_widget(block, dialog_area);

    if has_preview {
        // Three rows: summary header (2), choice list (5–6), diff preview (rest).
        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(2),
                Constraint::Length(ApprovalChoice::ALL.len() as u16),
                Constraint::Min(3),
            ])
            .split(inner);

        let truncated: String = tool_summary
            .chars()
            .take((rows[0].width as usize).saturating_sub(2))
            .collect();
        f.render_widget(
            Paragraph::new(vec![
                Line::from(Span::styled(truncated, Style::default().fg(t.text_primary))),
                Line::from(""),
            ]),
            rows[0],
        );

        render_choice_list(f, app, pending, rows[1]);

        let preview_lines = preview.unwrap();
        let preview_block = Block::default()
            .borders(Borders::TOP)
            .border_style(Style::default().fg(t.border))
            .title(Span::styled(" preview ", Style::default().fg(t.text_muted)));
        let inner_preview = preview_block.inner(rows[2]);
        f.render_widget(preview_block, rows[2]);
        f.render_widget(
            Paragraph::new(preview_lines).style(Style::default().bg(t.surface)),
            inner_preview,
        );
    } else {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(2), Constraint::Min(1)])
            .split(inner);
        let truncated: String = tool_summary
            .chars()
            .take((width as usize).saturating_sub(4))
            .collect();
        f.render_widget(
            Paragraph::new(vec![
                Line::from(Span::styled(truncated, Style::default().fg(t.text_primary))),
                Line::from(""),
            ]),
            chunks[0],
        );
        render_choice_list(f, app, pending, chunks[1]);
    }
}

/// Produce a diff/content preview for the pending tool, when applicable. Returns
/// `None` for tools whose effects can't be summarized as a diff (Bash, Read).
fn build_diff_preview(tool: &ToolCall) -> Option<Vec<Line<'static>>> {
    let theme = Theme::dark();
    let t = &theme;
    match &tool.input {
        ToolInput::Edit {
            file_path,
            old_string,
            new_string,
            ..
        } => {
            let mut lines: Vec<Line<'static>> = Vec::new();
            lines.push(Line::from(Span::styled(
                format!(" {file_path}"),
                Style::default()
                    .fg(t.text_secondary)
                    .add_modifier(Modifier::BOLD),
            )));
            lines.push(Line::from(""));
            for (kind, txt) in [("- ", old_string), ("+ ", new_string)] {
                let color = if kind == "- " { t.error } else { t.success };
                for ln in txt.lines().take(20) {
                    lines.push(Line::from(vec![
                        Span::styled(kind.to_owned(), Style::default().fg(color)),
                        Span::styled(ln.to_owned(), Style::default().fg(color)),
                    ]));
                }
            }
            Some(lines)
        }
        ToolInput::Write { file_path, content } => {
            let mut lines: Vec<Line<'static>> = Vec::new();
            lines.push(Line::from(Span::styled(
                format!(" {file_path}  ({} bytes)", content.len()),
                Style::default()
                    .fg(t.text_secondary)
                    .add_modifier(Modifier::BOLD),
            )));
            lines.push(Line::from(""));
            for ln in content.lines().take(30) {
                lines.push(Line::from(Span::styled(
                    ln.to_owned(),
                    Style::default().fg(t.text_primary),
                )));
            }
            let total = content.lines().count();
            if total > 30 {
                lines.push(Line::from(Span::styled(
                    format!("… {} more lines", total - 30),
                    Style::default()
                        .fg(t.text_muted)
                        .add_modifier(Modifier::ITALIC),
                )));
            }
            Some(lines)
        }
        ToolInput::ApplyPatch { patch } => {
            let mut lines: Vec<Line<'static>> = Vec::new();
            for ln in patch.lines().take(40) {
                let color = match ln.chars().next() {
                    Some('+') if !ln.starts_with("+++") => t.success,
                    Some('-') if !ln.starts_with("---") => t.error,
                    Some('@') => t.accent,
                    _ => t.text_secondary,
                };
                lines.push(Line::from(Span::styled(
                    ln.to_owned(),
                    Style::default().fg(color),
                )));
            }
            let total = patch.lines().count();
            if total > 40 {
                lines.push(Line::from(Span::styled(
                    format!("… {} more diff lines", total - 40),
                    Style::default()
                        .fg(t.text_muted)
                        .add_modifier(Modifier::ITALIC),
                )));
            }
            Some(lines)
        }
        ToolInput::Bash { command, .. } => {
            // Bash gets a single-line "preview" so the user sees the exact
            // command that would run. Useful when the summary truncates.
            Some(vec![
                Line::from(Span::styled(
                    String::from("$ "),
                    Style::default().fg(t.accent),
                )),
                Line::from(Span::styled(
                    command.clone(),
                    Style::default().fg(t.text_primary),
                )),
            ])
        }
        _ => None,
    }
}

fn render_choice_list(
    f: &mut Frame,
    _app: &App,
    pending: &crate::app::PendingApproval,
    area: Rect,
) {
    let t = Theme::dark();
    let items: Vec<ListItem> = ApprovalChoice::ALL
        .iter()
        .enumerate()
        .map(|(i, choice)| {
            let style = if i == pending.selected {
                Style::default()
                    .fg(t.warning)
                    .add_modifier(Modifier::BOLD)
                    .bg(t.surface_raised)
            } else {
                Style::default().fg(t.text_primary)
            };
            ListItem::new(Line::from(Span::styled(choice.label(), style)))
        })
        .collect();
    f.render_widget(List::new(items).style(Style::default().bg(t.surface)), area);
}

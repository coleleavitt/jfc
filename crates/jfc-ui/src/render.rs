use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Position, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Clear, LineGauge, List, ListItem, Paragraph, Row, Table},
};

#[allow(unused_imports)]
use ratatui::style::Stylize as _;

use crate::app::{App, ApprovalChoice, SPINNER};
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

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(3),
            Constraint::Length(subagent_footer_height),
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
    input(f, app, chunks[2]);
    status(f, app, chunks[3]);

    if app.show_palette {
        palette(f, app);
    }

    if app.show_model_picker {
        model_picker(f, app);
    }

    if app.show_task_panel {
        task_panel(f, app);
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
    let total_lines = message_view_total_lines(app, inner_width);

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

fn input(f: &mut Frame, app: &mut App, area: Rect) {
    let t = app.theme;
    let border_style = if app.is_streaming {
        Style::default().fg(t.warning)
    } else {
        Style::default().fg(t.border)
    };
    let title = if app.is_streaming {
        format!(" {} streaming… ", SPINNER[app.spinner_frame])
    } else {
        " message ".to_string()
    };

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
    let right = " Ctrl+C: clear/quit  Ctrl+B: sessions  Ctrl+S: info  Ctrl+P: palette  Ctrl+M: models  Ctrl+O: thinking ";

    let total_width = area.width as usize;
    let right_start = total_width.saturating_sub(right.len());
    // Use char count, not byte length — `left` contains multi-byte chars
    // (⚡, ⏳) that would panic on byte-indexed slicing.
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

use ratatui::{
    Frame,
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
};

use crate::app::App;

/// Full-screen modal showing all background agents/teammates with detailed
/// status. Shown when `expanded_view == Teammates`. Mirrors Claude Code's
/// "Background tasks" dialog that categorizes agents by type and shows
/// elapsed time, token count, and current activity.
pub(super) fn teammates_panel(f: &mut Frame, app: &mut App) {
    let t = app.theme;
    let area = f.area();

    let w = (area.width as f32 * 0.80).round() as u16;
    let h = (area.height as f32 * 0.70).round() as u16;
    let x = area.x + (area.width.saturating_sub(w)) / 2;
    let y = area.y + (area.height.saturating_sub(h)) / 2;
    let popup = Rect::new(x, y, w, h);

    f.render_widget(Clear, popup);

    let alive: Vec<_> = app
        .engine
        .background_tasks
        .values()
        .filter(|bt| bt.status.is_alive())
        .collect();
    let terminal: Vec<_> = app
        .engine
        .background_tasks
        .values()
        .filter(|bt| bt.status.is_terminal())
        .collect();

    let title = format!(
        " Agents · {} running, {} completed ",
        alive.len(),
        terminal.len(),
    );

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(t.style_border)
        .title(Span::styled(title, t.style_accent_bold))
        .title_bottom(Span::styled(
            " ↑↓ navigate · Esc close · Ctrl+T cycle ",
            t.style_text_muted.add_modifier(Modifier::ITALIC),
        ))
        .style(Style::default().bg(t.surface));
    let inner = block.inner(popup);
    f.render_widget(block, popup);

    if app.engine.background_tasks.is_empty() && !app.engine.team_context.is_active() {
        let placeholder = Paragraph::new(vec![
            Line::from(""),
            Line::from(Span::styled(
                "  No agents running",
                Style::default()
                    .fg(t.text_muted)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(""),
            Line::from(Span::styled(
                "  Agents appear here when you fire subagents via the Task tool",
                Style::default().fg(t.text_muted),
            )),
            Line::from(Span::styled(
                "  or use /background to dispatch work.",
                Style::default().fg(t.text_muted),
            )),
        ])
        .style(Style::default().bg(t.surface));
        f.render_widget(placeholder, inner);
        return;
    }

    // Build lines for each agent
    let mut lines: Vec<Line> = Vec::new();
    let now = std::time::Instant::now();
    let render_width = inner.width as usize;

    // Sort: alive first (by start time), then terminal
    let mut all_tasks: Vec<_> = app.engine.background_tasks.values().collect();
    all_tasks.sort_by(|a, b| {
        let a_alive = a.status.is_alive();
        let b_alive = b.status.is_alive();
        b_alive
            .cmp(&a_alive)
            .then_with(|| a.started_at.cmp(&b.started_at))
    });

    for bt in &all_tasks {
        use jfc_core::TaskLifecycle;

        let elapsed_label =
            super::visual::format_elapsed_secs(now.duration_since(bt.started_at).as_secs());

        let total_tokens = bt
            .latest_input_tokens
            .saturating_add(bt.latest_cache_read_tokens)
            .saturating_add(bt.latest_cache_write_tokens)
            .saturating_add(bt.cumulative_output_tokens);
        let token_label = if total_tokens > 0 {
            format!(" · ↓ {} tok", super::format_token_count(total_tokens))
        } else {
            String::new()
        };

        let tools_label = if bt.tool_use_count > 0 {
            format!(
                " · {} tool{}",
                bt.tool_use_count,
                if bt.tool_use_count == 1 { "" } else { "s" }
            )
        } else {
            String::new()
        };

        let (icon, icon_style) = match bt.status {
            TaskLifecycle::Running => (
                "● ",
                Style::default().fg(t.accent).add_modifier(Modifier::BOLD),
            ),
            TaskLifecycle::Idle => ("○ ", Style::default().fg(t.text_muted)),
            TaskLifecycle::Completed => ("✓ ", Style::default().fg(t.success)),
            TaskLifecycle::Failed => ("✗ ", Style::default().fg(t.error)),
            _ => ("○ ", Style::default().fg(t.text_muted)),
        };

        let status_str = match bt.status {
            TaskLifecycle::Running => "running",
            TaskLifecycle::Idle => "idle",
            TaskLifecycle::Completed => "completed",
            TaskLifecycle::Failed => "failed",
            _ => "pending",
        };

        let right_side = format!("{status_str} · {elapsed_label}{token_label}{tools_label}");
        // Cell width, not codepoint count — `cell_width` is the mandated layout
        // metric (visual.rs); the agents fan renders the same BackgroundTasks
        // with it, so chars().count() here drifted one cell per wide glyph.
        let right_len = super::visual::cell_width(&right_side);
        let selected = app
            .viewing_task_id
            .as_deref()
            .map(|id| id == bt.task_id.as_str())
            .unwrap_or(false);
        let pointer = if selected { "▶ " } else { "  " };
        let desc_budget = render_width.saturating_sub(5 + right_len + 2);
        let desc = super::truncate_str(&bt.description, desc_budget);
        let pad_len = render_width.saturating_sub(5 + super::visual::cell_width(&desc) + right_len);
        let padding = " ".repeat(pad_len);

        let name_style = if bt.status.is_alive() {
            Style::default()
                .fg(t.text_primary)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(t.text_muted)
        };

        lines.push(Line::from(vec![
            Span::styled(pointer, Style::default().fg(t.accent)),
            Span::styled(icon, icon_style),
            Span::styled(desc, name_style),
            Span::styled(padding, Style::default()),
            Span::styled(right_side, Style::default().fg(t.text_muted)),
        ]));

        // Show last tool activity as a sub-line
        if let Some(ref tool) = bt.last_tool
            && lines.len() < inner.height as usize
        {
            let sub = format!("  › {tool}");
            let sub_trimmed = super::truncate_str(&sub, render_width.saturating_sub(2));
            lines.push(Line::from(Span::styled(
                sub_trimmed,
                Style::default().fg(t.text_muted),
            )));
        }

        if lines.len() >= inner.height as usize {
            break;
        }
    }

    // Team teammates section (if team is active)
    if app.engine.team_context.is_active() && !app.engine.team_context.teammates.is_empty() {
        if !lines.is_empty() && lines.len() < inner.height as usize {
            lines.push(Line::from(""));
        }
        if lines.len() < inner.height as usize {
            lines.push(Line::from(Span::styled(
                "Team:",
                Style::default()
                    .fg(t.text_secondary)
                    .add_modifier(Modifier::BOLD),
            )));
        }
        let mut teammates: Vec<_> = app.engine.team_context.teammates.values().collect();
        teammates.sort_by_key(|tm| &tm.name);
        for tm in &teammates {
            if lines.len() >= inner.height as usize {
                break;
            }
            let is_active = tm.abort_tx.is_some();
            let status = if is_active { "running" } else { "idle" };
            let icon = if is_active { "● " } else { "○ " };
            let style = if is_active {
                Style::default().fg(t.accent)
            } else {
                Style::default().fg(t.text_muted)
            };
            lines.push(Line::from(vec![
                Span::styled(icon, style),
                Span::styled(
                    tm.name.clone(),
                    if is_active {
                        Style::default()
                            .fg(t.text_primary)
                            .add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().fg(t.text_muted)
                    },
                ),
                Span::styled(format!("  {status}"), Style::default().fg(t.text_muted)),
            ]));
        }
    }

    let content = Paragraph::new(lines).style(Style::default().bg(t.surface));
    f.render_widget(content, inner);
}

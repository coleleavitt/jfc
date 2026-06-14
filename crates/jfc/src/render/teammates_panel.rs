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

    // Shared fleet ordering (agents.rs): active → live → fresh-fail → idle →
    // done → cancelled → stale-fail, started_at tie-break — so this modal and
    // the inline agents fan list the same roster in the same order.
    let mut all_tasks: Vec<_> = app.engine.background_tasks.values().collect();
    all_tasks.sort_by_key(|bt| super::roster::roster_sort_key(bt, app, now));

    for bt in &all_tasks {
        // ONE canonical roster row (render/roster.rs) — the same row format
        // the inline agents fan renders, so an agent reads identically in
        // both surfaces.
        lines.push(super::roster::roster_row(bt, app, render_width, now));

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

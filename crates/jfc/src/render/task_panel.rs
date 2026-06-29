use std::collections::HashSet;

use ratatui::{
    Frame,
    layout::{Constraint, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Clear, Paragraph, Row, Table},
};

use crate::app::App;
use jfc_session::{DeletedFilter, TaskStatus};

use super::task_panel_detail::{render_task_detail, task_model_badge};
use super::task_panel_order::{tree_order, tree_prefix};
use super::task_panel_widgets::{
    render_task_panel_widgets, split_widget_area, task_panel_widget_rows,
};

pub(super) fn task_panel(f: &mut Frame, app: &mut App) {
    let t = app.theme;
    let area = f.area();

    let w = (area.width as f32 * 0.80).round() as u16;
    let h = (area.height as f32 * 0.70).round() as u16;
    let x = area.x + (area.width.saturating_sub(w)) / 2;
    let y = area.y + (area.height.saturating_sub(h)) / 2;
    let popup = Rect::new(x, y, w, h);

    f.render_widget(Clear, popup);
    let widget_rows = task_panel_widget_rows(app);
    let (content_area, widget_area) = split_widget_area(popup, widget_rows.len());

    let all_tasks = app.engine.task_store.list(DeletedFilter::Exclude);
    let counts = app.engine.task_store.counts();

    let completed_ids: HashSet<&str> = all_tasks
        .iter()
        .filter(|tk| tk.status == TaskStatus::Completed)
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
        Cell::from("Model").style(
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

    let ordered = tree_order(&all_tasks);

    let rows: Vec<Row> = ordered
        .iter()
        .map(|(tk, depth)| {
            let status_style = match tk.status {
                TaskStatus::Pending | TaskStatus::Queued => t.style_text_muted,
                TaskStatus::InProgress => t.style_accent_bold,
                TaskStatus::Blocked => t.style_accent,
                TaskStatus::Completed => t.style_success.add_modifier(Modifier::CROSSED_OUT),
                _ => t.style_error,
            };
            let icon = format!("{} {}", tk.status.glyph(), tk.status.label());

            let subj_style = if tk.status == TaskStatus::Completed {
                t.style_text_muted.add_modifier(Modifier::CROSSED_OUT)
            } else {
                t.style_text_primary
            };

            let open_blockers: Vec<&str> = tk
                .blocked_by
                .iter()
                .filter(|id| !completed_ids.contains(id.as_str()))
                .map(|id| id.as_str())
                .collect();

            let prefix = tree_prefix(*depth);
            let subject_display = format!("{prefix}{}", tk.subject);

            Row::new(vec![
                Cell::from(tk.id.to_string()).style(Style::default().fg(t.text_muted)),
                Cell::from(icon).style(status_style),
                Cell::from(subject_display).style(subj_style),
                Cell::from(tk.owner.clone().unwrap_or_default())
                    .style(Style::default().fg(t.text_secondary)),
                Cell::from(task_model_badge(tk).unwrap_or_default())
                    .style(Style::default().fg(t.text_muted)),
                Cell::from(open_blockers.join(", ")).style(Style::default().fg(t.text_muted)),
            ])
        })
        .collect();

    // Clamp selection to valid range.
    if !ordered.is_empty() {
        let max = ordered.len().saturating_sub(1);
        if app.task_panel.selected > max {
            app.task_panel.selected = max;
        }
        app.task_panel.table.select(Some(app.task_panel.selected));
    }

    // Empty state: show a useful hint instead of a header-only table
    // when no tasks exist. The model creates tasks via TaskCreate;
    // this tells the user that and gives them a slash command path.
    if all_tasks.is_empty() {
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(t.style_border)
            .title(Span::styled(title, t.style_accent_bold))
            .title_bottom(Span::styled(" Esc close ", t.style_text_muted))
            .style(Style::default().bg(t.surface));
        let inner = block.inner(content_area);
        f.render_widget(block, content_area);
        let placeholder = Paragraph::new(vec![
            Line::from(""),
            Line::from(Span::styled(
                "  No tasks yet",
                Style::default()
                    .fg(t.text_muted)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(""),
            Line::from(Span::styled(
                "  The model creates tasks via TaskCreate when planning",
                Style::default().fg(t.text_muted),
            )),
            Line::from(Span::styled(
                "  multi-step work. Ask it to break down a request and",
                Style::default().fg(t.text_muted),
            )),
            Line::from(Span::styled(
                "  the list will populate here.",
                Style::default().fg(t.text_muted),
            )),
        ])
        .style(Style::default().bg(t.surface));
        f.render_widget(placeholder, inner);
        if let Some(widget_area) = widget_area {
            render_task_panel_widgets(f, app, widget_area, &widget_rows);
        }
        return;
    }

    let table = Table::new(
        rows,
        [
            Constraint::Length(10),
            Constraint::Length(15),
            Constraint::Min(20),
            Constraint::Length(14),
            Constraint::Length(22),
            Constraint::Length(18),
        ],
    )
    .header(header)
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(t.style_border)
            .title(Span::styled(title, t.style_accent_bold))
            .title_bottom(Span::styled(
                " ↑↓ navigate · Enter detail · Esc close ",
                t.style_text_muted.add_modifier(Modifier::ITALIC),
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

    // When detail mode is active, split the popup vertically: top = table, bottom = detail.
    let ordered_tasks = ordered.iter().map(|(task, _)| *task).collect::<Vec<_>>();
    if app.task_panel.detail && !ordered_tasks.is_empty() {
        use ratatui::layout::{Direction, Layout};

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(content_area);

        f.render_stateful_widget(table, chunks[0], &mut app.task_panel.table);
        if let Some(task) = ordered_tasks.get(app.task_panel.selected) {
            render_task_detail(f, app, task, chunks[1]);
        }
    } else {
        f.render_stateful_widget(table, content_area, &mut app.task_panel.table);
    }
    if let Some(widget_area) = widget_area {
        render_task_panel_widgets(f, app, widget_area, &widget_rows);
    }
}

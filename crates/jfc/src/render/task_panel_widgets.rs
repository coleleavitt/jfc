use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
};

use crate::app::App;

pub(super) fn task_panel_widget_rows(app: &App) -> Vec<String> {
    super::status_widgets::task_panel_widget_rows(
        &app.plugins.ui_widget_descriptors,
        &app.plugins.ui_widget_snapshots,
        &app.plugins.ui_widget_refresh_status,
    )
}

pub(super) fn split_widget_area(popup: Rect, row_count: usize) -> (Rect, Option<Rect>) {
    if row_count == 0 || popup.height < 8 {
        return (popup, None);
    }
    let widget_height = (row_count as u16)
        .saturating_add(2)
        .min(popup.height.saturating_sub(5))
        .max(3);
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(5), Constraint::Length(widget_height)])
        .split(popup);
    (chunks[0], Some(chunks[1]))
}

pub(super) fn render_task_panel_widgets(f: &mut Frame, app: &App, area: Rect, rows: &[String]) {
    if rows.is_empty() {
        return;
    }
    let t = app.theme;
    let lines = rows
        .iter()
        .map(|row| Line::from(Span::styled(format!("  {row}"), t.style_text_muted)))
        .collect::<Vec<_>>();
    let block = Block::default()
        .borders(Borders::TOP)
        .border_style(t.style_border)
        .title(Span::styled(
            " Plugin widgets ",
            t.style_text_muted.add_modifier(Modifier::BOLD),
        ))
        .style(Style::default().bg(t.surface));
    let inner = block.inner(area);
    f.render_widget(block, area);
    f.render_widget(
        Paragraph::new(lines).style(Style::default().bg(t.surface)),
        inner,
    );
}

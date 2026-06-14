//! Centered popup for switching sessions — same shape as `model_picker` so
//! the muscle memory transfers (Ctrl+P open / ↑↓ navigate / Enter select /
//! Esc cancel / type to filter). Replaces the "Ctrl+B opens a left sidebar"
//! flow for one-shot session selection while leaving the sidebar in place
//! for browse-and-stay use.

use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Clear, Paragraph, Row, Table},
};

use crate::app::App;

/// Sessions whose `display_title()` (case-insensitive) contains the
/// current filter substring. Returns metadata refs in the same order as
/// `app.session_meta` (which the loader sorts newest-first), so the
/// freshest match always lands at the top.
pub fn filtered_sessions(app: &App) -> Vec<&jfc_session::SessionMetadata> {
    let filter = app.session_picker_filter.to_ascii_lowercase();
    let filter = filter.trim();
    app.session_meta
        .iter()
        .filter(|m| {
            if filter.is_empty() {
                return true;
            }
            let title = m.display_title().to_ascii_lowercase();
            if title.contains(filter) {
                return true;
            }
            m.cwd
                .as_deref()
                .map(|c| c.to_ascii_lowercase().contains(filter))
                .unwrap_or(false)
        })
        .collect()
}

pub(super) fn session_picker(f: &mut Frame, app: &mut App) {
    let t = app.theme;
    let area = f.area();
    let width = (area.width * 9 / 10).clamp(60, 130);
    let height = (area.height * 8 / 10).clamp(12, 28);
    let x = area.width.saturating_sub(width) / 2;
    let y = area.height.saturating_sub(height) / 2;
    let picker_area = Rect::new(x, y, width, height);

    f.render_widget(Clear, picker_area);

    let total = app.session_meta.len();
    let visible = filtered_sessions(app);
    let title = if app.session_picker_filter.is_empty() {
        format!(" Switch Session · {total} sessions ")
    } else {
        format!(
            " Switch Session · {}/{} matching '{}' ",
            visible.len(),
            total,
            app.session_picker_filter
        )
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(t.style_accent)
        .title(Span::styled(title, t.style_accent_bold))
        .title_bottom(
            Line::from(vec![
                Span::styled(" ↑↓", Style::default().fg(t.text_muted)),
                Span::styled(" navigate ", Style::default().fg(t.text_secondary)),
                Span::styled("· ", Style::default().fg(t.text_muted)),
                Span::styled("Enter", Style::default().fg(t.text_muted)),
                Span::styled(" load ", Style::default().fg(t.text_secondary)),
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

    let filter_line = if app.session_picker_filter.is_empty() {
        Line::from(vec![
            Span::styled("  ⌕ ", Style::default().fg(t.accent)),
            Span::styled(
                "type to filter by title or cwd…",
                Style::default()
                    .fg(t.text_muted)
                    .add_modifier(Modifier::ITALIC),
            ),
        ])
    } else {
        Line::from(vec![
            Span::styled("  ⌕ ", Style::default().fg(t.accent)),
            Span::styled(
                app.session_picker_filter.clone(),
                Style::default()
                    .fg(t.text_primary)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled("▏", Style::default().fg(t.accent)),
        ])
    };
    f.render_widget(Paragraph::new(filter_line), chunks[0]);

    let header_style = t.style_text_muted.add_modifier(Modifier::BOLD);
    let header = Row::new(vec![
        Cell::from("  "),
        Cell::from("Title").style(header_style),
        Cell::from("cwd").style(header_style),
        Cell::from("Activity").style(header_style),
        Cell::from("Msgs").style(header_style),
    ])
    .height(1)
    .bottom_margin(0);

    let now = chrono::Utc::now();
    let current_id = app.engine.current_session_id.clone();
    let rows: Vec<Row> = visible
        .iter()
        .map(|m| {
            let is_current = current_id.as_ref() == Some(&m.id);
            let marker = if is_current { " ● " } else { "   " };
            let title_style = if is_current {
                t.style_accent_bold
            } else {
                t.style_text_primary
            };
            let cwd_label = jfc_session::shorten_cwd(m.cwd.as_deref());
            let when = jfc_session::relative_time(m.last_activity(), now);
            let msgs = format!("{}", m.message_count);
            Row::new(vec![
                Cell::from(Span::styled(marker, Style::default().fg(t.accent))),
                Cell::from(Span::styled(m.display_title(), title_style)),
                Cell::from(Span::styled(
                    cwd_label,
                    Style::default().fg(t.text_secondary),
                )),
                Cell::from(Span::styled(when, Style::default().fg(t.text_muted))),
                Cell::from(Span::styled(msgs, Style::default().fg(t.text_secondary))),
            ])
        })
        .collect();

    let widths = [
        Constraint::Length(3),
        Constraint::Min(20),
        Constraint::Min(20),
        Constraint::Length(10),
        Constraint::Length(6),
    ];

    let table = Table::new(rows, widths)
        .header(header)
        .column_spacing(1)
        .row_highlight_style(
            Style::default()
                .fg(t.bg)
                .bg(t.accent)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("▶ ")
        .style(Style::default().bg(t.surface));

    f.render_stateful_widget(table, chunks[1], &mut app.session_picker_state);
}

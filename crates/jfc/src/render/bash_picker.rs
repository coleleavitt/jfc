//! Centered popup listing background shells — the `/bashes` roster as a modal,
//! same shape as `session_picker` so the muscle memory transfers (↑↓ navigate,
//! Esc close). `x`/`d` cancels the selected running shell. Opened from the
//! Ctrl+X leader chord then `b`.

use ratatui::{
    Frame,
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Clear, Row, Table},
};

use crate::app::App;

pub(super) fn bash_picker(f: &mut Frame, app: &mut App) {
    let t = app.theme;
    let area = f.area();
    let width = (area.width * 9 / 10).clamp(60, 130);
    let height = (area.height * 8 / 10).clamp(10, 26);
    let x = area.width.saturating_sub(width) / 2;
    let y = area.height.saturating_sub(height) / 2;
    let picker_area = Rect::new(x, y, width, height);

    f.render_widget(Clear, picker_area);

    let total = app.bash_picker.tasks.len();
    let running = app.bash_picker.tasks.iter().filter(|s| s.running).count();
    let title = format!(" Background shells · {running} running / {total} total ");

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(t.style_accent)
        .title(Span::styled(title, t.style_accent_bold))
        .title_bottom(
            Line::from(vec![
                Span::styled(" ↑↓", Style::default().fg(t.text_muted)),
                Span::styled(" navigate ", Style::default().fg(t.text_secondary)),
                Span::styled("· ", Style::default().fg(t.text_muted)),
                Span::styled("x", Style::default().fg(t.text_muted)),
                Span::styled(" cancel ", Style::default().fg(t.text_secondary)),
                Span::styled("· ", Style::default().fg(t.text_muted)),
                Span::styled("Esc", Style::default().fg(t.text_muted)),
                Span::styled(" close ", Style::default().fg(t.text_secondary)),
            ])
            .right_aligned(),
        )
        .style(Style::default().bg(t.surface));

    let inner = block.inner(picker_area);
    f.render_widget(block, picker_area);

    if total == 0 {
        let empty = ratatui::widgets::Paragraph::new(Line::from(Span::styled(
            "  No background shells. Run a command with run_in_background, or Ctrl+B to detach a running one.",
            Style::default()
                .fg(t.text_muted)
                .add_modifier(Modifier::ITALIC),
        )))
        .style(Style::default().bg(t.surface));
        f.render_widget(empty, inner);
        return;
    }

    let table = shells_table(app);
    f.render_stateful_widget(table, inner, &mut app.bash_picker.table);
}

/// Build the shells table (header + one row per tracked background shell).
/// Extracted from `bash_picker` to keep the render entrypoint small.
fn shells_table(app: &App) -> Table<'static> {
    let t = app.theme;
    let header_style = t.style_text_muted.add_modifier(Modifier::BOLD);
    let header = Row::new(vec![
        Cell::from(" "),
        Cell::from("Status").style(header_style),
        Cell::from("Task").style(header_style),
        Cell::from("Lines").style(header_style),
        Cell::from("Command").style(header_style),
    ])
    .height(1);

    let rows: Vec<Row> = app.bash_picker.tasks.iter().map(shell_row).collect();

    let widths = [
        ratatui::layout::Constraint::Length(2),
        ratatui::layout::Constraint::Length(18),
        ratatui::layout::Constraint::Length(18),
        ratatui::layout::Constraint::Length(6),
        ratatui::layout::Constraint::Min(20),
    ];

    Table::new(rows, widths)
        .header(header)
        .column_spacing(1)
        .row_highlight_style(
            Style::default()
                .fg(t.bg)
                .bg(t.accent)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("▶ ")
        .style(Style::default().bg(t.surface))
}

fn shell_row(s: &jfc_engine::tools::BashTaskSnapshot) -> Row<'static> {
    // Theme colors are resolved at call sites that have `app`; here we keep it
    // self-contained by reading the global theme through the snapshot-agnostic
    // styling below (running → bold marker, settled → muted).
    let running_fg = if s.running {
        Style::default().add_modifier(Modifier::BOLD)
    } else {
        Style::default().add_modifier(Modifier::DIM)
    };
    let marker = if s.running { "▶" } else { "■" };
    let cmd_preview: String = s.command.chars().take(60).collect();
    Row::new(vec![
        Cell::from(Span::styled(marker, running_fg)),
        Cell::from(Span::styled(s.status.clone(), running_fg)),
        Cell::from(Span::raw(s.id.clone())),
        Cell::from(Span::raw(s.total_lines.to_string())),
        Cell::from(Span::raw(cmd_preview)),
    ])
}

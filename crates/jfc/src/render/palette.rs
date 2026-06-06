use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::Style,
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, Paragraph},
};

use crate::app::App;
use crate::input::palette_items;

pub(super) fn palette(f: &mut Frame, app: &App) {
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
        .border_style(t.style_accent)
        .title(Span::styled(" Command Palette ", t.style_accent))
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
                t.style_accent_bold.bg(t.surface_raised)
            } else {
                t.style_text_primary
            };
            ListItem::new(Line::from(Span::styled(*label, style)))
        })
        .collect();

    f.render_widget(
        List::new(items).style(Style::default().bg(t.surface)),
        chunks[1],
    );
}

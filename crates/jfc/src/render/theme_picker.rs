use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Padding, Paragraph, Wrap},
};

use crate::app::App;
use crate::input::filtered_theme_choices;

pub(super) fn theme_picker(f: &mut Frame, app: &mut App) {
    let t = app.theme;
    let screen = f.area();
    let width = 68u16.min(screen.width.saturating_sub(4));
    let height = 18u16.min(screen.height.saturating_sub(4));
    let area = Rect::new(
        screen.width.saturating_sub(width) / 2,
        screen.height.saturating_sub(height) / 2,
        width,
        height,
    );
    let block = Block::default()
        .title(" Theme Picker ")
        .borders(Borders::ALL)
        .border_style(t.style_accent)
        .padding(Padding::new(1, 1, 1, 1));
    f.render_widget(Clear, area);
    let inner = block.inner(area);
    f.render_widget(block, area);
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2),
            Constraint::Min(6),
            Constraint::Length(4),
        ])
        .split(inner);

    let query = if app.theme_picker.input.is_empty() {
        "type to filter themes".to_string()
    } else {
        app.theme_picker.input.clone()
    };
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("filter: ", Style::default().fg(t.text_muted)),
            Span::styled(
                query,
                Style::default()
                    .fg(t.text_primary)
                    .add_modifier(Modifier::BOLD),
            ),
        ])),
        chunks[0],
    );

    let filtered = filtered_theme_choices(app);
    let visible_rows = chunks[1].height.max(1) as usize;
    let offset = app
        .theme_picker
        .selected
        .saturating_sub(visible_rows.saturating_sub(1));
    let rows: Vec<Line<'static>> = filtered
        .iter()
        .enumerate()
        .skip(offset)
        .take(visible_rows)
        .map(|(idx, choice)| {
            let selected = idx == app.theme_picker.selected;
            let marker = if selected { "▶" } else { " " };
            let sample_theme = crate::theme::Theme::by_name(choice.name).unwrap_or(t);
            Line::from(vec![
                Span::styled(marker, Style::default().fg(t.accent)),
                Span::raw(" "),
                Span::styled("██", Style::default().fg(sample_theme.text_primary)),
                Span::styled("██", Style::default().fg(sample_theme.text_secondary)),
                Span::styled("██", Style::default().fg(sample_theme.accent)),
                Span::raw("  "),
                Span::styled(
                    choice.label,
                    Style::default()
                        .fg(if selected { t.accent } else { t.text_primary })
                        .add_modifier(if selected {
                            Modifier::BOLD
                        } else {
                            Modifier::empty()
                        }),
                ),
                Span::styled(
                    format!("  /theme {}", choice.name),
                    Style::default().fg(t.text_muted),
                ),
            ])
        })
        .collect();

    f.render_widget(Paragraph::new(rows), chunks[1]);

    let description = filtered
        .get(app.theme_picker.selected)
        .map(|choice| choice.description)
        .unwrap_or("No themes match the current filter.");
    f.render_widget(
        Paragraph::new(vec![
            Line::from(description),
            Line::from(vec![
                Span::styled(
                    "Enter",
                    Style::default().fg(t.accent).add_modifier(Modifier::BOLD),
                ),
                Span::raw(" apply  "),
                Span::styled(
                    "Esc",
                    Style::default().fg(t.accent).add_modifier(Modifier::BOLD),
                ),
                Span::raw(" close  "),
                Span::styled(
                    "↑/↓",
                    Style::default().fg(t.accent).add_modifier(Modifier::BOLD),
                ),
                Span::raw("/"),
                Span::styled(
                    "j/k",
                    Style::default().fg(t.accent).add_modifier(Modifier::BOLD),
                ),
                Span::raw(" move"),
            ]),
        ])
        .wrap(Wrap { trim: true })
        .style(t.style_text_muted),
        chunks[2],
    );
}

use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Clear, Paragraph, Row, Table},
};

use crate::app::App;
use crate::input::filtered_models;

/// Color-code each provider so the user can scan the picker by source at a glance.
pub(super) fn provider_color(provider: &str) -> Color {
    match provider {
        "anthropic" | "anthropic-oauth" => Color::Rgb(204, 120, 50), // Anthropic orange
        "openai" | "codex" => Color::Rgb(116, 170, 156),             // OpenAI green
        "gemini" => Color::Rgb(66, 133, 244),                        // Google blue
        "antigravity" => Color::Rgb(52, 168, 83),                    // Antigravity green
        "vertex" => Color::Rgb(234, 67, 53),                         // GCP red
        "bedrock" => Color::Rgb(255, 153, 0),                        // AWS orange
        "litellm" => Color::Rgb(168, 85, 247),                       // purple
        "openwebui" => Color::Rgb(100, 180, 200),                    // teal
        _ => Color::Gray,
    }
}

/// Friendly name for the provider badge column. Kept short so it doesn't crowd ids.
pub(super) fn provider_label(provider: &str) -> &'static str {
    match provider {
        "anthropic" => "API",
        "anthropic-oauth" => "OAuth",
        "openai" => "OpenAI",
        "codex" => "Codex",
        "gemini" => "Gemini",
        "antigravity" => "AI Pro",
        "vertex" => "Vertex",
        "bedrock" => "Bedrock",
        "litellm" => "LiteLLM",
        "openwebui" => "OpenWebUI",
        _ => "?",
    }
}

pub(super) fn model_picker(f: &mut Frame, app: &mut App) {
    let t = app.theme;
    let area = f.area();
    // Fluid sizing: take up to 90% of the screen, capped at 130 cols / 28 rows.
    // The previous fixed 60x16 truncated long OpenWebUI names like "Anthropic -
    // Claude Haiku 4.5 ($$)" mid-cell.
    let width = (area.width * 9 / 10).clamp(60, 130);
    let height = (area.height * 8 / 10).clamp(12, 28);
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
        .border_style(t.style_accent)
        .title(Span::styled(title, t.style_accent_bold))
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

    let header_style = t.style_text_muted.add_modifier(Modifier::BOLD);
    let header = Row::new(vec![
        Cell::from("  "),
        Cell::from("Model").style(header_style),
        Cell::from("Ctx").style(header_style),
        Cell::from("In $/M").style(header_style),
        Cell::from("Out $/M").style(header_style),
        Cell::from("Source").style(header_style),
    ])
    .height(1)
    .bottom_margin(0);

    let rows: Vec<Row> = visible
        .iter()
        .map(|m| {
            let is_current = m.id == app.engine.model;
            let marker = if is_current { " ● " } else { "   " };
            let name_style = if is_current {
                t.style_accent_bold
            } else {
                t.style_text_primary
            };
            let badge_style = Style::default()
                .fg(provider_color(&m.provider))
                .add_modifier(Modifier::BOLD);
            let ctx_str = m
                .context_window_tokens
                .map(|n| {
                    if n >= 1_000_000 {
                        format!("{}M", n / 1_000_000)
                    } else {
                        format!("{}k", n / 1000)
                    }
                })
                .unwrap_or_else(|| "—".into());
            let in_cost = m
                .input_cost
                .map(|c| format!("${:.2}", c))
                .unwrap_or_else(|| "—".into());
            let out_cost = m
                .output_cost
                .map(|c| format!("${:.2}", c))
                .unwrap_or_else(|| "—".into());
            Row::new(vec![
                Cell::from(Span::styled(marker, Style::default().fg(t.accent))),
                Cell::from(Span::styled(m.display_name.clone(), name_style)),
                Cell::from(Span::styled(ctx_str, Style::default().fg(t.text_secondary))),
                Cell::from(Span::styled(in_cost, Style::default().fg(t.text_secondary))),
                Cell::from(Span::styled(
                    out_cost,
                    Style::default().fg(t.text_secondary),
                )),
                Cell::from(Span::styled(
                    provider_label(&m.provider).to_string(),
                    badge_style,
                )),
            ])
        })
        .collect();

    let widths = [
        Constraint::Length(3),
        Constraint::Min(20),
        Constraint::Length(6),
        Constraint::Length(7),
        Constraint::Length(8),
        Constraint::Length(10),
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

    f.render_stateful_widget(table, chunks[1], &mut app.model_picker_state);
}

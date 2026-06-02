//! Renders the `AskUserQuestion` modal — the interactive multiple-choice
//! dialog driven by `app.pending_question` (see `input/question.rs`).
//!
//! Structurally a sibling of `render/approval.rs`: a centered, cleared dialog
//! with a header, a keyboard-navigable choice list (single- or multi-select),
//! an auto-injected "Other" free-text row, and — for a focused single-select
//! option that carries a `preview` — a side-by-side preview panel.

use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Wrap},
};

use crate::app::{App, PendingQuestion};
use crate::theme::Theme;

pub(super) fn question(f: &mut Frame, app: &App) {
    let Some(pending) = app.pending_question.as_ref() else {
        return;
    };
    let t = app.theme;
    let area = f.area();

    // Preview only applies to single-select (matches the contract), and only
    // for the currently-focused option.
    let focused_preview: Option<&str> = if pending.multi_select {
        None
    } else {
        pending
            .options
            .get(pending.selected)
            .and_then(|o| o.preview.as_deref())
    };
    let has_preview = focused_preview.is_some();

    let (width, height) = if has_preview {
        (
            (area.width * 8 / 10).clamp(70, 110),
            (area.height * 7 / 10).clamp(14, 30),
        )
    } else {
        (
            (area.width * 7 / 10).clamp(48, 90).min(area.width.saturating_sub(4)),
            (area.height * 6 / 10).clamp(10, 24).min(area.height.saturating_sub(4)),
        )
    };
    let x = area.width.saturating_sub(width) / 2;
    let y = area.height.saturating_sub(height) / 2;
    let dialog_area = Rect::new(x, y, width, height);
    f.render_widget(Clear, dialog_area);

    let accent = t.accent;
    let title = if pending.header.is_empty() {
        " Question ".to_string()
    } else {
        format!(" {} ", pending.header)
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(accent))
        .title(Span::styled(
            title,
            Style::default().fg(accent).add_modifier(Modifier::BOLD),
        ))
        .style(Style::default().bg(t.surface));
    let inner = block.inner(dialog_area);
    f.render_widget(block, dialog_area);

    // Question prose (top, wrapped, ≤4 rows), body (options [+ preview]),
    // footer hint (bottom).
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(question_height(&pending.question, inner.width)),
            Constraint::Min(3),
            Constraint::Length(1),
        ])
        .split(inner);

    f.render_widget(
        Paragraph::new(Line::from(Span::styled(
            pending.question.clone(),
            t.style_text_primary.add_modifier(Modifier::BOLD),
        )))
        .wrap(Wrap { trim: true }),
        rows[0],
    );

    if let Some(preview) = focused_preview {
        let cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(45), Constraint::Percentage(55)])
            .split(rows[1]);
        render_options(f, pending, cols[0], &t);
        render_preview(f, preview, cols[1], &t);
    } else {
        render_options(f, pending, rows[1], &t);
    }

    f.render_widget(
        Paragraph::new(footer_hint(pending, &t)),
        rows[2],
    );
}

/// Wrapped row count for the question prose, clamped to [1, 4].
fn question_height(question: &str, width: u16) -> u16 {
    if width == 0 {
        return 1;
    }
    let rows = Paragraph::new(question)
        .wrap(Wrap { trim: true })
        .line_count(width)
        .max(1) as u16;
    rows.clamp(1, 4)
}

fn render_options(f: &mut Frame, pending: &PendingQuestion, area: Rect, t: &Theme) {
    let mut items: Vec<ListItem> = Vec::with_capacity(pending.options.len() + 1);
    let highlight = Style::default()
        .fg(t.bg)
        .bg(t.accent)
        .add_modifier(Modifier::BOLD);

    for (i, opt) in pending.options.iter().enumerate() {
        let focused = i == pending.selected && !pending.editing_other;
        let marker = if pending.multi_select {
            if pending.chosen.contains(&i) {
                "[x] "
            } else {
                "[ ] "
            }
        } else if focused {
            "▶ "
        } else {
            "  "
        };
        let mut spans = vec![Span::styled(
            format!("{marker}{}", opt.label),
            if focused {
                highlight
            } else {
                t.style_text_primary
            },
        )];
        if !opt.description.is_empty() {
            spans.push(Span::styled(
                format!("  — {}", opt.description),
                t.style_text_muted,
            ));
        }
        items.push(ListItem::new(Line::from(spans)));
    }

    // Auto-injected "Other" free-text row.
    let other_idx = pending.other_row();
    let focused_other = pending.selected == other_idx;
    let marker = if pending.multi_select {
        if pending.chosen.contains(&other_idx) {
            "[x] "
        } else {
            "[ ] "
        }
    } else if focused_other {
        "▶ "
    } else {
        "  "
    };
    let other_text = if pending.editing_other {
        format!("Other: {}▏", pending.other_text)
    } else if pending.other_text.trim().is_empty() {
        "Other (type your own)…".to_owned()
    } else {
        format!("Other: {}", pending.other_text)
    };
    let other_style = if pending.editing_other {
        t.style_text_primary
            .fg(t.accent)
            .add_modifier(Modifier::BOLD)
    } else if focused_other {
        highlight
    } else {
        t.style_text_secondary
    };
    items.push(ListItem::new(Line::from(Span::styled(
        format!("{marker}{other_text}"),
        other_style,
    ))));

    f.render_widget(
        List::new(items).style(Style::default().bg(t.surface)),
        area,
    );
}

fn render_preview(f: &mut Frame, preview: &str, area: Rect, t: &Theme) {
    let block = Block::default()
        .borders(Borders::LEFT)
        .border_style(t.style_border)
        .title(Span::styled(" preview ", t.style_text_muted));
    let inner = block.inner(area);
    f.render_widget(block, area);
    // Rendered as a plain monospace box (the TUI variant of the contract's
    // preview); no markdown styling, just faithful lines.
    let lines: Vec<Line<'static>> = preview
        .lines()
        .map(|l| Line::from(Span::styled(l.to_owned(), t.style_text_primary)))
        .collect();
    f.render_widget(
        Paragraph::new(lines).style(Style::default().bg(t.surface)),
        inner,
    );
}

fn footer_hint(pending: &PendingQuestion, t: &Theme) -> Line<'static> {
    let hint = if pending.editing_other {
        "type your answer · Enter confirm · Esc back"
    } else if pending.multi_select {
        "↑/↓ move · Space toggle · Enter submit · Esc cancel"
    } else {
        "↑/↓ move · Enter select · Esc cancel"
    };
    Line::from(Span::styled(hint.to_owned(), t.style_text_muted))
}

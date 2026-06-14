//! Renders the `AskUserQuestion` modal — the interactive multiple-choice
//! dialog driven by `app.engine.pending_question` (see `input/question.rs`).
//!
//! Structurally a sibling of `render/approval.rs`: a centered, cleared dialog
//! with (for multi-question prompts) a header-chip nav bar, a keyboard-navigable
//! choice list (single- or multi-select), an auto-injected "Other" free-text
//! row, and — for a focused single-select option that carries a `preview` — a
//! side-by-side preview panel.

use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Wrap},
};

use crate::app::{App, PendingQuestion, QuestionItem};
use crate::theme::Theme;

pub(super) fn question(f: &mut Frame, app: &App) {
    let Some(pending) = app.engine.pending_question.as_ref() else {
        return;
    };
    let t = app.theme;
    let area = f.area();
    let item = pending.cur();
    let multi_question = pending.items.len() > 1;

    // Preview only applies to single-select (matches the contract), and only
    // for the currently-focused option.
    let focused_preview: Option<&str> = if item.multi_select {
        None
    } else {
        item.options
            .get(item.selected)
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
            (area.width * 7 / 10)
                .clamp(48, 90)
                .min(area.width.saturating_sub(4)),
            (area.height * 6 / 10)
                .clamp(10, 24)
                .min(area.height.saturating_sub(4)),
        )
    };
    let x = area.width.saturating_sub(width) / 2;
    let y = area.height.saturating_sub(height) / 2;
    let dialog_area = Rect::new(x, y, width, height);
    f.render_widget(Clear, dialog_area);

    let accent = t.accent;
    let title = if item.header.is_empty() {
        " Question ".to_string()
    } else {
        format!(" {} ", item.header)
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

    // [nav bar (multi only)] · question prose · body (options [+ preview]) · footer.
    let nav_h = if multi_question { 1 } else { 0 };
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(nav_h),
            Constraint::Length(question_height(&item.question, inner.width)),
            Constraint::Min(3),
            Constraint::Length(1),
        ])
        .split(inner);

    if multi_question {
        f.render_widget(Paragraph::new(nav_bar(pending, &t)), rows[0]);
    }

    f.render_widget(
        Paragraph::new(Line::from(Span::styled(
            item.question.clone(),
            t.style_text_primary.add_modifier(Modifier::BOLD),
        )))
        .wrap(Wrap { trim: true }),
        rows[1],
    );

    if let Some(preview) = focused_preview {
        let cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(45), Constraint::Percentage(55)])
            .split(rows[2]);
        render_options(f, item, pending.editing_other, cols[0], &t);
        render_preview(f, preview, cols[1], &t);
    } else {
        render_options(f, item, pending.editing_other, rows[2], &t);
    }

    f.render_widget(Paragraph::new(footer_hint(pending, &t)), rows[3]);
}

/// Header-chip nav bar across the questions: current is highlighted, answered
/// questions get a ✓ prefix.
fn nav_bar(pending: &PendingQuestion, t: &Theme) -> Line<'static> {
    let mut spans: Vec<Span<'static>> = Vec::new();
    for (i, q) in pending.items.iter().enumerate() {
        let label = if q.header.is_empty() {
            format!("Q{}", i + 1)
        } else {
            q.header.clone()
        };
        let mark = if q.answer.is_some() { "✓ " } else { "" };
        let text = format!(" {mark}{label} ");
        let style = if i == pending.current {
            Style::default()
                .fg(t.bg)
                .bg(t.accent)
                .add_modifier(Modifier::BOLD)
        } else if q.answer.is_some() {
            t.style_text_secondary
        } else {
            t.style_text_muted
        };
        spans.push(Span::styled(text, style));
        spans.push(Span::raw(" "));
    }
    Line::from(spans)
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

fn render_options(f: &mut Frame, item: &QuestionItem, editing_other: bool, area: Rect, t: &Theme) {
    let mut items: Vec<ListItem> = Vec::with_capacity(item.options.len() + 1);
    let highlight = Style::default()
        .fg(t.bg)
        .bg(t.accent)
        .add_modifier(Modifier::BOLD);

    for (i, opt) in item.options.iter().enumerate() {
        let focused = i == item.selected && !editing_other;
        let marker = if item.multi_select {
            if item.chosen.contains(&i) {
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
    let other_idx = item.other_row();
    let focused_other = item.selected == other_idx;
    let marker = if item.multi_select {
        if item.chosen.contains(&other_idx) {
            "[x] "
        } else {
            "[ ] "
        }
    } else if focused_other {
        "▶ "
    } else {
        "  "
    };
    let other_text = if editing_other {
        format!("Other: {}▏", item.other_text)
    } else if item.other_text.trim().is_empty() {
        "Other (type your own)…".to_owned()
    } else {
        format!("Other: {}", item.other_text)
    };
    let other_style = if editing_other {
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

    f.render_widget(List::new(items).style(Style::default().bg(t.surface)), area);
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
    let multi_question = pending.items.len() > 1;
    let hint = if pending.editing_other {
        "type your answer · Enter confirm · Esc back".to_owned()
    } else {
        let mut parts = vec!["↑/↓ move"];
        if pending.cur().multi_select {
            parts.push("Space toggle");
            parts.push("Enter confirm");
        } else {
            parts.push("Enter select");
        }
        if multi_question {
            parts.push("←/→ switch");
        }
        parts.push("Esc cancel");
        parts.join(" · ")
    };
    Line::from(Span::styled(hint, t.style_text_muted))
}

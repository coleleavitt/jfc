//! Renders the prompt-rewrite proposal modal driven by
//! `app.pending_rewrite_proposal` (see `input/prompt_rewrite.rs`).
//!
//! A centered, cleared dialog showing the original prompt, the proposed rewrite,
//! and the rationale, with an accept/reject/edit footer. Surfacing the rewrite
//! (never applying it silently) is the SPEC "require confirmation" contract.

use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
};

use crate::app::App;

pub(super) fn prompt_rewrite(f: &mut Frame, app: &App) {
    let Some(proposal) = app.pending_rewrite_proposal.as_ref() else {
        return;
    };
    let t = app.theme;
    let area = f.area();

    let width = (area.width * 7 / 10)
        .clamp(48, 96)
        .min(area.width.saturating_sub(4));
    let height = (area.height * 6 / 10)
        .clamp(12, 26)
        .min(area.height.saturating_sub(4));
    let x = area.width.saturating_sub(width) / 2;
    let y = area.height.saturating_sub(height) / 2;
    let dialog_area = Rect::new(x, y, width, height);
    f.render_widget(Clear, dialog_area);

    let accent = t.accent;
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(accent))
        .title(Span::styled(
            " Reduce likely false refusal ",
            Style::default().fg(accent).add_modifier(Modifier::BOLD),
        ))
        .style(Style::default().bg(t.surface));
    let inner = block.inner(dialog_area);
    f.render_widget(block, dialog_area);

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(3),    // rewrite
            Constraint::Length(2), // rationale
            Constraint::Length(3), // original (collapsed)
            Constraint::Length(1), // footer
        ])
        .split(inner);

    let label = |s: &'static str| Span::styled(s, Style::default().fg(t.text_muted));

    let rewrite = Paragraph::new(vec![
        Line::from(label("Proposed rewrite:")),
        Line::from(Span::styled(
            proposal.rewrite.clone(),
            Style::default().fg(t.text_primary).add_modifier(Modifier::BOLD),
        )),
    ])
    .wrap(Wrap { trim: true });
    f.render_widget(rewrite, rows[0]);

    let rationale = Paragraph::new(Line::from(vec![
        label("Why: "),
        Span::styled(proposal.rationale.clone(), Style::default().fg(t.text_primary)),
    ]))
    .wrap(Wrap { trim: true });
    f.render_widget(rationale, rows[1]);

    let original = Paragraph::new(vec![
        Line::from(label("Your original:")),
        Line::from(Span::styled(
            proposal.original.clone(),
            Style::default().fg(t.text_muted),
        )),
    ])
    .wrap(Wrap { trim: true });
    f.render_widget(original, rows[2]);

    let footer = Paragraph::new(Line::from(vec![
        Span::styled("[A]", Style::default().fg(accent).add_modifier(Modifier::BOLD)),
        label("ccept  "),
        Span::styled("[R]", Style::default().fg(accent).add_modifier(Modifier::BOLD)),
        label("eject (send original)  "),
        Span::styled("[E]", Style::default().fg(accent).add_modifier(Modifier::BOLD)),
        label("dit"),
    ]));
    f.render_widget(footer, rows[3]);
}

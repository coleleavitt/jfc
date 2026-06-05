use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, Paragraph},
};

use crate::app::{App, ApprovalChoice, PendingApproval};
use crate::theme::Theme;
use crate::types::{ToolCall, ToolInput};

pub(super) fn approval(f: &mut Frame, app: &App) {
    let Some(ref pending) = app.pending_approval else {
        return;
    };
    let t = app.theme;
    let area = f.area();

    // Mutating-tool kinds get the wider modal with a diff preview pane below
    // the choices. Read-only tools (Bash, Glob/Grep with side effects deferred
    // to the bash kind) keep the compact original layout.
    let preview = build_diff_preview(&pending.tool, &t);
    let has_preview = preview.is_some();

    let (width, height) = if has_preview {
        (
            (area.width * 8 / 10).clamp(70, 110),
            (area.height * 7 / 10).clamp(14, 28),
        )
    } else {
        (
            60u16.min(area.width.saturating_sub(4)),
            10u16.min(area.height.saturating_sub(4)),
        )
    };
    let x = area.width.saturating_sub(width) / 2;
    let y = area.height.saturating_sub(height) / 2;
    let dialog_area = Rect::new(x, y, width, height);

    f.render_widget(Clear, dialog_area);

    let kind_color = crate::message_view::tool_kind_color(&pending.tool.kind, &t);
    let tool_label = pending.tool.kind.label();
    let tool_input_summary = pending.tool.input.summary();

    // Count the queue depth so the user knows there's more behind the current
    // approval. Without this, multi-tool turns silently waited on each modal.
    let queue_len = app.approval_queue.len();
    let title = if queue_len > 0 {
        format!(" Allow tool use? · 1 of {} ", queue_len + 1)
    } else {
        " Allow tool use? ".to_string()
    };

    // Check if the command is destructive and the gate is enabled.
    let is_destructive = is_tool_destructive(&pending.tool);

    // Border/title color is a severity signal: destructive commands (rm -rf,
    // force-push, …) get `error` red; ordinary approvals get `warning` gold.
    // A static color for both would dull the signal so the user stops reading
    // it on the genuinely dangerous ones.
    let accent = if is_destructive { t.error } else { t.warning };

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

    if has_preview {
        // Three rows: summary header (2+), choice list (5-6), diff preview (rest).
        // If destructive, we add an extra warning line above the choices.
        let header_height = if is_destructive { 4 } else { 2 };
        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(header_height),
                Constraint::Length(ApprovalChoice::ALL.len() as u16),
                Constraint::Min(3),
            ])
            .split(inner);

        // Tool name styled with its kind color; arguments truncated
        // to fit the dialog width. Splitting into two spans makes the
        // identity colored without bleeding into the args.
        let arg_cap = (rows[0].width as usize).saturating_sub(tool_label.chars().count() + 3);
        let arg_truncated: String = tool_input_summary.chars().take(arg_cap).collect();
        let mut header_lines = vec![
            Line::from(vec![
                Span::styled(
                    tool_label.to_string(),
                    Style::default().fg(kind_color).add_modifier(Modifier::BOLD),
                ),
                Span::raw(" "),
                Span::styled(arg_truncated, Style::default().fg(t.text_primary)),
            ]),
            Line::from(""),
        ];
        if is_destructive {
            header_lines.push(destructive_warning_line(&t));
            header_lines.push(Line::from(""));
        }
        f.render_widget(Paragraph::new(header_lines), rows[0]);

        render_choice_list(f, pending, rows[1], &t);

        let preview_lines = preview.unwrap();
        let preview_block = Block::default()
            .borders(Borders::TOP)
            .border_style(t.style_border)
            .title(Span::styled(" preview ", t.style_text_muted));
        let inner_preview = preview_block.inner(rows[2]);
        f.render_widget(preview_block, rows[2]);
        f.render_widget(
            Paragraph::new(preview_lines).style(Style::default().bg(t.surface)),
            inner_preview,
        );
    } else {
        let header_height = if is_destructive { 4 } else { 2 };
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(header_height), Constraint::Min(1)])
            .split(inner);
        let arg_cap = (width as usize).saturating_sub(tool_label.chars().count() + 5);
        let arg_truncated: String = tool_input_summary.chars().take(arg_cap).collect();
        let mut header_lines = vec![
            Line::from(vec![
                Span::styled(
                    tool_label.to_string(),
                    Style::default().fg(kind_color).add_modifier(Modifier::BOLD),
                ),
                Span::raw(" "),
                Span::styled(arg_truncated, Style::default().fg(t.text_primary)),
            ]),
            Line::from(""),
        ];
        if is_destructive {
            header_lines.push(destructive_warning_line(&t));
            header_lines.push(Line::from(""));
        }
        f.render_widget(Paragraph::new(header_lines), chunks[0]);
        render_choice_list(f, pending, chunks[1], &t);
    }
}

/// Produce a diff/content preview for the pending tool, when applicable. Returns
/// `None` for tools whose effects can't be summarized as a diff (Read, etc.).
fn build_diff_preview(tool: &ToolCall, t: &Theme) -> Option<Vec<Line<'static>>> {
    match &tool.input {
        ToolInput::Edit {
            file_path,
            old_string,
            new_string,
            ..
        } => {
            let mut lines: Vec<Line<'static>> = Vec::new();
            lines.push(Line::from(Span::styled(
                format!(" {file_path}"),
                t.style_text_secondary.add_modifier(Modifier::BOLD),
            )));
            lines.push(Line::from(""));
            for (kind, txt) in [("- ", old_string), ("+ ", new_string)] {
                let color = if kind == "- " { t.error } else { t.success };
                for ln in txt.lines().take(20) {
                    lines.push(Line::from(vec![
                        Span::styled(kind.to_owned(), Style::default().fg(color)),
                        Span::styled(ln.to_owned(), Style::default().fg(color)),
                    ]));
                }
            }
            Some(lines)
        }
        ToolInput::Write { file_path, content } => {
            let mut lines: Vec<Line<'static>> = Vec::new();
            lines.push(Line::from(Span::styled(
                format!(" {file_path}  ({} bytes)", content.len()),
                t.style_text_secondary.add_modifier(Modifier::BOLD),
            )));
            lines.push(Line::from(""));
            for ln in content.lines().take(30) {
                lines.push(Line::from(Span::styled(
                    ln.to_owned(),
                    t.style_text_primary,
                )));
            }
            let total = content.lines().count();
            if total > 30 {
                lines.push(Line::from(Span::styled(
                    format!("… {} more lines", total - 30),
                    t.style_text_muted.add_modifier(Modifier::ITALIC),
                )));
            }
            Some(lines)
        }
        ToolInput::ApplyPatch { patch } => {
            let mut lines: Vec<Line<'static>> = Vec::new();
            for ln in patch.lines().take(40) {
                let color = match ln.chars().next() {
                    Some('+') if !ln.starts_with("+++") => t.success,
                    Some('-') if !ln.starts_with("---") => t.error,
                    Some('@') => t.accent,
                    _ => t.text_secondary,
                };
                lines.push(Line::from(Span::styled(
                    ln.to_owned(),
                    Style::default().fg(color),
                )));
            }
            let total = patch.lines().count();
            if total > 40 {
                lines.push(Line::from(Span::styled(
                    format!("… {} more diff lines", total - 40),
                    t.style_text_muted.add_modifier(Modifier::ITALIC),
                )));
            }
            Some(lines)
        }
        ToolInput::Bash { command, .. } => {
            // Bash gets a single-line "preview" so the user sees the exact
            // command that would run. Useful when the summary truncates.
            Some(vec![
                Line::from(Span::styled(
                    String::from("$ "),
                    Style::default().fg(t.accent),
                )),
                Line::from(Span::styled(
                    command.clone(),
                    Style::default().fg(t.text_primary),
                )),
            ])
        }
        _ => None,
    }
}

fn render_choice_list(f: &mut Frame, pending: &PendingApproval, area: Rect, t: &Theme) {
    let items: Vec<ListItem> = ApprovalChoice::ALL
        .iter()
        .enumerate()
        .map(|(i, choice)| {
            let style = if i == pending.selected {
                Style::default()
                    .fg(t.bg)
                    .add_modifier(Modifier::BOLD)
                    .bg(t.warning)
            } else {
                t.style_text_primary
            };
            ListItem::new(Line::from(Span::styled(choice.label(), style)))
        })
        .collect();
    f.render_widget(List::new(items).style(Style::default().bg(t.surface)), area);
}

/// Check whether a tool call represents a destructive bash command AND the
/// `DestructiveWarn` feature gate is enabled.
fn is_tool_destructive(tool: &ToolCall) -> bool {
    if !crate::feature_gates::is_enabled(crate::feature_gates::FeatureGate::DestructiveWarn) {
        return false;
    }
    let ToolInput::Bash { command, .. } = &tool.input else {
        return false;
    };
    crate::auto_classifier::is_destructive_bash(command)
}

/// Render the `[⚠ DESTRUCTIVE]` warning line with explanation.
fn destructive_warning_line(t: &Theme) -> Line<'static> {
    Line::from(vec![
        Span::styled(
            "[⚠ DESTRUCTIVE]",
            Style::default().fg(t.warning).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            " This command may cause irreversible changes",
            Style::default().fg(t.warning),
        ),
    ])
}

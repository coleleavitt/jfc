use ratatui::{
    Frame,
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
};

use crate::app::App;
use jfc_session::Task;

pub(super) fn render_task_detail(f: &mut Frame, app: &App, task: &Task, area: Rect) {
    let t = app.theme;

    let mut lines: Vec<Line> = Vec::new();
    lines.push(Line::from(vec![
        Span::styled(
            format!(" {} ", task.id),
            Style::default().fg(t.accent).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            task.subject.clone(),
            Style::default()
                .fg(t.text_primary)
                .add_modifier(Modifier::BOLD),
        ),
    ]));
    lines.push(Line::from(""));

    if !task.description.is_empty() {
        append_description(&mut lines, task, area.width, t);
    }
    if let Some(form) = &task.active_form {
        lines.push(Line::from(vec![
            Span::styled("  Active: ", Style::default().fg(t.text_muted)),
            Span::styled(form.clone(), Style::default().fg(t.text_primary)),
        ]));
    }
    if let Some(owner) = &task.owner {
        lines.push(Line::from(vec![
            Span::styled("  Owner: ", Style::default().fg(t.text_muted)),
            Span::styled(format!("@{owner}"), Style::default().fg(t.text_secondary)),
        ]));
    }

    if let Some(background_task) = app
        .engine
        .background_tasks
        .values()
        .find(|background_task| {
            background_task.task_id.as_str() == task.id.as_str()
                || task
                    .owner
                    .as_deref()
                    .is_some_and(|owner| background_task.description.contains(owner))
        })
    {
        lines.extend(super::roster::agent_detail_lines(
            background_task,
            &t,
            area.width,
        ));
    }
    if !task.blocked_by.is_empty() {
        append_blockers(&mut lines, task, t);
    }

    let block = Block::default()
        .borders(Borders::TOP)
        .border_style(t.style_border)
        .title(Span::styled(
            " Detail · Esc back · ↑↓ navigate ",
            t.style_text_muted,
        ))
        .style(Style::default().bg(t.surface));
    let inner = block.inner(area);
    f.render_widget(block, area);
    f.render_widget(
        Paragraph::new(lines).style(Style::default().bg(t.surface)),
        inner,
    );
}

pub(super) fn task_model_badge(task: &Task) -> Option<String> {
    let raw = task.metadata.as_ref()?.get("model")?.as_str()?;
    let model = raw.trim();
    if model.is_empty() {
        None
    } else {
        Some(super::truncate_str(&super::agents::model_fqn(model), 20))
    }
}

fn append_description(lines: &mut Vec<Line>, task: &Task, width: u16, t: crate::theme::Theme) {
    for (index, line) in task.description.lines().enumerate() {
        if index >= 3 {
            lines.push(Line::from(Span::styled(
                "  ...",
                Style::default().fg(t.text_muted),
            )));
            break;
        }
        let trimmed = super::truncate_str(line, width.saturating_sub(4) as usize);
        lines.push(Line::from(Span::styled(
            format!("  {trimmed}"),
            Style::default().fg(t.text_secondary),
        )));
    }
    lines.push(Line::from(""));
}

fn append_blockers(lines: &mut Vec<Line>, task: &Task, t: crate::theme::Theme) {
    lines.push(Line::from(""));
    let blockers: Vec<&str> = task.blocked_by.iter().map(|id| id.as_str()).collect();
    lines.push(Line::from(vec![
        Span::styled("  Blocked by: ", Style::default().fg(t.text_muted)),
        Span::styled(blockers.join(", "), Style::default().fg(t.warning)),
    ]));
}

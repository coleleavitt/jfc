use std::collections::{HashMap, HashSet};

use ratatui::{
    Frame,
    layout::{Constraint, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Clear, Paragraph, Row, Table},
};

use crate::app::App;
use jfc_session::{DeletedFilter, Task, TaskStatus};

/// Build a tree-ordered list of tasks. Root tasks (no parent) come first,
/// children are placed immediately after their parent with depth tracking.
fn tree_order(tasks: &[Task]) -> Vec<(&Task, u8)> {
    let mut children_of: HashMap<&str, Vec<&Task>> = HashMap::new();
    let mut roots: Vec<&Task> = Vec::new();

    for t in tasks {
        if let Some(ref pid) = t.parent_id {
            children_of.entry(pid.as_str()).or_default().push(t);
        } else {
            roots.push(t);
        }
    }

    let mut result = Vec::with_capacity(tasks.len());
    let mut stack: Vec<(&Task, u8)> = roots.into_iter().rev().map(|t| (t, 0u8)).collect();

    while let Some((task, depth)) = stack.pop() {
        result.push((task, depth));
        if let Some(kids) = children_of.get(task.id.as_str()) {
            for kid in kids.iter().rev() {
                stack.push((kid, depth + 1));
            }
        }
    }
    result
}

/// Render a tree prefix for the given depth level.
fn tree_prefix(depth: u8) -> String {
    if depth == 0 {
        String::new()
    } else {
        let indent = "  ".repeat((depth - 1) as usize);
        format!("{indent}├ ")
    }
}

pub(super) fn task_panel(f: &mut Frame, app: &mut App) {
    let t = app.theme;
    let area = f.area();

    let w = (area.width as f32 * 0.80).round() as u16;
    let h = (area.height as f32 * 0.70).round() as u16;
    let x = area.x + (area.width.saturating_sub(w)) / 2;
    let y = area.y + (area.height.saturating_sub(h)) / 2;
    let popup = Rect::new(x, y, w, h);

    f.render_widget(Clear, popup);

    let all_tasks = app.task_store.list(DeletedFilter::Exclude);
    let counts = app.task_store.counts();

    let completed_ids: HashSet<&str> = all_tasks
        .iter()
        .filter(|tk| tk.status == TaskStatus::Completed)
        .map(|tk| tk.id.as_str())
        .collect();

    let title = format!(
        " Tasks · {} total ({} done, {} in progress, {} pending) ",
        counts.pending + counts.in_progress + counts.completed,
        counts.completed,
        counts.in_progress,
        counts.pending,
    );

    let header = Row::new(vec![
        Cell::from("ID").style(
            Style::default()
                .fg(t.text_muted)
                .add_modifier(Modifier::BOLD),
        ),
        Cell::from("Status").style(
            Style::default()
                .fg(t.text_muted)
                .add_modifier(Modifier::BOLD),
        ),
        Cell::from("Subject").style(
            Style::default()
                .fg(t.text_muted)
                .add_modifier(Modifier::BOLD),
        ),
        Cell::from("Owner").style(
            Style::default()
                .fg(t.text_muted)
                .add_modifier(Modifier::BOLD),
        ),
        Cell::from("Model").style(
            Style::default()
                .fg(t.text_muted)
                .add_modifier(Modifier::BOLD),
        ),
        Cell::from("Blocked By").style(
            Style::default()
                .fg(t.text_muted)
                .add_modifier(Modifier::BOLD),
        ),
    ]);

    let ordered = tree_order(&all_tasks);

    let rows: Vec<Row> = ordered
        .iter()
        .map(|(tk, depth)| {
            let status_style = match tk.status {
                TaskStatus::Pending => t.style_text_muted,
                TaskStatus::InProgress => t.style_accent_bold,
                TaskStatus::Completed => t.style_success.add_modifier(Modifier::CROSSED_OUT),
                _ => t.style_error,
            };
            let icon = format!("{} {}", tk.status.glyph(), tk.status.label());

            let subj_style = if tk.status == TaskStatus::Completed {
                t.style_text_muted.add_modifier(Modifier::CROSSED_OUT)
            } else {
                t.style_text_primary
            };

            let open_blockers: Vec<&str> = tk
                .blocked_by
                .iter()
                .filter(|id| !completed_ids.contains(id.as_str()))
                .map(|id| id.as_str())
                .collect();

            let prefix = tree_prefix(*depth);
            let subject_display = format!("{prefix}{}", tk.subject);

            Row::new(vec![
                Cell::from(tk.id.to_string()).style(Style::default().fg(t.text_muted)),
                Cell::from(icon).style(status_style),
                Cell::from(subject_display).style(subj_style),
                Cell::from(tk.owner.clone().unwrap_or_default())
                    .style(Style::default().fg(t.text_secondary)),
                Cell::from(task_model_badge(tk).unwrap_or_default())
                    .style(Style::default().fg(t.text_muted)),
                Cell::from(open_blockers.join(", ")).style(Style::default().fg(t.text_muted)),
            ])
        })
        .collect();

    // Clamp selection to valid range.
    if !ordered.is_empty() {
        let max = ordered.len().saturating_sub(1);
        if app.task_panel_selected > max {
            app.task_panel_selected = max;
        }
        app.task_panel_state.select(Some(app.task_panel_selected));
    }

    // Empty state: show a useful hint instead of a header-only table
    // when no tasks exist. The model creates tasks via TaskCreate;
    // this tells the user that and gives them a slash command path.
    if all_tasks.is_empty() {
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(t.style_border)
            .title(Span::styled(title.clone(), t.style_accent_bold))
            .title_bottom(Span::styled(" Esc close ", t.style_text_muted))
            .style(Style::default().bg(t.surface));
        let inner = block.inner(popup);
        f.render_widget(block, popup);
        let placeholder = Paragraph::new(vec![
            Line::from(""),
            Line::from(Span::styled(
                "  No tasks yet",
                Style::default()
                    .fg(t.text_muted)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(""),
            Line::from(Span::styled(
                "  The model creates tasks via TaskCreate when planning",
                Style::default().fg(t.text_muted),
            )),
            Line::from(Span::styled(
                "  multi-step work. Ask it to break down a request and",
                Style::default().fg(t.text_muted),
            )),
            Line::from(Span::styled(
                "  the list will populate here.",
                Style::default().fg(t.text_muted),
            )),
        ])
        .style(Style::default().bg(t.surface));
        f.render_widget(placeholder, inner);
        return;
    }

    let table = Table::new(
        rows,
        [
            Constraint::Length(10),
            Constraint::Length(15),
            Constraint::Min(20),
            Constraint::Length(14),
            Constraint::Length(22),
            Constraint::Length(18),
        ],
    )
    .header(header)
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(t.style_border)
            .title(Span::styled(title, t.style_accent_bold))
            .title_bottom(Span::styled(
                " ↑↓ navigate · Enter detail · Esc close ",
                t.style_text_muted.add_modifier(Modifier::ITALIC),
            ))
            .style(Style::default().bg(t.surface)),
    )
    .row_highlight_style(
        Style::default()
            .bg(t.surface_raised)
            .add_modifier(Modifier::BOLD),
    )
    .highlight_symbol("▶ ")
    .style(Style::default().bg(t.surface));

    // When detail mode is active, split the popup vertically: top = table, bottom = detail.
    let ordered_tasks: Vec<&Task> = ordered.iter().map(|(t, _)| *t).collect();
    if app.task_panel_detail && !ordered_tasks.is_empty() {
        use ratatui::layout::{Direction, Layout};

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(popup);

        f.render_stateful_widget(table, chunks[0], &mut app.task_panel_state);
        if let Some(task) = ordered_tasks.get(app.task_panel_selected) {
            render_task_detail(f, app, task, chunks[1]);
        }
    } else {
        f.render_stateful_widget(table, popup, &mut app.task_panel_state);
    }
}

/// Render the detail pane for the currently-selected task.
fn render_task_detail(f: &mut Frame, app: &App, task: &Task, area: Rect) {
    let t = app.theme;

    let mut lines: Vec<Line> = Vec::new();

    // Header: task ID + subject
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

    // Description (truncated to fit)
    if !task.description.is_empty() {
        let max_desc_lines = 3usize;
        for (i, line) in task.description.lines().enumerate() {
            if i >= max_desc_lines {
                lines.push(Line::from(Span::styled(
                    "  …",
                    Style::default().fg(t.text_muted),
                )));
                break;
            }
            let trimmed = super::truncate_str(line, area.width.saturating_sub(4) as usize);
            lines.push(Line::from(Span::styled(
                format!("  {trimmed}"),
                Style::default().fg(t.text_secondary),
            )));
        }
        lines.push(Line::from(""));
    }

    // Active form
    if let Some(ref form) = task.active_form {
        lines.push(Line::from(vec![
            Span::styled("  Active: ", Style::default().fg(t.text_muted)),
            Span::styled(form.clone(), Style::default().fg(t.text_primary)),
        ]));
    }

    // Owner
    if let Some(ref owner) = task.owner {
        lines.push(Line::from(vec![
            Span::styled("  Owner: ", Style::default().fg(t.text_muted)),
            Span::styled(format!("@{owner}"), Style::default().fg(t.text_secondary)),
        ]));
    }

    // Correlate with background_tasks to show agent-specific info
    let agent_info = app.background_tasks.values().find(|bt| {
        bt.task_id.as_str() == task.id.as_str()
            || task
                .owner
                .as_deref()
                .is_some_and(|o| bt.description.contains(o))
    });

    if let Some(bt) = agent_info {
        let elapsed_secs = bt.started_at.elapsed().as_secs();
        let elapsed_label = if elapsed_secs < 60 {
            format!("{elapsed_secs}s")
        } else if elapsed_secs < 3600 {
            format!("{}m{}s", elapsed_secs / 60, elapsed_secs % 60)
        } else {
            format!("{}h{}m", elapsed_secs / 3600, (elapsed_secs % 3600) / 60)
        };

        let total_tokens = bt
            .latest_input_tokens
            .saturating_add(bt.latest_cache_read_tokens)
            .saturating_add(bt.latest_cache_write_tokens)
            .saturating_add(bt.cumulative_output_tokens);

        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "  Progress",
            Style::default()
                .fg(t.text_muted)
                .add_modifier(Modifier::BOLD),
        )));

        let mut stats = vec![format!("  ⏱ {elapsed_label}")];
        if total_tokens > 0 {
            stats.push(format!(
                "↓ {} tokens",
                super::format_token_count(total_tokens)
            ));
        }
        if bt.tool_use_count > 0 {
            stats.push(format!(
                "{} tool{}",
                bt.tool_use_count,
                if bt.tool_use_count == 1 { "" } else { "s" }
            ));
        }
        lines.push(Line::from(Span::styled(
            stats.join(" · "),
            Style::default().fg(t.text_secondary),
        )));

        // Last tool activity
        if let Some(ref tool) = bt.last_tool {
            lines.push(Line::from(vec![
                Span::styled("  › ", Style::default().fg(t.accent)),
                Span::styled(tool.clone(), Style::default().fg(t.text_primary)),
            ]));
        }

        // Model
        if let Some(ref model) = bt.model_used {
            lines.push(Line::from(vec![
                Span::styled("  Model: ", Style::default().fg(t.text_muted)),
                Span::styled(
                    super::agents::model_fqn(model),
                    Style::default().fg(t.text_secondary),
                ),
            ]));
        }
    }

    // Blocked by
    if !task.blocked_by.is_empty() {
        lines.push(Line::from(""));
        let blockers: Vec<&str> = task.blocked_by.iter().map(|id| id.as_str()).collect();
        lines.push(Line::from(vec![
            Span::styled("  Blocked by: ", Style::default().fg(t.text_muted)),
            Span::styled(blockers.join(", "), Style::default().fg(t.warning)),
        ]));
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
        Some(super::truncate_str(model, 20))
    }
}

use super::*;
pub(crate) fn format_token_count(n: u64) -> String {
    if n < 1_000 {
        format!("{n}")
    } else if n < 1_000_000 {
        format!("{:.1}k", n as f64 / 1_000.0)
    } else {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    }
}

/// Build the trailing "· N tools · K tok" suffix for a subagent row.
/// Empty when there's no live data yet (the task just started; nothing
/// to show beyond the description). Mirrors v131's
/// `(${z.toolUseCount} tools, ${z.tokenCount} tokens)` from
/// cli.2.1.131.beautified.js.
#[allow(dead_code)]
pub(crate) fn format_subagent_counters(bt: &crate::app::BackgroundTask) -> String {
    let mut parts: Vec<String> = Vec::new();
    if let Some(model) = bt.model_used.as_deref() {
        let badge = crate::message_view::pretty_model_badge(model);
        if !badge.is_empty() {
            parts.push(badge);
        }
    }
    if bt.tool_use_count > 0 {
        parts.push(format!(
            "{} tool{}",
            bt.tool_use_count,
            if bt.tool_use_count == 1 { "" } else { "s" }
        ));
    }
    let total_tokens = bt
        .latest_input_tokens
        .saturating_add(bt.latest_cache_read_tokens)
        .saturating_add(bt.latest_cache_write_tokens)
        .saturating_add(bt.cumulative_output_tokens);
    if total_tokens > 0 {
        parts.push(format!("{} tok", format_token_count(total_tokens)));
    }
    if parts.is_empty() {
        String::new()
    } else {
        format!(" · {}", parts.join(" · "))
    }
}

pub(crate) fn render_subagent_tree(f: &mut Frame, app: &App, area: Rect) {
    if area.height == 0 || area.width < 20 {
        return;
    }
    let t = app.theme;

    // Include both live agents AND recently-completed ones — Claude
    // Code keeps finished sub-agents in the fan as hollow circles
    // ("pinned" view) so a fan of 15 doesn't visually deflate every
    // time one completes. The 5-minute window matches the
    // task-fade-out window the pinned-tasks row uses.
    const COMPLETED_PIN_WINDOW: std::time::Duration = std::time::Duration::from_secs(300);
    let now = std::time::Instant::now();
    let mut active: Vec<&crate::app::BackgroundTask> = app
        .background_tasks
        .values()
        .filter(|bt| {
            if bt.status.is_alive() {
                return true;
            }
            // Recently terminal — keep on the fan until the pin window expires.
            now.duration_since(bt.started_at) < COMPLETED_PIN_WINDOW || bt.last_tool.is_some() // had at least one tool — surface its summary briefly
        })
        .collect();
    if active.is_empty() {
        return;
    }
    active.sort_by_key(|bt| bt.task_id.as_str().to_owned());

    // New layout — mirrors Claude Code's agent panel (bullet dots, no
    // box-drawing, right-aligned status):
    //
    //   ● main                                    ↑/↓ select · Enter view
    //   ○ §8.1 EVM lifter            edit · opus · 33 tools · 193.2k tok
    //   ○ BATCH18 lvar merge safety  Bash · opus · 10 tools · 60.1k tok
    //
    // Filled bullet = currently-most-active agent (the one whose
    // last_active_agent_task matches); open bullet = idle/passive.
    // No leader "agents" row — the bullets carry the structure.
    let mut lines: Vec<Line> = Vec::new();

    // Row 0: main / leader agent. Mirrors cli.js's MainLine: the `▶ `
    // pointer prefix appears when this row is "selected" (no
    // viewing_task_id — i.e. we're focused on the input). When a
    // sub-agent is selected (viewing_task_id is Some), main loses the
    // pointer and dims slightly.
    let main_selected = app.viewing_task_id.is_none();
    let pointer = if main_selected { "▶ " } else { "  " };
    let main_bullet = if main_selected { "● " } else { "○ " };
    let main_label = "main";
    let main_hint = if main_selected {
        " ↑/↓ select · Enter to view"
    } else {
        ""
    };
    let hint_chars = main_hint.chars().count();
    let main_padding = " ".repeat(
        (area.width as usize)
            .saturating_sub(pointer.len() + main_bullet.len() + main_label.len() + hint_chars),
    );
    let main_name_style = if main_selected {
        Style::default()
            .fg(t.text_primary)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(t.text_muted)
    };
    lines.push(Line::from(vec![
        Span::styled(pointer, Style::default().fg(t.accent)),
        Span::styled(
            main_bullet,
            Style::default().fg(t.accent).add_modifier(Modifier::BOLD),
        ),
        Span::styled(main_label, main_name_style),
        Span::styled(main_padding, Style::default()),
        Span::styled(main_hint, Style::default().fg(t.text_muted)),
    ]));

    // Each sub-agent gets a row. Glyph mapping mirrors Claude Code's
    // agent panel:
    //   * Filled `●` — the active agent (most-recent stream chunk)
    //   * Hollow `○` — idle / completed / failed (the "pinned" state)
    //   * `✓` / `✗` — explicit completed / failed terminal markers
    //     applied to text color, not the glyph (keeps the column
    //     visually aligned).
    for bt in active.iter() {
        use crate::types::TaskLifecycle;
        let status = bt.status;
        let is_terminal = status.is_terminal();
        let is_idle = matches!(status, TaskLifecycle::Idle);
        let is_active = !is_idle
            && !is_terminal
            && app
                .last_active_agent_task
                .as_deref()
                .map(|id| id == bt.task_id.as_str())
                .unwrap_or(false);

        // Status suffix on the right (right-aligned). Format mirrors
        // Claude Code's "5s · ↓ 14.3k tokens" pattern — elapsed + a
        // down-arrow + token count. We have richer info available
        // (tool name, tool count) so we tuck those into the LEFT
        // half between description and the spacer, and reserve the
        // RIGHT half for time+tokens, which is what eyes track most.
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
        // Right-side label. Live agents show elapsed + token count
        // (`2m 24s · ↓ 40.3k tok`). Terminal agents swap the elapsed
        // for the lifecycle label (`completed · ↓ 40.3k tok`) so the
        // pinned hollow-circle rows read at a glance.
        let right_label: std::borrow::Cow<'_, str> = if is_terminal {
            match status {
                TaskLifecycle::Completed => "completed".into(),
                TaskLifecycle::Failed => "failed".into(),
                TaskLifecycle::Cancelled => "cancelled".into(),
                _ => elapsed_label.clone().into(),
            }
        } else {
            elapsed_label.clone().into()
        };
        let right_side = if total_tokens > 0 {
            format!(
                " {right_label} · ↓ {} tok",
                format_token_count(total_tokens)
            )
        } else {
            format!(" {right_label}")
        };

        // Selection pointer + bullet. The `▶ ` pointer appears on the
        // row whose task_id matches `viewing_task_id` — mirrors cli.js's
        // figures.pointer prefix that signals "this is what Enter
        // applies to". Other rows get 2 spaces so columns stay aligned.
        let is_selected = app
            .viewing_task_id
            .as_deref()
            .map(|id| id == bt.task_id.as_str())
            .unwrap_or(false);
        let row_pointer = if is_selected { "▶ " } else { "  " };
        let row_pointer_style = Style::default().fg(t.accent);
        // Left side: bullet + description + middle stats.
        let bullet = if is_active { "● " } else { "○ " };
        let bullet_style = if is_active {
            Style::default().fg(t.accent).add_modifier(Modifier::BOLD)
        } else if matches!(status, TaskLifecycle::Completed) {
            // Completed agents get the success color on the hollow
            // dot — the "that hollow green circle" the user pointed
            // out from Claude Code's screenshot.
            Style::default().fg(t.success)
        } else if matches!(status, TaskLifecycle::Failed) {
            Style::default().fg(t.error)
        } else if matches!(status, TaskLifecycle::Cancelled) {
            Style::default()
                .fg(t.text_muted)
                .add_modifier(Modifier::DIM)
        } else if is_idle {
            Style::default().fg(t.text_muted)
        } else {
            Style::default().fg(t.text_secondary)
        };
        let name_style = if is_active {
            Style::default()
                .fg(t.text_primary)
                .add_modifier(Modifier::BOLD)
        } else if is_terminal {
            // Terminal agents fade to muted so the eye tracks live
            // ones, but stay legible.
            Style::default().fg(t.text_muted)
        } else if is_idle {
            Style::default().fg(t.text_muted)
        } else {
            Style::default().fg(t.text_primary)
        };
        // Middle stats: tool + model + tool count (no token count
        // here — it lives on the right). Compact so the description
        // gets the most room.
        let mut mid_parts: Vec<String> = Vec::new();
        if let Some(tool) = bt.last_tool.as_deref() {
            mid_parts.push(tool.to_owned());
        }
        if let Some(model) = bt.model_used.as_deref() {
            let badge = crate::message_view::pretty_model_badge(model);
            if !badge.is_empty() {
                mid_parts.push(badge);
            }
        }
        if bt.tool_use_count > 0 {
            mid_parts.push(format!("{} tools", bt.tool_use_count));
        }
        let mid = if mid_parts.is_empty() {
            String::new()
        } else {
            format!("  · {}", mid_parts.join(" · "))
        };

        // Running/paused glyph: cli.js's PLAY_ICON `▶` for in-flight,
        // PAUSE_ICON `⏸` for terminal. Placed right after the
        // description so the eye reads `description ▶ 2m 24s` as one
        // visual chunk.
        let activity_glyph = if is_terminal {
            " ⏸ "
        } else if is_idle {
            " ⏸ "
        } else {
            " ▶ "
        };

        // Budget the description so the row fits in `area.width`:
        // pointer(2) + bullet(2) + desc + mid + activity(3) + padding + right.
        let fixed = row_pointer.chars().count()
            + bullet.chars().count()
            + mid.chars().count()
            + activity_glyph.chars().count()
            + right_side.chars().count();
        let desc_budget = (area.width as usize).saturating_sub(fixed + 2).max(8);
        let desc = bt.description.as_str();
        let desc_trimmed = if desc.chars().count() > desc_budget {
            let mut s: String = desc.chars().take(desc_budget.saturating_sub(1)).collect();
            s.push('…');
            s
        } else {
            desc.to_owned()
        };
        let pad_len = (area.width as usize).saturating_sub(
            row_pointer.chars().count()
                + bullet.chars().count()
                + desc_trimmed.chars().count()
                + mid.chars().count()
                + activity_glyph.chars().count()
                + right_side.chars().count(),
        );
        let padding = " ".repeat(pad_len);

        lines.push(Line::from(vec![
            Span::styled(row_pointer, row_pointer_style),
            Span::styled(bullet, bullet_style),
            Span::styled(desc_trimmed, name_style),
            Span::styled(mid, Style::default().fg(t.text_muted)),
            Span::styled(activity_glyph, Style::default().fg(t.text_muted)),
            Span::styled(padding, Style::default()),
            Span::styled(right_side, Style::default().fg(t.text_muted)),
        ]));
    }

    let available_height = area.height as usize;
    let display: Vec<Line> = lines.into_iter().take(available_height).collect();
    f.render_widget(
        Paragraph::new(display).style(Style::default().bg(t.surface)),
        area,
    );
}

/// Render the teammate tree widget (the "agent fan").
///
/// Shows the team structure with box-drawing connectors:
/// ```text
///    ╒═ team-lead
///    ├─ researcher: Researching… · 1.2k tokens
///    ├─ implementer: Idle for 3s
///    └─ tester: Running tests… · 5 tool uses
/// ```
pub(crate) fn render_teammate_tree(f: &mut Frame, app: &App, area: Rect) {
    use crate::swarm::{self, types::teammate_color};

    if area.height == 0 || area.width < 20 {
        return;
    }

    let t = app.theme;
    let mut lines: Vec<Line> = Vec::new();

    // Collect teammates (sorted by name for stability)
    let mut teammates: Vec<(&String, &swarm::TeammateInfo)> =
        app.team_context.teammates.iter().collect();
    teammates.sort_by_key(|(_, info)| &info.name);

    // Filter out leader
    let teammates: Vec<_> = teammates
        .into_iter()
        .filter(|(_, info)| info.name != swarm::TEAM_LEAD_NAME)
        .collect();

    if teammates.is_empty() {
        return;
    }

    // Leader row
    lines.push(Line::from(vec![
        Span::styled("   ╒═ ", Style::default().fg(t.text_muted)),
        Span::styled(
            "team-lead",
            Style::default()
                .fg(ratatui::style::Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
    ]));

    // Teammate rows
    for (i, (_, info)) in teammates.iter().enumerate() {
        let is_last = i == teammates.len() - 1;
        let connector = if is_last { "   └─ " } else { "   ├─ " };

        let color = teammate_color(info.color.as_deref());

        // Look up activity from background tasks. Match by name suffix
        // so a teammate "ui-explorer" finds its task whose id is
        // "teammate-ui-explorer@<team>".
        let bt = app
            .background_tasks
            .values()
            .find(|bt| bt.task_id.as_str().contains(&info.name));
        let bt_status = bt.map(|bt| bt.status);
        let activity = bt.and_then(|bt| bt.last_tool.clone());

        let status_text = if matches!(bt_status, Some(crate::types::TaskLifecycle::Idle)) {
            // Source-of-truth: the runner has emitted TeammateEvent::Idle.
            // Don't fall back to elapsed-since-spawn timing — that
            // misreported "Idle for 30s" while the agent was actively
            // streaming for 30s.
            ": Idle".to_owned()
        } else if matches!(bt_status, Some(crate::types::TaskLifecycle::Completed)) {
            ": Done".to_owned()
        } else if matches!(bt_status, Some(crate::types::TaskLifecycle::Failed)) {
            ": Failed".to_owned()
        } else {
            match &activity {
                Some(tool) => format!(": {}…", tool),
                None => ": Working…".to_owned(),
            }
        };

        lines.push(Line::from(vec![
            Span::styled(connector, Style::default().fg(t.text_muted)),
            Span::styled(
                &info.name,
                Style::default().fg(color).add_modifier(Modifier::BOLD),
            ),
            Span::styled(status_text, Style::default().fg(t.text_muted)),
        ]));
    }

    // Render
    let available_height = area.height as usize;
    let display_lines: Vec<Line> = lines.into_iter().take(available_height).collect();
    f.render_widget(
        Paragraph::new(display_lines).style(Style::default().bg(t.surface)),
        area,
    );
}

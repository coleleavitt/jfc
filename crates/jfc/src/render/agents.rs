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
pub fn format_subagent_counters(bt: &crate::app::BackgroundTask) -> String {
    let mut parts: Vec<String> = Vec::new();
    if let Some(model) = bt.model_used.as_deref() {
        let badge = model_fqn(model);
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
    let total_tokens = bt.total_tokens();
    if total_tokens > 0 {
        parts.push(format!("{} tok", format_token_count(total_tokens)));
    }
    if parts.is_empty() {
        String::new()
    } else {
        format!(" · {}", parts.join(" · "))
    }
}

/// Full model id (FQN) for display, with the redundant provider prefix
/// trimmed — the provider is already shown as its own badge in the
/// status bar, so `bedrock-claude-4-6-opus` reads as `claude-4-6-opus`
/// here. Unlike the old `pretty_model_badge` (which collapsed everything
/// to `opus`/`sonnet`/`haiku`), this keeps the *version*, which is what a
/// developer actually needs to tell two runs apart.
pub(crate) fn model_fqn(raw: &str) -> String {
    for prefix in ["bedrock-", "vertex-", "anthropic-", "openwebui-"] {
        if let Some(rest) = raw.strip_prefix(prefix) {
            return rest.to_owned();
        }
    }
    raw.to_owned()
}

/// Background-task ids in fleet display order — the same ordering the
/// agent fan renders (active → running → fresh-failed → idle → done →
/// stale-failed, then a stable id tie-break). The tab strip and the
/// leader-key / arrow agent navigation all step through this list, so
/// cycling moves in the same meaningful order the user sees in the fan,
/// deterministically, instead of HashMap-iteration order.
pub(crate) fn fleet_ordered_task_ids(app: &App) -> Vec<String> {
    let now = std::time::Instant::now();
    let mut ids: Vec<(&str, u8)> = app
        .engine
        .background_tasks
        .values()
        .map(|bt| {
            (
                bt.task_id.as_str(),
                fleet_rank(
                    bt.status,
                    roster_is_active(bt, app),
                    failed_is_fresh(bt, now),
                ),
            )
        })
        .collect();
    ids.sort_by(|a, b| a.1.cmp(&b.1).then_with(|| a.0.cmp(b.0)));
    ids.into_iter().map(|(id, _)| id.to_owned()).collect()
}

// Ordering + active-detection now live in the canonical roster module.
use super::roster::{failed_is_fresh, fleet_rank, roster_is_active};

#[allow(dead_code)]
pub(crate) fn render_subagent_tree(f: &mut Frame, app: &App, area: Rect) {
    use jfc_core::TaskLifecycle;
    if area.height == 0 || area.width < 20 {
        return;
    }
    let t = app.theme;
    let width = area.width as usize;

    // Include both live agents AND recently-completed ones — finished
    // sub-agents linger as a fade-out tail so a fleet of 15 doesn't
    // visually deflate every time one completes. The 5-minute window
    // matches the task-fade-out window the pinned-tasks row uses.
    const COMPLETED_PIN_WINDOW: std::time::Duration = std::time::Duration::from_secs(300);
    let now = std::time::Instant::now();
    let mut shown: Vec<&crate::app::BackgroundTask> = app
        .engine
        .background_tasks
        .values()
        .filter(|bt| {
            if bt.status.is_alive() {
                return true;
            }
            // Strictly time-windowed. The old `|| bt.last_tool.is_some()`
            // escape kept every agent that had ever used a tool pinned
            // forever — hours-old failures crowded out live agents.
            let finished_at = bt.completed_at.unwrap_or(bt.started_at);
            now.duration_since(finished_at) < COMPLETED_PIN_WINDOW
        })
        .collect();
    if shown.is_empty() {
        return;
    }

    // ── Fleet counts (over ALL background tasks, not just the window) ──
    // The summary line is the one at-a-glance health signal: how many
    // running / idle / done / failed. With 30 agents you don't scan
    // rows, you scan this line — and a red ✗N pulls the eye instantly.
    let mut n_run = 0usize;
    let mut n_idle = 0usize;
    let mut n_done = 0usize;
    let mut n_fail = 0usize;
    for bt in app.engine.background_tasks.values() {
        match bt.status {
            TaskLifecycle::Idle => n_idle += 1,
            TaskLifecycle::Failed => n_fail += 1,
            TaskLifecycle::Completed => n_done += 1,
            TaskLifecycle::Cancelled => {}
            s if s.is_alive() => n_run += 1,
            _ => {}
        }
    }
    // Sort: active → running → fresh-failed → idle → done → stale-failed.
    // Stable tie-break on id so rows don't jitter between frames. Shared
    // active-detection (roster_is_active) with the teammates panel.
    shown.sort_by(|a, b| {
        fleet_rank(a.status, roster_is_active(a, app), failed_is_fresh(a, now))
            .cmp(&fleet_rank(
                b.status,
                roster_is_active(b, app),
                failed_is_fresh(b, now),
            ))
            .then_with(|| a.task_id.as_str().cmp(b.task_id.as_str()))
    });

    let mut lines: Vec<Line> = Vec::new();

    // ── Summary header (row 0). Replaces the old "agents" box title +
    // the "● main" row. Chips are coloured by the same semantic ramp the
    // rows use; the keybinding hint is right-aligned. ──
    let mut chips: Vec<Span> = vec![Span::styled(
        "agents  ",
        Style::default()
            .fg(t.text_muted)
            .add_modifier(Modifier::BOLD),
    )];
    let mut chip_w = cell_width("agents  ");
    let push_chip = |chips: &mut Vec<Span>, chip_w: &mut usize, txt: String, col| {
        *chip_w += cell_width(&txt);
        chips.push(Span::styled(txt, Style::default().fg(col)));
    };
    if n_run > 0 {
        push_chip(&mut chips, &mut chip_w, format!("●{n_run} "), t.warning);
    }
    if n_idle > 0 {
        push_chip(&mut chips, &mut chip_w, format!("○{n_idle} "), t.text_muted);
    }
    if n_done > 0 {
        push_chip(&mut chips, &mut chip_w, format!("✓{n_done} "), t.success);
    }
    if n_fail > 0 {
        // Failures get bold so they stand out even on a busy line.
        chip_w += cell_width(&format!("✗{n_fail} "));
        chips.push(Span::styled(
            format!("✗{n_fail} "),
            Style::default().fg(t.error).add_modifier(Modifier::BOLD),
        ));
    }
    let hint = " ↑↓ select · ↵ view";
    let hint_w = cell_width(hint);
    if chip_w + hint_w < width {
        chips.push(Span::styled(
            " ".repeat(width - chip_w - hint_w),
            Style::default(),
        ));
        chips.push(Span::styled(hint, Style::default().fg(t.text_muted)));
    }
    lines.push(Line::from(chips));

    // ── Agent rows. Two fixed gutter columns up front:
    //   col 0 (accent): selection pointer ▶ / blank  — the ONE accent use
    //   col 1 (semantic): status glyph ● ○ ✓ ✗        — colour = state
    // Then description (flex, left), then ALL metadata right-aligned in
    // one column, shedding fields right-to-left when the row is narrow. ──
    //
    // Window: the summary owns 1 row; the rest go to agent rows. If the
    // fleet overflows, drop a "+N more" footer. Sorting already floated
    // the rows that matter (running/fresh failures) to the top of the window.
    let body_rows = (area.height as usize).saturating_sub(1).max(1);
    let overflow = shown.len() > body_rows;
    let visible_rows = if overflow {
        body_rows.saturating_sub(1)
    } else {
        body_rows
    };

    for bt in shown.iter().take(visible_rows) {
        lines.push(super::roster::roster_row(bt, app, width, now));
    }

    if overflow {
        let hidden = shown.len() - visible_rows;
        lines.push(Line::from(Span::styled(
            format!("  ▾ +{hidden} more (running shown first)"),
            Style::default()
                .fg(t.text_muted)
                .add_modifier(Modifier::ITALIC),
        )));
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
#[allow(dead_code)]
pub(crate) fn render_teammate_tree(f: &mut Frame, app: &App, area: Rect) {
    use crate::theme::teammate_color;
    use jfc_engine::swarm;

    if area.height == 0 || area.width < 20 {
        return;
    }

    let t = app.theme;
    let mut lines: Vec<Line> = Vec::new();

    // Collect teammates (sorted by name for stability)
    let mut teammates: Vec<(&String, &swarm::TeammateInfo)> =
        app.engine.team_context.teammates.iter().collect();
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
            Style::default().fg(t.accent).add_modifier(Modifier::BOLD),
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
            .engine
            .background_tasks
            .values()
            .find(|bt| bt.task_id.as_str().contains(&info.name));
        let bt_status = bt.map(|bt| bt.status);
        let activity = bt.and_then(|bt| bt.last_tool.clone());

        let status_text = if matches!(bt_status, Some(jfc_core::TaskLifecycle::Idle)) {
            // Source-of-truth: the runner has emitted TeammateEvent::Idle.
            // Don't fall back to elapsed-since-spawn timing — that
            // misreported "Idle for 30s" while the agent was actively
            // streaming for 30s.
            ": Idle".to_owned()
        } else if matches!(bt_status, Some(jfc_core::TaskLifecycle::Completed)) {
            ": Done".to_owned()
        } else if matches!(bt_status, Some(jfc_core::TaskLifecycle::Failed)) {
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

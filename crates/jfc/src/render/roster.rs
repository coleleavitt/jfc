//! Canonical roster rendering — ONE row format and ONE detail body for every
//! surface that shows `BackgroundTask` agents (the inline agents fan, the
//! Agents modal/teammates panel, and the task detail view).
//!
//! Before this module, each surface hand-rolled its own row: the fan had the
//! field-shedding right column + stall flag, the teammates panel had a simpler
//! `status · elapsed · tok · tools` string, and they had already drifted on
//! glyphs and ordering. Now the row builder lives here and the panels only
//! decide *which* rows fit and what chrome (border/title) wraps them.

use super::visual::{RosterColor, cell_width, roster_status_glyph, truncate_cells};
use super::*;

/// True when `bt` is the agent whose stream is live right now. Shared by every
/// roster surface so "the active one floats to the top" means the same thing
/// in each.
pub(crate) fn roster_is_active(bt: &crate::app::BackgroundTask, app: &App) -> bool {
    !matches!(bt.status, TaskLifecycle::Idle)
        && !bt.status.is_terminal()
        && app.engine.last_active_agent_task.as_deref() == Some(bt.task_id.as_str())
}

/// Sort rank for the fleet ordering. Live work first — the agent the
/// user is waiting on must be visible without scrolling — then fresh
/// failures (actionable), then idle, completed, and finally stale
/// failures from old runs, which previously buried the running agent
/// under a wall of ✗ rows. Lower = higher on screen.
pub(crate) fn fleet_rank(status: TaskLifecycle, is_active: bool, failed_is_fresh: bool) -> u8 {
    match status {
        _ if is_active => 0, // the agent whose stream is live right now
        s if s.is_alive() && !matches!(s, TaskLifecycle::Idle) => 1,
        TaskLifecycle::Failed if failed_is_fresh => 2,
        TaskLifecycle::Idle => 3,
        TaskLifecycle::Completed => 4,
        TaskLifecycle::Cancelled => 5,
        TaskLifecycle::Failed => 6, // stale failure: keep visible, but below live work
        _ => 7,
    }
}

/// A failure is "fresh" (still actionable, ranked near the top) for this
/// long after it finished; older failures sink below completed rows.
pub(crate) const FRESH_FAILURE_WINDOW: std::time::Duration = std::time::Duration::from_secs(300);

pub(crate) fn failed_is_fresh(bt: &crate::app::BackgroundTask, now: std::time::Instant) -> bool {
    let finished_at = bt.completed_at.unwrap_or(bt.started_at);
    now.duration_since(finished_at) < FRESH_FAILURE_WINDOW
}

/// Stable fleet-ordering key for a background task: active → live → fresh-fail
/// → idle → done → cancelled → stale-fail, then a `started_at` tie-break. Every
/// roster surface sorts by this so the same roster renders in the same order
/// everywhere.
pub(crate) fn roster_sort_key(
    bt: &crate::app::BackgroundTask,
    app: &App,
    now: std::time::Instant,
) -> (u8, std::time::Instant) {
    (
        fleet_rank(
            bt.status,
            roster_is_active(bt, app),
            failed_is_fresh(bt, now),
        ),
        bt.started_at,
    )
}

/// A *running* agent that hasn't produced a chunk, tool call, or token in
/// this many seconds has gone quiet (wedged on a long tool, rate-limited,
/// hung) and gets the amber `⚠ stalled Ns` flag.
const STALL_SECS: u64 = 30;

/// Build the canonical roster row for one background agent:
///
/// ```text
/// ▶ ● description ⚠ stalled 31s        tool · model · 3 shells · 42s · ↓1.2k
/// ```
///
/// * col 0 (accent): selection pointer `▶ ` / blank — the ONE accent use
/// * col 1 (semantic): status glyph `● ○ ✓ ✗` — colour = state (shared SSOT)
/// * description (flex, left), optional stall flag
/// * ALL metadata right-aligned in one column, shedding fields right-to-left
///   (tool first, then model, then count) when the row is narrow, keeping
///   time + tokens.
///
/// This is THE row format: the agents fan and the Agents panel both render it,
/// so the same `BackgroundTask` reads identically wherever it appears.
pub(crate) fn roster_row(
    bt: &crate::app::BackgroundTask,
    app: &App,
    width: usize,
    now: std::time::Instant,
) -> Line<'static> {
    let t = app.theme;
    let status = bt.status;
    let is_terminal = status.is_terminal();
    let is_idle = matches!(status, TaskLifecycle::Idle);
    let is_active = roster_is_active(bt, app);

    // Selection pointer column (accent). Selected = the row whose task_id
    // matches viewing_task_id (what ↵ opens).
    let is_selected = app
        .task_panel
        .viewing_task_id
        .as_deref()
        .map(|id| id == bt.task_id.as_str())
        .unwrap_or(false);
    let ptr = if is_selected { "▶ " } else { "  " };

    // Status glyph column (semantic colour, never accent). Shared SSOT.
    let (glyph, role) = roster_status_glyph(status, is_active);
    let glyph_col = roster_glyph_color(role, status, &t);

    let mut fields = roster_fields(bt, now);

    let gutters_w = cell_width(ptr) + cell_width(glyph);
    let name_style = roster_name_style(status, is_active, &t);

    // Stall flag (running agents only).
    let stall = if !is_terminal && !is_idle {
        let quiet = bt.last_activity_at.elapsed().as_secs();
        (quiet >= STALL_SECS).then(|| format!("  ⚠ stalled {quiet}s"))
    } else {
        None
    };
    let stall_w = stall.as_deref().map(cell_width).unwrap_or(0);

    // Fit: shrink the right column (drop lowest-priority fields) until the
    // name gets a sane minimum, then truncate the name.
    const NAME_MIN: usize = 12;
    let mut right = if fields.is_empty() {
        String::new()
    } else {
        format!("  {}", fields.join(" · "))
    };
    while !fields.is_empty()
        && gutters_w + NAME_MIN + stall_w + cell_width(&right) > width
        && fields.len() > 2
    {
        fields.remove(0);
        right = format!("  {}", fields.join(" · "));
    }
    let right_w = cell_width(&right);
    let name_budget = width
        .saturating_sub(gutters_w + stall_w + right_w)
        .max(NAME_MIN);
    let name = truncate_cells(&bt.description, name_budget);
    let pad = width.saturating_sub(gutters_w + cell_width(&name) + stall_w + right_w);

    let mut spans = vec![
        Span::styled(
            ptr,
            Style::default().fg(t.accent).add_modifier(Modifier::BOLD),
        ),
        Span::styled(glyph, Style::default().fg(glyph_col)),
        Span::styled(name, name_style),
    ];
    if let Some(stall) = stall {
        spans.push(Span::styled(
            stall,
            Style::default().fg(t.warning).add_modifier(Modifier::BOLD),
        ));
    }
    spans.push(Span::styled(" ".repeat(pad), Style::default()));
    spans.push(Span::styled(right, Style::default().fg(t.text_muted)));
    Line::from(spans)
}

/// Resolve a [`RosterColor`] role against the theme — one mapping for every
/// surface. Accent is reserved for the selection pointer; status colour is
/// always semantic.
pub(crate) fn roster_glyph_color(role: RosterColor, status: TaskLifecycle, t: &Theme) -> Color {
    match role {
        RosterColor::Active => t.warning,
        RosterColor::Success => t.success,
        RosterColor::Error => t.error,
        RosterColor::Idle | RosterColor::Muted => {
            if matches!(status, TaskLifecycle::Cancelled | TaskLifecycle::Idle) {
                t.text_muted
            } else {
                t.text_secondary
            }
        }
    }
}

/// Name style: bold while active, red on failure, muted when terminal/idle,
/// primary otherwise.
fn roster_name_style(status: TaskLifecycle, is_active: bool, t: &Theme) -> Style {
    if is_active {
        Style::default()
            .fg(t.text_primary)
            .add_modifier(Modifier::BOLD)
    } else if matches!(status, TaskLifecycle::Failed) {
        Style::default().fg(t.error)
    } else if status.is_terminal() || matches!(status, TaskLifecycle::Idle) {
        Style::default().fg(t.text_muted)
    } else {
        Style::default().fg(t.text_primary)
    }
}

/// Right-side metadata fields in priority order (low → high): tool · model ·
/// count · time · ↓tok. The row builder drops from the FRONT (lowest priority
/// first) when the row is narrow, keeping time + tokens.
fn roster_fields(bt: &crate::app::BackgroundTask, now: std::time::Instant) -> Vec<String> {
    let status = bt.status;
    let elapsed = super::visual::format_elapsed_secs(now.duration_since(bt.started_at).as_secs());
    let time_label = if status.is_terminal() {
        match status {
            TaskLifecycle::Completed => "done".to_owned(),
            TaskLifecycle::Failed => "failed".to_owned(),
            TaskLifecycle::Cancelled => "cancelled".to_owned(),
            _ => elapsed,
        }
    } else {
        elapsed
    };
    let mut fields: Vec<String> = Vec::new();
    if let Some(tool) = bt.last_tool_info.as_deref().or(bt.last_tool.as_deref()) {
        fields.push(tool.to_owned());
    }
    if let Some(model) = bt.model_used.as_deref() {
        let m = super::agents::model_fqn(model);
        if !m.is_empty() {
            fields.push(m);
        }
    }
    if bt.tool_use_count > 0 {
        fields.push(tool_activity_count_label(bt));
    }
    fields.push(time_label);
    let total_tokens = bt.total_tokens();
    if total_tokens > 0 {
        fields.push(format!(
            "↓{}",
            super::agents::format_token_count(total_tokens)
        ));
    }
    fields
}

/// Build the canonical agent-detail body — the Progress stats line, last tool,
/// model, and the "Recent activity" transcript tail — for one background
/// agent. Used by every detail surface (the Tasks detail pane today; any
/// future agent drill-in) so the same agent reads identically wherever the
/// user inspects it.
pub(crate) fn agent_detail_lines(
    bt: &crate::app::BackgroundTask,
    t: &Theme,
    width: u16,
) -> Vec<Line<'static>> {
    let mut lines: Vec<Line> = Vec::new();

    // Freeze the elapsed clock once the agent reaches a terminal state.
    // `started_at`/`completed_at` are live `Instant`s, so reading
    // `started_at.elapsed()` unconditionally made a Failed/Completed
    // agent's "⏱" counter keep climbing forever (the fan rows already
    // freeze via `roster_fields`; the detail header did not). For a
    // terminal agent show the actual run duration (start → completion).
    let elapsed_secs = match bt.completed_at {
        Some(done) if bt.status.is_terminal() => {
            done.saturating_duration_since(bt.started_at).as_secs()
        }
        _ => bt.started_at.elapsed().as_secs(),
    };
    let elapsed_label = super::visual::format_elapsed_secs(elapsed_secs);
    let total_tokens = bt.total_tokens();

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
            super::agents::format_token_count(total_tokens)
        ));
    }
    if bt.tool_use_count > 0 {
        stats.push(tool_activity_count_label(bt));
    }
    lines.push(Line::from(Span::styled(
        stats.join(" · "),
        Style::default().fg(t.text_secondary),
    )));

    // Last tool activity
    if let Some(tool) = bt.last_tool_info.as_ref().or(bt.last_tool.as_ref()) {
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

    if let Some(result) = terminal_result_line(bt, t, width) {
        lines.push(result);
    }

    lines.extend(recent_activity_lines(bt, t, width));
    lines.extend(transcript_lines(bt, t, width));

    lines
}

fn tool_activity_count_label(bt: &crate::app::BackgroundTask) -> String {
    let kind = match bt.last_tool.as_deref() {
        Some("Bash") => "shell",
        _ => "tool",
    };
    format!(
        "{} {kind}{}",
        bt.tool_use_count,
        if bt.tool_use_count == 1 { "" } else { "s" }
    )
}

fn terminal_result_line(
    bt: &crate::app::BackgroundTask,
    t: &Theme,
    width: u16,
) -> Option<Line<'static>> {
    let (label, body, style) = match bt.status {
        jfc_core::TaskLifecycle::Completed => (
            "Result",
            bt.summary.as_deref()?,
            Style::default().fg(t.success),
        ),
        jfc_core::TaskLifecycle::Failed | jfc_core::TaskLifecycle::Cancelled => {
            ("Error", bt.error.as_deref()?, Style::default().fg(t.error))
        }
        _ => return None,
    };
    let prefix = format!("  {label}: ");
    let body_width = width.saturating_sub(prefix.len() as u16).max(8) as usize;
    let preview = super::truncate_str(&body.replace('\n', " "), body_width);
    Some(Line::from(vec![
        Span::styled(prefix, Style::default().fg(t.text_muted)),
        Span::styled(preview, style),
    ]))
}

fn recent_activity_lines(
    bt: &crate::app::BackgroundTask,
    t: &Theme,
    width: u16,
) -> Vec<Line<'static>> {
    if bt.recent_activities.is_empty() {
        return Vec::new();
    }
    let mut lines = vec![
        Line::from(""),
        Line::from(Span::styled(
            "  Recent tools",
            Style::default()
                .fg(t.text_muted)
                .add_modifier(Modifier::BOLD),
        )),
    ];
    for activity in &bt.recent_activities {
        let elapsed_secs = activity.elapsed_ms / 1000;
        let elapsed = super::visual::format_elapsed_secs(elapsed_secs);
        let label = activity.kind.label();
        let subject = activity.display.replace('\n', " ");
        let subject = super::truncate_str(&subject, width.saturating_sub(18) as usize);
        lines.push(Line::from(vec![
            Span::styled("  › ", Style::default().fg(t.accent)),
            Span::styled(format!("[{elapsed}] "), Style::default().fg(t.text_muted)),
            Span::styled(format!("{label} "), Style::default().fg(t.text_secondary)),
            Span::styled(subject, Style::default().fg(t.text_primary)),
        ]));
    }
    lines
}

/// The "Recent activity" transcript tail. The detached-agent sync
/// reconstructs `chat_messages` from the worker log; show the last few
/// entries (role-tagged one-line previews, width-clipped) so a user can drill
/// into a running agent's activity without leaving the panel.
fn transcript_lines(bt: &crate::app::BackgroundTask, t: &Theme, width: u16) -> Vec<Line<'static>> {
    let transcript: Vec<&jfc_core::ChatMessage> = bt
        .chat_messages
        .iter()
        .rev()
        .take(6)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    if transcript.is_empty() {
        return Vec::new();
    }
    let mut lines = vec![
        Line::from(""),
        Line::from(Span::styled(
            "  Recent activity",
            Style::default()
                .fg(t.text_muted)
                .add_modifier(Modifier::BOLD),
        )),
    ];
    for msg in transcript {
        let text = msg
            .parts
            .iter()
            .map(|p| p.text_only())
            .collect::<Vec<_>>()
            .join(" ");
        let text = text.trim();
        if text.is_empty() {
            continue;
        }
        let (tag, tag_style) = match msg.role {
            jfc_core::Role::User => ("›", Style::default().fg(t.accent)),
            jfc_core::Role::Assistant => ("⟐", Style::default().fg(t.text_secondary)),
        };
        // One-line preview per message — collapse newlines, clip to the panel
        // width so multi-paragraph replies don't blow up the row.
        let preview = text.replace('\n', " ");
        let preview = super::truncate_str(&preview, width.saturating_sub(6) as usize);
        lines.push(Line::from(vec![
            Span::styled(format!("  {tag} "), tag_style),
            Span::styled(preview, Style::default().fg(t.text_secondary)),
        ]));
    }
    lines
}

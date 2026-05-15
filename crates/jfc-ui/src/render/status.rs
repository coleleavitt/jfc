use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::Style,
    text::{Line, Span},
    widgets::{LineGauge, Paragraph},
};

use crate::app::App;
use crate::types::Role;

pub(super) fn status(f: &mut Frame, app: &App, area: Rect) {
    let t = app.theme;

    // Two-row status: row 0 = info line (model, profile, cwd, hints),
    // row 1 = context-window LineGauge with color-coded usage.
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Length(1)])
        .split(area);

    let cwd_display = {
        let home = std::env::var("HOME").unwrap_or_default();
        app.cwd
            .strip_prefix(&home)
            .map(|rest| format!("~{rest}"))
            .unwrap_or_else(|| app.cwd.clone())
    };

    let msg_count = app.messages.iter().filter(|m| m.role == Role::User).count();

    // Build status badges in priority order. Each badge is a short
    // string fragment; we join them with a `·` separator and truncate
    // from the right so low-priority badges drop first at narrow
    // widths while the model name and git branch always show.
    let mut badges: Vec<String> = Vec::new();

    badges.push(app.model.to_string());
    badges.push(effort_status_badge(app));

    if app.fast_mode {
        badges.push("⚡ FAST".to_string());
    }

    if let Some(status) = app.claude_status.as_ref() {
        if status.is_degraded() {
            badges.push(status.short_badge());
        }
    } else if app.claude_status_error.is_some() {
        badges.push("status unreachable".to_string());
    }

    match (&app.subscription_type, &app.seat_tier) {
        (Some(sub), Some(tier)) => badges.push(format!("{}·{}", sub, tier)),
        (Some(sub), None) => badges.push(sub.clone()),
        (None, Some(tier)) => badges.push(tier.clone()),
        (None, None) => {}
    }

    match app.permission_mode {
        crate::app::PermissionMode::Default => {}
        mode => badges.push(format!("{} {}", mode.symbol(), mode.label())),
    }

    if let Some(branch) = app.git_branch.as_deref()
        && !branch.is_empty()
    {
        let trimmed: String = if branch.chars().count() > 24 {
            let mut s: String = branch.chars().take(23).collect();
            s.push('…');
            s
        } else {
            branch.to_owned()
        };
        badges.push(format!("⎇ {}", trimmed));
    }

    let cost_total = crate::cost::total_cost(&app.usage_by_model);
    if cost_total > 0.001 {
        let cost_str = if cost_total < 0.01 {
            format!("${:.4}", cost_total)
        } else if cost_total < 10.0 {
            format!("${:.3}", cost_total)
        } else {
            format!("${:.2}", cost_total)
        };
        badges.push(cost_str);
    }

    if app.leader_key_active {
        badges.push("[^X …]".to_string());
    } else if app.viewing_task_id.is_some() {
        badges.push("[task view]".to_string());
    }

    let alive: Vec<&crate::app::BackgroundTask> = app
        .background_tasks
        .values()
        .filter(|bt| bt.status.is_alive())
        .collect();
    if !alive.is_empty() {
        let total_tools: u32 = alive.iter().map(|b| b.tool_use_count).sum();
        let total_tokens: u64 = alive
            .iter()
            .map(|b| {
                b.latest_input_tokens
                    .saturating_add(b.latest_cache_read_tokens)
                    .saturating_add(b.latest_cache_write_tokens)
                    .saturating_add(b.cumulative_output_tokens)
            })
            .sum();
        let mut s = format!("⏵ {}", alive.len());
        if total_tools > 0 {
            s.push_str(&format!(
                " · {} tool{}",
                total_tools,
                if total_tools == 1 { "" } else { "s" }
            ));
        }
        if total_tokens > 0 {
            s.push_str(&format!(
                " · {} tok",
                super::format_token_count(total_tokens)
            ));
        }
        badges.push(s);
    }

    if app.worktree_count > 0 {
        badges.push(format!("⌥ {} wt", app.worktree_count));
    }

    if !app.queued_prompts.is_empty() {
        badges.push(format!("⏳ {} queued", app.queued_prompts.len()));
    }

    let approval_count =
        app.approval_queue.len() + if app.pending_approval.is_some() { 1 } else { 0 };
    if approval_count > 0 {
        badges.push(format!("⏸ {approval_count}"));
    }

    if let Some(save_t) = app.last_session_save_at
        && save_t.elapsed().as_millis() < 2000
    {
        badges.push("✓ saved".to_string());
    }

    badges.push(format!("{} · {} msgs", cwd_display, msg_count));

    let right = " ? help · ^P palette ";

    let total_width = area.width as usize;
    let right_start = total_width.saturating_sub(right.len());

    let left_full = format!(" {} ", badges.join(" · "));
    let left_chars: usize = left_full.chars().count();
    let left_truncated = if left_chars > right_start.saturating_sub(1) {
        let truncated: String = left_full
            .chars()
            .take(right_start.saturating_sub(2))
            .collect();
        format!("{truncated}…")
    } else {
        left_full
    };

    let padding = " ".repeat(right_start.saturating_sub(left_truncated.chars().count()));

    let line = Line::from(vec![
        Span::styled(left_truncated, Style::default().fg(t.text_secondary)),
        Span::styled(padding, Style::default().fg(t.text_muted)),
        Span::styled(right, Style::default().fg(t.text_muted)),
    ]);

    f.render_widget(
        Paragraph::new(line).style(Style::default().bg(t.surface)),
        rows[0],
    );

    let used = app.tool_ctx.approx_tokens;
    let max = app.max_context_tokens.max(1);
    let ratio = (used as f64 / max as f64).clamp(0.0, 1.0);
    let pct = (ratio * 100.0).round() as u32;
    let bar_color = if pct < 60 {
        t.success
    } else if pct < 85 {
        t.warning
    } else {
        t.error
    };
    let label = context_gauge_label(used, max, pct);
    let gauge = LineGauge::default()
        .filled_style(Style::default().fg(bar_color))
        .unfilled_style(t.style_border)
        .label(Span::styled(label, t.style_text_secondary))
        .ratio(ratio);
    f.render_widget(gauge, rows[1]);
}

pub(super) fn context_gauge_label(used: usize, max: usize, pct: u32) -> String {
    format!(" ctx {}k / {}k · {}% ", used / 1000, max / 1000, pct)
}

pub(super) fn effort_status_badge(app: &App) -> String {
    match app.effort_state.current {
        Some(effort) => format!("effort {effort}"),
        None => "effort default".to_string(),
    }
}

pub(super) fn claude_status_footer(app: &App) -> String {
    if let Some(status) = app.claude_status.as_ref() {
        let age = status.age_secs();
        let net = format_bytes(status.bytes_in.saturating_add(status.bytes_out));
        if let Some(outage) = status.outage_context() {
            format!(
                " · {} · {}s · net {}",
                super::truncate_str(&outage, 36),
                age,
                net
            )
        } else {
            format!(" · status ok · {}s · net {}", age, net)
        }
    } else if app.claude_status_error.is_some() {
        " · status offline".to_owned()
    } else {
        String::new()
    }
}

fn format_bytes(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{bytes}B")
    } else if bytes < 1024 * 1024 {
        format!("{:.1}kB", bytes as f64 / 1024.0)
    } else {
        format!("{:.1}MB", bytes as f64 / (1024.0 * 1024.0))
    }
}

use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
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

    // Provider name renders as its own styled Span at the start of the
    // line so we can pulse it independently — the rest of the badges
    // share one foreground color, but the provider gets a subtle
    // alpha-blend pulse so a glance reveals which backend is live (bedrock
    // / anthropic-oauth / openwebui) without crowding the model name.
    let provider_badge = pretty_provider_label(app.provider.name());
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

    // Provider prefix: ` ● <provider> · `. The bullet was previously
    // pulsing on wall-clock time (every 1.6s regardless of network),
    // which made the status bar feel "alive" even when literally
    // nothing was happening — even typing in the input box, which
    // has no network signal. Now the bullet's intensity is driven by
    // the same `network_activity` factor the EKG sparkline uses, so
    // the two stay in lockstep: when the EKG is actively beating
    // (real bytes arriving), the bullet glows; when the wire is idle,
    // the bullet sits dim. Plumb through the EKG's beat-armed state
    // so the bullet "decays" alongside the trace over the PATTERN_LEN
    // tick window after the last byte.
    let beat_alive = app.network_beat_remaining > 0;
    let dot_intensity = if beat_alive {
        // Active beat → blend in proportion to current R-wave height.
        (0.4 + app.network_activity * 0.6).clamp(0.0, 1.0)
    } else {
        // Idle → flat. No pulse, no fake "things are happening" signal.
        0.0
    };
    let dot_color = blend_color(t.border, provider_accent(app.provider.name()), dot_intensity);

    let badge_str = format!(" {} · ", badges.join(" · "));
    // Budget for the badge string = right_start − provider prefix width.
    let provider_prefix_width = 4 + provider_badge.chars().count(); // " ● " + name + " · "
    let badge_budget = right_start.saturating_sub(provider_prefix_width).max(1);
    let badge_truncated = if badge_str.chars().count() > badge_budget {
        let truncated: String = badge_str.chars().take(badge_budget.saturating_sub(1)).collect();
        format!("{truncated}…")
    } else {
        badge_str
    };
    let consumed = provider_prefix_width + badge_truncated.chars().count();
    let padding = " ".repeat(right_start.saturating_sub(consumed));

    let line = Line::from(vec![
        Span::raw(" "),
        Span::styled("●", Style::default().fg(dot_color).add_modifier(Modifier::BOLD)),
        Span::raw(" "),
        Span::styled(
            provider_badge,
            Style::default()
                .fg(provider_accent(app.provider.name()))
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(badge_truncated, Style::default().fg(t.text_secondary)),
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

/// Friendly short label for a provider id. Anthropic providers are common
/// enough that we collapse `anthropic-oauth` to just `OAuth`; bedrock and
/// openwebui get their own short labels.
pub(super) fn pretty_provider_label(provider: &str) -> String {
    match provider {
        "anthropic" => "API".to_owned(),
        "anthropic-oauth" => "OAuth".to_owned(),
        "bedrock" => "Bedrock".to_owned(),
        "vertex" => "Vertex".to_owned(),
        "openwebui" => "OpenWebUI".to_owned(),
        "codex" => "Codex".to_owned(),
        other => other.to_owned(),
    }
}

/// Accent color per provider so the live-stream pulse reads as
/// "Bedrock-orange" / "OpenWebUI-teal" at a glance.
pub(super) fn provider_accent(provider: &str) -> Color {
    match provider {
        "anthropic" | "anthropic-oauth" | "bedrock" | "vertex" => Color::Rgb(204, 120, 50),
        "openwebui" => Color::Rgb(100, 180, 200),
        _ => Color::Gray,
    }
}

/// Linear-RGB interpolation between two `Color`s. Used for the provider
/// dot's pulse and the network EKG's leading-edge highlight. Colors
/// that aren't `Rgb` fall back to a binary swap at `t > 0.5` because
/// ratatui's palette colors don't carry component data.
pub(super) fn blend_color(from: Color, to: Color, t: f32) -> Color {
    let t = t.clamp(0.0, 1.0);
    match (from, to) {
        (Color::Rgb(r0, g0, b0), Color::Rgb(r1, g1, b1)) => {
            let lerp = |a: u8, b: u8| -> u8 {
                let af = a as f32;
                let bf = b as f32;
                (af + (bf - af) * t).round().clamp(0.0, 255.0) as u8
            };
            Color::Rgb(lerp(r0, r1), lerp(g0, g1), lerp(b0, b1))
        }
        _ => {
            if t < 0.5 {
                from
            } else {
                to
            }
        }
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

use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{LineGauge, Paragraph},
};

use crate::app::App;
use crate::types::Role;

/// One status-bar segment: its own pre-styled spans (so a segment can be
/// multi-colored, e.g. the diff stat's green `+` / red `−`) plus a drop
/// priority. When the line is too narrow the lowest-priority segments are
/// removed first, instead of char-truncating one monochrome run.
struct StatusSeg {
    spans: Vec<Span<'static>>,
    prio: u8,
}

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

    // ── Build prioritised, colour-coded segments (see `StatusSeg`) ──
    let provider_badge = pretty_provider_label(app.provider.name());
    let muted = Style::default().fg(t.text_muted);
    let sec = Style::default().fg(t.text_secondary);
    let gold = Style::default().fg(t.warning);
    let mut segs: Vec<StatusSeg> = Vec::new();
    macro_rules! push1 {
        ($s:expr, $style:expr, $prio:expr $(,)?) => {
            segs.push(StatusSeg {
                spans: vec![Span::styled($s, $style)],
                prio: $prio,
            })
        };
    }

    // Identity: model first, then cost in gold (the number you watch).
    push1!(app.model.to_string(), sec, 100);
    let cost_total = crate::cost::total_cost(&app.usage_by_model);
    if cost_total > 0.001 {
        let cost_str = if cost_total < 0.01 {
            format!("${:.4}", cost_total)
        } else if cost_total < 10.0 {
            format!("${:.3}", cost_total)
        } else {
            format!("${:.2}", cost_total)
        };
        push1!(cost_str, gold.add_modifier(Modifier::BOLD), 95);
    }

    // Problems / actionable state — high priority, coloured to draw the eye.
    let mcp_down: Vec<&str> = app
        .mcp_servers
        .iter()
        .filter(|s| matches!(s.status, crate::types::McpStatus::Error))
        .map(|s| s.name.as_str())
        .collect();
    if !mcp_down.is_empty() {
        push1!(
            format!("⚠ mcp: {}", mcp_down.join(", ")),
            Style::default().fg(t.error).add_modifier(Modifier::BOLD),
            93,
        );
    }
    if let Some(status) = app.claude_status.as_ref() {
        if status.is_degraded() {
            push1!(status.short_badge(), gold, 92);
        }
    } else if app.claude_status_error.is_some() {
        push1!(
            "status unreachable".to_owned(),
            Style::default().fg(t.error),
            92
        );
    }
    let approval_count =
        app.approval_queue.len() + if app.pending_approval.is_some() { 1 } else { 0 };
    if approval_count > 0 {
        push1!(
            format!("⏸ {approval_count}"),
            gold.add_modifier(Modifier::BOLD),
            90,
        );
    }
    if app.leader_key_active {
        push1!("[^X …]".to_owned(), Style::default().fg(t.accent), 88);
    } else if app.viewing_task_id.is_some() {
        push1!("[task view]".to_owned(), Style::default().fg(t.accent), 88);
    }
    if !app.queued_prompts.is_empty() {
        push1!(format!("⏳ {} queued", app.queued_prompts.len()), muted, 80);
    }
    let alive_n = app
        .background_tasks
        .values()
        .filter(|bt| bt.status.is_alive())
        .count();
    if alive_n > 0 {
        let tools: u32 = app
            .background_tasks
            .values()
            .filter(|bt| bt.status.is_alive())
            .map(|b| b.tool_use_count)
            .sum();
        let s = if tools > 0 {
            format!("⏵ {alive_n} · {tools} tools")
        } else {
            format!("⏵ {alive_n}")
        };
        push1!(s, gold, 78);
    }

    // Mode flags.
    if let crate::app::PermissionMode::Default = app.permission_mode {
    } else {
        push1!(
            format!(
                "{} {}",
                app.permission_mode.symbol(),
                app.permission_mode.label()
            ),
            gold,
            85,
        );
    }
    if app.fast_mode {
        push1!("⚡ FAST".to_owned(), gold, 60);
    }
    if app.effort_state.current.is_some() {
        push1!(effort_status_badge(app), muted, 50);
    }
    if let Some(ref rc) = app.remote_host {
        let clients = rc.client_count.load(std::sync::atomic::Ordering::Relaxed);
        let label = if clients > 0 {
            format!("📡 RC ({clients})")
        } else {
            "📡 RC".to_owned()
        };
        push1!(label, gold, 55);
    }

    // Repo zone: branch · diff (green/red) · cwd.
    if let Some(branch) = app.git_branch.as_deref().filter(|b| !b.is_empty()) {
        push1!(format!("⎇ {}", super::truncate_str(branch, 24)), muted, 70);
    }
    let diff = super::collect_diff_stats(app);
    if diff.total_files > 0 {
        segs.push(StatusSeg {
            spans: vec![
                Span::styled("Δ ", muted),
                Span::styled(
                    format!("+{}", diff.additions),
                    Style::default().fg(t.success),
                ),
                Span::styled(" ", muted),
                Span::styled(format!("−{}", diff.deletions), Style::default().fg(t.error)),
            ],
            prio: 65,
        });
    }
    push1!(cwd_display, muted, 45);

    match (&app.subscription_type, &app.seat_tier) {
        (Some(sub), Some(tier)) => push1!(format!("{sub}·{tier}"), muted, 40),
        (Some(sub), None) => push1!(sub.clone(), muted, 40),
        (None, Some(tier)) => push1!(tier.clone(), muted, 40),
        (None, None) => {}
    }
    if app.worktree_count > 0 {
        push1!(format!("⌥ {} wt", app.worktree_count), muted, 30);
    }
    if app
        .last_session_save_at
        .is_some_and(|t| t.elapsed().as_millis() < 2000)
    {
        push1!("✓ saved".to_owned(), Style::default().fg(t.success), 35);
    }
    // Lowest value — drops first when the line is tight.
    push1!(format!("{msg_count} msgs"), muted, 10);

    // ── Assemble: provider prefix (fixed) · segments · pad · right ──
    let right = " ? help · ^P palette ";
    let total_width = area.width as usize;
    let right_w = super::cell_width(right);
    let avail = total_width.saturating_sub(right_w);

    // Static provider dot — the network EKG (which used to drive a pulse
    // here) is gone; a steady provider-coloured dot just identifies the
    // backend without faking "liveness".
    let dot_color = provider_accent(app.provider.name());

    let prefix_w = 3 + super::cell_width(&provider_badge); // " ● <provider>"
    const SEP_W: usize = 3; // " · "
    let seg_w = |s: &StatusSeg| -> usize {
        s.spans
            .iter()
            .map(|sp| super::cell_width(&sp.content))
            .sum()
    };

    // Drop the lowest-priority segment until the line fits.
    loop {
        let segs_w: usize = segs.iter().map(|s| SEP_W + seg_w(s)).sum();
        if prefix_w + segs_w <= avail || segs.is_empty() {
            break;
        }
        if let Some((i, _)) = segs.iter().enumerate().min_by_key(|(_, s)| s.prio) {
            segs.remove(i);
        } else {
            break;
        }
    }

    let kept_w: usize = segs.iter().map(|s| SEP_W + seg_w(s)).sum();
    let pad = avail.saturating_sub(prefix_w + kept_w);

    let mut spans: Vec<Span> = vec![
        Span::raw(" "),
        Span::styled(
            "●",
            Style::default().fg(dot_color).add_modifier(Modifier::BOLD),
        ),
        Span::raw(" "),
        Span::styled(
            provider_badge,
            Style::default()
                .fg(provider_accent(app.provider.name()))
                .add_modifier(Modifier::BOLD),
        ),
    ];
    for s in segs {
        spans.push(Span::styled(" · ", muted));
        spans.extend(s.spans);
    }
    spans.push(Span::styled(" ".repeat(pad), Style::default()));
    spans.push(Span::styled(right, muted));

    f.render_widget(
        Paragraph::new(Line::from(spans)).style(Style::default().bg(t.surface)),
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

#[allow(dead_code)]
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

#[allow(dead_code)]
fn format_bytes(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{bytes}B")
    } else if bytes < 1024 * 1024 {
        format!("{:.1}kB", bytes as f64 / 1024.0)
    } else {
        format!("{:.1}MB", bytes as f64 / (1024.0 * 1024.0))
    }
}

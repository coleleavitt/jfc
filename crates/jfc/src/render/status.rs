use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
};

use crate::app::App;

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

    // The footer is two rows: row 0 is the divider line that DOUBLES as the
    // context gauge (it fills left→right and shifts green→amber→red as context
    // grows — no separate "ctx … bar" row), row 1 is the info line. This is
    // one fewer dense row than a dedicated gauge and reads softer.
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Length(1)])
        .split(area);

    // ── Row 0: divider-as-context-gauge ──────────────────────────────────
    {
        let used = app.engine.tool_ctx.approx_tokens;
        let max = app.engine.max_context_tokens.max(1);
        let ratio = (used as f64 / max as f64).clamp(0.0, 1.0);
        let pct = (ratio * 100.0).round() as u32;
        let gauge_color = if pct < 60 {
            t.success
        } else if pct < 85 {
            t.warning
        } else {
            t.error
        };
        // Compact count parked at the right end of the divider; the fill on the
        // left is the at-a-glance signal.
        let label = format!(" {} ", context_gauge_label(used, max, pct).trim());
        let w = rows[0].width as usize;
        let label_w = super::cell_width(&label).min(w);
        let gauge_w = w.saturating_sub(label_w);
        let filled = ((ratio * gauge_w as f64).round() as usize).min(gauge_w);
        let divider = Line::from(vec![
            Span::styled("─".repeat(filled), Style::default().fg(gauge_color)),
            Span::styled("─".repeat(gauge_w - filled), t.style_border),
            Span::styled(label, t.style_text_muted),
        ]);
        f.render_widget(
            Paragraph::new(divider).style(Style::default().bg(t.surface)),
            rows[0],
        );
    }

    // Just the project directory name — the full path was noise on a line
    // that's already tight; the branch + model carry the working context.
    let cwd_display = std::path::Path::new(&app.engine.cwd)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| app.engine.cwd.clone());

    // ── Build prioritised, colour-coded segments (see `StatusSeg`) ──
    let provider_badge = pretty_provider_label(app.engine.provider.name());
    let muted = Style::default().fg(t.text_muted);
    let sec = Style::default().fg(t.text_secondary);
    // Semantic split (was all `warning` gold): `cost` rides the money hue,
    // genuine alerts stay `warning`/`error`, and routine activity/mode/flag
    // state uses the calm `accent_secondary` so it no longer screams.
    let cost_style = Style::default().fg(t.cost_signal);
    let alert = Style::default().fg(t.warning);
    let activity = Style::default().fg(t.accent_secondary);
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
    push1!(app.engine.model.to_string(), sec, 100);
    let cost_total = jfc_engine::cost::total_cost(&app.engine.usage_by_model);
    if cost_total > 0.001 {
        let cost_str = if cost_total < 0.01 {
            format!("${:.4}", cost_total)
        } else if cost_total < 10.0 {
            format!("${:.3}", cost_total)
        } else {
            format!("${:.2}", cost_total)
        };
        push1!(cost_str, cost_style.add_modifier(Modifier::BOLD), 95);
    }

    // Problems / actionable state — high priority, coloured to draw the eye.
    let mcp_down: Vec<&str> = app
        .engine
        .mcp_servers
        .iter()
        .filter(|s| matches!(s.status, jfc_core::McpStatus::Error))
        .map(|s| s.name.as_str())
        .collect();
    if !mcp_down.is_empty() {
        push1!(
            format!("⚠ mcp: {}", mcp_down.join(", ")),
            Style::default().fg(t.error).add_modifier(Modifier::BOLD),
            93,
        );
    }
    if let Some(status) = app.engine.claude_status.as_ref() {
        if status.is_degraded() {
            push1!(status.short_badge(), alert, 92);
        }
    } else if app.engine.claude_status_error.is_some() {
        push1!(
            "status unreachable".to_owned(),
            Style::default().fg(t.error),
            92
        );
    }
    let approval_count = app.engine.approval_queue.len()
        + if app.engine.pending_approval.is_some() {
            1
        } else {
            0
        };
    if approval_count > 0 {
        push1!(
            format!("{approval_count} pending"),
            alert.add_modifier(Modifier::BOLD),
            90,
        );
    }
    if app.leader_key_active {
        push1!("[^X …]".to_owned(), Style::default().fg(t.accent), 88);
    } else if app.viewing_task_id.is_some() {
        push1!("[task view]".to_owned(), Style::default().fg(t.accent), 88);
    }
    if !app.engine.queued_prompts.is_empty() {
        push1!(
            format!("⏳ {} queued", app.engine.queued_prompts.len()),
            muted,
            80
        );
    }
    let alive_n = app
        .engine
        .background_tasks
        .values()
        .filter(|bt| bt.status.is_alive())
        .count();
    if alive_n > 0 {
        let tools: u32 = app
            .engine
            .background_tasks
            .values()
            .filter(|bt| bt.status.is_alive())
            .map(|b| b.tool_use_count)
            .sum();
        let s = if tools > 0 {
            format!("{alive_n} agents · {tools} tools")
        } else {
            format!("{alive_n} agents")
        };
        push1!(s, activity, 78);
    }

    // Mode flags.
    if let crate::app::PermissionMode::Default = app.engine.permission_mode {
    } else {
        // Plain label — the mode word (Bypass / Auto / Plan) reads on its own;
        // the leading symbol was emoji-zoo noise.
        push1!(app.engine.permission_mode.label().to_owned(), activity, 85);
    }
    if app.engine.fast_mode {
        push1!("fast".to_owned(), activity, 60);
    }
    if app.engine.effort_state.current.is_some() {
        push1!(effort_status_badge(app), muted, 50);
    }
    if let Some(ref rc) = app.remote_host {
        let clients = rc.client_count.load(std::sync::atomic::Ordering::Relaxed);
        let label = if clients > 0 {
            format!("RC {clients}")
        } else {
            "RC".to_owned()
        };
        push1!(label, activity, 55);
    }

    // Repo zone: branch · diff (green/red) · cwd. `⎇` stays — it's the
    // conventional, compact branch marker (the user's own shell prompt uses
    // it); the `Δ` diff prefix is dropped since the +/− colors say "diff".
    if let Some(branch) = app.engine.git_branch.as_deref().filter(|b| !b.is_empty()) {
        push1!(format!("⎇ {}", super::truncate_str(branch, 24)), muted, 70);
    }
    let diff = super::collect_diff_stats(app);
    if diff.total_files > 0 {
        segs.push(StatusSeg {
            spans: vec![
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

    if let Some(badge) = plan_badge(
        app.engine.subscription_type.as_deref(),
        app.engine.seat_tier.as_deref(),
    ) {
        push1!(badge, muted, 40);
    }

    // Unified rate-limit quota badge: the OAuth account snapshot already carries
    // 5h/7d utilization, claim, and overage state — surface the highest-pressure
    // quota so the user sees it climbing *before* getting rejected. Coloured by
    // pressure (amber ≥ 80%, red ≥ 95%).
    if let Some(snapshot) = app.engine.anthropic_account_snapshot.as_ref()
        && let Some((label, pct)) = quota_badge(snapshot)
    {
        let style = if pct >= 95 {
            Style::default().fg(t.error)
        } else if pct >= 80 {
            Style::default().fg(t.warning)
        } else {
            muted
        };
        push1!(label, style, 38);
    }
    if app
        .engine
        .last_session_save_at
        .is_some_and(|t| t.elapsed().as_millis() < 2000)
    {
        push1!("✓ saved".to_owned(), Style::default().fg(t.success), 35);
    }

    // ── Assemble: provider prefix (fixed) · segments · pad · right ──
    let right = " ? help · ^P palette ";
    let total_width = area.width as usize;
    let right_w = super::cell_width(right);
    let avail = total_width.saturating_sub(right_w);

    // Static provider dot — the network EKG (which used to drive a pulse
    // here) is gone; a steady provider-coloured dot just identifies the
    // backend without faking "liveness".
    let dot_color = provider_accent(app.engine.provider.name());

    let prefix_w = 3 + super::cell_width(&provider_badge); // " ● <provider>"
    const SEP_W: usize = 3; // " · "
    let seg_w = |s: &StatusSeg| -> usize {
        s.spans
            .iter()
            .map(|sp| super::cell_width(&sp.content))
            .sum()
    };

    // Drop the lowest-priority segment until the line fits.
    // Drop segments to fit, preserving the always-visible floor. See
    // `fit_segments` for the policy.
    let widths: Vec<usize> = segs.iter().map(|s| SEP_W + seg_w(s)).collect();
    let prios: Vec<u8> = segs.iter().map(|s| s.prio).collect();
    let keep = fit_segments(&prios, &widths, prefix_w, avail);
    let mut keep_iter = keep.iter();
    segs.retain(|_| *keep_iter.next().unwrap_or(&false));

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
                .fg(provider_accent(app.engine.provider.name()))
                .add_modifier(Modifier::BOLD),
        ),
    ];
    // Voice mode indicator — shown when recording or processing.
    match app.voice_state {
        jfc_voice::VoiceState::Recording => {
            spans.push(Span::styled(" · ", muted));
            spans.push(Span::styled(
                "●REC",
                Style::default()
                    .fg(app.theme.error)
                    .add_modifier(ratatui::style::Modifier::BOLD),
            ));
        }
        jfc_voice::VoiceState::Processing => {
            spans.push(Span::styled(" · ", muted));
            spans.push(Span::styled("…STT", Style::default().fg(app.theme.warning)));
        }
        jfc_voice::VoiceState::Idle => {
            // Also show interim transcript if available
            if let Some(ref interim) = app.voice_interim {
                spans.push(Span::styled(" · ", muted));
                let preview = if interim.len() > 40 {
                    format!("{}…", &interim[..37])
                } else {
                    interim.clone()
                };
                spans.push(Span::styled(
                    format!("\"{preview}\""),
                    Style::default().fg(app.theme.text_muted),
                ));
            }
        }
    }

    for s in segs {
        spans.push(Span::styled(" · ", muted));
        spans.extend(s.spans);
    }
    spans.push(Span::styled(" ".repeat(pad), Style::default()));
    spans.push(Span::styled(right, muted));

    // Info line sits below the gauge-divider.
    f.render_widget(
        Paragraph::new(Line::from(spans)).style(Style::default().bg(t.surface)),
        rows[1],
    );
}

pub(super) fn context_gauge_label(used: usize, max: usize, pct: u32) -> String {
    format!(" ctx {}k / {}k · {}% ", used / 1000, max / 1000, pct)
}

pub(super) fn effort_status_badge(app: &App) -> String {
    // ultracode standing mode takes precedence over the raw effort level so the
    // status bar shows the session mode the user enabled.
    if let Some(badge) = app.engine.effort_state.badge() {
        return badge;
    }
    "effort default".to_string()
}

/// Build the rate-limit quota badge from the OAuth account snapshot, or `None`
/// when no utilization is known. Returns `(label, peak_percent)` where the
/// label shows the highest-pressure window (5h or 7d) plus an overage hint, and
/// `peak_percent` drives the colour.
///
/// Examples: `"5h 87%"`, `"7d 95% · overage off"`, `"5h 40%"`.
pub(super) fn quota_badge(
    snapshot: &jfc_engine::providers::anthropic_accounts::AccountSnapshot,
) -> Option<(String, u32)> {
    let pct5 = snapshot.utilization_5h.and_then(utilization_percent);
    let pct7 = snapshot.utilization_7d.and_then(utilization_percent);

    // Pick the window with the higher pressure to surface.
    let (window, pct) = match (pct5, pct7) {
        (Some(a), Some(b)) if b > a => ("7d", b),
        (Some(a), _) => ("5h", a),
        (None, Some(b)) => ("7d", b),
        (None, None) => return None,
    };

    let mut label = format!("{window} {pct}%");
    // If overage is explicitly disabled, the user can't burst past the limit —
    // worth flagging so a rejection isn't a surprise.
    if snapshot.overage_disabled_reason.is_some() {
        label.push_str(" · overage off");
    } else if snapshot.is_using_overage {
        label.push_str(" · overage");
    }
    Some((label, pct))
}

fn utilization_percent(value: f64) -> Option<u32> {
    value
        .is_finite()
        .then(|| (value.clamp(0.0, 1.0) * 100.0).round() as u32)
}

/// Render the Anthropic plan/seat badge for the status bar, or `None` when no
/// subscription info is known.
///
/// The OAuth profile reports `subscription_type` as a lowercase id
/// (`"max"`, `"pro"`, `"team"`, `"enterprise"`). Rendered bare next to the
/// reasoning-effort badge (`effort high`) the lowercase `max` was read as a
/// *second effort level* — the user reported the footer "showing high and
/// max". Title Case (`Max`) plus the effort badge keeping its `effort ` prefix
/// (`effort max`) keeps the subscription tier distinct from the effort knob,
/// so the plan no longer needs a `◆` brand glyph.
pub(super) fn plan_badge(subscription: Option<&str>, seat: Option<&str>) -> Option<String> {
    let plan = subscription.map(pretty_plan_name);
    match (plan, seat) {
        // A real subscription is branded with `◆` + Title Case so the plan
        // (`◆ Max`) can't be misread as a second reasoning-effort level next
        // to `effort high`. A bare seat id is internal — left unbranded.
        (Some(plan), Some(seat)) => Some(format!("◆ {plan}·{seat}")),
        (Some(plan), None) => Some(format!("◆ {plan}")),
        (None, Some(seat)) => Some(seat.to_owned()),
        (None, None) => None,
    }
}

/// Title-case the known Anthropic plan ids; pass anything unrecognized through
/// unchanged so a new plan name still renders (just without our casing).
fn pretty_plan_name(subscription: &str) -> String {
    match subscription {
        "max" => "Max".to_owned(),
        "pro" => "Pro".to_owned(),
        "team" => "Team".to_owned(),
        "enterprise" => "Enterprise".to_owned(),
        "free" => "Free".to_owned(),
        other => other.to_owned(),
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
/// Provider brand accent for the status dot + badge. Delegates to the
/// canonical, fully-populated map in `model_picker` rather than keeping a
/// second divergent subset (this used to only know 4 providers and fell back
/// to gray for openai/gemini/litellm/…). Brand colors are identity, not
/// theme-tinted, so they intentionally live in code, not the Theme struct.
pub(super) fn provider_accent(provider: &str) -> Color {
    super::model_picker::provider_color(provider)
}

/// Priority at/above which a status segment is part of the always-visible
/// floor: model identity, running cost, and genuine alerts (MCP down,
/// degraded/unreachable status, pending approvals).
pub(super) const STATUS_FLOOR_PRIO: u8 = 90;

/// Decide which status segments survive in `avail` columns. Returns a keep
/// mask parallel to `prios`/`widths`.
///
/// Policy: while the kept segments don't fit, drop the lowest-priority
/// segment *below the floor* first — so narrow terminals shed context
/// (cwd, branch, plan, effort, mode flags) before they ever touch the floor
/// (`prio >= STATUS_FLOOR_PRIO`). Only once nothing below the floor remains
/// do we drop the lowest floor segment, as a last resort on an extremely
/// narrow width (better to truncate than render past the edge).
pub(super) fn fit_segments(
    prios: &[u8],
    widths: &[usize],
    prefix_w: usize,
    avail: usize,
) -> Vec<bool> {
    let mut keep = vec![true; prios.len()];
    loop {
        let used: usize = (0..prios.len())
            .filter(|&i| keep[i])
            .map(|i| widths[i])
            .sum();
        if prefix_w + used <= avail {
            break;
        }
        // Lowest-priority kept segment below the floor; else lowest kept overall.
        let pick = |floor_only: bool| {
            (0..prios.len())
                .filter(|&i| keep[i] && (!floor_only || prios[i] < STATUS_FLOOR_PRIO))
                .min_by_key(|&i| prios[i])
        };
        match pick(true).or_else(|| pick(false)) {
            Some(i) => keep[i] = false,
            None => break,
        }
    }
    keep
}

#[cfg(test)]
mod tests {
    use super::*;
    use jfc_engine::providers::anthropic_accounts::AccountSnapshot;

    fn snap(u5: Option<f64>, u7: Option<f64>) -> AccountSnapshot {
        AccountSnapshot {
            utilization_5h: u5,
            utilization_7d: u7,
            ..Default::default()
        }
    }

    #[test]
    fn quota_badge_picks_higher_pressure_window_normal() {
        let (label, pct) = quota_badge(&snap(Some(0.40), Some(0.95))).unwrap();
        assert_eq!(pct, 95);
        assert!(label.starts_with("7d 95%"), "got {label}");
    }

    #[test]
    fn quota_badge_uses_5h_when_higher_normal() {
        let (label, pct) = quota_badge(&snap(Some(0.87), Some(0.20))).unwrap();
        assert_eq!(pct, 87);
        assert!(label.starts_with("5h 87%"), "got {label}");
    }

    #[test]
    fn quota_badge_clamps_over_100_robust() {
        let (label, pct) = quota_badge(&snap(Some(1.01), None)).unwrap();
        assert_eq!(pct, 100);
        assert!(label.starts_with("5h 100%"), "got {label}");
    }

    #[test]
    fn quota_badge_none_without_utilization_robust() {
        assert!(quota_badge(&snap(None, None)).is_none());
    }

    #[test]
    fn quota_badge_flags_overage_off_normal() {
        let mut s = snap(Some(0.99), None);
        s.overage_disabled_reason = Some("out_of_credits".into());
        let (label, _) = quota_badge(&s).unwrap();
        assert!(label.contains("overage off"), "got {label}");
    }
}

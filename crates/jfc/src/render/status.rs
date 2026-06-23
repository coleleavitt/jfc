use ratatui::{
    Frame,
    layout::Rect,
    style::{Modifier, Style},
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
    if area.height == 0 || area.width == 0 {
        return;
    }
    let t = app.theme;
    let ui_tokens = t.claude_ui_tokens();
    let muted = Style::default().fg(t.text_muted);
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

    // The footer is now a single contextual action row, not a dashboard. Keep
    // it quiet until something is actionable or under pressure.
    if let Some((symbol, label)) = match app.engine.permission_mode {
        crate::app::PermissionMode::Default => None,
        crate::app::PermissionMode::Plan => Some(("◇", "plan mode on")),
        crate::app::PermissionMode::AcceptEdits => Some(("⏵⏵", "accept edits on")),
        crate::app::PermissionMode::BypassPermissions => Some(("!!", "bypass permissions on")),
        crate::app::PermissionMode::Auto => Some(("⏵⏵", "auto mode on")),
    } {
        push1!(
            format!("{symbol} {label}"),
            alert.add_modifier(Modifier::BOLD),
            88
        );
    }

    // Interaction-mode chip — only when the user set one explicitly via `/imode`
    // (the silent `Code` default shows nothing, so the bar is unchanged by
    // default). Uses the calm activity color, like other mode flags.
    if let Some(mode) = app.engine.interaction_mode {
        push1!(format!("[{}]", mode.label().to_lowercase()), activity, 60);
    }

    let safe_mode = jfc_engine::config::safe_mode_enabled();
    if safe_mode {
        push1!(
            "safe mode".to_owned(),
            alert.add_modifier(Modifier::BOLD),
            97
        );
    }
    let managed = jfc_engine::config::load_managed_settings();
    let plugins_disabled = safe_mode
        || managed
            .as_ref()
            .is_some_and(|m| m.disable_plugin_dirs || m.disable_plugin_urls);
    if plugins_disabled {
        push1!("plugins off".to_owned(), muted, 62);
    }
    // Problems / actionable state get the highest priority.
    let mcp_down: Vec<&str> = app
        .engine
        .mcp_servers
        .iter()
        .filter(|s| matches!(s.status, jfc_core::McpStatus::Error))
        .map(|s| s.name.as_str())
        .collect();
    if !mcp_down.is_empty() {
        push1!(
            format!("MCP issue: {} · /doctor", mcp_down.join(", ")),
            Style::default().fg(t.error).add_modifier(Modifier::BOLD),
            98,
        );
    }
    if let Some(status) = app.engine.claude_status.as_ref() {
        if status.is_degraded() {
            push1!(status.short_badge(), alert, 96);
        }
    } else if app.engine.claude_status_error.is_some() {
        push1!(
            "status unreachable".to_owned(),
            Style::default().fg(t.error),
            96
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
            format!("{approval_count} awaiting approval"),
            alert.add_modifier(Modifier::BOLD),
            94,
        );
    }
    if app.leader_key_active {
        push1!("leader key".to_owned(), Style::default().fg(t.accent), 90);
    } else if app.viewing_task_id.is_some() {
        push1!("agent view".to_owned(), Style::default().fg(t.accent), 86);
    }
    if !app.engine.queued_prompts.is_empty() {
        push1!(
            format!("{} queued", app.engine.queued_prompts.len()),
            muted,
            80
        );
    }

    if app.engine.fast_mode {
        push1!("fast".to_owned(), activity, 60);
    }
    if app.engine.effort_state.current.is_some() {
        push1!(effort_status_badge(app), muted, 45);
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

    if let Some(branch) = app.engine.git_branch.as_deref().filter(|b| !b.is_empty()) {
        push1!(
            format!("git {}", super::truncate_str(branch, 24)),
            muted,
            35
        );
    }
    let diff = super::collect_diff_stats(app);
    if diff.total_files > 0 {
        segs.push(StatusSeg {
            spans: vec![
                Span::styled(
                    format!("+{}", diff.additions),
                    Style::default().fg(ui_tokens.diff_added),
                ),
                Span::styled(" ", muted),
                Span::styled(
                    format!("−{}", diff.deletions),
                    Style::default().fg(ui_tokens.diff_removed),
                ),
            ],
            prio: 50,
        });
    }

    let used = app.engine.tool_ctx.approx_tokens;
    let max = app.engine.max_context_tokens.max(1);
    let ctx_pct = ((used as f64 / max as f64).clamp(0.0, 1.0) * 100.0).round() as u32;
    if ctx_pct >= 70 {
        let style = if ctx_pct >= 90 {
            Style::default().fg(t.error)
        } else {
            Style::default().fg(t.warning)
        };
        push1!(format!("ctx {ctx_pct}%"), style, 91);
    }

    if let Some(snapshot) = app.engine.anthropic_account_snapshot.as_ref()
        && let Some((label, pct)) = quota_badge(snapshot)
        && (pct >= 80 || label.contains("overage"))
    {
        let style = if pct >= 95 {
            Style::default().fg(t.error)
        } else if pct >= 80 {
            Style::default().fg(t.warning)
        } else {
            muted
        };
        push1!(label, style, 89);
    }
    if app
        .engine
        .last_session_save_at
        .is_some_and(|t| t.elapsed().as_millis() < 2000)
    {
        push1!("saved".to_owned(), Style::default().fg(t.success), 40);
    }

    const SEP_W: usize = 3; // " · "
    let seg_w = |s: &StatusSeg| -> usize {
        s.spans
            .iter()
            .map(|sp| super::cell_width(&sp.content))
            .sum()
    };

    if segs.is_empty() {
        push1!("? help".to_owned(), muted, 10);
    }

    let widths: Vec<usize> = segs.iter().map(|s| SEP_W + seg_w(s)).collect();
    let prios: Vec<u8> = segs.iter().map(|s| s.prio).collect();
    let keep = fit_segments(&prios, &widths, 0, area.width as usize);
    let mut keep_iter = keep.iter();
    segs.retain(|_| *keep_iter.next().unwrap_or(&false));

    let mut spans: Vec<Span> = vec![Span::raw("  ")];
    // Voice mode indicator — shown when recording or processing. The live RMS
    // animation lives at the input cursor (see `input_box`); here we keep a
    // plain textual label so there's always a clear indicator even when the
    // cursor is scrolled out of view.
    match app.voice_state {
        jfc_voice::VoiceState::Recording => {
            spans.push(Span::styled(
                "recording",
                Style::default()
                    .fg(app.theme.error)
                    .add_modifier(ratatui::style::Modifier::BOLD),
            ));
        }
        jfc_voice::VoiceState::Processing => {
            // Pulsing gray "…STT" — port of CC 2.1.177's processing indicator.
            let elapsed = app
                .voice_record_started
                .map(|t| t.elapsed().as_millis())
                .unwrap_or(0);
            let pulse = crate::render::voice_cursor::processing_pulse(elapsed);
            spans.push(Span::styled("transcribing", Style::default().fg(pulse)));
        }
        jfc_voice::VoiceState::Idle => {
            // Also show interim transcript if available
            if let Some(ref interim) = app.voice_interim {
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

    let had_voice = spans.len() > 1;
    for (idx, s) in segs.into_iter().enumerate() {
        if idx > 0 || had_voice {
            spans.push(Span::styled(" · ", muted));
        }
        if !had_voice && idx == 0 {
            // The leading two spaces already provide the input/footer inset.
        }
        spans.extend(s.spans);
    }

    f.render_widget(
        Paragraph::new(Line::from(spans)).style(Style::default().bg(t.bg)),
        area,
    );
}

#[cfg(test)]
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
#[cfg(test)]
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
#[cfg(test)]
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

/// Priority at/above which a status segment is part of the always-visible
/// floor: genuine alerts (MCP down, degraded/unreachable status, pending
/// approvals, high context pressure).
pub(super) const STATUS_FLOOR_PRIO: u8 = 90;

/// Decide which status segments survive in `avail` columns. Returns a keep
/// mask parallel to `prios`/`widths`.
///
/// Policy: while the kept segments don't fit, drop the lowest-priority
/// segment *below the floor* first — so narrow terminals shed context
/// (branch, effort, routine hints) before they ever touch the floor
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

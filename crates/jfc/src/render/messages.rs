use super::agents::{render_subagent_tree, render_teammate_tree};
use super::visual::*;
use super::*;

// Re-export from message_view — the canonical definitions now live there.
pub(crate) use crate::message_view::task_body::task_view_body_lines;

fn modal_overlay_locks_scroll(app: &App) -> bool {
    app.palette.visible
        || app.model_picker.visible
        || app.session_picker.visible
        || app.task_panel.visible
        || matches!(
            app.task_panel.expanded_view,
            crate::app::ExpandedView::Tasks
        )
        || matches!(
            app.task_panel.expanded_view,
            crate::app::ExpandedView::Teammates
        )
        || app.show_help
        || (app.show_diagnostic_panel && !app.engine.diagnostics.is_empty())
        || app.engine.pending_approval.is_some()
        || app.engine.pending_question.is_some()
        || !app.engine.pending_elicitations.is_empty()
        || app.pending_rewrite_proposal.is_some()
}

fn should_snap_to_bottom(app: &App, visible: usize, total_lines: usize) -> bool {
    !modal_overlay_locks_scroll(app)
        && (app.follow_bottom || app.scroll_offset + visible > total_lines)
}

fn context_window_label(tokens: usize) -> String {
    if tokens >= 1_000_000 {
        let whole = tokens / 1_000_000;
        if tokens.is_multiple_of(1_000_000) {
            format!("{whole}M context")
        } else {
            let tenth = (tokens % 1_000_000) / 100_000;
            format!("{whole}.{tenth}M context")
        }
    } else if tokens >= 1_000 {
        format!("{}k context", tokens / 1_000)
    } else {
        format!("{tokens} context")
    }
}

fn stream_quiet_duration(
    phase: crate::spinner::SpinnerPhase,
    lifecycle_visible: bool,
    last_token_at: Option<std::time::Instant>,
    now: std::time::Instant,
) -> std::time::Duration {
    if lifecycle_visible
        || !matches!(
            phase,
            crate::spinner::SpinnerPhase::Thinking | crate::spinner::SpinnerPhase::Responding
        )
    {
        return std::time::Duration::default();
    }
    last_token_at
        .map(|last| now.duration_since(last))
        .unwrap_or_default()
}

fn pretty_model_label(model: &str) -> String {
    let short = model.strip_prefix("claude-").unwrap_or(model);
    for (prefix, label) in [
        ("opus-", "Opus"),
        ("sonnet-", "Sonnet"),
        ("haiku-", "Haiku"),
    ] {
        if let Some(rest) = short.strip_prefix(prefix) {
            let version = rest
                .split('-')
                .take_while(|part| part.chars().all(|c| c.is_ascii_digit()))
                .collect::<Vec<_>>()
                .join(".");
            if !version.is_empty() {
                return format!("{label} {version}");
            }
        }
    }
    truncate_str(model, 36)
}

fn provider_label(provider: &str) -> String {
    match provider {
        "anthropic-oauth" => "OAuth".to_string(),
        "anthropic" => "Anthropic".to_string(),
        "openai" => "OpenAI".to_string(),
        "gemini" => "Gemini".to_string(),
        "openwebui" => "OpenWebUI".to_string(),
        other => other.to_string(),
    }
}

fn effort_header_label(app: &App) -> String {
    if let Some(badge) = app.engine.effort_state.badge() {
        if let Some(level) = badge.strip_prefix("effort ") {
            format!("{level} effort")
        } else {
            badge
        }
    } else {
        "default effort".to_string()
    }
}

fn claude_logo_row(row: usize, t: Theme) -> Vec<Span<'static>> {
    let flag_red = ratatui::style::Color::Rgb(217, 0, 18);
    let flag_blue = ratatui::style::Color::Rgb(0, 51, 160);
    let flag_orange = ratatui::style::Color::Rgb(242, 168, 0);
    let face_style = Style::default().fg(flag_red);
    let body_style = Style::default().fg(flag_blue);
    let foot_style = Style::default().fg(flag_orange);
    let hat_style = Style::default().fg(flag_red).bg(t.bg);
    let orange_style = Style::default()
        .fg(flag_orange)
        .bg(t.bg)
        .add_modifier(Modifier::BOLD);
    let weave_blue_style = Style::default()
        .fg(flag_blue)
        .bg(t.bg)
        .add_modifier(Modifier::BOLD);
    match row {
        0 => vec![
            Span::raw("  "),
            Span::styled("▟", hat_style),
            Span::styled("╳", orange_style),
            Span::styled("◇", weave_blue_style),
            Span::styled("╳", orange_style),
            Span::styled("▙", hat_style),
            Span::raw("  "),
        ],
        1 => vec![
            Span::styled(" ▐", face_style),
            Span::styled("▛███▜", face_style),
            Span::styled("▌", face_style),
            Span::raw(" "),
        ],
        2 => vec![Span::styled("▝▜█████▛▘", body_style)],
        _ => vec![
            Span::raw("  "),
            Span::styled("▘▘", foot_style),
            Span::raw(" "),
            Span::styled("▝▝", foot_style),
            Span::raw("  "),
        ],
    }
}

fn with_claude_logo(row: usize, t: Theme, mut text_spans: Vec<Span<'static>>) -> Line<'static> {
    let mut spans = claude_logo_row(row, t);
    spans.push(Span::raw("  "));
    spans.append(&mut text_spans);
    Line::from(spans)
}

fn empty_session_lines(app: &App, t: Theme, width: u16) -> Vec<Line<'static>> {
    let provider = app.engine.provider.name();
    let setup_issues = app
        .engine
        .mcp_servers
        .iter()
        .filter(|server| matches!(server.status, jfc_core::McpStatus::Error))
        .count();
    let title_spans = vec![
        Span::styled(
            "JFC",
            Style::default()
                .fg(t.text_primary)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!(" v{}", env!("CARGO_PKG_VERSION")),
            Style::default().fg(t.text_muted),
        ),
    ];
    let model_spans = vec![Span::styled(
        format!(
            "{} ({}) with {} · {}",
            pretty_model_label(app.engine.model.as_str()),
            context_window_label(app.engine.max_context_tokens),
            effort_header_label(app),
            provider_label(provider),
        ),
        Style::default().fg(t.text_secondary),
    )];
    let cwd_spans = vec![Span::styled(
        truncate_str(&app.engine.cwd, 72),
        Style::default().fg(t.text_secondary),
    )];

    let mut lines = if width >= 70 {
        vec![
            with_claude_logo(0, t, title_spans),
            with_claude_logo(1, t, model_spans),
            with_claude_logo(2, t, cwd_spans),
            with_claude_logo(3, t, Vec::new()),
            Line::from(""),
        ]
    } else {
        vec![
            Line::from(title_spans),
            Line::from(model_spans),
            Line::from(cwd_spans),
            Line::from(""),
        ]
    };

    if setup_issues > 0 {
        let issue_word = if setup_issues == 1 { "issue" } else { "issues" };
        lines.push(Line::from(vec![
            Span::styled(
                format!("{setup_issues} setup {issue_word}: "),
                Style::default().fg(t.warning).add_modifier(Modifier::BOLD),
            ),
            Span::styled("MCP", Style::default().fg(t.warning)),
            Span::styled(" · /doctor", Style::default().fg(t.text_muted)),
        ]));
        lines.push(Line::from(""));
    }

    lines.extend([
        Line::from(Span::styled(
            "What can I help you with?",
            Style::default().fg(t.text_muted),
        )),
        Line::from(""),
        Line::from(Span::styled(
            "  ?    keybindings",
            Style::default().fg(t.text_muted),
        )),
        Line::from(Span::styled(
            "  Ctrl+P    palette · Ctrl+M    model picker",
            Style::default().fg(t.text_muted),
        )),
    ]);
    lines
}

pub(super) fn messages(f: &mut Frame, app: &mut App, area: Rect) {
    use crate::message_view::MessageView;
    use ratatui::widgets::Widget;

    // Record area for the mouse handler (drag-scroll target).
    *app.messages_rect.borrow_mut() = Some(area);
    let t = app.theme;

    if let Some(ref task_id) = app.task_panel.viewing_task_id.clone() {
        messages_task_view(f, app, area, task_id);
        return;
    }

    // Total horizontal overhead for the borderless transcript is just the
    // block padding (1 left + 1 right). Keep the measured width and the
    // MessageView render width identical so scroll/follow-bottom math cannot
    // drift from wrapping.
    // The transcript is flat — no box border — so it costs no rows/cols beyond
    // padding (region separators are the flat-top docks below, not a frame
    // around the messages).
    let inner_width = area.width.saturating_sub(2) as usize;

    // Build render items ONCE per frame and share them with `MessageView::render`.
    // Pre-fix this function called `message_view_total_lines` (one
    // `build_render_items` walk) and the widget then ran `build_render_items`
    // again — gdb sampling showed the second walk's `Vec<Line<'static>>::to_vec`
    // out of `RenderCache` was the dominant remaining hot spot once syntect/onig
    // and the highlighted tool-height path stopped building styled line Vecs.
    // Sharing one items vec halves the per-frame deep-clone work.
    //
    // The earlier `app.total_lines` cache that gated `message_view_total_lines`
    // is no longer needed — items are required for paint anyway, and
    // `tool_block_height` is now a deterministic row-count query instead of a
    // second renderer.
    let render_ctx = crate::message_view::RenderCtx::from_app(app);

    // Virtualized transcript: the persistent height index revalidates
    // per-message fingerprints (cheap integer compares) and re-measures only
    // changed messages — `total_lines` comes from its prefix sums instead of
    // a full build_render_items walk. Items are then built ONLY for the
    // messages intersecting the visible window, so per-frame work is
    // O(window), not O(transcript).
    let total_lines = app.height_index.borrow_mut().sync(&render_ctx, inner_width);

    // No top/bottom border rows — content fills the full height.
    let visible = area.height as usize;

    // Compute the new scroll offset locally — `items` borrows from `app`, so we
    // can't write `app.scroll_offset` until after `MessageView::render` consumes
    // them. The new value is also passed into `PrebuiltItems` so the widget
    // sees it during paint instead of the (still-old) `app.scroll_offset`.
    let scroll_before = app.scroll_offset;
    let new_scroll_offset = if should_snap_to_bottom(app, visible, total_lines) {
        total_lines.saturating_sub(visible)
    } else {
        app.scroll_offset.min(total_lines.saturating_sub(visible))
    };

    // Build items for the visible message window only. `window_top` is the
    // absolute row of the first windowed message's top edge; the widget
    // receives a window-relative scroll so its skip math lines up.
    let (win_first, win_last, window_top) = {
        let index = app.height_index.borrow();
        index.window(new_scroll_offset, visible)
    };
    let win_prev_role = app.height_index.borrow().prev_role_before(win_first);
    let items = crate::message_view::build_render_items_window(
        &render_ctx,
        inner_width,
        win_first,
        win_last,
        win_prev_role,
    );
    let window_scroll = new_scroll_offset.saturating_sub(window_top);
    // Trace the scroll math result. Bug class this catches: when
    // `total_lines` is undercounted (width mismatch), `scroll_offset`
    // gets pinned to a value smaller than the true bottom row,
    // leaving the latest content offscreen. Compare `total_lines`
    // here against actual rendered height to spot off-by-N errors.
    tracing::trace!(
        target: "jfc::render::scroll",
        inner_width,
        total_lines,
        visible,
        scroll_before,
        scroll_after = new_scroll_offset,
        follow_bottom = app.follow_bottom,
        "messages scroll math"
    );

    // Flat transcript: no box border, just 1-cell horizontal padding so prose
    // doesn't kiss the edge. Region separation comes from the flat-top docks
    // below (input/spinner), not a frame around the messages — matching Claude
    // Code & codex, which leave the transcript open.
    //
    // (Earlier this was a rounded border that also pulsed `border ↔ accent`
    // while streaming; both the pulse and the box are gone now.)
    let block = Block::default()
        .padding(Padding::horizontal(1))
        .style(Style::default().bg(t.bg));

    let inner = block.inner(area);
    f.render_widget(block, area);

    // Snapshot the values we'll need to commit back to App after `items` is
    // dropped. The placeholder branch doesn't consume `items`, so we commit
    // *after* the if/else with an explicit `drop(items)`.
    let totals_to_commit = (
        total_lines,
        (
            app.engine.messages.len(),
            app.engine.streaming_text.len(),
            inner_width,
        ),
        visible,
        new_scroll_offset,
    );

    if app.engine.messages.is_empty() && app.engine.streaming_text.is_empty() {
        let placeholder = Paragraph::new(empty_session_lines(app, t, inner.width))
            .style(Style::default().bg(t.bg));
        f.render_widget(placeholder, inner);
    } else {
        // The widget walks the items it's handed and skips `scroll` rows
        // from the FIRST item — with windowed items that must be the
        // window-relative offset, not the absolute transcript offset.
        MessageView {
            app,
            prebuilt: Some(crate::message_view::PrebuiltItems {
                items,
                total_h: total_lines,
                scroll: window_scroll,
            }),
        }
        .render(inner, f.buffer_mut());

        // (The "token rain" border cell that pulsed on each arriving token
        // lived here — removed. It faked liveness in the corner of the
        // frame; the spinner row already reports real token flow.)
    }

    // Commit the freshly-computed values back to App. By this point both
    // branches above have finished rendering and any borrow of `app` via the
    // items vec is dropped. `App::max_scroll` (used by event-loop key
    // handlers) reads these — staling them by a frame caused PgDn at end-of-
    // buffer to silently no-op while still feeling laggy.
    let (total_lines_v, total_lines_key_v, viewport_h_v, scroll_v) = totals_to_commit;
    app.total_lines = total_lines_v;
    app.total_lines_key = total_lines_key_v;
    app.viewport_height = viewport_h_v;
    app.scroll_offset = scroll_v;

    // "While you were away" recap band, drawn over the top of the transcript
    // on return from a long autonomous run. Ephemeral + dismissable (Esc /
    // next submit), so overlaying the top few rows is fine — no scroll-math
    // entanglement.
    if let Some(recap) = app.away_recap.as_deref() {
        away_recap_band(f, t, inner, recap);
    }
}

/// Render the away-recap as a left-ribboned band over the top of `inner`.
/// First line is the bold "While you were away" header; the rest are muted
/// detail rows. A solid surface bg keeps it readable over the transcript.
fn away_recap_band(f: &mut Frame, t: crate::theme::Theme, inner: Rect, recap: &str) {
    let recap_lines: Vec<&str> = recap.lines().collect();
    if recap_lines.is_empty() || inner.height == 0 {
        return;
    }
    let h = (recap_lines.len() as u16).min(inner.height);
    let band = Rect { height: h, ..inner };
    let lines: Vec<Line<'static>> = recap_lines
        .iter()
        .enumerate()
        .take(h as usize)
        .map(|(i, l)| {
            let body_style = if i == 0 {
                Style::default().fg(t.accent).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(t.text_secondary)
            };
            Line::from(vec![
                Span::styled("▌ ", Style::default().fg(t.accent)),
                Span::styled((*l).to_string(), body_style),
            ])
        })
        .collect();
    f.render_widget(
        Paragraph::new(lines).style(Style::default().bg(t.surface)),
        band,
    );
}

/// Build a `Line` for one agent row inside the workflow detail panel.
fn workflow_agent_line(
    agent: &jfc_engine::workflows::task::AgentProgress,
    in_current_phase: bool,
    frame: usize,
    t: Theme,
) -> Line<'static> {
    use jfc_engine::workflows::task::AgentStatus;
    let bullet = if in_current_phase { "●" } else { "○" };
    let (status_glyph, status_color) = match agent.status {
        AgentStatus::Running => {
            let ch = crate::app::SPINNER[frame % crate::app::SPINNER.len()];
            (ch.to_string(), t.accent)
        }
        AgentStatus::Done => ("✓".to_string(), t.success),
        AgentStatus::Failed => ("✗".to_string(), t.error),
        AgentStatus::Queued => ("○".to_string(), t.text_muted),
        AgentStatus::Skipped => ("–".to_string(), t.text_muted),
    };
    let label = truncate_str(&agent.label, 42);
    let status_str = agent.status.to_string();
    // Right-pad label to fixed width so status aligns.
    let pad = 44usize.saturating_sub(label.chars().count());
    let padded_label = format!("{}{}", label, " ".repeat(pad));
    Line::from(vec![
        Span::styled(
            format!("  {} #{:<2} ", bullet, agent.index),
            Style::default().fg(t.text_muted),
        ),
        Span::styled(padded_label, t.style_text_primary),
        Span::styled(status_str, Style::default().fg(status_color)),
        Span::raw(" "),
        Span::styled(
            status_glyph,
            Style::default()
                .fg(status_color)
                .add_modifier(Modifier::BOLD),
        ),
    ])
}

/// Render the workflow progress detail panel (called when `bt.workflow_progress.is_some()`).
fn render_workflow_detail(
    f: &mut Frame,
    area: Rect,
    bt: &crate::app::BackgroundTask,
    t: Theme,
    scroll_offset: usize,
    visible: usize,
) {
    let wfp = match bt.workflow_progress.as_ref() {
        Some(w) => w,
        None => return,
    };

    let frame = {
        use std::time::{SystemTime, UNIX_EPOCH};
        let ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);
        (ms / 80) as usize
    };

    let elapsed_s = wfp.started_at.elapsed().as_secs();
    let running_count = wfp.running_count();

    // Header line: "workflow: <name> · <status> · N agents · K cache hits · Xs"
    let header = format!(
        " workflow: {} · {} · {} agent{} · {} cache hit{} · {}s",
        wfp.meta.name,
        wfp.status,
        running_count,
        if running_count == 1 { "" } else { "s" },
        wfp.cache_hits,
        if wfp.cache_hits == 1 { "" } else { "s" },
        elapsed_s,
    );

    let divider = "─".repeat(area.width.saturating_sub(2) as usize);

    let mut lines: Vec<Line<'static>> = Vec::new();
    lines.push(Line::from(Span::styled(header, t.style_accent_bold)));
    lines.push(Line::from(Span::styled(
        divider.clone(),
        Style::default().fg(t.border),
    )));

    // Phase header
    if let Some(ref phase_name) = wfp.current_phase {
        lines.push(Line::from(vec![
            Span::styled(" Phase: ", Style::default().fg(t.text_muted)),
            Span::styled(phase_name.clone(), t.style_text_primary_bold),
            Span::styled(
                "  (current)",
                Style::default()
                    .fg(t.text_muted)
                    .add_modifier(Modifier::ITALIC),
            ),
        ]));
    }

    // Agent rows
    for agent in &wfp.agents {
        let in_current = wfp.current_phase.as_deref() == agent.phase.as_deref();
        lines.push(workflow_agent_line(agent, in_current, frame, t));
    }

    lines.push(Line::from(Span::styled(
        divider,
        Style::default().fg(t.border),
    )));
    lines.push(Line::from(Span::styled(
        " Logs:",
        Style::default().fg(t.text_muted),
    )));
    for entry in &wfp.logs {
        lines.push(Line::from(vec![
            Span::raw("   "),
            Span::styled(entry.clone(), Style::default().fg(t.text_muted)),
        ]));
    }
    if wfp.logs.is_empty() {
        lines.push(Line::from(Span::styled(
            "   (none)",
            Style::default().fg(t.text_muted),
        )));
    }

    let total = lines.len();
    let scroll = scroll_offset.min(total.saturating_sub(visible));
    let para = Paragraph::new(lines)
        .style(Style::default().bg(t.bg))
        .scroll((scroll as u16, 0));
    f.render_widget(para, area);
}

pub(super) fn messages_task_view(f: &mut Frame, app: &mut App, area: Rect, task_id: &str) {
    let t = app.theme;
    // Flat dock: a single TOP divider, not a full box — so no L/R borders.
    // MessageView has no internal gutter; estimate with the same width it
    // renders at.
    let inner_width = area.width as usize;

    let (title_str, body_lines, use_message_view) = match app.engine.background_tasks.get(task_id) {
        None => (format!("task {task_id} (not found)"), Vec::new(), false),
        Some(bt) => {
            let title = format!(
                " {} · {} ",
                &bt.task_id.as_str()[..bt.task_id.as_str().len().min(12)],
                bt.description
            );
            // Use the rich MessageView pipeline when we have structured messages.
            // Fall back to the markdown string renderer for tasks that have no
            // chat_messages yet (e.g. daemon-launched detached agents whose events
            // only arrive as TaskProgress strings).
            let use_mv = !bt.chat_messages.is_empty();
            if use_mv {
                (title, Vec::new(), true)
            } else {
                static EMPTY: std::sync::OnceLock<std::collections::HashSet<usize>> =
                    std::sync::OnceLock::new();
                let empty = EMPTY.get_or_init(std::collections::HashSet::new);
                let expanded = app
                    .task_panel
                    .viewing_expanded
                    .get(task_id)
                    .unwrap_or(empty);
                let task_done = matches!(bt.status, jfc_core::TaskLifecycle::Completed);
                // Lead with the canonical agent-detail body (render/roster.rs)
                // — the same Progress/last-tool/model header the Tasks detail
                // pane shows — so drilling into a detached agent reads the
                // same as inspecting it from the task panel.
                let mut lines = super::roster::agent_detail_lines(bt, &t, area.width);
                lines.push(Line::from(""));
                lines.extend(task_view_body_lines(
                    &bt.messages,
                    expanded,
                    &t,
                    inner_width,
                    task_done,
                ));
                (title, lines, false)
            }
        }
    };

    let block = Block::default()
        .borders(Borders::TOP)
        .border_style(t.style_accent)
        .title(Span::styled(title_str, t.style_accent_bold))
        .style(Style::default().bg(t.bg));

    let inner = block.inner(area);
    f.render_widget(block, area);

    let task_status = app.engine.background_tasks.get(task_id).map(|bt| bt.status);
    let task_is_running = matches!(task_status, Some(jfc_core::TaskLifecycle::Running));
    let task_is_idle = matches!(task_status, Some(jfc_core::TaskLifecycle::Idle));

    // While the task is still running, append a spinner+"Receiving…"
    // row so the user can tell at a glance that more output is on
    // the way (vs. a frozen panel). The frame index pulls from the
    // same wall-clock source as `tool_status_icon_animated` so the
    // glyph rotates in lockstep with the running-tool bullet.
    //
    // For Idle teammates, swap the live spinner for a static "⏸ idle"
    // hint so the user can tell the difference between "still
    // streaming" and "agent finished its turn, waiting for next ping"
    // without staring at the panel for a few seconds.
    let visible = inner.height as usize;

    // Workflow tasks get a dedicated progress panel.
    let has_workflow = app
        .engine
        .background_tasks
        .get(task_id)
        .map(|bt| bt.workflow_progress.is_some())
        .unwrap_or(false);
    if has_workflow {
        if let Some(bt) = app.engine.background_tasks.get(task_id) {
            let scroll = app.scroll_offset;
            render_workflow_detail(f, inner, bt, t, scroll, visible);
        }
        return;
    }

    if use_message_view {
        // Rich MessageView path — same pipeline as the main chat.
        use crate::message_view::{MessageView, PrebuiltItems, RenderCtx, build_render_items_ctx};
        use ratatui::widgets::Widget;

        let chat_msgs = app
            .engine
            .background_tasks
            .get(task_id)
            .map(|bt| bt.chat_messages.as_slice())
            .unwrap_or(&[]);

        // Compute scroll BEFORE borrowing app through items, then assign after.
        let total_lines_est = {
            let msgs = app
                .engine
                .background_tasks
                .get(task_id)
                .map(|bt| bt.chat_messages.as_slice())
                .unwrap_or(&[]);
            let ctx = RenderCtx::from_task(msgs, app);
            let est_items = build_render_items_ctx(&ctx, inner_width);
            est_items
                .iter()
                .map(|i| i.height_with_app(inner_width, Some(app)))
                .sum::<usize>()
        };
        let new_scroll = if should_snap_to_bottom(app, visible, total_lines_est) {
            total_lines_est.saturating_sub(visible)
        } else {
            app.scroll_offset
                .min(total_lines_est.saturating_sub(visible))
        };
        app.scroll_offset = new_scroll;
        app.total_lines = total_lines_est;
        app.viewport_height = visible;

        // Now build items for real (same data, but app.scroll_offset is now settled).
        let ctx = RenderCtx::from_task(chat_msgs, app);
        let items = build_render_items_ctx(&ctx, inner_width);
        let mv = MessageView {
            app,
            prebuilt: Some(PrebuiltItems {
                items,
                total_h: total_lines_est,
                scroll: new_scroll,
            }),
        };
        mv.render(inner, f.buffer_mut());

        // Spinner / idle hint: paint it below the MessageView content
        // in whatever space remains (or overlap the last row if full).
        if task_is_running || task_is_idle {
            let frame = (app.launched_at.elapsed().as_millis() / 80) as usize;
            let hint_line = if task_is_running {
                let spinner_glyph = crate::app::SPINNER[frame % crate::app::SPINNER.len()];
                Line::from(vec![
                    Span::styled(
                        spinner_glyph.to_string(),
                        Style::default().fg(t.accent).add_modifier(Modifier::BOLD),
                    ),
                    Span::raw("  "),
                    Span::styled("Receiving output…", Style::default().fg(t.text_muted)),
                ])
            } else {
                Line::from(vec![
                    Span::styled("⏸  ", Style::default().fg(t.text_muted)),
                    Span::styled(
                        "idle — waiting for next message",
                        Style::default()
                            .fg(t.text_muted)
                            .add_modifier(Modifier::ITALIC),
                    ),
                ])
            };
            // Render the hint in a 1-row strip at the bottom of the inner area.
            if inner.height >= 1 {
                let hint_area = Rect::new(inner.x, inner.y + inner.height - 1, inner.width, 1);
                f.render_widget(
                    Paragraph::new(hint_line).style(Style::default().bg(t.bg)),
                    hint_area,
                );
            }
        }
    } else {
        // Legacy string-log path — used for daemon-launched agents whose
        // events only arrive as TaskProgress strings with no structured data.
        let mut body_lines = body_lines;
        if task_is_running {
            let frame = (app.launched_at.elapsed().as_millis() / 80) as usize;
            let spinner_glyph = crate::app::SPINNER[frame % crate::app::SPINNER.len()];
            if !body_lines.is_empty() {
                body_lines.push(Line::from(""));
            }
            body_lines.push(Line::from(vec![
                Span::styled(
                    spinner_glyph.to_string(),
                    Style::default().fg(t.accent).add_modifier(Modifier::BOLD),
                ),
                Span::raw("  "),
                Span::styled("Receiving output…", Style::default().fg(t.text_muted)),
            ]));
        } else if task_is_idle {
            if !body_lines.is_empty() {
                body_lines.push(Line::from(""));
            }
            body_lines.push(Line::from(vec![
                Span::styled("⏸  ", Style::default().fg(t.text_muted)),
                Span::styled(
                    "idle — waiting for next message",
                    Style::default()
                        .fg(t.text_muted)
                        .add_modifier(Modifier::ITALIC),
                ),
            ]));
        }

        let render_width = inner.width;
        let total_lines: usize = body_lines
            .iter()
            .map(|line| {
                if line.width() == 0 || render_width == 0 {
                    1
                } else {
                    Paragraph::new(line.clone())
                        .wrap(ratatui::widgets::Wrap { trim: false })
                        .line_count(render_width)
                        .max(1)
                }
            })
            .sum();

        if should_snap_to_bottom(app, visible, total_lines) {
            app.scroll_offset = total_lines.saturating_sub(visible);
        } else {
            app.scroll_offset = app.scroll_offset.min(total_lines.saturating_sub(visible));
        }
        app.total_lines = total_lines;
        app.viewport_height = visible;

        if body_lines.is_empty() {
            let placeholder_text = if task_is_running {
                "Waiting for first chunk…"
            } else {
                "No messages yet for this background task."
            };
            let placeholder = Paragraph::new(vec![
                Line::from(""),
                Line::from(Span::styled(
                    placeholder_text,
                    Style::default().fg(t.text_muted),
                )),
            ])
            .style(Style::default().bg(t.bg));
            f.render_widget(placeholder, inner);
        } else {
            let para = Paragraph::new(body_lines)
                .style(Style::default().bg(t.bg))
                .wrap(ratatui::widgets::Wrap { trim: false })
                .scroll((app.scroll_offset as u16, 0));
            f.render_widget(para, inner);
        }
    }
}

/// Longest run of leading whitespace-delimited words shared by every
/// string in `items`, returned as a borrowed prefix of the first item
/// (including the trailing space). Empty when there are fewer than two
/// items or they share no leading word — so callers can `strip_prefix`
/// unconditionally. Used to de-noise the task tab strip where every tab
/// starts with the same verb ("Implement …").
fn common_word_prefix<'a>(items: &[&'a str]) -> &'a str {
    if items.len() < 2 {
        return "";
    }
    let first = items[0];
    // Walk word boundaries (positions just after each space) and keep the
    // longest boundary at which every item agrees.
    let mut best = 0usize;
    for (i, ch) in first.char_indices() {
        if ch == ' ' {
            let cand = i + 1; // include the trailing space
            if items
                .iter()
                .all(|s| s.as_bytes().get(..cand) == first.as_bytes().get(..cand))
            {
                best = cand;
            } else {
                break;
            }
        }
    }
    &first[..best]
}

pub(super) fn subagent_footer(f: &mut Frame, app: &App, area: Rect) {
    let t = app.theme;
    // One quiet tab strip. Selected tab tracks `viewing_task_id`; navigation
    // hints are appended only when there is room.
    let task_ids: Vec<String> = super::agents::fleet_ordered_task_ids(app);
    if task_ids.is_empty() {
        f.render_widget(
            Paragraph::new(Line::from(vec![Span::styled(
                "↑ back · no agents",
                Style::default().fg(t.text_muted),
            )]))
            .style(Style::default().bg(t.bg)),
            area,
        );
        return;
    }
    let selected = app
        .task_panel
        .viewing_task_id
        .as_ref()
        .and_then(|id| task_ids.iter().position(|t| t == id))
        .unwrap_or(0);
    // Strip the common leading words shared by every tab (e.g. each task
    // is "Implement X language adapter" → drop "Implement " so the tab
    // shows the part that actually differs instead of truncating it away).
    let descs: Vec<&str> = task_ids
        .iter()
        .map(|id| {
            app.engine
                .background_tasks
                .get(id)
                .map(|b| b.description.as_str())
                .unwrap_or(id.as_str())
        })
        .collect();
    let common = common_word_prefix(&descs);

    // Per-tab cell: a semantic status glyph (colour = state, matching the
    // fan) + the de-prefixed, truncated title.
    struct Tab {
        glyph: &'static str,
        color: ratatui::style::Color,
        title: String,
    }
    let tabs: Vec<Tab> = task_ids
        .iter()
        .map(|id| {
            let bt = app.engine.background_tasks.get(id);
            let desc = bt.map(|b| b.description.as_str()).unwrap_or(id.as_str());
            let desc = desc.strip_prefix(common).unwrap_or(desc).trim_start();
            let desc = if desc.is_empty() {
                bt.map(|b| b.description.as_str()).unwrap_or(id.as_str())
            } else {
                desc
            };
            let title = truncate_cells(desc, 22);
            let (glyph, color) = match bt.map(|b| b.status) {
                Some(jfc_core::TaskLifecycle::Running) => {
                    (crate::spinner::frame_for(app.spinner_frame), t.warning)
                }
                Some(jfc_core::TaskLifecycle::Completed) => ("✓", t.success),
                Some(jfc_core::TaskLifecycle::Failed) => ("✗", t.error),
                _ => ("○", t.text_muted),
            };
            Tab {
                glyph,
                color,
                title,
            }
        })
        .collect();

    // Window the tabs around `selected` so they never run off the edge.
    // Grow outward (right-biased) from the selected tab while the strip
    // fits, reserving room for the `‹ … ›` overflow arrows.
    let n = tabs.len();
    let avail = area.width as usize;
    let cell_w = |i: usize| cell_width(tabs[i].glyph) + 1 + cell_width(&tabs[i].title);
    const DIV: usize = 3; // " · "
    let mut lo = selected;
    let mut hi = selected;
    let mut total = cell_w(selected);
    loop {
        let reserve = if lo > 0 { 2 } else { 0 } + if hi + 1 < n { 2 } else { 0 };
        let mut grew = false;
        if hi + 1 < n && total + DIV + cell_w(hi + 1) + reserve <= avail {
            hi += 1;
            total += DIV + cell_w(hi);
            grew = true;
        }
        if lo > 0 && total + DIV + cell_w(lo - 1) + reserve <= avail {
            lo -= 1;
            total += DIV + cell_w(lo);
            grew = true;
        }
        if !grew {
            break;
        }
    }

    let mut spans: Vec<Span> = Vec::new();
    if lo > 0 {
        spans.push(Span::styled("‹ ", Style::default().fg(t.text_muted)));
    }
    for (i, tab) in tabs.iter().enumerate().take(hi + 1).skip(lo) {
        if i > lo {
            spans.push(Span::styled(" · ", Style::default().fg(t.border)));
        }
        let sel = i == selected;
        let glyph_style = Style::default().fg(tab.color);
        let title_style = if sel {
            Style::default()
                .fg(t.text_primary)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(t.text_muted)
        };
        spans.push(Span::styled(format!("{} ", tab.glyph), glyph_style));
        spans.push(Span::styled(tab.title.clone(), title_style));
    }
    if hi + 1 < n {
        spans.push(Span::styled(" ›", Style::default().fg(t.text_muted)));
    }

    let used_w: usize = spans.iter().map(|sp| cell_width(&sp.content)).sum();
    let hint = format!("↑ back · ←/→ · {}/{}", selected + 1, n);
    let hint_w = cell_width(&hint);
    if used_w + hint_w + 3 <= area.width as usize {
        spans.push(Span::styled(
            " ".repeat((area.width as usize).saturating_sub(used_w + hint_w)),
            Style::default(),
        ));
        spans.push(Span::styled(hint, Style::default().fg(t.text_muted)));
    }
    f.render_widget(
        Paragraph::new(Line::from(spans)).style(Style::default().bg(t.bg)),
        area,
    );
}

/// Pick the next open task to surface under the spinner — first
/// in-progress task wins, falling back to the first pending task.
/// Mirrors v126 cli.js:323851 (`m` = next task) which indents
/// `Next: ${m.subject}` underneath the spinner verb. Returns `None`
/// when the task list is empty so the renderer can shrink to a 1-row
/// spinner instead of leaving a blank second line.
fn next_open_task_subject(app: &App) -> Option<String> {
    use jfc_session::DeletedFilter;
    let tasks = app.engine.task_store.list(DeletedFilter::Exclude);
    pick_next_open_task(&tasks).map(|t| t.subject.clone())
}

/// Pure priority picker for the "Next: …" sub-status. In-progress wins
/// over pending so users see *what's running right now* rather than
/// *what's queued*. Falls back to the first pending when nothing is
/// active. Returns `None` when nothing is open. Extracted from
/// `next_open_task_subject` so unit tests can exercise the priority
/// rules without building an `App` fixture.
fn pick_next_open_task(tasks: &[jfc_session::Task]) -> Option<&jfc_session::Task> {
    use jfc_session::TaskStatus;
    tasks
        .iter()
        .find(|t| matches!(t.status, TaskStatus::InProgress))
        .or_else(|| tasks.iter().find(|t| t.status.is_open()))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum ActivityLabel {
    Working,
    Planning,
    Reading,
    Searching,
    Running,
    Editing,
    Creating,
}

impl ActivityLabel {
    fn as_str(self) -> &'static str {
        match self {
            Self::Creating => "Creating",
            Self::Editing => "Editing",
            Self::Running => "Running",
            Self::Searching => "Searching",
            Self::Reading => "Reading",
            Self::Planning => "Planning",
            Self::Working => "Working",
        }
    }

    fn for_tool(tool: &jfc_core::ToolCall) -> Self {
        match &tool.kind {
            ToolKind::Write => Self::Creating,
            ToolKind::Edit | ToolKind::MultiEdit | ToolKind::ApplyPatch => Self::Editing,
            ToolKind::Bash | ToolKind::BashOutput | ToolKind::HcomRun | ToolKind::HcomTerm => {
                Self::Running
            }
            ToolKind::Glob
            | ToolKind::Grep
            | ToolKind::Search
            | ToolKind::WebFetch
            | ToolKind::WebSearch
            | ToolKind::ToolSearch
            | ToolKind::Lsp => Self::Searching,
            ToolKind::Read | ToolKind::NotebookRead => Self::Reading,
            ToolKind::TaskCreate
            | ToolKind::TaskUpdate
            | ToolKind::TaskList
            | ToolKind::TaskDone
            | ToolKind::TaskStop
            | ToolKind::TaskGet
            | ToolKind::TaskValidate
            | ToolKind::PlanCreate
            | ToolKind::PlanList
            | ToolKind::PlanShow
            | ToolKind::PlanAdvance
            | ToolKind::PlanArchive
            | ToolKind::PlanMaterialize => Self::Planning,
            _ => Self::Working,
        }
    }
}

#[derive(Clone)]
struct FileActivity {
    path: String,
    additions: usize,
    deletions: usize,
}

#[derive(Clone)]
struct ActivityDetails {
    label: ActivityLabel,
    file: Option<FileActivity>,
    subject: Option<String>,
}

fn changed_line_count(text: &str) -> usize {
    if text.is_empty() {
        0
    } else {
        text.lines().count().max(1)
    }
}

fn estimate_multiedit_counts(edits: &serde_json::Value) -> (usize, usize) {
    let Some(edits) = edits.as_array() else {
        return (0, 0);
    };
    edits.iter().fold((0, 0), |(adds, dels), edit| {
        let old = edit
            .get("old_string")
            .or_else(|| edit.get("old_str"))
            .and_then(serde_json::Value::as_str)
            .unwrap_or("");
        let new = edit
            .get("new_string")
            .or_else(|| edit.get("new_str"))
            .and_then(serde_json::Value::as_str)
            .unwrap_or("");
        (
            adds + changed_line_count(new),
            dels + changed_line_count(old),
        )
    })
}

fn file_activity_for_tool(tool: &jfc_core::ToolCall) -> Option<FileActivity> {
    if let ToolOutput::Diff(diff) = &tool.output {
        return Some(FileActivity {
            path: diff.file_path.clone(),
            additions: diff.additions,
            deletions: diff.deletions,
        });
    }
    match &tool.input {
        ToolInput::Write { file_path, content } => Some(FileActivity {
            path: file_path.clone(),
            additions: changed_line_count(content),
            deletions: 0,
        }),
        ToolInput::Edit {
            file_path,
            old_string,
            new_string,
            ..
        } => Some(FileActivity {
            path: file_path.clone(),
            additions: changed_line_count(new_string),
            deletions: changed_line_count(old_string),
        }),
        ToolInput::MultiEdit { file_path, edits } => {
            let (additions, deletions) = estimate_multiedit_counts(edits);
            Some(FileActivity {
                path: file_path.clone(),
                additions,
                deletions,
            })
        }
        _ => None,
    }
}

fn activity_display_path(path: &str) -> String {
    let name = path
        .rsplit(|ch| ch == '/' || ch == '\\')
        .find(|part| !part.is_empty())
        .unwrap_or(path);
    truncate_str(name, 48)
}

fn first_activity_line(text: &str) -> Option<String> {
    text.lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(|line| truncate_str(line, 64))
}

fn background_task_id_from_output(output: &ToolOutput) -> Option<String> {
    let text = match output {
        ToolOutput::Text(text) => text.as_str(),
        ToolOutput::LargeText(text) => text.content.as_str(),
        _ => return None,
    };
    text.lines()
        .find_map(|line| line.strip_prefix("task_id: "))
        .map(str::trim)
        .filter(|task_id| task_id.starts_with("bash_"))
        .map(ToOwned::to_owned)
}

fn bash_command_for_task(app: &App, task_id: &str) -> Option<String> {
    if let Some(command) = jfc_engine::tools::bash_task_command(task_id)
        && let Some(line) = first_activity_line(&command)
    {
        return Some(line);
    }
    for message in app.engine.messages.iter().rev() {
        for part in message.parts.iter().rev() {
            let MessagePart::Tool(tool) = part else {
                continue;
            };
            let ToolInput::Bash { command, .. } = &tool.input else {
                continue;
            };
            if background_task_id_from_output(&tool.output).as_deref() == Some(task_id) {
                return first_activity_line(command);
            }
        }
    }
    None
}

fn activity_subject_for_tool(app: &App, tool: &jfc_core::ToolCall) -> Option<String> {
    let subject = match &tool.input {
        ToolInput::Bash { command, .. } => return first_activity_line(command),
        ToolInput::BashOutput { task_id, .. } => {
            return bash_command_for_task(app, task_id)
                .or_else(|| Some(format!("output {task_id}")));
        }
        ToolInput::Read { file_path, .. } => activity_display_path(file_path),
        ToolInput::Glob { pattern, path } => match path {
            Some(path) => format!("{pattern} in {path}"),
            None => pattern.clone(),
        },
        ToolInput::Grep { pattern, path, .. } => match path {
            Some(path) => format!("{pattern} in {path}"),
            None => pattern.clone(),
        },
        ToolInput::Search { query, path } => match path {
            Some(path) => format!("{query} in {path}"),
            None => query.clone(),
        },
        ToolInput::WebFetch { url, .. } => url.clone(),
        ToolInput::WebSearch { query, .. } | ToolInput::ToolSearch { query, .. } => query.clone(),
        ToolInput::Lsp {
            kind, file, line, ..
        } => format!("{kind} {file}:{line}"),
        ToolInput::HcomRun { .. } | ToolInput::HcomTerm { .. } => tool.input.summary(),
        _ => return None,
    };
    let trimmed = subject.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(truncate_str(trimmed, 64))
    }
}

fn active_tool_activity(app: &App) -> Option<ActivityDetails> {
    let mut best: Option<ActivityDetails> = None;
    let mut consider = |tool: &jfc_core::ToolCall| {
        let label = match &tool.input {
            ToolInput::Edit { old_string, .. } if old_string.is_empty() => ActivityLabel::Creating,
            _ => ActivityLabel::for_tool(tool),
        };
        let details = ActivityDetails {
            label,
            file: file_activity_for_tool(tool),
            subject: activity_subject_for_tool(app, tool),
        };
        let replace = best.as_ref().is_none_or(|current| {
            label > current.label
                || current.file.is_none() && details.file.is_some()
                || current.file.is_none() && current.subject.is_none() && details.subject.is_some()
        });
        if replace {
            best = Some(details);
        }
    };

    for tool in &app.engine.pending_tool_calls {
        consider(tool);
    }

    for tool in &app.engine.active_tool_calls {
        consider(tool);
    }

    for message in &app.engine.messages {
        for part in &message.parts {
            let MessagePart::Tool(tool) = part else {
                continue;
            };
            let executing = matches!(tool.status, ToolStatus::Pending | ToolStatus::Running)
                || app
                    .engine
                    .in_progress_tool_use_ids
                    .contains(tool.id.as_str());
            if executing {
                consider(tool);
            }
        }
    }

    best
}

fn plural_suffix(count: usize) -> &'static str {
    if count == 1 { "" } else { "s" }
}

fn working_activity_subject(app: &App) -> String {
    if !app.engine.pending_tool_calls.is_empty() {
        let count = app.engine.pending_tool_calls.len();
        return format!("dispatching {count} tool call{}", plural_suffix(count));
    }
    if app.engine.in_flight_tool_batches > 0 {
        let count = app.engine.in_flight_tool_batches;
        return format!("running {count} tool batch{}", plural_suffix(count));
    }
    if !app.engine.in_progress_tool_use_ids.is_empty() {
        let count = app.engine.in_progress_tool_use_ids.len();
        return format!("waiting on {count} tool result{}", plural_suffix(count));
    }
    if app.engine.in_flight_eager_dispatches > 0 {
        let count = app.engine.in_flight_eager_dispatches;
        return format!("dispatching {count} eager tool{}", plural_suffix(count));
    }
    let alive_agents = app
        .engine
        .background_tasks
        .values()
        .filter(|task| task.status.is_alive())
        .count();
    if alive_agents > 0 {
        return format!(
            "tracking {alive_agents} background agent{}",
            plural_suffix(alive_agents)
        );
    }
    if app.engine.queued_prompts.len() > 0 {
        let count = app.engine.queued_prompts.len();
        return format!(
            "waiting to drain {count} queued prompt{}",
            plural_suffix(count)
        );
    }
    if app.engine.is_streaming && app.engine.streaming_response_bytes == 0 {
        return "waiting for first model event".to_owned();
    }
    if app.engine.is_streaming {
        return "waiting for model stream".to_owned();
    }
    if app.engine.turn_started_at.is_some() {
        return "between model/tool steps".to_owned();
    }
    "active turn".to_owned()
}

fn working_activity(app: &App) -> ActivityDetails {
    ActivityDetails {
        label: ActivityLabel::Working,
        file: None,
        subject: Some(working_activity_subject(app)),
    }
}

fn push_swept_text(
    spans: &mut Vec<Span<'static>>,
    text: &str,
    tick: usize,
    base: Color,
    sweep: Color,
) {
    if crate::spinner::reduced_motion() {
        spans.push(Span::styled(text.to_owned(), Style::default().fg(base)));
        return;
    }
    let chars: Vec<char> = text.chars().collect();
    let len = chars.len().max(1);
    let head = tick % (len + 4);
    for (idx, ch) in chars.into_iter().enumerate() {
        let distance = head.abs_diff(idx);
        let color = if distance <= 1 { sweep } else { base };
        spans.push(Span::styled(ch.to_string(), Style::default().fg(color)));
    }
}

fn push_activity_spans(
    spans: &mut Vec<Span<'static>>,
    details: &ActivityDetails,
    tick: usize,
    t: Theme,
) {
    let ui_tokens = t.claude_ui_tokens();
    if let Some(file) = &details.file {
        push_swept_text(
            spans,
            details.label.as_str(),
            tick,
            t.text_secondary,
            t.warning,
        );
        spans.push(Span::raw(" "));
        spans.push(Span::styled(
            activity_display_path(&file.path),
            Style::default().fg(t.accent_secondary),
        ));
        spans.push(Span::raw(" "));
        spans.push(Span::styled(
            format!("+{}", file.additions),
            Style::default().fg(ui_tokens.diff_added),
        ));
        spans.push(Span::raw(" "));
        spans.push(Span::styled(
            format!("-{}", file.deletions),
            Style::default().fg(ui_tokens.diff_removed),
        ));
    } else if let Some(subject) = &details.subject {
        push_swept_text(
            spans,
            details.label.as_str(),
            tick,
            t.text_secondary,
            t.warning,
        );
        spans.push(Span::raw(" "));
        spans.push(Span::styled(
            subject.clone(),
            Style::default().fg(t.accent_secondary),
        ));
    } else {
        let label = format!("{}…", details.label.as_str());
        push_swept_text(spans, &label, tick, t.warning, t.text_primary);
    }
}

/// Single- or double-row spinner widget rendered between the message
/// scroll and the input bar (v126 layout, cli.js:323180-323235 + 323851).
/// Row 0 = verb + elapsed + live-token-count + stall-status, composed in
/// `crate::spinner`. Row 1 (when present) = `□ Next: <task subject>`,
/// matching cli.js's `Next: ${m.subject}` line.
pub(super) fn spinner_row(f: &mut Frame, app: &App, area: Rect) {
    if area.height == 0 {
        return;
    }
    let t = app.theme;
    let now = std::time::Instant::now();
    // Compaction takes precedence — a compact request runs to completion
    // before the user's submit ever fires the actual stream, so during
    // that window the spinner should read `Compacting…`, not a stale
    // `Fermenting…` from the previous turn.
    // `verb_spans` holds the phase label (and, for the recovery path, the
    // retry status). The renderer assembles the final line as
    // `glyph + verb_spans + body`. For the compact path we keep a single
    // pre-formatted string since compaction has its own status shape.
    let mut verb_spans: Vec<Span<'static>> = Vec::new();
    let mut compact_body: Option<String> = None;
    let mut tail_body: String = String::new();
    let mut head_glyph: &'static str = "";
    // True once the wire has gone quiet long enough that the row should
    // read as stalled — the glyph + label render muted instead of accent.
    let mut dim = false;
    if let Some(started) = app.engine.compacting_started_at {
        let elapsed = now.duration_since(started);
        // Pass the pre-compact token count so the spinner shows
        // *what's being compacted*. `tool_ctx.approx_tokens` still
        // reflects the pre-compact estimate during the compact (it's
        // only updated to the post-compact value when CompactionDone
        // fires), so it's the right source.
        let pre = app.engine.tool_ctx.approx_tokens as u64;
        compact_body = Some(crate::spinner::format_compact_status(
            app.spinner_frame,
            elapsed,
            pre,
            app.engine.compacting_output_chars,
        ));
    } else if let Some(recovery) = app.engine.network_recovery_status.as_ref() {
        head_glyph = crate::glyphs::RECOVERY;
        let label = match recovery.status_code {
            Some(code) => format!("{code} {}", recovery.reason.label()),
            None => recovery.reason.label().to_owned(),
        };
        verb_spans.push(Span::styled(
            label,
            Style::default().fg(t.warning).add_modifier(Modifier::BOLD),
        ));
        let last_seen = now.duration_since(recovery.updated_at).as_secs();
        tail_body = format!(
            " · retrying {} · attempt {} · last {}s",
            recovery.provider.label(),
            recovery.attempts,
            last_seen
        );
        if let Some(status) = app.engine.claude_status.as_ref()
            && let Some(outage) = status.outage_context()
        {
            tail_body.push_str(" · status ");
            tail_body.push_str(&truncate_str(&outage, 72));
        }
    } else {
        // Prefer the user-turn clock so a multi-step agentic loop reads
        // cumulative time, not just the current sub-stream's age. Fall back
        // to `streaming_started_at` for the brief first frame after submit
        // before the agentic gate updates the turn clock.
        let elapsed = app
            .engine
            .turn_started_at
            .or(app.engine.streaming_started_at)
            .map(|t| now.duration_since(t))
            .unwrap_or_default();
        let stream_is_live = app.engine.is_streaming;
        let lifecycle = app.engine.stream_lifecycle.as_ref().filter(|_| {
            app.engine.streaming_response_bytes == 0 && app.engine.streaming_thinking_tokens == 0
        });
        let stall = if stream_is_live {
            stream_quiet_duration(
                app.spinner_state.phase,
                lifecycle.is_some(),
                app.engine.streaming_last_token_at,
                now,
            )
        } else {
            std::time::Duration::default()
        };
        // Visible output token count = the Claude `responseLengthRef`
        // accumulator / 4. The usage handler floors the accumulator up to the
        // wire-truth `output_tokens*4`, while streamed chars keep growing it
        // between batched usage deltas. Cost/accounting still use real usage;
        // this display count is intentionally smoothed so it doesn't jump by
        // provider batch sizes.
        let live_tokens = if stream_is_live {
            (app.engine.streaming_response_bytes / 4) as u64
        } else {
            0
        };
        // Thinking signal, driven off the *held* phase so the tail stays
        // consistent with the hysteresis-stabilized row: while the internal
        // Thinking phase is held past the actual reasoning end, the tail still
        // reads `thinking` instead of racing ahead to `thought for Ns`.
        let thinking = match app.spinner_state.phase {
            crate::spinner::SpinnerPhase::Thinking => {
                let thinking_elapsed = app
                    .engine
                    .thinking_started_at
                    .map(|started| now.duration_since(started))
                    .unwrap_or(elapsed);
                Some(crate::spinner::ThinkingStatus::Live(thinking_elapsed))
            }
            crate::spinner::SpinnerPhase::Responding => {
                match (app.engine.thinking_started_at, app.engine.thinking_ended_at) {
                    (Some(start), Some(end)) => Some(crate::spinner::ThinkingStatus::Done(
                        end.duration_since(start),
                    )),
                    _ => None,
                }
            }
            _ => None,
        };
        // Windowed tokens/sec: the tick handler samples (elapsed, count)
        // into a trailing window for whichever counter is live this phase
        // — thinking tokens while reasoning, output tokens while
        // responding — and we read Δcount/Δt over it here. Self-smoothing
        // over TOKEN_RATE_WINDOW, so it reflects *current* throughput, not
        // a lifetime average. We render with `&App`, so we can only read.
        let token_rate = if stream_is_live {
            crate::spinner::windowed_token_rate(&app.engine.token_rate_samples)
        } else {
            None
        };
        let segs = crate::spinner::status_segments(
            app.spinner_frame,
            elapsed,
            live_tokens,
            token_rate,
            stall,
            thinking,
            app.engine.streaming_thinking_tokens,
        );
        head_glyph = segs.glyph;
        dim = segs.dim;
        let activity = match app.spinner_state.phase {
            crate::spinner::SpinnerPhase::Responding => active_tool_activity(app),
            crate::spinner::SpinnerPhase::ToolUse | crate::spinner::SpinnerPhase::Working => {
                active_tool_activity(app).or_else(|| Some(working_activity(app)))
            }
            _ => None,
        };
        let label: std::borrow::Cow<'_, str> = {
            if let Some(status) = lifecycle {
                std::borrow::Cow::Borrowed(status.phase.label())
            } else {
                match app.spinner_state.phase {
                    crate::spinner::SpinnerPhase::Responding
                    | crate::spinner::SpinnerPhase::ToolUse
                    | crate::spinner::SpinnerPhase::Working => activity
                        .as_ref()
                        .map(|details| std::borrow::Cow::Borrowed(details.label.as_str()))
                        .unwrap_or_else(|| {
                            std::borrow::Cow::Borrowed(app.spinner_state.phase.label())
                        }),
                    crate::spinner::SpinnerPhase::Requesting
                    | crate::spinner::SpinnerPhase::Thinking => {
                        std::borrow::Cow::Borrowed(app.spinner_state.phase.label())
                    }
                    crate::spinner::SpinnerPhase::Compacting
                    | crate::spinner::SpinnerPhase::NetworkRecovery => {
                        std::borrow::Cow::Borrowed(app.spinner_state.phase.label())
                    }
                }
            }
        };
        let mut label = label.into_owned();
        tail_body = segs.body;
        if let Some(detail) = lifecycle.and_then(|status| status.detail.as_deref())
            && !detail.trim().is_empty()
        {
            tail_body.push_str(" · ");
            tail_body.push_str(detail.trim());
        }
        if !matches!(
            app.spinner_state.phase,
            crate::spinner::SpinnerPhase::Compacting
                | crate::spinner::SpinnerPhase::NetworkRecovery
        ) && !label.ends_with('…')
        {
            label.push('…');
        }
        // Label color: secondary while live, muted once the stream has
        // gone quiet (the honest "stalled" tint — dimmer, not redder).
        let label_color = if dim { t.text_muted } else { t.warning };
        if let Some(activity) = activity {
            push_activity_spans(&mut verb_spans, &activity, app.spinner_frame, t);
        } else {
            push_swept_text(
                &mut verb_spans,
                &label,
                app.spinner_frame,
                label_color,
                t.text_primary,
            );
        }
    };
    // The `· N agents…` fanout badge that used to live here is gone — the
    // agent fan's `agents  ●N ○N ✓N ✗N` summary line (just below the
    // input) already carries the live count, and saying it twice was part
    // of the "see the same agent everywhere" redundancy.
    let spans: Vec<Span<'static>> = if let Some(body) = compact_body {
        // Compact path: keep the legacy single-string format. Compaction
        // has its own status line ("Compacting…", different shape) and
        // animating the shimmer there would be misleading — the verb
        // isn't a free-rotating spinner during compact.
        vec![Span::styled(body, Style::default().fg(t.text_secondary))]
    } else {
        // The glyph is the row's only motion: it cycles one frame per
        // tick whenever this row is on screen — and the row is only drawn
        // while a turn is actually live (streaming, compacting, running
        // tools, or fanning subagents; see `show_spinner` in frame.rs).
        // When the turn ends the row disappears entirely, so there's no
        // free-running pulse on an idle screen. It holds accent while
        // active and dims to muted once the wire has gone quiet, matching
        // the label.
        let glyph_color = if dim { t.text_muted } else { t.warning };
        let mut s = vec![Span::styled(
            format!("{} ", head_glyph),
            Style::default()
                .fg(glyph_color)
                .add_modifier(Modifier::BOLD),
        )];
        s.extend(verb_spans);
        let tail = tail_body.trim_start_matches(" · ").trim();
        if !tail.is_empty() {
            s.push(Span::styled(format!(" ({tail})"), t.style_text_muted));
        }
        s
    };
    let line = Line::from(spans);
    let row0 = Rect { height: 1, ..area };
    f.render_widget(Paragraph::new(line).style(Style::default().bg(t.bg)), row0);

    // Row 1: "Next: <task subject>" when a task is queued, otherwise a
    // rotating tip if streaming (CC 177 parity). The tip only shows during
    // streaming — idle screens stay quiet.
    if area.height >= 2 {
        let row1 = Rect {
            x: area.x,
            y: area.y + 1,
            width: area.width,
            height: 1,
        };
        if let Some(subj) = next_open_task_subject(app) {
            let prefix = "  □ Next: ";
            let max_body = (area.width as usize).saturating_sub(prefix.chars().count() + 1);
            let trimmed: String = if subj.chars().count() > max_body && max_body > 1 {
                let mut out: String = subj.chars().take(max_body.saturating_sub(1)).collect();
                out.push('…');
                out
            } else {
                subj
            };
            let row1_line = Line::from(vec![
                Span::styled(prefix, Style::default().fg(t.text_muted)),
                Span::styled(trimmed, Style::default().fg(t.text_muted)),
            ]);
            f.render_widget(
                Paragraph::new(row1_line).style(Style::default().bg(t.bg)),
                row1,
            );
        } else if app.engine.is_streaming || app.engine.turn_started_at.is_some() {
            // Show a spinner tip during streaming when no task is queued
            if let Some(tip) = crate::spinner::spinner_tip(app.spinner_frame) {
                let row1_line = Line::from(vec![Span::styled(
                    format!("  Tip: {tip}"),
                    Style::default().fg(t.text_muted),
                )]);
                f.render_widget(
                    Paragraph::new(row1_line).style(Style::default().bg(t.bg)),
                    row1,
                );
            }
        }
    }

    // The agent fan moved below the input — see `agent_fan_below_input`.
    // Keeping the spinner row at 2 rows (label + Next) means the activity
    // indicator stays glued to the prompt while the parallel work fan
    // lives on the other side, where peripheral status belongs.
}

/// Pinned todo list above the input. Mirrors Claude Code's compact task status:
/// one quiet header row (`3 tasks (1 done, 2 open)`), then up to the dynamic
/// visible cap task rows with status glyphs and an optional `… +N more` footer.
/// In-progress tasks bubble to the top so the row the user is actively driving
/// stays on screen even with a long pending queue. Per-subagent model badges
/// deliberately don't render here — they belong in the agent fan tree where
/// execution lives, not in the todo list where intent lives.
pub(super) fn tasks_pinned_row(f: &mut Frame, app: &App, area: Rect) {
    if area.height == 0 || area.width < 10 {
        return;
    }
    let t = app.theme;
    let all = app
        .engine
        .task_store
        .list(jfc_session::DeletedFilter::Exclude);
    if all.is_empty() {
        return;
    }
    // Defensive parity with the layout-side hide-when-all-done logic:
    // if the only thing we'd render is `Tasks (n/n done)` (no open
    // tasks, no recently-completed fade-out tail), skip entirely. The
    // layout already collapses our chunk height to 0 in that case, but
    // this lets `tasks_pinned_row` be safely called from elsewhere.
    let any_live = all.iter().any(|t| t.status.is_open());
    let now = std::time::Instant::now();
    let any_recent = all
        .iter()
        .filter(|t| matches!(t.status, jfc_session::TaskStatus::Completed))
        .any(|t| {
            app.engine
                .task_completion_times
                .get(&t.id)
                .is_some_and(|ts| now.duration_since(*ts).as_secs() < 30)
        });
    if !any_live && !any_recent {
        return;
    }
    let in_progress: Vec<_> = all
        .iter()
        .filter(|t| t.status == jfc_session::TaskStatus::InProgress)
        .collect();
    let mut pending: Vec<_> = all
        .iter()
        .filter(|t| t.status.is_open() && t.status != jfc_session::TaskStatus::InProgress)
        .collect();
    let completed: Vec<_> = all
        .iter()
        .filter(|t| t.status == jfc_session::TaskStatus::Completed)
        .collect();
    let completed_ids: std::collections::HashSet<&str> =
        completed.iter().map(|t| t.id.as_str()).collect();

    // Sort pending: unblocked first, then blocked (sorted by id for stability).
    pending.sort_by(|a, b| {
        let a_blocked = a
            .blocked_by
            .iter()
            .any(|id| !completed_ids.contains(id.as_str()));
        let b_blocked = b
            .blocked_by
            .iter()
            .any(|id| !completed_ids.contains(id.as_str()));
        a_blocked.cmp(&b_blocked).then_with(|| a.id.cmp(&b.id))
    });

    // Which todo is the running agent actually working on right now?
    // Link through the active agent's `parent_task_id` so that one task
    // reads as the live focus (bright + animated) while the rest of the
    // in-progress set dims — instead of N identical bold rows.
    let active_todo_id: Option<String> = app
        .engine
        .last_active_agent_task
        .as_deref()
        .and_then(|aid| app.engine.background_tasks.get(aid))
        .and_then(|bt| bt.parent_task_id.clone());

    let active_open = in_progress.len() + pending.len();

    // Flat: no divider rule — the pinned list floats above the input (the
    // input keeps the one top rule), so the bottom reads as two boundaries,
    // not a stack of shelves. Keep the background transparent to the chat
    // dock rather than drawing a full-width task box.
    let block = Block::default().style(Style::default().bg(t.bg));
    let inner = block.inner(area);
    f.render_widget(block, area);

    let render_width = inner.width as usize;
    let visible_budget = inner.height as usize;
    let mut rendered: Vec<Line<'static>> = Vec::new();

    let now_sort = std::time::Instant::now();
    let recent_done = completed
        .iter()
        .filter(|task| {
            app.engine
                .task_completion_times
                .get(&task.id)
                .is_some_and(|t| now_sort.duration_since(*t).as_secs() < 30)
        })
        .count();

    let title_line = tasks_pinned_header_line(recent_done, active_open, &t);

    // Everything done: collapse to a single recent-completion line. Historical
    // completions stay out of the dock so old task stores don't dominate the
    // current prompt.
    if !any_live {
        let n = recent_done.max(1);
        let label = if n == 1 {
            "1 task done".to_owned()
        } else {
            format!("{n} tasks done")
        };
        f.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled("  ", Style::default()),
                Span::styled("✓ ", Style::default().fg(t.success)),
                Span::styled(label, Style::default().fg(t.success)),
            ]))
            .style(Style::default().bg(t.bg)),
            inner,
        );
        return;
    }

    // Live work: compact header line, then the recent-completed summary.
    rendered.push(title_line);
    if recent_done > 0 && rendered.len() < visible_budget {
        let label = if recent_done == 1 {
            "1 just completed".to_owned()
        } else {
            format!("{recent_done} just completed")
        };
        rendered.push(Line::from(vec![
            Span::styled("  ", Style::default()),
            Span::styled("✓ ", Style::default().fg(t.success)),
            Span::styled(label, Style::default().fg(t.success)),
        ]));
    }

    // In-progress tasks use a filled square, not a second spinner. The live
    // spinner row already carries activity; the checklist is state, so it
    // stays stable while the turn redraws.
    let mut focal_used = false;
    let mut open_rendered = 0usize;
    for (i, task) in in_progress.iter().enumerate() {
        if rendered.len() >= visible_budget {
            break;
        }
        let is_focal = match active_todo_id.as_deref() {
            Some(id) => task.id.as_str() == id,
            None => i == 0 && !focal_used,
        };
        if is_focal {
            focal_used = true;
        }
        let (glyph, glyph_style, name_style) = if is_focal {
            (
                "■".to_string(),
                Style::default().fg(t.accent).add_modifier(Modifier::BOLD),
                Style::default()
                    .fg(t.text_primary)
                    .add_modifier(Modifier::BOLD),
            )
        } else {
            (
                "■".to_string(),
                Style::default().fg(t.accent_secondary),
                Style::default()
                    .fg(t.text_secondary)
                    .add_modifier(Modifier::BOLD),
            )
        };
        let avail = render_width.saturating_sub(5);
        rendered.push(Line::from(vec![
            Span::styled("  ", Style::default()),
            Span::styled(format!("{glyph} "), glyph_style),
            Span::styled(truncate_str(&task.subject, avail), name_style),
        ]));
        open_rendered += 1;
        // activeForm sub-line only for the focal task — that's the one
        // whose live activity ("Constructing CFG…") is worth the row.
        if is_focal
            && let Some(ref form) = task.active_form
            && form != &task.subject
            && rendered.len() < visible_budget
        {
            let sub_avail = render_width.saturating_sub(7);
            rendered.push(Line::from(vec![
                Span::styled("    ", Style::default()),
                Span::styled(
                    truncate_str(form, sub_avail),
                    Style::default()
                        .fg(t.text_muted)
                        .add_modifier(Modifier::ITALIC),
                ),
                Span::styled("…", Style::default().fg(t.text_muted)),
            ]));
        }
    }
    for task in &pending {
        if rendered.len() >= visible_budget {
            break;
        }
        let open_blockers: Vec<&str> = task
            .blocked_by
            .iter()
            .filter(|id| !completed_ids.contains(id.as_str()))
            .map(|id| id.as_str())
            .collect();
        let blocked = task.status == jfc_session::TaskStatus::Blocked || !open_blockers.is_empty();
        // Queued = hollow checkbox; blocked keeps the same shape but dims and
        // spells out the blocker so the row stays text-first.
        let icon = task.status.glyph();
        let color = if blocked {
            t.text_muted
        } else {
            t.text_secondary
        };
        let blockers_suffix = if blocked {
            format!(" · blocked by {}", open_blockers.join(", "))
        } else {
            String::new()
        };
        let avail = render_width.saturating_sub(5 + blockers_suffix.len());
        rendered.push(Line::from(vec![
            Span::styled("  ", Style::default()),
            Span::styled(format!("{icon} "), Style::default().fg(color)),
            Span::styled(
                truncate_str(&task.subject, avail),
                Style::default().fg(color),
            ),
            Span::styled(
                blockers_suffix,
                Style::default()
                    .fg(t.text_muted)
                    .add_modifier(Modifier::ITALIC),
            ),
        ]));
        open_rendered += 1;
    }

    // Overflow footer if we couldn't fit everything.
    let hidden_open = active_open.saturating_sub(open_rendered);
    if hidden_open > 0 && rendered.len() < visible_budget {
        rendered.push(Line::from(Span::styled(
            format!("  … +{hidden_open} more · open /tasks for the full list"),
            Style::default()
                .fg(t.text_muted)
                .add_modifier(Modifier::ITALIC),
        )));
    }

    f.render_widget(
        Paragraph::new(rendered).style(Style::default().bg(t.bg)),
        inner,
    );
}

#[cfg(test)]
mod stream_status_tests {
    use super::stream_quiet_duration;
    use crate::spinner::SpinnerPhase;
    use std::time::{Duration, Instant};

    #[test]
    fn quiet_duration_only_applies_to_content_phases_regression() {
        let now = Instant::now();
        let last = now - Duration::from_secs(51);

        assert_eq!(
            stream_quiet_duration(SpinnerPhase::ToolUse, false, Some(last), now),
            Duration::default()
        );
        assert_eq!(
            stream_quiet_duration(SpinnerPhase::Working, false, Some(last), now),
            Duration::default()
        );
        assert_eq!(
            stream_quiet_duration(SpinnerPhase::Thinking, true, Some(last), now),
            Duration::default()
        );
        assert_eq!(
            stream_quiet_duration(SpinnerPhase::Thinking, false, Some(last), now),
            Duration::from_secs(51)
        );
    }
}

fn tasks_pinned_header_line(_recent_done: usize, active_open: usize, t: &Theme) -> Line<'static> {
    let task_word = if active_open == 1 { "task" } else { "tasks" };
    let summary = format!("({active_open} open)");
    let spans = vec![
        Span::styled("  ", Style::default()),
        Span::styled(
            format!("{active_open} {task_word}"),
            Style::default()
                .fg(t.text_secondary)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(format!(" {summary}"), Style::default().fg(t.text_muted)),
    ];
    Line::from(spans)
}

/// Render the running-agents tree in its own chunk beneath the input box.
/// Honors the same team-vs-subagent dispatch as the legacy in-spinner
/// path. Skips entirely when there's no live data — caller already gates
/// on `tree_rows > 0`, but defensive return keeps the function safe to
/// call unconditionally in future call sites.
#[allow(dead_code)]
pub(super) fn agent_fan_below_input(f: &mut Frame, app: &App, area: Rect) {
    if area.height == 0 {
        return;
    }
    let t = app.theme;
    let is_team = app.engine.team_context.is_active();
    // Flat dock: a single TOP divider. The subagent path draws its own
    // `agents  ●N ○N ✓N ✗N` summary line, so no box title there; the
    // team path keeps a `team` label since its tree has no summary row.
    let mut block = Block::default()
        .borders(Borders::TOP)
        .border_style(Style::default().fg(t.border))
        .style(Style::default().bg(t.surface));
    if is_team {
        block = block.title(Span::styled(
            " team ",
            Style::default()
                .fg(t.text_secondary)
                .add_modifier(Modifier::BOLD),
        ));
    }
    let inner = block.inner(area);
    f.render_widget(block, area);
    if is_team {
        render_teammate_tree(f, app, inner);
    } else {
        render_subagent_tree(f, app, inner);
    }
}

#[cfg(test)]
mod next_task_tests {
    use super::*;
    use jfc_session::{DeletedFilter, TaskStore};

    #[test]
    fn empty_store_returns_none_normal() {
        let store = TaskStore::in_memory();
        let tasks = store.list(DeletedFilter::Exclude);
        assert!(pick_next_open_task(&tasks).is_none());
    }

    #[test]
    fn single_pending_task_picked_normal() {
        let store = TaskStore::in_memory();
        store
            .create(
                "Wire spinner".into(),
                String::new(),
                None,
                Vec::<String>::new(),
            )
            .unwrap();
        let tasks = store.list(DeletedFilter::Exclude);
        let picked = pick_next_open_task(&tasks).expect("should pick the pending task");
        assert_eq!(picked.subject, "Wire spinner");
    }

    #[test]
    fn in_progress_wins_over_pending_normal() {
        // v126's `Next: ${m.subject}` shows the *active* task, not the
        // queued one — what's running matters more than what's queued.
        let store = TaskStore::in_memory();
        let pending = store
            .create(
                "First (pending)".into(),
                String::new(),
                None,
                Vec::<String>::new(),
            )
            .unwrap();
        let active = store
            .create(
                "Second (will be in-progress)".into(),
                String::new(),
                None,
                Vec::<String>::new(),
            )
            .unwrap();
        store
            .update(
                active.id.as_str(),
                jfc_session::TaskPatch {
                    status: Some(jfc_session::TaskStatus::InProgress),
                    ..Default::default()
                },
            )
            .unwrap();
        let tasks = store.list(DeletedFilter::Exclude);
        let picked = pick_next_open_task(&tasks).expect("in-progress should win");
        assert_eq!(picked.subject, "Second (will be in-progress)");
        // Sanity: the pending task IS in the list, just not picked.
        assert!(
            tasks.iter().any(|t| t.id.as_str() == pending.id.as_str()),
            "pending task should still be in the list"
        );
    }

    #[test]
    fn only_completed_returns_none_robust() {
        let store = TaskStore::in_memory();
        let t = store
            .create(
                "Done thing".into(),
                String::new(),
                None,
                Vec::<String>::new(),
            )
            .unwrap();
        store
            .update(
                t.id.as_str(),
                jfc_session::TaskPatch {
                    status: Some(jfc_session::TaskStatus::Completed),
                    ..Default::default()
                },
            )
            .unwrap();
        let tasks = store.list(DeletedFilter::Exclude);
        assert!(
            pick_next_open_task(&tasks).is_none(),
            "completed-only store should yield no open task"
        );
    }

    #[test]
    fn skips_completed_when_pending_exists_robust() {
        let store = TaskStore::in_memory();
        let done = store
            .create(
                "Already done".into(),
                String::new(),
                None,
                Vec::<String>::new(),
            )
            .unwrap();
        store
            .update(
                done.id.as_str(),
                jfc_session::TaskPatch {
                    status: Some(jfc_session::TaskStatus::Completed),
                    ..Default::default()
                },
            )
            .unwrap();
        store
            .create(
                "Still queued".into(),
                String::new(),
                None,
                Vec::<String>::new(),
            )
            .unwrap();
        let tasks = store.list(DeletedFilter::Exclude);
        let picked = pick_next_open_task(&tasks).expect("pending should be picked");
        assert_eq!(picked.subject, "Still queued");
    }
}

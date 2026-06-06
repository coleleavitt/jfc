//! `UiEvent::Tick` handler — fires every IDLE_TICK_MS / ANIM_TICK_MS and
//! drives all per-tick housekeeping: spinner, watchdog, network EKG,
//! daemon-state sync, toast expiry, OAuth refresh, hot-reloads, kinetic
//! scroll, heartbeat, MCP-tools change watcher, file-watcher reload,
//! worktree count, git branch, swarm permission poller, swarm inbox poller.
//!
//! Returns `true` when the handler dirtied state that needs a fresh draw.

use std::sync::Arc;

use crate::app::App;
use crate::providers::AnthropicOAuthProvider;
use crate::runtime::{
    AppEvent, EventSender, ProviderEvent, TeamEvent, UiEvent, maybe_continue_task_factory,
    read_git_branch_from_root, sync_detached_background_tasks_from_daemon,
};
use crate::toast;

use super::super::guards::{CONFIG_RELOAD_REMINDER, MCP_REFRESH_REMINDER};

pub(crate) async fn handle_tick(
    app: &mut App,
    tx: &EventSender,
    oauth_for_snapshot: Option<&Arc<AnthropicOAuthProvider>>,
) -> bool {
    let mut needs_draw = false;

    app.spinner_frame = (app.spinner_frame + 1) % crate::app::SPINNER.len();
    app.check_stream_watchdog();

    // Advance the spinner phase machine here, on the throttled tick, so the
    // status label can only change as fast as the dwell allows (anti-flicker).
    // The renderer reads `app.spinner_state.phase`; it no longer derives the
    // label raw from per-frame fields.
    {
        let now = std::time::Instant::now();
        let turn_active = app.turn_started_at.is_some() || app.is_streaming;
        let thinking_live = app.thinking_started_at.is_some() && app.thinking_ended_at.is_none();
        if !turn_active {
            // Turn fully ended — reset the per-turn thinking clock so the next
            // turn re-measures its own minimum-thinking-display window.
            app.spinner_state.thinking_first_seen_at = None;
        } else if thinking_live && app.spinner_state.thinking_first_seen_at.is_none() {
            app.spinner_state.thinking_first_seen_at = Some(now);
        }
        let raw = crate::spinner::RawPhaseInputs {
            compacting: app.compacting_started_at.is_some(),
            network_recovery: app.network_recovery_status.is_some(),
            is_streaming: app.is_streaming,
            thinking_live,
            thinking_ended: app.thinking_ended_at.is_some(),
            output_started: app.streaming_response_bytes > 0,
            tools_pending: !app.pending_tool_calls.is_empty(),
            turn_active,
        };
        let next = crate::spinner::next_phase(
            app.spinner_state.phase,
            app.spinner_state.entered_at,
            app.spinner_state.thinking_first_seen_at,
            now,
            raw,
        );
        if next != app.spinner_state.phase {
            app.spinner_state.phase = next;
            app.spinner_state.entered_at = now;
        }
    }

    // Windowed tokens/sec sampling: push one (elapsed, count) point per tick
    // while streaming, then trim to TOKEN_RATE_WINDOW. The render path reads
    // the window each frame; we mutate it here because tick.rs has `&mut App`
    // while the renderer only has `&App`.
    //
    // We sample whichever counter is live this phase — thinking tokens while
    // the model is reasoning, output tokens once it's responding — so the
    // `tok/s` chip honestly reflects the work actually happening. At the
    // thinking→responding hand-off the live count drops (output starts below
    // the thinking total); we clear the window then so the rate is always
    // measured within a single phase rather than across the discontinuity.
    if app.is_streaming
        && let Some(started) = app.streaming_started_at
    {
        let elapsed = started.elapsed();
        let thinking_live = app.thinking_started_at.is_some() && app.thinking_ended_at.is_none();
        let count = if thinking_live {
            app.streaming_thinking_tokens
        } else {
            // True cumulative wire output tokens — so tok/s is measured on the
            // real count, not the chars/4 estimate. It only advances on usage
            // events, which the windowed rate handles fine.
            app.turn_output_tokens
        };
        if app
            .token_rate_samples
            .back()
            .is_some_and(|&(_, last)| count < last)
        {
            app.token_rate_samples.clear();
        }
        app.token_rate_samples.push_back((elapsed, count));
        crate::spinner::trim_token_samples(&mut app.token_rate_samples);
    }

    // Detached background workers update their progress in
    // `daemon-state.json` (they're a different process — no
    // AppEvent channel back to the UI). Re-read once a
    // second so the fan row shows live tool/token counts
    // instead of frozen zeros.
    let detached_sync_due = app
        .last_detached_sync_at
        .map(|t| t.elapsed() >= std::time::Duration::from_secs(1))
        .unwrap_or(true);
    if detached_sync_due {
        app.last_detached_sync_at = Some(std::time::Instant::now());
        // Detached workers (and in non-team mode, the
        // session task store) write task updates straight
        // to the JSON file from their own process. The UI's
        // TaskStore handle is loaded once and never re-reads
        // on its own — this mtime-gated reload picks up
        // those external TaskUpdate/TaskDone writes so the
        // todo panel reflects background-agent progress.
        if app.task_store.reload_if_changed() {
            needs_draw = true;
        }
        if sync_detached_background_tasks_from_daemon(app) {
            needs_draw = true;
            // A detached agent reaching a terminal state via daemon sync (not
            // an AppEvent) must be able to wake a parked leader. The agentic
            // resume hook normally fires from the TaskCompleted/TaskFailed
            // AppEvent handlers (handlers/task.rs), but detached agents never
            // emit those events back to the UI — only the JSON-file sync sees
            // their completion. Without this call a leader that delegated all
            // its remaining work to detached agents would stay parked until
            // the user sends another prompt. Resume first so a still-active
            // turn picks up its continuation before the factory considers new
            // queue work.
            super::task::maybe_resume_after_background(app, tx).await;
            // Re-evaluate the task factory after detached
            // agents transition. Without this,
            // maybe_continue_task_factory's
            // `background_tasks.any(is_alive)` gate blocks
            // the queue while agents run, but their later
            // completion (via daemon sync, not AppEvent)
            // never re-triggers the factory — the queue
            // stalls until the user sends another prompt.
            maybe_continue_task_factory(app, tx).await;
        }
    }
    // Auto-clear expired toasts every tick. Cheap (O(N) over
    // a tiny vec capped at MAX_TOASTS) and the only reliable
    // place to do it — toasts have no creation-time timer.
    if toast::prune_expired(&mut app.toasts, std::time::Instant::now()) {
        needs_draw = true;
    }

    // Idle-return detection: if 75+ minutes since last user
    // activity and we haven't shown the prompt yet, show a
    // toast suggesting /clear to save tokens on the next turn.
    if !app.idle_return_shown
        && !app.is_streaming
        && app.last_user_activity_at.elapsed().as_secs() >= 75 * 60
        && !app.messages.is_empty()
    {
        app.idle_return_shown = true;
        let tokens_est = app.tool_ctx.approx_tokens;
        crate::toast::push_with_cap(
            &mut app.toasts,
            crate::toast::Toast::new(
                crate::toast::ToastKind::Info,
                format!(
                    "Welcome back! Context: ~{tokens_est} tokens. \
                     Consider `/clear` to save on re-caching or `/compact` to trim."
                ),
            ),
        );
        needs_draw = true;
    }

    // Idle-drain safety net for queued prompts. JFC's queue normally drains on
    // stream Done/Error and CompactionDone, but those are one-shot events: if a
    // turn ends through a path that doesn't fire one (e.g. a user interrupt
    // whose aborted task never surfaces an Error, or any future gap), a queued
    // prompt can strand with nothing to drain it — the "queue a message after
    // cancelling and it never runs" bug. Claude Code avoids this by reading its
    // command queue whenever the query loop goes idle; this mirrors that. The
    // guard is strict: every in-flight signal must be clear, so this only ever
    // fires when the app is genuinely idle with work waiting. `drain` sets
    // `is_streaming` synchronously, so it can't double-fire on the next tick.
    if !app.queued_prompts.is_empty()
        && !app.is_streaming
        && app.turn_started_at.is_none()
        && app.pending_question.is_none()
        && !app.pipeline_busy_for_submit()
    {
        tracing::warn!(
            target: "jfc::ui::queue",
            depth = app.queued_prompts.len(),
            "idle-drain safety net: draining queued prompts the event path missed"
        );
        crate::runtime::drain_queued_prompts(app, tx).await;
        needs_draw = true;
    }

    // Speculative compaction: when the context reaches ~80% of the
    // compact threshold and we're idle (not streaming, not already
    // compacting), pre-set `force_compact_pending` so the next submit
    // fires compaction immediately instead of discovering it needs to
    // compact on the hot path. This matches CC 2.1.144's "precomputed
    // compact" concept — the actual LLM call still happens at submit
    // time, but the user sees it fire instantly instead of after a
    // "context estimation → discover over-limit → then compact" dance.
    if !app.is_streaming
        && app.compacting_started_at.is_none()
        && !app.speculative_compact_fired
        && !app.force_compact_pending
    {
        let est = app.tool_ctx.approx_tokens;
        let level = crate::compact::compact_level(est, app.max_context_tokens);
        if matches!(level, crate::compact::CompactLevel::Precompute) {
            tracing::info!(
                target: "jfc::compact",
                est,
                max = app.max_context_tokens,
                "speculative compact: context at ~80% threshold — pre-arming compaction"
            );
            app.force_compact_pending = true;
            app.speculative_compact_fired = true;
        }
    }

    // Refresh the cached Anthropic OAuth account snapshot every ~10s
    // so the ribbon shows up-to-date 5h/7d utilization and the
    // active rate-limit claim. The manager call locks a mutex,
    // so we throttle and run it on a background task.
    let needs_refresh = app
        .anthropic_snapshot_refreshed_at
        .map(|t| t.elapsed().as_secs() >= 10)
        .unwrap_or(true);
    if needs_refresh && let Some(oauth) = oauth_for_snapshot {
        app.anthropic_snapshot_refreshed_at = Some(std::time::Instant::now());
        let oauth = oauth.clone();
        let tx = tx.clone();
        tokio::spawn(async move {
            if let Ok(mgr) = oauth.account_manager().await {
                let snapshot = mgr.snapshot_for_ui().await;
                let _ = tx
                    .send(AppEvent::Provider(
                        ProviderEvent::AnthropicSnapshotUpdated { snapshot },
                    ))
                    .await;
            }
        });
    }

    // Kinetic scroll: apply velocity, decay, stop.
    {
        let now = std::time::Instant::now();
        let dt = now.duration_since(app.last_scroll_tick).as_secs_f32();
        app.last_scroll_tick = now;
        if app.scroll_velocity.abs() > 0.5 {
            let delta = app.scroll_velocity * dt;
            let lines = delta.round() as i32;
            if lines > 0 {
                app.scroll_down(lines as usize);
                needs_draw = true;
            } else if lines < 0 {
                app.scroll_up(lines.unsigned_abs() as usize);
                needs_draw = true;
            }
            app.scroll_velocity *= 0.85;
            if app.scroll_velocity.abs() < 0.5 {
                app.scroll_velocity = 0.0;
                needs_draw = true;
            }
        }
    }

    app.update_wants_animation_frame();
    // When there's genuine on-screen motion (streaming, a live agent, a
    // running task spinner, kinetic scroll, a visible toast), every tick
    // must redraw — otherwise the animation only advances when an input
    // event happens to force a draw, which is the "braille only moves
    // when I type" jank. Idle (no motion) still skips the draw.
    if app
        .wants_animation_frame
        .load(std::sync::atomic::Ordering::Relaxed)
    {
        needs_draw = true;
    }

    // v132 OnHeartbeat — fire every ~30s so registered
    // handlers (telemetry batchers, MCP keep-alive, daemon
    // wakeup probes) actually run. Async fire because we
    // don't care about the result — short-circuit logic
    // would block the UI thread.
    let now = std::time::Instant::now();
    let heartbeat_due = app
        .last_heartbeat_at
        .map(|t| now.duration_since(t).as_secs() >= 30)
        .unwrap_or(true);
    if heartbeat_due {
        app.last_heartbeat_at = Some(now);
        let session_id = app
            .current_session_id
            .as_ref()
            .map(|s| s.as_str().to_owned())
            .unwrap_or_else(|| "<no-session>".to_owned());
        crate::hooks::fire_async(
            crate::hooks::HookPoint::OnHeartbeat,
            &crate::hooks::HookContext::for_session(&session_id),
        );
    }

    // v132 MCP `notifications/tools/list_changed` —
    // detect inbound notifications by comparing the
    // process-global refresh counter against our last-
    // seen value. On change, emit a toast + system-
    // reminder so the user knows the tool catalog
    // mutated and the model picks up the change next
    // turn.
    let cur_refresh = crate::mcp::registry::refresh_counter();
    if cur_refresh > app.last_mcp_refresh_seen {
        app.last_mcp_refresh_seen = cur_refresh;
        toast::push_with_cap(
            &mut app.toasts,
            toast::Toast::new(
                toast::ToastKind::Info,
                "MCP server pushed tools/list_changed — catalog refreshed",
            ),
        );
        app.queue_background_reminder(MCP_REFRESH_REMINDER);
        needs_draw = true;
        // Re-sync sidebar MCP status after catalog change.
        if let Some(registry) = crate::tools::snapshot_mcp_registry() {
            let servers = registry
                .list()
                .await
                .iter()
                .map(|s| crate::types::McpServerInfo {
                    name: s.name.clone(),
                    status: match s.status {
                        crate::mcp::McpServerStatus::Connected => {
                            crate::types::McpStatus::Connected
                        }
                        crate::mcp::McpServerStatus::Failed => crate::types::McpStatus::Error,
                        crate::mcp::McpServerStatus::Disabled => crate::types::McpStatus::Disabled,
                    },
                })
                .collect();
            app.mcp_servers = servers;
            needs_draw = true;
        }
    }

    // v132 file-watcher reload — detect CLAUDE.md /
    // agents / settings edits by comparing the global
    // change counter against our last-seen value. On
    // change, queue a system-reminder so the model
    // picks up the new content on the next outbound
    // request. The reminder lives wire-only via the
    // background-reminder queue, so repeated FS events
    // between turns collapse to a single entry.
    let cur_fw = crate::file_watcher::change_counter();
    if cur_fw > app.last_file_watcher_seen {
        app.last_file_watcher_seen = cur_fw;
        let already_queued = app
            .pending_background_reminders
            .iter()
            .any(|body| body == CONFIG_RELOAD_REMINDER);
        if already_queued {
            tracing::debug!(
                target: "jfc::file_watcher",
                counter = cur_fw,
                "config reload reminder already queued for next outbound request"
            );
        } else {
            toast::push_with_cap(
                &mut app.toasts,
                toast::Toast::new(
                    toast::ToastKind::Info,
                    "Config file changed — reloaded for next turn",
                ),
            );
            app.queue_background_reminder(CONFIG_RELOAD_REMINDER);
            needs_draw = true;
        }
    }

    // Hot-reload keybindings when keybindings.toml changes.
    let cur_kb = crate::file_watcher::keybindings_change_counter();
    if cur_kb > app.last_keybindings_watcher_seen {
        app.last_keybindings_watcher_seen = cur_kb;
        crate::keybindings::load();
        toast::push_with_cap(
            &mut app.toasts,
            toast::Toast::new(toast::ToastKind::Info, "Reloaded keybindings.toml"),
        );
        needs_draw = true;
    }

    // Refresh the worktree count at most once per 10s,
    // only if we're inside a git repo.
    let now = std::time::Instant::now();
    app.resolve_git_root();
    let is_git = matches!(app.git_root, Some(Some(_)));
    let due = is_git
        && app
            .worktree_count_last_refresh
            .map(|t| now.duration_since(t).as_millis() >= 10_000)
            .unwrap_or(true);
    if due {
        let cwd = std::env::current_dir().unwrap_or_default();
        app.worktree_count_last_refresh = Some(now);
        let tx = tx.clone();
        tokio::spawn(async move {
            let count = match tokio::time::timeout(
                std::time::Duration::from_secs(2),
                crate::worktrees::list_worktrees_async(&cwd),
            )
            .await
            {
                Ok(Ok(list)) => list.len().saturating_sub(1),
                Ok(Err(error)) => {
                    tracing::debug!(
                        target: "jfc::worktrees",
                        %error,
                        "worktree count refresh failed"
                    );
                    0
                }
                Err(_) => {
                    tracing::warn!(
                        target: "jfc::worktrees",
                        "worktree count refresh timed out"
                    );
                    0
                }
            };
            let _ = tx
                .send(AppEvent::Ui(UiEvent::WorktreeCountLoaded(count)))
                .await;
        });
    }

    // Git branch refresh — every 5s from cached git root.
    let git_due = is_git
        && app
            .git_branch_last_refresh
            .map(|t| now.duration_since(t).as_millis() >= 5_000)
            .unwrap_or(true);
    if git_due {
        if let Some(Some(ref root)) = app.git_root {
            app.git_branch = read_git_branch_from_root(root).await;
        }
        app.git_branch_last_refresh = Some(now);
    }

    // Resolve any pending teammate permission requests at
    // ~1Hz (12 ticks × 80ms). The teammate runner blocks
    // on `poll_for_response` after writing a request; if
    // nothing ever resolves, the call times out at 5
    // minutes and the tool fails. This loop provides the
    // leader-side response: apply the leader's own
    // permission_mode to the request and write a resolution
    // file the teammate's poll picks up.
    if app.team_context.is_active()
        && app.spinner_frame.is_multiple_of(12)
        && let Some(team_name) = app.team_context.team_name.clone()
    {
        let mode = app.permission_mode;
        let tx_swarm = tx.clone();
        tokio::spawn(async move {
            let pending = crate::swarm::permission_sync::read_pending_permissions(&team_name).await;
            for req in pending {
                if !matches!(
                    req.status,
                    crate::swarm::types::PermissionRequestStatus::Pending
                ) {
                    continue;
                }
                let mutation = matches!(
                    req.tool_name.as_str(),
                    "Bash" | "Write" | "Edit" | "ApplyPatch"
                );
                // Three outcomes:
                //   Some(true)  → auto-approve
                //   Some(false) → auto-deny
                //   None        → defer to the user
                let auto: Option<bool> = match mode {
                    crate::app::PermissionMode::BypassPermissions => Some(true),
                    crate::app::PermissionMode::Plan => Some(false),
                    crate::app::PermissionMode::AcceptEdits => {
                        if matches!(req.tool_name.as_str(), "Bash") {
                            None
                        } else {
                            Some(true)
                        }
                    }
                    crate::app::PermissionMode::Default | crate::app::PermissionMode::Auto => {
                        if mutation {
                            // Mutations need a human in
                            // Default/Auto. Surface to
                            // the user via toast +
                            // /swarm-approve|deny.
                            None
                        } else {
                            Some(true)
                        }
                    }
                };
                match auto {
                    Some(approve) => {
                        let resolution = crate::swarm::types::PermissionResolution {
                            decision: if approve {
                                crate::swarm::types::PermissionDecision::Approved
                            } else {
                                crate::swarm::types::PermissionDecision::Rejected
                            },
                            resolved_by: "leader".to_owned(),
                            feedback: if approve {
                                None
                            } else {
                                Some(format!("Auto-denied by leader permission_mode={:?}", mode))
                            },
                            updated_input: None,
                            permission_updates: Vec::new(),
                        };
                        if let Err(e) = crate::swarm::permission_sync::resolve_permission(
                            &req.id,
                            &resolution,
                            &team_name,
                        )
                        .await
                        {
                            tracing::warn!(
                                target: "jfc::swarm",
                                error = %e,
                                request_id = %req.id,
                                "failed to resolve permission request"
                            );
                        }
                    }
                    None => {
                        // User-gate path: surface a
                        // toast (once per request id).
                        // The toast tells the user
                        // exactly which slash command
                        // resolves it.
                        let toast_text = format!(
                            "🔒 {} wants to {} — /swarm-approve {} or /swarm-deny {}",
                            req.worker_name, req.tool_name, req.id, req.id,
                        );
                        let _ = tx_swarm
                            .send(AppEvent::Ui(UiEvent::Toast {
                                kind: crate::toast::ToastKind::Warning,
                                text: toast_text,
                            }))
                            .await;
                    }
                }
            }
        });
    }

    // Poll leader inbox for teammate messages every ~1s (12 ticks * 80ms).
    // Only active when a team is running.
    if app.team_context.is_active()
        && app.spinner_frame.is_multiple_of(12)
        && let Some(ref team_name) = app.team_context.team_name
    {
        let team_name = team_name.clone();
        let tx_inbox = tx.clone();
        tokio::spawn(async move {
            let messages = crate::swarm::runner::poll_leader_inbox(&team_name).await;
            for msg in messages {
                // Hand off to the main thread which has
                // mutable access to `app.messages` —
                // injects into the transcript AND shows
                // a toast in one place. Mirrors v126's
                // `<teammate-message>` injection.
                let _ = tx_inbox
                    .send(AppEvent::Team(TeamEvent::Inbox {
                        from: msg.from,
                        text: msg.text,
                        summary: msg.summary,
                    }))
                    .await;
            }
        });
    }

    needs_draw
}

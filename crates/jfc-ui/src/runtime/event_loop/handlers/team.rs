//! `TeamEvent::*` handlers — teammate lifecycle, inbox messages, and spawn
//! registration.

use crate::app::App;
use crate::runtime::{AppEvent, EventSender, TaskEvent, TeamEvent};
use crate::types::*;
use crate::{session, toast};

/// Dispatch a single `TeamEvent` variant.
pub(crate) async fn handle_team_event(app: &mut App, tx: &EventSender, ev: TeamEvent) {
    match ev {
        TeamEvent::Runner(teammate_ev) => handle_runner(app, tx, teammate_ev).await,
        TeamEvent::Inbox {
            from,
            text,
            summary,
        } => handle_inbox(app, tx, from, text, summary).await,
        TeamEvent::Spawned {
            name,
            team_name,
            agent_id,
            color,
            agent_type,
            cwd,
            abort_tx,
        } => handle_spawned(
            app, name, team_name, agent_id, color, agent_type, cwd, abort_tx,
        ),
    }
}

async fn handle_runner(
    app: &mut App,
    tx: &EventSender,
    teammate_ev: crate::swarm::runner::TeammateEvent,
) {
    use crate::swarm::runner::TeammateEvent;
    match teammate_ev {
        TeammateEvent::Idle {
            task_id,
            agent_id: _,
            agent_name,
            reason,
            summary,
        } => {
            tracing::info!("[Swarm] Teammate {agent_name} went idle (reason: {reason:?})");
            // Mark the BackgroundTask Idle so the task
            // panel stops showing "Receiving output…" forever
            // and the subagent tree can render the agent
            // dimmer. Without this transition the panel
            // pinned to the bottom looking alive even
            // though the teammate had already sent its
            // message and stopped producing chunks.
            if let Some(bt) = app.background_tasks.get_mut(&task_id) {
                if matches!(bt.status, crate::types::TaskLifecycle::Running) {
                    bt.status = crate::types::TaskLifecycle::Idle;
                }
                bt.last_tool = None;
            }
            // Surface to the user as a toast — without this
            // the user has no way to tell that a teammate
            // finished its turn and is waiting. Summary
            // (when present) is the model's own one-line
            // recap, which reads better than the raw reason.
            let msg = match (summary.as_deref(), reason.as_deref()) {
                (Some(s), _) if !s.is_empty() => {
                    format!("⏸ {agent_name} idle — {s}")
                }
                (_, Some(r)) if !r.is_empty() => {
                    format!("⏸ {agent_name} idle ({r})")
                }
                _ => format!("⏸ {agent_name} is idle"),
            };
            toast::push_with_cap(
                &mut app.toasts,
                toast::Toast::new(toast::ToastKind::Info, msg),
            );
        }
        TeammateEvent::Progress {
            task_id,
            agent_id: _,
            token_count,
            tool_use_count,
            last_tool,
            model_id,
            cost_usd,
        } => {
            // Aggregate teammate cost into the parent's usage map so
            // the status bar reflects actual spend across all agents.
            if let (Some(model), Some(cost)) = (&model_id, cost_usd) {
                let entry = app.usage_by_model.entry(model.clone()).or_default();
                entry.cost_usd = entry.cost_usd.map_or(Some(cost), |c| Some(c + cost));
            }

            // Update background task state for UI display.
            // Revive an Idle task back to Running — the agent
            // is producing tool-progress events again, so it
            // is no longer idle.
            if let Some(bt) = app.background_tasks.get_mut(&task_id) {
                if matches!(bt.status, crate::types::TaskLifecycle::Idle) {
                    bt.status = crate::types::TaskLifecycle::Running;
                }
                bt.last_tool = last_tool;
                // The teammate event already gives us a
                // single combined token figure; route it
                // into `latest_input_tokens` so the fan UI
                // shows it without overwriting the
                // per-turn output sum. (Teammates don't
                // emit input/output separately yet.)
                bt.latest_input_tokens = token_count;
                bt.tool_use_count = tool_use_count as u32;

                // v132 per-agent token budget enforcement.
                // When the agent's total tokens exceed
                // its configured ceiling, set its status
                // to Failed and surface a kill toast
                // exactly once. We don't actually SIGKILL
                // the in-flight tokio task here — that
                // requires the swarm interrupt path —
                // but the fan UI / approval flow uses
                // `bt.status` to decide whether to keep
                // accepting work, so flipping it stops
                // the bleed.
                if let (Some(cap), false) = (bt.max_input_tokens, bt.budget_killed) {
                    let total = bt.latest_input_tokens + bt.cumulative_output_tokens;
                    if total > cap {
                        bt.budget_killed = true;
                        bt.status = crate::types::TaskLifecycle::Failed;
                        bt.completed_at = Some(std::time::Instant::now());
                        bt.error = Some(format!(
                            "killed: token budget {cap} exceeded ({total} used)"
                        ));
                        let agent = bt
                            .description
                            .lines()
                            .next()
                            .unwrap_or(bt.task_id.as_str())
                            .to_owned();
                        let total_for_msg = total;
                        toast::push_with_cap(
                            &mut app.toasts,
                            toast::Toast::new(
                                toast::ToastKind::Error,
                                format!(
                                    "Agent {agent} killed: budget {cap} exceeded ({total_for_msg} tokens)"
                                ),
                            ),
                        );
                    }
                }
            }
            // Mark this teammate as the live one for the
            // spinner-area tree highlight.
            app.last_active_agent_task = Some(task_id);
        }
        TeammateEvent::TextDelta {
            task_id,
            agent_id: _,
            delta,
        } => {
            // A new text delta means the teammate is producing
            // output again — revive Idle → Running so the
            // task panel resumes its "Receiving output…" spinner.
            if let Some(bt) = app.background_tasks.get_mut(&task_id)
                && matches!(bt.status, crate::types::TaskLifecycle::Idle)
            {
                bt.status = crate::types::TaskLifecycle::Running;
            }
            // Translate to AgentChunk so the existing
            // chunk handler (with coalescing rules and
            // BackgroundTask.messages append) handles it
            // — same path as one-shot subagents.
            let _ = tx
                .send(AppEvent::Task(TaskEvent::AgentChunk {
                    task_id: crate::ids::TaskId::from(task_id),
                    text: delta,
                }))
                .await;
        }
        TeammateEvent::Completed { task_id, agent_id } => {
            tracing::info!("[Swarm] Teammate {agent_id} completed");
            if let Some(bt) = app.background_tasks.get_mut(&task_id) {
                bt.status = crate::types::TaskLifecycle::Completed;
                bt.completed_at = Some(std::time::Instant::now());
            }
            // Mark the member inactive on the team file so a
            // later `set_member_active(true)` (e.g. an agent
            // that gets re-spawned) can observe the prior
            // state and the roster reflects who's currently
            // running.
            if let Some(team_name) = app.team_context.team_name.clone() {
                // agent_id is "name@team" — `set_member_active`
                // matches on the bare name field.
                let member_name = agent_id
                    .split_once('@')
                    .map(|(n, _)| n.to_owned())
                    .unwrap_or_else(|| agent_id.clone());
                tokio::spawn(async move {
                    let _ = crate::swarm::team_helpers::set_member_active(
                        &team_name,
                        &member_name,
                        false,
                    )
                    .await;
                });
            }
        }
        TeammateEvent::Failed {
            task_id,
            agent_id,
            error,
        } => {
            tracing::warn!("[Swarm] Teammate {agent_id} failed: {error}");
            if let Some(bt) = app.background_tasks.get_mut(&task_id) {
                bt.status = crate::types::TaskLifecycle::Failed;
                bt.completed_at = Some(std::time::Instant::now());
                bt.error = Some(error);
            }
            if let Some(team_name) = app.team_context.team_name.clone() {
                let member_name = agent_id
                    .split_once('@')
                    .map(|(n, _)| n.to_owned())
                    .unwrap_or_else(|| agent_id.clone());
                tokio::spawn(async move {
                    let _ = crate::swarm::team_helpers::set_member_active(
                        &team_name,
                        &member_name,
                        false,
                    )
                    .await;
                });
            }
        }
        TeammateEvent::Cancelled { task_id, agent_id } => {
            tracing::info!("[Swarm] Teammate {agent_id} cancelled");
            if let Some(bt) = app.background_tasks.get_mut(&task_id) {
                bt.status = crate::types::TaskLifecycle::Cancelled;
                bt.completed_at = Some(std::time::Instant::now());
            }
            if let Some(team_name) = app.team_context.team_name.clone() {
                let member_name = agent_id
                    .split_once('@')
                    .map(|(n, _)| n.to_owned())
                    .unwrap_or_else(|| agent_id.clone());
                tokio::spawn(async move {
                    let _ = crate::swarm::team_helpers::set_member_active(
                        &team_name,
                        &member_name,
                        false,
                    )
                    .await;
                });
            }
        }
        TeammateEvent::MessageSent {
            from,
            to,
            text,
            summary,
        } => {
            tracing::info!("[Swarm] Message from {from} → {to}");
            // Route the outbound message to the recipient's
            // mailbox so its polling loop picks it up. Mirrors
            // v126's `sendMessageToTeammate` (cli.js around
            // 396870) — the producing teammate writes; the
            // recipient consumes via `read_mailbox`. Without
            // this, the SendMessage tool was a no-op past
            // logging.
            let team_name = app.team_context.team_name.clone().unwrap_or_default();
            if team_name.is_empty() {
                tracing::warn!("[Swarm] MessageSent dropped — no active team_context");
            } else {
                let recipient = to.clone();
                let msg = crate::swarm::types::MailboxMessage {
                    from: from.clone(),
                    text: text.clone(),
                    timestamp: chrono::Utc::now().to_rfc3339(),
                    color: None,
                    summary: summary.clone(),
                    read: false,
                };
                tokio::spawn(async move {
                    if let Err(e) =
                        crate::swarm::mailbox::write_to_mailbox(&recipient, msg, &team_name).await
                    {
                        tracing::warn!("[Swarm] Failed to deliver message {from} → {to}: {e}");
                    }
                });
            }
        }
    }
}

async fn handle_inbox(
    app: &mut App,
    tx: &EventSender,
    from: String,
    text: String,
    summary: Option<String>,
) {
    // Append the teammate's message to the transcript as a
    // user-role turn tagged with the teammate's name so it
    // survives session save/load and the model sees it on
    // its next request. v126 wraps these in a
    // `<teammate-message from="…">…</teammate-message>` XML
    // block; we use the same shape so the leader's system
    // prompt rules for parsing teammate messages still
    // apply.
    let body = format!(
        "<teammate-message from=\"{}\">\n{}\n</teammate-message>",
        from, text
    );
    let mut msg = ChatMessage::user(body);
    msg.agent_name = Some(from.clone());
    app.messages.push(msg);
    // Also surface a brief toast so the user notices the
    // arrival without needing to scroll the transcript.
    let preview = summary
        .as_deref()
        .filter(|s| !s.is_empty())
        .map(|s| s.to_owned())
        .unwrap_or_else(|| {
            // Snap to a char boundary so multi-byte chars
            // (emoji, accented) at byte 60 don't panic.
            let mut cap = text.len().min(60);
            while cap > 0 && !text.is_char_boundary(cap) {
                cap -= 1;
            }
            text[..cap].to_owned()
        });
    toast::push_with_cap(
        &mut app.toasts,
        toast::Toast::new(toast::ToastKind::Info, format!("{from}: {preview}")),
    );
    // Persist so a session reload doesn't lose the message.
    if let Some(ref session_id) = app.current_session_id {
        let sid = session_id.clone();
        let msgs = app.messages.clone();
        let cwd = app.cwd.clone();
        let model = app.model.clone();
        tokio::spawn(async move {
            session::save_session(&sid, &msgs, Some(cwd.as_str()), Some(model.as_str())).await;
        });
    }

    // Wake the leader so it actually processes the teammate's message instead
    // of leaving it to sit until the next manual user prompt. Only when idle:
    // if a turn is already streaming (or tools/approvals are pending) the
    // message is already in `app.messages` and the in-flight turn picks it up
    // on its next request — and the `is_streaming` gate means concurrent
    // arrivals don't each spawn a duplicate stream. Treat this as a fresh
    // turn (reset the agentic-turn counter) since it's a new external input.
    let leader_idle = !app.is_streaming
        && app.pending_approval.is_none()
        && app.pending_tool_calls.is_empty()
        && app.approval_queue.is_empty();
    if leader_idle {
        tracing::info!(
            target: "jfc::swarm",
            from = %from,
            "leader idle — waking to process inbound teammate message"
        );
        app.agentic_turn_count = 0;
        crate::stream::continue_agentic_loop(app, tx).await;
    }
}

fn handle_spawned(
    app: &mut App,
    name: String,
    team_name: String,
    agent_id: String,
    color: Option<String>,
    agent_type: Option<String>,
    cwd: String,
    abort_tx: Option<tokio::sync::watch::Sender<bool>>,
) {
    // Activate the team if this is the first teammate to
    // join — switches the leader from "no team" to "running
    // a team" so the teammate tree, send-message routing,
    // and per-team context all light up.
    if app.team_context.team_name.is_none() {
        app.team_context.team_name = Some(team_name.clone());
        app.team_context.team_file_path =
            Some(crate::swarm::team_helpers::team_file_path(&team_name));
        app.team_context.lead_agent_id = Some(crate::swarm::types::make_agent_id(
            crate::swarm::TEAM_LEAD_NAME,
            &team_name,
        ));
        // Activate the team task store. Migrate any tasks
        // already created in the session store so IDs
        // remain valid — the leader frequently TaskCreates
        // a plan before the first teammate spawn, and
        // those IDs would otherwise vanish at team
        // activation. See `TaskStore::migrate_from`.
        let team_store = jfc_session::TaskStore::open_team(&team_name);
        let _ = team_store.migrate_from(&app.task_store);
        app.task_store = team_store;
    }
    // Register the teammate in the in-memory roster. The
    // render code reads this to draw the teammate tree and
    // power per-name lookups; previously the HashMap stayed
    // empty regardless of how many teammates spawned. The
    // `abort_tx` is critical — it keeps the runner's
    // watch channel alive for the teammate's lifetime.
    // Without storing it here, every teammate was marked
    // "Done" on its first poll.
    app.team_context.teammates.insert(
        agent_id.clone(),
        crate::swarm::types::TeammateInfo {
            name: name.clone(),
            agent_type,
            color,
            cwd,
            spawned_at: std::time::Instant::now(),
            backend: crate::swarm::types::BackendType::InProcess,
            abort_tx,
        },
    );
}

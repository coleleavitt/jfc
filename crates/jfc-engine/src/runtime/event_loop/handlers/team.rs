//! `TeamEvent::*` handlers — teammate lifecycle, inbox messages, and spawn
//! registration.

use crate::app::EngineState;
use crate::runtime::{EngineEvent, EventSender, TaskEvent, TeamEvent};
use crate::toast;
use crate::types::*;

/// Dispatch a single `TeamEvent` variant.
pub async fn handle_team_event(state: &mut EngineState, tx: &EventSender, ev: TeamEvent) {
    match ev {
        TeamEvent::Runner(teammate_ev) => handle_runner(state, tx, teammate_ev).await,
        TeamEvent::Inbox {
            from,
            text,
            formatted,
            color,
            summary,
        } => handle_inbox(state, tx, from, text, formatted, color, summary).await,
        TeamEvent::Spawned {
            name,
            team_name,
            agent_id,
            color,
            agent_type,
            cwd,
            backend_type,
            abort_tx,
        } => handle_spawned(
            state,
            name,
            team_name,
            agent_id,
            color,
            agent_type,
            cwd,
            backend_type,
            abort_tx,
        ),
    }
}

async fn handle_runner(
    state: &mut EngineState,
    tx: &EventSender,
    teammate_ev: crate::swarm::runner::TeammateEvent,
) {
    use crate::swarm::runner::TeammateEvent;

    // Mirror the teammate lifecycle into the unified agent registry so the
    // shared roster (and `wait`/`abort`) stay in sync with the legacy
    // `background_tasks` map below. Cheap, fire-and-forget, and keyed by the
    // event's `agent_id`, which the teammate was registered under at spawn.
    {
        let backend = crate::agents::TeamBackend::new(crate::tools::agent_registry().clone());
        backend.apply(&teammate_ev).await;
    }

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
            if let Some(bt) = state.background_tasks.get_mut(&task_id) {
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
                &mut state.toasts,
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
                let entry = state.usage_by_model.entry(model.clone()).or_default();
                entry.cost_usd = entry.cost_usd.map_or(Some(cost), |c| Some(c + cost));
            }

            // Update background task state for UI display.
            // Revive an Idle task back to Running — the agent
            // is producing tool-progress events again, so it
            // is no longer idle.
            if let Some(bt) = state.background_tasks.get_mut(&task_id) {
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
                            &mut state.toasts,
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
            state.last_active_agent_task = Some(task_id);
        }
        TeammateEvent::TextDelta {
            task_id,
            agent_id: _,
            delta,
        } => {
            // A new text delta means the teammate is producing
            // output again — revive Idle → Running so the
            // task panel resumes its "Receiving output…" spinner.
            if let Some(bt) = state.background_tasks.get_mut(&task_id)
                && matches!(bt.status, crate::types::TaskLifecycle::Idle)
            {
                bt.status = crate::types::TaskLifecycle::Running;
            }
            // Translate to AgentChunk so the existing
            // chunk handler (with coalescing rules and
            // BackgroundTask.messages append) handles it
            // — same path as one-shot subagents.
            let _ = tx
                .send(EngineEvent::Task(TaskEvent::AgentChunk {
                    task_id: crate::ids::TaskId::from(task_id),
                    text: delta,
                }))
                .await;
        }
        TeammateEvent::Completed { task_id, agent_id } => {
            tracing::info!("[Swarm] Teammate {agent_id} completed");
            if let Some(bt) = state.background_tasks.get_mut(&task_id) {
                bt.status = crate::types::TaskLifecycle::Completed;
                bt.completed_at = Some(std::time::Instant::now());
            }
            mark_runtime_teammate_inactive(state, &agent_id);
            // Mark the member inactive on the team file so a
            // later `set_member_active(true)` (e.g. an agent
            // that gets re-spawned) can observe the prior
            // state and the roster reflects who's currently
            // running.
            if let Some(team_name) = state.team_context.team_name.clone() {
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
            if let Some(bt) = state.background_tasks.get_mut(&task_id) {
                bt.status = crate::types::TaskLifecycle::Failed;
                bt.completed_at = Some(std::time::Instant::now());
                bt.error = Some(error);
            }
            mark_runtime_teammate_inactive(state, &agent_id);
            if let Some(team_name) = state.team_context.team_name.clone() {
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
            if let Some(bt) = state.background_tasks.get_mut(&task_id) {
                bt.status = crate::types::TaskLifecycle::Cancelled;
                bt.completed_at = Some(std::time::Instant::now());
            }
            mark_runtime_teammate_inactive(state, &agent_id);
            if let Some(team_name) = state.team_context.team_name.clone() {
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
            task_id,
            agent_id,
            from,
            to,
            text,
            summary,
        } => {
            tracing::info!("[Swarm] Message from {from} → {to}");
            if let Some(bt) = state.background_tasks.get_mut(&task_id) {
                record_teammate_sent_message(bt, &from, &to, &text, summary.as_deref());
            } else {
                tracing::debug!(
                    target: "jfc::swarm",
                    task_id = %task_id,
                    agent_id = %agent_id,
                    from = %from,
                    to = %to,
                    "teammate sent message but no background task transcript was found"
                );
            }
        }
    }
}

fn record_teammate_sent_message(
    bt: &mut crate::app::BackgroundTask,
    from: &str,
    to: &str,
    text: &str,
    summary: Option<&str>,
) {
    bt.last_activity_at = std::time::Instant::now();
    let body = match summary.filter(|s| !s.is_empty()) {
        Some(summary) => format!("Message to @{to}: {summary}\n\n{text}"),
        None => format!("Message to @{to}:\n\n{text}"),
    };
    bt.push_log(body.clone());
    let mut msg = ChatMessage::assistant(body);
    msg.agent_name = Some(from.to_owned());
    bt.push_chat(msg);
}

fn mark_runtime_teammate_inactive(state: &mut EngineState, agent_id: &str) {
    if let Some(teammate) = state.team_context.teammates.get_mut(agent_id) {
        teammate.abort_tx = None;
    }
}

async fn handle_inbox(
    state: &mut EngineState,
    tx: &EventSender,
    from: String,
    text: String,
    formatted: String,
    color: Option<String>,
    summary: Option<String>,
) {
    tracing::info!(
        target: "jfc::swarm",
        from = %from,
        has_color = color.is_some(),
        has_summary = summary.as_deref().is_some_and(|s| !s.is_empty()),
        text_chars = text.chars().count(),
        formatted_chars = formatted.chars().count(),
        "leader inbox received teammate message"
    );
    let body = formatted;
    let mut msg = ChatMessage::user(body);
    msg.agent_name = Some(from.clone());
    state.messages.push(msg);
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
        &mut state.toasts,
        toast::Toast::new(toast::ToastKind::Info, format!("{from}: {preview}")),
    );
    // Persist so a session reload doesn't lose the message (debounced —
    // a teammate burst shouldn't deep-clone the transcript per message).
    crate::runtime::session_save::request_save(state);

    // Wake the leader so it actually processes the teammate's message instead
    // of leaving it to sit until the next manual user prompt. Only when idle:
    // if a turn is already streaming (or tools/approvals are pending) the
    // message is already in `state.messages` and the in-flight turn picks it up
    // on its next request — and the `is_streaming` gate means concurrent
    // arrivals don't each spawn a duplicate stream. Treat this as a fresh
    // turn (reset the agentic-turn counter) since it's a new external input.
    let leader_idle = leader_ready_for_inbox_wake(state);
    if leader_idle {
        tracing::info!(
            target: "jfc::swarm",
            from = %from,
            "leader idle — waking to process inbound teammate message"
        );
        state.agentic_turn_count = 0;
        crate::stream::continue_agentic_loop(state, tx).await;
    } else {
        tracing::debug!(
            target: "jfc::swarm",
            from = %from,
            is_streaming = state.is_streaming,
            pending_approval = state.pending_approval.is_some(),
            approval_queue = state.approval_queue.len(),
            pending_tool_calls = state.pending_tool_calls.len(),
            pending_classifications = state.pending_classifications,
            in_flight_eager_dispatches = state.in_flight_eager_dispatches,
            in_flight_tool_batches = state.in_flight_tool_batches,
            compacting = state.compacting_started_at.is_some(),
            pending_elicitations = state.pending_elicitations.len(),
            "leader busy — inbound teammate message queued in transcript"
        );
    }
}

fn leader_ready_for_inbox_wake(state: &EngineState) -> bool {
    !state.is_streaming
        && state.pending_approval.is_none()
        && state.approval_queue.is_empty()
        && state.pending_tool_calls.is_empty()
        && state.pending_classifications == 0
        && state.in_flight_eager_dispatches == 0
        && state.in_flight_tool_batches == 0
        && state.compacting_started_at.is_none()
        && state.pending_elicitations.is_empty()
        // An open AskUserQuestion modal keeps the leader paused: a teammate
        // inbox wake must not continue the loop until the user has answered.
        && state.pending_question.is_none()
}

fn handle_spawned(
    state: &mut EngineState,
    name: String,
    team_name: String,
    agent_id: String,
    color: Option<String>,
    agent_type: Option<String>,
    cwd: String,
    backend_type: crate::swarm::types::BackendType,
    abort_tx: Option<tokio::sync::watch::Sender<bool>>,
) {
    // Activate the team if this is the first teammate to
    // join — switches the leader from "no team" to "running
    // a team" so the teammate tree, send-message routing,
    // and per-team context all light up.
    if state.team_context.team_name.is_none() {
        state.team_context.team_name = Some(team_name.clone());
        state.team_context.team_file_path =
            Some(crate::swarm::team_helpers::team_file_path(&team_name));
        state.team_context.lead_agent_id = Some(crate::swarm::types::make_agent_id(
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
        let _ = team_store.migrate_from(&state.task_store);
        state.task_store = team_store;
    }
    // Register the teammate in the in-memory roster. The
    // render code reads this to draw the teammate tree and
    // power per-name lookups; previously the HashMap stayed
    // empty regardless of how many teammates spawned. The
    // `abort_tx` is critical — it keeps the runner's
    // watch channel alive for the teammate's lifetime.
    // Without storing it here, every teammate was marked
    // "Done" on its first poll.
    state.team_context.teammates.insert(
        agent_id,
        crate::swarm::types::TeammateInfo {
            name,
            agent_type,
            color,
            cwd,
            spawned_at: std::time::Instant::now(),
            backend: backend_type,
            abort_tx,
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn background_task(id: &str) -> crate::app::BackgroundTask {
        crate::app::BackgroundTask {
            task_id: crate::ids::TaskId::from(id.to_owned()),
            description: "import-fix teammate".to_owned(),
            status: crate::types::TaskLifecycle::Running,
            started_at: std::time::Instant::now(),
            completed_at: None,
            summary: None,
            error: None,
            last_tool: None,
            last_tool_info: None,
            recent_activities: Vec::new(),
            messages: Vec::new(),
            chat_messages: Vec::new(),
            tool_use_count: 0,
            latest_input_tokens: 0,
            latest_cache_read_tokens: 0,
            latest_cache_write_tokens: 0,
            cumulative_output_tokens: 0,
            model_used: None,
            agent_messages: Vec::new(),
            max_input_tokens: None,
            budget_killed: false,
            parent_task_id: None,
            workflow_progress: None,
            last_activity_at: std::time::Instant::now(),
        }
    }

    #[test]
    fn teammate_sent_message_records_task_transcript_normal() {
        let mut bt = background_task("teammate-import-fix@hiddify");

        record_teammate_sent_message(
            &mut bt,
            "import-fix",
            "team-lead",
            "fixed the Dart cluster",
            Some("Dart cluster complete"),
        );

        assert_eq!(bt.messages.len(), 1);
        assert!(bt.messages[0].contains("Message to @team-lead: Dart cluster complete"));
        assert!(bt.messages[0].contains("fixed the Dart cluster"));
        assert_eq!(bt.chat_messages.len(), 1);
        assert_eq!(
            bt.chat_messages[0].agent_name.as_deref(),
            Some("import-fix")
        );
        let text = bt.chat_messages[0]
            .parts
            .iter()
            .filter_map(|part| match part {
                crate::types::MessagePart::Text(text) => Some(text.as_str()),
                _ => None,
            })
            .collect::<String>();
        assert!(text.contains("fixed the Dart cluster"));
    }
}

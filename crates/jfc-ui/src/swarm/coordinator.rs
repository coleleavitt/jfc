//! Fan-out / fan-in coordination for in-process teammates.
//!
//! Implements the main agent loop (`run_teammate_loop`) and the polling
//! primitives used to drive it:
//!
//! - `run_teammate_loop` — outer loop: run turn → go idle → wait for message → repeat
//! - `poll_for_next_message` — file-based mailbox polling with priority ordering
//! - `check_task_list_for_work` — auto-claim unblocked tasks from the shared task list

use std::time::Duration;

use tokio::sync::watch;
use tracing::debug;

use super::TEAM_LEAD_NAME;
use super::mailbox;
use super::runner::{PollResult, TeammateEvent, TeammateRunnerConfig, TeammateExit};
use super::types::*;

// ─── Main agent loop ─────────────────────────────────────────────────────────

/// Main loop for an in-process teammate.
///
/// This mirrors v126's `runInProcessTeammate` in `inProcessRunner.ts`.
/// The actual API streaming is delegated to the leader's provider — the
/// teammate sends its prompts through an internal channel and receives
/// streamed responses.
///
/// Loop shape:
/// 1. Format the initial prompt as a teammate-message
/// 2. Run agent turn (stream → tools)
/// 3. Go idle — send idle notification
/// 4. Poll for next message (mailbox or task list)
/// 5. On shutdown request → route through permission_sync
/// 6. On abort signal → immediately exit loop
pub(super) async fn run_teammate_loop(
    config: TeammateRunnerConfig,
    mut abort_rx: watch::Receiver<bool>,
    event_tx: tokio::sync::mpsc::UnboundedSender<TeammateEvent>,
) -> anyhow::Result<TeammateExit> {
    let identity = &config.identity;
    let task_id = format!("teammate-{}", identity.agent_id);

    debug!(
        "[InProcessRunner] Starting agent loop for {} (team: {})",
        identity.agent_name, identity.team_name
    );

    // Format initial prompt as a teammate message from the leader
    let mut current_prompt = format_teammate_message(
        TEAM_LEAD_NAME,
        &config.prompt,
        None,
        Some(&config.description),
    );

    let mut iteration = 0u64;
    let mut conversation_history: Vec<jfc_provider::ProviderMessage> = Vec::new();
    let mut active_task_id: Option<String> = None;

    let exit_reason: TeammateExit = loop {
        // Check abort before processing
        if *abort_rx.borrow() {
            debug!(
                "[InProcessRunner] {} aborted before iteration",
                identity.agent_name
            );
            break TeammateExit::Cancelled;
        }

        iteration += 1;
        debug!(
            "[InProcessRunner] {} iteration {iteration}, prompt len={}",
            identity.agent_name,
            current_prompt.len()
        );

        // ─── Run agent turn ──────────────────────────────────────────────
        // Build messages, stream from provider, execute tools in a loop
        // until the model returns EndTurn (no more tool calls).

        let turn_result = super::executor::run_single_turn(
            &config,
            &current_prompt,
            &mut conversation_history,
            &event_tx,
            &task_id,
            &mut abort_rx,
        )
        .await;

        match turn_result {
            super::executor::TurnResult::Completed {
                token_count,
                tool_count,
                last_tool,
            } => {
                if let (Some(store), Some(task_id)) =
                    (config.task_store.as_ref(), active_task_id.take())
                {
                    let _ = store.update(
                        &task_id,
                        jfc_session::TaskPatch {
                            status: Some(jfc_session::TaskStatus::Completed),
                            ..Default::default()
                        },
                    );
                }
                let _ = event_tx.send(TeammateEvent::Progress {
                    task_id: task_id.clone(),
                    agent_id: identity.agent_id.clone(),
                    token_count,
                    tool_use_count: tool_count,
                    last_tool,
                    model_id: Some(config.model_id.as_str().to_owned()),
                    cost_usd: None,
                });
            }
            super::executor::TurnResult::Aborted => {
                debug!("[InProcessRunner] {} turn aborted", identity.agent_name);
                break TeammateExit::Cancelled;
            }
            super::executor::TurnResult::Error(e) => {
                tracing::warn!("[InProcessRunner] {} turn error: {e}", identity.agent_name);
                // Continue to idle — don't crash the teammate
            }
        }

        // ─── Go idle ─────────────────────────────────────────────────────
        let _ = event_tx.send(TeammateEvent::Idle {
            task_id: task_id.clone(),
            agent_id: identity.agent_id.clone(),
            agent_name: identity.agent_name.clone(),
            reason: Some("available".to_owned()),
            summary: None,
        });

        // Send idle notification to leader's mailbox
        let _ = mailbox::send_idle_notification(
            &identity.agent_name,
            identity.color.as_deref(),
            &identity.team_name,
            Some("available"),
            None,
        )
        .await;

        debug!(
            "[InProcessRunner] {} waiting for next message",
            identity.agent_name
        );

        // ─── Poll for next message ───────────────────────────────────────
        let poll_result =
            poll_for_next_message(identity, config.task_store.clone(), &mut abort_rx).await;

        match poll_result {
            PollResult::NewMessage {
                message,
                from,
                color,
                summary,
            } => {
                debug!(
                    "[InProcessRunner] {} received message from {from}",
                    identity.agent_name
                );
                if from == "user" {
                    current_prompt = message;
                } else {
                    current_prompt = format_teammate_message(
                        &from,
                        &message,
                        color.as_deref(),
                        summary.as_deref(),
                    );
                }
            }
            PollResult::ShutdownRequest {
                request,
                original_message: _,
            } => {
                debug!(
                    "[InProcessRunner] {} received shutdown request",
                    identity.agent_name
                );
                // Route the shutdown through the same permission_sync
                // protocol that gates plan-mode tool calls. The leader
                // either auto-approves (Bypass / Default-non-mutation)
                // or escalates to the user via a `/swarm-approve` toast.
                // Auto-approving shutdowns blindly meant any teammate
                // could exit unilaterally — even mid-task.
                let req = super::permission_sync::create_permission_request(
                    "shutdown",
                    "shutdown",
                    serde_json::json!({
                        "reason": request.as_ref().and_then(|r| r.reason.clone()).unwrap_or_default(),
                    }),
                    &format!(
                        "Teammate {} requests graceful shutdown",
                        identity.agent_name
                    ),
                    &identity.agent_id,
                    &identity.agent_name,
                    identity.color.as_deref(),
                    &identity.team_name,
                );
                let request_id = req.id.clone();
                if let Err(e) = super::permission_sync::write_permission_request(&req).await {
                    tracing::warn!(
                        target: "jfc::swarm::coordinator",
                        error = %e,
                        "failed to write shutdown request — staying alive"
                    );
                    continue;
                }
                let resolved = super::permission_sync::poll_for_response(
                    &request_id,
                    &identity.team_name,
                    std::time::Duration::from_secs(300),
                )
                .await;
                let approved = matches!(
                    resolved.as_ref().map(|r| r.status),
                    Some(super::types::PermissionRequestStatus::Approved)
                );
                if approved {
                    debug!(
                        "[InProcessRunner] {} shutdown approved — exiting",
                        identity.agent_name
                    );
                    break TeammateExit::Completed;
                }
                debug!(
                    "[InProcessRunner] {} shutdown denied — staying alive",
                    identity.agent_name
                );
            }
            PollResult::TaskAvailable { task_id, prompt } => {
                debug!(
                    "[InProcessRunner] {} claimed task from task list",
                    identity.agent_name
                );
                active_task_id = Some(task_id);
                current_prompt =
                    format_teammate_message("task-list", &prompt, None, Some("auto-claimed task"));
            }
            PollResult::Aborted => {
                debug!(
                    "[InProcessRunner] {} aborted while waiting",
                    identity.agent_name
                );
                break TeammateExit::Cancelled;
            }
        }
    };

    Ok(exit_reason)
}

// ─── Mailbox polling ─────────────────────────────────────────────────────────

/// Poll for the next message or signal. Checks:
/// 1. Abort signal (highest priority)
/// 2. File-based mailbox (shutdown requests prioritized, then leader, then any)
/// 3. Repeats every POLL_INTERVAL_MS
pub(super) async fn poll_for_next_message(
    identity: &TeammateIdentity,
    task_store: Option<std::sync::Arc<jfc_session::TaskStore>>,
    abort_rx: &mut watch::Receiver<bool>,
) -> PollResult {
    let mut poll_count = 0u32;

    loop {
        // Check abort
        if *abort_rx.borrow() {
            return PollResult::Aborted;
        }

        // Wait before first poll to let messages arrive
        if poll_count > 0 {
            tokio::select! {
                _ = tokio::time::sleep(Duration::from_millis(super::POLL_INTERVAL_MS)) => {}
                _ = abort_rx.changed() => {
                    if *abort_rx.borrow() {
                        return PollResult::Aborted;
                    }
                }
            }
        }
        poll_count += 1;

        // Check abort again after sleep
        if *abort_rx.borrow() {
            return PollResult::Aborted;
        }

        // Read mailbox
        let messages = mailbox::read_mailbox(&identity.agent_name, &identity.team_name).await;

        // Priority 1: Shutdown requests
        for (idx, msg) in messages.iter().enumerate() {
            if !msg.read {
                if let Some(shutdown) = mailbox::parse_shutdown_request(&msg.text) {
                    let _ =
                        mailbox::mark_message_read(&identity.agent_name, &identity.team_name, idx)
                            .await;
                    return PollResult::ShutdownRequest {
                        request: Some(shutdown),
                        original_message: msg.text.clone(),
                    };
                }
            }
        }

        // Priority 2: Messages from leader
        for (idx, msg) in messages.iter().enumerate() {
            if !msg.read && msg.from == TEAM_LEAD_NAME {
                let _ = mailbox::mark_message_read(&identity.agent_name, &identity.team_name, idx)
                    .await;
                return PollResult::NewMessage {
                    message: msg.text.clone(),
                    from: msg.from.clone(),
                    color: msg.color.clone(),
                    summary: msg.summary.clone(),
                };
            }
        }

        // Priority 3: Any unread message
        for (idx, msg) in messages.iter().enumerate() {
            if !msg.read {
                let _ = mailbox::mark_message_read(&identity.agent_name, &identity.team_name, idx)
                    .await;
                return PollResult::NewMessage {
                    message: msg.text.clone(),
                    from: msg.from.clone(),
                    color: msg.color.clone(),
                    summary: msg.summary.clone(),
                };
            }
        }

        // Check task list for unclaimed work (auto-claiming)
        if let Some((task_id, prompt)) =
            check_task_list_for_work(identity, task_store.clone()).await
        {
            return PollResult::TaskAvailable { task_id, prompt };
        }

        // No messages found — continue polling
    }
}

// ─── Task list auto-claiming ─────────────────────────────────────────────────

/// Check the team's task list for an unblocked, unowned task to claim.
/// Returns a formatted prompt if a task was successfully claimed.
pub(super) async fn check_task_list_for_work(
    identity: &TeammateIdentity,
    store: Option<std::sync::Arc<jfc_session::TaskStore>>,
) -> Option<(String, String)> {
    let store = store.unwrap_or_else(|| jfc_session::TaskStore::open_team(&identity.team_name));
    let task = store.claim_next_available(&identity.agent_name)?;
    debug!(
        "[InProcessRunner] {} auto-claimed task #{}: {}",
        identity.agent_name, task.id, task.subject
    );

    let subject = if task.subject.is_empty() {
        "(unnamed task)"
    } else {
        task.subject.as_str()
    };
    let mut prompt = format!(
        "Complete all open tasks. Start with task #{}:\n\n{}",
        task.id, subject
    );
    if !task.description.is_empty() {
        prompt.push_str(&format!("\n\n{}", task.description));
    }
    Some((task.id.to_string(), prompt))
}

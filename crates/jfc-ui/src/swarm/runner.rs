//! In-process teammate runner.
//!
//! Implements the agent loop for teammates running in the same process as the
//! leader. This is the primary execution mode (v126's `teammateMode: "in-process"`).
//!
//! Lifecycle:
//! 1. Spawned via `start_teammate()` — registers task state, starts background tokio task
//! 2. Runs the agent loop: send prompt → stream response → execute tools → repeat
//! 3. Goes idle after each turn — sends idle notification to leader
//! 4. Polls for next message (mailbox or pending_user_messages)
//! 5. On shutdown request → model decides approve/reject → if approved, exit loop
//! 6. On abort signal → immediately exit loop
//!
//! Communication:
//! - Receives work via `pending_user_messages` (fast path) or mailbox polling
//! - Sends results to leader via `SendMessage` tool calls
//! - Idle notifications are auto-delivered to leader's mailbox

use std::time::Duration;

use tokio::sync::{mpsc, watch};
use tracing::{debug, warn};

use super::mailbox;
use super::types::*;
use super::TEAM_LEAD_NAME;

/// Teammate colors matching v126's palette. Cycled through on each spawn.
const TEAMMATE_COLORS: &[&str] = &[
    "#4FC3F7", "#81C784", "#FFB74D", "#BA68C8", "#F06292",
    "#4DD0E1", "#AED581", "#FFD54F", "#7986CB", "#A1887F",
];

static COLOR_INDEX: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// Assign a color for a new teammate (cycles through palette).
pub fn assign_teammate_color() -> String {
    let idx = COLOR_INDEX.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    TEAMMATE_COLORS[idx % TEAMMATE_COLORS.len()].to_owned()
}

/// Configuration for starting an in-process teammate.
pub struct TeammateRunnerConfig {
    pub identity: TeammateIdentity,
    pub prompt: String,
    pub description: String,
    pub model: Option<String>,
    pub agent_type: Option<String>,
    /// Provider for API calls. Shared with the leader.
    pub provider: std::sync::Arc<dyn crate::provider::Provider>,
    /// Model ID to use for this teammate's API calls.
    pub model_id: crate::provider::ModelId,
    /// System prompt additions (agent-specific + teammate addendum).
    pub system_prompt: Option<String>,
}

/// Message types the runner can receive from its poll loop.
#[derive(Debug)]
pub enum PollResult {
    /// New message from leader or another teammate.
    NewMessage {
        message: String,
        from: String,
        color: Option<String>,
        summary: Option<String>,
    },
    /// Shutdown request received.
    ShutdownRequest {
        request: Option<ShutdownRequest>,
        original_message: String,
    },
    /// A task from the task list is available to claim.
    TaskAvailable { prompt: String },
    /// The teammate was aborted (lifecycle abort signal).
    Aborted,
}

/// Start an in-process teammate. Spawns a background tokio task that runs
/// the agent loop. Returns abort handle and task ID.
///
/// This is the main entry point called after spawn validation.
/// Single source of truth for the public task id of an in-process
/// teammate. The runner uses this internally; the dispatcher uses it
/// to register the matching `BackgroundTask` so streaming events
/// route correctly.
pub fn teammate_task_id(agent_id: &str) -> String {
    format!("teammate-{agent_id}")
}

pub fn start_teammate(
    config: TeammateRunnerConfig,
    event_tx: mpsc::UnboundedSender<TeammateEvent>,
) -> (String, watch::Sender<bool>) {
    let (abort_tx, abort_rx) = watch::channel(false);
    let task_id = format!("teammate-{}", &config.identity.agent_id);

    let identity = config.identity.clone();
    let task_id_clone = task_id.clone();

    tokio::spawn(async move {
        let result = run_teammate_loop(config, abort_rx, event_tx.clone()).await;

        match result {
            Ok(()) => {
                debug!(
                    "[InProcessRunner] Teammate {} completed normally",
                    identity.agent_name
                );
                let _ = event_tx.send(TeammateEvent::Completed {
                    task_id: task_id_clone,
                    agent_id: identity.agent_id,
                });
            }
            Err(e) => {
                warn!(
                    "[InProcessRunner] Teammate {} failed: {e}",
                    identity.agent_name
                );
                let _ = event_tx.send(TeammateEvent::Failed {
                    task_id: task_id_clone,
                    agent_id: identity.agent_id,
                    error: e.to_string(),
                });
            }
        }
    });

    (task_id, abort_tx)
}

/// Events emitted by the teammate runner back to the leader.
#[derive(Debug, Clone)]
pub enum TeammateEvent {
    /// Teammate has gone idle (finished processing, waiting for next message).
    Idle {
        task_id: String,
        agent_id: String,
        agent_name: String,
        reason: Option<String>,
        summary: Option<String>,
    },
    /// Teammate is actively processing (status update for UI).
    Progress {
        task_id: String,
        agent_id: String,
        token_count: u64,
        tool_use_count: u64,
        last_tool: Option<String>,
    },
    /// Teammate completed and exited its loop.
    Completed {
        task_id: String,
        agent_id: String,
    },
    /// Teammate encountered a fatal error.
    Failed {
        task_id: String,
        agent_id: String,
        error: String,
    },
    /// Teammate wants to send a message (goes through SendMessage tool).
    MessageSent {
        from: String,
        to: String,
        text: String,
        summary: Option<String>,
    },
    /// One streaming-text delta from the teammate's current turn.
    /// The main loop translates this into `AppEvent::AgentChunk` so
    /// the task panel fills live as the teammate streams. Without
    /// it, drilling into a running teammate showed "No messages yet"
    /// until the entire turn finished.
    TextDelta {
        task_id: String,
        agent_id: String,
        delta: String,
    },
}

/// Main loop for an in-process teammate.
///
/// This mirrors v126's `runInProcessTeammate` in `inProcessRunner.ts`.
/// The actual API streaming is delegated to the leader's provider — the
/// teammate sends its prompts through an internal channel and receives
/// streamed responses. For now this is a structural skeleton that:
/// 1. Formats the initial prompt as a teammate-message
/// 2. Enters the idle/poll loop
/// 3. Responds to shutdown/abort signals
///
/// Full integration with the streaming API and tool execution will connect
/// to the existing `stream.rs` / `scheduler.rs` infrastructure.
async fn run_teammate_loop(
    config: TeammateRunnerConfig,
    mut abort_rx: watch::Receiver<bool>,
    event_tx: mpsc::UnboundedSender<TeammateEvent>,
) -> anyhow::Result<()> {
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
    let mut conversation_history: Vec<crate::provider::ProviderMessage> = Vec::new();

    loop {
        // Check abort before processing
        if *abort_rx.borrow() {
            debug!("[InProcessRunner] {} aborted before iteration", identity.agent_name);
            break;
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

        let turn_result = run_single_turn(
            &config,
            &current_prompt,
            &mut conversation_history,
            &event_tx,
            &task_id,
            &mut abort_rx,
        )
        .await;

        match turn_result {
            TurnResult::Completed { token_count, tool_count, last_tool } => {
                let _ = event_tx.send(TeammateEvent::Progress {
                    task_id: task_id.clone(),
                    agent_id: identity.agent_id.clone(),
                    token_count,
                    tool_use_count: tool_count,
                    last_tool,
                });
            }
            TurnResult::Aborted => {
                debug!("[InProcessRunner] {} turn aborted", identity.agent_name);
                break;
            }
            TurnResult::Error(e) => {
                warn!("[InProcessRunner] {} turn error: {e}", identity.agent_name);
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

        debug!("[InProcessRunner] {} waiting for next message", identity.agent_name);

        // ─── Poll for next message ───────────────────────────────────────
        let poll_result = poll_for_next_message(identity, &mut abort_rx).await;

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
                    current_prompt =
                        format_teammate_message(&from, &message, color.as_deref(), summary.as_deref());
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
                        target: "jfc::swarm::runner",
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
                    break;
                }
                debug!(
                    "[InProcessRunner] {} shutdown denied — staying alive",
                    identity.agent_name
                );
            }
            PollResult::TaskAvailable { prompt } => {
                debug!(
                    "[InProcessRunner] {} claimed task from task list",
                    identity.agent_name
                );
                current_prompt = format_teammate_message(
                    "task-list",
                    &prompt,
                    None,
                    Some("auto-claimed task"),
                );
            }
            PollResult::Aborted => {
                debug!("[InProcessRunner] {} aborted while waiting", identity.agent_name);
                break;
            }
        }
    }

    Ok(())
}

/// Poll for the next message or signal. Checks:
/// 1. Abort signal (highest priority)
/// 2. File-based mailbox (shutdown requests prioritized, then leader, then any)
/// 3. Repeats every POLL_INTERVAL_MS
async fn poll_for_next_message(
    identity: &TeammateIdentity,
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
                    let _ = mailbox::mark_message_read(
                        &identity.agent_name,
                        &identity.team_name,
                        idx,
                    )
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
                let _ = mailbox::mark_message_read(
                    &identity.agent_name,
                    &identity.team_name,
                    idx,
                )
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
                let _ = mailbox::mark_message_read(
                    &identity.agent_name,
                    &identity.team_name,
                    idx,
                )
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
        if let Some(prompt) = check_task_list_for_work(identity).await {
            return PollResult::TaskAvailable { prompt };
        }

        // No messages found — continue polling
    }
}

/// Check the team's task list for an unblocked, unowned task to claim.
/// Returns a formatted prompt if a task was successfully claimed.
async fn check_task_list_for_work(identity: &TeammateIdentity) -> Option<String> {
    use crate::swarm::team_helpers;

    // Read the task file from the team's task directory
    let tasks_dir = team_helpers::tasks_dir(&identity.team_name);
    let tasks_file = tasks_dir.join("tasks.json");

    let content = tokio::fs::read_to_string(&tasks_file).await.ok()?;
    let tasks: Vec<serde_json::Value> = serde_json::from_str(&content).ok()?;

    // Find first pending, unowned, unblocked task
    let completed_ids: std::collections::HashSet<String> = tasks
        .iter()
        .filter(|t| t["status"].as_str() == Some("completed"))
        .filter_map(|t| t["id"].as_str().map(str::to_owned))
        .collect();

    for task in &tasks {
        let status = task["status"].as_str().unwrap_or("");
        if status != "pending" {
            continue;
        }
        // Skip if owned
        if task["owner"].as_str().is_some_and(|o| !o.is_empty()) {
            continue;
        }
        // Skip if blocked
        let blocked_by = task["blockedBy"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str())
                    .any(|id| !completed_ids.contains(id))
            })
            .unwrap_or(false);
        if blocked_by {
            continue;
        }

        // Found a claimable task — claim it by writing owner
        let task_id = task["id"].as_str()?;
        let subject = task["subject"].as_str().unwrap_or("(unnamed task)");
        let description = task["description"].as_str().unwrap_or("");

        // Attempt to claim by updating the file (simplified — no lock contention handling)
        let mut tasks_mut: Vec<serde_json::Value> = tasks.clone();
        for t in &mut tasks_mut {
            if t["id"].as_str() == Some(task_id) {
                t["owner"] = serde_json::Value::String(identity.agent_name.clone());
                t["status"] = serde_json::Value::String("in_progress".to_owned());
            }
        }
        if let Ok(json) = serde_json::to_string_pretty(&tasks_mut) {
            let _ = tokio::fs::write(&tasks_file, json).await;
        }

        debug!(
            "[InProcessRunner] {} auto-claimed task #{}: {}",
            identity.agent_name, task_id, subject
        );

        let mut prompt = format!("Complete all open tasks. Start with task #{task_id}:\n\n {subject}");
        if !description.is_empty() {
            prompt.push_str(&format!("\n\n{description}"));
        }
        return Some(prompt);
    }

    None
}

// ─── Agent Turn Execution ────────────────────────────────────────────────────

/// Result of running a single agent turn (one prompt → stream → tools cycle).
#[derive(Debug)]
enum TurnResult {
    Completed {
        token_count: u64,
        tool_count: u64,
        last_tool: Option<String>,
    },
    Aborted,
    Error(String),
}

/// Run a single turn: build messages, call the API, parse response, execute tools.
/// Returns when the model finishes (EndTurn) or an error/abort occurs.
async fn run_single_turn(
    config: &TeammateRunnerConfig,
    prompt: &str,
    history: &mut Vec<crate::provider::ProviderMessage>,
    event_tx: &tokio::sync::mpsc::UnboundedSender<TeammateEvent>,
    task_id: &str,
    abort_rx: &mut tokio::sync::watch::Receiver<bool>,
) -> TurnResult {
    use crate::provider::{
        ProviderContent, ProviderMessage, ProviderRole, StopReason, StreamEvent, StreamOptions,
    };
    use crate::tools;
    use crate::types::{ToolInput, ToolKind};
    use futures::StreamExt;

    let identity = &config.identity;
    let provider = &config.provider;
    let model = &config.model_id;

    // Build system prompt
    let mut system = String::new();
    if let Some(ref sp) = config.system_prompt {
        system.push_str(sp);
    }
    system.push_str(super::TEAMMATE_SYSTEM_PROMPT_ADDENDUM);

    // Add user message to history
    history.push(ProviderMessage {
        role: ProviderRole::User,
        content: vec![ProviderContent::Text(prompt.to_owned())],
    });

    let mut total_tokens: u64 = 0;
    let mut total_tools: u64 = 0;
    let mut last_tool_name: Option<String> = None;
    let max_turns = 25u32; // safety limit
    let mut turn = 0u32;

    loop {
        turn += 1;
        if turn > max_turns {
            return TurnResult::Error("max turns exceeded".into());
        }

        // Check abort
        if *abort_rx.borrow() {
            return TurnResult::Aborted;
        }

        // Build stream options
        let opts = StreamOptions::new(model.clone())
            .system(system.clone())
            .tools(tools::all_tool_defs());

        let stream = match provider.stream(history.clone(), &opts).await {
            Ok(s) => s,
            Err(e) => return TurnResult::Error(format!("provider stream error: {e}")),
        };

        let mut response_text = String::new();
        let mut tool_calls: Vec<(String, String, ToolKind, ToolInput, serde_json::Value)> = Vec::new();
        let mut stop_reason = StopReason::EndTurn;

        futures::pin_mut!(stream);
        while let Some(event_result) = stream.next().await {
            if *abort_rx.borrow() {
                return TurnResult::Aborted;
            }
            let event = match event_result {
                Ok(e) => e,
                Err(e) => return TurnResult::Error(format!("stream error: {e}")),
            };
            match event {
                StreamEvent::TextDelta { delta, .. } => {
                    total_tokens += (delta.len() / 4) as u64;
                    // Forward to the leader so the task panel for this
                    // teammate shows live output. The handler translates
                    // to `AppEvent::AgentChunk` keyed by `task_id`.
                    let _ = event_tx.send(TeammateEvent::TextDelta {
                        task_id: task_id.to_owned(),
                        agent_id: identity.agent_id.clone(),
                        delta: delta.clone(),
                    });
                    response_text.push_str(&delta);
                }
                StreamEvent::ToolDone { tool_name, tool_use_id, input_json, .. } => {
                    let input_value: serde_json::Value =
                        serde_json::from_str(&input_json).unwrap_or_default();
                    let kind = ToolKind::from_name(&tool_name);
                    let parsed_input = ToolInput::from_value(&tool_name, input_value.clone());
                    tool_calls.push((tool_use_id, tool_name.clone(), kind, parsed_input, input_value));
                    last_tool_name = Some(tool_name);
                }
                StreamEvent::Usage { input_tokens, output_tokens, .. } => {
                    total_tokens = (input_tokens + output_tokens) as u64;
                }
                StreamEvent::Done { stop_reason: r } => {
                    stop_reason = r;
                }
                StreamEvent::Error { message } => {
                    return TurnResult::Error(format!("stream error: {message}"));
                }
                _ => {}
            }
        }

        // Add assistant response to history
        let mut assistant_content = Vec::new();
        if !response_text.is_empty() {
            assistant_content.push(ProviderContent::Text(response_text.clone()));
        }
        for (id, name, _, _, input_val) in &tool_calls {
            assistant_content.push(ProviderContent::ToolUse {
                id: id.clone(),
                name: name.clone(),
                input: input_val.clone(),
            });
        }
        if !assistant_content.is_empty() {
            history.push(ProviderMessage {
                role: ProviderRole::Assistant,
                content: assistant_content,
            });
        }

        // If no tool calls, we're done with this turn
        if tool_calls.is_empty() {
            break;
        }

        // Execute tools
        let cwd = std::env::current_dir().unwrap_or_default();
        let mut tool_results: Vec<ProviderContent> = Vec::new();

        for (id, name, kind, input, raw_input) in &tool_calls {
            total_tools += 1;

            // Emit progress
            let _ = event_tx.send(TeammateEvent::Progress {
                task_id: task_id.to_owned(),
                agent_id: identity.agent_id.clone(),
                token_count: total_tokens,
                tool_use_count: total_tools,
                last_tool: Some(name.clone()),
            });

            // Permission gate: when the teammate is running with
            // `plan_mode_required = true`, no tool runs without the
            // leader's explicit OK. Mirrors v126's plan-mode where the
            // worker writes a `SwarmPermissionRequest` to the team's
            // pending dir and blocks on the leader to resolve it. We
            // only gate plan-mode here because a fully-trusted
            // teammate should run unchecked — the gate adds latency
            // and the leader has nothing to add for routine reads.
            if identity.plan_mode_required {
                let request = super::permission_sync::create_permission_request(
                    name,
                    id,
                    raw_input.clone(),
                    &format!("Teammate {} requests {}", identity.agent_name, name),
                    &identity.agent_id,
                    &identity.agent_name,
                    identity.color.as_deref(),
                    &identity.team_name,
                );
                let request_id = request.id.clone();
                if let Err(e) =
                    super::permission_sync::write_permission_request(&request).await
                {
                    tracing::warn!(
                        target: "jfc::swarm::runner",
                        error = %e,
                        "failed to write permission request — denying tool by default"
                    );
                    tool_results.push(ProviderContent::ToolResult {
                        tool_use_id: id.clone(),
                        content: format!(
                            "Permission request could not be written; tool '{name}' denied."
                        ),
                        is_error: true,
                    });
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
                if !approved {
                    let feedback = resolved
                        .as_ref()
                        .and_then(|r| r.feedback.clone())
                        .unwrap_or_else(|| "denied or timed out".to_owned());
                    tool_results.push(ProviderContent::ToolResult {
                        tool_use_id: id.clone(),
                        content: format!(
                            "Tool '{name}' was not approved by the leader: {feedback}"
                        ),
                        is_error: true,
                    });
                    continue;
                }
            }

            let result = tools::execute_tool(
                kind.clone(),
                input.clone(),
                cwd.clone(),
                None,
                None,
                None,
            )
            .await;

            tool_results.push(ProviderContent::ToolResult {
                tool_use_id: id.clone(),
                content: result.output.clone(),
                is_error: result.is_error(),
            });
        }

        // Add tool results to history
        history.push(ProviderMessage {
            role: ProviderRole::User,
            content: tool_results,
        });

        // Don't gate on `stop_reason == EndTurn` — proxies like
        // OpenWebUI/LiteLLM emit `Done{EndTurn}` on the final `[DONE]`
        // SSE marker even when the chunk that finished the turn carried
        // tool_calls. Trusting it makes the runner execute tools once,
        // then break before re-streaming with the tool_results — the
        // model never sees what the tools returned. The empty-tool_calls
        // check above (line ~700) is the correct termination signal.
        let _ = stop_reason;
    }

    TurnResult::Completed {
        token_count: total_tokens,
        tool_count: total_tools,
        last_tool: last_tool_name,
    }
}

// ─── Leader inbox polling ────────────────────────────────────────────────────

/// Check the leader's inbox for new messages from teammates.
/// Returns formatted messages ready to be injected into the conversation.
/// Called periodically by the main event loop (every LEADER_POLL_INTERVAL_MS).
pub async fn poll_leader_inbox(team_name: &str) -> Vec<IncomingTeammateMessage> {
    let messages = mailbox::read_mailbox(super::TEAM_LEAD_NAME, team_name).await;
    let mut incoming = Vec::new();

    for (idx, msg) in messages.iter().enumerate() {
        if msg.read {
            continue;
        }

        // Skip idle notifications — they're informational, not conversation content
        if mailbox::is_idle_notification(&msg.text) {
            // Mark as read so we don't re-process
            let _ = mailbox::mark_message_read(super::TEAM_LEAD_NAME, team_name, idx).await;
            continue;
        }

        // Format as teammate-message for conversation injection
        let formatted = format_teammate_message(
            &msg.from,
            &msg.text,
            msg.color.as_deref(),
            msg.summary.as_deref(),
        );

        incoming.push(IncomingTeammateMessage {
            from: msg.from.clone(),
            text: msg.text.clone(),
            formatted,
            color: msg.color.clone(),
            summary: msg.summary.clone(),
        });

        // Mark as read
        let _ = mailbox::mark_message_read(super::TEAM_LEAD_NAME, team_name, idx).await;
    }

    incoming
}

/// A message from a teammate ready for delivery to the leader's conversation.
#[derive(Debug, Clone)]
pub struct IncomingTeammateMessage {
    pub from: String,
    pub text: String,
    /// Pre-formatted `<teammate-message>` XML for conversation injection.
    pub formatted: String,
    pub color: Option<String>,
    pub summary: Option<String>,
}

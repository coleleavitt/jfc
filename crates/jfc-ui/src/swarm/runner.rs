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

use super::TEAM_LEAD_NAME;
use super::mailbox;
use super::types::*;

/// Teammate colors matching v126's palette. Cycled through on each spawn.
const TEAMMATE_COLORS: &[&str] = &[
    "#4FC3F7", "#81C784", "#FFB74D", "#BA68C8", "#F06292", "#4DD0E1", "#AED581", "#FFD54F",
    "#7986CB", "#A1887F",
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
    /// Shared task list used by TaskCreate/TaskUpdate/TaskList/TaskDone.
    pub task_store: Option<std::sync::Arc<crate::tasks::TaskStore>>,
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
    TaskAvailable { task_id: String, prompt: String },
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
    let task_id = format!("teammate-{}", config.identity.agent_id);

    let identity = config.identity.clone();
    let task_id_clone = task_id.clone();

    tokio::spawn(async move {
        let result = run_teammate_loop(config, abort_rx, event_tx.clone()).await;

        match result {
            Ok(TeammateExit::Completed) => {
                debug!(
                    "[InProcessRunner] Teammate {} completed normally",
                    identity.agent_name
                );
                let _ = event_tx.send(TeammateEvent::Completed {
                    task_id: task_id_clone,
                    agent_id: identity.agent_id,
                });
            }
            Ok(TeammateExit::Cancelled) => {
                // Abort signal — either explicit (.send(true)) or the
                // watch::Sender was dropped. The previous version silently
                // mapped this to Completed, which lit up every teammate as
                // ": Done" in the UI before they did any work, because the
                // spawn site at stream.rs:1962 dropped the abort handle.
                debug!(
                    "[InProcessRunner] Teammate {} cancelled",
                    identity.agent_name
                );
                let _ = event_tx.send(TeammateEvent::Cancelled {
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

/// Why a teammate's run loop ended. Aborted vs naturally exhausted is a
/// meaningful distinction for the UI — aborted means "not done yet",
/// completed means "the agent decided it was finished".
#[derive(Debug)]
enum TeammateExit {
    Completed,
    Cancelled,
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
    Completed { task_id: String, agent_id: String },
    /// Teammate's run loop was cancelled before natural completion —
    /// either by an explicit abort signal (ESC×2, kill button) or by the
    /// abort_tx watch::Sender being dropped. Distinct from Completed so
    /// the UI can label the row "Cancelled" instead of "Done".
    Cancelled { task_id: String, agent_id: String },
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
    let mut conversation_history: Vec<crate::provider::ProviderMessage> = Vec::new();
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
            TurnResult::Completed {
                token_count,
                tool_count,
                last_tool,
            } => {
                if let (Some(store), Some(task_id)) =
                    (config.task_store.as_ref(), active_task_id.take())
                {
                    let _ = store.update(
                        &task_id,
                        crate::tasks::TaskPatch {
                            status: Some(crate::tasks::TaskStatus::Completed),
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
                });
            }
            TurnResult::Aborted => {
                debug!("[InProcessRunner] {} turn aborted", identity.agent_name);
                break TeammateExit::Cancelled;
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

/// Poll for the next message or signal. Checks:
/// 1. Abort signal (highest priority)
/// 2. File-based mailbox (shutdown requests prioritized, then leader, then any)
/// 3. Repeats every POLL_INTERVAL_MS
async fn poll_for_next_message(
    identity: &TeammateIdentity,
    task_store: Option<std::sync::Arc<crate::tasks::TaskStore>>,
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

/// Check the team's task list for an unblocked, unowned task to claim.
/// Returns a formatted prompt if a task was successfully claimed.
async fn check_task_list_for_work(
    identity: &TeammateIdentity,
    store: Option<std::sync::Arc<crate::tasks::TaskStore>>,
) -> Option<(String, String)> {
    let store = store.unwrap_or_else(|| crate::tasks::TaskStore::open_team(&identity.team_name));
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

    let mut estimated_tokens_without_usage: u64 = 0;
    let mut latest_input_tokens: u64 = 0;
    let mut latest_cache_read_tokens: u64 = 0;
    let mut latest_cache_write_tokens: u64 = 0;
    let mut cumulative_output_tokens: u64 = 0;
    let mut total_tools: u64 = 0;
    let mut last_tool_name: Option<String> = None;
    // Unlimited turns — matches Claude Code, which has no fixed cap.
    // The teammate runs until end_turn, abort, or upstream error.
    let mut turn = 0u32;

    loop {
        turn += 1;

        // Check abort
        if *abort_rx.borrow() {
            return TurnResult::Aborted;
        }

        // Build stream options
        let opts = StreamOptions::new(model.clone())
            .system(system.clone())
            .tools(tools::all_tool_defs());

        // Two-stage context safety mirroring v131 Claude Code: (1) try
        // LLM-based auto-compaction at 100k tokens, (2) fall through
        // to byte-budget eviction if compaction is skipped or fails.
        // Same logic as `tools::execute_task` — a long-running teammate
        // doing multi-turn research can otherwise blow the context
        // window before its final summary turn.
        let compacted =
            crate::stream::auto_compact_subagent_history(history, provider.as_ref(), model.clone())
                .await;
        if compacted {
            tracing::info!(
                target: "jfc::swarm::runner",
                task_id,
                turn,
                agent_id = %identity.agent_id,
                "teammate transcript auto-compacted"
            );
        }
        let elided = crate::stream::cap_messages_for_budget(
            history,
            crate::stream::SUBAGENT_HISTORY_BUDGET_BYTES,
        );
        if elided {
            tracing::info!(
                target: "jfc::swarm::runner",
                task_id,
                turn,
                agent_id = %identity.agent_id,
                "teammate history elided to fit byte budget"
            );
        }

        let stream = match provider.stream(history.clone(), &opts).await {
            Ok(s) => s,
            Err(e) => return TurnResult::Error(format!("provider stream error: {e}")),
        };

        let mut response_text = String::new();
        // (id, name, kind, input, raw_input, validation_error)
        // — `validation_error` is `Some` when the model's JSON failed
        // shape validation; we then skip execution and ship the error
        // back as a tool_result so the model sees what went wrong.
        let mut tool_calls: Vec<(
            String,
            String,
            ToolKind,
            ToolInput,
            serde_json::Value,
            Option<String>,
        )> = Vec::new();
        let mut stop_reason = StopReason::EndTurn;
        let mut saw_usage_this_turn = false;
        let mut estimated_turn_tokens: u64 = 0;
        let mut usage_baseline = (0u32, 0u32, 0u32, 0u32);

        futures::pin_mut!(stream);
        loop {
            if *abort_rx.borrow() {
                return TurnResult::Aborted;
            }

            let event_result = tokio::select! {
                biased;
                changed = abort_rx.changed() => {
                    if changed.is_err() || *abort_rx.borrow() {
                        return TurnResult::Aborted;
                    }
                    continue;
                }
                event_result = stream.next() => event_result,
            };

            let Some(event_result) = event_result else {
                break;
            };

            let event = match event_result {
                Ok(e) => e,
                Err(e) => return TurnResult::Error(format!("stream error: {e}")),
            };
            match event {
                StreamEvent::TextDelta { delta, .. } => {
                    if !saw_usage_this_turn {
                        estimated_turn_tokens += (delta.len() / 4) as u64;
                    }
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
                StreamEvent::ToolDone {
                    tool_name,
                    tool_use_id,
                    input_json,
                    ..
                } => {
                    let input_value: serde_json::Value =
                        serde_json::from_str(&input_json).unwrap_or_default();
                    let kind = ToolKind::from_name(&tool_name);
                    let (parsed_input, validation_err) =
                        match ToolInput::from_value(&tool_name, input_value.clone()) {
                            Ok(parsed) => (parsed, None),
                            Err(err) => {
                                let msg = err.to_string();
                                warn!(
                                    target: "jfc::swarm::runner",
                                    tool_name = %tool_name,
                                    error = %msg,
                                    "tool input shape validation failed — failing tool"
                                );
                                // Stub the parsed input with a Generic so
                                // the assistant turn we replay to the
                                // provider still echoes a coherent shape;
                                // the validation_err flag short-circuits
                                // execution below.
                                (
                                    crate::types::ToolInput::Generic {
                                        summary: input_value.to_string(),
                                    },
                                    Some(msg),
                                )
                            }
                        };
                    tool_calls.push((
                        tool_use_id,
                        tool_name.clone(),
                        kind,
                        parsed_input,
                        input_value,
                        validation_err,
                    ));
                    last_tool_name = Some(tool_name);
                }
                StreamEvent::Usage {
                    input_tokens,
                    output_tokens,
                    cache_read_tokens,
                    cache_write_tokens,
                } => {
                    let output_delta = output_tokens.saturating_sub(usage_baseline.1) as u64;
                    usage_baseline = (
                        input_tokens,
                        output_tokens,
                        cache_read_tokens,
                        cache_write_tokens,
                    );
                    saw_usage_this_turn = true;
                    latest_input_tokens = input_tokens as u64;
                    latest_cache_read_tokens = cache_read_tokens as u64;
                    latest_cache_write_tokens = cache_write_tokens as u64;
                    cumulative_output_tokens =
                        cumulative_output_tokens.saturating_add(output_delta);
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

        if !saw_usage_this_turn {
            estimated_tokens_without_usage =
                estimated_tokens_without_usage.saturating_add(estimated_turn_tokens);
        }

        // Add assistant response to history
        let mut assistant_content = Vec::new();
        if !response_text.is_empty() {
            assistant_content.push(ProviderContent::Text(response_text.clone()));
        }
        for (id, name, _, _, input_val, _) in &tool_calls {
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

        for (id, name, kind, input, raw_input, validation_err) in &tool_calls {
            total_tools += 1;

            // If shape validation failed during streaming, short-circuit
            // with an error tool_result so the model can self-correct on
            // the next turn rather than us silently executing a stub.
            if let Some(err) = validation_err {
                tool_results.push(ProviderContent::ToolResult {
                    tool_use_id: id.clone(),
                    content: format!("Tool input rejected: {err}"),
                    is_error: true,
                });
                continue;
            }

            // Emit progress
            let _ = event_tx.send(TeammateEvent::Progress {
                task_id: task_id.to_owned(),
                agent_id: identity.agent_id.clone(),
                token_count: estimated_tokens_without_usage
                    .saturating_add(latest_input_tokens)
                    .saturating_add(latest_cache_read_tokens)
                    .saturating_add(latest_cache_write_tokens)
                    .saturating_add(cumulative_output_tokens),
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
                if let Err(e) = super::permission_sync::write_permission_request(&request).await {
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
                config.task_store.clone(),
                Some(identity.team_name.as_str()),
            )
            .await;

            tool_results.push(ProviderContent::ToolResult {
                tool_use_id: id.clone(),
                content: crate::stream::cap_tool_result(&result.output),
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
        token_count: estimated_tokens_without_usage
            .saturating_add(latest_input_tokens)
            .saturating_add(latest_cache_read_tokens)
            .saturating_add(latest_cache_write_tokens)
            .saturating_add(cumulative_output_tokens),
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

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::swarm::mailbox;
    use crate::swarm::test_support::HomeOverride;

    fn make_identity() -> TeammateIdentity {
        TeammateIdentity {
            agent_id: "alice@alpha".into(),
            agent_name: "alice".into(),
            team_name: "alpha".into(),
            color: Some("#FF0000".into()),
            plan_mode_required: false,
            parent_session_id: "session-1".into(),
        }
    }

    #[test]
    fn teammate_task_id_format_normal() {
        assert_eq!(teammate_task_id("alice@alpha"), "teammate-alice@alpha");
    }

    #[test]
    fn assign_teammate_color_cycles_through_palette_normal() {
        // Two consecutive calls must return real palette entries (hex strings).
        // We don't lock the order because the COLOR_INDEX is process-global,
        // but every value should start with `#` and be 7 chars long.
        for _ in 0..5 {
            let c = assign_teammate_color();
            assert_eq!(c.len(), 7, "expected `#RRGGBB`, got {c}");
            assert!(c.starts_with('#'));
        }
    }

    #[tokio::test]
    async fn poll_leader_inbox_returns_empty_for_no_messages_normal() {
        let _g = HomeOverride::new();
        // Empty inbox → empty result.
        let incoming = poll_leader_inbox("alpha").await;
        assert!(incoming.is_empty());
    }

    #[tokio::test]
    async fn poll_leader_inbox_filters_idle_notifications_robust() {
        let _g = HomeOverride::new();
        // Idle notifications are informational; they should be silently
        // marked-read and not surface as conversation injections.
        mailbox::send_idle_notification("alice", None, "alpha", Some("done"), None)
            .await
            .unwrap();
        let incoming = poll_leader_inbox("alpha").await;
        assert!(incoming.is_empty());

        // Underlying message should be marked read.
        let msgs = mailbox::read_mailbox(crate::swarm::TEAM_LEAD_NAME, "alpha").await;
        assert_eq!(msgs.len(), 1);
        assert!(msgs[0].read);
    }

    #[tokio::test]
    async fn poll_leader_inbox_returns_unread_real_messages_normal() {
        let _g = HomeOverride::new();
        // Plain text from a teammate → surfaces as IncomingTeammateMessage.
        mailbox::send_to_leader("alice", "got a result", Some("#FF0000"), "alpha")
            .await
            .unwrap();
        let incoming = poll_leader_inbox("alpha").await;
        assert_eq!(incoming.len(), 1);
        assert_eq!(incoming[0].from, "alice");
        assert_eq!(incoming[0].text, "got a result");
        assert!(incoming[0].formatted.contains("teammate_id=\"alice\""));
        assert!(incoming[0].formatted.contains("got a result"));
        assert_eq!(incoming[0].color.as_deref(), Some("#FF0000"));

        // Subsequent poll yields nothing — message was marked read.
        let incoming2 = poll_leader_inbox("alpha").await;
        assert!(incoming2.is_empty());
    }

    #[tokio::test]
    async fn poll_leader_inbox_skips_already_read_messages_robust() {
        let _g = HomeOverride::new();
        mailbox::send_to_leader("alice", "first", None, "alpha")
            .await
            .unwrap();
        // Mark it read manually.
        mailbox::mark_message_read(crate::swarm::TEAM_LEAD_NAME, "alpha", 0)
            .await
            .unwrap();
        let incoming = poll_leader_inbox("alpha").await;
        assert!(incoming.is_empty());
    }

    #[tokio::test]
    async fn check_task_list_for_work_returns_none_when_no_tasks_robust() {
        let _g = HomeOverride::new();
        let identity = make_identity();
        let result = check_task_list_for_work(&identity, None).await;
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn check_task_list_for_work_claims_pending_unowned_task_normal() {
        let _g = HomeOverride::new();
        let identity = make_identity();

        // Set up a task list with one claimable task.
        let tasks_dir = crate::swarm::team_helpers::tasks_dir(&identity.team_name);
        tokio::fs::create_dir_all(&tasks_dir).await.unwrap();
        let tasks = serde_json::json!([
            {
                "id": "1",
                "subject": "Implement feature X",
                "description": "Use the new API.",
                "status": "pending",
                "owner": "",
                "blockedBy": []
            }
        ]);
        tokio::fs::write(
            tasks_dir.join("tasks.json"),
            serde_json::to_string_pretty(&tasks).unwrap(),
        )
        .await
        .unwrap();

        let (_task_id, prompt) = check_task_list_for_work(&identity, None).await.unwrap();
        assert!(prompt.contains("task #1"));
        assert!(prompt.contains("Implement feature X"));
        assert!(prompt.contains("Use the new API"));

        // The task should be marked in_progress with this teammate as owner.
        let updated = crate::tasks::TaskStore::open_team(&identity.team_name)
            .get("1")
            .unwrap();
        assert_eq!(updated.status, crate::tasks::TaskStatus::InProgress);
        assert_eq!(updated.owner.as_deref(), Some("alice"));
    }

    #[tokio::test]
    async fn check_task_list_for_work_skips_owned_task_robust() {
        let _g = HomeOverride::new();
        let identity = make_identity();

        let tasks_dir = crate::swarm::team_helpers::tasks_dir(&identity.team_name);
        tokio::fs::create_dir_all(&tasks_dir).await.unwrap();
        let tasks = serde_json::json!([
            {
                "id": "1",
                "subject": "Already taken",
                "status": "pending",
                "owner": "bob",
                "blockedBy": []
            }
        ]);
        tokio::fs::write(
            tasks_dir.join("tasks.json"),
            serde_json::to_string_pretty(&tasks).unwrap(),
        )
        .await
        .unwrap();

        // Task already owned by another agent → no claim.
        assert!(check_task_list_for_work(&identity, None).await.is_none());
    }

    #[tokio::test]
    async fn check_task_list_for_work_skips_blocked_task_robust() {
        let _g = HomeOverride::new();
        let identity = make_identity();

        let tasks_dir = crate::swarm::team_helpers::tasks_dir(&identity.team_name);
        tokio::fs::create_dir_all(&tasks_dir).await.unwrap();
        // Task #2 is blocked by task #1 which is still pending.
        let tasks = serde_json::json!([
            {"id": "1", "subject": "Foundation", "status": "pending", "owner": "bob", "blockedBy": []},
            {"id": "2", "subject": "Depends", "status": "pending", "owner": "", "blockedBy": ["1"]}
        ]);
        tokio::fs::write(
            tasks_dir.join("tasks.json"),
            serde_json::to_string_pretty(&tasks).unwrap(),
        )
        .await
        .unwrap();

        // Neither task is claimable for `alice` (1 owned, 2 blocked).
        assert!(check_task_list_for_work(&identity, None).await.is_none());
    }

    #[tokio::test]
    async fn check_task_list_for_work_picks_up_unblocked_after_completion_normal() {
        let _g = HomeOverride::new();
        let identity = make_identity();

        let tasks_dir = crate::swarm::team_helpers::tasks_dir(&identity.team_name);
        tokio::fs::create_dir_all(&tasks_dir).await.unwrap();
        // Task #1 is completed, so #2 is now unblocked and claimable.
        let tasks = serde_json::json!([
            {"id": "1", "subject": "Done", "status": "completed", "owner": "bob", "blockedBy": []},
            {"id": "2", "subject": "Now ready", "status": "pending", "owner": "", "blockedBy": ["1"]}
        ]);
        tokio::fs::write(
            tasks_dir.join("tasks.json"),
            serde_json::to_string_pretty(&tasks).unwrap(),
        )
        .await
        .unwrap();

        let (_task_id, prompt) = check_task_list_for_work(&identity, None).await.unwrap();
        assert!(prompt.contains("Now ready"));
    }

    #[tokio::test]
    async fn check_task_list_for_work_skips_non_pending_status_robust() {
        let _g = HomeOverride::new();
        let identity = make_identity();

        let tasks_dir = crate::swarm::team_helpers::tasks_dir(&identity.team_name);
        tokio::fs::create_dir_all(&tasks_dir).await.unwrap();
        let tasks = serde_json::json!([
            {"id": "1", "subject": "In flight", "status": "in_progress", "owner": "", "blockedBy": []},
            {"id": "2", "subject": "Done", "status": "completed", "owner": "", "blockedBy": []}
        ]);
        tokio::fs::write(
            tasks_dir.join("tasks.json"),
            serde_json::to_string_pretty(&tasks).unwrap(),
        )
        .await
        .unwrap();

        // No `pending` task → nothing to claim.
        assert!(check_task_list_for_work(&identity, None).await.is_none());
    }

    #[tokio::test]
    async fn check_task_list_for_work_handles_missing_optional_fields_robust() {
        // The JSON parser uses `unwrap_or(...)` for description / subject;
        // a sparse task should still be claimable without panicking.
        let _g = HomeOverride::new();
        let identity = make_identity();

        let tasks_dir = crate::swarm::team_helpers::tasks_dir(&identity.team_name);
        tokio::fs::create_dir_all(&tasks_dir).await.unwrap();
        let tasks = serde_json::json!([
            {"id": "1", "status": "pending"}
        ]);
        tokio::fs::write(
            tasks_dir.join("tasks.json"),
            serde_json::to_string_pretty(&tasks).unwrap(),
        )
        .await
        .unwrap();

        let (_task_id, prompt) = check_task_list_for_work(&identity, None).await.unwrap();
        assert!(prompt.contains("task #1"));
        assert!(prompt.contains("(unnamed task)"));
    }

    #[test]
    fn poll_result_variants_constructable_normal() {
        // Smoke test the enum so coverage hits the variant constructors.
        let _ = PollResult::Aborted;
        let _ = PollResult::TaskAvailable {
            task_id: "1".into(),
            prompt: "do it".into(),
        };
        let _ = PollResult::NewMessage {
            message: "hi".into(),
            from: "leader".into(),
            color: None,
            summary: None,
        };
        let _ = PollResult::ShutdownRequest {
            request: None,
            original_message: "x".into(),
        };
    }

    /// A no-op provider that returns a single, configurable stream.
    /// Used to drive `start_teammate` end-to-end without needing a real
    /// API. Each `stream()` call returns the events from `script` once,
    /// then errors on subsequent calls so the loop doesn't infinitely
    /// re-stream a finished turn.
    struct StubProvider {
        script: std::sync::Mutex<Option<Vec<crate::provider::StreamEvent>>>,
    }

    impl StubProvider {
        fn new(events: Vec<crate::provider::StreamEvent>) -> Self {
            Self {
                script: std::sync::Mutex::new(Some(events)),
            }
        }
    }

    #[async_trait::async_trait]
    impl crate::provider::Provider for StubProvider {
        fn name(&self) -> &str {
            "stub"
        }
        fn available_models(&self) -> Vec<crate::provider::ModelInfo> {
            vec![crate::provider::ModelInfo::new(
                "stub-model",
                "Stub Model",
                "stub",
            )]
        }
        async fn stream(
            &self,
            _messages: Vec<crate::provider::ProviderMessage>,
            _options: &crate::provider::StreamOptions,
        ) -> anyhow::Result<crate::provider::EventStream> {
            use futures::stream;
            let events = self
                .script
                .lock()
                .unwrap()
                .take()
                .ok_or_else(|| anyhow::anyhow!("StubProvider script exhausted"))?;
            let stream = stream::iter(events.into_iter().map(Ok));
            Ok(Box::pin(stream))
        }
    }
    impl crate::provider::seal::Sealed for StubProvider {}

    #[tokio::test(flavor = "current_thread")]
    async fn start_teammate_completes_after_endturn_normal() {
        // Drive a single full agent turn through the runner: text delta +
        // EndTurn (no tools), then immediately abort so the post-turn idle
        // loop exits without polling forever.
        let _g = HomeOverride::new();

        use crate::provider::{StopReason, StreamEvent};
        let provider: std::sync::Arc<dyn crate::provider::Provider> =
            std::sync::Arc::new(StubProvider::new(vec![
                StreamEvent::TextDelta {
                    index: 0,
                    delta: "hi".into(),
                },
                StreamEvent::Done {
                    stop_reason: StopReason::EndTurn,
                },
            ]));

        let identity = make_identity();
        let config = TeammateRunnerConfig {
            identity: identity.clone(),
            prompt: "hello".into(),
            description: "test".into(),
            model: None,
            agent_type: None,
            provider,
            model_id: crate::provider::ModelId::new("stub-model"),
            system_prompt: Some("be brief".into()),
            task_store: None,
        };

        let (event_tx, mut event_rx) = tokio::sync::mpsc::unbounded_channel();
        let (task_id, abort_tx) = start_teammate(config, event_tx);
        assert_eq!(task_id, "teammate-alice@alpha");

        // Give the loop a moment to run the turn, then abort.
        // The stub will exhaust on the second `stream()` call (after Idle).
        // Either way, abort_tx forces a clean exit.
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        let _ = abort_tx.send(true);

        // Drain events with a tight overall timeout.
        let mut got_text_delta = false;
        let mut got_idle = false;
        let mut got_terminal = false;
        let drain = async {
            while let Some(ev) = event_rx.recv().await {
                match ev {
                    TeammateEvent::TextDelta { delta, .. } => {
                        if delta == "hi" {
                            got_text_delta = true;
                        }
                    }
                    TeammateEvent::Idle { .. } => got_idle = true,
                    TeammateEvent::Completed { .. } | TeammateEvent::Failed { .. } | TeammateEvent::Cancelled { .. } => {
                        got_terminal = true;
                        break;
                    }
                    _ => {}
                }
            }
        };
        let _ = tokio::time::timeout(std::time::Duration::from_secs(2), drain).await;

        assert!(got_text_delta, "expected text delta from stub stream");
        assert!(got_idle, "expected idle event after first turn");
        assert!(got_terminal, "expected Completed or Failed terminal event");
    }

    /// Make a stub provider that returns multiple scripts in sequence, one
    /// per `stream()` call. After the last script is consumed, subsequent
    /// calls error.
    struct ScriptedProvider {
        scripts: std::sync::Mutex<std::collections::VecDeque<Vec<crate::provider::StreamEvent>>>,
    }

    impl ScriptedProvider {
        fn new(scripts: Vec<Vec<crate::provider::StreamEvent>>) -> Self {
            Self {
                scripts: std::sync::Mutex::new(scripts.into_iter().collect()),
            }
        }
    }

    #[async_trait::async_trait]
    impl crate::provider::Provider for ScriptedProvider {
        fn name(&self) -> &str {
            "scripted"
        }
        fn available_models(&self) -> Vec<crate::provider::ModelInfo> {
            vec![]
        }
        async fn stream(
            &self,
            _messages: Vec<crate::provider::ProviderMessage>,
            _options: &crate::provider::StreamOptions,
        ) -> anyhow::Result<crate::provider::EventStream> {
            use futures::stream;
            let next = self
                .scripts
                .lock()
                .unwrap()
                .pop_front()
                .ok_or_else(|| anyhow::anyhow!("scripts exhausted"))?;
            Ok(Box::pin(stream::iter(next.into_iter().map(Ok))))
        }
    }
    impl crate::provider::seal::Sealed for ScriptedProvider {}

    #[tokio::test(flavor = "current_thread")]
    async fn start_teammate_executes_tool_then_endturn_normal() {
        // Drive a full tool-use cycle. The first stream returns a tool call
        // (LS — read-only, no side effects), then the runner re-streams; the
        // second script returns text + EndTurn. After idle, we abort to end
        // the loop quickly. This exercises the run_single_turn tool-execution
        // path including Usage / TextDone / ToolDone events.
        let _g = HomeOverride::new();

        use crate::provider::{StopReason, StreamEvent};

        // Pick a tool that exists and is benign. `Read` requires a path; we
        // pass a non-existent one — `tools::execute_tool` returns an error
        // result but the runner appends it as a tool_result (is_error: true)
        // and continues. That keeps the test hermetic.
        let tool_input = serde_json::json!({"file_path": "/nonexistent-path-for-test"});
        let tool_input_json = tool_input.to_string();

        let scripts = vec![
            vec![
                StreamEvent::Usage {
                    input_tokens: 5,
                    output_tokens: 3,
                    cache_read_tokens: 0,
                    cache_write_tokens: 0,
                },
                StreamEvent::ToolDone {
                    index: 0,
                    tool_name: "Read".into(),
                    tool_use_id: "call-1".into(),
                    input_json: tool_input_json,
                },
                StreamEvent::Done {
                    stop_reason: StopReason::ToolUse,
                },
            ],
            vec![
                StreamEvent::TextDelta {
                    index: 0,
                    delta: "ok".into(),
                },
                StreamEvent::Done {
                    stop_reason: StopReason::EndTurn,
                },
            ],
        ];

        let provider: std::sync::Arc<dyn crate::provider::Provider> =
            std::sync::Arc::new(ScriptedProvider::new(scripts));

        let config = TeammateRunnerConfig {
            identity: make_identity(),
            prompt: "do thing".into(),
            description: "test".into(),
            model: None,
            agent_type: None,
            provider,
            model_id: crate::provider::ModelId::new("stub-model"),
            system_prompt: None,
            task_store: None,
        };

        let (event_tx, mut event_rx) = tokio::sync::mpsc::unbounded_channel();
        let (_task_id, abort_tx) = start_teammate(config, event_tx);

        tokio::time::sleep(std::time::Duration::from_millis(80)).await;
        let _ = abort_tx.send(true);

        let mut got_progress_with_tool = false;
        let mut got_terminal = false;
        let drain = async {
            while let Some(ev) = event_rx.recv().await {
                match ev {
                    TeammateEvent::Progress {
                        last_tool: Some(t), ..
                    } if t == "Read" => {
                        got_progress_with_tool = true;
                    }
                    TeammateEvent::Completed { .. } | TeammateEvent::Failed { .. } | TeammateEvent::Cancelled { .. } => {
                        got_terminal = true;
                        break;
                    }
                    _ => {}
                }
            }
        };
        let _ = tokio::time::timeout(std::time::Duration::from_secs(3), drain).await;
        assert!(
            got_progress_with_tool,
            "expected Progress event with last_tool=Read"
        );
        assert!(got_terminal);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn start_teammate_picks_up_leader_message_after_idle_normal() {
        // First turn streams text + EndTurn. While the loop is idle, we plant
        // a leader message in alice's mailbox. The runner picks it up via
        // poll_for_next_message → priority-2 (leader) branch → triggers a
        // second turn. We then abort. Exercises the leader-message branch
        // and the second-iteration prompt update.
        let _g = HomeOverride::new();

        use crate::provider::{StopReason, StreamEvent};

        // Two scripts: one per turn. Both end with EndTurn (no tools).
        let scripts = vec![
            vec![
                StreamEvent::TextDelta {
                    index: 0,
                    delta: "first".into(),
                },
                StreamEvent::Done {
                    stop_reason: StopReason::EndTurn,
                },
            ],
            vec![
                StreamEvent::TextDelta {
                    index: 0,
                    delta: "second".into(),
                },
                StreamEvent::Done {
                    stop_reason: StopReason::EndTurn,
                },
            ],
        ];

        let identity = make_identity();
        let provider: std::sync::Arc<dyn crate::provider::Provider> =
            std::sync::Arc::new(ScriptedProvider::new(scripts));

        let config = TeammateRunnerConfig {
            identity: identity.clone(),
            prompt: "go".into(),
            description: "test".into(),
            model: None,
            agent_type: None,
            provider,
            model_id: crate::provider::ModelId::new("stub-model"),
            system_prompt: None,
            task_store: None,
        };

        let (event_tx, mut event_rx) = tokio::sync::mpsc::unbounded_channel();
        let (_task_id, abort_tx) = start_teammate(config, event_tx);

        // Wait for the first idle, then plant a follow-up leader message.
        let mut idle_count = 0u32;
        let mut second_text = false;
        let mut terminal = false;
        let drain = async {
            while let Some(ev) = event_rx.recv().await {
                match ev {
                    TeammateEvent::Idle { .. } => {
                        idle_count += 1;
                        if idle_count == 1 {
                            // Plant leader message after first idle so the
                            // poll loop picks it up on its next tick.
                            mailbox::write_to_mailbox(
                                &identity.agent_name,
                                crate::swarm::types::MailboxMessage {
                                    from: crate::swarm::TEAM_LEAD_NAME.into(),
                                    text: "next prompt".into(),
                                    timestamp: "t".into(),
                                    color: None,
                                    summary: None,
                                    read: false,
                                },
                                &identity.team_name,
                            )
                            .await
                            .unwrap();
                        } else {
                            // After the second idle, abort to end the loop.
                            let _ = abort_tx.send(true);
                        }
                    }
                    TeammateEvent::TextDelta { delta, .. } => {
                        if delta == "second" {
                            second_text = true;
                        }
                    }
                    TeammateEvent::Completed { .. } | TeammateEvent::Failed { .. } | TeammateEvent::Cancelled { .. } => {
                        terminal = true;
                        break;
                    }
                    _ => {}
                }
            }
        };
        let _ = tokio::time::timeout(std::time::Duration::from_secs(5), drain).await;
        assert!(idle_count >= 2, "expected at least 2 idle events");
        assert!(second_text, "expected second turn to stream `second`");
        assert!(terminal);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn start_teammate_failed_when_provider_errors_robust() {
        // First stream call exhausts the (empty) script → provider errors →
        // run_single_turn returns TurnResult::Error. The loop logs and continues
        // to idle. Then we abort to force termination, and the runner reports
        // Completed (graceful, since errors don't abort the loop).
        let _g = HomeOverride::new();

        let provider: std::sync::Arc<dyn crate::provider::Provider> =
            std::sync::Arc::new(StubProvider::new(vec![]));

        let config = TeammateRunnerConfig {
            identity: make_identity(),
            prompt: "hi".into(),
            description: "test".into(),
            model: None,
            agent_type: None,
            provider,
            model_id: crate::provider::ModelId::new("stub-model"),
            system_prompt: None,
            task_store: None,
        };

        let (event_tx, mut event_rx) = tokio::sync::mpsc::unbounded_channel();
        let (_task_id, abort_tx) = start_teammate(config, event_tx);

        // Let the loop hit its first stream error, then abort.
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        let _ = abort_tx.send(true);

        let mut saw_terminal = false;
        let drain = async {
            while let Some(ev) = event_rx.recv().await {
                if matches!(
                    ev,
                    TeammateEvent::Completed { .. } | TeammateEvent::Failed { .. } | TeammateEvent::Cancelled { .. }
                ) {
                    saw_terminal = true;
                    break;
                }
            }
        };
        let _ = tokio::time::timeout(std::time::Duration::from_secs(2), drain).await;
        assert!(saw_terminal);
    }

    // Regression: dropping the abort_tx must emit Cancelled, NOT Completed.
    //
    // This is the smoking-gun test for the "all teammates marked Done"
    // bug. `start_teammate` returns a `watch::Sender<bool>`; if a caller
    // drops it (the original `stream.rs:1962` bug — leading underscore
    // made it look like an intentional bind), `watch::Receiver::changed()`
    // immediately resolves Err and the runner's `tokio::select! { biased; ...}`
    // returns `TurnResult::Aborted` on the FIRST stream poll. The old
    // path then ran `Ok(())` → `TeammateEvent::Completed`, which the UI
    // rendered as ": Done" before the teammate did any work.
    //
    // After the fix, the runner returns `Ok(TeammateExit::Cancelled)`
    // and start_teammate emits `TeammateEvent::Cancelled`. Verifies the
    // distinction at the event-stream level.
    #[tokio::test(flavor = "current_thread")]
    async fn dropping_abort_tx_emits_cancelled_not_completed_normal() {
        let _g = HomeOverride::new();

        // Provider script: a single text delta then EndTurn — plenty of
        // work for the runner to actually do if it weren't aborted.
        let provider: std::sync::Arc<dyn crate::provider::Provider> = std::sync::Arc::new(
            StubProvider::new(vec![
                crate::provider::StreamEvent::TextDelta {
                    index: 0,
                    delta: "hello".into(),
                },
                crate::provider::StreamEvent::Done {
                    stop_reason: crate::provider::StopReason::EndTurn,
                },
            ]),
        );
        let identity = make_identity();
        let config = TeammateRunnerConfig {
            identity,
            prompt: "p".into(),
            description: "d".into(),
            model: None,
            agent_type: None,
            provider,
            model_id: crate::provider::ModelId::new("stub-model"),
            system_prompt: None,
            task_store: None,
        };
        let (event_tx, mut event_rx) = tokio::sync::mpsc::unbounded_channel();
        let (_task_id, abort_tx) = start_teammate(config, event_tx);
        // Drop the abort handle IMMEDIATELY — this is the original bug.
        drop(abort_tx);

        let mut last_terminal: Option<&'static str> = None;
        let drain = async {
            while let Some(ev) = event_rx.recv().await {
                match ev {
                    TeammateEvent::Completed { .. } => {
                        last_terminal = Some("Completed");
                        break;
                    }
                    TeammateEvent::Cancelled { .. } => {
                        last_terminal = Some("Cancelled");
                        break;
                    }
                    TeammateEvent::Failed { .. } => {
                        last_terminal = Some("Failed");
                        break;
                    }
                    _ => {}
                }
            }
        };
        let _ = tokio::time::timeout(std::time::Duration::from_secs(2), drain).await;
        assert_eq!(
            last_terminal,
            Some("Cancelled"),
            "dropping abort_tx must surface as Cancelled, not Completed — \
             see TeammateInfo.abort_tx and stream.rs spawn site"
        );
    }

    #[test]
    fn teammate_event_variants_serialize_through_debug_normal() {
        // Coverage for each TeammateEvent variant.
        let events = vec![
            TeammateEvent::Idle {
                task_id: "t".into(),
                agent_id: "a".into(),
                agent_name: "alice".into(),
                reason: None,
                summary: None,
            },
            TeammateEvent::Progress {
                task_id: "t".into(),
                agent_id: "a".into(),
                token_count: 0,
                tool_use_count: 0,
                last_tool: None,
            },
            TeammateEvent::Completed {
                task_id: "t".into(),
                agent_id: "a".into(),
            },
            TeammateEvent::Cancelled {
                task_id: "t".into(),
                agent_id: "a".into(),
            },
            TeammateEvent::Failed {
                task_id: "t".into(),
                agent_id: "a".into(),
                error: "e".into(),
            },
            TeammateEvent::MessageSent {
                from: "alice".into(),
                to: "team-lead".into(),
                text: "ok".into(),
                summary: None,
            },
            TeammateEvent::TextDelta {
                task_id: "t".into(),
                agent_id: "a".into(),
                delta: "x".into(),
            },
        ];
        for ev in &events {
            // Just exercise the Debug impl.
            let s = format!("{ev:?}");
            assert!(!s.is_empty());
        }
    }
}

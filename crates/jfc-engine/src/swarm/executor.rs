//! Per-agent execution: single-turn streaming, tool dispatch, retry logic.
//!
//! Implements `run_single_turn` — the innermost loop that:
//! 1. Opens a streaming API call with the provider
//! 2. Parses `TextDelta`, `ToolDone`, `Usage`, and `Done` events
//! 3. Executes tool calls (with optional plan-mode permission gate)
//! 4. Appends assistant + tool_result turns to the conversation history
//! 5. Returns when the model emits EndTurn (no further tool calls)
//!
//! Retry logic for retryable provider errors (e.g. Anthropic 529) is
//! handled here via `sleep_retry_or_abort`.

use tracing::warn;

use super::runner::{TeammateEvent, TeammateRunnerConfig};

// ─── Turn result ─────────────────────────────────────────────────────────────

/// Result of running a single agent turn (one prompt → stream → tools cycle).
#[derive(Debug)]
pub enum TurnResult {
    Completed {
        token_count: u64,
        tool_count: u64,
        last_tool: Option<String>,
    },
    Aborted,
    Error(String),
}

// ─── Helpers ─────────────────────────────────────────────────────────────────

/// Sleep for `delay`, but return `false` immediately if the abort signal fires.
/// Returns `true` if the sleep completed normally, `false` if aborted.
pub async fn sleep_retry_or_abort(
    delay: std::time::Duration,
    abort_rx: &mut tokio::sync::watch::Receiver<bool>,
) -> bool {
    tokio::select! {
        _ = tokio::time::sleep(delay) => true,
        changed = abort_rx.changed() => {
            !(changed.is_err() || *abort_rx.borrow())
        }
    }
}

// ─── Single-turn execution ───────────────────────────────────────────────────

/// Run a single turn: build messages, call the API, parse response, execute tools.
/// Returns when the model finishes (EndTurn) or an error/abort occurs.
pub async fn run_single_turn(
    config: &TeammateRunnerConfig,
    prompt: &str,
    history: &mut Vec<jfc_provider::ProviderMessage>,
    event_tx: &tokio::sync::mpsc::UnboundedSender<TeammateEvent>,
    task_id: &str,
    abort_rx: &mut tokio::sync::watch::Receiver<bool>,
) -> TurnResult {
    use crate::tools;
    use crate::types::{ToolInput, ToolKind};
    use futures::StreamExt;
    use jfc_provider::{
        ProviderContent, ProviderMessage, ProviderRole, StopReason, StreamEvent, StreamOptions,
    };

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
        // Shared with `tools::execute_task` via
        // `apply_subagent_context_safety` — a long-running teammate doing
        // multi-turn research can otherwise blow the context window before
        // its final summary turn.
        let context_safety =
            crate::stream::apply_subagent_context_safety(history, provider.as_ref(), model.clone())
                .await;
        if context_safety.compacted {
            tracing::info!(
                target: "jfc::swarm::executor",
                task_id,
                turn,
                agent_id = %identity.agent_id,
                "teammate transcript auto-compacted"
            );
        }
        if context_safety.elided {
            tracing::info!(
                target: "jfc::swarm::executor",
                task_id,
                turn,
                agent_id = %identity.agent_id,
                "teammate history elided to fit byte budget"
            );
        }

        let mut stream_retry_attempt = 0u32;
        let (
            response_text,
            tool_calls,
            stop_reason,
            saw_usage_this_turn,
            estimated_turn_tokens,
            turn_input_tokens,
            turn_cache_read_tokens,
            turn_cache_write_tokens,
            turn_output_tokens,
        ) = loop {
            let stream = match crate::stream::open_stream_with_bedrock_retries(
                provider.as_ref(),
                std::sync::Arc::new(history.clone()),
                &opts,
            )
            .await
            {
                Ok(s) => s,
                Err(e) => {
                    let message = e.to_string();
                    if let Some(retry) = jfc_provider::retry::retryable_stream_error(&message) {
                        let delay = jfc_provider::retry::stream_retry_delay(stream_retry_attempt);
                        warn!(
                            target: "jfc::swarm::executor",
                            task_id,
                            turn,
                            retry_attempt = stream_retry_attempt + 1,
                            provider = retry.provider,
                            delay_ms = delay.as_millis() as u64,
                            error = %retry.message,
                            "teammate stream open hit retryable provider error"
                        );
                        stream_retry_attempt = stream_retry_attempt.saturating_add(1);
                        if !sleep_retry_or_abort(delay, abort_rx).await {
                            return TurnResult::Aborted;
                        }
                        continue;
                    }
                    return TurnResult::Error(format!("provider stream error: {e}"));
                }
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
            let mut turn_input_tokens = 0u64;
            let mut turn_cache_read_tokens = 0u64;
            let mut turn_cache_write_tokens = 0u64;
            let mut turn_output_tokens = 0u64;
            let mut retryable_stream_error: Option<String> = None;

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
                    Err(e) => {
                        let message = e.to_string();
                        if jfc_provider::retry::retryable_stream_error(&message).is_some() {
                            retryable_stream_error = Some(message);
                            break;
                        }
                        return TurnResult::Error(format!("stream error: {e}"));
                    }
                };
                match event {
                    StreamEvent::TextDelta { delta, .. } => {
                        if !saw_usage_this_turn {
                            estimated_turn_tokens += (delta.len() / 4) as u64;
                        }
                        // Forward to the leader so the task panel for this
                        // teammate shows live output. The handler translates
                        // to `TaskEvent::AgentChunk` keyed by `task_id`.
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
                                        target: "jfc::swarm::executor",
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
                        turn_input_tokens = input_tokens as u64;
                        turn_cache_read_tokens = cache_read_tokens as u64;
                        turn_cache_write_tokens = cache_write_tokens as u64;
                        turn_output_tokens = turn_output_tokens.saturating_add(output_delta);
                    }
                    StreamEvent::Done { stop_reason: r } => {
                        stop_reason = r;
                    }
                    StreamEvent::Error { message } => {
                        if jfc_provider::retry::retryable_stream_error(&message).is_some() {
                            retryable_stream_error = Some(message);
                            break;
                        }
                        return TurnResult::Error(format!("stream error: {message}"));
                    }
                    _ => {}
                }
            }

            if let Some(message) = retryable_stream_error {
                let Some(retry) = jfc_provider::retry::retryable_stream_error(&message) else {
                    unreachable!("message was classified above");
                };
                let delay = jfc_provider::retry::stream_retry_delay(stream_retry_attempt);
                warn!(
                    target: "jfc::swarm::executor",
                    task_id,
                    turn,
                    retry_attempt = stream_retry_attempt + 1,
                    provider = retry.provider,
                    delay_ms = delay.as_millis() as u64,
                    error = %retry.message,
                    "teammate stream event hit retryable provider error"
                );
                stream_retry_attempt = stream_retry_attempt.saturating_add(1);
                if !sleep_retry_or_abort(delay, abort_rx).await {
                    return TurnResult::Aborted;
                }
                continue;
            }

            break (
                response_text,
                tool_calls,
                stop_reason,
                saw_usage_this_turn,
                estimated_turn_tokens,
                turn_input_tokens,
                turn_cache_read_tokens,
                turn_cache_write_tokens,
                turn_output_tokens,
            );
        };

        cumulative_output_tokens = cumulative_output_tokens.saturating_add(turn_output_tokens);

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
                // Swarm executor accumulates tool calls from its own loop;
                // Gemini thought signatures aren't threaded through here yet.
                thought_signature: None,
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
            return TurnResult::Completed {
                token_count: estimated_tokens_without_usage
                    .saturating_add(turn_input_tokens)
                    .saturating_add(turn_cache_read_tokens)
                    .saturating_add(turn_cache_write_tokens)
                    .saturating_add(cumulative_output_tokens),
                tool_count: total_tools,
                last_tool: last_tool_name,
            };
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
                    .saturating_add(turn_input_tokens)
                    .saturating_add(turn_cache_read_tokens)
                    .saturating_add(turn_cache_write_tokens)
                    .saturating_add(cumulative_output_tokens),
                tool_use_count: total_tools,
                last_tool: Some(name.clone()),
                model_id: Some(model.as_str().to_owned()),
                cost_usd: None,
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
                        target: "jfc::swarm::executor",
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

            // Set the per-task identity so tools that look up the
            // calling agent (currently only `SendMessage` — the mailbox
            // needs `from = <teammate-name>` instead of the leader's
            // hardcoded "team-lead") read the right name. The future is
            // scoped: outside this `scope` the task-local resolves to
            // `None`, preserving leader behavior on the leader's own
            // tool path.
            //
            // Race tool execution against the abort channel so user
            // cancellation kills running tools immediately.
            let tool_future = tools::CURRENT_AGENT_NAME.scope(
                identity.agent_name.clone(),
                tools::execute_tool(
                    kind.clone(),
                    input.clone(),
                    cwd.clone(),
                    None,
                    config.task_store.clone(),
                    Some(identity.team_name.as_str()),
                ),
            );
            tokio::pin!(tool_future);

            let result = tokio::select! {
                biased;
                changed = abort_rx.changed() => {
                    if changed.is_err() || *abort_rx.borrow() {
                        return TurnResult::Aborted;
                    }
                    // Spurious wake — finish waiting on the tool
                    tool_future.await
                }
                res = &mut tool_future => res,
            };

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
        // check above is the correct termination signal.
        let _ = stop_reason;
    }
}

use std::{collections::HashMap, io::Write};

use clap::ValueEnum;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, ValueEnum)]
pub(super) enum HeadlessOutputFormat {
    #[default]
    Text,
    Json,
    #[value(name = "stream-json")]
    StreamJson,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, ValueEnum)]
pub(super) enum HeadlessInputFormat {
    #[default]
    Text,
    #[value(name = "stream-json")]
    StreamJson,
}

#[derive(Debug, Clone, Default)]
pub(super) struct PrintModeConfig {
    pub(super) output_format: HeadlessOutputFormat,
    pub(super) input_format: HeadlessInputFormat,
    pub(super) include_hook_events: bool,
    pub(super) include_partial_messages: bool,
    pub(super) session_mirror: Option<std::path::PathBuf>,
    pub(super) permission_prompt_tool: Option<String>,
    pub(super) sdk_url: Option<String>,
    pub(super) custom_betas: Vec<String>,
    pub(super) fine_grained_tool_streaming: bool,
    pub(super) strict_tool_schemas: bool,
    pub(super) max_turns: Option<u32>,
}

/// `--print` headless one-shot mode, driven by the shared engine: the same
/// `ops::submit_prompt` + `handle_engine_event` dispatch the TUI uses, with
/// stream-json/text emission instead of rendering. This replaced a fully
/// duplicated turn loop (own provider stream, own tool executor, own
/// permission flow) as stage 4 of the jfc-engine extraction.
pub(super) async fn run_print_mode(
    provider: std::sync::Arc<dyn jfc_provider::Provider>,
    model: jfc_provider::ModelId,
    prompt: String,
    config: PrintModeConfig,
) -> anyhow::Result<()> {
    use crate::app::PermissionMode;
    use crate::runtime::{
        ControlEvent, EngineEvent, FrontendDirective, StreamEvent as EngineStreamEvent, ToolEvent,
    };

    let parsed_input = parse_headless_input(&prompt, config.input_format);
    let prompt_text = parsed_input.prompt.clone();
    let mut recovered_permission_responses = parsed_input.permission_responses;

    let (tx, mut rx) = jfc_engine::channel();
    // Engine::new registers the global tool-event sender, so plan-mode tools
    // / economy events reach this loop exactly as in the TUI (the old
    // duplicated loop silently dropped them).
    let mut engine = jfc_engine::Engine::new(provider, model.clone(), tx.clone());
    let state = &mut engine.state;
    // Print mode never persists sessions — the optional --session-mirror file
    // is its own wire-level persistence below.
    state.no_session_persistence = true;
    state.custom_betas = config.custom_betas.clone();
    state.fine_grained_tool_streaming = config.fine_grained_tool_streaming;
    state.strict_tool_schemas = config.strict_tool_schemas;
    let max_turns = config.max_turns.unwrap_or(10).max(1);
    state.max_turns = Some(max_turns);
    // Without a permission prompt endpoint every tool is auto-approved
    // (legacy behavior). With one, the engine's standard permission gate
    // applies and gated tools round-trip through the HTTP prompt below.
    state.permission_mode = if config.permission_prompt_tool.is_some() {
        PermissionMode::Default
    } else {
        PermissionMode::BypassPermissions
    };

    // Seed the transcript: session mirror first, then any stream-json input.
    let mut seed = config
        .session_mirror
        .as_ref()
        .and_then(|path| load_mirror_messages(path).ok())
        .unwrap_or_default();
    seed.extend(parsed_input.messages.clone());
    state.messages = chat_messages_from_provider(&seed);

    let session_id = config
        .session_mirror
        .as_ref()
        .and_then(|path| load_mirror_session_id(path).ok().flatten())
        .unwrap_or_else(headless_session_id);

    let mut stdout = std::io::stdout().lock();
    let mut mirror_events = Vec::new();
    if config.include_hook_events && config.output_format == HeadlessOutputFormat::StreamJson {
        emit_stream_json(
            &mut stdout,
            &mut mirror_events,
            &config,
            serde_json::json!({
                "type": "hook",
                "hook": "session_start",
                "session_id": &session_id,
            }),
        )?;
    }
    if config.output_format == HeadlessOutputFormat::StreamJson {
        emit_stream_json(
            &mut stdout,
            &mut mirror_events,
            &config,
            serde_json::json!({
                "type": "system",
                "subtype": "init",
                "session_id": &session_id,
                "model": model.as_str(),
                "input_format": input_format_wire(config.input_format),
                "output_format": output_format_wire(config.output_format),
                "permission_prompt_tool": config.permission_prompt_tool.as_deref(),
                "sdk_url": config.sdk_url.as_deref(),
            }),
        )?;
    }
    if config.include_hook_events && config.output_format == HeadlessOutputFormat::StreamJson {
        emit_stream_json(
            &mut stdout,
            &mut mirror_events,
            &config,
            serde_json::json!({"type": "hook", "hook": "before_stream", "session_id": &session_id}),
        )?;
    }

    // Kick the first turn: a plain prompt goes through the full submit op;
    // stream-json input that carried its own messages resumes the seeded
    // transcript directly.
    if parsed_input.messages.is_empty() {
        engine
            .submit(prompt_text.clone(), Vec::new(), None)
            .await?;
    } else {
        engine.start_turn_from_transcript(&prompt_text).await;
    }

    let mut accumulated = String::new();
    let mut stop_reason: Option<String> = Some("end_turn".to_owned());
    let mut usage_totals = UsageTotals::default();
    let mut exit_code = 0;
    let mut tool_use_index: usize = 0;

    while let Some(ev) = rx.recv().await {
        // ── 1. Wire emission (before dispatch consumes the event) ──
        match &ev {
            EngineEvent::Stream(EngineStreamEvent::Chunk { text, reasoning }) => {
                if let Some(delta) = text {
                    accumulated.push_str(delta);
                    match config.output_format {
                        HeadlessOutputFormat::Text => {
                            let _ = stdout.write_all(delta.as_bytes());
                            let _ = stdout.flush();
                        }
                        HeadlessOutputFormat::Json => {}
                        HeadlessOutputFormat::StreamJson => {
                            emit_stream_json(
                                &mut stdout,
                                &mut mirror_events,
                                &config,
                                serde_json::json!({
                                    "type": "assistant_delta",
                                    "delta": delta,
                                }),
                            )?;
                            if config.include_partial_messages {
                                emit_stream_json(
                                    &mut stdout,
                                    &mut mirror_events,
                                    &config,
                                    serde_json::json!({
                                        "type": "partial_message",
                                        "message": {
                                            "role": "assistant",
                                            "content": accumulated,
                                        },
                                    }),
                                )?;
                            }
                        }
                    }
                }
                if reasoning.is_some()
                    && config.output_format == HeadlessOutputFormat::StreamJson
                {
                    emit_stream_json(
                        &mut stdout,
                        &mut mirror_events,
                        &config,
                        serde_json::json!({
                            "type": "thinking_delta",
                            "delta": reasoning.as_deref().unwrap_or_default(),
                        }),
                    )?;
                }
            }
            EngineEvent::Stream(EngineStreamEvent::ToolInputDelta { index, delta }) => {
                if config.output_format == HeadlessOutputFormat::StreamJson {
                    emit_stream_json(
                        &mut stdout,
                        &mut mirror_events,
                        &config,
                        serde_json::json!({
                            "type": "tool_input_delta",
                            "index": index,
                            "delta": delta,
                        }),
                    )?;
                }
            }
            EngineEvent::Stream(EngineStreamEvent::Tool(tool)) => {
                if config.output_format == HeadlessOutputFormat::StreamJson {
                    emit_stream_json(
                        &mut stdout,
                        &mut mirror_events,
                        &config,
                        serde_json::json!({
                            "type": "tool_use",
                            "index": tool_use_index,
                            "id": tool.id.as_str(),
                            "name": tool.kind.label(),
                            "input": tool.input.to_value(),
                        }),
                    )?;
                }
                tool_use_index += 1;
            }
            EngineEvent::Stream(EngineStreamEvent::ServerToolResult {
                tool_use_id,
                tool_kind,
                content,
            }) => {
                if config.output_format == HeadlessOutputFormat::StreamJson {
                    emit_stream_json(
                        &mut stdout,
                        &mut mirror_events,
                        &config,
                        serde_json::json!({
                            "type": "server_tool_result",
                            "tool_use_id": tool_use_id.as_str(),
                            "tool_kind": tool_kind.wire_type(),
                            "content": content,
                        }),
                    )?;
                }
            }
            EngineEvent::Stream(EngineStreamEvent::RedactedThinking(data)) => {
                if config.output_format == HeadlessOutputFormat::StreamJson {
                    emit_stream_json(
                        &mut stdout,
                        &mut mirror_events,
                        &config,
                        serde_json::json!({
                            "type": "redacted_thinking",
                            "data": data,
                        }),
                    )?;
                }
            }
            EngineEvent::Stream(EngineStreamEvent::ResponseId { id, input_tokens }) => {
                if config.output_format == HeadlessOutputFormat::StreamJson {
                    emit_stream_json(
                        &mut stdout,
                        &mut mirror_events,
                        &config,
                        serde_json::json!({
                            "type": "response_metadata",
                            "response_id": id,
                            "input_tokens": input_tokens,
                        }),
                    )?;
                }
            }
            EngineEvent::Stream(EngineStreamEvent::Usage {
                input_tokens,
                output_tokens,
                cache_read_tokens,
                cache_write_tokens,
            }) => {
                usage_totals.add(
                    *input_tokens,
                    *output_tokens,
                    *cache_read_tokens,
                    *cache_write_tokens,
                );
                if config.output_format == HeadlessOutputFormat::StreamJson {
                    emit_stream_json(
                        &mut stdout,
                        &mut mirror_events,
                        &config,
                        serde_json::json!({
                            "type": "usage",
                            "usage": usage_totals.to_json(),
                        }),
                    )?;
                }
            }
            EngineEvent::Stream(EngineStreamEvent::FallbackTriggered {
                original_model,
                fallback_model,
                reason,
            }) => {
                if config.output_format == HeadlessOutputFormat::StreamJson {
                    emit_stream_json(
                        &mut stdout,
                        &mut mirror_events,
                        &config,
                        serde_json::json!({
                            "type": "fallback_triggered",
                            "original_model": original_model,
                            "fallback_model": fallback_model,
                            "reason": reason.to_string(),
                        }),
                    )?;
                }
            }
            EngineEvent::Stream(EngineStreamEvent::Error(message)) => {
                emit_headless_error(&mut stdout, &mut mirror_events, &config, message)?;
                exit_code = 1;
            }
            EngineEvent::Stream(EngineStreamEvent::Done(reason)) => {
                stop_reason = Some(stop_reason_wire(reason));
                tool_use_index = 0;
            }
            EngineEvent::Tool(ToolEvent::Result { tool_id, result }) => {
                if config.output_format == HeadlessOutputFormat::StreamJson {
                    emit_stream_json(
                        &mut stdout,
                        &mut mirror_events,
                        &config,
                        serde_json::json!({
                            "type": "tool_result",
                            "tool_use_id": tool_id.as_str(),
                            "content": &result.output,
                            "is_error": result.is_error(),
                        }),
                    )?;
                }
            }
            EngineEvent::Control(ControlEvent::Notice { .. }) => {
                // Engine notices (memory recall, compaction milestones) are
                // TUI toasts; print mode keeps the wire clean.
            }
            _ => {}
        }

        // ── 2. Dispatch through the shared engine pump ──
        match engine.handle_event(ev).await? {
            Some(FrontendDirective::SubmitPrompt(text)) => {
                // Pre-submit compaction re-fired the prompt.
                let _ = engine.submit(text, Vec::new(), None).await?;
            }
            Some(FrontendDirective::RunCommand(text)) => {
                // Engine command semantics are shared since stage 8 — print
                // mode runs /compact, /model, /task-* etc. like any frontend.
                let _ = jfc_engine::commands::run_command(&mut engine.state, &text, Some(&tx))
                    .await;
            }
            None => {}
        }
        // Print mode has no viewport: view effects are meaningless here.
        let _ = engine.drain_effects();

        // ── 3. Approval pump: resolve every parked tool via the HTTP
        //       permission prompt (or recovered stream-json responses). ──
        while let Some(pending_tool) = engine
            .state
            .pending_approval
            .as_ref()
            .map(|p| p.tool.clone())
        {
            let allowed = headless_permission_decision(
                &pending_tool,
                &config,
                &session_id,
                &mut recovered_permission_responses,
                &mut stdout,
                &mut mirror_events,
            )
            .await?;
            engine.resolve_approval(pending_tool.id.as_str().to_owned(), allowed);
        }

        // ── 4. Termination: the turn settled and nothing is in flight. ──
        if engine.is_idle() {
            break;
        }
    }

    if config.include_hook_events && config.output_format == HeadlessOutputFormat::StreamJson {
        emit_stream_json(
            &mut stdout,
            &mut mirror_events,
            &config,
            serde_json::json!({"type": "hook", "hook": "after_stream", "session_id": &session_id}),
        )?;
    }
    match config.output_format {
        HeadlessOutputFormat::Text => {
            let _ = stdout.write_all(b"\n");
        }
        HeadlessOutputFormat::Json => {
            write_json_line(
                &mut stdout,
                serde_json::json!({
                    "type": "result",
                    "session_id": &session_id,
                    "model": model.as_str(),
                    "stop_reason": stop_reason.clone(),
                    "content": accumulated,
                    "usage": usage_totals.to_json(),
                }),
            )?;
        }
        HeadlessOutputFormat::StreamJson => {
            emit_stream_json(
                &mut stdout,
                &mut mirror_events,
                &config,
                serde_json::json!({
                    "type": "result",
                    "session_id": &session_id,
                    "model": model.as_str(),
                    "stop_reason": stop_reason.clone(),
                    "content": accumulated,
                    "usage": usage_totals.to_json(),
                }),
            )?;
        }
    }
    let _ = stdout.flush();
    if let Some(path) = config.session_mirror {
        let wire_messages = jfc_engine::stream::build_provider_messages(&engine.state.messages);
        let mirror = serde_json::json!({
            "session_id": &session_id,
            "messages": wire_messages.iter().map(provider_message_to_json).collect::<Vec<_>>(),
            "events": mirror_events,
            "stop_reason": stop_reason.clone(),
            "usage": usage_totals.to_json(),
        });
        std::fs::write(path, serde_json::to_vec_pretty(&mirror)?)?;
    }
    if exit_code != 0 {
        std::process::exit(exit_code);
    }
    Ok(())
}

/// Best-effort reconstruction of an engine transcript from provider-wire
/// messages (session mirror / stream-json input). Text, tool_use,
/// tool_result, redacted-thinking, and server-tool-result blocks round-trip;
/// anything else is dropped with a log.
fn chat_messages_from_provider(
    messages: &[jfc_provider::ProviderMessage],
) -> Vec<jfc_core::ChatMessage> {
    use jfc_core::{ChatMessage, MessagePart, Role, ToolOutput, ToolStatus};
    use jfc_provider::{ProviderContent, ProviderRole};

    let mut out: Vec<ChatMessage> = Vec::new();
    for msg in messages {
        match msg.role {
            ProviderRole::User => {
                // Tool results attach to the matching pending tool_use in the
                // transcript; plain text becomes a user message.
                let mut texts: Vec<String> = Vec::new();
                for content in &msg.content {
                    match content {
                        ProviderContent::Text(t) => texts.push(t.clone()),
                        ProviderContent::ToolResult {
                            tool_use_id,
                            content,
                            is_error,
                        } => {
                            let mut attached = false;
                            'outer: for prev in out.iter_mut().rev() {
                                for part in prev.parts.iter_mut() {
                                    if let MessagePart::Tool(tc) = part
                                        && tc.id.as_str() == tool_use_id
                                    {
                                        tc.output = ToolOutput::Text(content.clone());
                                        tc.status = if *is_error {
                                            ToolStatus::Failed
                                        } else {
                                            ToolStatus::Completed
                                        };
                                        attached = true;
                                        break 'outer;
                                    }
                                }
                            }
                            if !attached {
                                tracing::warn!(
                                    target: "jfc::headless",
                                    tool_use_id,
                                    "orphaned tool_result in seeded transcript; dropping"
                                );
                            }
                        }
                        other => {
                            tracing::debug!(
                                target: "jfc::headless",
                                ?other,
                                "unsupported user content in seeded transcript; dropping"
                            );
                        }
                    }
                }
                if !texts.is_empty() {
                    out.push(ChatMessage::user(texts.join("\n")));
                }
            }
            ProviderRole::Assistant => {
                let mut parts: Vec<MessagePart> = Vec::new();
                for content in &msg.content {
                    match content {
                        ProviderContent::Text(t) => parts.push(MessagePart::Text(t.clone())),
                        ProviderContent::RedactedThinking { data } => {
                            parts.push(MessagePart::RedactedThinking(data.clone()));
                        }
                        ProviderContent::ToolUse {
                            id, name, input, ..
                        } => {
                            let kind = jfc_core::ToolKind::from_name(name);
                            match jfc_core::ToolInput::from_value(name, input.clone()) {
                                Ok(tool_input) => {
                                    parts.push(MessagePart::tool(
                                        jfc_core::ToolCall::new_pending(
                                            jfc_engine::ids::ToolId::from(id.clone()),
                                            kind,
                                            tool_input,
                                        ),
                                    ));
                                }
                                Err(err) => {
                                    tracing::warn!(
                                        target: "jfc::headless",
                                        name,
                                        %err,
                                        "tool_use in seeded transcript failed schema parse; dropping"
                                    );
                                }
                            }
                        }
                        other => {
                            tracing::debug!(
                                target: "jfc::headless",
                                ?other,
                                "unsupported assistant content in seeded transcript; dropping"
                            );
                        }
                    }
                }
                if !parts.is_empty() {
                    let mut m = ChatMessage::assistant(String::new());
                    m.parts = parts;
                    // ChatMessage::assistant seeds an empty Text part via the
                    // constructor on some paths; ensure role stays correct.
                    debug_assert_eq!(m.role, Role::Assistant);
                    out.push(m);
                }
            }
        }
    }
    out
}

/// Resolve a parked tool approval for print mode: recovered stream-json
/// permission responses first, then the `--permission-prompt-tool` HTTP
/// round-trip, defaulting to allow when no endpoint is configured (matching
/// the legacy headless flow).
async fn headless_permission_decision(
    tool: &jfc_core::ToolCall,
    config: &PrintModeConfig,
    session_id: &str,
    recovered_permission_responses: &mut HashMap<String, serde_json::Value>,
    stdout: &mut impl Write,
    mirror_events: &mut Vec<serde_json::Value>,
) -> anyhow::Result<bool> {
    let Some(prompt_tool) = config.permission_prompt_tool.as_deref() else {
        return Ok(true);
    };
    let tool_id = tool.id.as_str();
    if let Some(recovered) = recovered_permission_responses.remove(tool_id) {
        let allowed = permission_response_allows(&recovered);
        emit_stream_json(
            stdout,
            mirror_events,
            config,
            serde_json::json!({
                "type": "permission_response",
                "tool_use_id": tool_id,
                "decision": if allowed { "allow" } else { "deny" },
                "source": "input",
                "response": recovered,
            }),
        )?;
        return Ok(allowed);
    }
    let request = serde_json::json!({
        "type": "permission_request",
        "session_id": session_id,
        "tool_name": prompt_tool,
        "tool_use": {
            "id": tool_id,
            "name": tool.kind.label(),
            "input": tool.input.to_value(),
        },
        "status": "requested",
    });
    emit_stream_json(stdout, mirror_events, config, request.clone())?;
    let Some(url) = config.sdk_url.as_deref() else {
        return Ok(false);
    };
    let client = reqwest::Client::new();
    let response = tokio::time::timeout(
        std::time::Duration::from_secs(120),
        client.post(url).json(&request).send(),
    )
    .await
    .map_err(|_| anyhow::anyhow!("permission prompt timed out after 120s"))?
    .map_err(|e| anyhow::anyhow!("permission prompt request failed: {e}"))?;
    let status = response.status();
    if !status.is_success() {
        return Ok(false);
    }
    let body = response.text().await.unwrap_or_default();
    if body.trim().is_empty() {
        emit_stream_json(
            stdout,
            mirror_events,
            config,
            serde_json::json!({
                "type": "permission_response",
                "tool_use_id": tool_id,
                "decision": "allow",
            }),
        )?;
        return Ok(true);
    }
    let parsed: serde_json::Value = serde_json::from_str(&body)
        .map_err(|e| anyhow::anyhow!("permission prompt response was not JSON: {e}"))?;
    let allowed = permission_response_allows(&parsed);
    emit_stream_json(
        stdout,
        mirror_events,
        config,
        serde_json::json!({
            "type": "permission_response",
            "tool_use_id": tool_id,
            "decision": if allowed { "allow" } else { "deny" },
            "response": parsed,
        }),
    )?;
    Ok(allowed)
}

#[derive(Debug, Default)]
struct UsageTotals {
    input_tokens: u64,
    output_tokens: u64,
    cache_read_tokens: u64,
    cache_write_tokens: u64,
}

impl UsageTotals {
    fn add(
        &mut self,
        input_tokens: u32,
        output_tokens: u32,
        cache_read_tokens: u32,
        cache_write_tokens: u32,
    ) {
        self.input_tokens += u64::from(input_tokens);
        self.output_tokens += u64::from(output_tokens);
        self.cache_read_tokens += u64::from(cache_read_tokens);
        self.cache_write_tokens += u64::from(cache_write_tokens);
    }

    fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "input_tokens": self.input_tokens,
            "output_tokens": self.output_tokens,
            "cache_read_tokens": self.cache_read_tokens,
            "cache_write_tokens": self.cache_write_tokens,
        })
    }
}
fn permission_response_allows(value: &serde_json::Value) -> bool {
    if value.as_bool().unwrap_or(false) {
        return true;
    }
    let Some(obj) = value.as_object() else {
        return false;
    };
    for key in ["allow", "allowed", "approve", "approved"] {
        if obj.get(key).and_then(|v| v.as_bool()).unwrap_or(false) {
            return true;
        }
    }
    obj.get("decision")
        .or_else(|| obj.get("status"))
        .or_else(|| obj.get("result"))
        .and_then(|v| v.as_str())
        .is_some_and(|s| {
            matches!(
                s.to_ascii_lowercase().as_str(),
                "allow" | "allowed" | "approve" | "approved" | "yes"
            )
        })
}

fn emit_headless_error(
    out: &mut impl Write,
    mirror_events: &mut Vec<serde_json::Value>,
    config: &PrintModeConfig,
    message: &str,
) -> anyhow::Result<()> {
    match config.output_format {
        HeadlessOutputFormat::Text => eprintln!("\n[stream error: {message}]"),
        HeadlessOutputFormat::Json => {
            write_json_line(
                out,
                serde_json::json!({
                    "type": "error",
                    "message": message,
                }),
            )?;
        }
        HeadlessOutputFormat::StreamJson => {
            emit_stream_json(
                out,
                mirror_events,
                config,
                serde_json::json!({
                    "type": "error",
                    "message": message,
                }),
            )?;
        }
    }
    Ok(())
}

fn emit_stream_json(
    out: &mut impl Write,
    mirror_events: &mut Vec<serde_json::Value>,
    config: &PrintModeConfig,
    value: serde_json::Value,
) -> anyhow::Result<()> {
    mirror_events.push(value.clone());
    if config.output_format == HeadlessOutputFormat::StreamJson {
        write_json_line(out, value)?;
    }
    Ok(())
}

fn write_json_line(out: &mut impl Write, value: serde_json::Value) -> anyhow::Result<()> {
    serde_json::to_writer(&mut *out, &value)?;
    out.write_all(b"\n")?;
    out.flush()?;
    Ok(())
}

fn load_mirror_session_id(path: &std::path::Path) -> anyhow::Result<Option<String>> {
    if !path.exists() {
        return Ok(None);
    }
    let value: serde_json::Value = serde_json::from_slice(&std::fs::read(path)?)?;
    Ok(value
        .get("session_id")
        .and_then(|v| v.as_str())
        .map(str::to_owned))
}

fn load_mirror_messages(
    path: &std::path::Path,
) -> anyhow::Result<Vec<jfc_provider::ProviderMessage>> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let value: serde_json::Value = serde_json::from_slice(&std::fs::read(path)?)?;
    let Some(messages) = value.get("messages").and_then(|v| v.as_array()) else {
        return Ok(Vec::new());
    };
    Ok(messages
        .iter()
        .filter_map(provider_message_from_json)
        .collect())
}

fn provider_message_from_json(value: &serde_json::Value) -> Option<jfc_provider::ProviderMessage> {
    let role_value = value
        .get("role")
        .and_then(|v| v.as_str())
        .or_else(|| value.get("type").and_then(|v| v.as_str()))?;
    let role = match role_value {
        "user" => jfc_provider::ProviderRole::User,
        "user_message" => jfc_provider::ProviderRole::User,
        "assistant" => jfc_provider::ProviderRole::Assistant,
        "assistant_message" => jfc_provider::ProviderRole::Assistant,
        _ => return None,
    };
    let content_value = value
        .get("content")
        .or_else(|| value.get("message").and_then(|m| m.get("content")))?;
    let content = match content_value {
        serde_json::Value::String(text) => vec![jfc_provider::ProviderContent::Text(text.clone())],
        serde_json::Value::Array(items) => items
            .iter()
            .filter_map(provider_content_from_json)
            .collect::<Vec<_>>(),
        _ => Vec::new(),
    };
    if content.is_empty() {
        None
    } else {
        Some(jfc_provider::ProviderMessage { role, content })
    }
}

fn provider_content_from_json(value: &serde_json::Value) -> Option<jfc_provider::ProviderContent> {
    let ty = value.get("type").and_then(|v| v.as_str())?;
    match ty {
        "text" => value
            .get("text")
            .and_then(|v| v.as_str())
            .map(|text| jfc_provider::ProviderContent::Text(text.to_owned())),
        "tool_use" => Some(jfc_provider::ProviderContent::ToolUse {
            id: value.get("id")?.as_str()?.to_owned(),
            name: value.get("name")?.as_str()?.to_owned(),
            input: value
                .get("input")
                .cloned()
                .unwrap_or_else(|| serde_json::json!({})),
            thought_signature: value
                .get("thought_signature")
                .and_then(|v| v.as_str())
                .map(str::to_owned),
        }),
        "tool_result" => Some(jfc_provider::ProviderContent::ToolResult {
            tool_use_id: value.get("tool_use_id")?.as_str()?.to_owned(),
            content: value
                .get("content")
                .and_then(|v| v.as_str())
                .map(str::to_owned)
                .unwrap_or_else(|| {
                    value
                        .get("content")
                        .map(serde_json::Value::to_string)
                        .unwrap_or_default()
                }),
            is_error: value
                .get("is_error")
                .and_then(|v| v.as_bool())
                .unwrap_or(false),
        }),
        "server_tool_use" => Some(jfc_provider::ProviderContent::ServerToolUse {
            id: value.get("id")?.as_str()?.to_owned(),
            name: value.get("name")?.as_str()?.to_owned(),
            input: value
                .get("input")
                .cloned()
                .unwrap_or_else(|| serde_json::json!({})),
        }),
        "redacted_thinking" => value.get("data").and_then(|v| v.as_str()).map(|data| {
            jfc_provider::ProviderContent::RedactedThinking {
                data: data.to_owned(),
            }
        }),
        wire if wire.ends_with("_tool_result") => {
            Some(jfc_provider::ProviderContent::ServerToolResult {
                tool_use_id: value.get("tool_use_id")?.as_str()?.to_owned(),
                tool_kind: jfc_provider::ServerToolResultKind::from_wire_type(wire),
                content: value
                    .get("content")
                    .cloned()
                    .unwrap_or_else(|| serde_json::json!({})),
            })
        }
        _ => None,
    }
}

fn provider_message_to_json(message: &jfc_provider::ProviderMessage) -> serde_json::Value {
    let role = match message.role {
        jfc_provider::ProviderRole::User => "user",
        jfc_provider::ProviderRole::Assistant => "assistant",
    };
    serde_json::json!({
        "role": role,
        "content": message.content.iter().map(provider_content_to_json).collect::<Vec<_>>(),
    })
}

fn provider_content_to_json(content: &jfc_provider::ProviderContent) -> serde_json::Value {
    match content {
        jfc_provider::ProviderContent::Text(text) => {
            serde_json::json!({"type": "text", "text": text})
        }
        jfc_provider::ProviderContent::ToolResult {
            tool_use_id,
            content,
            is_error,
        } => serde_json::json!({
            "type": "tool_result",
            "tool_use_id": tool_use_id,
            "content": content,
            "is_error": is_error,
        }),
        jfc_provider::ProviderContent::ToolUse {
            id,
            name,
            input,
            thought_signature,
        } => serde_json::json!({
            "type": "tool_use",
            "id": id,
            "name": name,
            "input": input,
            "thought_signature": thought_signature,
        }),
        jfc_provider::ProviderContent::ServerToolUse { id, name, input } => serde_json::json!({
            "type": "server_tool_use",
            "id": id,
            "name": name,
            "input": input,
        }),
        jfc_provider::ProviderContent::ServerToolResult {
            tool_use_id,
            tool_kind,
            content,
        } => serde_json::json!({
            "type": tool_kind.wire_type(),
            "tool_use_id": tool_use_id,
            "content": content,
        }),
        jfc_provider::ProviderContent::RedactedThinking { data } => {
            serde_json::json!({"type": "redacted_thinking", "data": data})
        }
        jfc_provider::ProviderContent::Attachment(_) => {
            serde_json::json!({"type": "attachment", "unsupported": true})
        }
    }
}

struct ParsedHeadlessInput {
    prompt: String,
    messages: Vec<jfc_provider::ProviderMessage>,
    permission_responses: HashMap<String, serde_json::Value>,
}

fn parse_headless_input(raw: &str, format: HeadlessInputFormat) -> ParsedHeadlessInput {
    match format {
        HeadlessInputFormat::Text => ParsedHeadlessInput {
            prompt: raw.to_owned(),
            messages: Vec::new(),
            permission_responses: HashMap::new(),
        },
        HeadlessInputFormat::StreamJson => parse_stream_json_input(raw),
    }
}

fn parse_stream_json_input(input: &str) -> ParsedHeadlessInput {
    let mut messages = Vec::new();
    let mut prompts = Vec::new();
    let mut permission_responses = HashMap::new();

    for line in input.lines().map(str::trim).filter(|line| !line.is_empty()) {
        let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        if let Some(arr) = value.get("messages").and_then(|v| v.as_array()) {
            messages.extend(arr.iter().filter_map(provider_message_from_json));
            continue;
        }
        if let Some(message) = value.get("message").and_then(provider_message_from_json) {
            messages.push(message);
            continue;
        }
        if let Some(response_id) = permission_response_id(&value) {
            permission_responses.insert(response_id, value.clone());
            continue;
        }
        if value.get("type").and_then(|v| v.as_str()) == Some("tool_result") {
            if let Some(content) = provider_content_from_json(&value) {
                messages.push(jfc_provider::ProviderMessage {
                    role: jfc_provider::ProviderRole::User,
                    content: vec![content],
                });
            }
            continue;
        }
        if let Some(message) = provider_message_from_json(&value) {
            messages.push(message);
            continue;
        }
        if let Some(prompt) = value.get("prompt").and_then(|v| v.as_str()) {
            prompts.push(prompt.to_owned());
        }
    }

    if messages.is_empty() {
        let prompt = if prompts.is_empty() {
            extract_stream_json_prompt(input).unwrap_or_else(|| input.to_owned())
        } else {
            prompts.join("\n")
        };
        ParsedHeadlessInput {
            prompt,
            messages: Vec::new(),
            permission_responses,
        }
    } else {
        let prompt = prompts.join("\n");
        ParsedHeadlessInput {
            prompt,
            messages,
            permission_responses,
        }
    }
}

fn permission_response_id(value: &serde_json::Value) -> Option<String> {
    let ty = value.get("type").and_then(|v| v.as_str())?;
    if ty != "permission_response" && ty != "permissionResult" {
        return None;
    }
    value
        .get("tool_use_id")
        .or_else(|| value.get("toolUseId"))
        .or_else(|| value.get("id"))
        .and_then(|v| v.as_str())
        .map(str::to_owned)
        .or_else(|| {
            value
                .get("tool_use")
                .or_else(|| value.get("toolUse"))
                .and_then(|tool| tool.get("id"))
                .and_then(|v| v.as_str())
                .map(str::to_owned)
        })
}
fn stop_reason_wire(reason: &jfc_provider::StopReason) -> String {
    match reason {
        jfc_provider::StopReason::EndTurn => "end_turn".to_owned(),
        jfc_provider::StopReason::ToolUse => "tool_use".to_owned(),
        jfc_provider::StopReason::PauseTurn => "pause_turn".to_owned(),
        jfc_provider::StopReason::Refusal => "refusal".to_owned(),
        jfc_provider::StopReason::MaxTokens => "max_tokens".to_owned(),
        jfc_provider::StopReason::StopSequence => "stop_sequence".to_owned(),
        jfc_provider::StopReason::Other(value) => value.clone(),
    }
}

fn headless_session_id() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    format!("headless-{:x}{:08x}", now.as_secs(), now.subsec_nanos())
}

fn input_format_wire(format: HeadlessInputFormat) -> &'static str {
    match format {
        HeadlessInputFormat::Text => "text",
        HeadlessInputFormat::StreamJson => "stream-json",
    }
}

fn output_format_wire(format: HeadlessOutputFormat) -> &'static str {
    match format {
        HeadlessOutputFormat::Text => "text",
        HeadlessOutputFormat::Json => "json",
        HeadlessOutputFormat::StreamJson => "stream-json",
    }
}

fn extract_stream_json_prompt(input: &str) -> Option<String> {
    let mut parts = Vec::new();
    for line in input.lines().map(str::trim).filter(|line| !line.is_empty()) {
        let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        if let Some(prompt) = value.get("prompt").and_then(|v| v.as_str()) {
            parts.push(prompt.to_owned());
            continue;
        }
        if value.get("type").and_then(|v| v.as_str()) != Some("user")
            && value.get("type").and_then(|v| v.as_str()) != Some("user_message")
        {
            continue;
        }
        if let Some(text) = value.get("content").and_then(|v| v.as_str()) {
            parts.push(text.to_owned());
            continue;
        }
        if let Some(content) = value
            .get("message")
            .and_then(|m| m.get("content"))
            .and_then(content_text)
        {
            parts.push(content);
        }
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join("\n"))
    }
}

fn content_text(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::String(s) => Some(s.clone()),
        serde_json::Value::Array(items) => {
            let mut out = Vec::new();
            for item in items {
                if item.get("type").and_then(|v| v.as_str()) == Some("text")
                    && let Some(text) = item.get("text").and_then(|v| v.as_str())
                {
                    out.push(text.to_owned());
                }
            }
            if out.is_empty() {
                None
            } else {
                Some(out.join("\n"))
            }
        }
        _ => None,
    }
}

/// `--remote-session <id>` entry. Streams events from a managed-agent
/// session to stdout. Minimal first cut — full TUI integration with
/// rendering of v132's 17 event types lives in `managed_session.rs`
/// and ships behind a follow-on flag once the eventer is verified.
pub(super) async fn run_remote_session(
    client: jfc_anthropic_sdk::Client,
    session_id: String,
) -> anyhow::Result<()> {
    use futures::StreamExt;

    let session = jfc_engine::managed_session::ManagedSession::new(client, session_id.clone());
    eprintln!("--remote-session: subscribing to session {session_id}");
    let mut stream = session
        .connect()
        .await
        .map_err(|e| anyhow::anyhow!("session connect: {e}"))?;
    while let Some(event) = stream.next().await {
        match event {
            Ok(ev) => {
                println!("{}", jfc_engine::managed_session::render_event_line(&ev));
            }
            Err(e) => {
                eprintln!("[stream error: {e}]");
                break;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use jfc_provider::{ProviderContent, ProviderRole};

    #[test]
    fn stream_json_input_parses_messages_and_tool_results() {
        let input = r#"{"type":"user","content":"fix it"}"#.to_owned()
            + "\n"
            + r#"{"type":"assistant","content":[{"type":"tool_use","id":"toolu_1","name":"Read","input":{"file_path":"Cargo.toml"}}]}"#
            + "\n"
            + r#"{"type":"tool_result","tool_use_id":"toolu_1","content":"ok","is_error":false}"#;

        let parsed = parse_stream_json_input(&input);
        assert_eq!(parsed.messages.len(), 3);
        assert_eq!(parsed.messages[0].role, ProviderRole::User);
        assert!(matches!(
            parsed.messages[1].content[0],
            ProviderContent::ToolUse { .. }
        ));
        assert!(matches!(
            parsed.messages[2].content[0],
            ProviderContent::ToolResult { .. }
        ));
    }

    #[test]
    fn stream_json_input_recovers_orphaned_permission_response() {
        let input = r#"{"type":"permission_response","tool_use_id":"toolu_1","decision":"ALLOW"}"#;
        let parsed = parse_stream_json_input(input);
        assert!(parsed.permission_responses.contains_key("toolu_1"));
        assert!(permission_response_allows(
            parsed.permission_responses.get("toolu_1").unwrap()
        ));
    }
}

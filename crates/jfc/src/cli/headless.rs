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

/// `--print` headless one-shot mode. Builds a minimal stream against
/// the active provider and exits with the stream's stop_reason. Text
/// output preserves the legacy behavior; JSON modes emit SDK-style
/// events for scripted callers.
pub(super) async fn run_print_mode(
    provider: std::sync::Arc<dyn jfc_provider::Provider>,
    model: jfc_provider::ModelId,
    prompt: String,
    config: PrintModeConfig,
) -> anyhow::Result<()> {
    use futures::StreamExt;
    use jfc_provider::{
        ProviderContent, ProviderMessage, ProviderRole, StopReason, StreamEvent, StreamOptions,
    };

    let parsed_input = parse_headless_input(&prompt, config.input_format);
    let prompt = parsed_input.prompt.clone();
    let mut recovered_permission_responses = parsed_input.permission_responses;
    let mut messages = config
        .session_mirror
        .as_ref()
        .and_then(|path| load_mirror_messages(path).ok())
        .unwrap_or_default();
    if parsed_input.messages.is_empty() {
        messages.push(ProviderMessage {
            role: ProviderRole::User,
            content: vec![ProviderContent::Text(prompt.clone())],
        });
    } else {
        messages.extend(parsed_input.messages);
    }
    let advisor_model = if matches!(
        provider.stream_convention(),
        jfc_provider::StreamConvention::AnthropicNative
    ) && matches!(provider.name(), "anthropic" | "anthropic-oauth")
    {
        crate::advisor::active_server_advisor_model()
    } else {
        None
    };
    let mut opts = StreamOptions::new(model.clone())
        .max_tokens(8192)
        .custom_betas(config.custom_betas.clone());
    let pewter_owl_header = crate::feature_gates::pewter_owl_header_enabled(model.as_str(), true);
    let pewter_owl_tool = crate::feature_gates::pewter_owl_tool_enabled(model.as_str(), true);
    let pewter_owl_brief = crate::feature_gates::pewter_owl_brief_enabled(model.as_str(), true);
    if config.fine_grained_tool_streaming {
        opts = opts.eager_input_streaming(true);
    }
    if config.strict_tool_schemas {
        opts = opts.strict_tool_schemas(true);
    }
    if pewter_owl_header {
        opts = opts.narration_summaries(true);
    }
    let mut advertised_tools = crate::tools::all_tool_defs_with_mcp().await;
    crate::tools::apply_send_user_message_policy(
        &mut advertised_tools,
        pewter_owl_brief,
        pewter_owl_tool,
    );
    opts = opts.tools(advertised_tools);
    let mut system_prompt = String::new();
    if pewter_owl_brief {
        system_prompt.push_str(
            "Plain assistant text is hidden from the main chat view. Put every \
             substantive user-facing reply in `SendUserMessage`; use normal \
             assistant text only for internal reasoning that can be omitted \
             from the user's visible transcript.",
        );
    } else if pewter_owl_tool {
        system_prompt.push_str(
            "`SendUserMessage` is available for exact user-visible content \
             between tool calls, such as generated snippets, specific values, \
             and direct replies to mid-task user messages. Routine narration \
             and final answers may remain normal assistant text.",
        );
    }
    if let Some(advisor_model) = advisor_model {
        if !system_prompt.is_empty() {
            system_prompt.push_str("\n\n");
        }
        system_prompt.push_str(crate::advisor::SERVER_ADVISOR_SYSTEM_PROMPT);
        opts = opts.advisor_model(advisor_model);
    }
    if !system_prompt.is_empty() {
        opts = opts.system(system_prompt);
    }
    let mut stdout = std::io::stdout().lock();
    let mut exit_code = 0;
    let mut accumulated = String::new();
    let mut stop_reason = Some("end_turn".to_owned());
    let mut usage_totals = UsageTotals::default();
    let session_id = config
        .session_mirror
        .as_ref()
        .and_then(|path| load_mirror_session_id(path).ok().flatten())
        .unwrap_or_else(headless_session_id);
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
    let max_turns = config.max_turns.unwrap_or(10).max(1);
    for turn_idx in 0..max_turns {
        let mut stream = provider
            .stream(messages.clone(), &opts)
            .await
            .map_err(|e| anyhow::anyhow!("stream open failed: {e}"))?;
        let mut turn_text = String::new();
        let mut turn_content = Vec::new();
        let mut pending_tools = Vec::new();
        let mut tool_inputs: HashMap<usize, String> = HashMap::new();
        let mut turn_stop_reason = StopReason::EndTurn;

        while let Some(event) = stream.next().await {
            match event {
                Ok(StreamEvent::TextDelta { delta, .. }) => {
                    accumulated.push_str(&delta);
                    turn_text.push_str(&delta);
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
                Ok(StreamEvent::ThinkingDelta { delta, .. }) => {
                    if config.output_format == HeadlessOutputFormat::StreamJson {
                        emit_stream_json(
                            &mut stdout,
                            &mut mirror_events,
                            &config,
                            serde_json::json!({
                                "type": "thinking_delta",
                                "delta": delta,
                            }),
                        )?;
                    }
                }
                Ok(StreamEvent::ToolDelta { index, delta }) => {
                    tool_inputs.entry(index).or_default().push_str(&delta);
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
                Ok(StreamEvent::ToolDone {
                    index,
                    tool_name,
                    tool_use_id,
                    input_json,
                    thought_signature,
                }) => {
                    if !turn_text.is_empty() {
                        turn_content.push(ProviderContent::Text(std::mem::take(&mut turn_text)));
                    }
                    let input_json = if input_json.is_empty() {
                        tool_inputs.remove(&index).unwrap_or_default()
                    } else {
                        input_json
                    };
                    let input = parse_json_or_string(&input_json);
                    turn_content.push(ProviderContent::ToolUse {
                        id: tool_use_id.clone(),
                        name: tool_name.clone(),
                        input: input.clone(),
                        thought_signature,
                    });
                    pending_tools.push(HeadlessToolUse {
                        id: tool_use_id.clone(),
                        name: tool_name.clone(),
                        input: input.clone(),
                    });
                    if config.output_format == HeadlessOutputFormat::StreamJson {
                        emit_stream_json(
                            &mut stdout,
                            &mut mirror_events,
                            &config,
                            serde_json::json!({
                                "type": "tool_use",
                                "index": index,
                                "id": tool_use_id,
                                "name": tool_name,
                                "input": input,
                            }),
                        )?;
                    }
                }
                Ok(StreamEvent::ServerToolResult {
                    tool_use_id,
                    tool_kind,
                    content,
                }) => {
                    if !turn_text.is_empty() {
                        turn_content.push(ProviderContent::Text(std::mem::take(&mut turn_text)));
                    }
                    turn_content.push(ProviderContent::ServerToolResult {
                        tool_use_id: tool_use_id.clone(),
                        tool_kind: tool_kind.clone(),
                        content: content.clone(),
                    });
                    if config.output_format == HeadlessOutputFormat::StreamJson {
                        emit_stream_json(
                            &mut stdout,
                            &mut mirror_events,
                            &config,
                            serde_json::json!({
                                "type": "server_tool_result",
                                "tool_use_id": tool_use_id,
                                "tool_kind": tool_kind.wire_type(),
                                "content": content,
                            }),
                        )?;
                    }
                }
                Ok(StreamEvent::RedactedThinkingDone { data, .. }) => {
                    turn_content.push(ProviderContent::RedactedThinking { data: data.clone() });
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
                Ok(StreamEvent::ResponseMetadata {
                    response_id,
                    input_tokens,
                }) => {
                    if config.output_format == HeadlessOutputFormat::StreamJson {
                        emit_stream_json(
                            &mut stdout,
                            &mut mirror_events,
                            &config,
                            serde_json::json!({
                                "type": "response_metadata",
                                "response_id": response_id,
                                "input_tokens": input_tokens,
                            }),
                        )?;
                    }
                }
                Ok(StreamEvent::Usage {
                    input_tokens,
                    output_tokens,
                    cache_read_tokens,
                    cache_write_tokens,
                }) => {
                    usage_totals.add(
                        input_tokens,
                        output_tokens,
                        cache_read_tokens,
                        cache_write_tokens,
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
                Ok(StreamEvent::FallbackTriggered(info)) => {
                    if config.output_format == HeadlessOutputFormat::StreamJson {
                        emit_stream_json(
                            &mut stdout,
                            &mut mirror_events,
                            &config,
                            serde_json::json!({
                                "type": "fallback_triggered",
                                "original_model": info.original_model.as_str(),
                                "fallback_model": info.fallback_model.as_str(),
                                "reason": info.reason.to_string(),
                            }),
                        )?;
                    }
                }
                Ok(StreamEvent::Error { message }) => {
                    emit_headless_error(&mut stdout, &mut mirror_events, &config, &message)?;
                    exit_code = 1;
                    break;
                }
                Ok(StreamEvent::Done { stop_reason: r }) => {
                    turn_stop_reason = r;
                    break;
                }
                Ok(StreamEvent::TextDone { .. }) | Ok(StreamEvent::ThinkingDone { .. }) => {}
                Err(e) => {
                    emit_headless_error(&mut stdout, &mut mirror_events, &config, &e.to_string())?;
                    exit_code = 1;
                    break;
                }
            }
        }

        if !turn_text.is_empty() {
            turn_content.push(ProviderContent::Text(turn_text));
        }
        if !turn_content.is_empty() {
            messages.push(ProviderMessage {
                role: ProviderRole::Assistant,
                content: turn_content,
            });
        }
        stop_reason = Some(stop_reason_wire(&turn_stop_reason));

        if exit_code != 0 {
            break;
        }

        match turn_stop_reason {
            StopReason::ToolUse if !pending_tools.is_empty() => {
                let mut results = Vec::with_capacity(pending_tools.len());
                for tool in pending_tools {
                    let result = execute_headless_tool(
                        &tool,
                        &config,
                        &session_id,
                        &mut recovered_permission_responses,
                        &mut stdout,
                        &mut mirror_events,
                    )
                    .await?;
                    results.push(ProviderContent::ToolResult {
                        tool_use_id: tool.id,
                        content: result.output,
                        is_error: result.is_error,
                    });
                }
                messages.push(ProviderMessage {
                    role: ProviderRole::User,
                    content: results,
                });
            }
            StopReason::PauseTurn if turn_idx + 1 < max_turns => {
                if config.output_format == HeadlessOutputFormat::StreamJson {
                    emit_stream_json(
                        &mut stdout,
                        &mut mirror_events,
                        &config,
                        serde_json::json!({
                            "type": "pause_turn_resume",
                            "turn": turn_idx + 1,
                        }),
                    )?;
                }
            }
            StopReason::ToolUse | StopReason::PauseTurn => {
                let message = format!(
                    "headless agent loop stopped after {max_turns} turn(s) with stop_reason={}",
                    stop_reason.as_deref().unwrap_or("unknown")
                );
                emit_headless_error(&mut stdout, &mut mirror_events, &config, &message)?;
                exit_code = 1;
                break;
            }
            _ => break,
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
        let mirror = serde_json::json!({
            "session_id": &session_id,
            "messages": messages.iter().map(provider_message_to_json).collect::<Vec<_>>(),
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

#[derive(Debug, Clone)]
struct HeadlessToolUse {
    id: String,
    name: String,
    input: serde_json::Value,
}

#[derive(Debug, Clone)]
struct HeadlessToolResult {
    output: String,
    is_error: bool,
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

async fn execute_headless_tool(
    tool: &HeadlessToolUse,
    config: &PrintModeConfig,
    session_id: &str,
    recovered_permission_responses: &mut HashMap<String, serde_json::Value>,
    stdout: &mut impl Write,
    mirror_events: &mut Vec<serde_json::Value>,
) -> anyhow::Result<HeadlessToolResult> {
    match request_headless_permission(
        tool,
        config,
        session_id,
        recovered_permission_responses,
        stdout,
        mirror_events,
    )
    .await?
    {
        PermissionDecision::Allow => {}
        PermissionDecision::Deny(reason) => {
            let output = format!("Permission denied for {}: {reason}", tool.name);
            emit_stream_json(
                stdout,
                mirror_events,
                config,
                serde_json::json!({
                    "type": "tool_result",
                    "tool_use_id": &tool.id,
                    "content": output,
                    "is_error": true,
                }),
            )?;
            return Ok(HeadlessToolResult {
                output,
                is_error: true,
            });
        }
    }

    let input = match crate::types::ToolInput::from_value(&tool.name, tool.input.clone()) {
        Ok(input) => input,
        Err(err) => {
            let output = format!(
                "Tool input for {} did not match the local schema: {err}",
                tool.name
            );
            emit_stream_json(
                stdout,
                mirror_events,
                config,
                serde_json::json!({
                    "type": "tool_result",
                    "tool_use_id": &tool.id,
                    "content": output,
                    "is_error": true,
                }),
            )?;
            return Ok(HeadlessToolResult {
                output,
                is_error: true,
            });
        }
    };
    let kind = crate::types::ToolKind::from_name(&tool.name);
    let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    let result = crate::tools::execute_tool(kind, input, cwd, None, None, None).await;
    let is_error = result.is_error();
    let output = result.output;
    emit_stream_json(
        stdout,
        mirror_events,
        config,
        serde_json::json!({
            "type": "tool_result",
            "tool_use_id": &tool.id,
            "content": &output,
            "is_error": is_error,
        }),
    )?;
    Ok(HeadlessToolResult { output, is_error })
}

enum PermissionDecision {
    Allow,
    Deny(String),
}

async fn request_headless_permission(
    tool: &HeadlessToolUse,
    config: &PrintModeConfig,
    session_id: &str,
    recovered_permission_responses: &mut HashMap<String, serde_json::Value>,
    stdout: &mut impl Write,
    mirror_events: &mut Vec<serde_json::Value>,
) -> anyhow::Result<PermissionDecision> {
    let Some(prompt_tool) = config.permission_prompt_tool.as_deref() else {
        return Ok(PermissionDecision::Allow);
    };
    if let Some(recovered) = recovered_permission_responses.remove(&tool.id) {
        let allowed = permission_response_allows(&recovered);
        emit_stream_json(
            stdout,
            mirror_events,
            config,
            serde_json::json!({
                "type": "permission_response",
                "tool_use_id": &tool.id,
                "decision": if allowed { "allow" } else { "deny" },
                "source": "input",
                "response": recovered,
            }),
        )?;
        return if allowed {
            Ok(PermissionDecision::Allow)
        } else {
            Ok(PermissionDecision::Deny(
                "permission response from input denied the tool".to_owned(),
            ))
        };
    }
    let request = serde_json::json!({
        "type": "permission_request",
        "session_id": session_id,
        "tool_name": prompt_tool,
        "tool_use": {
            "id": &tool.id,
            "name": &tool.name,
            "input": &tool.input,
        },
        "status": "requested",
    });
    emit_stream_json(stdout, mirror_events, config, request.clone())?;
    let Some(url) = config.sdk_url.as_deref() else {
        return Ok(PermissionDecision::Deny(
            "--permission-prompt-tool was set but --sdk-url was not provided".to_owned(),
        ));
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
        return Ok(PermissionDecision::Deny(format!(
            "permission prompt endpoint returned HTTP {status}"
        )));
    }
    let body = response.text().await.unwrap_or_default();
    if body.trim().is_empty() {
        emit_stream_json(
            stdout,
            mirror_events,
            config,
            serde_json::json!({
                "type": "permission_response",
                "tool_use_id": &tool.id,
                "decision": "allow",
            }),
        )?;
        return Ok(PermissionDecision::Allow);
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
            "tool_use_id": &tool.id,
            "decision": if allowed { "allow" } else { "deny" },
            "response": parsed,
        }),
    )?;
    if allowed {
        Ok(PermissionDecision::Allow)
    } else {
        Ok(PermissionDecision::Deny(
            "permission prompt endpoint denied the tool".to_owned(),
        ))
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

fn parse_json_or_string(raw: &str) -> serde_json::Value {
    if raw.trim().is_empty() {
        return serde_json::json!({});
    }
    serde_json::from_str(raw).unwrap_or_else(|_| serde_json::json!(raw))
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

    let session = crate::managed_session::ManagedSession::new(client, session_id.clone());
    eprintln!("--remote-session: subscribing to session {session_id}");
    let mut stream = session
        .connect()
        .await
        .map_err(|e| anyhow::anyhow!("session connect: {e}"))?;
    while let Some(event) = stream.next().await {
        match event {
            Ok(ev) => {
                println!("{}", crate::managed_session::render_event_line(&ev));
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

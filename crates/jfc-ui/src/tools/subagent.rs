use std::path::{Path, PathBuf};

use tracing::warn;

use super::{ExecutionResult, all_tool_defs, execute_tool};
use crate::provider::ToolDef;
use crate::types::{ToolInput, ToolKind};

pub(super) async fn execute_skill_in(
    cwd: &Path,
    name: &str,
    args: Option<&str>,
) -> ExecutionResult {
    let skills = crate::agents::load_skills(cwd);
    // Be permissive with what the model passes in. v126 lets the model
    // call a skill by its name (`do-178b`), but in practice the model
    // sometimes passes the inner-file path it sees in the listing
    // (`do-178b/SKILL`) or the full `.md` filename. Strip the suffix
    // and any "/SKILL" tail before lookup so a small naming wobble
    // doesn't return Unknown.
    let normalized = name
        .trim()
        .trim_end_matches(".md")
        .trim_end_matches("/SKILL")
        .trim_end_matches("/Skill")
        .trim_end_matches("/skill")
        .trim_end_matches('/');
    let candidates: [&str; 2] = [normalized, name];
    let found = candidates
        .iter()
        .find_map(|c| crate::agents::find_skill_by_name(&skills, c));
    match found {
        Some(skill) => {
            let body = match args.filter(|s| !s.is_empty()) {
                Some(a) => format!("{}\n\n# Args\n{}", skill.body, a),
                None => skill.body.clone(),
            };
            ExecutionResult::success(body)
        }
        None => {
            // Surface the available skills in the error so the model
            // can self-correct without having to ask the user. The
            // previous bare "Unknown skill: do-178b" gave it nothing
            // to recover with.
            let available: Vec<&str> = skills.iter().map(|s| s.name.as_str()).collect();
            let suffix = if available.is_empty() {
                String::from(" (no skills installed)")
            } else {
                format!(". Available: {}", available.join(", "))
            };
            ExecutionResult::failure(format!("Unknown skill: {name}{suffix}"))
        }
    }
}

/// Default agentic-loop bound when an agent definition doesn't pin one.
/// Generous enough that legitimate multi-tool tasks complete; tight enough
/// Safety cap for subagent turns. Claude Code has no fixed limit — agents
/// run until end_turn or abort. We keep a generous cap to prevent truly
/// runaway agents (e.g. infinite tool loops), but set it high enough that
/// real multi-step tasks complete normally. Override per-agent via
/// `agent_def.max_turns`.
const DEFAULT_AGENT_MAX_TURNS: u32 = 200;

/// Apply an agent's `allowedTools` (allowlist) and `disallowedTools`
/// (blocklist) to the parent's full tool catalogue. An empty `allowed`
/// means "all tools allowed" (matches v126 conventions); a non-empty
/// `allowed` is exact membership. `disallowed` always subtracts.
/// The Task tool itself is also dropped — recursive subagent spawning
/// is intentionally not wired (would deadlock the single-stream model).
pub(super) fn filter_tools_for_agent(
    all: Vec<ToolDef>,
    allowed: &[String],
    disallowed: &[String],
) -> Vec<ToolDef> {
    let allow_all = allowed.is_empty();
    all.into_iter()
        .filter(|t| {
            if t.name.eq_ignore_ascii_case("Task") {
                return false;
            }
            if !allow_all && !allowed.iter().any(|a| a.eq_ignore_ascii_case(&t.name)) {
                return false;
            }
            !disallowed.iter().any(|d| d.eq_ignore_ascii_case(&t.name))
        })
        .collect()
}

fn subagent_model_alias(model: &str, provider_name: &str) -> String {
    match (model.trim().to_ascii_lowercase().as_str(), provider_name) {
        ("haiku", "openwebui") => "bedrock-claude-4-5-haiku".to_string(),
        ("sonnet", "openwebui") => "bedrock-claude-4-6-sonnet".to_string(),
        ("opus", "openwebui") => "bedrock-claude-4-6-opus".to_string(),
        ("haiku", _) => "claude-haiku-4-5".to_string(),
        ("sonnet", _) => "claude-sonnet-4-6".to_string(),
        ("opus", _) => "claude-opus-4-6".to_string(),
        _ => model.trim().to_string(),
    }
}

pub(crate) fn selected_subagent_model(
    task_input: &crate::types::TaskInput,
    agent_def: Option<&crate::agents::AgentDef>,
    parent_model: crate::provider::ModelId,
    provider_name: &str,
) -> Result<crate::provider::ModelId, String> {
    let raw = std::env::var("CLAUDE_CODE_SUBAGENT_MODEL")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .or_else(|| task_input.model.clone())
        .or_else(|| agent_def.and_then(|a| a.model.clone()));

    let Some(raw) = raw else {
        return Ok(parent_model);
    };

    if raw.eq_ignore_ascii_case("inherit") || raw.eq_ignore_ascii_case("parent") {
        return Ok(parent_model);
    }

    let aliased = subagent_model_alias(&raw, provider_name);
    let spec = crate::provider::ModelSpec::parse_lenient(&aliased)
        .map_err(|e| format!("invalid subagent model {raw:?}: {e}"))?;

    if let Some(prefix) = spec.provider() {
        if prefix.as_str() != provider_name {
            return Err(format!(
                "subagent model {aliased:?} targets provider {prefix}, but the active provider is {provider_name}; provider switching for subagents is not wired yet"
            ));
        }
    }

    Ok(spec.into_model())
}

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, path::PathBuf, sync::Mutex};

    use super::*;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn task_input(model: Option<&str>) -> crate::types::TaskInput {
        crate::types::TaskInput {
            description: "inspect".to_string(),
            prompt: "inspect".to_string(),
            subagent_type: Some("explore".to_string()),
            category: None,
            run_in_background: false,
            model: model.map(str::to_string),
            name: None,
            team_name: None,
            mode: None,
            isolation: None,
        }
    }

    fn agent_model(model: Option<&str>) -> crate::agents::AgentDef {
        crate::agents::AgentDef {
            name: "Explore".to_string(),
            source: PathBuf::from("builtin"),
            model: model.map(str::to_string),
            isolation: None,
            skills: Vec::new(),
            allowed_tools: Vec::new(),
            disallowed_tools: Vec::new(),
            permission_mode: None,
            forks_parent_context: None,
            background: None,
            color: None,
            effort: None,
            max_turns: None,
            max_input_tokens: None,
            memory: None,
            mcp_servers: Vec::new(),
            hooks: HashMap::new(),
            key_trigger: None,
            use_when: Vec::new(),
            avoid_when: Vec::new(),
            cost: None,
            system_prompt: String::new(),
        }
    }

    #[test]
    fn selected_subagent_model_uses_agent_model_before_parent() {
        let _guard = ENV_LOCK.lock().unwrap();
        unsafe { std::env::remove_var("CLAUDE_CODE_SUBAGENT_MODEL") };

        let model = selected_subagent_model(
            &task_input(None),
            Some(&agent_model(Some("haiku"))),
            crate::provider::ModelId::new("claude-opus-4-6"),
            "openwebui",
        )
        .unwrap();

        assert_eq!(model.as_str(), "bedrock-claude-4-5-haiku");
    }

    #[test]
    fn selected_subagent_model_env_overrides_task_model() {
        let _guard = ENV_LOCK.lock().unwrap();
        unsafe { std::env::set_var("CLAUDE_CODE_SUBAGENT_MODEL", "haiku") };

        let model = selected_subagent_model(
            &task_input(Some("opus")),
            Some(&agent_model(Some("sonnet"))),
            crate::provider::ModelId::new("claude-opus-4-6"),
            "openwebui",
        )
        .unwrap();

        unsafe { std::env::remove_var("CLAUDE_CODE_SUBAGENT_MODEL") };
        assert_eq!(model.as_str(), "bedrock-claude-4-5-haiku");
    }

    #[test]
    fn selected_subagent_model_rejects_cross_provider_models() {
        let _guard = ENV_LOCK.lock().unwrap();
        unsafe { std::env::remove_var("CLAUDE_CODE_SUBAGENT_MODEL") };

        let error = selected_subagent_model(
            &task_input(Some("anthropic/claude-haiku-4-5")),
            None,
            crate::provider::ModelId::new("bedrock-claude-4-6-opus"),
            "openwebui",
        )
        .unwrap_err();

        assert!(error.contains("provider switching for subagents is not wired yet"));
    }
}

/// Run a subagent. The agent gets its own system prompt, tool catalogue
/// (filtered by the agent's allow/disallow lists), an optional cwd
/// override (used for worktree isolation), and a turn cap from
/// `agent_def.max_turns` (defaults to `DEFAULT_AGENT_MAX_TURNS`).
///
/// This is a real agentic loop — when the subagent emits `tool_use`,
/// we execute the tool here and feed the `tool_result` back to the
/// model on the next iteration, exactly like the parent stream loop in
/// `stream::stream_response`. Without the loop the subagent could never
/// `Read` a file or run `Bash`; it could only produce prose.
pub async fn execute_task(
    task_input: &crate::types::TaskInput,
    provider: &dyn crate::provider::Provider,
    model_id: crate::provider::ModelId,
    tx: Option<&tokio::sync::mpsc::Sender<crate::app::AppEvent>>,
    task_id: Option<&str>,
    agent_def: Option<&crate::agents::AgentDef>,
    cwd_override: Option<PathBuf>,
) -> ExecutionResult {
    use crate::provider::{
        ProviderContent, ProviderMessage, ProviderRole, StopReason, StreamEvent, StreamOptions,
    };
    use futures::StreamExt;

    let model = match selected_subagent_model(task_input, agent_def, model_id, provider.name()) {
        Ok(model) => model,
        Err(error) => {
            return ExecutionResult::failure(error);
        }
    };

    let cwd = cwd_override
        .clone()
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));

    // System prompt: prefer the agent's compiled prompt when we have a
    // definition. Without one, fall back to a minimal default that
    // tells the model it's a subagent with tools — without ANY system
    // prompt some models just ack and emit `end_turn` immediately,
    // which produced the "Task completed in 3 seconds with empty
    // output" symptom when subagent_type lookup missed.
    let system_prompt = match agent_def {
        Some(agent) => {
            let skills = crate::agents::load_skills(&cwd);
            Some(crate::agents::build_agent_system_prompt(agent, &skills))
        }
        None => Some(
            "You are a subagent dispatched to handle a specific task. You have \
             direct access to the user's filesystem and shell via tools (Bash, \
             Read, Write, Edit, Glob, Grep, etc.). Use the tools to complete the \
             task — don't just describe what you would do. When you have enough \
             information, write a thorough text summary of your findings and \
             stop. Working directory: "
                .to_owned()
                + cwd.display().to_string().as_str(),
        ),
    };

    // Tool catalogue: full list filtered by the agent's allow/disallow.
    // When there's no agent definition we still drop `Task` to avoid
    // recursive subagent spawning, but otherwise pass everything.
    let (allowed, disallowed): (&[String], &[String]) = match agent_def {
        Some(a) => (&a.allowed_tools, &a.disallowed_tools),
        None => (&[], &[]),
    };
    let tools = filter_tools_for_agent(all_tool_defs(), allowed, disallowed);

    let max_turns = agent_def
        .and_then(|a| a.max_turns)
        .unwrap_or(DEFAULT_AGENT_MAX_TURNS);

    let mut conversation = vec![ProviderMessage {
        role: ProviderRole::User,
        content: vec![ProviderContent::Text(task_input.prompt.clone())],
    }];
    let mut final_text = String::new();
    let mut last_error: Option<String> = None;
    let mut turn: u32 = 0;
    // Cumulative counters surfaced to the parent UI via TaskProgress
    // so the fan view can render "(N tools, M tokens)". Mirrors v131
    // Claude Code's `toolUseCount` / `cumulativeOutputTokens` fields.
    let mut total_tool_uses: u32 = 0;
    let started_at = std::time::Instant::now();
    let emit_progress = |tx: Option<&tokio::sync::mpsc::Sender<crate::app::AppEvent>>,
                         id: Option<&str>,
                         last_tool: Option<String>,
                         tool_use_count: Option<u32>,
                         input_tokens: Option<u64>,
                         output_tokens: Option<u64>| {
        if let (Some(tx), Some(id)) = (tx, id) {
            // TaskProgress is non-critical; the next progress update supersedes this one.
            let _ = tx.try_send(crate::app::AppEvent::TaskProgress {
                task_id: crate::ids::TaskId::from(id),
                last_tool,
                elapsed_ms: started_at.elapsed().as_millis() as u64,
                tool_use_count,
                input_tokens,
                output_tokens,
            });
        }
    };

    'outer: loop {
        turn += 1;
        if turn > max_turns {
            warn!(
                target: "jfc::tools",
                task_id = ?task_id,
                turn,
                max_turns,
                "subagent exceeded max_turns — bailing"
            );
            last_error = Some(format!(
                "Subagent exceeded max_turns ({max_turns}). Returning partial output."
            ));
            break;
        }

        let mut options = StreamOptions::new(model.clone()).tools(tools.clone());
        if let Some(sp) = &system_prompt {
            options = options.system(sp.clone());
        }

        // Two-stage context safety, matching v131 Claude Code's
        // approach. (1) When the running estimate crosses 100k tokens,
        // try an LLM-based summarization pass — that's the proper
        // mirror of cli.2.1.131's `Sp7()` auto-compaction. The
        // subagent's transcript is folded into a `<summary>` block
        // and the loop continues with the original prompt + summary +
        // most recent pair. (2) If compaction is skipped or fails,
        // fall through to a byte-budget eviction so a single oversized
        // tool result still can't blow the request past Bedrock's
        // 1M-token cap (the original 8.85M-token 400).
        let compacted = crate::stream::auto_compact_subagent_history(
            &mut conversation,
            provider,
            model.clone(),
        )
        .await;
        if compacted {
            tracing::info!(
                target: "jfc::tools",
                task_id = ?task_id,
                turn,
                "subagent transcript auto-compacted"
            );
        }
        let elided = crate::stream::cap_messages_for_budget(
            &mut conversation,
            crate::stream::SUBAGENT_HISTORY_BUDGET_BYTES,
        );
        if elided {
            tracing::info!(
                target: "jfc::tools",
                task_id = ?task_id,
                turn,
                "subagent history elided to fit byte budget"
            );
        }

        let stream = match crate::stream::open_stream_with_bedrock_retries(
            provider,
            conversation.clone(),
            &options,
        )
        .await
        {
            Ok(s) => s,
            Err(e) => return ExecutionResult::failure(format!("Subagent stream error: {e}")),
        };
        tokio::pin!(stream);

        // Per-iteration accumulators. `tool_uses` collects every
        // tool_use block the model emits this turn so we can execute
        // them in order and feed the results back on the next pass.
        let mut turn_text = String::new();
        let mut tool_uses: Vec<(String, String, String)> = Vec::new(); // (id, name, input_json)
        let mut stop_reason: Option<StopReason> = None;

        while let Some(event) = stream.next().await {
            match event {
                Ok(StreamEvent::TextDelta { delta, .. }) => {
                    // Pipe deltas through to the task panel so the user
                    // sees the subagent's prose stream live.
                    if let (Some(tx), Some(id)) = (tx, task_id) {
                        let _ = tx
                            .send(crate::app::AppEvent::AgentChunk {
                                task_id: crate::ids::TaskId::from(id),
                                text: delta.clone(),
                            })
                            .await;
                    }
                    turn_text.push_str(&delta);
                }
                Ok(StreamEvent::TextDone { text: t, .. }) => {
                    if turn_text.is_empty() {
                        turn_text = t;
                    }
                }
                Ok(StreamEvent::ToolDone {
                    tool_name,
                    tool_use_id,
                    input_json,
                    ..
                }) => {
                    tool_uses.push((tool_use_id, tool_name, input_json));
                }
                Ok(StreamEvent::Usage {
                    input_tokens,
                    output_tokens,
                    ..
                }) => {
                    // Surface this turn's input + output tokens to the
                    // parent fan UI. `latest_input_tokens` is overwritten
                    // (the live request size); `output_tokens` is folded
                    // into `cumulative_output_tokens` by the handler.
                    emit_progress(
                        tx,
                        task_id,
                        None,
                        None,
                        Some(input_tokens as u64),
                        Some(output_tokens as u64),
                    );
                }
                Ok(StreamEvent::Done { stop_reason: sr }) => {
                    stop_reason = Some(sr);
                }
                Ok(StreamEvent::Error { message }) => {
                    last_error = Some(message);
                    break 'outer;
                }
                Err(e) => {
                    last_error = Some(e.to_string());
                    break 'outer;
                }
                Ok(_) => {}
            }
        }

        // Append the assistant turn (text + tool_uses, if any) so the
        // next iteration's request reflects the running history.
        let mut assistant_content = Vec::new();
        if !turn_text.is_empty() {
            assistant_content.push(ProviderContent::Text(turn_text.clone()));
        }
        for (id, name, input_json) in &tool_uses {
            let parsed_input: serde_json::Value =
                serde_json::from_str(input_json).unwrap_or(serde_json::Value::Null);
            assistant_content.push(ProviderContent::ToolUse {
                id: id.clone(),
                name: name.clone(),
                input: parsed_input,
            });
        }
        if !assistant_content.is_empty() {
            conversation.push(ProviderMessage {
                role: ProviderRole::Assistant,
                content: assistant_content,
            });
        }

        if !turn_text.is_empty() {
            // Replace, not append — the most recent text is the one to
            // surface as the subagent's final reply when the loop ends.
            final_text = turn_text;
        }

        // No tool calls → subagent is done speaking. Don't also gate on
        // `stop_reason == EndTurn`: the OWUI/LiteLLM proxy emits
        // `Done{EndTurn}` on the final `[DONE]` SSE marker even when
        // the chunk that *finished* the turn carried tool_calls — so
        // the stop_reason we end up with is `EndTurn` despite there
        // being unexecuted tool_uses. Trusting it would cause the
        // subagent to return empty in 3–7s without ever calling Read /
        // Glob / Grep, exactly the symptom in the user's screenshot.
        if tool_uses.is_empty() {
            break;
        }
        let _ = stop_reason; // suppress "unused" — kept for future use

        // Execute every tool the subagent requested this turn, then
        // feed the results back as a single user turn (Anthropic API
        // requires all `tool_result`s to be batched in one user msg
        // immediately following the assistant turn that called them).
        let mut tool_results: Vec<ProviderContent> = Vec::new();
        for (id, name, input_json) in tool_uses {
            // Defense in depth: even though the tool list was filtered
            // upstream, re-check here in case the model hallucinated a
            // disallowed name. Provider-side filtering should already
            // make this unreachable for compliant models.
            if !disallowed.is_empty() && disallowed.iter().any(|d| d.eq_ignore_ascii_case(&name)) {
                tool_results.push(ProviderContent::ToolResult {
                    tool_use_id: id.clone(),
                    content: format!("Tool '{name}' is not allowed for this agent."),
                    is_error: true,
                });
                continue;
            }
            let kind = ToolKind::from_name(&name);
            let parsed: serde_json::Value =
                serde_json::from_str(&input_json).unwrap_or(serde_json::Value::Null);
            // If shape validation rejects the input, surface the error as a
            // tool_result so the subagent's model can retry rather than
            // executing on a silently-defaulted payload.
            let input = match ToolInput::from_value(&name, parsed) {
                Ok(input) => input,
                Err(err) => {
                    tool_results.push(ProviderContent::ToolResult {
                        tool_use_id: id.clone(),
                        content: format!("Tool input rejected: {err}"),
                        is_error: true,
                    });
                    total_tool_uses = total_tool_uses.saturating_add(1);
                    emit_progress(
                        tx,
                        task_id,
                        Some(name.clone()),
                        Some(total_tool_uses),
                        None,
                        None,
                    );
                    continue;
                }
            };
            let result = execute_tool(kind, input, cwd.clone(), None, None, None).await;
            let is_error = result.is_error();
            // Cap each tool result so a single Read on a multi-MB file
            // can't push the running conversation past Bedrock's 1M
            // limit on its own. Mirrors the parent stream loop in
            // `stream::stream_response`.
            tool_results.push(ProviderContent::ToolResult {
                tool_use_id: id.clone(),
                content: crate::stream::cap_tool_result(&result.output),
                is_error,
            });
            total_tool_uses = total_tool_uses.saturating_add(1);
            emit_progress(
                tx,
                task_id,
                Some(name.clone()),
                Some(total_tool_uses),
                None,
                None,
            );
        }
        conversation.push(ProviderMessage {
            role: ProviderRole::User,
            content: tool_results,
        });
    }

    if let Some(err) = last_error {
        if final_text.is_empty() {
            ExecutionResult::failure(err)
        } else {
            ExecutionResult::success(format!("{final_text}\n\n[note: {err}]"))
        }
    } else {
        ExecutionResult::success(final_text)
    }
}

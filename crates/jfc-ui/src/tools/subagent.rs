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
/// Claude Code has no fixed limit — agents run until end_turn or abort.
/// `None` = unlimited (matches CC behavior). Per-agent override via
/// `agent_def.max_turns` still wins when present.
const DEFAULT_AGENT_MAX_TURNS: Option<u32> = None;

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
    allow_nested_task: bool,
) -> Vec<ToolDef> {
    let allow_all = allowed.is_empty();
    all.into_iter()
        .filter(|t| {
            if !allow_nested_task && t.name.eq_ignore_ascii_case("Task") {
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
        ("haiku", "anthropic" | "anthropic-oauth") => {
            crate::providers::anthropic_models::ALIAS_HAIKU.to_string()
        }
        ("sonnet", "anthropic" | "anthropic-oauth") => {
            crate::providers::anthropic_models::ALIAS_SONNET.to_string()
        }
        ("opus", "anthropic" | "anthropic-oauth") => {
            crate::providers::anthropic_models::ALIAS_OPUS.to_string()
        }
        ("haiku", "openai") => "gpt-5-mini".to_string(),
        ("sonnet", "openai") => "gpt-5".to_string(),
        ("opus", "openai") => "gpt-5.1".to_string(),
        ("haiku", "codex") => "gpt-5.1-codex-mini".to_string(),
        ("sonnet", "codex") => "gpt-5.4".to_string(),
        ("opus", "codex") => "gpt-5.1-codex-max".to_string(),
        ("haiku", "openwebui") => "bedrock-claude-4-5-haiku".to_string(),
        ("sonnet", "openwebui") => "bedrock-claude-4-6-sonnet".to_string(),
        ("opus", "openwebui") => "bedrock-claude-4-6-opus".to_string(),
        ("haiku", "bedrock") => "anthropic.claude-haiku-4-5-20251001-v1:0".to_string(),
        ("sonnet", "bedrock") => "anthropic.claude-sonnet-4-5-20250929-v1:0".to_string(),
        ("opus", "bedrock") => "anthropic.claude-opus-4-5-20251101-v1:0".to_string(),
        ("haiku", "vertex") => "claude-haiku-4-5@20251001".to_string(),
        ("sonnet", "vertex") => "claude-sonnet-4-5@20250929".to_string(),
        ("opus", "vertex") => "claude-opus-4-5@20251101".to_string(),
        _ => model.trim().to_string(),
    }
}

/// Lazily cached agent-model config. Config is unlikely to change mid-session,
/// so we parse it once and reuse the `agents` map on every subagent spawn.
fn cached_agent_models() -> &'static std::collections::HashMap<String, crate::config::AgentConfig> {
    static CACHE: std::sync::OnceLock<
        std::collections::HashMap<String, crate::config::AgentConfig>,
    > = std::sync::OnceLock::new();
    CACHE.get_or_init(|| crate::config::load().agents)
}

pub(crate) fn selected_subagent_model(
    task_input: &crate::types::TaskInput,
    agent_def: Option<&crate::agents::AgentDef>,
    parent_model: crate::provider::ModelId,
    provider_name: &str,
) -> Result<crate::provider::ModelId, String> {
    let config_model = task_input
        .subagent_type
        .as_deref()
        .and_then(|name| cached_agent_models().get(name))
        .and_then(|a| a.model.clone())
        .filter(|s| !s.is_empty());

    let raw = std::env::var("CLAUDE_CODE_SUBAGENT_MODEL")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .or_else(|| task_input.model.clone())
        .or(config_model)
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
    use std::{
        collections::{HashMap, VecDeque},
        path::PathBuf,
        sync::{
            Mutex,
            atomic::{AtomicUsize, Ordering},
        },
    };

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
            parent_task_id: None,
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
    fn selected_subagent_model_maps_builtin_tiers_for_openai() {
        let _guard = ENV_LOCK.lock().unwrap();
        unsafe { std::env::remove_var("CLAUDE_CODE_SUBAGENT_MODEL") };

        let model = selected_subagent_model(
            &task_input(None),
            Some(&agent_model(Some("haiku"))),
            crate::provider::ModelId::new("gpt-5.5"),
            "openai",
        )
        .unwrap();

        assert_eq!(model.as_str(), "gpt-5-mini");
    }

    #[test]
    fn selected_subagent_model_maps_builtin_tiers_for_anthropic_oauth() {
        let _guard = ENV_LOCK.lock().unwrap();
        unsafe { std::env::remove_var("CLAUDE_CODE_SUBAGENT_MODEL") };

        let model = selected_subagent_model(
            &task_input(Some("haiku")),
            None,
            crate::provider::ModelId::new("claude-opus-4-7"),
            "anthropic-oauth",
        )
        .unwrap();

        assert_eq!(
            model.as_str(),
            crate::providers::anthropic_models::ALIAS_HAIKU
        );
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

    struct ScriptedProvider {
        scripts: Mutex<VecDeque<Vec<crate::provider::StreamEvent>>>,
        calls: AtomicUsize,
    }

    impl ScriptedProvider {
        fn new(scripts: Vec<Vec<crate::provider::StreamEvent>>) -> Self {
            Self {
                scripts: Mutex::new(scripts.into()),
                calls: AtomicUsize::new(0),
            }
        }
    }

    #[async_trait::async_trait]
    impl crate::provider::Provider for ScriptedProvider {
        fn name(&self) -> &str {
            "anthropic"
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
            self.calls.fetch_add(1, Ordering::SeqCst);
            let events = self
                .scripts
                .lock()
                .unwrap()
                .pop_front()
                .ok_or_else(|| anyhow::anyhow!("scripts exhausted"))?;
            Ok(Box::pin(stream::iter(events.into_iter().map(Ok))))
        }
    }

    impl crate::provider::seal::Sealed for ScriptedProvider {}

    #[tokio::test(flavor = "current_thread")]
    async fn execute_task_retries_retryable_stream_error_normal() {
        let provider = ScriptedProvider::new(vec![
            vec![crate::provider::StreamEvent::Error {
                message: format!(
                    "{}Anthropic transient API error 529: overloaded",
                    crate::providers::anthropic::AUTO_RETRY_SENTINEL
                ),
            }],
            vec![
                crate::provider::StreamEvent::TextDelta {
                    index: 0,
                    delta: "recovered".into(),
                },
                crate::provider::StreamEvent::Done {
                    stop_reason: crate::provider::StopReason::EndTurn,
                },
            ],
        ]);

        let result = execute_task(
            &task_input(None),
            &provider,
            crate::provider::ModelId::new("claude-opus-4-7"),
            None,
            None,
            Some(&agent_model(None)),
            None,
            None,
            None,
        )
        .await;

        assert!(
            !result.is_error(),
            "subagent should recover: {}",
            result.output
        );
        assert_eq!(result.output, "recovered");
        assert_eq!(provider.calls.load(Ordering::SeqCst), 2);
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
    tx: Option<&tokio::sync::mpsc::Sender<crate::runtime::AppEvent>>,
    task_id: Option<&str>,
    agent_def: Option<&crate::agents::AgentDef>,
    cwd_override: Option<PathBuf>,
    task_store: Option<std::sync::Arc<crate::tasks::TaskStore>>,
    active_team_name: Option<&str>,
) -> ExecutionResult {
    execute_task_inner(
        task_input,
        provider,
        model_id,
        tx,
        task_id,
        agent_def,
        cwd_override,
        task_store,
        active_team_name.map(str::to_owned),
        0,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
async fn execute_task_inner(
    task_input: &crate::types::TaskInput,
    provider: &dyn crate::provider::Provider,
    model_id: crate::provider::ModelId,
    tx: Option<&tokio::sync::mpsc::Sender<crate::runtime::AppEvent>>,
    task_id: Option<&str>,
    agent_def: Option<&crate::agents::AgentDef>,
    cwd_override: Option<PathBuf>,
    task_store: Option<std::sync::Arc<crate::tasks::TaskStore>>,
    active_team_name: Option<String>,
    depth: u8,
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
    let allow_nested_task = depth < 2;
    let tools = filter_tools_for_agent(all_tool_defs(), allowed, disallowed, allow_nested_task);

    let max_turns: Option<u32> = agent_def
        .and_then(|a| a.max_turns)
        .or(DEFAULT_AGENT_MAX_TURNS);

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
    let emit_progress = |tx: Option<&tokio::sync::mpsc::Sender<crate::runtime::AppEvent>>,
                         id: Option<&str>,
                         last_tool: Option<String>,
                         tool_use_count: Option<u32>,
                         input_tokens: Option<u64>,
                         cache_read_tokens: Option<u64>,
                         cache_write_tokens: Option<u64>,
                         output_tokens: Option<u64>| {
        if let (Some(tx), Some(id)) = (tx, id) {
            // TaskProgress is non-critical; the next progress update supersedes this one.
            let _ = tx.try_send(crate::runtime::AppEvent::Task(
                crate::runtime::TaskEvent::Progress {
                    task_id: crate::ids::TaskId::from(id),
                    last_tool,
                    elapsed_ms: started_at.elapsed().as_millis() as u64,
                    tool_use_count,
                    input_tokens,
                    cache_read_tokens,
                    cache_write_tokens,
                    output_tokens,
                },
            ));
        }
    };

    'outer: loop {
        if task_id
            .map(crate::daemon::background_agent_cancel_requested)
            .unwrap_or(false)
        {
            return ExecutionResult::failure("cancelled: background agent cancellation requested");
        }
        turn += 1;
        if let Some(cap) = max_turns {
            if turn > cap {
                warn!(
                    target: "jfc::tools",
                    task_id = ?task_id,
                    turn,
                    max_turns = cap,
                    "subagent exceeded max_turns — bailing"
                );
                last_error = Some(format!(
                    "Subagent exceeded max_turns ({cap}). Returning partial output."
                ));
                break;
            }
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

        // Per-iteration accumulators. `tool_uses` collects every
        // tool_use block the model emits this turn so we can execute
        // them in order and feed the results back on the next pass.
        let mut stream_retry_attempt = 0u32;
        let (turn_text, tool_uses, stop_reason) = loop {
            let stream = match crate::stream::open_stream_with_bedrock_retries(
                provider,
                std::sync::Arc::new(conversation.clone()),
                &options,
            )
            .await
            {
                Ok(s) => s,
                Err(e) => {
                    let message = e.to_string();
                    if let Some(retry) = crate::providers::retry::retryable_stream_error(&message) {
                        let delay =
                            crate::providers::retry::stream_retry_delay(stream_retry_attempt);
                        tracing::warn!(
                            target: "jfc::tools::subagent",
                            task_id = ?task_id,
                            turn,
                            retry_attempt = stream_retry_attempt + 1,
                            provider = retry.provider,
                            delay_ms = delay.as_millis() as u64,
                            error = %retry.message,
                            "subagent stream open hit retryable provider error"
                        );
                        stream_retry_attempt = stream_retry_attempt.saturating_add(1);
                        tokio::time::sleep(delay).await;
                        if task_id
                            .map(crate::daemon::background_agent_cancel_requested)
                            .unwrap_or(false)
                        {
                            return ExecutionResult::failure(
                                "cancelled: background agent cancellation requested",
                            );
                        }
                        continue;
                    }
                    return ExecutionResult::failure(format!("Subagent stream error: {e}"));
                }
            };
            tokio::pin!(stream);

            let mut turn_text = String::new();
            let mut tool_uses: Vec<(String, String, String)> = Vec::new(); // (id, name, input_json)
            let mut stop_reason: Option<StopReason> = None;
            let mut usage_baseline = (0u32, 0u32, 0u32, 0u32);
            let mut reported_input_for_turn = false;
            let mut retryable_stream_error: Option<String> = None;

            while let Some(event) = stream.next().await {
                if task_id
                    .map(crate::daemon::background_agent_cancel_requested)
                    .unwrap_or(false)
                {
                    return ExecutionResult::failure(
                        "cancelled: background agent cancellation requested",
                    );
                }
                match event {
                    Ok(StreamEvent::TextDelta { delta, .. }) => {
                        // Pipe deltas through to the task panel so the user
                        // sees the subagent's prose stream live.
                        if let (Some(tx), Some(id)) = (tx, task_id) {
                            let _ = tx
                                .send(crate::runtime::AppEvent::Task(
                                    crate::runtime::TaskEvent::AgentChunk {
                                        task_id: crate::ids::TaskId::from(id),
                                        text: delta.clone(),
                                    },
                                ))
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
                        cache_read_tokens,
                        cache_write_tokens,
                    }) => {
                        let output_delta = output_tokens.saturating_sub(usage_baseline.1);
                        usage_baseline = (
                            input_tokens,
                            output_tokens,
                            cache_read_tokens,
                            cache_write_tokens,
                        );
                        // Surface this turn's input + output tokens to the
                        // parent fan UI. Input/cache are sent once per API
                        // round-trip so the session cost ledger can add the
                        // request once; output remains a streaming delta.
                        let input_update = if reported_input_for_turn {
                            None
                        } else {
                            reported_input_for_turn = true;
                            Some((
                                input_tokens as u64,
                                cache_read_tokens as u64,
                                cache_write_tokens as u64,
                            ))
                        };
                        emit_progress(
                            tx,
                            task_id,
                            None,
                            None,
                            input_update.map(|(input, _, _)| input),
                            input_update.map(|(_, cache_read, _)| cache_read),
                            input_update.map(|(_, _, cache_write)| cache_write),
                            Some(output_delta as u64),
                        );
                    }
                    Ok(StreamEvent::Done { stop_reason: sr }) => {
                        stop_reason = Some(sr);
                    }
                    Ok(StreamEvent::Error { message }) => {
                        if crate::providers::retry::retryable_stream_error(&message).is_some() {
                            retryable_stream_error = Some(message);
                            break;
                        }
                        last_error = Some(message);
                        break 'outer;
                    }
                    Err(e) => {
                        let message = e.to_string();
                        if crate::providers::retry::retryable_stream_error(&message).is_some() {
                            retryable_stream_error = Some(message);
                            break;
                        }
                        last_error = Some(message);
                        break 'outer;
                    }
                    Ok(_) => {}
                }
            }

            if let Some(message) = retryable_stream_error {
                let Some(retry) = crate::providers::retry::retryable_stream_error(&message) else {
                    unreachable!("message was classified above");
                };
                let delay = crate::providers::retry::stream_retry_delay(stream_retry_attempt);
                tracing::warn!(
                    target: "jfc::tools::subagent",
                    task_id = ?task_id,
                    turn,
                    retry_attempt = stream_retry_attempt + 1,
                    provider = retry.provider,
                    delay_ms = delay.as_millis() as u64,
                    error = %retry.message,
                    "subagent stream event hit retryable provider error"
                );
                stream_retry_attempt = stream_retry_attempt.saturating_add(1);
                tokio::time::sleep(delay).await;
                if task_id
                    .map(crate::daemon::background_agent_cancel_requested)
                    .unwrap_or(false)
                {
                    return ExecutionResult::failure(
                        "cancelled: background agent cancellation requested",
                    );
                }
                continue;
            }

            break (turn_text, tool_uses, stop_reason);
        };

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
                        None,
                        None,
                    );
                    continue;
                }
            };
            let result = if let ToolInput::Task(nested_task) = &input {
                if depth >= 2 {
                    ExecutionResult::failure(
                        "Nested Task depth limit reached. Summarize current work instead of spawning another subagent.",
                    )
                } else {
                    let agents = crate::agents::load_agents(&cwd);
                    let nested_agent = nested_task
                        .subagent_type
                        .as_deref()
                        .and_then(|t| agents.iter().find(|a| a.name.eq_ignore_ascii_case(t)));
                    Box::pin(execute_task_inner(
                        nested_task,
                        provider,
                        model.clone(),
                        None,
                        None,
                        nested_agent,
                        Some(cwd.clone()),
                        task_store.clone(),
                        active_team_name.clone(),
                        depth + 1,
                    ))
                    .await
                }
            } else {
                execute_tool(
                    kind,
                    input,
                    cwd.clone(),
                    None,
                    task_store.clone(),
                    active_team_name.as_deref(),
                )
                .await
            };
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
    } else if final_text.trim().is_empty() {
        // No tool error, but also no text output. The subagent exited
        // its inner loop without ever producing a final reply. This
        // happens when:
        //   - the provider returned stop_reason=EndTurn with zero
        //     content blocks (transient gateway hiccup)
        //   - every assistant turn was tool-only and the last tool
        //     batch produced no follow-up text before EndTurn fired
        //   - the subagent was prompted to be silent and complied
        //     literally (rare but possible)
        //
        // Returning success("") makes the parent's auto-continuation
        // proceed with a blank tool_result — the parent model then has
        // to fabricate context, which is exactly the hallucination
        // failure mode we want to avoid. Surface this as a structured
        // failure instead so the parent sees "subagent produced no
        // output" and either reissues the Task with a clearer prompt
        // or asks the user for clarification.
        tracing::warn!(
            target: "jfc::tools::subagent",
            "subagent completed with empty final_text and no error — flagging as failure"
        );
        ExecutionResult::failure(
            "Subagent finished without producing any text output. \
             This usually means the inner loop ended on a tool batch \
             with no follow-up reply. Try reissuing the Task with a \
             clearer prompt that requests a final summary, or ask the \
             user to clarify what they want the subagent to report \
             back."
                .to_owned(),
        )
    } else {
        ExecutionResult::success(final_text)
    }
}

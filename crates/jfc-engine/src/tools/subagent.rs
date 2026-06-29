use std::path::{Path, PathBuf};

use tracing::warn;

use super::{ExecutionResult, all_tool_defs_with_mcp, execute_tool};
use crate::types::{ToolInput, ToolKind};
use jfc_provider::ToolDef;

pub async fn execute_skill_in(cwd: &Path, name: &str, args: Option<&str>) -> ExecutionResult {
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
            if !skill.is_user_invocable() {
                return ExecutionResult::failure(format!(
                    "Skill `{}` is not user-invocable.",
                    skill.name
                ));
            }
            // Best-effort usage telemetry (the curator's foundation). Never
            // affects invocation — errors are logged inside record_skill_use.
            jfc_learn::record_skill_use(cwd, &skill.name).await;
            let memory_root = jfc_memory::project_memory_dir(cwd);
            let context = crate::agents::SkillRenderContext::new(Some(cwd), Some(&memory_root));
            let body = crate::agents::render_skill_invocation(skill, context, args);
            ExecutionResult::success(body)
        }
        None => {
            // Surface the available skills in the error so the model
            // can self-correct without having to ask the user, but keep the
            // list short and omit internal/superpower skills. Dumping every
            // global skill back into the chat caused OpenWebUI-routed models
            // to chase unrelated "superpowers:*" names instead of recovering.
            const MAX_UNKNOWN_SKILL_SUGGESTIONS: usize = 20;
            let mut available: Vec<&str> = skills
                .iter()
                .filter(|skill| skill.is_discoverable())
                .map(|s| s.name.as_str())
                .collect();
            available.truncate(MAX_UNKNOWN_SKILL_SUGGESTIONS);
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
#[derive(Debug, Clone, PartialEq, Eq)]
enum ToolAllowScope {
    All,
    Only(Vec<String>),
}

#[cfg(test)]
pub fn filter_tools_for_agent(
    all: Vec<ToolDef>,
    allowed: &[String],
    disallowed: &[String],
    allow_nested_task: bool,
) -> Vec<ToolDef> {
    let scope = if allowed.is_empty() {
        ToolAllowScope::All
    } else {
        ToolAllowScope::Only(allowed.to_vec())
    };
    filter_tools_for_agent_scope(all, &scope, disallowed, allow_nested_task)
}

fn filter_tools_for_agent_scope(
    all: Vec<ToolDef>,
    allowed: &ToolAllowScope,
    disallowed: &[String],
    allow_nested_task: bool,
) -> Vec<ToolDef> {
    all.into_iter()
        .filter(|t| {
            if !allow_nested_task && t.name.eq_ignore_ascii_case("Task") {
                return false;
            }
            match allowed {
                ToolAllowScope::All => {}
                ToolAllowScope::Only(allowed) => {
                    if !allowed.iter().any(|a| tool_policy_matches(a, &t.name)) {
                        return false;
                    }
                }
            }
            !disallowed.iter().any(|d| tool_policy_matches(d, &t.name))
        })
        .collect()
}

fn tool_policy_matches(policy_name: &str, tool_name: &str) -> bool {
    if policy_name.eq_ignore_ascii_case(tool_name) {
        return true;
    }
    if !super::is_code_navigation_tool_name(policy_name)
        || !super::is_code_navigation_tool_name(tool_name)
    {
        return false;
    }
    code_navigation_leaf(policy_name).eq_ignore_ascii_case(code_navigation_leaf(tool_name))
}

fn code_navigation_leaf(name: &str) -> &str {
    let trimmed = name.trim();
    trimmed.rsplit("__").next().unwrap_or(trimmed)
}

fn scoped_allowed_tools(agent_allowed: &[String], task_allowed: &[String]) -> ToolAllowScope {
    match (agent_allowed.is_empty(), task_allowed.is_empty()) {
        (true, true) => ToolAllowScope::All,
        (false, true) => ToolAllowScope::Only(agent_allowed.to_vec()),
        (true, false) => ToolAllowScope::Only(task_allowed.to_vec()),
        (false, false) => ToolAllowScope::Only(
            agent_allowed
                .iter()
                .filter(|agent_tool| {
                    task_allowed.iter().any(|task_tool| {
                        tool_policy_matches(task_tool, agent_tool)
                            || tool_policy_matches(agent_tool, task_tool)
                    })
                })
                .cloned()
                .collect(),
        ),
    }
}

fn scoped_disallowed_tools(agent_disallowed: &[String], task_disallowed: &[String]) -> Vec<String> {
    let mut disallowed = agent_disallowed.to_vec();
    for tool in task_disallowed {
        if !disallowed
            .iter()
            .any(|existing| existing.eq_ignore_ascii_case(tool))
        {
            disallowed.push(tool.clone());
        }
    }
    disallowed
}

/// The permission policy a subagent's tool calls are gated by (CS-JFC-005).
///
/// Subagents (including detached background workers) run tools without the
/// interactive approval gate. We map the agent's declared `permissionMode` onto
/// the engine permission policy and apply it per tool call. When an agent does
/// not declare a mode it defaults to `BypassPermissions` — preserving existing
/// autonomous-subagent behavior — but the catastrophic-bash backstop and any
/// explicitly-declared restrictive mode are now honored instead of ignored.
fn delegated_permission_mode(
    agent_def: Option<&crate::agents::AgentDef>,
) -> crate::app::PermissionMode {
    use crate::app::PermissionMode as Engine;
    use jfc_core::PermissionMode as Core;
    match agent_def.and_then(|a| a.permission_mode) {
        Some(Core::Default) => Engine::Default,
        Some(Core::AcceptEdits) => Engine::AcceptEdits,
        Some(Core::Plan) => Engine::Plan,
        Some(Core::Auto) => Engine::Auto,
        // `DontAsk` is an explicit "don't prompt" → bypass. No declared mode
        // keeps the historical autonomous default.
        Some(Core::BypassPermissions) | Some(Core::DontAsk) | None => Engine::BypassPermissions,
    }
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

/// Map a Task `category` (advertised on the Task tool as "Task category for
/// model selection") to a model tier alias (`haiku`/`sonnet`/`opus`). This gives
/// subagents a sensible cost-appropriate default — cheap models for read-only
/// mapping/exploration, the heavy model for hard reasoning — when no explicit
/// model is set. Returns `None` for unknown categories (fall back to parent).
///
/// The tiers are deliberately conservative: only clearly-cheap categories get
/// `haiku`, only clearly-hard ones get `opus`, everything recognized else maps
/// to `sonnet`. Unknown categories inherit the parent model unchanged.
fn category_to_tier(category: &str) -> Option<&'static str> {
    match category.trim().to_ascii_lowercase().as_str() {
        // Cheap, read-only / mechanical work → smallest model.
        "explore" | "exploration" | "search" | "mapping" | "map" | "read" | "readonly"
        | "lookup" | "summarize" | "summarisation" | "summarization" | "classify"
        | "classification" | "lint" | "format" | "trivial" | "cheap" | "fast" => Some("haiku"),
        // Hard reasoning / architecture / security → largest model.
        "architecture" | "design" | "plan" | "planning" | "reasoning" | "hard" | "complex"
        | "security" | "audit" | "review" | "debug" | "refactor" | "heavy" => Some("opus"),
        // Standard implementation / general work → balanced model.
        "build" | "implement" | "implementation" | "code" | "coding" | "edit" | "test"
        | "testing" | "fix" | "general" | "balanced" => Some("sonnet"),
        _ => None,
    }
}

/// Infer a model tier from the *text* of a subagent's task when no explicit
/// model/category was given — a lightweight, deterministic complexity router in
/// the spirit of LLM cascades (FrugalGPT / RouteLLM): cheap signals decide
/// whether the work is mechanical (→ `haiku`), hard reasoning (→ `opus`), or
/// ordinary (→ `sonnet`).
///
/// Strictly a *last* resort: it sits below `category_to_tier` in the cascade,
/// so an explicit category, config, agent-def, or `model` always wins. It only
/// upgrades/downgrades the otherwise-inherited (often heavy) parent model for a
/// bare `Task { prompt }` with no other hint. Conservative by design — it
/// returns `None` (inherit parent) unless the signal is clear, so it can never
/// route hard work to a weak model on a weak signal.
fn prompt_complexity_tier(prompt: &str, description: &str) -> Option<&'static str> {
    let text = format!("{description}\n{prompt}").to_ascii_lowercase();
    if text.trim().is_empty() {
        return None;
    }

    // Hard-reasoning vocabulary → upgrade. These verbs/nouns reliably mark work
    // that benefits from the strongest model.
    const HARD: &[&str] = &[
        "architect",
        "design ",
        "prove",
        "security",
        "vulnerab",
        "exploit",
        "race condition",
        "deadlock",
        "refactor",
        "redesign",
        "root cause",
        "debug ",
        "why does",
        "trade-off",
        "tradeoff",
        "algorithm",
        "optimi", // optimise/optimize/optimization
        "concurren",
        "unsafe",
        "invariant",
    ];
    // Mechanical / read-only vocabulary → downgrade.
    const CHEAP: &[&str] = &[
        "list ",
        "find ",
        "grep",
        "where is",
        "locate",
        "rename",
        "typo",
        "format ",
        "lint",
        "count ",
        "summari", // summarise/summarize
        "read ",
        "look up",
        "lookup",
        "extract ",
        "enumerate",
        "what files",
    ];

    let hard_hits = HARD.iter().filter(|kw| text.contains(**kw)).count();
    let cheap_hits = CHEAP.iter().filter(|kw| text.contains(**kw)).count();

    // A long, multi-part prompt is itself weak evidence of complexity; a very
    // short one is weak evidence of a mechanical lookup. Use these only to break
    // a tie, never to override explicit vocabulary.
    let long = text.len() > 600;
    let short = text.len() < 80;

    match hard_hits.cmp(&cheap_hits) {
        std::cmp::Ordering::Greater => Some("opus"),
        std::cmp::Ordering::Less => Some("haiku"),
        std::cmp::Ordering::Equal => {
            if hard_hits > 0 {
                // Equal but non-zero hits on both → ordinary implementation.
                Some("sonnet")
            } else if long {
                Some("sonnet")
            } else if short {
                Some("haiku")
            } else {
                None
            }
        }
    }
}

/// Lazily cached agent-model config. Config is unlikely to change mid-session,
/// so we parse it once and reuse the `agents` map on every subagent spawn.
fn cached_agent_models() -> &'static std::collections::HashMap<String, crate::config::AgentConfig> {
    static CACHE: std::sync::OnceLock<
        std::collections::HashMap<String, crate::config::AgentConfig>,
    > = std::sync::OnceLock::new();
    CACHE.get_or_init(|| crate::config::load_arc().agents.clone())
}

fn selected_subagent_model_request(
    task_input: &crate::types::TaskInput,
    agent_def: Option<&crate::agents::AgentDef>,
) -> Option<String> {
    let config_model = task_input
        .subagent_type
        .as_deref()
        .and_then(|name| cached_agent_models().get(name))
        .and_then(|a| a.model.clone());

    selected_subagent_model_request_from_sources(task_input, agent_def, config_model)
}

fn selected_subagent_model_request_from_sources(
    task_input: &crate::types::TaskInput,
    agent_def: Option<&crate::agents::AgentDef>,
    config_model: Option<String>,
) -> Option<String> {
    let category_tier = task_input
        .category
        .as_deref()
        .and_then(category_to_tier)
        .map(str::to_string);

    let complexity_tier = if task_input.category.is_none() {
        prompt_complexity_tier(&task_input.prompt, &task_input.description).map(str::to_string)
    } else {
        None
    };

    std::env::var("CLAUDE_CODE_SUBAGENT_MODEL")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .or_else(|| task_input.model.clone())
        .or_else(|| config_model.filter(|s| !s.is_empty()))
        .or_else(|| agent_def.and_then(|a| a.model.clone()))
        .or(category_tier)
        .or(complexity_tier)
}

pub fn selected_subagent_model(
    task_input: &crate::types::TaskInput,
    agent_def: Option<&crate::agents::AgentDef>,
    parent_model: jfc_provider::ModelId,
    provider_name: &str,
) -> Result<jfc_provider::ModelId, String> {
    let raw = selected_subagent_model_request(task_input, agent_def);
    let Some(raw) = raw else {
        return Ok(parent_model);
    };

    if raw.eq_ignore_ascii_case("inherit") || raw.eq_ignore_ascii_case("parent") {
        return Ok(parent_model);
    }

    let aliased = subagent_model_alias(&raw, provider_name);
    let spec = jfc_provider::ModelSpec::parse_lenient(&aliased)
        .map_err(|e| format!("invalid subagent model {raw:?}: {e}"))?;

    if let Some(prefix) = spec.provider()
        && !crate::runtime::bootstrap::provider_name_matches_request(provider_name, prefix.as_str())
    {
        return Err(format!(
            "subagent model {aliased:?} targets provider {prefix}, but the active provider is {provider_name}; provider switching for subagents is not wired yet"
        ));
    }

    Ok(spec.into_model())
}

/// Like [`selected_subagent_model`], but resolves provider-qualified specs
/// (`openai/gpt-5.2`, `anthropic/haiku`) against the full provider registry
/// instead of rejecting them. Returns the (possibly switched) provider along
/// with the model. The council already routes this way
/// (`runtime::bootstrap::resolve_provider_model`); subagents previously
/// hard-errored, so `Task(model: "openai/…")` failed under an Anthropic
/// session even when an OpenAI provider was configured.
pub fn selected_subagent_provider_model(
    task_input: &crate::types::TaskInput,
    agent_def: Option<&crate::agents::AgentDef>,
    parent_provider: std::sync::Arc<dyn jfc_provider::Provider>,
    parent_model: jfc_provider::ModelId,
    registry: &[std::sync::Arc<dyn jfc_provider::Provider>],
) -> Result<
    (
        std::sync::Arc<dyn jfc_provider::Provider>,
        jfc_provider::ModelId,
    ),
    String,
> {
    match selected_subagent_model(task_input, agent_def, parent_model, parent_provider.name()) {
        Ok(model) => Ok((parent_provider, model)),
        Err(same_provider_error) => {
            // Cross-provider spec: re-derive the raw request and route it
            // through the registry. Resolution failure reports the original
            // error so the message still names the missing provider.
            let raw = selected_subagent_model_request(task_input, agent_def);
            if let Some(raw) = raw
                && let Some(resolution) =
                    crate::runtime::bootstrap::resolve_provider_model(registry, &raw)
            {
                tracing::info!(
                    target: "jfc::tools",
                    requested = %raw,
                    provider = %resolution.provider.name(),
                    model = %resolution.model.as_str(),
                    "subagent model routed to a different provider"
                );
                return Ok((resolution.provider, resolution.model));
            }
            Err(same_provider_error)
        }
    }
}

fn tool_info_from_raw_json(name: &str, input_json: &str) -> Option<String> {
    let value = serde_json::from_str(input_json).ok()?;
    let input = ToolInput::from_value(name, value).ok()?;
    Some(tool_progress_info(name, &input))
}

fn tool_progress_info(name: &str, input: &ToolInput) -> String {
    let summary = input.summary();
    let summary = summary.lines().next().unwrap_or_default().trim();
    if summary.is_empty() {
        return name.to_owned();
    }
    format!("{name}({})", truncate_tool_progress_summary(summary))
}

fn truncate_tool_progress_summary(summary: &str) -> String {
    const MAX_CHARS: usize = 80;
    let mut chars = summary.chars();
    let mut out = chars.by_ref().take(MAX_CHARS).collect::<String>();
    if chars.next().is_some() {
        out.push_str("...");
    }
    out
}

fn append_code_navigation_guidance(system_prompt: &mut Option<String>) {
    const GUIDANCE: &str = "\n\n# Source Code Navigation\n\
        When a visible CodeGraph tool fits the task, use it before broad Read \
        or Grep. Use the exact visible tool name and schema; installs may expose \
        names like `mcp__codegraph__codegraph_explore` instead of \
        `codegraph_explore`.";
    match system_prompt {
        Some(prompt) => prompt.push_str(GUIDANCE),
        None => *system_prompt = Some(GUIDANCE.trim_start().to_owned()),
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
    provider: &dyn jfc_provider::Provider,
    model_id: jfc_provider::ModelId,
    tx: Option<&tokio::sync::mpsc::Sender<crate::runtime::EngineEvent>>,
    task_id: Option<&str>,
    agent_def: Option<&crate::agents::AgentDef>,
    cwd_override: Option<PathBuf>,
    task_store: Option<std::sync::Arc<jfc_session::TaskStore>>,
    active_team_name: Option<&str>,
) -> ExecutionResult {
    let project_root = cwd_override
        .as_deref()
        .unwrap_or_else(|| std::path::Path::new("."));
    let launch_plan = match crate::agents::select_task_agent_launch_plan(task_input, project_root) {
        Ok(plan) => plan,
        Err(error) => {
            return ExecutionResult::failure(format!(
                "Task agent launch descriptor unavailable: {error}"
            ));
        }
    };
    match launch_plan.backend {
        crate::agents::AgentLaunchBackend::InProcess => {}
        crate::agents::AgentLaunchBackend::BackgroundWorker => {
            return ExecutionResult::failure(format!(
                "Task launcher {} resolved to a background-worker backend for foreground execution",
                launch_plan.descriptor.name
            ));
        }
        crate::agents::AgentLaunchBackend::ProcessBridge { ref command } => {
            return crate::agents::execute_process_bridge_agent_launch(
                crate::agents::ProcessBridgeAgentLaunchInvocation {
                    descriptor: &launch_plan.descriptor,
                    command,
                    task_input,
                    task_id,
                    cwd: cwd_override.as_deref(),
                    model_id: Some(&model_id),
                    provider_name: Some(provider.name()),
                    active_team_name,
                },
            )
            .await;
        }
    }
    tracing::debug!(
        target: "jfc::tools::subagent",
        launcher = %launch_plan.descriptor.name,
        handler = %launch_plan.descriptor.executor.handler,
        "selected descriptor-owned agent launch backend"
    );

    // StructuredOutput schema: when the parent provides a schema, install it
    // as a task-local for the subagent's whole run so its StructuredOutput
    // tool call validates against it (work-stealing-safe; see
    // structured_output::with_schema).
    let validator = match task_input.schema {
        Some(ref schema) => match crate::tools::structured_output::compile_schema(schema) {
            Ok(v) => Some(v),
            Err(e) => {
                return ExecutionResult::failure(format!("Task: invalid schema rejected: {e}"));
            }
        },
        None => None,
    };
    crate::tools::structured_output::with_schema(
        validator,
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
        ),
    )
    .await
}
async fn execute_task_inner(
    task_input: &crate::types::TaskInput,
    provider: &dyn jfc_provider::Provider,
    model_id: jfc_provider::ModelId,
    tx: Option<&tokio::sync::mpsc::Sender<crate::runtime::EngineEvent>>,
    task_id: Option<&str>,
    agent_def: Option<&crate::agents::AgentDef>,
    cwd_override: Option<PathBuf>,
    task_store: Option<std::sync::Arc<jfc_session::TaskStore>>,
    active_team_name: Option<String>,
    depth: u8,
) -> ExecutionResult {
    use jfc_provider::{ProviderContent, ProviderMessage, ProviderRole, StreamOptions};

    let provider_registry = crate::tools::snapshot_provider_registry();
    let resolved_provider_model = provider_registry
        .iter()
        .find(|candidate| candidate.name() == provider.name())
        .cloned()
        .map(|parent_provider| {
            selected_subagent_provider_model(
                task_input,
                agent_def,
                parent_provider,
                model_id.clone(),
                &provider_registry,
            )
        });
    let (provider_override, model) = match resolved_provider_model {
        Some(Ok((resolved_provider, model))) => (Some(resolved_provider), model),
        Some(Err(error)) => {
            return ExecutionResult::failure(error);
        }
        None => match selected_subagent_model(task_input, agent_def, model_id, provider.name()) {
            Ok(model) => (None, model),
            Err(error) => {
                return ExecutionResult::failure(error);
            }
        },
    };
    let provider = provider_override.as_deref().unwrap_or(provider);

    let cwd = cwd_override
        .clone()
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));

    // System prompt: prefer the agent's compiled prompt when we have a
    // definition. Without one, fall back to a minimal default that
    // tells the model it's a subagent with tools — without ANY system
    // prompt some models just ack and emit `end_turn` immediately,
    // which produced the "Task completed in 3 seconds with empty
    // output" symptom when subagent_type lookup missed.
    let mut system_prompt = match agent_def {
        Some(agent) => {
            let skills = crate::agents::load_skills(&cwd);
            let memory_root = jfc_memory::project_memory_dir(&cwd);
            Some(crate::agents::build_agent_system_prompt_with_context(
                agent,
                &skills,
                crate::agents::SkillRenderContext::new(Some(&cwd), Some(&memory_root)),
            ))
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
    if let Some(schema) = &task_input.schema {
        let schema_text =
            serde_json::to_string_pretty(schema).unwrap_or_else(|_| schema.to_string());
        let instruction = format!(
            "\n\n# Structured Output\n\
             This task requires structured output. You must call the \
             StructuredOutput tool with a JSON object that validates against \
             this schema. Raw JSON text or prose is not accepted as the final \
             answer for this task.\n\n{schema_text}"
        );
        match &mut system_prompt {
            Some(prompt) => prompt.push_str(&instruction),
            None => system_prompt = Some(instruction),
        }
    }

    // Subagent context inheritance: when the parent seeded
    // `forks_parent_context` (set by `build_parent_context_seed` in
    // tool_dispatch.rs when `subagent_context_inheritance` is on),
    // inject it as a `<parent_context>` block into the system prompt.
    if let Some(Some(seed)) = agent_def.map(|a| a.forks_parent_context.as_ref()) {
        match &mut system_prompt {
            Some(prompt) => inject_parent_context(prompt, seed),
            None => {
                let mut s = String::new();
                inject_parent_context(&mut s, seed);
                system_prompt = Some(s);
            }
        }
    }

    let (agent_allowed, agent_disallowed): (&[String], &[String]) = match agent_def {
        Some(a) => (&a.allowed_tools, &a.disallowed_tools),
        None => (&[], &[]),
    };
    let allowed = scoped_allowed_tools(agent_allowed, &task_input.allowed_tools);
    let disallowed = scoped_disallowed_tools(agent_disallowed, &task_input.disallowed_tools);
    // CS-JFC-005: the permission policy gating this subagent's tool calls.
    let permission_mode = delegated_permission_mode(agent_def);
    let schema_required = task_input.schema.is_some();
    if schema_required
        && disallowed
            .iter()
            .any(|d| d.eq_ignore_ascii_case("StructuredOutput"))
    {
        return ExecutionResult::failure(
            "Task schema requires StructuredOutput, but the agent disallows that tool.",
        );
    }
    let allow_nested_task = depth < 2;
    let all_tools = all_tool_defs_with_mcp().await;
    let structured_output_def = all_tools
        .iter()
        .find(|tool| tool.name.eq_ignore_ascii_case("StructuredOutput"))
        .cloned();
    let mut tools =
        filter_tools_for_agent_scope(all_tools, &allowed, &disallowed, allow_nested_task);
    if schema_required {
        if !tools
            .iter()
            .any(|tool| tool.name.eq_ignore_ascii_case("StructuredOutput"))
        {
            let Some(def) = structured_output_def else {
                return ExecutionResult::failure("StructuredOutput tool is not available.");
            };
            tools.push(def);
        }
    } else {
        tools.retain(|tool| !tool.name.eq_ignore_ascii_case("StructuredOutput"));
    }
    if tools
        .iter()
        .any(|tool| super::is_code_navigation_tool_name(&tool.name))
    {
        append_code_navigation_guidance(&mut system_prompt);
    }

    let max_turns: Option<u32> = agent_def
        .and_then(|a| a.max_turns)
        .or(DEFAULT_AGENT_MAX_TURNS);

    let mut conversation = vec![ProviderMessage {
        role: ProviderRole::User,
        content: vec![ProviderContent::Text(task_input.prompt.clone())],
    }];
    let mut final_text = String::new();
    // Full narrative across turns. `final_text` holds only the LAST turn's text,
    // which loses earlier substantive output when the agent's final turn is a
    // short preamble before a tool call (e.g. "Let me write the report:") and
    // the loop then ends — the exact "lost report" failure. We keep every
    // non-trivial turn so the harvested result can fall back to the full
    // narrative instead of a dangling preamble.
    let mut narrative: Vec<String> = Vec::new();
    let mut last_error: Option<String> = None;
    let mut structured_output: Option<serde_json::Value> = None;
    let mut turn: u32 = 0;
    // Cumulative counters surfaced to the parent UI via TaskProgress
    // so the fan view can render "(N tools, M tokens)". Mirrors v131
    // Claude Code's `toolUseCount` / `cumulativeOutputTokens` fields.
    let mut total_tool_uses: u32 = 0;
    let started_at = std::time::Instant::now();
    let emit_progress = |tx: Option<&tokio::sync::mpsc::Sender<crate::runtime::EngineEvent>>,
                         id: Option<&str>,
                         last_tool: Option<String>,
                         last_tool_info: Option<String>,
                         tool_use_count: Option<u32>,
                         input_tokens: Option<u64>,
                         cache_read_tokens: Option<u64>,
                         cache_write_tokens: Option<u64>,
                         output_tokens: Option<u64>| {
        if let (Some(tx), Some(id)) = (tx, id) {
            // TaskProgress is non-critical; the next progress update supersedes this one.
            let _ = tx.try_send(crate::runtime::EngineEvent::Task(
                crate::runtime::TaskEvent::Progress {
                    task_id: crate::ids::TaskId::from(id),
                    last_tool,
                    last_tool_info,
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
        if let Some(cap) = max_turns
            && turn > cap
        {
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

        let mut options = StreamOptions::new(model.clone()).tools(tools.clone());
        if let Some(sp) = &system_prompt {
            options = options.system(sp.clone());
        }
        // Apply reasoning effort: Task.effort > AgentDef.effort > None (server default).
        //
        // Previously: the fallback was `crate::effort::active_global()`, which let
        // the parent's pinned effort leak through to every child. Useful for
        // interactive sessions (one `/effort max` pin propagates), but a silent
        // confound for experiments comparing model tiers or prompt layers — the
        // parent's effort becomes a hidden independent variable. Now: if neither
        // Task nor AgentDef set effort, we send no `reasoning_effort` field and
        // the provider/model policy applies its default (typically None → server
        // picks). To restore the old propagation, set `task_input.effort` explicitly
        // on every spawn or pin it in the agent def.
        if let Some(effort_val) = task_input.effort.as_deref() {
            options = options.reasoning_effort(effort_val);
        } else if let Some(agent_effort) = agent_def.and_then(|a| a.effort.as_ref()) {
            let val = match agent_effort {
                jfc_core::Effort::Minimal => "low",
                jfc_core::Effort::Low => "low",
                jfc_core::Effort::Medium => "medium",
                jfc_core::Effort::High => "high",
                jfc_core::Effort::XHigh => "xhigh",
            };
            options = options.reasoning_effort(val);
        }
        // Old fallback removed: `else if let Some(global) = crate::effort::active_global()`
        // See comment above for rationale.

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
        let context_safety = crate::stream::apply_subagent_context_safety(
            &mut conversation,
            provider,
            model.clone(),
        )
        .await;
        if context_safety.compacted {
            tracing::info!(
                target: "jfc::tools",
                task_id = ?task_id,
                turn,
                "subagent transcript auto-compacted"
            );
        }
        if context_safety.elided {
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
                    if let Some(retry) = jfc_provider::retry::retryable_stream_error(&message) {
                        let delay = jfc_provider::retry::stream_retry_delay(stream_retry_attempt);
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

            // Shared per-turn drain (stream/agent_drain.rs) — the same driver
            // the teammate loop uses. The sink pipes text deltas to the task
            // panel and forwards usage to the parent fan UI.
            use crate::stream::agent_drain::{
                AgentDrainEvent, AgentDrainOutcome, DrainCancel, drain_agent_stream,
            };
            let cancelled = || {
                task_id
                    .map(crate::daemon::background_agent_cancel_requested)
                    .unwrap_or(false)
            };
            let mut reported_input_for_turn = false;
            let mut sink = |ev: AgentDrainEvent| -> futures::future::BoxFuture<'static, ()> {
                match ev {
                    AgentDrainEvent::TextDelta(delta) => {
                        // Pipe deltas through to the task panel so the user
                        // sees the subagent's prose stream live. Blocking send
                        // — try_send would drop deltas under backpressure and
                        // permanently lose subagent prose.
                        if let (Some(tx), Some(id)) = (tx, task_id) {
                            let tx = tx.clone();
                            let task_id = crate::ids::TaskId::from(id);
                            return Box::pin(async move {
                                let _ = tx
                                    .send(crate::runtime::EngineEvent::Task(
                                        crate::runtime::TaskEvent::AgentChunk {
                                            task_id,
                                            text: delta,
                                        },
                                    ))
                                    .await;
                            });
                        }
                    }
                    AgentDrainEvent::Usage {
                        input_tokens,
                        cache_read_tokens,
                        cache_write_tokens,
                        output_delta,
                    } => {
                        // Surface this turn's input + output tokens to the
                        // parent fan UI. Input/cache are sent once per API
                        // round-trip so the session cost ledger can add the
                        // request once; output remains a streaming delta.
                        let input_update = if reported_input_for_turn {
                            None
                        } else {
                            reported_input_for_turn = true;
                            Some((input_tokens, cache_read_tokens, cache_write_tokens))
                        };
                        emit_progress(
                            tx,
                            task_id,
                            None,
                            None,
                            None,
                            input_update.map(|(input, _, _)| input),
                            input_update.map(|(_, cache_read, _)| cache_read),
                            input_update.map(|(_, _, cache_write)| cache_write),
                            Some(output_delta),
                        );
                    }
                    AgentDrainEvent::ToolUse { name, input_json } => {
                        let tool_info = tool_info_from_raw_json(&name, &input_json);
                        emit_progress(
                            tx,
                            task_id,
                            Some(name),
                            tool_info,
                            None,
                            None,
                            None,
                            None,
                            None,
                        );
                    }
                }
                Box::pin(async {})
            };
            let outcome =
                drain_agent_stream(stream, DrainCancel::Poll(&cancelled), &mut sink).await;

            match outcome {
                AgentDrainOutcome::Completed(drained) => {
                    break (
                        drained.text,
                        drained
                            .tool_uses
                            .into_iter()
                            .map(|tu| (tu.id, tu.name, tu.input_json, tu.thought_signature))
                            .collect::<Vec<_>>(),
                        drained.stop_reason,
                    );
                }
                AgentDrainOutcome::Cancelled => {
                    return ExecutionResult::failure(
                        "cancelled: background agent cancellation requested",
                    );
                }
                AgentDrainOutcome::Fatal(message) => {
                    last_error = Some(message);
                    break 'outer;
                }
                AgentDrainOutcome::Retryable(message) => {
                    let Some(retry) = jfc_provider::retry::retryable_stream_error(&message) else {
                        last_error = Some(message);
                        break 'outer;
                    };
                    let delay = jfc_provider::retry::stream_retry_delay(stream_retry_attempt);
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
                    if cancelled() {
                        return ExecutionResult::failure(
                            "cancelled: background agent cancellation requested",
                        );
                    }
                    continue;
                }
            }
        };

        // Append the assistant turn (text + tool_uses, if any) so the
        // next iteration's request reflects the running history.
        let mut assistant_content = Vec::new();
        if !turn_text.is_empty() {
            assistant_content.push(ProviderContent::Text(turn_text.clone()));
        }
        for (id, name, input_json, sig) in &tool_uses {
            let parsed_input: serde_json::Value =
                serde_json::from_str(input_json).unwrap_or(serde_json::Value::Null);
            assistant_content.push(ProviderContent::ToolUse {
                id: id.clone(),
                name: name.clone(),
                input: parsed_input,
                thought_signature: sig.clone(),
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
            final_text = turn_text.clone();
            // …but also retain it so a final short preamble doesn't erase a
            // substantive earlier turn (the "lost report" case).
            narrative.push(turn_text);
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
        for (id, name, input_json, _sig) in tool_uses {
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
                        tool_info_from_raw_json(&name, &input_json),
                        Some(total_tool_uses),
                        None,
                        None,
                        None,
                        None,
                    );
                    continue;
                }
            };
            // CS-JFC-005: gate high-impact tools through the delegated permission
            // policy before dispatch. A detached/background subagent has no human
            // to answer an approval prompt and no classifier wired in, so anything
            // that would prompt or need classification fails closed. Internal
            // control tools (StructuredOutput, nested Task) are exempt — they do
            // not touch the host and are governed by their own depth/schema gates.
            let exempt_from_gate = matches!(
                input,
                ToolInput::StructuredOutput { .. } | ToolInput::Task(_)
            );
            if !exempt_from_gate {
                use crate::app::PermissionDecision;
                match permission_mode.decide_parts(&kind, &input) {
                    PermissionDecision::Approved => {}
                    PermissionDecision::Denied(reason) => {
                        tool_results.push(ProviderContent::ToolResult {
                            tool_use_id: id.clone(),
                            content: format!(
                                "Tool '{name}' denied by the agent's permission policy: {reason}"
                            ),
                            is_error: true,
                        });
                        total_tool_uses = total_tool_uses.saturating_add(1);
                        continue;
                    }
                    PermissionDecision::NeedsPrompt | PermissionDecision::NeedsClassifier => {
                        tool_results.push(ProviderContent::ToolResult {
                            tool_use_id: id.clone(),
                            content: format!(
                                "Tool '{name}' requires interactive approval and cannot run \
                                 unattended in a background/subagent context. Grant it \
                                 explicitly via the agent's permissionMode or allowedTools."
                            ),
                            is_error: true,
                        });
                        total_tool_uses = total_tool_uses.saturating_add(1);
                        continue;
                    }
                }
            }
            let structured_payload = match &input {
                ToolInput::StructuredOutput { data } => Some(data.clone()),
                _ => None,
            };
            let tool_info = tool_progress_info(&name, &input);
            let result = if let ToolInput::StructuredOutput { .. } = &input {
                if schema_required {
                    execute_tool(
                        kind,
                        input,
                        cwd.clone(),
                        None,
                        task_store.clone(),
                        active_team_name.as_deref(),
                    )
                    .await
                } else {
                    ExecutionResult::failure(
                        "StructuredOutput is only available for tasks with a schema.",
                    )
                }
            } else if let ToolInput::Task(nested_task) = &input {
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
            if !is_error && let Some(data) = structured_payload {
                structured_output = Some(data);
            }
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
                Some(tool_info),
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

    let mut result = if let Some(data) = structured_output {
        ExecutionResult::success(serde_json::to_string(&data).unwrap_or_else(|_| data.to_string()))
    } else if schema_required {
        ExecutionResult::failure(
            "Subagent ended without calling StructuredOutput with a valid object matching the required schema.",
        )
    } else if let Some(err) = last_error {
        if final_text.is_empty() {
            ExecutionResult::failure(err)
        } else {
            ExecutionResult::success(format!("{final_text}\n\n[note: {err}]"))
        }
    } else if final_text.trim().is_empty() {
        // No error and all tools completed, but the final turn emitted no
        // prose. If earlier turns produced substantive narrative, surface that
        // (the "lost report" guard) rather than a synthetic one-liner.
        if !narrative.is_empty() {
            ExecutionResult::success(narrative.join("\n\n"))
        } else {
            // Pure file-editing tasks (Write, Edit, Bash) often emit no summary
            // paragraph at all. Treat as success with a synthetic summary so the
            // parent doesn't misreport the run as failed.
            let summary = if total_tool_uses > 0 {
                format!(
                    "Completed task successfully. Executed {total_tool_uses} tool \
                     call{} in isolated context.",
                    if total_tool_uses == 1 { "" } else { "s" }
                )
            } else {
                "Completed task successfully.".to_string()
            };
            ExecutionResult::success(summary)
        }
    } else {
        ExecutionResult::success(harvest_final_output(&final_text, &narrative))
    };

    if is_verification_agent(agent_def)
        && let Some(path) = persist_verification_report(&cwd, &result.output, total_tool_uses)
    {
        let display = path
            .strip_prefix(&cwd)
            .unwrap_or(path.as_path())
            .display()
            .to_string();
        result
            .output
            .push_str(&format!("\n\n[verification report persisted: {display}]"));
    }

    result
}

/// Choose the text to surface as a subagent's final result, guarding against the
/// "lost report" case: when the agent's LAST turn is a short preamble (e.g.
/// "Let me write the report:") that precedes a tool call after which the loop
/// ended, `final_text` holds only that dangling fragment and discards the
/// substantive earlier narrative. When the final turn looks like such a preamble
/// and there is richer prior narrative, return the full accumulated narrative.
fn harvest_final_output(final_text: &str, narrative: &[String]) -> String {
    let trimmed = final_text.trim();
    let looks_like_preamble = trimmed.len() < 120 && trimmed.ends_with(':');
    if looks_like_preamble && narrative.len() > 1 {
        let joined = narrative.join("\n\n");
        if joined.trim().len() > trimmed.len() {
            return joined;
        }
    }
    final_text.to_owned()
}

fn is_verification_agent(agent_def: Option<&crate::agents::AgentDef>) -> bool {
    agent_def
        .map(|agent| agent.name.eq_ignore_ascii_case("verification"))
        .unwrap_or(false)
}

fn persist_verification_report(cwd: &Path, output: &str, total_tool_uses: u32) -> Option<PathBuf> {
    let verdict = verification_verdict(output)?;
    if verdict == "PASS" {
        return None;
    }

    let root = cwd.join(".jfc/verification");
    let reports_dir = root.join("reports");
    if let Err(err) = std::fs::create_dir_all(&reports_dir) {
        tracing::warn!(
            target: "jfc::verification",
            error = %err,
            path = %reports_dir.display(),
            "failed to create verification report directory"
        );
        return None;
    }

    let now = chrono::Utc::now();
    let file_name = format!(
        "{}-{}.md",
        now.format("%Y%m%dT%H%M%SZ"),
        verdict.to_ascii_lowercase()
    );
    let report_path = reports_dir.join(file_name);
    let report = format!(
        "# Verification Report: {verdict}\n\n\
         - Generated: {}\n\
         - Agent: verification\n\
         - Tool calls: {total_tool_uses}\n\n\
         ## Output\n\n{}\n",
        now.to_rfc3339(),
        output.trim()
    );

    if let Err(err) = atomic_write(&report_path, &report) {
        tracing::warn!(
            target: "jfc::verification",
            error = %err,
            path = %report_path.display(),
            "failed to write verification report"
        );
        return None;
    }

    if let Err(err) = update_verification_index(&root, cwd, &report_path, verdict, now, output) {
        tracing::warn!(
            target: "jfc::verification",
            error = %err,
            path = %root.join("index.md").display(),
            "failed to update verification index"
        );
    }

    Some(report_path)
}

fn verification_verdict(output: &str) -> Option<&'static str> {
    for line in output.lines().rev().map(str::trim) {
        if line.eq_ignore_ascii_case("VERDICT: FAIL") {
            return Some("FAIL");
        }
        if line.eq_ignore_ascii_case("VERDICT: PARTIAL") {
            return Some("PARTIAL");
        }
        if line.eq_ignore_ascii_case("VERDICT: PASS") {
            return Some("PASS");
        }
    }
    None
}

fn update_verification_index(
    root: &Path,
    cwd: &Path,
    report_path: &Path,
    verdict: &str,
    now: chrono::DateTime<chrono::Utc>,
    output: &str,
) -> std::io::Result<()> {
    let index_path = root.join("index.md");
    let old = std::fs::read_to_string(&index_path).unwrap_or_default();
    let rel_report = report_path
        .strip_prefix(cwd)
        .unwrap_or(report_path)
        .display()
        .to_string();
    let excerpt = verification_excerpt(output);
    let new_entry = format!(
        "- {} `{}` [{}]({}) - {}\n",
        now.to_rfc3339(),
        verdict,
        rel_report,
        rel_report,
        excerpt
    );

    let mut body = String::from(
        "# Verification Findings\n\n\
         Generated by the verification agent when a run ends with \
         `VERDICT: FAIL` or `VERDICT: PARTIAL`.\n\n\
         ## Recent Reports\n\n",
    );
    body.push_str(&new_entry);
    for line in old.lines().filter(|line| line.starts_with("- ")).take(49) {
        body.push_str(line);
        body.push('\n');
    }
    atomic_write(&index_path, &body)
}

fn verification_excerpt(output: &str) -> String {
    let line = output
        .lines()
        .map(str::trim)
        .find(|line| {
            !line.is_empty()
                && !line.starts_with("```")
                && !line.eq_ignore_ascii_case("VERDICT: FAIL")
                && !line.eq_ignore_ascii_case("VERDICT: PARTIAL")
                && !line.eq_ignore_ascii_case("VERDICT: PASS")
        })
        .unwrap_or("verification reported a blocker");
    let mut excerpt: String = line.chars().take(160).collect();
    if line.chars().count() > 160 {
        excerpt.push_str("...");
    }
    excerpt
}

fn atomic_write(path: &Path, body: &str) -> std::io::Result<()> {
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, body)?;
    std::fs::rename(tmp, path)
}

/// Build a compact context seed for subagent context inheritance.
///
/// When `subagent_context_inheritance = true` in config, the tool dispatch
/// path calls this before spawning a subagent. The returned JSON object is
/// stored in `AgentDef::forks_parent_context` and injected into the
/// subagent's system prompt by [`inject_parent_context`].
///
/// The seed includes:
/// - The current working directory path
/// - A compact CLAUDE.md summary from the project hierarchy (up to 4 000 chars)
pub fn build_parent_context_seed(cwd: &Path) -> serde_json::Value {
    let mut seed = serde_json::Map::new();

    seed.insert(
        "cwd".to_string(),
        serde_json::Value::String(cwd.display().to_string()),
    );

    // Load the CLAUDE.md hierarchy and render a compact summary.
    let hierarchy = crate::context::ClaudeMdHierarchy::load(cwd);
    if let Some(rendered) = hierarchy.render() {
        let trimmed: String = rendered.chars().take(4000).collect();
        seed.insert(
            "claude_md_summary".to_string(),
            serde_json::Value::String(trimmed),
        );
    }

    serde_json::Value::Object(seed)
}

/// Inject a `forks_parent_context` seed into an existing system prompt string.
///
/// Called inside `execute_task_inner` when `agent_def.forks_parent_context`
/// is `Some`. Appends a `<parent_context>` block so the subagent knows what
/// the parent already loaded, saving redundant codebase re-scans.
fn inject_parent_context(system_prompt: &mut String, seed: &serde_json::Value) {
    let mut block = String::from(
        "\n\n<parent_context>\n\
         The parent session has already loaded the following project context.\n\
         You may rely on it directly instead of re-scanning from scratch:\n\n",
    );

    if let Some(cwd) = seed.get("cwd").and_then(|v| v.as_str()) {
        block.push_str(&format!("**Working directory:** `{cwd}`\n\n"));
    }

    if let Some(claude_md) = seed.get("claude_md_summary").and_then(|v| v.as_str()) {
        if !claude_md.is_empty() {
            block.push_str("**CLAUDE.md context (from parent):**\n\n```\n");
            block.push_str(claude_md);
            block.push_str("\n```\n");
        }
    }

    block.push_str("</parent_context>");
    system_prompt.push_str(&block);
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

    /// Acquire the env serialization lock, tolerating poisoning. These tests
    /// mutate the process-global `CLAUDE_CODE_SUBAGENT_MODEL` env var, so they
    /// must run serially. Without poison tolerance, the FIRST test to panic
    /// (a real assertion failure) poisons the mutex and every subsequent test
    /// fails with a *lock* panic — masking which test actually broke. Recover
    /// the guard so only the genuine failure is reported.
    fn env_lock() -> std::sync::MutexGuard<'static, ()> {
        ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn clear_subagent_model_env() {
        // SAFETY: subagent model-selection tests hold `env_lock()`, so no
        // sibling test in this module mutates this process environment key at
        // the same time.
        unsafe { std::env::remove_var("CLAUDE_CODE_SUBAGENT_MODEL") };
    }

    fn set_subagent_model_env(value: &str) {
        // SAFETY: subagent model-selection tests hold `env_lock()`, so no
        // sibling test in this module mutates this process environment key at
        // the same time.
        unsafe { std::env::set_var("CLAUDE_CODE_SUBAGENT_MODEL", value) };
    }

    fn make_tool_def(name: &str) -> ToolDef {
        ToolDef {
            name: name.to_owned(),
            description: "test".to_owned(),
            input_schema: serde_json::json!({"type": "object"}),
        }
    }

    #[test]
    fn harvest_returns_full_narrative_when_final_is_preamble_normal() {
        // The agent did substantive work, then its last turn was a preamble
        // before a tool call, after which the loop ended.
        let narrative = vec![
            "## Findings\nThe bug is in foo.rs:42 where the lock is dropped early.".to_string(),
            "Let me write the report:".to_string(),
        ];
        let final_text = "Let me write the report:";
        let out = harvest_final_output(final_text, &narrative);
        assert!(out.contains("foo.rs:42"), "must recover the lost report");
        assert!(out.contains("Let me write the report:"));
    }

    #[test]
    fn harvest_keeps_final_text_when_substantive_normal() {
        let narrative = vec![
            "intermediate thinking".to_string(),
            "## Final Answer\nHere is the complete result with all details.".to_string(),
        ];
        let final_text = "## Final Answer\nHere is the complete result with all details.";
        let out = harvest_final_output(final_text, &narrative);
        assert_eq!(out, final_text, "a substantive final turn is used as-is");
    }

    #[test]
    fn harvest_keeps_final_text_when_no_prior_narrative_robust() {
        // Only one turn (which happens to end in a colon) — nothing to recover.
        let narrative = vec!["Plan:".to_string()];
        let out = harvest_final_output("Plan:", &narrative);
        assert_eq!(out, "Plan:");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn persist_background_result_writes_full_body_to_db_normal() {
        let body = "X".repeat(5000);
        let handle = crate::runtime::persist_background_result("toolu_test_abc123", &body)
            .expect("artifact written");
        assert_eq!(
            handle.to_string_lossy(),
            "db:background-result:toolu_test_abc123"
        );
        let row = jfc_knowledge::KnowledgeStore::open_default()
            .await
            .unwrap()
            .get_session_artifact("__daemon__", "background_result", "toolu_test_abc123")
            .await
            .unwrap()
            .expect("read back");
        assert_eq!(
            row.value_json.len(),
            5000,
            "full body persisted, not truncated"
        );
    }

    fn task_input(model: Option<&str>) -> crate::types::TaskInput {
        crate::types::TaskInput {
            description: "inspect".to_string(),
            prompt: "inspect".to_string(),
            subagent_type: Some("explore".to_string()),
            category: None,
            run_in_background: false,
            model: model.map(str::to_string),
            launcher: None,
            effort: None,
            name: None,
            team_name: None,
            mode: None,
            isolation: None,
            parent_task_id: None,
            schema: None,
            allowed_tools: Vec::new(),
            disallowed_tools: Vec::new(),
            cwd: None,
        }
    }

    #[test]
    fn scoped_task_tools_narrow_agent_tool_lists_regression() {
        let allowed = scoped_allowed_tools(
            &["Read".to_owned(), "Grep".to_owned(), "Edit".to_owned()],
            &["read".to_owned(), "Bash".to_owned()],
        );
        assert_eq!(allowed, ToolAllowScope::Only(vec!["Read".to_owned()]));

        let disallowed = scoped_disallowed_tools(
            &["Write".to_owned()],
            &["write".to_owned(), "Grep".to_owned()],
        );
        assert_eq!(disallowed, vec!["Write", "Grep"]);
    }

    #[test]
    fn scoped_task_tools_empty_intersection_denies_all_regression() {
        let allowed = scoped_allowed_tools(&["Read".to_owned()], &["Bash".to_owned()]);
        assert_eq!(allowed, ToolAllowScope::Only(Vec::new()));

        let tools = filter_tools_for_agent_scope(
            vec![make_tool_def("Read"), make_tool_def("Bash")],
            &allowed,
            &[],
            false,
        );
        assert!(
            tools.is_empty(),
            "empty scoped intersection must not widen to all tools"
        );
    }

    #[test]
    fn scoped_task_tools_match_raw_codegraph_to_mcp_allowlist_regression() {
        let allowed = scoped_allowed_tools(
            &["mcp__codegraph__codegraph_explore".to_owned()],
            &["codegraph_explore".to_owned()],
        );
        assert_eq!(
            allowed,
            ToolAllowScope::Only(vec!["mcp__codegraph__codegraph_explore".to_owned()])
        );

        let tools = filter_tools_for_agent_scope(
            vec![
                make_tool_def("Read"),
                make_tool_def("mcp__codegraph__codegraph_explore"),
            ],
            &allowed,
            &[],
            false,
        );
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].name, "mcp__codegraph__codegraph_explore");
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
        let _guard = env_lock();
        clear_subagent_model_env();

        let model = selected_subagent_model(
            &task_input(None),
            Some(&agent_model(Some("haiku"))),
            jfc_provider::ModelId::new("claude-opus-4-6"),
            "openwebui",
        )
        .unwrap();

        assert_eq!(model.as_str(), "bedrock-claude-4-5-haiku");
    }

    struct NamedTestProvider(&'static str);

    #[async_trait::async_trait]
    impl jfc_provider::Provider for NamedTestProvider {
        fn name(&self) -> &str {
            self.0
        }
        fn available_models(&self) -> Vec<jfc_provider::ModelInfo> {
            Vec::new()
        }
        async fn stream(
            &self,
            _messages: Vec<jfc_provider::ProviderMessage>,
            _options: &jfc_provider::StreamOptions,
        ) -> anyhow::Result<jfc_provider::EventStream> {
            Ok(Box::pin(futures::stream::empty()))
        }
    }

    impl jfc_provider::seal::Sealed for NamedTestProvider {}

    // Regression: a provider-qualified model on Task ("openai/gpt-x") must
    // switch to that provider via the registry, not hard-error as it did
    // before selected_subagent_provider_model existed.
    #[test]
    fn selected_subagent_provider_model_switches_provider_regression() {
        let _guard = env_lock();
        clear_subagent_model_env();

        let anthropic: std::sync::Arc<dyn jfc_provider::Provider> =
            std::sync::Arc::new(NamedTestProvider("anthropic-oauth"));
        let openai: std::sync::Arc<dyn jfc_provider::Provider> =
            std::sync::Arc::new(NamedTestProvider("openai"));
        let registry = vec![anthropic.clone(), openai.clone()];

        let Ok((resolved_provider, resolved_model)) = selected_subagent_provider_model(
            &task_input(Some("openai/gpt-5.2")),
            None,
            anthropic.clone(),
            jfc_provider::ModelId::new("claude-opus-4-6"),
            &registry,
        ) else {
            panic!("qualified spec should switch providers");
        };

        assert_eq!(resolved_provider.name(), "openai");
        assert_eq!(resolved_model.as_str(), "gpt-5.2");
    }

    #[test]
    fn selected_subagent_provider_model_accepts_anthropic_prefix_under_oauth_regression() {
        let _guard = env_lock();
        clear_subagent_model_env();

        let anthropic: std::sync::Arc<dyn jfc_provider::Provider> =
            std::sync::Arc::new(NamedTestProvider("anthropic-oauth"));
        let registry = vec![anthropic.clone()];

        let Ok((resolved_provider, resolved_model)) = selected_subagent_provider_model(
            &task_input(Some("anthropic/claude-opus-4-6")),
            None,
            anthropic,
            jfc_provider::ModelId::new("claude-sonnet-4-6"),
            &registry,
        ) else {
            panic!("anthropic prefix should resolve to OAuth Anthropic provider");
        };

        assert_eq!(resolved_provider.name(), "anthropic-oauth");
        assert_eq!(resolved_model.as_str(), "claude-opus-4-6");
    }

    #[test]
    fn subagent_model_request_keeps_config_source_for_provider_resolution_regression() {
        let _guard = env_lock();
        clear_subagent_model_env();

        let task = task_input(None);
        let raw = selected_subagent_model_request_from_sources(
            &task,
            Some(&agent_model(Some("anthropic/claude-sonnet"))),
            Some("openai/gpt-5.2".to_owned()),
        );

        assert_eq!(raw.as_deref(), Some("openai/gpt-5.2"));
    }

    // Robust: a qualified spec naming a provider that is NOT configured
    // still errors, and the error names the missing provider.
    #[test]
    fn selected_subagent_provider_model_unknown_provider_errors_robust() {
        let _guard = env_lock();
        clear_subagent_model_env();

        let anthropic: std::sync::Arc<dyn jfc_provider::Provider> =
            std::sync::Arc::new(NamedTestProvider("anthropic-oauth"));
        let registry = vec![anthropic.clone()];

        let Err(err) = selected_subagent_provider_model(
            &task_input(Some("zai/glm-5")),
            None,
            anthropic.clone(),
            jfc_provider::ModelId::new("claude-opus-4-6"),
            &registry,
        ) else {
            panic!("unknown provider must error");
        };
        assert!(err.contains("zai"), "{err}");
    }

    #[test]
    fn verification_verdict_detects_terminal_status_robust() {
        assert_eq!(verification_verdict("stuff\nVERDICT: FAIL\n"), Some("FAIL"));
        assert_eq!(
            verification_verdict("stuff\nVERDICT: PARTIAL\n"),
            Some("PARTIAL")
        );
        assert_eq!(verification_verdict("stuff\nVERDICT: PASS\n"), Some("PASS"));
        assert_eq!(verification_verdict("stuff\nverdict: fail\n"), Some("FAIL"));
        assert_eq!(verification_verdict("stuff"), None);
    }

    #[test]
    fn persist_verification_report_writes_failures_only_normal() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let pass = persist_verification_report(tmp.path(), "all good\nVERDICT: PASS", 2);
        assert!(pass.is_none());

        let fail = persist_verification_report(
            tmp.path(),
            "### Check: cargo test\n**Result: FAIL**\nVERDICT: FAIL",
            3,
        )
        .expect("report path");

        assert!(fail.exists());
        let index = std::fs::read_to_string(tmp.path().join(".jfc/verification/index.md"))
            .expect("index exists");
        assert!(index.contains("`FAIL`"));
        assert!(index.contains(".jfc/verification/reports/"));
    }

    #[test]
    fn selected_subagent_model_env_overrides_task_model() {
        let _guard = env_lock();
        set_subagent_model_env("haiku");

        let model = selected_subagent_model(
            &task_input(Some("opus")),
            Some(&agent_model(Some("sonnet"))),
            jfc_provider::ModelId::new("claude-opus-4-6"),
            "openwebui",
        )
        .unwrap();

        clear_subagent_model_env();
        assert_eq!(model.as_str(), "bedrock-claude-4-5-haiku");
    }

    #[test]
    fn selected_subagent_model_maps_builtin_tiers_for_openai() {
        let _guard = env_lock();
        clear_subagent_model_env();

        let model = selected_subagent_model(
            &task_input(None),
            Some(&agent_model(Some("haiku"))),
            jfc_provider::ModelId::new("gpt-5.5"),
            "openai",
        )
        .unwrap();

        assert_eq!(model.as_str(), "gpt-5-mini");
    }

    #[test]
    fn selected_subagent_model_maps_builtin_tiers_for_anthropic_oauth() {
        let _guard = env_lock();
        clear_subagent_model_env();

        let model = selected_subagent_model(
            &task_input(Some("haiku")),
            None,
            jfc_provider::ModelId::new("claude-opus-4-7"),
            "anthropic-oauth",
        )
        .unwrap();

        assert_eq!(
            model.as_str(),
            crate::providers::anthropic_models::ALIAS_HAIKU
        );
    }

    #[test]
    fn prompt_complexity_tier_upgrades_hard_reasoning_normal() {
        // Unambiguously hard: two hard signals, no mechanical phrasing → opus.
        assert_eq!(
            prompt_complexity_tier("redesign the auth architecture", "design"),
            Some("opus")
        );
        assert_eq!(
            prompt_complexity_tier(
                "prove the concurrency invariant holds under the new unsafe block",
                "audit"
            ),
            Some("opus")
        );
    }

    #[test]
    fn prompt_complexity_tier_mixed_signal_is_balanced_robust() {
        // Mechanical phrasing ("find") + a hard noun ("race condition") is a
        // genuine tie with non-zero hits → balanced sonnet, never the cheapest.
        assert_eq!(
            prompt_complexity_tier("find and fix the race condition in the scheduler", "debug"),
            Some("sonnet")
        );
    }

    #[test]
    fn prompt_complexity_tier_downgrades_mechanical_normal() {
        assert_eq!(
            prompt_complexity_tier("list all files that import serde", "map"),
            Some("haiku")
        );
        assert_eq!(
            prompt_complexity_tier("grep for the error string", "search"),
            Some("haiku")
        );
    }

    #[test]
    fn prompt_complexity_tier_inherits_on_weak_signal_robust() {
        // A medium-length, signal-free prompt yields None (inherit the parent).
        assert_eq!(
            prompt_complexity_tier(
                "Please take a careful pass over the module and report what you see in general.",
                "task"
            ),
            None
        );
        // Empty text never routes.
        assert_eq!(prompt_complexity_tier("", ""), None);
    }

    // End-to-end: a bare Task (no category, no model) routes by prompt
    // complexity instead of inheriting the heavy parent model.
    #[test]
    fn selected_subagent_model_routes_by_prompt_complexity_when_no_category_normal() {
        let _guard = env_lock();
        clear_subagent_model_env();

        let mut input = task_input(None);
        input.subagent_type = None; // no config-model lookup
        input.category = None;
        input.prompt = "list every file that references the old API".to_string();
        input.description = "find call sites".to_string();

        let model = selected_subagent_model(
            &input,
            None,
            jfc_provider::ModelId::new("claude-opus-4-7"),
            "anthropic-oauth",
        )
        .unwrap();

        // Mechanical lookup → haiku tier, not the inherited opus parent.
        assert_eq!(
            model.as_str(),
            crate::providers::anthropic_models::ALIAS_HAIKU
        );
    }

    // The cost-AND-quality safety net: a genuinely ambiguous category-less
    // Task with a realistic (non-"inspect") signal-free prompt must INHERIT the
    // parent model, not silently downgrade to haiku. This guards the t907
    // behavior change — bare Tasks only re-route on a clear signal.
    #[test]
    fn selected_subagent_model_weak_signal_inherits_parent_normal() {
        let _guard = env_lock();
        clear_subagent_model_env();

        let mut input = task_input(None);
        input.subagent_type = None;
        input.category = None;
        input.description = "follow-up".to_string();
        input.prompt =
            "Continue with the work we discussed and let me know how it goes when you can."
                .to_string();

        let parent = jfc_provider::ModelId::new("claude-opus-4-6");
        let model =
            selected_subagent_model(&input, None, parent.clone(), "anthropic-oauth").unwrap();

        assert_eq!(
            model.as_str(),
            parent.as_str(),
            "weak/ambiguous prompt must inherit the parent, not downgrade"
        );
    }

    // An explicit category still wins over the complexity heuristic.
    #[test]
    fn selected_subagent_model_category_beats_complexity_robust() {
        let _guard = env_lock();
        clear_subagent_model_env();

        let mut input = task_input(None);
        input.subagent_type = None;
        // Prompt screams "hard" but the explicit category says cheap mapping.
        input.category = Some("explore".to_string());
        input.prompt = "redesign the security architecture and prove the invariant".to_string();
        input.description = "audit".to_string();

        let model = selected_subagent_model(
            &input,
            None,
            jfc_provider::ModelId::new("claude-opus-4-7"),
            "anthropic-oauth",
        )
        .unwrap();

        // category=explore → haiku wins; the opus-leaning prompt is ignored.
        assert_eq!(
            model.as_str(),
            crate::providers::anthropic_models::ALIAS_HAIKU
        );
    }

    #[test]
    fn selected_subagent_model_rejects_cross_provider_models() {
        let _guard = env_lock();
        clear_subagent_model_env();

        let error = selected_subagent_model(
            &task_input(Some("anthropic/claude-haiku-4-5")),
            None,
            jfc_provider::ModelId::new("bedrock-claude-4-6-opus"),
            "openwebui",
        )
        .unwrap_err();

        assert!(error.contains("provider switching for subagents is not wired yet"));
    }

    #[test]
    fn tool_progress_info_formats_read_path_normal() {
        let info = tool_progress_info(
            "Read",
            &ToolInput::Read {
                file_path: "crates/jfc/src/render/roster.rs".into(),
                offset: None,
                limit: None,
            },
        );
        assert_eq!(info, "Read(crates/jfc/src/render/roster.rs)");
    }

    #[test]
    fn tool_progress_info_formats_bash_command_normal() {
        let info = tool_progress_info(
            "Bash",
            &ToolInput::Bash {
                command: "cargo test -p jfc\ncargo clippy".into(),
                timeout: None,
                workdir: None,
                run_in_background: None,
                suppress_output: None,
            },
        );
        assert_eq!(info, "Bash(cargo test -p jfc)");
    }

    #[test]
    fn append_code_navigation_guidance_names_mcp_codegraph_normal() {
        let mut system_prompt = Some("base".to_owned());

        append_code_navigation_guidance(&mut system_prompt);

        let prompt = system_prompt.expect("prompt");
        assert!(prompt.contains("CodeGraph"));
        assert!(prompt.contains("mcp__codegraph__codegraph_explore"));
        assert!(prompt.contains("before broad Read"));
    }

    /// Build a TaskInput carrying a `category` and no explicit model, to test
    /// category→tier routing.
    fn task_input_with_category(category: &str) -> crate::types::TaskInput {
        let mut t = task_input(None);
        t.subagent_type = None; // avoid agent-config model interference
        t.category = Some(category.to_string());
        t
    }

    #[test]
    fn selected_subagent_model_routes_explore_category_to_haiku_normal() {
        let _guard = env_lock();
        clear_subagent_model_env();

        let model = selected_subagent_model(
            &task_input_with_category("explore"),
            None,
            jfc_provider::ModelId::new("claude-opus-4-6"),
            "openwebui",
        )
        .unwrap();
        assert_eq!(model.as_str(), "bedrock-claude-4-5-haiku");
    }

    #[test]
    fn selected_subagent_model_routes_security_category_to_opus_normal() {
        let _guard = env_lock();
        clear_subagent_model_env();

        let model = selected_subagent_model(
            &task_input_with_category("security"),
            None,
            jfc_provider::ModelId::new("bedrock-claude-4-5-haiku"),
            "openwebui",
        )
        .unwrap();
        assert_eq!(model.as_str(), "bedrock-claude-4-6-opus");
    }

    #[test]
    fn selected_subagent_model_explicit_model_overrides_category_robust() {
        let _guard = env_lock();
        clear_subagent_model_env();

        // category says haiku, but an explicit Task `model` must win.
        let mut t = task_input_with_category("explore");
        t.model = Some("sonnet".to_string());
        let model = selected_subagent_model(
            &t,
            None,
            jfc_provider::ModelId::new("claude-opus-4-6"),
            "openwebui",
        )
        .unwrap();
        assert_eq!(model.as_str(), "bedrock-claude-4-6-sonnet");
    }

    #[test]
    fn selected_subagent_model_unknown_category_inherits_parent_robust() {
        let _guard = env_lock();
        clear_subagent_model_env();

        let parent = jfc_provider::ModelId::new("claude-opus-4-6");
        let model = selected_subagent_model(
            &task_input_with_category("wibble-unknown"),
            None,
            parent.clone(),
            "anthropic",
        )
        .unwrap();
        assert_eq!(model.as_str(), parent.as_str());
    }

    struct ScriptedProvider {
        name: &'static str,
        scripts: Mutex<VecDeque<Vec<jfc_provider::StreamEvent>>>,
        calls: AtomicUsize,
    }

    impl ScriptedProvider {
        fn new(scripts: Vec<Vec<jfc_provider::StreamEvent>>) -> Self {
            Self::named("anthropic", scripts)
        }

        fn named(name: &'static str, scripts: Vec<Vec<jfc_provider::StreamEvent>>) -> Self {
            Self {
                name,
                scripts: Mutex::new(scripts.into()),
                calls: AtomicUsize::new(0),
            }
        }
    }

    #[async_trait::async_trait]
    impl jfc_provider::Provider for ScriptedProvider {
        fn name(&self) -> &str {
            self.name
        }

        fn available_models(&self) -> Vec<jfc_provider::ModelInfo> {
            vec![]
        }

        async fn stream(
            &self,
            _messages: Vec<jfc_provider::ProviderMessage>,
            _options: &jfc_provider::StreamOptions,
        ) -> anyhow::Result<jfc_provider::EventStream> {
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

    impl jfc_provider::seal::Sealed for ScriptedProvider {}

    #[tokio::test(flavor = "current_thread")]
    async fn execute_task_retries_retryable_stream_error_normal() {
        let provider = ScriptedProvider::new(vec![
            vec![jfc_provider::StreamEvent::Error {
                message: format!(
                    "{}Anthropic transient API error 529: overloaded",
                    crate::providers::anthropic::AUTO_RETRY_SENTINEL
                ),
            }],
            vec![
                jfc_provider::StreamEvent::TextDelta {
                    index: 0,
                    delta: "recovered".into(),
                },
                jfc_provider::StreamEvent::Done {
                    stop_reason: jfc_provider::StopReason::EndTurn,
                },
            ],
        ]);

        let result = execute_task(
            &task_input(None),
            &provider,
            jfc_provider::ModelId::new("claude-opus-4-7"),
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

    #[tokio::test(flavor = "current_thread")]
    async fn execute_task_uses_selected_process_bridge_launcher_normal() {
        // Given: a project plugin declares a process-bridge agent launcher.
        jfc_plugin_host::clear_discovered_plugin_state_cache_for_tests();
        let dir = tempfile::TempDir::new().expect("tempdir");
        let plugin = dir.path().join("plugins").join("agent-plugin");
        std::fs::create_dir_all(plugin.join("workflows")).expect("plugin dirs");
        let launcher = plugin.join("variant-agent.sh");
        std::fs::write(
            &launcher,
            "#!/bin/sh\n\
             line=$(cat)\n\
             id=$(printf '%s' \"$line\" | sed -n 's/.*\"id\":\"\\([^\"]*\\)\".*/\\1/p')\n\
             printf '{\"type\":\"response\",\"id\":\"%s\",\"response\":{\"kind\":\"agent_launch_result\",\"result\":{\"output\":\"selected launcher ran\",\"is_error\":false}}}\\n' \"$id\"\n",
        )
        .expect("write launcher script");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;

            let mut permissions = std::fs::metadata(&launcher)
                .expect("launcher metadata")
                .permissions();
            permissions.set_mode(0o755);
            std::fs::set_permissions(&launcher, permissions).expect("launcher permissions");
        }
        std::fs::write(
            plugin.join(".jfc-plugin.toml"),
            format!(
                r#"[plugin]
name = "agent-plugin"
workflows_dir = "workflows"

[[agent_launches]]
name = "variant-agent"
label = "Variant Agent"
description = "Launches a plugin-defined variant agent."

[agent_launches.executor]
kind = "process_bridge"
handler = "{}"
"#,
                launcher.display()
            ),
        )
        .expect("write plugin manifest");
        let mut input = task_input(None);
        input.launcher = Some("variant-agent".to_owned());
        let provider = ScriptedProvider::new(Vec::new());

        // When: foreground Task execution receives the launcher selection.
        let result = execute_task(
            &input,
            &provider,
            jfc_provider::ModelId::new("claude-opus-4-7"),
            None,
            Some("task_1"),
            Some(&agent_model(None)),
            Some(dir.path().to_path_buf()),
            None,
            None,
        )
        .await;

        // Then: the selected process bridge owns the observable Task result.
        assert!(!result.is_error(), "{}", result.output);
        assert_eq!(result.output, "selected launcher ran");
        assert_eq!(provider.calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn execute_task_switches_provider_from_registered_registry_regression() {
        let _guard = env_lock();
        clear_subagent_model_env();
        let anthropic: std::sync::Arc<ScriptedProvider> =
            std::sync::Arc::new(ScriptedProvider::named("anthropic", Vec::new()));
        let openai: std::sync::Arc<ScriptedProvider> =
            std::sync::Arc::new(ScriptedProvider::named(
                "openai",
                vec![vec![
                    jfc_provider::StreamEvent::TextDelta {
                        index: 0,
                        delta: "switched".into(),
                    },
                    jfc_provider::StreamEvent::Done {
                        stop_reason: jfc_provider::StopReason::EndTurn,
                    },
                ]],
            ));
        crate::tools::register_provider_registry(vec![
            anthropic.clone() as std::sync::Arc<dyn jfc_provider::Provider>,
            openai.clone() as std::sync::Arc<dyn jfc_provider::Provider>,
        ]);

        let result = execute_task(
            &task_input(Some("openai/gpt-5.2")),
            anthropic.as_ref(),
            jfc_provider::ModelId::new("claude-opus-4-7"),
            None,
            None,
            Some(&agent_model(None)),
            None,
            None,
            None,
        )
        .await;

        crate::tools::register_provider_registry(Vec::new());
        assert!(
            !result.is_error(),
            "qualified subagent model should switch providers: {}",
            result.output
        );
        assert_eq!(result.output, "switched");
        assert_eq!(
            anthropic.calls.load(Ordering::SeqCst),
            0,
            "parent provider must not receive a provider-qualified child stream"
        );
        assert_eq!(openai.calls.load(Ordering::SeqCst), 1);
    }

    // ── Effort precedence tests ──────────────────────────────────────────────
    // These lock the Task.effort > AgentDef.effort > None precedence chain after
    // removing the old `active_global()` leak. The precedence is applied when
    // building StreamOptions in `execute_task_inner` (lines 604-625 as of this
    // comment). We test the *result* of that logic by building the options with
    // the same helper chain and asserting the output effort matches expected.

    /// Helper that mimics the effort-resolution block in execute_task_inner.
    fn resolve_effort_for_test(
        task_effort: Option<&str>,
        agent_effort: Option<jfc_core::Effort>,
    ) -> Option<String> {
        let mut opts = jfc_provider::StreamOptions::new("claude-opus-4-7");

        if let Some(effort_val) = task_effort {
            opts = opts.reasoning_effort(effort_val);
        } else if let Some(agent_effort) = agent_effort {
            let val = match agent_effort {
                jfc_core::Effort::Minimal => "low",
                jfc_core::Effort::Low => "low",
                jfc_core::Effort::Medium => "medium",
                jfc_core::Effort::High => "high",
                jfc_core::Effort::XHigh => "xhigh",
            };
            opts = opts.reasoning_effort(val);
        }
        // Old fallback (active_global) removed — see comment in execute_task_inner.

        opts.reasoning_effort
    }

    #[test]
    fn effort_precedence_task_wins_normal() {
        // Task.effort set → it wins, agent_def.effort is ignored.
        let resolved = resolve_effort_for_test(Some("max"), Some(jfc_core::Effort::Low));
        assert_eq!(resolved, Some("max".to_string()));
    }

    #[test]
    fn effort_precedence_agent_def_wins_when_task_is_none_normal() {
        // Task.effort = None, AgentDef.effort set → agent def wins.
        let resolved = resolve_effort_for_test(None, Some(jfc_core::Effort::High));
        assert_eq!(resolved, Some("high".to_string()));
    }

    #[test]
    fn effort_precedence_defaults_to_none_when_both_unset_normal() {
        // Task.effort = None, AgentDef.effort = None → no effort field sent
        // (server/provider applies its default). This is the intended behavior
        // after removing the active_global() leak.
        let resolved = resolve_effort_for_test(None, None);
        assert_eq!(resolved, None);
    }
}

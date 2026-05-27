use std::sync::Arc;

use crate::runtime::{StreamRequestMetadata, StreamRequestOverrides, StreamToolChoice};
use crate::tools;
use jfc_provider::{
    ModelId, Provider, ProviderContent, ProviderMessage, ProviderRole, StreamConvention,
    StreamOptions,
};

use super::model_policy::{max_output_tokens_for, thinking_mode_for};

pub(super) struct PreparedStreamRequest {
    pub(super) opts: StreamOptions,
    pub(super) system_prompt_tokens: usize,
    pub(super) metadata: StreamRequestMetadata,
}

/// Pull the most recent user-role text out of a provider message vec. Used by
/// the memory-recall pass to know what query the user actually asked. Returns
/// `None` when the conversation is empty or the last user turn carried only
/// tool results (no plain text). Concatenates multiple text blocks in the
/// same message with newlines so multi-paragraph prompts survive intact.
fn last_user_text(messages: &[ProviderMessage]) -> Option<String> {
    for msg in messages.iter().rev() {
        if msg.role != ProviderRole::User {
            continue;
        }
        let mut buf = String::new();
        for c in &msg.content {
            if let ProviderContent::Text(t) = c {
                if !buf.is_empty() {
                    buf.push('\n');
                }
                buf.push_str(t);
            }
        }
        if !buf.trim().is_empty() {
            return Some(buf);
        }
    }
    None
}

/// True when the conversation is in the middle of an agentic tool loop —
/// i.e. the most recent provider message carries `ToolResult` blocks the
/// model still has to react to. On a post-tool continuation the trailing
/// user turn holds ONLY tool results (no plain text), so `last_user_text`
/// skips it and walks back to an older prompt; if that older prompt was
/// informational ("what is X"), `user_text_requests_action` returns false
/// and the tool catalog is wrongly cleared — leaving the model with zero
/// tools mid-loop, which it answers with raw `<tool_calls>` XML and a
/// max-token stall. Tools must NEVER be suppressed while tool results are
/// outstanding: the model is continuing work, not starting a new prose Q&A.
fn conversation_is_mid_tool_loop(messages: &[ProviderMessage]) -> bool {
    let Some(last) = messages.last() else {
        return false;
    };
    last.content
        .iter()
        .any(|c| matches!(c, ProviderContent::ToolResult { .. }))
}

fn user_text_requests_action(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    let normalized = lower.split_whitespace().collect::<Vec<_>>().join(" ");
    let trimmed = normalized.trim();
    if trimmed.is_empty() || trimmed.starts_with('/') {
        return false;
    }

    if explicitly_requests_tool_use(trimmed) {
        return true;
    }

    let strong_action_terms = [
        "add",
        "apply",
        "build",
        "change",
        "check",
        "commit",
        "continue",
        "create",
        "debug",
        "delete",
        "do",
        "edit",
        "find",
        "fix",
        "grep",
        "implement",
        "inspect",
        "investigate",
        "look",
        "open",
        "patch",
        "proceed",
        "push",
        "read",
        "remove",
        "run",
        "search",
        "test",
        "trace",
        "update",
        "write",
        // Extended developer verb allowlist:
        "refactor",
        "optimize",
        "reorganize",
        "cleanup",
        "clean",
        "format",
        "lint",
        "compile",
        "audit",
        "review",
        "restructure",
        "verify",
        "profile",
        "revert",
        "stage",
        "merge",
        "pull",
        "clone",
        "analyze",
        "migrate",
        "deploy",
        "install",
        "configure",
        "scaffold",
        "generate",
        "rename",
        "move",
        "copy",
        "replace",
        "extract",
        "inline",
        "split",
    ];
    let has_action_term = trimmed
        .split(' ')
        .any(|word| strong_action_terms.contains(&word));
    if !has_action_term {
        return false;
    }

    let informational_prefixes = [
        "what ",
        "why ",
        "how ",
        "explain",
        "tell me",
        "describe",
        "summarize",
    ];
    let starts_informational = informational_prefixes
        .iter()
        .any(|prefix| trimmed.starts_with(prefix));
    if starts_informational {
        let explicit_repo_action_terms = [
            "read",
            "run",
            "trace",
            "debug",
            "investigate",
            "inspect",
            "grep",
            "search",
            "open",
            "fix",
            "implement",
            "edit",
            "change",
            "update",
            "build",
            "test",
        ];
        return trimmed
            .split(' ')
            .any(|word| explicit_repo_action_terms.contains(&word));
    }

    true
}

fn explicitly_requests_tool_use(trimmed: &str) -> bool {
    [
        "use codegraph",
        "use the codegraph",
        "use mcp",
        "use the mcp",
        "use rg",
        "use ripgrep",
        "use grep",
        "use bash",
        "use shell",
        "use terminal",
        "use tool",
        "use tools",
        "use the tool",
        "use the tools",
    ]
    .iter()
    .any(|needle| trimmed.contains(needle))
}

fn anthropic_tool_choice_value(_choice: StreamToolChoice) -> serde_json::Value {
    serde_json::json!({ "type": "auto" })
}

pub(super) async fn prepare_stream_request(
    provider: Arc<dyn Provider>,
    messages: &[ProviderMessage],
    model: &ModelId,
    overrides: StreamRequestOverrides,
) -> PreparedStreamRequest {
    let cwd = std::env::current_dir()
        .ok()
        .and_then(|p| p.to_str().map(str::to_owned))
        .unwrap_or_default();

    // Build prompt sections (matching Claude Code's structure)
    let skills_listing = if let Ok(cwd_path) = std::env::current_dir() {
        let skills = crate::agents::load_skills(&cwd_path);
        let block = crate::agents::render_skills_section(&skills);
        if block.is_empty() {
            String::new()
        } else {
            format!(
                "{block}\nTo use a listed skill, call the Skill tool with \
                 `name` set to the listed skill name and optional `args` for \
                 extra context. On OpenAI-compatible routes the callable may \
                 be advertised as lowercase `skill`; use the exact callable \
                 name shown in the tool list."
            )
        }
    } else {
        String::new()
    };

    // Auto-dispatch nudge — surfaces every agent's `keyTrigger` to
    // the leader so the model proactively fires Explore / Plan /
    // verification without the user having to ask. v132 + oh-my-
    // opencode parity: "Default Bias: DELEGATE" + Intent → Dispatch
    // routing table. Only the built-in agents are consulted here;
    // user-defined `.claude/agents/*.md` already merge into the same
    // list via `load_all_agents`, so their `keyTrigger` frontmatter
    // also takes effect.
    let dispatch_section = {
        let cwd_for_agents =
            std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
        let agents = crate::agents::load_agents(&cwd_for_agents);
        crate::agents::render_dispatch_section(&agents)
    };

    let diagnostics_block = {
        let diags = crate::diagnostics::global_snapshot();
        crate::diagnostics::render_for_prompt(&diags).unwrap_or_default()
    };

    let tool_guidance = "\
## Using your tools\n\
Prefer dedicated tools over Bash when one fits (Read, Write, Edit, Glob, Grep) — reserve Bash for shell-only operations.\n\
\n\
### Code navigation — reach for the graph FIRST\n\
The workspace is indexed into a code graph (auto-built for whatever directory you're in — it is NOT specific to this project). For anything about *code structure*, the graph tools are faster and more precise than grep/Read, and they return exact `file:start-end` ranges. Use this routing:\n\
- **Find a symbol by name** (function, struct, enum, trait, type) → `graph_search` (add `include_code=true` to get the body inline — this replaces the search-then-Read/sed dance). Do NOT grep for an identifier like `SalesforceApi` or `from_sf_cli`; `graph_search` resolves it in one call and never needs regex-guessing.\n\
- **\"How does X work\" / understand an area / a bug's blast radius** → `graph_context`.\n\
- **Who calls this / what does it call** → `graph_callers` / `graph_callees` (never grep for call sites).\n\
- **Impact of changing a symbol** → `graph_impact`.\n\
- **A file's symbol map** (instead of reading the whole file or `nl`) → `graph_outline`.\n\
- **One symbol's signature/body** → `graph_node`; **several related ones at once** → `graph_explore`.\n\
- **A string the graph can't index** (log message, error text, config key, comment) → `graph_grep` (regex content search that also tells you the enclosing function), or plain `Grep` for non-code files.\n\
Reserve **Read** for a file you're about to edit or a non-source file; reserve **Grep** for literal/non-identifier text. When you do Read a large source file for one symbol, pass `offset`/`limit` (from a `graph_search`/`graph_outline` range) instead of reading the whole thing.\n\
\n\
Only use tools to complete tasks. All text you output outside of tool use is displayed to the user; tools are how you take action. Never use Bash echo or code comments as a way to communicate with the user during the session.\n\
\n\
CRITICAL: LEADING CONVERSATIONAL PROSE IS STRICTLY FORBIDDEN during tool execution turns. \
You must NOT explain what you are about to do. Do NOT write preambles like 'Sure, let me check the files.' \
or 'I will run a grep search now.' Call the appropriate tool immediately in your very first token. \
You are only allowed to output conversational prose when answering an informational question, \
or when the task is fully completed and you are presenting the final results to the user.\n\n\
You can call multiple tools in a single response. If you intend to call multiple tools and there are no dependencies between the calls, make all of the independent calls in the same block, otherwise you MUST wait for previous calls to finish first to determine the dependent values (do NOT use placeholders or guess missing parameters).\n\
If the user provides a specific value for a parameter (for example provided in quotes), make sure to use that value EXACTLY. DO NOT make up values for or ask about optional parameters.\n\
When reporting results, be accurate about what you verified vs. what you assumed. Distinguish between what you confirmed (ran a command, read a file) and what you believe but did not check. Do not assert assumptions as facts.";

    let coding_instructions = "\
## Doing tasks\n\
The user will primarily request software engineering tasks. When given an unclear or generic instruction, consider it in the context of software engineering and the current working directory.\n\
You are highly capable and often allow users to complete ambitious tasks that would otherwise be too complex or take too long. Defer to user judgement about whether a task is too large.\n\
For exploratory questions, respond in 2-3 sentences with a recommendation and the main tradeoff. Don't implement until the user agrees.\n\
Prefer editing existing files to creating new ones.\n\
Be careful not to introduce security vulnerabilities (command injection, XSS, SQL injection). Prioritize writing safe, secure, and correct code.\n\
Don't add features, refactor, or introduce abstractions beyond what the task requires. Three similar lines is better than a premature abstraction.\n\
Don't add error handling or validation for scenarios that can't happen. Trust internal code and framework guarantees. Only validate at system boundaries.\n\
Default to writing no comments. Only add one when the WHY is non-obvious: a hidden constraint, a subtle invariant, a workaround for a specific bug.\n\
When reporting results, be accurate about what you verified vs. what you assumed. Distinguish between what you confirmed (ran a command, read a file) and what you believe but did not check.";

    let safety_instructions = "\
## Executing actions with care\n\
Read, search, and investigate freely — looking is not acting. For actions that are hard to reverse, affect shared systems, or are otherwise risky (deleting data, force-pushing, sending messages, modifying shared infrastructure), confirm with the user before proceeding unless durably authorized. Approval in one context doesn't extend to the next.\n\
When you encounter an obstacle, do not use destructive actions as a shortcut. Try to identify root causes rather than bypassing safety checks. If you discover unexpected state like unfamiliar files or branches, investigate before deleting or overwriting — it may represent in-progress work.";

    let tone_style = "\
## Tone and style\n\
Only use emojis if the user explicitly requests it.\n\
Your responses should be short and concise.\n\
When referencing specific functions or pieces of code include the pattern file_path:line_number to allow the user to easily navigate to the source.\n\
Do not use a colon before tool calls.";

    // Measure component sizes for budget breakdown before they're consumed by format!.
    let skills_chars = skills_listing.len();
    let dispatch_chars = dispatch_section.len();
    let diagnostics_chars = diagnostics_block.len();
    let mut system_prompt = format!(
        "You are jfc, a coding assistant running as a CLI in the user's terminal. \
         You have direct access to the user's filesystem and shell via tools \
         (Bash, Read, Write, Edit, Glob, Grep). You also have a code graph \
         indexed over the workspace with tools for symbol search, callers/callees, \
         outlines, and content grep — see 'Code navigation' below. When the user \
         asks you to do something — read a file, run a command, write code — USE \
         the tools to do it directly. Don't describe how the user could do it \
         manually; you are the one doing it. Working directory: {cwd}\n\n\
         ## Task tracking\n\
         For any request with 2 or more distinct steps, use TaskCreate to plan \
         before starting. Call TaskCreate once per step with both `subject` \
         and `description`. \
         Mark each step complete with TaskDone immediately after finishing it — \
         never batch completions. Update a step's description mid-work with \
         TaskUpdate if scope changes. TaskList shows the user your current plan \
         in the sidebar. OpenAI-compatible providers may advertise task tools \
         as lowercase names such as `taskcreate`, `taskdone`, `taskupdate`, \
         and `tasklist`; use the exact callable name shown in the tool list. \
         This is the primary way users track your progress, so use it \
         consistently on all non-trivial work.\n\n\
         ## Available skills\n\n\
         {skills_listing}\n\n\
         {dispatch_section}\n\n\
         ## Current diagnostics\n\n\
         {diagnostics_block}\n\n\
         {tool_guidance}\n\n\
         {coding_instructions}\n\n\
         {safety_instructions}\n\n\
         {tone_style}"
    );

    // v126 CLAUDE.md hierarchy — managed → user → project → .claude/ → local
    // overrides. Each layer is appended with its origin labeled so the model
    // can tell which rule came from which file. We load on every stream call
    // so live edits to CLAUDE.md take effect on the next turn (matching CC).
    let mut overrides = overrides;
    if let Ok(cwd_path) = std::env::current_dir() {
        let hierarchy = crate::context::ClaudeMdHierarchy::load(&cwd_path);
        if let Some(layered) = hierarchy.render() {
            system_prompt.push_str("\n\n");
            system_prompt.push_str(&layered);
        }
        // Extract disallowed-tools from frontmatter and merge with CLI ones.
        let fm_disallowed = hierarchy.collect_disallowed_tools();
        if !fm_disallowed.is_empty() {
            overrides.disallowed_tools.extend(fm_disallowed);
        }

        let memories = crate::memory::load_all_memories(&cwd_path);

        let recall_enabled =
            crate::memory_recall::is_enabled(crate::config::load().memory_recall_enabled);
        let mut recall_block: Option<String> = None;
        if recall_enabled && !memories.is_empty() {
            let last_user_query = last_user_text(messages);
            if let Some(query) = last_user_query {
                let trimmed = query.trim();
                if !trimmed.is_empty() && !trimmed.starts_with('/') {
                    recall_block = crate::memory_recall::run_recall(
                        trimmed,
                        &memories,
                        provider.clone(),
                        model.clone(),
                    )
                    .await;
                }
            }
        }

        if let Some(ref b) = recall_block {
            tracing::debug!(
                target: "jfc::stream",
                recall_block_len = b.len(),
                "using memory recall block (skipping full memory dump)"
            );
            system_prompt.push_str(b);
        } else if let Some(memories_section) = crate::memory::render_memories_section(&memories) {
            system_prompt.push_str(&memories_section);
        }

        // Plan recall — parallel to memory recall above. Two-phase LLM call
        // selects relevant plans from `.jfc/plans/` then synthesizes a
        // `<system-reminder>` context block. Same gating logic as memory
        // recall: skip on empty plan set, slash commands, or when disabled
        // via env var / runtime override / persisted config.
        let plan_recall_enabled =
            crate::plan_recall::is_enabled(crate::config::load().plan_recall_enabled);
        if plan_recall_enabled {
            if let Ok(plan_store) = crate::plan::PlanStore::open_project(Some(&cwd_path)) {
                let plans = plan_store.list(None);
                if !plans.is_empty() {
                    let last_user_query = last_user_text(messages);
                    if let Some(query) = last_user_query {
                        let trimmed = query.trim();
                        if !trimmed.is_empty() && !trimmed.starts_with('/') {
                            // `run_plan_recall` handles its own caching via
                            // `plan_recall::cache_recall` — repeated turns with
                            // the same query reuse the prior synthesis.
                            if let Some(block) = crate::plan_recall::run_plan_recall(
                                trimmed,
                                &plans,
                                provider.clone(),
                                model.clone(),
                            )
                            .await
                            {
                                tracing::debug!(
                                    target: "jfc::stream",
                                    plan_recall_block_len = block.len(),
                                    "appending plan recall block"
                                );
                                system_prompt.push_str(&block);
                            }
                        }
                    }
                }
            }
        }

        // t221 — AutoSearchHints: scan the user's prompt for code-path /
        // symbol mentions and inject a recall hint block built from project
        // + user memory. Parallels the memory_recall / plan_recall hooks
        // but is local (no LLM call) and always cheap to run.
        if let Some(last_user_query) = last_user_text(messages) {
            let trimmed = last_user_query.trim();
            if !trimmed.is_empty() && !trimmed.starts_with('/') {
                if let Some(hint_block) =
                    jfc_learn::auto_hints::run_pre_turn_hint(trimmed, &cwd_path)
                {
                    tracing::debug!(
                        target: "jfc::stream",
                        hint_block_len = hint_block.len(),
                        "injecting auto-hint recall block"
                    );
                    system_prompt.push_str("\n\n");
                    system_prompt.push_str(&hint_block);
                }
            }
        }

        if let Some(block) = crate::tools::render_pending_auto_context(&cwd_path) {
            system_prompt.push_str(&block);
        }

        let git_ctx = crate::git_context::get_git_context();
        if git_ctx.current_branch.is_some() || !git_ctx.recent_commits.is_empty() {
            system_prompt.push_str("\n\n");
            system_prompt.push_str(&git_ctx.to_prompt_string());
        }

        if let Some(env_block) = crate::env_context::get().to_prompt_string() {
            system_prompt.push_str(&env_block);
        }
    }

    if let Some(gates) = crate::feature_gates::system_prompt_section() {
        system_prompt.push_str(&gates);
    }

    let doc_cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    if let Some(doc_rules) = crate::document_formats::system_prompt_section(&doc_cwd) {
        system_prompt.push_str("\n\n");
        system_prompt.push_str(&doc_rules);
    }

    if crate::feature_gates::is_enabled(crate::feature_gates::FeatureGate::Marsh) {
        let chunks = crate::feature_gates::marsh_drain();
        if !chunks.is_empty() {
            let body = chunks.join("\n");
            let preview: String = body.chars().take(8_000).collect();
            system_prompt.push_str(&format!(
                "\n\n{}",
                crate::system_reminder::format(&format!(
                    "Bash subprocess output captured since last turn:\n```\n{preview}\n```"
                ))
            ));
        }
    }

    if crate::feature_gates::is_enabled(crate::feature_gates::FeatureGate::Harrier) {
        system_prompt.push_str(
            "\n\n## Investigate before asking\n\
             When the user's request is concrete and bounded (a specific \
             file, a named symbol, a known feature area), spend up to ~1 \
             minute on read-only investigation (Read / Grep / Glob / git \
             log) **before** asking a clarifying question. The user almost \
             always prefers a self-answered question over a back-and-forth \
             — they brought the question to you to save themselves the \
             investigation. Only escalate to AskUserQuestion when the \
             investigation surfaces multiple incompatible interpretations \
             that would meaningfully change the plan.",
        );
    }

    if let Some(suffix) = crate::output_style::active().system_prompt_suffix() {
        system_prompt.push_str(suffix);
        tracing::debug!(
            target: "jfc::stream",
            style = %crate::output_style::active().name(),
            "appended OutputStyle suffix to system prompt"
        );
    }

    // Drain queued background reminders (file watcher / MCP refresh / …)
    // into this request's system prompt. The reminders were posted by
    // FS-event handlers and live wire-only — they never persist in
    // `app.messages`, so re-issuing or compacting the conversation
    // doesn't re-show them. Each reminder is wrapped in the canonical
    // `<system-reminder>` envelope so the model treats it as background
    // context, not a user instruction.
    if !overrides.background_reminders.is_empty() {
        tracing::debug!(
            target: "jfc::stream",
            count = overrides.background_reminders.len(),
            "appending background reminders to system prompt"
        );
        for body in &overrides.background_reminders {
            system_prompt.push_str("\n\n");
            system_prompt.push_str(&crate::system_reminder::format(body));
        }
    }

    // Inject the last session's handoff summary so the model knows where
    // the previous session left off. Only on the first request per session
    // (handoff is static context).
    if let Some(root) = crate::context::discover_git_root()
        && let Some(handoff) = crate::sprint::HandoffSummary::read_latest(&root)
    {
        let truncated: String = handoff.chars().take(4000).collect();
        system_prompt.push_str("\n\n## Previous Session Handoff\n");
        system_prompt.push_str(&truncated);
    }

    // Temporal awareness is now fully implemented in
    // stream/messages/provider_messages.rs — time gap markers (<!-- +Nm -->)
    // are prepended to user messages when the gap exceeds 1 minute.

    // NOTE: Sprint budget injection was removed from here because the
    // char-based token estimate (system_prompt.len()/4 + messages.len()/4)
    // massively overestimates on large system prompts, causing false warnings
    // at 15% actual utilization. The real sprint budget is injected via the
    // CLAUDE.md system prompt section which uses actual API-reported token
    // counts from `app.last_usage_input` / `app.max_context_tokens`.

    let thinking_mode = thinking_mode_for(model.as_str());
    tracing::debug!(
        target: "jfc::stream::budget",
        skills_chars,
        dispatch_chars,
        diagnostics_chars,
        total_system_chars = system_prompt.len(),
        estimated_tokens = system_prompt.len() / 4,
        "system prompt budget breakdown"
    );
    tracing::info!(
        target: "jfc::stream",
        model = %model,
        has_thinking_support = thinking_mode.has_thinking_support(),
        supports_adaptive = thinking_mode.supports_adaptive(),
        system_prompt_len = system_prompt.len(),
        tool_count = tools::all_tool_defs().len(),
        "preparing stream request"
    );
    let system_prompt_tokens = system_prompt.len() / 4;
    let max_out = max_output_tokens_for(model.as_str());
    let mut advertised_tools = tools::all_tool_defs_with_mcp().await;

    #[cfg(feature = "permission-automation")]
    {
        let cwd_for_perms =
            std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
        let cfg = crate::config::feature_config::FeatureConfig::load(&cwd_for_perms);
        let rules = crate::permissions::RuleSet::from_config(&cfg);
        let before = advertised_tools.len();
        let mut suppressed: Vec<String> = Vec::new();
        advertised_tools.retain(|t| {
            let decision = crate::permissions::check_tool_permission(&rules, &t.name, None);
            if matches!(decision.action, crate::permissions::PermissionAction::Deny) {
                suppressed.push(t.name.clone());
                false
            } else {
                true
            }
        });
        if !suppressed.is_empty() {
            tracing::info!(
                target: "jfc::stream::permissions",
                suppressed_count = suppressed.len(),
                tools = ?suppressed,
                "pre-flight: suppressed denied tools from catalog"
            );
            system_prompt.push_str(&format!(
                "\n\n## Tools suppressed by policy\n\nThe following tools \
                 are denied by `.jfc/permissions.toml` and are NOT available \
                 this session: {}.\n",
                suppressed.join(", "),
            ));
        }
        let _ = before;
    }

    // Filter out tools disallowed by CLI flags and/or CLAUDE.md frontmatter.
    if !overrides.disallowed_tools.is_empty() {
        let before = advertised_tools.len();
        let disallowed_lower: Vec<String> = overrides
            .disallowed_tools
            .iter()
            .map(|t| t.to_lowercase())
            .collect();
        let mut suppressed: Vec<String> = Vec::new();
        advertised_tools.retain(|t| {
            if disallowed_lower.contains(&t.name.to_lowercase()) {
                suppressed.push(t.name.clone());
                false
            } else {
                true
            }
        });
        if !suppressed.is_empty() {
            tracing::info!(
                target: "jfc::stream::tools",
                removed = suppressed.len(),
                total_before = before,
                tools = ?suppressed,
                "removed disallowed tools from catalog"
            );
        }
    }

    // A post-tool continuation re-sends the conversation with the trailing
    // user turn carrying only tool_result blocks. The model is mid-loop and
    // MUST keep its tools — treat that as action-expected regardless of what
    // the last *text* prompt looked like, otherwise the catalog is stripped
    // and the model emits raw <tool_calls> XML until it hits max tokens.
    let mid_tool_loop = conversation_is_mid_tool_loop(messages);
    let action_expected = mid_tool_loop
        || last_user_text(messages)
            .as_deref()
            .map(user_text_requests_action)
            .unwrap_or(false);
    if !action_expected && !advertised_tools.is_empty() {
        tracing::debug!(
            target: "jfc::stream::tools",
            tool_count = advertised_tools.len(),
            "suppressing tool catalog for non-action prompt"
        );
        advertised_tools.clear();
    }
    let advertised_tool_count = advertised_tools.len();

    let mut base = StreamOptions::new(model.clone())
        .system(system_prompt)
        .tools(advertised_tools)
        .max_tokens(max_out);
    if matches!(
        provider.stream_convention(),
        StreamConvention::AnthropicNative
    ) && !base.tools.is_empty()
    {
        base.provider_options.insert(
            "tool_choice".to_owned(),
            anthropic_tool_choice_value(overrides.tool_choice),
        );
    }
    if let Some(effort) = crate::effort::active_global() {
        base = base.reasoning_effort(effort);
    }
    if crate::effort::active_fast_mode() {
        base = base.fast_mode(true);
    }

    let opts = thinking_mode.apply_to(base);

    PreparedStreamRequest {
        opts,
        system_prompt_tokens,
        metadata: StreamRequestMetadata {
            advertised_tool_count,
            action_expected,
            tool_choice: overrides.tool_choice,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::{conversation_is_mid_tool_loop, user_text_requests_action};
    use jfc_provider::{ProviderContent, ProviderMessage, ProviderRole};

    fn user_text(s: &str) -> ProviderMessage {
        ProviderMessage {
            role: ProviderRole::User,
            content: vec![ProviderContent::Text(s.into())],
        }
    }

    fn user_tool_result(id: &str) -> ProviderMessage {
        ProviderMessage {
            role: ProviderRole::User,
            content: vec![ProviderContent::ToolResult {
                tool_use_id: id.into(),
                content: "ok".into(),
                is_error: false,
            }],
        }
    }

    // Normal: a post-tool continuation ends with a user message carrying
    // tool_result blocks — that's mid-loop, so tools must stay advertised.
    #[test]
    fn mid_tool_loop_detected_on_trailing_tool_result_normal() {
        let msgs = vec![
            user_text("what is ownership in rust"),
            user_tool_result("toolu_1"),
        ];
        assert!(conversation_is_mid_tool_loop(&msgs));
    }

    // Robust: a conversation whose last turn is plain user text is NOT
    // mid-loop — the normal action-intent heuristic governs tool suppression.
    #[test]
    fn plain_trailing_user_text_is_not_mid_loop_robust() {
        let msgs = vec![user_text("explain how borrowing works")];
        assert!(!conversation_is_mid_tool_loop(&msgs));
    }

    // Robust: empty conversation is never mid-loop.
    #[test]
    fn empty_conversation_is_not_mid_loop_robust() {
        assert!(!conversation_is_mid_tool_loop(&[]));
    }

    #[test]
    fn action_intent_detects_toolish_prompts_normal() {
        assert!(user_text_requests_action("read the file and trace the bug"));
        assert!(user_text_requests_action("continue please thank you"));
        assert!(user_text_requests_action("do all of the fixes please"));
        assert!(user_text_requests_action(
            "why is this bug happening read this session"
        ));
        assert!(user_text_requests_action(
            "what do you think of this codebase use codegraph and stuff"
        ));
        assert!(user_text_requests_action(
            "explain the architecture and use codegraph"
        ));
    }

    #[test]
    fn action_intent_leaves_plain_questions_alone_robust() {
        assert!(!user_text_requests_action("what is ownership in rust?"));
        assert!(!user_text_requests_action("explain how borrowing works"));
        assert!(!user_text_requests_action(
            "what is the use of lifetimes in rust?"
        ));
        assert!(!user_text_requests_action("this is pretty wild right"));
        assert!(!user_text_requests_action("/help"));
    }
}

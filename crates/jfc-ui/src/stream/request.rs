use std::sync::Arc;

use crate::provider::{
    ModelId, Provider, ProviderContent, ProviderMessage, ProviderRole, StreamOptions,
};
use crate::tools;

use super::model_policy::{max_output_tokens_for, thinking_mode_for};

pub(super) struct PreparedStreamRequest {
    pub(super) opts: StreamOptions,
    pub(super) system_prompt_tokens: usize,
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

pub(super) async fn prepare_stream_request(
    provider: Arc<dyn Provider>,
    messages: &[ProviderMessage],
    model: &ModelId,
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
You can call multiple tools in a single response. If you intend to call multiple tools and there are no dependencies between the calls, make all of the independent calls in the same block, otherwise you MUST wait for previous calls to finish first to determine the dependent values (do NOT use placeholders or guess missing parameters).\n\
If the user provides a specific value for a parameter (for example provided in quotes), make sure to use that value EXACTLY. DO NOT make up values for or ask about optional parameters.\n\
\n\
**Tool calls are ground truth.** Never claim that you wrote a file, ran a command, deployed something, edited code, sent a message, or executed any other side-effect unless a successful tool call THIS TURN proves it (Write/Edit/Bash/SendMessage/TeamCreate/etc.). If you intend to perform such an action, you MUST emit the corresponding tool call — describing the action in prose does NOT execute it. If you cannot or did not call the tool, say so explicitly (e.g. \"I haven't written the file yet; calling Write now…\") and then issue the call. Phrases like \"Done\", \"wrote\", \"created\", \"updated\", \"deployed\", \"executed\", \"applied the patch\", or \"now writing for real\" are forbidden unless backed by a completed tool call in the current turn.";

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
         (Bash, Read, Write, Edit, Glob, Grep). When the user asks you to do \
         something — read a file, run a command, write code — USE the tools to \
         do it directly. Don't describe how the user could do it manually; you \
         are the one doing it. Working directory: {cwd}\n\n\
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
    if let Ok(cwd_path) = std::env::current_dir() {
        let hierarchy = crate::context::ClaudeMdHierarchy::load(&cwd_path);
        if let Some(layered) = hierarchy.render() {
            system_prompt.push_str("\n\n");
            system_prompt.push_str(&layered);
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

    let mut base = StreamOptions::new(model.clone())
        .system(system_prompt)
        .tools(advertised_tools)
        .max_tokens(max_out);
    if let Some(effort) = crate::effort::active_global() {
        base = base.reasoning_effort(effort);
    }
    if crate::effort::active_fast_mode() {
        base = base.fast_mode(true);
    }

    PreparedStreamRequest {
        opts: thinking_mode.apply_to(base),
        system_prompt_tokens,
    }
}

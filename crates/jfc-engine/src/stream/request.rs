use std::sync::Arc;

use crate::runtime::{StreamRequestMetadata, StreamRequestOverrides, StreamToolChoice};
use crate::tools;
use jfc_provider::{
    DEFAULT_MAX_OUTPUT_TOKENS, ModelId, ModelRequestPolicy, ModelRequestProfile,
    ModelResolutionReason, ModelSpec, Provider, ProviderContent, ProviderId, ProviderMessage,
    ProviderRole, ResolvedModel, StreamConvention, StreamOptions,
};

pub struct PreparedStreamRequest {
    pub opts: StreamOptions,
    pub system_prompt_tokens: usize,
    pub metadata: StreamRequestMetadata,
    /// Byte length of the memory-recall block injected into the system prompt
    /// this turn (0 = no recall). Surfaced to the user as "recalled memory".
    pub recalled_memory_chars: usize,
}

fn normalize_thinking_display(value: &str) -> Option<&'static str> {
    match value.trim().to_ascii_lowercase().as_str() {
        "show" | "shown" | "visible" | "on" | "true" | "summary" | "summarize" | "summarized" => {
            Some("summarized")
        }
        "hide" | "hidden" | "off" | "false" | "omit" | "omitted" => Some("omitted"),
        _ => None,
    }
}

fn requested_thinking_display(overrides: &StreamRequestOverrides) -> Option<String> {
    if let Some(value) = overrides.thinking_display.as_deref() {
        return match normalize_thinking_display(value) {
            Some(display) => Some(display.to_owned()),
            None => {
                tracing::warn!(
                    target: "jfc::stream",
                    value,
                    "ignoring unsupported thinking display mode"
                );
                None
            }
        };
    }
    std::env::var("JFC_THINKING_DISPLAY")
        .ok()
        .and_then(|value| normalize_thinking_display(&value).map(str::to_owned))
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

    // Questions about the user's *local* machine or repo state need tools to
    // answer truthfully even when phrased informationally ("tell me about my
    // device", "what's installed", "what's in this repo"). These carry no
    // action verb, so the `strong_action_terms` gate below would suppress the
    // whole catalog — the model then can't inspect anything and emulates a
    // tool call as raw `<Bash .../>` text that leaks into the transcript
    // (observed on gpt-5.5 with "tell me about my device"). High-precision
    // deictic/possessive references to local resources keep tools available.
    if references_local_environment(trimmed) {
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

/// Detect prompts that reference the user's concrete local environment —
/// their machine, hardware, or working repo — which can only be answered by
/// inspecting it with tools. Kept high-precision: matches possessive/deictic
/// phrases ("my device", "this repo") and a few unambiguous status questions
/// ("what's installed") rather than bare nouns, so prose questions like
/// "what is the memory model" don't trip it.
fn references_local_environment(trimmed: &str) -> bool {
    const LOCAL_REFERENCES: &[&str] = &[
        // Possessive references to the local machine.
        "my device",
        "my machine",
        "my system",
        "my hardware",
        "my computer",
        "my laptop",
        "my desktop",
        "my workstation",
        "my rig",
        "my setup",
        "my environment",
        "my cpu",
        "my gpu",
        "my ram",
        "my disk",
        "my os",
        "my kernel",
        "my specs",
        // Possessive/deictic references to the working repo.
        "my repo",
        "my project",
        "my codebase",
        "this machine",
        "this device",
        "this system",
        "this computer",
        "this repo",
        "this project",
        "this codebase",
        "this directory",
        "this folder",
        "this crate",
        "this package",
        // Status questions that require inspecting the local environment.
        "system specs",
        "hardware specs",
        "what's installed",
        "whats installed",
        "what is installed",
        "what's running",
        "whats running",
        "what is running",
    ];
    LOCAL_REFERENCES
        .iter()
        .any(|needle| trimmed.contains(needle))
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
        "tool call",
        "tool calls",
        "websearch",
        "web search",
        "available tools",
        "tools available",
        "backends i have",
        "backend i have",
        "search backends",
        "search backend",
        "primo:",
    ]
    .iter()
    .any(|needle| trimmed.contains(needle))
}

fn preserve_non_action_tool(tool_name: &str) -> bool {
    // CodeGraph (read-only MCP code navigation) stays available on
    // informational turns: "how does X work" is exactly the question the
    // system prompt tells the model to answer with codegraph_explore. The
    // old behavior stripped every MCP tool here, which contradicted that
    // guidance and trained the model to answer structure questions from
    // memory or punt to Read/Bash on the next action turn.
    matches!(tool_name, "ToolSearch" | "ToolSuggest" | "SendUserMessage")
        || crate::tools::is_code_navigation_tool_name(tool_name)
}

fn anthropic_tool_choice_value(_choice: StreamToolChoice) -> serde_json::Value {
    serde_json::json!({ "type": "auto" })
}

/// Pick a fast/cheap model for the pre-flight memory/plan recall. Recall is a
/// trivial select→extract classification, so running it on the main model
/// (e.g. opus) is needlessly slow and expensive — a haiku-class model does it
/// in a fraction of the time, which is the bigger half of cold-recall latency.
/// Uses the provider's own haiku model when it advertises one; otherwise falls
/// back to the main model (recall then behaves exactly as before, and still
/// degrades to a full memory dump if it errors). Provider-aware, so it picks
/// the right haiku id for Anthropic/Bedrock/Vertex and no-ops for providers
/// (e.g. OpenWebUI) that don't offer one.
fn fast_recall_model(provider: &Arc<dyn Provider>, main: &ModelId) -> ModelId {
    provider
        .available_models()
        .into_iter()
        .find(|m| m.id.as_str().contains("haiku"))
        .map(|m| m.id)
        .unwrap_or_else(|| main.clone())
}

fn truncate_chars(text: &str, max_chars: usize) -> String {
    let mut iter = text.chars();
    let mut out: String = iter.by_ref().take(max_chars).collect();
    if iter.next().is_some() {
        out.push_str("\n...[truncated]");
    }
    out
}

async fn mcp_server_instructions_section() -> String {
    const MAX_SERVERS: usize = 8;
    const MAX_CHARS_PER_SERVER: usize = 6_000;
    const MAX_TOTAL_CHARS: usize = 18_000;

    let Some(registry) = tools::snapshot_mcp_registry() else {
        return String::new();
    };
    let entries = registry.all_server_instructions().await;
    if entries.is_empty() {
        return String::new();
    }

    let mut out = String::from(
        "## MCP Server Instructions\n\n\
         Connected MCP servers provided these usage instructions during the \
         `initialize` handshake. Follow the instructions for a server when \
         using tools from that server.\n",
    );
    let mut used = out.chars().count();
    let mut included = 0usize;
    for (name, instructions) in entries.into_iter().take(MAX_SERVERS) {
        let body = truncate_chars(&instructions, MAX_CHARS_PER_SERVER);
        let block = format!("\n### {name}\n{body}\n");
        let block_chars = block.chars().count();
        if used + block_chars > MAX_TOTAL_CHARS {
            out.push_str("\n...[additional MCP instructions omitted]\n");
            break;
        }
        out.push_str(&block);
        used += block_chars;
        included += 1;
    }

    if included == 0 { String::new() } else { out }
}

/// Render the connected MCP servers' behavior-affecting tool metadata
/// (annotation hints + titles) into a prompt section. Delegates the rendering
/// to the registry, which owns the metadata; this just snapshots the registry
/// and caps total size.
async fn mcp_tool_metadata_section() -> String {
    const MAX_TOTAL_CHARS: usize = 8_000;
    let Some(registry) = tools::snapshot_mcp_registry() else {
        return String::new();
    };
    let section = registry.tool_metadata_prompt_section().await;
    truncate_chars(&section, MAX_TOTAL_CHARS)
}

pub async fn prepare_stream_request(
    provider: Arc<dyn Provider>,
    messages: &[ProviderMessage],
    model: &ModelId,
    overrides: StreamRequestOverrides,
) -> PreparedStreamRequest {
    // Filled in below if a memory-recall block is injected this turn; surfaced
    // to the user. Declared at function scope so it outlives the recall block.
    let mut recalled_memory_chars = 0usize;
    let cwd = std::env::current_dir()
        .ok()
        .and_then(|p| p.to_str().map(str::to_owned))
        .unwrap_or_default();

    // Build prompt sections (matching Claude Code's structure)
    let skills_listing = if let Ok(cwd_path) = std::env::current_dir() {
        crate::prompt_context_cache::skills_listing(&cwd_path)
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
        crate::prompt_context_cache::dispatch_section(&cwd_for_agents)
    };

    let diagnostics_block = {
        let diags = crate::diagnostics::global_snapshot();
        crate::diagnostics::render_for_prompt(&diags).unwrap_or_default()
    };

    let tool_guidance = "\
## Using your tools\n\
Prefer dedicated tools over Bash when one fits (Read, Write, Edit, Glob, Grep) — reserve Bash for shell-only operations.\n\
\n\
### Tool discovery — specialized tools are progressive\n\
To keep the prompt small, only core tools plus intent-matched tools are advertised at the start of an action turn. \
If you need a capability that is not in the visible tool list, call `ToolSearch` or `ToolSuggest` with the capability name. \
Tools returned by those discovery calls are advertised on the next continuation, so you can invoke the exact matching tool after the result arrives. \
Explicit managed/user allowlists still override this and expose only the allowed tools.\n\
\n\
### Code navigation — reach for CodeGraph FIRST\n\
The workspace may be indexed into a CodeGraph MCP code graph. For anything about *code structure*, CodeGraph tools are faster and more precise than Grep/Read and should be your first lookup. Use the exact visible tool name shown in the catalog: MCP hosts usually expose names like `mcp__codegraph__codegraph_explore`, `mcp__codegraph__codegraph_search`, and `mcp__codegraph__codegraph_node`; raw MCP names are `codegraph_explore`, `codegraph_search`, and `codegraph_node`.\n\
- **\"How does X work\" / understand an area / bug blast radius** → `codegraph_explore`.\n\
- **Find a symbol by name** (function, struct, enum, trait, type) → `codegraph_search` (ask for code inline when the schema supports it). Do NOT grep for an identifier like `SalesforceApi` or `from_sf_cli`; CodeGraph resolves it in one call and never needs regex-guessing.\n\
- **Who calls this / what does it call** → `codegraph_callers` / `codegraph_callees`.\n\
- **Impact of changing a symbol** → `codegraph_impact`.\n\
- **A file's symbol map** (instead of reading the whole file or `nl`) → `codegraph_files`.\n\
- **One symbol's signature/body** → `codegraph_node`; **several related ones at once** → `codegraph_explore`.\n\
Use **Read** mainly for a file you are about to edit, a precise range CodeGraph identified, or a non-source file. Use **Grep** mainly for literal strings the graph cannot index, such as log messages, config keys, comments, or non-code files. Do not start coding tasks with a broad file-reading survey when one CodeGraph query can identify the relevant symbols.\n\
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
Read, search, and investigate as needed — looking is not acting, but keep exploration proportionate to the edit. For straightforward coding tasks, make one targeted CodeGraph/search pass, then edit; do not survey many files first unless the first result shows the change crosses modules. For actions that are hard to reverse, affect shared systems, or are otherwise risky (deleting data, force-pushing, sending messages, modifying shared infrastructure), confirm with the user before proceeding unless durably authorized. Approval in one context doesn't extend to the next.\n\
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
         indexed over the workspace when CodeGraph MCP is connected, with tools \
         for source-aware exploration, symbol search, callers/callees, impact, \
         and file maps — see 'Code navigation' below. When the user \
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
    let mcp_instructions = mcp_server_instructions_section().await;
    if !mcp_instructions.is_empty() {
        system_prompt.push_str("\n\n");
        system_prompt.push_str(&mcp_instructions);
    }

    // Behavior-affecting MCP tool metadata (read-only/destructive/idempotent
    // hints + titles) so the model can tell which MCP tools are safe to call
    // freely vs. need confirmation. Only tools carrying actionable annotations
    // appear, so this stays empty for servers that advertise none.
    let mcp_tool_metadata = mcp_tool_metadata_section().await;
    if !mcp_tool_metadata.is_empty() {
        system_prompt.push_str("\n\n");
        system_prompt.push_str(&mcp_tool_metadata);
    }

    // v126 CLAUDE.md hierarchy — managed → user → project → .claude/ → local
    // overrides. Each layer is appended with its origin labeled so the model
    // can tell which rule came from which file. A short-lived cache prevents
    // agentic-loop continuations from repeatedly rereading the same files.
    let mut overrides = overrides;
    if let Ok(cwd_path) = std::env::current_dir() {
        let hierarchy =
            crate::prompt_context_cache::context_hierarchy(&cwd_path, &overrides.extra_dirs);
        if let Some(layered) = hierarchy.rendered {
            system_prompt.push_str("\n\n");
            system_prompt.push_str(&layered);
        }
        // Extract disallowed-tools from frontmatter and merge with CLI ones.
        let fm_disallowed = hierarchy.disallowed_tools;
        if !fm_disallowed.is_empty() {
            overrides.disallowed_tools.extend(fm_disallowed);
        }

        let memories = crate::prompt_context_cache::memories(&cwd_path);

        let config = crate::config::load_arc();
        let recall_enabled = crate::memory_recall::is_enabled(config.memory_recall_enabled);
        let plan_recall_enabled = crate::plan_recall::is_enabled(config.plan_recall_enabled);
        // Run recall on a fast (haiku) model, not the main model — the other
        // half of the cold-recall speedup (alongside running the two recalls
        // concurrently below).
        let recall_model = fast_recall_model(&provider, model);

        // Memory recall and plan recall are INDEPENDENT two-phase LLM
        // round-trips. They used to run as back-to-back `.await`s — up to ~4
        // sequential LLM calls (≈4–12s on a cold cache) blocking the turn
        // before `provider.stream()` ever fires, which is the dominant reason
        // a cold turn lags a thin client. Run them CONCURRENTLY so cold-recall
        // latency is the slower of the two, not their sum. Both are cache
        // hits / no-ops in the steady state. (`tokio::join!` runs them on this
        // one task — no extra threads, shared `&` borrows are fine.)
        let memory_fut = async {
            if recall_enabled
                && !memories.is_empty()
                && let Some(query) = last_user_text(messages)
            {
                let trimmed = query.trim();
                if !trimmed.is_empty() && !trimmed.starts_with('/') {
                    // Cache check BEFORE run_recall so we only fire the
                    // `MemoryRecalled` toast on a fresh (cache-miss) recall,
                    // not on every agentic-loop continuation.
                    let was_cached = crate::memory_recall::cached_recall(trimmed).is_some();
                    let block = crate::memory_recall::run_recall(
                        trimmed,
                        &memories,
                        provider.clone(),
                        recall_model.clone(),
                    )
                    .await;
                    let fresh = !was_cached && block.is_some();
                    return (block, fresh);
                }
            }
            (None, false)
        };
        let plan_fut = async {
            if plan_recall_enabled
                && let Ok(plan_store) = crate::plan::PlanStore::open_project(Some(&cwd_path))
            {
                let plans = plan_store.list(None);
                if !plans.is_empty()
                    && let Some(query) = last_user_text(messages)
                {
                    let trimmed = query.trim();
                    if !trimmed.is_empty() && !trimmed.starts_with('/') {
                        // `run_plan_recall` handles its own caching.
                        return crate::plan_recall::run_plan_recall(
                            trimmed,
                            &plans,
                            provider.clone(),
                            recall_model.clone(),
                        )
                        .await;
                    }
                }
            }
            None
        };
        // Bound the wait: haiku recall almost always finishes well under this,
        // but a network hiccup must never stall the turn. On timeout, proceed
        // with no recall (the full-memory-dump fallback below) — the turn
        // starts; recall just doesn't enrich this one.
        const RECALL_DEADLINE_MS: u64 = 1500;
        let ((recall_block, recall_was_fresh), plan_block) = match tokio::time::timeout(
            std::time::Duration::from_millis(RECALL_DEADLINE_MS),
            async { tokio::join!(memory_fut, plan_fut) },
        )
        .await
        {
            Ok(r) => r,
            Err(_) => {
                tracing::debug!(
                    target: "jfc::stream",
                    deadline_ms = RECALL_DEADLINE_MS,
                    "recall exceeded deadline; proceeding without it this turn"
                );
                ((None, false), None)
            }
        };

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
        // Only set recalled_memory_chars on a fresh recall (cache miss) so
        // the `MemoryRecalled` toast fires once per turn, not once per
        // agentic-loop substream continuation.
        recalled_memory_chars = if recall_was_fresh {
            recall_block.as_ref().map_or(0, |b| b.len())
        } else {
            0
        };
        if let Some(memory_store_section) = sdk_memory_store_prompt_section().await {
            system_prompt.push_str(&memory_store_section);
        }
        if let Some(block) = plan_block {
            tracing::debug!(
                target: "jfc::stream",
                plan_recall_block_len = block.len(),
                "appending plan recall block"
            );
            system_prompt.push_str(&block);
        }

        // t221 — AutoSearchHints: scan the user's prompt for code-path /
        // symbol mentions and inject a recall hint block built from project
        // + user memory. Parallels the memory_recall / plan_recall hooks
        // but is local (no LLM call) and always cheap to run.
        if let Some(last_user_query) = last_user_text(messages) {
            let trimmed = last_user_query.trim();
            if !trimmed.is_empty()
                && !trimmed.starts_with('/')
                && let Some(hint_block) =
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
             file, a named symbol, a known feature area), do a small targeted \
             investigation **only if you would otherwise ask a clarifying \
             question**. Prefer one CodeGraph query or one precise search, \
             then act. Do not use this as permission for a broad Read/Grep/Glob \
             survey before routine edits. Escalate to AskUserQuestion only \
             when that targeted check surfaces multiple incompatible \
             interpretations that would meaningfully change the plan.",
        );
    }

    if let Some(suffix) = crate::output_style::active_suffix(&doc_cwd) {
        system_prompt.push_str(&suffix);
        tracing::debug!(
            target: "jfc::stream",
            style = %crate::output_style::active().name(),
            "appended OutputStyle suffix to system prompt"
        );
    }

    let server_advisor_model = if matches!(
        provider.stream_convention(),
        StreamConvention::AnthropicNative
    ) && matches!(provider.name(), "anthropic" | "anthropic-oauth")
    {
        crate::advisor::active_server_advisor_model()
    } else {
        None
    };
    if let Some(model) = &server_advisor_model {
        tracing::info!(
            target: "jfc::advisor",
            advisor_model = %model,
            "injecting server advisor prompt/tool"
        );
        system_prompt.push_str("\n\n");
        system_prompt.push_str(crate::advisor::SERVER_ADVISOR_SYSTEM_PROMPT);
    }
    let local_advisor_model = crate::advisor::active_local_advisor_model();
    if let Some(model) = &local_advisor_model {
        tracing::info!(
            target: "jfc::advisor",
            advisor_model = %model,
            "injecting local advisor prompt"
        );
        system_prompt.push_str("\n\n## Local Advisor Tool\n\n");
        system_prompt.push_str(
            "You have access to an `Advisor` tool backed by JFC's configured \
             local/client-side advisor model. It takes no parameters. When you \
             call it, JFC snapshots the current conversation, sends that \
             snapshot through the configured advisor provider/model, and returns \
             the advisor's feedback as this tool's result. Call it before \
             substantive work on multi-step tasks, when stuck, when considering \
             a change of approach, and before declaring substantial work done.",
        );
    }

    // Drain queued background reminders (file watcher / MCP refresh / …)
    // into this request's system prompt. The reminders were posted by
    // FS-event handlers and live wire-only — they never persist in
    // `app.engine.messages`, so re-issuing or compacting the conversation
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

    let system_prompt_tokens_before_total_reminder = system_prompt.len() / 4;
    let total_tokens_reminder_mode = overrides
        .total_tokens_reminder_mode
        .unwrap_or_else(crate::total_tokens_reminder::active_mode);
    if let Some(reminder) = crate::total_tokens_reminder::render_for_request_with_mode(
        total_tokens_reminder_mode,
        messages,
        system_prompt_tokens_before_total_reminder,
        overrides.last_usage_input_tokens,
        overrides.context_window_tokens,
    ) {
        system_prompt.push_str("\n\n");
        system_prompt.push_str(&reminder);
    }

    let provider_name = provider.name().to_owned();
    let selected_model_info = provider
        .available_models()
        .into_iter()
        .find(|info| info.id == *model);
    let model_profile = ModelRequestProfile::from_provider_model(
        &provider_name,
        model.as_str(),
        selected_model_info
            .as_ref()
            .and_then(|info| info.context_window_tokens),
        selected_model_info
            .as_ref()
            .and_then(|info| info.max_output_tokens),
    );
    let thinking_mode = model_profile.thinking_mode();
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
    let max_out = model_profile
        .max_output_tokens()
        .unwrap_or(DEFAULT_MAX_OUTPUT_TOKENS);
    let pewter_owl_header = crate::feature_gates::pewter_owl_header_enabled(model.as_str(), false);
    let pewter_owl_tool = crate::feature_gates::pewter_owl_tool_enabled(model.as_str(), false);
    let pewter_owl_brief = crate::feature_gates::pewter_owl_brief_enabled(model.as_str(), false);
    let effective_brief_mode = overrides.brief_mode || pewter_owl_brief;
    if effective_brief_mode {
        system_prompt.push_str(
            "\n\n## Brief User Messages\n\nPlain assistant text is hidden from \
             the main chat view. Put every substantive user-facing reply in \
             `SendUserMessage`; use normal assistant text only for internal \
             reasoning that can be omitted from the user's visible transcript.",
        );
    } else if pewter_owl_tool {
        system_prompt.push_str(
            "\n\n## Pewter Owl Messaging\n\n`SendUserMessage` is available for \
             exact user-visible content between tool calls, such as generated \
             snippets, specific values, and direct replies to mid-task user \
             messages. Routine narration and final answers may remain normal \
             assistant text.",
        );
    }
    let full_tool_catalog = tools::all_tool_defs_with_mcp().await;
    let full_tool_count = full_tool_catalog.len();
    let mut advertised_tools = if overrides.allowed_tools.is_empty() {
        let tool_intent = last_user_text(messages);
        let selected =
            tools::progressive_tool_defs(full_tool_catalog, messages, tool_intent.as_deref());
        tracing::debug!(
            target: "jfc::stream::tools",
            selected = selected.len(),
            full = full_tool_count,
            "selected progressive tool catalog"
        );
        selected
    } else {
        full_tool_catalog
    };
    tools::apply_send_user_message_policy(
        &mut advertised_tools,
        effective_brief_mode,
        pewter_owl_tool,
    );

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

    // Hide JFC's local Advisor tool unless a local advisor model is configured.
    // The upstream server advisor, when active, is injected through
    // StreamOptions instead of the normal local tool catalog.
    if local_advisor_model.is_none() {
        advertised_tools.retain(|t| t.name != "Advisor");
    }

    if !overrides.allowed_tools.is_empty() {
        let before = advertised_tools.len();
        let allowed_lower: Vec<String> = overrides
            .allowed_tools
            .iter()
            .map(|t| t.to_lowercase())
            .collect();
        let mut suppressed: Vec<String> = Vec::new();
        advertised_tools.retain(|t| {
            if allowed_lower.contains(&t.name.to_lowercase()) {
                true
            } else {
                suppressed.push(t.name.clone());
                false
            }
        });
        if !suppressed.is_empty() {
            tracing::info!(
                target: "jfc::stream::tools",
                removed = suppressed.len(),
                total_before = before,
                tools = ?suppressed,
                "removed tools outside allowlist"
            );
            system_prompt.push_str(&format!(
                "\n\n## Tools suppressed by managed/user allowlist\n\nOnly these tools \
                 are available this session: {}.\n",
                overrides.allowed_tools.join(", "),
            ));
        }
    }

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
        let before = advertised_tools.len();
        advertised_tools.retain(|tool| preserve_non_action_tool(&tool.name));
        tracing::debug!(
            target: "jfc::stream::tools",
            before,
            after = advertised_tools.len(),
            "reduced tool catalog for non-action prompt"
        );
    }
    let advertised_tool_names = advertised_tools
        .iter()
        .map(|tool| tool.name.clone())
        .collect::<Vec<_>>();
    if let Some(rules) = crate::review::tool_scoped_prompt_rules(&advertised_tool_names) {
        system_prompt.push_str("\n\n");
        system_prompt.push_str(&rules);
    }
    let system_prompt_tokens = system_prompt.len() / 4;
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
    if let Some(advisor_model) = server_advisor_model {
        base = base.advisor_model(advisor_model);
    }
    if crate::effort::active_fast_mode() {
        base = base.fast_mode(true);
    }
    if pewter_owl_header {
        base = base.narration_summaries(true);
    }
    let thinking_display = requested_thinking_display(&overrides);
    if !overrides.custom_betas.is_empty() {
        base = base.custom_betas(overrides.custom_betas);
    }
    if overrides.fine_grained_tool_streaming
        || std::env::var("JFC_FINE_GRAINED_TOOL_STREAMING")
            .map(|v| matches!(v.as_str(), "1" | "true" | "yes" | "on"))
            .unwrap_or(false)
    {
        base = base.eager_input_streaming(true);
    }
    if overrides.strict_tool_schemas
        || std::env::var("JFC_STRICT_TOOL_SCHEMAS")
            .map(|v| matches!(v.as_str(), "1" | "true" | "yes" | "on"))
            .unwrap_or(false)
    {
        base = base.strict_tool_schemas(true);
    }
    if let Some(tokens) = overrides.task_budget {
        base = base.task_budget(tokens);
    }
    // Forward the post-compaction savings hint so the API's context-management
    // assist (context-hint-2026-04-09) knows how much we just freed. The body
    // builder gates on a >=20k floor (matching cli.js's `2e4`), so a trivial
    // compaction won't emit the hint.
    base.context_hint_tokens_saved = overrides.context_hint_tokens_saved;

    let mut opts = thinking_mode.apply_to(base);
    opts = crate::exploration::apply_to_stream_options(
        opts,
        provider.name(),
        provider.stream_convention(),
    );
    opts = model_profile.clamp_options(opts);
    // Log the resolved request params after per-model clamping so every spawn's
    // actual reasoning_effort, max_tokens, and thinking mode are observable.
    // Critical for experiments comparing model tiers / effort levels — the
    // post-clamp values are what the model actually sees, not the input strings.
    tracing::debug!(
        target: "jfc::stream",
        model = %model,
        reasoning_effort = ?opts.reasoning_effort,
        max_tokens = opts.max_tokens,
        adaptive_thinking = opts.adaptive_thinking,
        thinking_budget = ?opts.thinking_budget,
        "resolved request after clamp_options"
    );
    if let Some(max) = overrides.max_thinking_tokens
        && let Some(budget) = opts.thinking_budget.as_mut()
    {
        *budget = (*budget).min(max);
    }
    if opts.adaptive_thinking || opts.thinking_budget.is_some() {
        let display = thinking_display.unwrap_or_else(|| "summarized".into());
        opts = opts.thinking_display(display);
        // Request server-authoritative thinking token estimates so the spinner
        // can show a live thinking-token chip. Only meaningful when thinking is
        // active; the API otherwise streams thinking_delta without estimates.
        opts = opts.thinking_token_count(true);
    }

    PreparedStreamRequest {
        opts,
        system_prompt_tokens,
        metadata: StreamRequestMetadata {
            advertised_tool_count,
            action_expected,
            tool_choice: overrides.tool_choice,
            resolved_model: Some(ResolvedModel::new(
                ModelSpec::qualified(ProviderId::new(provider.name()), model.clone()),
                ModelSpec::qualified(ProviderId::new(provider.name()), model.clone()),
                ModelResolutionReason::Requested,
                selected_model_info.as_ref(),
            )),
        },
        recalled_memory_chars,
    }
}

async fn sdk_memory_store_prompt_section() -> Option<String> {
    let ids = configured_memory_store_ids();
    if ids.is_empty() {
        return None;
    }
    let Some(client) = crate::sdk_bridge::build_client() else {
        return Some(crate::system_reminder::format(
            "JFC_MEMORY_STORE_IDS is configured, but no Anthropic SDK API key profile is available. \
             Remote SDK memory stores were not loaded for this turn.",
        ));
    };
    let service = jfc_anthropic_sdk::memory_stores::MemoryStoreService::new(client);
    let limit = std::env::var("JFC_MEMORY_STORE_LIMIT")
        .ok()
        .and_then(|v| v.parse::<u32>().ok())
        .unwrap_or(20)
        .clamp(1, 100);
    let timeout = std::env::var("JFC_MEMORY_STORE_TIMEOUT_SECS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .map(std::time::Duration::from_secs)
        .unwrap_or_else(|| std::time::Duration::from_secs(8));

    let mut out = String::from("\n\n## SDK memory stores\n\n");
    let mut loaded_any = false;
    for store_id in ids {
        let params = jfc_anthropic_sdk::pagination::ListParams {
            limit: Some(limit),
            ..Default::default()
        };
        match tokio::time::timeout(timeout, service.list_memories(&store_id, &params)).await {
            Ok(Ok(page)) => {
                loaded_any = true;
                out.push_str(&format!("### {store_id}\n\n"));
                if page.data.is_empty() {
                    out.push_str("(no memories returned)\n\n");
                } else {
                    for memory in page.data {
                        let body = render_sdk_memory_content(&memory);
                        out.push_str(&format!("- `{}`: {}\n", memory.id, body));
                    }
                    out.push('\n');
                }
            }
            Ok(Err(err)) => {
                out.push_str(&format!(
                    "### {store_id}\n\n(remote memory load failed: {err})\n\n"
                ));
            }
            Err(_) => {
                out.push_str(&format!(
                    "### {store_id}\n\n(remote memory load timed out after {}s)\n\n",
                    timeout.as_secs()
                ));
            }
        }
    }

    if loaded_any || !out.trim().is_empty() {
        Some(out)
    } else {
        None
    }
}

fn configured_memory_store_ids() -> Vec<String> {
    let raw = std::env::var("JFC_MEMORY_STORE_IDS")
        .ok()
        .or_else(|| std::env::var("JFC_MEMORY_STORE_ID").ok())
        .unwrap_or_default();
    raw.split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect()
}

fn render_sdk_memory_content(memory: &jfc_anthropic_sdk::memory_stores::Memory) -> String {
    let content = memory
        .content
        .as_ref()
        .map(memory_value_to_text)
        .or_else(|| {
            memory
                .extra
                .get("content")
                .or_else(|| memory.extra.get("text"))
                .or_else(|| memory.extra.get("body"))
                .map(memory_value_to_text)
        })
        .unwrap_or_else(|| "(empty)".to_owned());
    content
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(1_000)
        .collect()
}

fn memory_value_to_text(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Object(map) => {
            for key in ["text", "content", "body", "value"] {
                if let Some(text) = map.get(key).and_then(|v| v.as_str()) {
                    return text.to_owned();
                }
            }
            serde_json::to_string(value).unwrap_or_else(|_| value.to_string())
        }
        _ => serde_json::to_string(value).unwrap_or_else(|_| value.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::{
        conversation_is_mid_tool_loop, prepare_stream_request, preserve_non_action_tool,
        user_text_requests_action,
    };
    use jfc_provider::{
        EventStream, ModelId, ModelInfo, Provider, ProviderContent, ProviderMessage, ProviderRole,
        StreamConvention, StreamOptions,
    };

    struct TestProvider {
        name: &'static str,
        convention: StreamConvention,
    }

    #[async_trait::async_trait]
    impl Provider for TestProvider {
        fn name(&self) -> &str {
            self.name
        }

        fn available_models(&self) -> Vec<ModelInfo> {
            Vec::new()
        }

        fn stream_convention(&self) -> StreamConvention {
            self.convention
        }

        async fn stream(
            &self,
            _messages: Vec<ProviderMessage>,
            _options: &StreamOptions,
        ) -> anyhow::Result<EventStream> {
            Ok(Box::pin(futures::stream::empty()))
        }
    }

    impl jfc_provider::seal::Sealed for TestProvider {}

    struct TemperatureGlobalGuard;

    impl TemperatureGlobalGuard {
        fn set(value: f64) -> Self {
            crate::exploration::set_temperature_global(Some(value));
            Self
        }
    }

    impl Drop for TemperatureGlobalGuard {
        fn drop(&mut self) {
            crate::exploration::set_temperature_global(None);
            crate::exploration::set_exploration_level_global(None);
        }
    }

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
        assert!(user_text_requests_action(
            "see what websearch backends I have right"
        ));
        assert!(user_text_requests_action(
            "use primo please use the tool calls please"
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
        // Bare environment-adjacent nouns in a prose question must NOT trip
        // the local-environment detector — only concrete possessive/deictic
        // references do.
        assert!(!user_text_requests_action("what is the rust memory model"));
        assert!(!user_text_requests_action(
            "explain how the os schedules threads"
        ));
    }

    // REGRESSION (gpt-5.5 "tell me about my device" leaked raw <Bash/> XML):
    // questions about the local machine or repo carry no action verb but must
    // keep tools advertised so the model can actually inspect the system.
    #[test]
    fn action_intent_keeps_tools_for_local_environment_questions_regression() {
        assert!(user_text_requests_action("tell me about my device"));
        assert!(user_text_requests_action("what are my system specs"));
        assert!(user_text_requests_action("describe this machine"));
        assert!(user_text_requests_action("what's installed on here"));
        assert!(user_text_requests_action("tell me about this repo"));
        assert!(user_text_requests_action("what is this codebase"));
    }

    #[test]
    fn non_action_catalog_keeps_discovery_tools_regression() {
        assert!(preserve_non_action_tool("ToolSearch"));
        assert!(preserve_non_action_tool("ToolSuggest"));
        assert!(preserve_non_action_tool("SendUserMessage"));
        assert!(!preserve_non_action_tool("Bash"));
        assert!(!preserve_non_action_tool("Read"));
        assert!(!preserve_non_action_tool("WebFetch"));
    }

    // Regression: informational ("how does X work") turns must keep CodeGraph
    // tools advertised — the system prompt tells the model to answer code
    // structure questions with codegraph_explore, so stripping them here
    // contradicted the prompt and pushed the model back to Read/Bash.
    #[test]
    fn preserve_non_action_keeps_codegraph_tools_regression() {
        assert!(preserve_non_action_tool(
            "mcp__codegraph__codegraph_explore"
        ));
        assert!(preserve_non_action_tool("mcp__codegraph__codegraph_search"));
        assert!(preserve_non_action_tool("codegraph_node"));
        assert!(!preserve_non_action_tool("mcp__github__create_issue"));
    }

    #[tokio::test]
    async fn prepare_preserves_discovery_tools_for_plain_question_regression() {
        let provider = Arc::new(TestProvider {
            name: "openai-test",
            convention: StreamConvention::OpenAiNative,
        });
        let request = prepare_stream_request(
            provider,
            &[user_text("what is ownership in rust?")],
            &ModelId::new("test-model"),
            Default::default(),
        )
        .await;

        let tool_names = request
            .opts
            .tools
            .iter()
            .map(|tool| tool.name.as_str())
            .collect::<Vec<_>>();
        assert!(tool_names.contains(&"ToolSearch"));
        assert!(tool_names.contains(&"ToolSuggest"));
        assert!(!tool_names.contains(&"Bash"));
        assert!(!tool_names.contains(&"Read"));
    }

    #[tokio::test]
    #[serial_test::serial]
    async fn prepare_injects_total_tokens_reminder_countdown_normal() {
        let provider = Arc::new(TestProvider {
            name: "openai-test",
            convention: StreamConvention::OpenAiNative,
        });
        let overrides = crate::runtime::StreamRequestOverrides {
            last_usage_input_tokens: Some(150),
            context_window_tokens: Some(200),
            total_tokens_reminder_mode: Some(
                crate::total_tokens_reminder::TotalTokensReminderMode::Countdown,
            ),
            ..Default::default()
        };
        let request = prepare_stream_request(
            provider,
            &[user_text("write a small function")],
            &ModelId::new("test-model"),
            overrides,
        )
        .await;

        assert!(
            request
                .opts
                .system
                .as_deref()
                .unwrap_or_default()
                .contains("<total_tokens>50 tokens left</total_tokens>")
        );
    }

    #[tokio::test]
    #[serial_test::serial]
    async fn prepare_applies_temperature_when_thinking_absent_normal() {
        let _guard = TemperatureGlobalGuard::set(0.8);
        let provider = Arc::new(TestProvider {
            name: "openai-test",
            convention: StreamConvention::OpenAiNative,
        });
        let request = prepare_stream_request(
            provider,
            &[user_text("write a small function")],
            &ModelId::new("test-model"),
            Default::default(),
        )
        .await;

        assert_eq!(request.opts.temperature, Some(0.8));
    }

    #[tokio::test]
    #[serial_test::serial]
    async fn prepare_skips_temperature_for_anthropic_thinking_regression() {
        let _guard = TemperatureGlobalGuard::set(0.8);
        let provider = Arc::new(TestProvider {
            name: "anthropic",
            convention: StreamConvention::AnthropicNative,
        });
        let request = prepare_stream_request(
            provider,
            &[user_text("write a small function")],
            &ModelId::new("claude-opus-4-8"),
            Default::default(),
        )
        .await;

        assert!(request.opts.adaptive_thinking);
        assert_eq!(request.opts.temperature, None);
    }
}

use jfc_provider::{ProviderContent, ProviderMessage};

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
pub(super) fn conversation_is_mid_tool_loop(messages: &[ProviderMessage]) -> bool {
    let Some(last) = messages.last() else {
        return false;
    };
    last.content
        .iter()
        .any(|c| matches!(c, ProviderContent::ToolResult { .. }))
}

pub(super) fn user_text_requests_action(text: &str) -> bool {
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
        "boilerplate",
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

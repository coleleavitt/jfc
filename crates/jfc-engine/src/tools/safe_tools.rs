/// Synchronous helpers, safe-tool executors, and utility functions used by
/// the tool dispatcher. Nothing in here touches global process state
/// directly — callers go through `registry` for that.
use std::path::Path;
use std::process::Stdio;

use tokio::process::Command;

use crate::runtime::ExecutionResult;

use super::defs::all_tool_defs;
use super::registry::snapshot_mcp_registry;

#[cfg(unix)]
unsafe extern "C" {
    fn setsid() -> i32;
}

// ---------------------------------------------------------------------------
// Tool search / suggest
// ---------------------------------------------------------------------------

pub async fn all_tool_defs_with_mcp() -> Vec<jfc_provider::ToolDef> {
    let mut tools = all_tool_defs();
    let builtin_names: std::collections::HashSet<String> =
        tools.iter().map(|t| t.name.clone()).collect();
    if let Some(registry) = snapshot_mcp_registry() {
        for tool in registry.all_advertised_tool_defs().await {
            // Codegraph #284: external MCP servers occasionally advertise
            // tool names that double our own prefix (e.g. an MCP server
            // re-publishes a `graph_search` tool when we already host
            // `graph_search` natively, producing a `mcp__jfc__graph_search`
            // collision). Drop those: the agent gets a single, canonical
            // implementation and shadowing surprises don't reach the model.
            if builtin_names.contains(&tool.name) {
                tracing::warn!(
                    target: "jfc::tools::mcp",
                    tool = %tool.name,
                    "dropping MCP-advertised tool that collides with a builtin name"
                );
                continue;
            }
            tools.push(tool);
        }
    }
    tools
}

pub async fn execute_tool_search(query: &str, limit: Option<u64>, cwd: &Path) -> ExecutionResult {
    let query = query.trim().to_ascii_lowercase();
    let limit = limit.unwrap_or(20).clamp(1, 50) as usize;
    let mut rows: Vec<(usize, String)> = Vec::new();

    for tool in all_tool_defs_with_mcp().await {
        let haystack = format!(
            "{} {} {}",
            tool.name,
            tool.description,
            tool.input_schema
                .get("properties")
                .map(|v| v.to_string())
                .unwrap_or_default()
        )
        .to_ascii_lowercase();
        let score = relevance_score(&haystack, &query);
        if score > 0 {
            rows.push((
                score,
                format!(
                    "- tool `{}`: {}\n  schema: {}",
                    tool.name,
                    tool.description,
                    compact_schema(&tool.input_schema)
                ),
            ));
        }
    }

    for skill in crate::agents::load_skills(cwd) {
        if !skill.is_discoverable() {
            continue;
        }
        let haystack = format!(
            "{} {} {} {}",
            skill.name,
            skill.description.clone().unwrap_or_default(),
            skill.context.as_str(),
            skill.body.lines().take(6).collect::<Vec<_>>().join(" ")
        )
        .to_ascii_lowercase();
        let score = relevance_score(&haystack, &query);
        if score > 0 {
            let mut details = Vec::new();
            if skill.context.is_fork() {
                details.push("fork".to_owned());
            }
            if !skill.files.is_empty() {
                details.push(format!("{} package files", skill.files.len()));
            }
            let detail_suffix = if details.is_empty() {
                String::new()
            } else {
                format!(" ({})", details.join(", "))
            };
            rows.push((
                score.saturating_add(1),
                format!(
                    "- skill `{}`{}: {}\n  invoke: Skill {{ \"name\": \"{}\" }}",
                    skill.name,
                    detail_suffix,
                    skill.description.as_deref().unwrap_or("no description"),
                    skill.name
                ),
            ));
        }
    }

    rows.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    let body = rows
        .into_iter()
        .take(limit)
        .map(|(_, row)| row)
        .collect::<Vec<_>>()
        .join("\n");
    if body.is_empty() {
        ExecutionResult::success(format!("No tools or skills matched query `{query}`."))
    } else {
        ExecutionResult::success(format!("Matches for `{query}`:\n{body}"))
    }
}

pub async fn execute_tool_suggest(intent: &str, limit: Option<u64>, cwd: &Path) -> ExecutionResult {
    execute_tool_search(intent, Some(limit.unwrap_or(8).clamp(1, 20)), cwd).await
}

fn relevance_score(haystack: &str, query: &str) -> usize {
    if query.is_empty() {
        return 1;
    }
    let mut score = 0usize;
    if haystack.contains(query) {
        score += 8;
    }
    for term in query.split_whitespace().filter(|s| !s.is_empty()) {
        if haystack.contains(term) {
            score += 2;
        }
    }
    score
}

fn compact_schema(schema: &serde_json::Value) -> String {
    let required = schema
        .get("required")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        })
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "none".to_owned());
    let props = schema
        .get("properties")
        .and_then(|v| v.as_object())
        .map(|obj| obj.keys().cloned().collect::<Vec<_>>().join(", "))
        .unwrap_or_else(|| "none".to_owned());
    format!("required [{required}], properties [{props}]")
}

// ---------------------------------------------------------------------------
// Shell / process helpers
// ---------------------------------------------------------------------------

pub fn configure_tool_command(command: &mut Command) {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("SUDO_ASKPASS", "/bin/false")
        .env("SSH_ASKPASS", "/bin/false");

    #[cfg(unix)]
    unsafe {
        command.pre_exec(|| {
            if setsid() == -1 {
                Err(std::io::Error::last_os_error())
            } else {
                Ok(())
            }
        });
    }
}

pub fn terminal_safe_text(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();

    while let Some(ch) = chars.next() {
        match ch {
            '\u{1b}' => match chars.peek().copied() {
                Some('[') => {
                    chars.next();
                    for c in chars.by_ref() {
                        if ('@'..='~').contains(&c) {
                            break;
                        }
                    }
                }
                Some(']') => {
                    chars.next();
                    let mut previous_was_esc = false;
                    for c in chars.by_ref() {
                        if c == '\u{7}' || (previous_was_esc && c == '\\') {
                            break;
                        }
                        previous_was_esc = c == '\u{1b}';
                    }
                }
                Some(_) => {
                    chars.next();
                }
                None => {}
            },
            '\t' | '\n' | '\r' => out.push(ch),
            c if c.is_control() => {}
            c => out.push(c),
        }
    }

    out
}

pub fn non_interactive_shell_command(command: &str) -> String {
    let trimmed = command.trim_start();
    let leading_len = command.len() - trimmed.len();

    if trimmed == "sudo" {
        return format!("{}sudo -n", &command[..leading_len]);
    }

    let Some(rest) = trimmed.strip_prefix("sudo ") else {
        return command.to_string();
    };

    if rest.starts_with("-n ") || rest == "-n" || rest.starts_with("--non-interactive ") {
        command.to_string()
    } else {
        format!("{}sudo -n {}", &command[..leading_len], rest)
    }
}

// ---------------------------------------------------------------------------
// Permission helper (feature-gated)
// ---------------------------------------------------------------------------

#[cfg(feature = "permission-automation")]
pub fn tool_permission_path(input: &crate::types::ToolInput) -> Option<&str> {
    use crate::types::ToolInput;
    match input {
        ToolInput::Edit { file_path, .. }
        | ToolInput::Write { file_path, .. }
        | ToolInput::Read { file_path, .. } => Some(file_path.as_str()),
        ToolInput::Bash {
            workdir: Some(workdir),
            ..
        }
        | ToolInput::Glob {
            path: Some(workdir),
            ..
        }
        | ToolInput::Grep {
            path: Some(workdir),
            ..
        }
        | ToolInput::Search {
            path: Some(workdir),
            ..
        } => Some(workdir.as_str()),
        ToolInput::MemoryDelete { path } => Some(path.as_str()),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Slop guard
// ---------------------------------------------------------------------------

/// The sentinel marker appended to tool outputs when slop_guard finds issues.
/// Used by the event loop to detect and aggregate findings across a batch.
pub const SLOP_GUARD_MARKER: &str = "\n\n--- Slop Guard ---\n";

/// Run the slop_guard checks on a file that was just written/edited.
/// Returns the original result with findings appended on success,
/// or the original result unchanged if slop_guard panics, times out
/// (>2s), or finds nothing.
pub async fn maybe_run_slop_guard(
    mut result: ExecutionResult,
    file_path: &Path,
    file_content: &str,
    cwd: &Path,
) -> ExecutionResult {
    use std::time::Duration;

    // Non-blocking: if slop_guard panics or exceeds 2s, skip silently.
    let path = file_path.to_path_buf();
    let content = file_content.to_string();
    let workspace = cwd.to_path_buf();

    let handle = tokio::spawn(async move {
        crate::slop_guard::run_all_checks(&path, &content, &workspace).await
    });

    let guard_result = tokio::time::timeout(Duration::from_secs(2), handle).await;

    match guard_result {
        Ok(Ok(report)) => {
            tracing::debug!(
                target: "jfc::slop_guard",
                file = %file_path.display(),
                has_findings = report.has_findings,
                "slop_guard completed"
            );
            if report.has_findings {
                let formatted = crate::slop_guard::format_report(&report);
                tracing::debug!(
                    target: "jfc::slop_guard",
                    file = %file_path.display(),
                    findings = %formatted,
                    "slop_guard findings"
                );
                result.output.push_str(SLOP_GUARD_MARKER);
                result.output.push_str(&formatted);
            }
        }
        Ok(Err(_join_err)) => {
            tracing::debug!(
                target: "jfc::slop_guard",
                file = %file_path.display(),
                "slop_guard panicked, skipping"
            );
        }
        Err(_timeout) => {
            tracing::debug!(
                target: "jfc::slop_guard",
                file = %file_path.display(),
                "slop_guard timed out (>2s), skipping"
            );
        }
    }

    result
}

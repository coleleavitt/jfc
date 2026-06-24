/// Synchronous helpers, safe-tool executors, and utility functions used by
/// the tool dispatcher. Nothing in here touches global process state
/// directly — callers go through `registry` for that.
use std::path::Path;
use std::process::Stdio;

use tokio::process::Command;

use crate::runtime::ExecutionResult;

pub use super::discovery::{execute_tool_search, execute_tool_suggest};

#[cfg(unix)]
unsafe extern "C" {
    fn setsid() -> i32;
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
    old_content: Option<&str>,
    cwd: &Path,
) -> ExecutionResult {
    use std::time::Duration;

    // Non-blocking: if slop_guard panics or exceeds 2s, skip silently.
    let path = file_path.to_path_buf();
    let content = file_content.to_string();
    let old = old_content.map(str::to_owned);
    let workspace = cwd.to_path_buf();

    let handle = tokio::spawn(async move {
        // Run the full guard pipeline (slop checks + static wiring guard), not
        // slop alone, so post-edit reporting is one unified surface.
        crate::guards::run_guard_pipeline(&path, &content, old.as_deref(), &workspace).await
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

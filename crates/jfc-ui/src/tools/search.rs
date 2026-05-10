use std::path::{Path, PathBuf};

use tokio::process::Command;
use tracing::{debug, warn};

use super::bash::execute_bash;
use super::{ExecutionResult, configure_tool_command, terminal_safe_text};

pub(super) async fn execute_glob(pattern: &str, path: Option<&str>, cwd: &Path) -> ExecutionResult {
    debug!(target: "jfc::tools", pattern, path, "glob: searching");
    let base = path.map(PathBuf::from).unwrap_or_else(|| cwd.to_path_buf());
    let mut cmd = Command::new("rg");
    cmd.arg("--files")
        .arg("--glob")
        .arg(pattern)
        .current_dir(&base);
    configure_tool_command(&mut cmd);
    match cmd.output().await {
        Ok(out) => {
            let stdout = terminal_safe_text(String::from_utf8_lossy(&out.stdout).trim());
            if stdout.is_empty() {
                debug!(target: "jfc::tools", pattern, "glob: no files matched");
                ExecutionResult::success("No files matched")
            } else {
                let count = stdout.lines().count();
                debug!(target: "jfc::tools", pattern, count, "glob: matches found");
                ExecutionResult::success(stdout)
            }
        }
        Err(_) => {
            let cmd_str = format!(
                "find '{}' -name '{}' 2>/dev/null | sort",
                base.display(),
                pattern
            );
            execute_bash(&cmd_str, Some(10_000), cwd).await
        }
    }
}

pub(super) async fn execute_grep(
    pattern: &str,
    path: Option<&str>,
    glob: Option<&str>,
    output_mode: Option<&str>,
    cwd: &Path,
) -> ExecutionResult {
    debug!(target: "jfc::tools", pattern, path, output_mode, "grep: searching");
    let search_path = path.unwrap_or(".");
    let mut cmd = Command::new("rg");
    cmd.arg("--no-heading").arg("-n");

    match output_mode.unwrap_or("content") {
        "files_with_matches" => {
            cmd.arg("-l");
        }
        "count" => {
            cmd.arg("-c");
        }
        _ => {}
    }

    if let Some(g) = glob {
        cmd.arg("--glob").arg(g);
    }

    cmd.arg(pattern).arg(search_path).current_dir(cwd);
    configure_tool_command(&mut cmd);

    match cmd.output().await {
        Ok(out) => {
            let stdout = terminal_safe_text(String::from_utf8_lossy(&out.stdout).trim());
            let stderr = terminal_safe_text(String::from_utf8_lossy(&out.stderr).trim());
            if stdout.is_empty() && out.status.code() == Some(1) {
                debug!(target: "jfc::tools", pattern, "grep: no matches found");
                ExecutionResult::success("No matches found")
            } else if !stderr.is_empty() && stdout.is_empty() {
                warn!(target: "jfc::tools", pattern, error = %stderr, "grep: rg error");
                ExecutionResult::failure(stderr)
            } else {
                let result_lines = stdout.lines().count();
                debug!(target: "jfc::tools", pattern, result_lines, "grep: matches found");
                ExecutionResult::success(stdout)
            }
        }
        Err(e) => {
            warn!(target: "jfc::tools", error = %e, "grep: rg not found or failed");
            ExecutionResult::failure(format!("rg not found or failed: {e}"))
        }
    }
}

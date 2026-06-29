use std::path::{Path, PathBuf};

use tokio::process::Command;
use tracing::{debug, warn};

use super::ExecutionResult;
use super::safe_tools::{configure_tool_command, terminal_safe_text};

pub async fn execute_glob(pattern: &str, path: Option<&str>, cwd: &Path) -> ExecutionResult {
    debug!(target: "jfc::tools", pattern, path, "glob: searching");
    let base = path.map(PathBuf::from).unwrap_or_else(|| cwd.to_path_buf());
    let mut cmd = Command::new("rg");
    cmd.arg("--files")
        .arg("--glob")
        .arg(pattern)
        .current_dir(&base);
    // Apply `.jfcignore` / `.claudeignore` so AI-private files never surface in
    // glob results, mirroring the Read guard in dispatch.rs.
    apply_access_policy_ignores(&mut cmd, cwd);
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
            // Fallback when rg is unavailable. Arguments are passed directly
            // to `find` (no shell), so quotes/metacharacters in `pattern` or
            // `base` cannot inject commands.
            let mut find_cmd = Command::new("find");
            find_cmd.arg(&base).arg("-name").arg(pattern);
            configure_tool_command(&mut find_cmd);
            match find_cmd.output().await {
                Ok(out) => {
                    let stdout = String::from_utf8_lossy(&out.stdout);
                    let mut lines: Vec<&str> =
                        stdout.lines().filter(|l| !l.trim().is_empty()).collect();
                    lines.sort_unstable();
                    let joined = terminal_safe_text(&lines.join("\n"));
                    if joined.is_empty() {
                        ExecutionResult::success("No files matched")
                    } else {
                        ExecutionResult::success(joined)
                    }
                }
                Err(e) => ExecutionResult::failure(format!("glob fallback failed: {e}")),
            }
        }
    }
}

pub async fn execute_grep(
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
    // Apply `.jfcignore` / `.claudeignore` so AI-private file contents never
    // surface in grep results, mirroring the Read guard in dispatch.rs.
    apply_access_policy_ignores(&mut cmd, cwd);
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

/// Pass any `.jfcignore` / `.claudeignore` rules to `rg` via `--ignore-file`,
/// so AI-private files are filtered out of Glob/Grep results the same way the
/// Read tool refuses them. No-op when the project defines no such files (the
/// common case), so projects not using the feature pay nothing. `rg` treats
/// these as additional gitignore-format rule files layered on its own
/// gitignore handling.
fn apply_access_policy_ignores(cmd: &mut Command, cwd: &Path) {
    let policy = crate::access_policy::AccessPolicy::for_root(cwd);
    for file in policy.ignore_files() {
        cmd.arg("--ignore-file").arg(file);
    }
}

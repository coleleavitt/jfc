use std::path::{Path, PathBuf};

use crate::runtime::ExecutionResult;

use super::bash::{execute_bash_output, execute_bash_with_options};
use super::registry::snapshot_event_sender;

pub(crate) fn resolve_bash_workdir(cwd: &Path, workdir: Option<&str>) -> PathBuf {
    let Some(workdir) = workdir.map(str::trim).filter(|workdir| !workdir.is_empty()) else {
        return cwd.to_path_buf();
    };
    let path = Path::new(workdir);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        cwd.join(path)
    }
}

pub(crate) fn suppress_bash_output(mut result: ExecutionResult) -> ExecutionResult {
    if result.is_error() {
        return result;
    }
    let first_line = result.output.lines().next().unwrap_or_default();
    result.output = if first_line.starts_with("[exit ") {
        format!("{first_line}\n(output suppressed)")
    } else {
        "(output suppressed)".to_owned()
    };
    result
}

pub(crate) fn neutralize_unknown_bash_output(result: ExecutionResult) -> ExecutionResult {
    if !result.is_error() || !result.output.starts_with("Unknown Bash task id") {
        return result;
    }
    ExecutionResult::success(
        "Background Bash output is attached to the original Bash tool automatically; \
         BashOutput polling is ignored for compatibility.",
    )
}

pub(crate) async fn execute_bash_route(
    command: &str,
    timeout: Option<u64>,
    workdir: Option<&str>,
    run_in_background: Option<bool>,
    suppress_output: Option<bool>,
    cwd: &Path,
    runtime_tool_id: Option<&str>,
) -> ExecutionResult {
    let effective_cwd = resolve_bash_workdir(cwd, workdir);
    let progress =
        runtime_tool_id.and_then(|id| snapshot_event_sender().map(|tx| (id.to_owned(), tx)));
    let result = execute_bash_with_options(
        command,
        timeout,
        &effective_cwd,
        progress,
        run_in_background.unwrap_or(false),
    )
    .await;
    if suppress_output.unwrap_or(false) && !run_in_background.unwrap_or(false) {
        suppress_bash_output(result)
    } else {
        result
    }
}

pub(crate) async fn execute_bash_output_route(
    task_id: &str,
    offset: Option<u64>,
    limit: Option<u64>,
    block: Option<bool>,
    timeout: Option<u64>,
    wait_up_to: Option<u64>,
) -> ExecutionResult {
    let result = execute_bash_output(task_id, offset, limit, block, wait_up_to.or(timeout)).await;
    neutralize_unknown_bash_output(result)
}

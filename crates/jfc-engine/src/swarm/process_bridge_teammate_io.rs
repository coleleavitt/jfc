use jfc_plugin_sdk::{BridgeEnvelope, BridgeResponse};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader, Lines};
use tokio::process::{ChildStderr, ChildStdin, ChildStdout};

pub(crate) fn stdout_lines(stdout: ChildStdout) -> Lines<BufReader<ChildStdout>> {
    BufReader::new(stdout).lines()
}

pub(crate) async fn read_stdout_line(
    lines: &mut Lines<BufReader<ChildStdout>>,
    launcher_name: &str,
) -> Option<Result<String, String>> {
    match lines.next_line().await {
        Ok(Some(line)) => Some(Ok(line)),
        Ok(None) => None,
        Err(error) => Some(Err(format!(
            "ProcessBridge teammate launcher `{launcher_name}` stdout read failed: {error}"
        ))),
    }
}

pub(crate) async fn read_stderr(mut stderr: ChildStderr) -> String {
    let mut text = String::new();
    if stderr.read_to_string(&mut text).await.is_ok() {
        text
    } else {
        String::new()
    }
}

pub(crate) async fn write_bridge_response(
    stdin: &mut ChildStdin,
    launcher_name: &str,
    id: String,
    response: BridgeResponse,
) -> Result<(), String> {
    let line = serde_json::to_string(&BridgeEnvelope::response(id, response)).map_err(|error| {
        format!(
            "ProcessBridge teammate launcher `{launcher_name}` response serialization failed: {error}"
        )
    })?;
    stdin.write_all(line.as_bytes()).await.map_err(|error| {
        format!("ProcessBridge teammate launcher `{launcher_name}` response write failed: {error}")
    })?;
    stdin.write_all(b"\n").await.map_err(|error| {
        format!(
            "ProcessBridge teammate launcher `{launcher_name}` response newline write failed: {error}"
        )
    })
}

pub(crate) async fn finish_stderr_task(task: &mut tokio::task::JoinHandle<String>) -> String {
    if task.is_finished() {
        task.await.unwrap_or_default()
    } else {
        task.abort();
        String::new()
    }
}

pub(crate) fn bridge_exit_failure(
    launcher_name: &str,
    status: std::io::Result<std::process::ExitStatus>,
    stderr: &str,
) -> Option<String> {
    let status = match status {
        Ok(status) => status,
        Err(error) => {
            return Some(format!(
                "ProcessBridge teammate launcher `{launcher_name}` wait failed: {error}"
            ));
        }
    };
    if status.success() {
        return None;
    }
    let stderr = stderr.trim();
    if stderr.is_empty() {
        return Some(format!(
            "ProcessBridge teammate launcher `{launcher_name}` exited with {status}"
        ));
    }
    Some(format!(
        "ProcessBridge teammate launcher `{launcher_name}` exited with {status}; stderr: {stderr}"
    ))
}

use std::process::Stdio;
use std::time::Duration;

use anyhow::Context;
use jfc_plugin_sdk::{
    BridgeEnvelope, BridgeRequest, BridgeResponse, BridgeUiPanelRefreshRequest,
    BridgeUiPanelRefreshResult, ProcessBridgeCommand, UiPanelDescriptor,
};
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

const WIDGET_REFRESH_TIMEOUT: Duration = Duration::from_secs(30);

pub(super) async fn execute_process_bridge_panel_refresh(
    panel: &UiPanelDescriptor,
    handler: &str,
    state: Option<serde_json::Value>,
) -> anyhow::Result<BridgeUiPanelRefreshResult> {
    let command = parse_process_bridge_handler(handler)?;
    let request_id = format!("ui-panel-{}", uuid::Uuid::new_v4());
    let mut request = BridgeUiPanelRefreshRequest::new(panel.id.clone(), panel.scope);
    if let Some(state) = state {
        request = request.with_state(state);
    }
    let request = BridgeEnvelope::request(
        request_id.clone(),
        BridgeRequest::UiPanelRefresh { refresh: request },
    );
    let request_line =
        serde_json::to_string(&request).context("UI panel refresh request serialization failed")?;
    let mut child = spawn_bridge_process(&command, &panel.id, "panel")?;
    write_request(&mut child, &request_line, &panel.id, "panel").await?;
    let output = tokio::time::timeout(WIDGET_REFRESH_TIMEOUT, child.wait_with_output())
        .await
        .context("UI panel refresh process timed out")??;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let line = stdout
        .lines()
        .find(|line| !line.trim().is_empty())
        .with_context(|| {
            empty_response_message(&panel.id, "panel", &output.stderr, output.status.success())
        })?;
    response_line_to_panel_refresh_result(panel, &request_id, line)
}

fn parse_process_bridge_handler(handler: &str) -> anyhow::Result<ProcessBridgeCommand> {
    let trimmed = handler.trim();
    if trimmed.is_empty() {
        anyhow::bail!("UI widget refresh ProcessBridge handler is empty");
    }
    if trimmed.starts_with('{') {
        return serde_json::from_str::<ProcessBridgeCommand>(trimmed)
            .context("UI widget refresh ProcessBridge handler JSON is invalid");
    }
    Ok(ProcessBridgeCommand::new(trimmed))
}

fn spawn_bridge_process(
    command: &ProcessBridgeCommand,
    item_id: &str,
    item_kind: &str,
) -> anyhow::Result<tokio::process::Child> {
    Command::new(&command.command)
        .args(&command.args)
        .kill_on_drop(true)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| {
            format!(
                "UI {item_kind} `{item_id}` refresh could not start `{}`",
                command.command
            )
        })
}

async fn write_request(
    child: &mut tokio::process::Child,
    request_line: &str,
    item_id: &str,
    item_kind: &str,
) -> anyhow::Result<()> {
    let Some(mut stdin) = child.stdin.take() else {
        anyhow::bail!("UI {item_kind} `{item_id}` refresh could not open bridge stdin");
    };
    stdin
        .write_all(request_line.as_bytes())
        .await
        .context("UI widget refresh request write failed")?;
    stdin
        .write_all(b"\n")
        .await
        .context("UI widget refresh request newline write failed")?;
    drop(stdin);
    Ok(())
}

fn response_line_to_panel_refresh_result(
    panel: &UiPanelDescriptor,
    request_id: &str,
    line: &str,
) -> anyhow::Result<BridgeUiPanelRefreshResult> {
    let frame = serde_json::from_str::<BridgeEnvelope>(line)
        .context("UI panel refresh returned invalid JSONL")?;
    let BridgeEnvelope::Response { id, response } = frame else {
        anyhow::bail!("UI panel `{}` refresh returned a request frame", panel.id);
    };
    if id != request_id {
        anyhow::bail!(
            "UI panel `{}` refresh response id `{id}` did not match `{request_id}`",
            panel.id
        );
    }
    match response {
        BridgeResponse::UiPanelRefresh { result } => Ok(result),
        BridgeResponse::Error(error) => {
            anyhow::bail!(
                "UI panel `{}` refresh bridge error `{}`: {}",
                panel.id,
                error.code,
                error.message
            )
        }
        other => anyhow::bail!(
            "UI panel `{}` refresh returned unexpected response: {other:?}",
            panel.id
        ),
    }
}

fn empty_response_message(item_id: &str, item_kind: &str, stderr: &[u8], success: bool) -> String {
    let stderr = String::from_utf8_lossy(stderr);
    let suffix = stderr.trim();
    let suffix = if suffix.is_empty() {
        String::new()
    } else {
        format!("; stderr: {suffix}")
    };
    format!(
        "UI {item_kind} `{item_id}` refresh exited {}without a response frame{}",
        if success { "" } else { "unsuccessfully " },
        suffix
    )
}

use std::process::Stdio;
use std::time::Duration;

use jfc_plugin_sdk::{
    BridgeEnvelope, BridgeRequest, BridgeResponse, BridgeUiWidgetRefreshRequest,
    BridgeUiWidgetRefreshResult, PluginId, ProcessBridgeCommand, UiMutationScope,
    UiWidgetDescriptor, UiWidgetRefreshKind,
};
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

use crate::{PluginHostError, PluginRuntime};

const WIDGET_REFRESH_TIMEOUT: Duration = Duration::from_secs(30);

impl PluginRuntime {
    pub async fn refresh_ui_widget_snapshot(
        &self,
        plugin_id: &PluginId,
        scope: UiMutationScope,
        widget_id: &str,
        state: Option<serde_json::Value>,
    ) -> Result<BridgeUiWidgetRefreshResult, PluginHostError> {
        let key = (plugin_id.clone(), scope, widget_id.to_owned());
        let widget = self
            .ui_widgets
            .get(&key)
            .ok_or_else(|| {
                PluginHostError::plugin(format!(
                    "UI widget `{}` for plugin `{}` is not registered",
                    widget_id,
                    plugin_id.as_str()
                ))
            })?
            .descriptor();
        let refresh = widget.refresh.as_ref().ok_or_else(|| {
            PluginHostError::plugin(format!(
                "UI widget `{}` has no refresh descriptor",
                widget.id
            ))
        })?;
        if refresh.kind != UiWidgetRefreshKind::ProcessBridge {
            return Err(PluginHostError::plugin(format!(
                "UI widget `{}` refresh is not process_bridge",
                widget.id
            )));
        }
        execute_process_bridge_widget_refresh(widget, &refresh.handler, state).await
    }
}

async fn execute_process_bridge_widget_refresh(
    widget: &UiWidgetDescriptor,
    handler: &str,
    state: Option<serde_json::Value>,
) -> Result<BridgeUiWidgetRefreshResult, PluginHostError> {
    let command = parse_process_bridge_handler(handler)?;
    let request_id = format!("ui-widget-{}", uuid::Uuid::new_v4());
    let mut request = BridgeUiWidgetRefreshRequest::new(widget.id.clone(), widget.scope);
    if let Some(state) = state {
        request = request.with_state(state);
    }
    let request = BridgeEnvelope::request(
        request_id.clone(),
        BridgeRequest::UiWidgetRefresh { refresh: request },
    );
    let request_line = serde_json::to_string(&request).map_err(|error| {
        PluginHostError::plugin(format!(
            "UI widget refresh request serialization failed: {error}"
        ))
    })?;
    let mut child = spawn_bridge_process(&command, &widget.id)?;
    write_request(&mut child, &request_line, &widget.id).await?;
    let output = tokio::time::timeout(WIDGET_REFRESH_TIMEOUT, child.wait_with_output())
        .await
        .map_err(|error| {
            PluginHostError::plugin(format!("UI widget refresh process timed out: {error}"))
        })?
        .map_err(|error| {
            PluginHostError::plugin(format!("UI widget refresh process wait failed: {error}"))
        })?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let line = stdout
        .lines()
        .find(|line| !line.trim().is_empty())
        .ok_or_else(|| empty_response_error(&widget.id, &output.stderr, output.status.success()))?;
    response_line_to_refresh_result(widget, &request_id, line)
}

fn parse_process_bridge_handler(handler: &str) -> Result<ProcessBridgeCommand, PluginHostError> {
    let trimmed = handler.trim();
    if trimmed.is_empty() {
        return Err(PluginHostError::plugin(
            "UI widget refresh ProcessBridge handler is empty",
        ));
    }
    if trimmed.starts_with('{') {
        return serde_json::from_str::<ProcessBridgeCommand>(trimmed).map_err(|error| {
            PluginHostError::plugin(format!(
                "UI widget refresh ProcessBridge handler JSON is invalid: {error}"
            ))
        });
    }
    Ok(ProcessBridgeCommand::new(trimmed))
}

fn spawn_bridge_process(
    command: &ProcessBridgeCommand,
    widget_id: &str,
) -> Result<tokio::process::Child, PluginHostError> {
    Command::new(&command.command)
        .args(&command.args)
        .kill_on_drop(true)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| {
            PluginHostError::plugin(format!(
                "UI widget `{widget_id}` refresh could not start `{}`: {error}",
                command.command
            ))
        })
}

async fn write_request(
    child: &mut tokio::process::Child,
    request_line: &str,
    widget_id: &str,
) -> Result<(), PluginHostError> {
    let Some(mut stdin) = child.stdin.take() else {
        return Err(PluginHostError::plugin(format!(
            "UI widget `{widget_id}` refresh could not open bridge stdin"
        )));
    };
    stdin
        .write_all(request_line.as_bytes())
        .await
        .map_err(|error| {
            PluginHostError::plugin(format!("UI widget refresh request write failed: {error}"))
        })?;
    stdin.write_all(b"\n").await.map_err(|error| {
        PluginHostError::plugin(format!(
            "UI widget refresh request newline write failed: {error}"
        ))
    })?;
    drop(stdin);
    Ok(())
}

fn response_line_to_refresh_result(
    widget: &UiWidgetDescriptor,
    request_id: &str,
    line: &str,
) -> Result<BridgeUiWidgetRefreshResult, PluginHostError> {
    let frame = serde_json::from_str::<BridgeEnvelope>(line).map_err(|error| {
        PluginHostError::plugin(format!("UI widget refresh returned invalid JSONL: {error}"))
    })?;
    let BridgeEnvelope::Response { id, response } = frame else {
        return Err(PluginHostError::plugin(format!(
            "UI widget `{}` refresh returned a request frame",
            widget.id
        )));
    };
    if id != request_id {
        return Err(PluginHostError::plugin(format!(
            "UI widget `{}` refresh response id `{id}` did not match `{request_id}`",
            widget.id
        )));
    }
    match response {
        BridgeResponse::UiWidgetRefresh { result } => Ok(result),
        BridgeResponse::Error(error) => Err(PluginHostError::plugin(format!(
            "UI widget `{}` refresh bridge error `{}`: {}",
            widget.id, error.code, error.message
        ))),
        other => Err(PluginHostError::plugin(format!(
            "UI widget `{}` refresh returned unexpected response: {other:?}",
            widget.id
        ))),
    }
}

fn empty_response_error(widget_id: &str, stderr: &[u8], success: bool) -> PluginHostError {
    let stderr = String::from_utf8_lossy(stderr);
    let suffix = stderr.trim();
    let suffix = if suffix.is_empty() {
        String::new()
    } else {
        format!("; stderr: {suffix}")
    };
    PluginHostError::plugin(format!(
        "UI widget `{widget_id}` refresh exited {}without a response frame{}",
        if success { "" } else { "unsuccessfully " },
        suffix
    ))
}

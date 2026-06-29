use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use anyhow::Context;
use jfc_plugin_sdk::{
    BridgeEnvelope, BridgePromptContextRefreshRequest, BridgePromptContextRefreshResult,
    BridgeRequest, BridgeResponse, ProcessBridgeCommand, RuntimeExtensionDescriptor,
};
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

const PROMPT_CONTEXT_TIMEOUT: Duration = Duration::from_secs(10);

pub(super) struct PromptContextBridgeInvocation<'a> {
    pub(super) extension: &'a RuntimeExtensionDescriptor,
    pub(super) cwd: &'a Path,
    pub(super) state: Option<serde_json::Value>,
    pub(super) max_chars: usize,
}

pub(super) async fn refresh_process_bridge_prompt_context(
    invocation: PromptContextBridgeInvocation<'_>,
) -> anyhow::Result<BridgePromptContextRefreshResult> {
    let command = parse_process_bridge_handler(&invocation.extension.executor.handler)?;
    let request_id = format!("prompt-context-{}", uuid::Uuid::new_v4());
    let mut refresh = BridgePromptContextRefreshRequest::new(invocation.extension.id.clone())
        .with_cwd(invocation.cwd.to_string_lossy().into_owned())
        .with_max_chars(invocation.max_chars);
    if let Some(state) = invocation.state {
        refresh = refresh.with_state(state);
    }
    let request = BridgeEnvelope::request(
        request_id.clone(),
        BridgeRequest::PromptContextRefresh { refresh },
    );
    let request_line = serde_json::to_string(&request)
        .context("prompt-context refresh request serialization failed")?;
    let mut child = spawn_bridge_process(&command, &invocation.extension.id)?;
    write_request(&mut child, &request_line, &invocation.extension.id).await?;
    let output = tokio::time::timeout(PROMPT_CONTEXT_TIMEOUT, child.wait_with_output())
        .await
        .context("prompt-context refresh process timed out")??;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let line = stdout
        .lines()
        .find(|line| !line.trim().is_empty())
        .with_context(|| {
            empty_response_message(
                invocation.extension,
                &output.stderr,
                output.status.success(),
            )
        })?;
    response_line_to_prompt_context_result(invocation.extension, &request_id, line)
}

fn parse_process_bridge_handler(handler: &str) -> anyhow::Result<ProcessBridgeCommand> {
    let trimmed = handler.trim();
    if trimmed.is_empty() {
        anyhow::bail!("prompt-context ProcessBridge handler is empty");
    }
    if trimmed.starts_with('{') {
        return serde_json::from_str::<ProcessBridgeCommand>(trimmed)
            .context("prompt-context ProcessBridge handler JSON is invalid");
    }
    Ok(ProcessBridgeCommand::new(trimmed))
}

fn spawn_bridge_process(
    command: &ProcessBridgeCommand,
    extension_id: &str,
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
                "prompt-context extension `{extension_id}` could not start `{}`",
                command.command
            )
        })
}

async fn write_request(
    child: &mut tokio::process::Child,
    request_line: &str,
    extension_id: &str,
) -> anyhow::Result<()> {
    let Some(mut stdin) = child.stdin.take() else {
        anyhow::bail!("prompt-context extension `{extension_id}` could not open bridge stdin");
    };
    stdin
        .write_all(request_line.as_bytes())
        .await
        .context("prompt-context refresh request write failed")?;
    stdin
        .write_all(b"\n")
        .await
        .context("prompt-context refresh request newline write failed")?;
    drop(stdin);
    Ok(())
}

fn response_line_to_prompt_context_result(
    extension: &RuntimeExtensionDescriptor,
    request_id: &str,
    line: &str,
) -> anyhow::Result<BridgePromptContextRefreshResult> {
    let frame = serde_json::from_str::<BridgeEnvelope>(line)
        .context("prompt-context refresh returned invalid JSONL")?;
    let BridgeEnvelope::Response { id, response } = frame else {
        anyhow::bail!(
            "prompt-context extension `{}` returned a request frame",
            extension.id
        );
    };
    if id != request_id {
        anyhow::bail!(
            "prompt-context extension `{}` response id `{id}` did not match `{request_id}`",
            extension.id
        );
    }
    match response {
        BridgeResponse::PromptContextRefresh { result } => Ok(result),
        BridgeResponse::Error(error) => {
            anyhow::bail!(
                "prompt-context extension `{}` bridge error `{}`: {}",
                extension.id,
                error.code,
                error.message
            )
        }
        other => anyhow::bail!(
            "prompt-context extension `{}` returned unexpected response: {other:?}",
            extension.id
        ),
    }
}

fn empty_response_message(
    extension: &RuntimeExtensionDescriptor,
    stderr: &[u8],
    success: bool,
) -> String {
    let stderr = String::from_utf8_lossy(stderr);
    let suffix = stderr.trim();
    let suffix = if suffix.is_empty() {
        String::new()
    } else {
        format!("; stderr: {suffix}")
    };
    format!(
        "prompt-context extension `{}` exited {}without a response frame{}",
        extension.id,
        if success { "" } else { "unsuccessfully " },
        suffix
    )
}

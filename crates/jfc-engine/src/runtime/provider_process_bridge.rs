use std::process::Stdio;

use anyhow::Context;
use jfc_plugin_sdk::{
    BridgeEnvelope, BridgeRequest, ProcessBridgeCommand, ProviderExecutorDescriptor,
};
use jfc_provider::{EventStream, ProviderMessage, StreamOptions};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;
use tokio_stream::wrappers::ReceiverStream;

use super::provider_bridge_codec::{provider_message_to_bridge, stream_options_to_bridge};
use super::provider_bridge_events::response_line_to_event;

const PROVIDER_BRIDGE_BUFFER: usize = 32;

pub(crate) struct ProviderBridgeInvocation<'a> {
    pub(crate) provider_name: &'a str,
    pub(crate) executor: &'a ProviderExecutorDescriptor,
    pub(crate) messages: Vec<ProviderMessage>,
    pub(crate) options: &'a StreamOptions,
}

pub(crate) async fn stream(
    invocation: ProviderBridgeInvocation<'_>,
) -> anyhow::Result<EventStream> {
    let command = parse_process_bridge_handler(&invocation.executor.handler)?;
    let request_id = format!("provider-{}", uuid::Uuid::new_v4());
    let request = BridgeEnvelope::request(
        request_id.clone(),
        BridgeRequest::ProviderStream {
            provider: invocation.provider_name.to_owned(),
            messages: invocation
                .messages
                .into_iter()
                .map(provider_message_to_bridge)
                .collect(),
            options: stream_options_to_bridge(invocation.options),
        },
    );
    let request_line = serde_json::to_string(&request)
        .context("ProcessBridge provider request serialization failed")?;
    let mut child = spawn_bridge_process(&command, invocation.provider_name)?;
    write_request(&mut child, &request_line, invocation.provider_name).await?;

    let stdout = child.stdout.take().context("bridge stdout unavailable")?;
    let stderr = child.stderr.take().context("bridge stderr unavailable")?;
    let provider_name = invocation.provider_name.to_owned();
    let (tx, rx) = tokio::sync::mpsc::channel(PROVIDER_BRIDGE_BUFFER);
    tokio::spawn(async move {
        let mut stderr_task = tokio::spawn(read_stderr(stderr));
        let mut lines = BufReader::new(stdout).lines();
        while let Some(line) = read_stdout_line(&mut lines, &provider_name).await {
            let event =
                line.and_then(|line| response_line_to_event(&provider_name, &request_id, &line));
            if tx.send(event).await.is_err() {
                let _ = child.kill().await;
                return;
            }
        }
        let status = child.wait().await;
        let stderr = finish_stderr_task(&mut stderr_task).await;
        if let Err(error) = bridge_exit_error(&provider_name, status, &stderr) {
            let _ = tx.send(Err(error)).await;
        }
    });

    Ok(Box::pin(ReceiverStream::new(rx)))
}

fn parse_process_bridge_handler(handler: &str) -> anyhow::Result<ProcessBridgeCommand> {
    let trimmed = handler.trim();
    if trimmed.is_empty() {
        anyhow::bail!("ProcessBridge provider handler is empty");
    }
    if trimmed.starts_with('{') {
        return serde_json::from_str::<ProcessBridgeCommand>(trimmed)
            .context("ProcessBridge provider handler JSON is invalid");
    }
    Ok(ProcessBridgeCommand::new(trimmed))
}

fn spawn_bridge_process(
    command: &ProcessBridgeCommand,
    provider_name: &str,
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
                "ProcessBridge provider `{provider_name}` could not start `{}`",
                command.command
            )
        })
}

async fn write_request(
    child: &mut tokio::process::Child,
    request_line: &str,
    provider_name: &str,
) -> anyhow::Result<()> {
    let Some(mut stdin) = child.stdin.take() else {
        anyhow::bail!("ProcessBridge provider `{provider_name}` could not open bridge stdin");
    };
    stdin
        .write_all(request_line.as_bytes())
        .await
        .context("ProcessBridge provider request write failed")?;
    stdin
        .write_all(b"\n")
        .await
        .context("ProcessBridge provider request newline write failed")?;
    drop(stdin);
    Ok(())
}

async fn read_stderr(mut stderr: tokio::process::ChildStderr) -> String {
    let mut text = String::new();
    if stderr.read_to_string(&mut text).await.is_ok() {
        text
    } else {
        String::new()
    }
}

async fn finish_stderr_task(task: &mut tokio::task::JoinHandle<String>) -> String {
    if task.is_finished() {
        task.await.unwrap_or_default()
    } else {
        task.abort();
        String::new()
    }
}

async fn read_stdout_line(
    lines: &mut tokio::io::Lines<BufReader<tokio::process::ChildStdout>>,
    provider_name: &str,
) -> Option<anyhow::Result<String>> {
    match lines.next_line().await {
        Ok(Some(line)) => Some(Ok(line)),
        Ok(None) => None,
        Err(error) => Some(Err(anyhow::anyhow!(
            "ProcessBridge provider `{provider_name}` stdout read failed: {error}"
        ))),
    }
}

fn bridge_exit_error(
    provider_name: &str,
    status: std::io::Result<std::process::ExitStatus>,
    stderr: &str,
) -> anyhow::Result<()> {
    let status =
        status.with_context(|| format!("ProcessBridge provider `{provider_name}` wait failed"))?;
    if status.success() {
        return Ok(());
    }
    let stderr = stderr.trim();
    if stderr.is_empty() {
        anyhow::bail!("ProcessBridge provider `{provider_name}` exited with {status}");
    }
    anyhow::bail!(
        "ProcessBridge provider `{provider_name}` exited with {status}; stderr: {stderr}"
    );
}

use tokio::process::{Child, ChildStdin};
use tokio::sync::{mpsc, watch};

use super::process_bridge_teammate::ProcessBridgeTeammateConfig;
use super::process_bridge_teammate_events::{
    BridgeTeammateFrame, TeammateBridgeEventContext, is_terminal_event,
    response_line_to_teammate_frame,
};
use super::process_bridge_teammate_host_requests::{
    TeammateHostRequestOutcome, handle_teammate_host_request,
};
use super::process_bridge_teammate_io::{
    bridge_exit_failure, finish_stderr_task, read_stderr, read_stdout_line, stdout_lines,
    write_bridge_response,
};
use super::runner::TeammateEvent;

pub(crate) async fn run_until_terminal(
    config: &ProcessBridgeTeammateConfig,
    request_id: &str,
    child: &mut Child,
    stdin: &mut ChildStdin,
    abort_rx: &mut watch::Receiver<bool>,
    event_tx: &mpsc::UnboundedSender<TeammateEvent>,
) -> TeammateEvent {
    let Some(stdout) = child.stdout.take() else {
        return failed(config, "ProcessBridge teammate launcher stdout unavailable");
    };
    let Some(stderr) = child.stderr.take() else {
        return failed(config, "ProcessBridge teammate launcher stderr unavailable");
    };
    let mut stderr_task = tokio::spawn(read_stderr(stderr));
    let mut lines = stdout_lines(stdout);
    let mut stdout_open = true;

    loop {
        tokio::select! {
            biased;
            changed = abort_rx.changed() => {
                stop_child(child).await;
                let _ = finish_stderr_task(&mut stderr_task).await;
                match changed {
                    Ok(()) | Err(_) => return cancelled(config),
                }
            }
            line = read_stdout_line(&mut lines, &config.descriptor.name), if stdout_open => {
                let Some(line) = line else {
                    stdout_open = false;
                    continue;
                };
                match line.and_then(|line| decode_line(config, request_id, line)) {
                    Ok(BridgeTeammateFrame::Event(event)) if is_terminal_event(&event) => {
                        stop_child(child).await;
                        let _ = finish_stderr_task(&mut stderr_task).await;
                        return event;
                    }
                    Ok(BridgeTeammateFrame::Event(event)) => {
                        if event_tx.send(event).is_err() {
                            stop_child(child).await;
                            let _ = finish_stderr_task(&mut stderr_task).await;
                            return cancelled(config);
                        }
                    }
                    Ok(BridgeTeammateFrame::HostRequest { id, request }) => {
                        let outcome = handle_teammate_host_request(config, request).await;
                        if let Err(error) =
                            emit_host_request_outcome(config, stdin, event_tx, id, outcome).await
                        {
                            stop_child(child).await;
                            let _ = finish_stderr_task(&mut stderr_task).await;
                            return failed(config, error);
                        }
                    }
                    Err(error) => {
                        stop_child(child).await;
                        let _ = finish_stderr_task(&mut stderr_task).await;
                        return failed(config, error);
                    }
                }
            }
            status = child.wait() => {
                let stderr = finish_stderr_task(&mut stderr_task).await;
                if let Some(error) = bridge_exit_failure(&config.descriptor.name, status, &stderr) {
                    return failed(config, error);
                }
                return TeammateEvent::Completed {
                    task_id: config.task_id.clone(),
                    agent_id: config.agent_id.clone(),
                };
            }
        }
    }
}

fn decode_line(
    config: &ProcessBridgeTeammateConfig,
    request_id: &str,
    line: String,
) -> Result<BridgeTeammateFrame, String> {
    let context = TeammateBridgeEventContext {
        launcher_name: &config.descriptor.name,
        request_id,
        task_id: &config.task_id,
        agent_id: &config.agent_id,
        agent_name: &config.descriptor.label,
    };
    response_line_to_teammate_frame(&context, &line)
}

async fn emit_host_request_outcome(
    config: &ProcessBridgeTeammateConfig,
    stdin: &mut ChildStdin,
    event_tx: &mpsc::UnboundedSender<TeammateEvent>,
    id: String,
    outcome: TeammateHostRequestOutcome,
) -> Result<(), String> {
    if let Some(event) = outcome.event
        && event_tx.send(event).is_err()
    {
        return Err("teammate event channel closed while handling host request".to_owned());
    }
    write_bridge_response(stdin, &config.descriptor.name, id, outcome.response).await
}

async fn stop_child(child: &mut Child) {
    let _ = child.start_kill();
    let _ = child.wait().await;
}

fn cancelled(config: &ProcessBridgeTeammateConfig) -> TeammateEvent {
    TeammateEvent::Cancelled {
        task_id: config.task_id.clone(),
        agent_id: config.agent_id.clone(),
    }
}

fn failed(config: &ProcessBridgeTeammateConfig, error: impl Into<String>) -> TeammateEvent {
    TeammateEvent::Failed {
        task_id: config.task_id.clone(),
        agent_id: config.agent_id.clone(),
        error: error.into(),
    }
}

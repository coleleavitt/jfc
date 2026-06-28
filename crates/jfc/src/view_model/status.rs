use std::collections::BTreeSet;

use jfc_core::{ChatMessage, MessagePart, ToolInput, ToolOutput, ToolStatus};
use ui_model::status::{StatusRow, StatusSegment, StatusTone};

use crate::app::App;

pub(crate) fn status_row(app: &App) -> StatusRow {
    let mut row = StatusRow::default();
    if let Some(segment) = shell_activity_status_segment(&app.engine.messages) {
        row.push(segment);
    }
    row
}

pub(crate) fn shell_activity_status_segment(messages: &[ChatMessage]) -> Option<StatusSegment> {
    let count = shell_activity_count(messages);
    if count == 0 {
        None
    } else {
        Some(StatusSegment::new(
            format!(" {count} shell{} ", if count == 1 { "" } else { "s" }),
            StatusTone::ShellActivity,
            87,
        ))
    }
}

#[cfg(test)]
fn shell_activity_badge(messages: &[ChatMessage]) -> Option<String> {
    shell_activity_status_segment(messages).map(|segment| segment.text)
}

fn shell_activity_count(messages: &[ChatMessage]) -> usize {
    let mut active = 0usize;
    let mut backgrounded = BTreeSet::new();
    let mut finished = BTreeSet::new();

    for message in messages {
        for part in &message.parts {
            let MessagePart::Tool(tool) = part else {
                continue;
            };
            match &tool.input {
                ToolInput::Bash { .. } => {
                    if matches!(tool.status, ToolStatus::Pending | ToolStatus::Running) {
                        active += 1;
                    }
                    if let Some(task_id) = background_task_id_from_output(&tool.output) {
                        if bash_output_finished_task(&tool.output) {
                            finished.insert(task_id);
                        } else if jfc_engine::tools::bash_task_is_running(&task_id) {
                            backgrounded.insert(task_id);
                        } else {
                            finished.insert(task_id);
                        }
                    }
                }
                ToolInput::BashOutput { task_id, .. } => {
                    if bash_output_finished_task(&tool.output) {
                        finished.insert(task_id.clone());
                    }
                }
                _ => {}
            }
        }
    }

    for task_id in finished {
        backgrounded.remove(&task_id);
    }
    active + backgrounded.len()
}

fn background_task_id_from_output(output: &ToolOutput) -> Option<String> {
    let text = match output {
        ToolOutput::Text(text) => text.as_str(),
        ToolOutput::LargeText(text) => text.content.as_str(),
        _ => return None,
    };
    text.lines()
        .find_map(|line| line.strip_prefix("task_id: "))
        .map(str::trim)
        .filter(|task_id| task_id.starts_with("bash_"))
        .map(ToOwned::to_owned)
}

fn bash_output_finished_task(output: &ToolOutput) -> bool {
    let text = match output {
        ToolOutput::Text(text) => text.as_str(),
        ToolOutput::LargeText(text) => text.content.as_str(),
        _ => return false,
    };
    text.lines()
        .any(|line| line == "retrieval_status: success" || bash_status_line_is_terminal(line))
}

fn bash_status_line_is_terminal(line: &str) -> bool {
    let Some(status) = line.strip_prefix("status: ") else {
        return false;
    };
    !status.trim().starts_with("running")
}

#[cfg(test)]
mod tests {
    use super::*;
    use jfc_core::{ToolCall, ToolKind};

    fn tool(input: ToolInput, status: ToolStatus, output: ToolOutput) -> MessagePart {
        MessagePart::tool(ToolCall {
            id: "tool-1".into(),
            kind: match &input {
                ToolInput::Bash { .. } => ToolKind::Bash,
                ToolInput::BashOutput { .. } => ToolKind::BashOutput,
                _ => ToolKind::Generic("test".into()),
            },
            status,
            input,
            output,
            display: jfc_core::ToolDisplayState::DEFAULT,
            elapsed_ms: None,
            started_at: None,
            thought_signature: None,
        })
    }

    #[test]
    fn shell_activity_badge_counts_running_bash_normal() {
        let mut msg = ChatMessage::assistant(String::new());
        msg.parts.clear();
        msg.parts.push(tool(
            ToolInput::Bash {
                command: "cargo test".into(),
                timeout: None,
                workdir: None,
                run_in_background: None,
                suppress_output: None,
            },
            ToolStatus::Running,
            ToolOutput::Empty,
        ));

        assert_eq!(shell_activity_badge(&[msg]).as_deref(), Some(" 1 shell "));
    }

    #[test]
    fn shell_activity_status_segment_preserves_rendered_badge_contract_normal() {
        let mut msg = ChatMessage::assistant(String::new());
        msg.parts.clear();
        msg.parts.push(tool(
            ToolInput::Bash {
                command: "cargo test".into(),
                timeout: None,
                workdir: None,
                run_in_background: None,
                suppress_output: None,
            },
            ToolStatus::Running,
            ToolOutput::Empty,
        ));

        let segment = shell_activity_status_segment(&[msg]).expect("shell activity segment");
        assert_eq!(segment.text, " 1 shell ");
        assert_eq!(segment.tone, StatusTone::ShellActivity);
        assert_eq!(segment.priority, 87);
    }

    #[test]
    fn shell_activity_badge_ignores_stale_background_task_without_live_metadata_regression() {
        let mut msg = ChatMessage::assistant(String::new());
        msg.parts.clear();
        msg.parts.push(tool(
            ToolInput::Bash {
                command: "sleep 60".into(),
                timeout: Some(120_000),
                workdir: None,
                run_in_background: Some(true),
                suppress_output: None,
            },
            ToolStatus::Completed,
            ToolOutput::Text(
                "Command running in background.\ntask_id: bash_abc123\nstatus: running".into(),
            ),
        ));

        assert_eq!(shell_activity_badge(&[msg]), None);
    }

    #[test]
    fn shell_activity_badge_clears_terminal_background_status_regression() {
        let mut msg = ChatMessage::assistant(String::new());
        msg.parts.clear();
        msg.parts.push(tool(
            ToolInput::Bash {
                command: "sleep 60".into(),
                timeout: Some(120_000),
                workdir: None,
                run_in_background: Some(true),
                suppress_output: None,
            },
            ToolStatus::Completed,
            ToolOutput::Text(
                "Command running in background.\ntask_id: bash_done\nstatus: timed_out after 120000ms"
                    .into(),
            ),
        ));

        assert_eq!(shell_activity_badge(&[msg]), None);
    }

    #[test]
    fn shell_activity_badge_clears_background_after_successful_read_regression() {
        let mut msg = ChatMessage::assistant(String::new());
        msg.parts.clear();
        msg.parts.push(tool(
            ToolInput::Bash {
                command: "sleep 0.1".into(),
                timeout: Some(120_000),
                workdir: None,
                run_in_background: Some(true),
                suppress_output: None,
            },
            ToolStatus::Completed,
            ToolOutput::Text("task_id: bash_done\nstatus: running".into()),
        ));
        msg.parts.push(tool(
            ToolInput::BashOutput {
                task_id: "bash_done".into(),
                offset: None,
                limit: None,
                block: None,
                timeout: None,
                wait_up_to: None,
            },
            ToolStatus::Completed,
            ToolOutput::Text("retrieval_status: success\nstatus: completed exit=0\n\nok".into()),
        ));

        assert_eq!(shell_activity_badge(&[msg]), None);
    }

    #[test]
    fn shell_activity_badge_clears_after_output_folded_into_bash_regression() {
        let mut msg = ChatMessage::assistant(String::new());
        msg.parts.clear();
        msg.parts.push(tool(
            ToolInput::Bash {
                command: "sleep 0.1".into(),
                timeout: Some(120_000),
                workdir: None,
                run_in_background: Some(true),
                suppress_output: None,
            },
            ToolStatus::Completed,
            ToolOutput::Text(
                "retrieval_status: success\n\
                 task_id: bash_done\n\
                 status: completed exit=0\n\
                 \n\
                 ok"
                .into(),
            ),
        ));

        assert_eq!(shell_activity_badge(&[msg]), None);
    }
}

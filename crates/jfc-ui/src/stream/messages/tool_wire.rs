use std::borrow::Cow;

use crate::provider::ProviderContent;
use crate::stream::tool_results::cap_tool_result;
use crate::types::{ToolCall, ToolOutput, ToolStatus};

const MICROCOMPACT_TURN_THRESHOLD: usize = 10;
const MICROCOMPACT_MAX_CHARS: usize = 500;

#[derive(Default)]
pub(super) struct ToolWireCounters {
    pub tool_use_count: usize,
    pub tool_result_count: usize,
    pub abandoned_count: usize,
}

pub(super) fn tool_use_content(tc: &ToolCall, counters: &mut ToolWireCounters) -> ProviderContent {
    counters.tool_use_count += 1;
    ProviderContent::ToolUse {
        id: tc.id.as_str().to_owned(),
        name: tc.kind.api_name().to_owned(),
        input: tc.input.to_value(),
    }
}

pub(super) fn tool_result_content(
    tc: &ToolCall,
    turns_ago: usize,
    counters: &mut ToolWireCounters,
) -> ProviderContent {
    let (result_text, is_error) = tool_result_text(tc, counters);
    let capped = cap_tool_result(&result_text);
    let content =
        if turns_ago > MICROCOMPACT_TURN_THRESHOLD && capped.len() > MICROCOMPACT_MAX_CHARS {
            let boundary = capped.floor_char_boundary(MICROCOMPACT_MAX_CHARS);
            format!(
                "{}… [older output truncated, {} chars total]",
                &capped[..boundary],
                capped.len()
            )
        } else {
            capped
        };
    ProviderContent::ToolResult {
        tool_use_id: tc.id.as_str().to_owned(),
        content,
        is_error,
    }
}

fn tool_result_text(tc: &ToolCall, counters: &mut ToolWireCounters) -> (String, bool) {
    // After ExecutionStatus unification, tools can in principle land in
    // any of six states. In practice tools never reach Idle (that's a
    // Task-only state for sub-agents that are alive but quiescent), and
    // Cancelled is treated as a flavor of "the tool was never executed".
    match tc.status {
        ToolStatus::Completed | ToolStatus::Failed => {
            counters.tool_result_count += 1;
            let text: Cow<str> = match &tc.output {
                ToolOutput::Text(s) => Cow::Borrowed(s.as_str()),
                ToolOutput::LargeText(lt) => Cow::Borrowed(lt.content.as_str()),
                ToolOutput::Command {
                    stdout,
                    stderr,
                    exit_code,
                } => Cow::Owned(format!(
                    "exit: {}\nstdout: {}\nstderr: {}",
                    exit_code.unwrap_or(-1),
                    stdout,
                    stderr
                )),
                ToolOutput::FileContent { content, .. } => Cow::Borrowed(content.as_str()),
                ToolOutput::FileList(files) => Cow::Owned(files.join("\n")),
                ToolOutput::Diff(d) => Cow::Owned(format!("Applied diff to {}", d.file_path)),
                ToolOutput::Empty => Cow::Borrowed(""),
            };
            (text.into_owned(), tc.status == ToolStatus::Failed)
        }
        ToolStatus::Cancelled => {
            counters.abandoned_count += 1;
            (
                "Tool was cancelled before it could run. No output was produced.".to_owned(),
                true,
            )
        }
        ToolStatus::Idle => {
            tracing::error!(
                target: "jfc::stream",
                tool_id = %tc.id.as_str(),
                "tool reached Idle state — should not happen"
            );
            counters.abandoned_count += 1;
            (
                "Tool was abandoned: unexpected Idle state. No output was produced.".to_owned(),
                true,
            )
        }
        ToolStatus::Pending | ToolStatus::Running => {
            counters.abandoned_count += 1;
            (
                "Tool was abandoned: the user moved on before approving or executing it. \
                 No output was produced."
                    .to_owned(),
                true,
            )
        }
    }
}

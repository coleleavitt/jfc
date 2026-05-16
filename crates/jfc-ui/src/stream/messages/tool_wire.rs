use std::borrow::Cow;

use crate::stream::tool_results::cap_tool_result;
use crate::types::{ToolCall, ToolKind, ToolOutput, ToolStatus};
use jfc_provider::ProviderContent;

const MICROCOMPACT_TURN_THRESHOLD: usize = 10;
const MICROCOMPACT_MAX_CHARS: usize = 500;

#[derive(Default)]
pub(super) struct ToolWireCounters {
    pub tool_use_count: usize,
    pub tool_result_count: usize,
    pub abandoned_count: usize,
    /// Number of `server_tool_use` blocks emitted on resend. Tracked
    /// separately so the `build_provider_messages_with_tool_results`
    /// trace log shows server-side tool round-trips distinctly from
    /// regular tool_use → tool_result pairs.
    pub server_tool_use_count: usize,
    /// Number of `server_tool_result` blocks emitted on resend (e.g.
    /// `web_search_tool_result`).
    pub server_tool_result_count: usize,
}

/// True iff the ToolCall is an Anthropic server-side tool. These do
/// NOT round-trip as plain `tool_use` + paired user `tool_result`;
/// they emit `server_tool_use` on the assistant message AND
/// `server_tool_result` on the SAME assistant message (no paired
/// user turn). See cli.js v142:7057 and :441375.
pub(super) fn is_server_tool(kind: &ToolKind) -> bool {
    matches!(
        kind,
        ToolKind::ServerWebSearch | ToolKind::ServerCodeExecution
    )
}

pub(super) fn tool_use_content(tc: &ToolCall, counters: &mut ToolWireCounters) -> ProviderContent {
    // Server-side tools (web_search, code_execution) must round-trip
    // with their wire `type: "server_tool_use"` — emitting plain
    // `tool_use` breaks Anthropic's server-side sampling loop
    // resumption. The api_name for these is the bare server-tool
    // name (e.g. "web_search"), NOT the JFC-internal
    // "server_tool_use:web_search" prefix used during the stream.
    if is_server_tool(&tc.kind) {
        counters.server_tool_use_count += 1;
        return ProviderContent::ServerToolUse {
            id: tc.id.as_str().to_owned(),
            name: server_tool_wire_name(&tc.kind),
            input: tc.input.to_value(),
        };
    }
    counters.tool_use_count += 1;
    ProviderContent::ToolUse {
        id: tc.id.as_str().to_owned(),
        name: tc.kind.api_name().to_owned(),
        input: tc.input.to_value(),
    }
}

/// Resolve the bare wire name for a server-side tool. Mirrors cli.js
/// v142:441090 — the `server_tool_use.name` field is the un-prefixed
/// tool name ("web_search", "code_execution"), unlike JFC's internal
/// `ToolKind::api_name()` which returns the namespaced
/// "server_tool_use:web_search" form for log disambiguation.
fn server_tool_wire_name(kind: &ToolKind) -> String {
    match kind {
        ToolKind::ServerWebSearch => "web_search".to_owned(),
        ToolKind::ServerCodeExecution => "code_execution".to_owned(),
        // is_server_tool gate above is the source of truth.
        _ => kind.api_name().to_owned(),
    }
}

/// Build the matching `server_tool_result` block for a completed
/// server-side ToolCall, if the runtime captured one from the stream.
///
/// Returns `None` when:
///   * the tool is not a server-side tool (caller should use
///     `tool_result_content` instead);
///   * the result hasn't arrived yet (still streaming, or upstream
///     emitted only a `server_tool_use` block with no paired result —
///     observed when `pause_turn` fires mid-loop).
///
/// The returned `ServerToolResult` rides on the SAME assistant
/// message as the originating `server_tool_use`, not a paired user
/// turn. This is the wire shape cli.js v142:441375 emits on resend.
pub(super) fn server_tool_result_content(
    tc: &ToolCall,
    counters: &mut ToolWireCounters,
) -> Option<ProviderContent> {
    if !is_server_tool(&tc.kind) {
        return None;
    }
    if let ToolOutput::ServerToolResult { tool_kind, content } = &tc.output {
        counters.server_tool_result_count += 1;
        return Some(ProviderContent::ServerToolResult {
            tool_use_id: tc.id.as_str().to_owned(),
            tool_kind: tool_kind.clone(),
            content: content.clone(),
        });
    }
    None
}

pub(super) fn tool_result_content(
    tc: &ToolCall,
    turns_ago: usize,
    counters: &mut ToolWireCounters,
) -> ProviderContent {
    // Server-side tools must NOT round-trip as a synthetic user
    // `tool_result` — that breaks Anthropic's server-side sampling
    // loop on the next request. Callers should branch on
    // `is_server_tool` and emit `server_tool_result_content` on the
    // SAME assistant message instead. Reaching here for a server
    // tool indicates a builder regression; we emit a placeholder that
    // is harmless on the wire but loudly visible in the trace log.
    if is_server_tool(&tc.kind) {
        tracing::error!(
            target: "jfc::stream",
            tool_id = %tc.id.as_str(),
            tool_kind = %tc.kind.label(),
            "tool_result_content called for server-side tool — caller should route via server_tool_result_content instead"
        );
        return ProviderContent::ToolResult {
            tool_use_id: tc.id.as_str().to_owned(),
            content: "[internal] server-side tool result mis-routed; ignore".to_owned(),
            is_error: true,
        };
    }
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
                // Unreachable under normal flow: tool_result_text is only
                // called for non-server tools (the caller guards via
                // `tool_result_content`'s is_server_tool branch above).
                // The match arm is kept exhaustive so the compiler
                // enforces that any future ToolOutput variant gets
                // considered. If we ever hit it, fall back to the
                // public display rendering — it's still correct text,
                // just verbose.
                ToolOutput::ServerToolResult { .. } => Cow::Owned(tc.output.to_display_string()),
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

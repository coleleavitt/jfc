//! Translate a persisted background-agent log into `ChatMessage`s so the
//! task view can run through the same `MessageView` renderer that drives
//! the main chat.
//!
//! The daemon log is a line-oriented stream interleaving:
//!
//!   * lifecycle markers — `[worker-running] pid=1234 …`, `[worker-exited]
//!     elapsed_ms=…`, `[started] foo`, `[cancel-requested]`, `[worktree]
//!     path=…`
//!   * progress markers   — `[tool] write`
//!   * terminal markers   — `[Completed] summary`, `[Failed] reason`,
//!     `[Cancelled] note`
//!   * raw assistant prose between markers (newline-delimited paragraphs)
//!
//! The parser ingests these as a typed `LogEntry` enum (parse-don't-validate
//! — every downstream consumer matches exhaustively) and folds them into
//! a stream of `MessagePart`s that the rich renderer already knows how to
//! draw: tool blocks, task status, prose paragraphs.
//!
//! Constructive choices the renderer relies on:
//!   * Contiguous prose lines coalesce into ONE `MessagePart::Text` (the
//!     fragmented "Let me / implement / SPIR-V lif" symptom that the
//!     legacy `task_view_body_lines` fallback produced never reaches the
//!     screen — `markdown::to_lines` wraps the whole paragraph once).
//!   * `[tool] X` between prose flushes the prose accumulator first, so
//!     "I'll write the file. [tool] Write … and then it ran." reads as
//!     prose · tool block · prose, not interleaved noise.
//!   * Pure-bookkeeping markers (`[worker-running]`, `[worktree]`, …) land
//!     in a `MessagePart::Reasoning` collapse so the panel doesn't drown in
//!     pid lines. `MessageView` already renders Reasoning collapsed-by-default.

use jfc_core::{ExecutionStatus, TaskLifecycle, TaskStatusPart, ToolInput, ToolKind};

use crate::ids::{TaskId, ToolId};
use crate::types::{ChatMessage, MessagePart, ToolCall, ToolDisplayState, ToolOutput};

/// Typed view of one persisted log line. Built once during parse; consumed
/// once during fold. Public-in-crate so unit tests can assert the
/// classification without re-implementing the prefix matcher.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum LogEntry<'a> {
    /// `[tool] <name>` — one round-trip ended with this tool name. The
    /// daemon log records only the name, not input/output, so the
    /// rendered block is a synthetic `Generic { summary }` shell.
    Tool { name: &'a str },
    /// `[Completed] …`, `[Failed] …`, `[Cancelled] …`. Terminal markers.
    Terminal {
        status: TaskLifecycle,
        body: &'a str,
    },
    /// `[worker-running] …`, `[worker-started] …`, `[worker-exited] …`,
    /// `[started] …`, `[cancel-requested]`, `[worktree] …`. Bookkeeping.
    Lifecycle(&'a str),
    /// Anything not bracketed — raw assistant prose chunk.
    Prose(&'a str),
}

impl<'a> LogEntry<'a> {
    fn classify(line: &'a str) -> Self {
        let trimmed = line.trim_end_matches(['\r', '\n']);
        // Pure-prose fast path: most lines have no leading `[`.
        if !trimmed.starts_with('[') {
            return Self::Prose(trimmed);
        }
        // `[tool] X`
        if let Some(name) = trimmed.strip_prefix("[tool] ") {
            return Self::Tool { name };
        }
        // Terminal markers — match the leading bracket label, keep the
        // remainder as the body.
        for (prefix, status) in [
            ("[Completed] ", TaskLifecycle::Completed),
            ("[Completed]", TaskLifecycle::Completed),
            ("[Failed] ", TaskLifecycle::Failed),
            ("[Failed]", TaskLifecycle::Failed),
            ("[Cancelled] ", TaskLifecycle::Cancelled),
            ("[Cancelled]", TaskLifecycle::Cancelled),
        ] {
            if let Some(body) = trimmed.strip_prefix(prefix) {
                return Self::Terminal { status, body };
            }
        }
        // Anything else with a leading bracket is treated as bookkeeping
        // — matches `[worker-running]`, `[worker-started]`, `[worker-
        // exited]`, `[started]`, `[cancel-requested]`, `[worktree]`,
        // future markers without a parser change.
        Self::Lifecycle(trimmed)
    }
}

/// Fold a slice of persisted log lines into a `ChatMessage` sequence ready
/// for `MessageView`. Returns an empty `Vec` for an empty input.
///
/// Lifecycle bookkeeping (`[started]`, `[worker-started]`, `[worker-
/// running]`, `[worker-exited]`, `[cancel-requested]`, `[worktree]`)
/// is **dropped** outright — these used to land as
/// `MessagePart::Reasoning` blobs and were rendered by `MessageView` as
/// `∴ Thinking — [started] foo [worker-started] pid=…`, which is
/// misleading (no model thinking is happening; it's process spawn
/// chatter). The agent transcript should show prose, tool calls, and
/// terminal status — nothing else.
pub(crate) fn parse_agent_log_to_chat_messages(lines: &[String]) -> Vec<ChatMessage> {
    let mut out: Vec<ChatMessage> = Vec::new();
    let mut prose_buf = String::new();
    let mut tool_idx: u32 = 0;
    // Coalesce a run of consecutive identical tool calls into one row.
    // A replayed agent that wrote 6 files logs 6 `Write` lines; rendering
    // them as 6 near-identical "● Write replayed" rows is pure noise.
    // `(name, count)` accumulates the run; flushed as `Write ×6` when the
    // run ends (a different tool, prose, or terminal status).
    let mut pending_tool: Option<(String, u32)> = None;

    let flush_prose = |out: &mut Vec<ChatMessage>, buf: &mut String| {
        if buf.is_empty() {
            return;
        }
        let trimmed = buf.trim_end_matches('\n').to_owned();
        if !trimmed.is_empty() {
            out.push(ChatMessage::assistant_parts(vec![MessagePart::Text(
                trimmed,
            )]));
        }
        buf.clear();
    };

    let flush_tool =
        |out: &mut Vec<ChatMessage>, pending: &mut Option<(String, u32)>, tool_idx: &mut u32| {
            let Some((name, count)) = pending.take() else {
                return;
            };
            let kind = ToolKind::from_name(&name);
            // Terse summary; `×N` when the run collapsed more than one
            // call. The old "(persisted log: tool ran — input/output not
            // recorded)" boilerplate said nothing and repeated per row.
            let summary = if count > 1 {
                format!("×{count} · replayed")
            } else {
                "replayed".to_owned()
            };
            let tool = ToolCall {
                id: ToolId::from(format!("agent-log-{tool_idx}")),
                kind,
                status: ExecutionStatus::Completed,
                input: ToolInput::Generic { summary },
                output: ToolOutput::Empty,
                display: ToolDisplayState::DEFAULT,
                elapsed_ms: None,
                started_at: None,
                thought_signature: None,
            };
            *tool_idx = tool_idx.saturating_add(1);
            out.push(ChatMessage::assistant_parts(vec![MessagePart::tool_boxed(
                Box::new(tool),
            )]));
        };

    for line in lines {
        match LogEntry::classify(line) {
            LogEntry::Prose(s) => {
                flush_tool(&mut out, &mut pending_tool, &mut tool_idx);
                prose_buf.push_str(s);
                prose_buf.push('\n');
            }
            LogEntry::Tool { name } => {
                flush_prose(&mut out, &mut prose_buf);
                match pending_tool.as_mut() {
                    // Same tool as the run in progress → just count it.
                    Some((pending_name, count)) if pending_name == name => {
                        *count += 1;
                    }
                    // Different tool → flush the old run, start a new one.
                    _ => {
                        flush_tool(&mut out, &mut pending_tool, &mut tool_idx);
                        pending_tool = Some((name.to_owned(), 1));
                    }
                }
            }
            LogEntry::Terminal { status, body } => {
                flush_tool(&mut out, &mut pending_tool, &mut tool_idx);
                flush_prose(&mut out, &mut prose_buf);
                let (summary, error) = match status {
                    TaskLifecycle::Failed | TaskLifecycle::Cancelled => {
                        (None, Some(body.to_owned()))
                    }
                    _ => (Some(body.to_owned()), None),
                };
                out.push(ChatMessage::assistant_parts(vec![MessagePart::TaskStatus(
                    TaskStatusPart {
                        task_id: TaskId::from("agent-log"),
                        description: "agent".to_owned(),
                        status,
                        summary,
                        error,
                        elapsed_ms: None,
                        model: None,
                    },
                )]));
            }
            LogEntry::Lifecycle(_) => {
                // Dropped intentionally — see fn doc.
            }
        }
    }
    flush_tool(&mut out, &mut pending_tool, &mut tool_idx);
    flush_prose(&mut out, &mut prose_buf);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(raw: &[&str]) -> Vec<String> {
        raw.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn empty_input_yields_empty_output() {
        assert!(parse_agent_log_to_chat_messages(&[]).is_empty());
    }

    #[test]
    fn consecutive_prose_coalesces_into_single_text_part() {
        // Mirrors the bug from the screenshot: each SSE chunk landed on
        // its own line; the parser must coalesce them into one Text.
        let input = lines(&[
            "Let me implement",
            "the full SPIR-V lifter with",
            "20 opcodes and 20 tests:",
        ]);
        let out = parse_agent_log_to_chat_messages(&input);
        assert_eq!(out.len(), 1);
        match &out[0].parts[..] {
            [MessagePart::Text(s)] => {
                assert!(s.contains("Let me implement"));
                assert!(s.contains("20 opcodes and 20 tests:"));
                assert_eq!(s.matches('\n').count(), 2);
            }
            other => panic!("expected single Text part, got {other:?}"),
        }
    }

    #[test]
    fn tool_marker_between_prose_splits_into_three_messages() {
        let input = lines(&["I'll write the file.", "[tool] write", "And then it ran."]);
        let out = parse_agent_log_to_chat_messages(&input);
        assert_eq!(out.len(), 3);
        assert!(matches!(out[0].parts[0], MessagePart::Text(_)));
        assert!(matches!(out[1].parts[0], MessagePart::Tool(_)));
        assert!(matches!(out[2].parts[0], MessagePart::Text(_)));
    }

    #[test]
    fn completed_marker_emits_task_status_with_summary() {
        let input = lines(&["work done", "[Completed] all good"]);
        let out = parse_agent_log_to_chat_messages(&input);
        assert_eq!(out.len(), 2);
        match &out[1].parts[0] {
            MessagePart::TaskStatus(ts) => {
                assert_eq!(ts.status, TaskLifecycle::Completed);
                assert_eq!(ts.summary.as_deref(), Some("all good"));
                assert!(ts.error.is_none());
            }
            other => panic!("expected TaskStatus, got {other:?}"),
        }
    }

    #[test]
    fn failed_marker_routes_body_into_error_field() {
        let input = lines(&["[Failed] cwd missing"]);
        let out = parse_agent_log_to_chat_messages(&input);
        match &out[0].parts[0] {
            MessagePart::TaskStatus(ts) => {
                assert_eq!(ts.status, TaskLifecycle::Failed);
                assert_eq!(ts.error.as_deref(), Some("cwd missing"));
                assert!(ts.summary.is_none());
            }
            other => panic!("expected TaskStatus(Failed), got {other:?}"),
        }
    }

    #[test]
    fn lifecycle_markers_collapse_into_reasoning() {
        let input = lines(&[
            "[worker-running] pid=42 provider=anthropic cwd=/tmp",
            "[worktree] created at /tmp/wt",
            "actual work",
        ]);
        let out = parse_agent_log_to_chat_messages(&input);
        // Lifecycle markers are dropped (bookkeeping noise); only prose remains.
        assert_eq!(out.len(), 1);
        assert!(matches!(&out[0].parts[0], MessagePart::Text(t) if t == "actual work"));
    }

    #[test]
    fn classify_terminal_without_trailing_space_still_matches() {
        // Edge case: the registry writes `[cancel-requested]` with no
        // body — should classify as Lifecycle, not as a malformed
        // Terminal.
        assert!(matches!(
            LogEntry::classify("[cancel-requested]"),
            LogEntry::Lifecycle(_)
        ));
        // But `[Completed]` (no body) still maps to a Completed terminal.
        assert!(matches!(
            LogEntry::classify("[Completed]"),
            LogEntry::Terminal {
                status: TaskLifecycle::Completed,
                ..
            }
        ));
    }
}

//! Microcompaction — a cheap, no-LLM context trim.
//!
//! Full compaction (`compact::compact`) summarizes the transcript via a model
//! call, which is expensive and loses fidelity. Microcompaction is the
//! lighter-weight pass Claude Code runs at boundaries: it truncates the
//! *oldest, largest* tool-result text blocks in place — keeping a head + tail
//! window and a truncation marker — without any provider call.
//!
//! This reclaims the bulk of the context that big Read/Bash/Grep outputs
//! occupy (which dominate a long agentic session) while preserving recent tool
//! results verbatim, so the model still sees what it just did. It runs at a
//! lower threshold than full compaction and defers (or avoids) the costly
//! summarization pass.

use crate::types::{ChatMessage, MessagePart};
use jfc_core::ToolOutput;

use super::{CHARS_PER_TOKEN, CompactLevel};
use tracing::{debug, info};

/// A tool result larger than this many chars is a microcompaction candidate.
/// Smaller results aren't worth the fidelity loss.
const MICROCOMPACT_MIN_CHARS: usize = 2_000;
/// Chars of the head kept verbatim when a tool result is truncated.
const KEEP_HEAD_CHARS: usize = 600;
/// Chars of the tail kept verbatim (errors/summaries often live at the end).
const KEEP_TAIL_CHARS: usize = 400;
/// The newest N messages are never microcompacted — recent tool results are
/// what the model is actively reasoning about.
const PROTECT_RECENT_MESSAGES: usize = 12;

/// Truncate one large tool-output string to a head + marker + tail window.
/// Returns `None` when the text is already small enough to leave untouched.
fn truncate_middle(text: &str) -> Option<String> {
    let len = text.chars().count();
    if len <= MICROCOMPACT_MIN_CHARS {
        return None;
    }
    let head: String = text.chars().take(KEEP_HEAD_CHARS).collect();
    let tail: String = text
        .chars()
        .skip(len.saturating_sub(KEEP_TAIL_CHARS))
        .collect();
    let dropped = len - KEEP_HEAD_CHARS - KEEP_TAIL_CHARS;
    Some(format!(
        "{head}\n\n[… {dropped} chars elided by microcompaction …]\n\n{tail}"
    ))
}

/// Truncate the high-volume textual field of one tool output in place,
/// returning the chars saved (0 if nothing was trimmed). Structured outputs
/// (diffs, file lists, server-tool results) carry no large free-text field and
/// are intentionally left untouched so their wire shape round-trips faithfully.
fn trim_tool_output(output: &mut ToolOutput) -> usize {
    // Borrow the mutable string field this output type exposes, if any.
    let field: Option<&mut String> = match output {
        ToolOutput::Text(s) => Some(s),
        ToolOutput::Command { stdout, .. } => Some(stdout),
        ToolOutput::FileContent { content, .. } => Some(content),
        // LargeText carries derived line/byte counts that would desync if we
        // truncated its content; it's already a collapse-managed large output,
        // so leave it to the existing collapse path. Structured / non-textual
        // outputs have no large free-text field.
        ToolOutput::LargeText(_)
        | ToolOutput::Diff(_)
        | ToolOutput::FileList(_)
        | ToolOutput::ServerToolResult { .. }
        | ToolOutput::Empty => None,
    };
    let Some(text) = field else {
        return 0;
    };
    let Some(new) = truncate_middle(text) else {
        return 0;
    };
    let saved = text.chars().count() - new.chars().count();
    *text = new;
    saved
}

/// Apply microcompaction in place to `messages`, truncating the oldest large
/// tool-result text blocks. Returns the number of characters reclaimed.
///
/// Only the high-volume free-text outputs (Text, Command stdout, FileContent)
/// are touched; structured and collapse-managed (LargeText) outputs are left
/// intact so their wire shape / derived counts stay consistent. The newest
/// [`PROTECT_RECENT_MESSAGES`] messages are skipped.
pub fn microcompact(messages: &mut [ChatMessage]) -> usize {
    if messages.len() <= PROTECT_RECENT_MESSAGES {
        return 0;
    }
    let cutoff = messages.len() - PROTECT_RECENT_MESSAGES;
    let mut saved = 0usize;
    let mut trimmed = 0usize;

    for msg in messages[..cutoff].iter_mut() {
        for part in msg.parts.iter_mut() {
            let MessagePart::Tool(tc) = part else {
                continue;
            };
            let s = trim_tool_output(&mut tc.output);
            if s > 0 {
                saved += s;
                trimmed += 1;
            }
        }
    }

    if trimmed > 0 {
        info!(
            target: "jfc::compact::micro",
            trimmed_blocks = trimmed,
            chars_saved = saved,
            protected_recent = PROTECT_RECENT_MESSAGES,
            "microcompaction trimmed old tool results"
        );
    } else {
        debug!(target: "jfc::compact::micro", "microcompaction found nothing to trim");
    }
    saved
}


/// Whether microcompaction would reclaim a meaningful amount of context — used
/// to decide whether to run the (cheap) pass before falling back to full
/// compaction. Returns the estimated reclaimable chars without mutating.
/// Minimum approximate tokens that must be recoverable before the micro pass
/// runs. Avoids churning the transcript for tiny savings.
const MIN_SAVINGS_TOKENS: usize = 4_000;

/// Run microcompaction only when it is likely to be useful at the current
/// context-pressure level. Updates `approx_tokens` by the estimated token
/// savings and returns the number of tokens saved. Returns 0 when skipped.
pub fn microcompact_if_helpful(
    messages: &mut [ChatMessage],
    approx_tokens: &mut usize,
    level: CompactLevel,
) -> usize {
    if !matches!(
        level,
        CompactLevel::Warn | CompactLevel::Compact | CompactLevel::Blocked
    ) {
        return 0;
    }
    let savings_chars = microcompact_savings(messages);
    let savings_tokens = savings_chars / CHARS_PER_TOKEN;
    if savings_tokens < MIN_SAVINGS_TOKENS {
        return 0;
    }
    let actual_chars = microcompact(messages);
    let actual_tokens = actual_chars / CHARS_PER_TOKEN;
    *approx_tokens = approx_tokens.saturating_sub(actual_tokens);
    actual_tokens
}

pub fn microcompact_savings(messages: &[ChatMessage]) -> usize {
    if messages.len() <= PROTECT_RECENT_MESSAGES {
        return 0;
    }
    let cutoff = messages.len() - PROTECT_RECENT_MESSAGES;
    let mut saved = 0usize;
    for msg in &messages[..cutoff] {
        for part in &msg.parts {
            let MessagePart::Tool(tc) = part else {
                continue;
            };
            let text = match &tc.output {
                ToolOutput::Text(s) => Some(s.as_str()),
                ToolOutput::Command { stdout, .. } => Some(stdout.as_str()),
                ToolOutput::FileContent { content, .. } => Some(content.as_str()),
                _ => None,
            };
            if let Some(t) = text {
                let len = t.chars().count();
                if len > MICROCOMPACT_MIN_CHARS {
                    saved += len - KEEP_HEAD_CHARS - KEEP_TAIL_CHARS;
                }
            }
        }
    }
    saved
}

#[cfg(test)]
mod tests {
    use super::*;
    use jfc_core::{ToolCall, ToolKind};

    fn tool_msg(output: ToolOutput) -> ChatMessage {
        let input = jfc_core::ToolInput::from_value(
            "Bash",
            serde_json::json!({"command":"echo test"}),
        )
        .expect("valid bash input");
        let mut tc = ToolCall::new_pending("tool_1".into(), ToolKind::Bash, input);
        tc.output = output;
        let mut m = ChatMessage::assistant(String::new());
        m.parts = vec![MessagePart::Tool(Box::new(tc))];
        m
    }

    fn padding(n: usize) -> Vec<ChatMessage> {
        (0..n).map(|_| ChatMessage::user("x".to_owned())).collect()
    }

    #[test]
    fn truncate_middle_keeps_head_and_tail_normal() {
        let text = "A".repeat(5_000);
        let out = truncate_middle(&text).expect("should truncate");
        assert!(out.contains("elided by microcompaction"));
        assert!(out.chars().count() < text.chars().count());
        // Head + tail preserved.
        assert!(out.starts_with(&"A".repeat(KEEP_HEAD_CHARS)));
        assert!(out.ends_with(&"A".repeat(KEEP_TAIL_CHARS)));
    }

    #[test]
    fn truncate_middle_leaves_small_text_robust() {
        assert!(truncate_middle("short output").is_none());
        assert!(truncate_middle(&"A".repeat(MICROCOMPACT_MIN_CHARS)).is_none());
    }

    #[test]
    fn microcompact_trims_old_large_tool_result_normal() {
        // One old message with a big Text output, then enough recent padding to
        // push it past the protected window.
        let mut messages = vec![tool_msg(ToolOutput::Text("B".repeat(8_000)))];
        messages.extend(padding(PROTECT_RECENT_MESSAGES + 1));
        let before = super::microcompact_savings(&messages);
        assert!(before > 0, "should see reclaimable savings");

        let saved = microcompact(&mut messages);
        assert!(saved > 0);
        // The old tool result is now truncated.
        if let MessagePart::Tool(tc) = &messages[0].parts[0] {
            if let ToolOutput::Text(s) = &tc.output {
                assert!(s.contains("elided by microcompaction"));
                assert!(s.chars().count() < 8_000);
            } else {
                panic!("expected Text output");
            }
        } else {
            panic!("expected Tool part");
        }
    }

    #[test]
    fn microcompact_protects_recent_messages_robust() {
        // A big tool result within the protected recent window is NOT trimmed.
        let mut messages = padding(2);
        messages.push(tool_msg(ToolOutput::Text("C".repeat(8_000))));
        // Total messages <= PROTECT_RECENT_MESSAGES → nothing trimmed.
        let saved = microcompact(&mut messages);
        assert_eq!(saved, 0);
    }

    #[test]
    fn microcompact_noop_on_small_transcript_robust() {
        let mut messages = padding(3);
        assert_eq!(microcompact(&mut messages), 0);
    }
}

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
/// Chars of the head kept verbatim when a tool result is truncated (depth 0).
const KEEP_HEAD_CHARS: usize = 600;
/// Chars of the tail kept verbatim (errors/summaries often live at the end).
const KEEP_TAIL_CHARS: usize = 400;
/// The newest N messages are never microcompacted — recent tool results are
/// what the model is actively reasoning about.
const PROTECT_RECENT_MESSAGES: usize = 12;
/// Age-tiered compression (ported from magic-context's "caveman" depth tiers):
/// the older a tool result is, the smaller the verbatim window we keep. Each
/// depth tier past 0 shrinks the kept head+tail by this factor, so ancient
/// outputs are reduced harder than recent-but-unprotected ones. A floor keeps
/// every tier readable. `MAX_DEPTH` bounds how aggressive the oldest tier gets.
const DEPTH_SHRINK_NUM: usize = 2;
const DEPTH_SHRINK_DEN: usize = 3;
const MIN_HEAD_CHARS: usize = 160;
const MIN_TAIL_CHARS: usize = 120;
const MAX_DEPTH: usize = 4;
/// How many older messages share one depth tier. Messages are bucketed by
/// distance before the protect window: the oldest tiers compress hardest.
const MESSAGES_PER_DEPTH: usize = 8;

/// Keep window (head, tail) chars for a given age `depth`. Depth 0 = the
/// newest-eligible tier (full window); each tier multiplies by 2/3 down to a
/// readable floor.
fn keep_window(depth: usize) -> (usize, usize) {
    let depth = depth.min(MAX_DEPTH);
    let mut head = KEEP_HEAD_CHARS;
    let mut tail = KEEP_TAIL_CHARS;
    for _ in 0..depth {
        head = head * DEPTH_SHRINK_NUM / DEPTH_SHRINK_DEN;
        tail = tail * DEPTH_SHRINK_NUM / DEPTH_SHRINK_DEN;
    }
    (head.max(MIN_HEAD_CHARS), tail.max(MIN_TAIL_CHARS))
}

/// Truncate one large tool-output string to a head + marker + tail window,
/// where the window size shrinks with age `depth` (0 = newest-eligible). Returns
/// `None` when the text is already at or below the kept window for that depth.
///
/// Within that same age-tiered budget, this routes through
/// [`jfc_compress::compress_tool_output`]: build/test logs, grep/search
/// output, and unified diffs keep their *important* lines (errors, fails,
/// summaries, changed hunks) instead of a blind positional head/tail cut
/// that can elide a fatal error sitting in the middle of a 10k-line log.
/// Content with no specialized compressor (prose/source/JSON/HTML) — and
/// any case where content-aware compression wouldn't beat the window —
/// falls back to the original head/tail behavior, so this is never worse.
fn truncate_middle_at_depth(text: &str, depth: usize) -> Option<String> {
    let len = text.chars().count();
    if len <= MICROCOMPACT_MIN_CHARS {
        return None;
    }
    let (keep_head, keep_tail) = keep_window(depth);
    // Nothing to gain if the window already covers the text.
    if len <= keep_head + keep_tail {
        return None;
    }
    let out = jfc_compress::compress_tool_output(text, keep_head, keep_tail, "");
    // `compress_tool_output` returns the input verbatim when it's already
    // under budget; we've established it isn't, so any verbatim return means
    // nothing was reclaimed — treat as "no change".
    if out.compressed_chars >= len {
        return None;
    }
    Some(out.text)
}

/// Age depth for a message at index `idx` given the compaction `cutoff` (the
/// first protected index). Index 0 is the oldest → highest depth; messages just
/// before the cutoff → depth 0. Buckets of [`MESSAGES_PER_DEPTH`].
fn depth_for_index(idx: usize, cutoff: usize) -> usize {
    let distance_from_cutoff = cutoff.saturating_sub(idx + 1);
    (distance_from_cutoff / MESSAGES_PER_DEPTH).min(MAX_DEPTH)
}

/// Truncate the high-volume textual field of one tool output in place,
/// returning the chars saved (0 if nothing was trimmed). Structured outputs
/// (diffs, file lists, server-tool results) carry no large free-text field and
/// are intentionally left untouched so their wire shape round-trips faithfully.
fn trim_tool_output(output: &mut ToolOutput, depth: usize) -> usize {
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
    let Some(new) = truncate_middle_at_depth(text, depth) else {
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

    for (idx, msg) in messages[..cutoff].iter_mut().enumerate() {
        // Older messages compress to a smaller window (age-tiered depth).
        let depth = depth_for_index(idx, cutoff);
        for part in msg.parts.iter_mut() {
            let MessagePart::Tool(tc) = part else {
                continue;
            };
            let s = trim_tool_output(&mut tc.output, depth);
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
    for (idx, msg) in messages[..cutoff].iter().enumerate() {
        let (keep_head, keep_tail) = keep_window(depth_for_index(idx, cutoff));
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
                if len > MICROCOMPACT_MIN_CHARS && len > keep_head + keep_tail {
                    saved += len - keep_head - keep_tail;
                }
            }
        }
    }
    saved
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Test helper: depth-0 (full-window) truncation, the original behavior.
    fn truncate_middle(text: &str) -> Option<String> {
        truncate_middle_at_depth(text, 0)
    }
    use jfc_core::{ToolCall, ToolKind};

    fn tool_msg(output: ToolOutput) -> ChatMessage {
        let input =
            jfc_core::ToolInput::from_value("Bash", serde_json::json!({"command":"echo test"}))
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

    // Content-aware: a build error in the MIDDLE of a long log survives
    // microcompaction, where the old blind head/tail cut would have elided
    // it. This is the headline win of the jfc-compress port.
    #[test]
    fn microcompact_keeps_mid_log_error_robust() {
        // Realistic cargo/pytest-style run: status/warn lines throughout so
        // the content detector recognizes it as build output, with one fatal
        // error in the middle.
        let mut lines = Vec::with_capacity(400);
        for i in 0..400 {
            if i == 200 {
                lines.push("error[E0599]: no method named `frobnicate` for `Widget`".to_owned());
            } else if i % 5 == 0 {
                lines.push(format!("[INFO] test_case_{i} ... ok"));
            } else if i % 7 == 0 {
                lines.push(format!("WARNING: deprecated API in module_{i}"));
            } else {
                lines.push(format!("   Compiling crate_{i} v0.1.0"));
            }
        }
        let log = lines.join("\n");

        // Precondition: a depth-0 blind head/tail cut drops the mid-log error.
        let (keep_head, keep_tail) = keep_window(0);
        let head: String = log.chars().take(keep_head).collect();
        let tail: String = log.chars().skip(log.chars().count() - keep_tail).collect();
        assert!(
            !head.contains("E0599") && !tail.contains("E0599"),
            "precondition: blind head/tail must drop the mid-log error"
        );

        let mut messages = vec![tool_msg(ToolOutput::Text(log.clone()))];
        messages.extend(padding(PROTECT_RECENT_MESSAGES + 1));
        let saved = microcompact(&mut messages);
        assert!(saved > 0, "should reclaim chars");

        let MessagePart::Tool(tc) = &messages[0].parts[0] else {
            panic!("expected Tool part");
        };
        let ToolOutput::Text(s) = &tc.output else {
            panic!("expected Text output");
        };
        assert!(
            s.contains("E0599"),
            "content-aware microcompaction must keep the mid-log error"
        );
        assert!(s.chars().count() < log.chars().count(), "must compress");
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

    // ── Age-tiered depth (magic-context caveman parity) ──────────────────

    // Normal: keep_window shrinks monotonically with depth, down to a floor.
    #[test]
    fn keep_window_shrinks_with_depth_normal() {
        let (h0, t0) = keep_window(0);
        let (h1, t1) = keep_window(1);
        let (hmax, tmax) = keep_window(MAX_DEPTH);
        assert_eq!((h0, t0), (KEEP_HEAD_CHARS, KEEP_TAIL_CHARS));
        assert!(h1 < h0 && t1 < t0, "depth 1 keeps less than depth 0");
        assert!(
            hmax >= MIN_HEAD_CHARS && tmax >= MIN_TAIL_CHARS,
            "never below the floor"
        );
        // Beyond MAX_DEPTH is clamped (no further shrink / no panic).
        assert_eq!(keep_window(MAX_DEPTH + 5), keep_window(MAX_DEPTH));
    }

    // Normal: depth_for_index increases for older messages (smaller index).
    #[test]
    fn depth_for_index_increases_with_age_normal() {
        let cutoff = 40;
        // Just before the cutoff → depth 0; far older → higher depth.
        assert_eq!(depth_for_index(cutoff - 1, cutoff), 0);
        assert!(depth_for_index(0, cutoff) > depth_for_index(cutoff - 1, cutoff));
        // Clamped at MAX_DEPTH for arbitrarily old messages.
        assert!(depth_for_index(0, 10_000) <= MAX_DEPTH);
    }

    // Robust: an OLD large tool result is compressed harder (smaller kept head)
    // than a RECENT-but-unprotected one of the same size.
    #[test]
    fn older_results_compress_harder_robust() {
        // Oldest message is the big one; fill many messages between it and the
        // protect window so it lands in a high depth tier.
        let mut old_first = vec![tool_msg(ToolOutput::Text("Z".repeat(8_000)))];
        old_first.extend(padding(
            MESSAGES_PER_DEPTH * MAX_DEPTH + PROTECT_RECENT_MESSAGES + 2,
        ));
        microcompact(&mut old_first);
        let old_kept = match &old_first[0].parts[0] {
            MessagePart::Tool(tc) => match &tc.output {
                ToolOutput::Text(s) => s.chars().count(),
                _ => panic!("text"),
            },
            _ => panic!("tool"),
        };

        // Same big result placed just before the protect window → depth 0.
        let mut recent = padding(2);
        recent.push(tool_msg(ToolOutput::Text("Z".repeat(8_000))));
        recent.extend(padding(PROTECT_RECENT_MESSAGES + 1));
        microcompact(&mut recent);
        let recent_kept = match &recent[2].parts[0] {
            MessagePart::Tool(tc) => match &tc.output {
                ToolOutput::Text(s) => s.chars().count(),
                _ => panic!("text"),
            },
            _ => panic!("tool"),
        };

        assert!(
            old_kept < recent_kept,
            "older result ({old_kept}) must keep less than a recent one ({recent_kept})"
        );
    }
}

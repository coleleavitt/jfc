use std::{collections::HashMap, sync::Arc, time::Duration};

use futures::StreamExt;
use tokio::sync::{Mutex, mpsc};

use crate::app::{App, AppEvent};
use crate::context::ReadDedupCache;
use crate::provider::{
    ModelId, Provider, ProviderContent, ProviderMessage, ProviderRole, StopReason, StreamEvent,
    StreamOptions,
};
use crate::scheduler;
use crate::tools;
use crate::types::*;

/// Reuse the same cap that `ToolOutput::approx_text_len` enforces — the wire
/// truncation here and the local token estimate must agree to a byte, or
/// `compact_level` will fire on phantom-large outputs that the API never sees.
pub(crate) const MAX_TOOL_RESULT_CHARS: usize = ToolOutput::APPROX_LEN_CAP;

/// Bytes shown at each end of a truncated tool result. v131 Claude Code
/// uses 2000 chars (`ImH = 2e3`) for its persisted-output preview; we
/// mirror that so the model gets a recognizable amount of head context.
pub(crate) const TRUNCATION_PREVIEW_CHARS: usize = 2_000;

/// Tool results above this size get spilled to a temp file on disk
/// instead of being held entirely in memory + the conversation. v131
/// uses 400_000 bytes as `EIK` for the same gate. Below this limit,
/// the in-memory `truncate_tool_result` path applies (50KB cap, head
/// + tail preview).
pub(crate) const TOOL_RESULT_DISK_PERSIST_BYTES: usize = 400_000;
const STREAM_INTERRUPT_POLL: Duration = Duration::from_millis(50);

/// Persist `body` to a temp file under `/tmp/jfc-tool-results/` and
/// return a v131-style `<persisted-output>` reference the model can
/// read. The reference includes the original byte count, the
/// absolute on-disk path (so the model can `Read` the full output if
/// it really needs it), and a 2000-char head preview so the common
/// case ("just check the start") doesn't require an extra tool call.
///
/// On any I/O failure (full disk, read-only /tmp, ENOSPC) the
/// fallback is `truncate_tool_result(body)` — better to ship a
/// truncated in-line version than to silently drop the result.
pub(crate) fn persist_tool_result(body: &str) -> String {
    use std::io::Write as _;
    let dir = std::env::temp_dir().join("jfc-tool-results");
    if let Err(e) = std::fs::create_dir_all(&dir) {
        tracing::warn!(
            target: "jfc::stream",
            error = %e,
            "failed to create tool-result spill dir, falling back to in-memory truncation"
        );
        return truncate_tool_result(body);
    }
    let id = uuid::Uuid::new_v4().simple().to_string();
    let path = dir.join(format!("{id}.txt"));
    let file_open = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path);
    let mut file = match file_open {
        Ok(f) => f,
        Err(e) => {
            tracing::warn!(
                target: "jfc::stream",
                path = %path.display(),
                error = %e,
                "failed to create tool-result spill file, falling back"
            );
            return truncate_tool_result(body);
        }
    };
    if let Err(e) = file.write_all(body.as_bytes()) {
        tracing::warn!(
            target: "jfc::stream",
            path = %path.display(),
            error = %e,
            "failed to write tool-result spill, falling back"
        );
        let _ = std::fs::remove_file(&path);
        return truncate_tool_result(body);
    }
    let preview_end = floor_char_boundary(body, TRUNCATION_PREVIEW_CHARS);
    let preview = &body[..preview_end];
    let total = body.len();
    format!(
        "<persisted-output original_bytes=\"{total}\" path=\"{}\">\n\
         Output too large for inline conversation ({total} bytes). \
         Full output saved to: {}\n\n\
         Preview (first {preview_end} chars):\n\
         {preview}\n…\n\
         </persisted-output>",
        path.display(),
        path.display()
    )
}

/// Apply the appropriate cap to a tool result: spill to disk above
/// 400KB, head/tail truncate above 50KB, otherwise pass through.
/// This is the single entry point callers should use — it picks the
/// right strategy based on size.
pub(crate) fn cap_tool_result(body: &str) -> String {
    if body.len() > TOOL_RESULT_DISK_PERSIST_BYTES {
        persist_tool_result(body)
    } else {
        truncate_tool_result(body)
    }
}

/// Truncate `s` to at most `MAX_TOOL_RESULT_CHARS` bytes when oversized.
/// The marker mirrors v131's `<persisted-output>` structure (without
/// disk persistence — preview only): first 2000 chars, then a tagged
/// note disclosing the original byte count and how much was dropped,
/// then the last 2000 chars. Slice boundaries are snapped to UTF-8
/// codepoints so this can't panic on emoji/multi-byte content (the
/// fix for the panic at stream.rs:334:14 from
/// `build_provider_messages_with_tool_results`' FilterMap closure).
pub(crate) fn truncate_tool_result(s: &str) -> String {
    if s.len() <= MAX_TOOL_RESULT_CHARS {
        return s.to_owned();
    }
    let preview = TRUNCATION_PREVIEW_CHARS.min(MAX_TOOL_RESULT_CHARS / 2);
    let head_end = floor_char_boundary(s, preview);
    let tail_start = ceil_char_boundary(s, s.len().saturating_sub(preview));
    let head = &s[..head_end];
    let tail = &s[tail_start..];
    let omitted = s.len() - head_end - (s.len() - tail_start);
    let total = s.len();
    format!(
        "<truncated-output original_bytes=\"{total}\" omitted_bytes=\"{omitted}\">\n\
         Output too large for the conversation. Showing first {preview} \
         chars and last {preview} chars; {omitted} bytes omitted from the \
         middle. If you need the elided section, ask the user or re-invoke \
         the tool with a narrower scope (smaller path / line range / Grep \
         pattern).\n\n\
         --- preview head ---\n\
         {head}\n\
         --- preview tail ---\n\
         {tail}\n\
         </truncated-output>"
    )
}

/// Round `i` down to the nearest UTF-8 char boundary in `s`. `str::is_char_boundary`
/// is true at byte 0 and `s.len()`, plus every codepoint boundary in between —
/// so the loop terminates in O(4) steps for any valid UTF-8.
fn floor_char_boundary(s: &str, mut i: usize) -> usize {
    if i >= s.len() {
        return s.len();
    }
    while i > 0 && !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

/// Round `i` up to the nearest UTF-8 char boundary in `s`.
fn ceil_char_boundary(s: &str, mut i: usize) -> usize {
    if i >= s.len() {
        return s.len();
    }
    while i < s.len() && !s.is_char_boundary(i) {
        i += 1;
    }
    i
}

/// Pull the most recent user-role text out of a provider message vec. Used by
/// the memory-recall pass to know what query the user actually asked. Returns
/// `None` when the conversation is empty or the last user turn carried only
/// tool results (no plain text). Concatenates multiple text blocks in the
/// same message with newlines so multi-paragraph prompts survive intact.
fn last_user_text(messages: &[ProviderMessage]) -> Option<String> {
    for msg in messages.iter().rev() {
        if msg.role != ProviderRole::User {
            continue;
        }
        let mut buf = String::new();
        for c in &msg.content {
            if let ProviderContent::Text(t) = c {
                if !buf.is_empty() {
                    buf.push('\n');
                }
                buf.push_str(t);
            }
        }
        if !buf.trim().is_empty() {
            return Some(buf);
        }
    }
    None
}

/// Mirrors v131 Claude Code's `Yd6 = 1e5` constant — auto-compaction
/// triggers when the running subagent transcript crosses this many
/// estimated tokens. We multiply by `BYTES_PER_TOKEN` to convert to a
/// byte threshold (the unit our `estimate_provider_message_bytes`
/// returns). 100k tokens ≈ 400KB at 4 chars/tok; v131 uses the same
/// figure for both main-loop and subagent compaction.
pub(crate) const SUBAGENT_AUTO_COMPACT_TOKEN_THRESHOLD: usize = 100_000;

/// Same chars-per-token heuristic v131 uses (`z_$ = 4`).
pub(crate) const BYTES_PER_TOKEN: usize = 4;

/// Verbatim summary prompt from v131 deob (`fd6` constant in
/// cli.2.1.131.beautified.js). Kept word-for-word so subagent
/// summaries match the structure Claude Code's main loop produces —
/// future Claude Code releases that tweak this template are easy to
/// re-port.
pub(crate) const SUBAGENT_AUTO_COMPACT_PROMPT: &str = "\
You have been working on the task described above but have not yet completed it. \
Write a continuation summary that will allow you (or another instance of yourself) \
to resume work efficiently in a future context window where the conversation history \
will be replaced with this summary. Your summary should be structured, concise, and \
actionable. Include:\n\
1. Task Overview\n\
The user's core request and success criteria\n\
Any clarifications or constraints they specified\n\
2. Current State\n\
What has been completed so far\n\
Files created, modified, or analyzed (with paths if relevant)\n\
Key outputs or artifacts produced\n\
3. Important Discoveries\n\
Technical constraints or requirements uncovered\n\
Decisions made and their rationale\n\
Errors encountered and how they were resolved\n\
What approaches were tried that didn't work (and why)\n\
4. Next Steps\n\
Specific actions needed to complete the task\n\
Any blockers or open questions to resolve\n\
\n\
Wrap the entire summary in <summary>...</summary> tags so it can be parsed.";

/// Render a provider message as plain text for inclusion in a summary
/// request. Tool calls and tool results are flattened to a one-line
/// description so the summary model sees the *shape* of what the
/// subagent did without the full payload (which is what we're trying
/// to compress in the first place).
pub(crate) fn render_message_as_text(msg: &crate::provider::ProviderMessage) -> String {
    use crate::provider::{ProviderContent, ProviderRole};
    let role = match msg.role {
        ProviderRole::User => "user",
        ProviderRole::Assistant => "assistant",
    };
    let mut out = format!("[{role}] ");
    for c in &msg.content {
        match c {
            ProviderContent::Text(t) => out.push_str(t),
            ProviderContent::ToolUse { name, input, .. } => {
                let preview = serde_json::to_string(input)
                    .unwrap_or_default()
                    .chars()
                    .take(200)
                    .collect::<String>();
                out.push_str(&format!(
                    "\n  <tool_use name=\"{name}\" input=\"{preview}\"/>"
                ));
            }
            ProviderContent::ToolResult {
                content, is_error, ..
            } => {
                let head: String = content.chars().take(400).collect();
                let err = if *is_error { " error" } else { "" };
                out.push_str(&format!(
                    "\n  <tool_result{err} bytes=\"{}\">{head}…</tool_result>",
                    content.len()
                ));
            }
            ProviderContent::Attachment(att) => {
                out.push_str(&format!(
                    "\n  <attachment kind=\"{}\" bytes=\"{}\"/>",
                    att.kind.mime_type(),
                    att.bytes.len()
                ));
            }
        }
    }
    out
}

/// Pull the contents of a single `<summary>...</summary>` tag from the
/// model's reply. v131's prompt asks the model to wrap the output, so
/// we extract that span exactly. Falls back to `None` when the tag
/// isn't present so callers can decide whether to use the raw text or
/// abandon the compaction attempt.
pub(crate) fn extract_summary_tag(s: &str) -> Option<String> {
    let open = s.find("<summary>")?;
    let after_open = open + "<summary>".len();
    let close_rel = s[after_open..].find("</summary>")?;
    Some(s[after_open..after_open + close_rel].trim().to_owned())
}

/// Run an LLM-based summarization pass over the subagent's running
/// history. Mirrors v131's `Sp7()` compaction call. Returns `true`
/// when the transcript was rewritten, `false` when nothing happened
/// (under threshold, too short, or the summary call failed). On
/// success the message list becomes:
///
///   `[ original_prompt, <summary message>, last_pair... ]`
///
/// preserving the original task description (so the subagent never
/// loses sight of why it was spawned) and the most recent
/// assistant+user-tool-result pair (so the loop's next iteration has
/// a coherent immediate context to act on).
pub(crate) async fn auto_compact_subagent_history(
    messages: &mut Vec<crate::provider::ProviderMessage>,
    provider: &dyn crate::provider::Provider,
    model: crate::provider::ModelId,
) -> bool {
    use crate::provider::*;
    use futures::StreamExt;

    let total_bytes: usize = messages.iter().map(estimate_provider_message_bytes).sum();
    let est_tokens = total_bytes / BYTES_PER_TOKEN;
    if est_tokens < SUBAGENT_AUTO_COMPACT_TOKEN_THRESHOLD {
        return false;
    }
    // Need at least: prompt + N evictable + last pair. With <4 messages
    // there's nothing meaningful to compact — defer to byte budget
    // eviction in that case.
    if messages.len() < 4 {
        return false;
    }

    let to_summarize_end = messages.len().saturating_sub(2);
    let mut transcript = String::new();
    for msg in messages.iter().take(to_summarize_end).skip(1) {
        transcript.push_str(&render_message_as_text(msg));
        transcript.push_str("\n\n");
    }
    if transcript.trim().is_empty() {
        return false;
    }

    // Original task prompt as a header so the summary model knows what
    // the subagent was *trying* to accomplish. Without this the model
    // produces vague summaries like "the assistant ran some tools".
    let original_task = messages
        .first()
        .map(render_message_as_text)
        .unwrap_or_default();

    let opts = StreamOptions::new(model)
        .system(SUBAGENT_AUTO_COMPACT_PROMPT.to_owned())
        .max_tokens(4_096);
    let summary_request = vec![ProviderMessage {
        role: ProviderRole::User,
        content: vec![ProviderContent::Text(format!(
            "Original task:\n{original_task}\n\nTranscript so far:\n{transcript}"
        ))],
    }];

    let stream = match provider.stream(summary_request, &opts).await {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!(target: "jfc::stream", error = %e, "subagent compaction stream failed");
            return false;
        }
    };
    futures::pin_mut!(stream);
    let mut text = String::new();
    while let Some(ev) = stream.next().await {
        match ev {
            Ok(StreamEvent::TextDelta { delta, .. }) => text.push_str(&delta),
            Ok(StreamEvent::TextDone { text: t, .. }) => {
                if text.is_empty() {
                    text = t;
                }
            }
            Ok(StreamEvent::Error { message }) => {
                tracing::warn!(target: "jfc::stream", error = %message, "subagent compaction error");
                return false;
            }
            Err(e) => {
                tracing::warn!(target: "jfc::stream", error = %e, "subagent compaction stream error");
                return false;
            }
            _ => {}
        }
    }

    let summary = extract_summary_tag(&text).unwrap_or_else(|| text.trim().to_owned());
    if summary.trim().is_empty() {
        return false;
    }

    let summary_msg = ProviderMessage {
        role: ProviderRole::Assistant,
        content: vec![ProviderContent::Text(format!(
            "[earlier subagent turns auto-compacted to fit context]\n\n<summary>\n{summary}\n</summary>"
        ))],
    };
    messages.splice(1..to_summarize_end, std::iter::once(summary_msg));
    tracing::info!(
        target: "jfc::stream",
        est_tokens_before = est_tokens,
        summary_chars = summary.len(),
        new_msg_count = messages.len(),
        "subagent auto-compaction applied"
    );
    true
}

/// Soft cap on total request bytes for a subagent / teammate provider call.
/// Estimated at ~4 chars/token, this is ≈125k tokens — well under the 1M
/// Bedrock cap and leaves room for system prompt + tool catalogue. The
/// teammate research scenario that triggered the 8.85M-token 400 was a
/// single subagent loop accumulating many large `Read` results unbounded
/// across 20 turns; this cap drops the oldest assistant/tool pairs once
/// the running total trips it.
pub(crate) const SUBAGENT_HISTORY_BUDGET_BYTES: usize = 500_000;

/// Rough byte count of a provider message used for budget enforcement.
/// Tool-use input is serialized to estimate JSON overhead. Tool results
/// are counted by raw content length (the `truncate_tool_result` cap
/// keeps each one ≤30KB so the figure stays tractable).
pub(crate) fn estimate_provider_message_bytes(msg: &crate::provider::ProviderMessage) -> usize {
    use crate::provider::ProviderContent;
    msg.content
        .iter()
        .map(|c| match c {
            ProviderContent::Text(t) => t.len(),
            ProviderContent::ToolUse { name, input, .. } => {
                name.len() + serde_json::to_string(input).map(|s| s.len()).unwrap_or(0)
            }
            ProviderContent::ToolResult { content, .. } => content.len(),
            // Attachments are base64-encoded on the wire — that's
            // ~4/3 the raw byte size. Use the raw size as a lower
            // bound for budget purposes; over-estimating is safer
            // than under-estimating since this drives compaction.
            ProviderContent::Attachment(att) => att.bytes.len() * 4 / 3,
        })
        .sum::<usize>()
        + 16
}

/// Drop oldest assistant/tool-result pairs (everything between the first
/// user message and the most recent assistant turn) until the total byte
/// estimate fits under `max_bytes`. The first user message — which holds
/// the subagent's *task prompt* — is always preserved so the model never
/// loses sight of what it was asked to do. Returns true when truncation
/// occurred so callers can log / surface it. Mirrors opencode's
/// `ForkContext.truncateForBudget` (packages/opencode/src/session/
/// fork-context.ts:49-71) which uses the same oldest-first eviction
/// strategy with a token-budget threshold.
pub(crate) fn cap_messages_for_budget(
    messages: &mut Vec<crate::provider::ProviderMessage>,
    max_bytes: usize,
) -> bool {
    let total: usize = messages.iter().map(estimate_provider_message_bytes).sum();
    if total <= max_bytes || messages.len() <= 1 {
        return false;
    }
    // Walk from the second message forward, dropping pairs until we fit.
    // Always keep messages[0] (the original task prompt) intact.
    let mut running = total;
    let mut drop_until: usize = 1;
    while running > max_bytes && drop_until < messages.len() {
        running -= estimate_provider_message_bytes(&messages[drop_until]);
        drop_until += 1;
    }
    if drop_until > 1 {
        // Drain[1..drop_until] but the very last assistant turn before
        // the most recent user (tool_results) message must stay paired —
        // Anthropic rejects a tool_result that doesn't immediately follow
        // its tool_use. So if the eviction window ends mid-pair (last
        // dropped is an assistant carrying tool_use), keep dropping
        // forward through its matching user/tool_result message too.
        if let Some(last_dropped) = messages.get(drop_until.saturating_sub(1))
            && matches!(last_dropped.role, crate::provider::ProviderRole::Assistant)
            && drop_until < messages.len()
        {
            drop_until += 1;
        }
        messages.drain(1..drop_until);
        // Insert a marker so the model knows context was elided. Placed
        // right after the prompt so it reads as "you asked X; some
        // earlier work was dropped to fit the request budget; here are
        // the recent results."
        messages.insert(
            1,
            crate::provider::ProviderMessage {
                role: crate::provider::ProviderRole::Assistant,
                content: vec![crate::provider::ProviderContent::Text(
                    "[earlier subagent turns elided to fit the request budget — \
                     continuing from the most recent results]"
                        .to_owned(),
                )],
            },
        );
        true
    } else {
        false
    }
}

#[cfg(test)]
mod truncate_tests {
    use super::*;

    // Normal: short input passes through unchanged.
    #[test]
    fn truncate_short_passes_through_normal() {
        assert_eq!(truncate_tool_result("hello"), "hello");
    }

    // Robust: the original panic. A multi-byte char (4-byte emoji) sitting
    // exactly at the byte-`half` boundary used to crash with "byte index N
    // is not a char boundary". Fix snaps to the nearest valid boundary.
    #[test]
    fn truncate_does_not_panic_on_multibyte_char_at_split_boundary_robust() {
        // Build a string where MAX/2 lands inside a 🦀 (4 bytes).
        let prefix_bytes = MAX_TOOL_RESULT_CHARS / 2 - 2;
        let mut s = String::with_capacity(MAX_TOOL_RESULT_CHARS * 2);
        for _ in 0..prefix_bytes {
            s.push('a');
        }
        s.push('🦀'); // straddles byte-`half` (2 bytes before, 2 after)
        for _ in 0..(MAX_TOOL_RESULT_CHARS) {
            s.push('b');
        }
        // Must not panic.
        let _ = truncate_tool_result(&s);
    }

    // Robust: input with mixed ASCII + multibyte content still produces a
    // valid UTF-8 result (no half-codepoints in the output).
    #[test]
    fn truncate_output_is_valid_utf8_robust() {
        let s: String = std::iter::repeat("héllo 🌟 ").take(5000).collect();
        let out = truncate_tool_result(&s);
        // The .chars() iterator panics on invalid UTF-8 — driving it to
        // completion proves the output is well-formed.
        let _ = out.chars().count();
    }

    // Normal: head and tail markers from the input are preserved inside
    // the v131-style `<truncated-output>` envelope.
    #[test]
    fn truncate_keeps_head_and_tail_normal() {
        let mid: String = "x".repeat(MAX_TOOL_RESULT_CHARS * 2);
        let s = format!("HEAD{mid}TAIL");
        let out = truncate_tool_result(&s);
        assert!(out.starts_with("<truncated-output"));
        assert!(out.contains("HEAD"));
        assert!(out.contains("TAIL"));
        assert!(out.contains("omitted_bytes"));
        assert!(out.ends_with("</truncated-output>"));
    }

    // Normal: marker exposes original byte count so the model can judge
    // whether a re-invocation with a narrower scope is worth it.
    #[test]
    fn truncate_marker_includes_original_byte_count_normal() {
        let s = "x".repeat(MAX_TOOL_RESULT_CHARS * 3);
        let out = truncate_tool_result(&s);
        let expected = format!("original_bytes=\"{}\"", s.len());
        assert!(out.contains(&expected), "marker missing byte count: {out}");
    }
}

#[cfg(test)]
mod budget_tests {
    use super::*;
    use crate::provider::{ProviderContent, ProviderMessage, ProviderRole};

    fn user_text(s: &str) -> ProviderMessage {
        ProviderMessage {
            role: ProviderRole::User,
            content: vec![ProviderContent::Text(s.to_owned())],
        }
    }
    fn assistant_text(s: &str) -> ProviderMessage {
        ProviderMessage {
            role: ProviderRole::Assistant,
            content: vec![ProviderContent::Text(s.to_owned())],
        }
    }
    fn assistant_tool_use(id: &str, name: &str) -> ProviderMessage {
        ProviderMessage {
            role: ProviderRole::Assistant,
            content: vec![ProviderContent::ToolUse {
                id: id.to_owned(),
                name: name.to_owned(),
                input: serde_json::json!({"path": "x"}),
            }],
        }
    }
    fn user_tool_result(id: &str, content: &str) -> ProviderMessage {
        ProviderMessage {
            role: ProviderRole::User,
            content: vec![ProviderContent::ToolResult {
                tool_use_id: id.to_owned(),
                content: content.to_owned(),
                is_error: false,
            }],
        }
    }

    // Normal: under-budget messages pass through unchanged.
    #[test]
    fn cap_messages_under_budget_passes_through_normal() {
        let mut msgs = vec![user_text("hi"), assistant_text("hello"), user_text("ok")];
        let elided = cap_messages_for_budget(&mut msgs, 1_000_000);
        assert!(!elided);
        assert_eq!(msgs.len(), 3);
    }

    // Robust: empty / single-message lists never truncate.
    #[test]
    fn cap_messages_single_message_no_op_robust() {
        let mut msgs = vec![user_text("just one")];
        let elided = cap_messages_for_budget(&mut msgs, 0);
        assert!(!elided);
        assert_eq!(msgs.len(), 1);
    }

    // Normal: oversized middle is dropped, prompt + tail are preserved.
    #[test]
    fn cap_messages_drops_oldest_pairs_keeps_prompt_and_tail_normal() {
        let big = "x".repeat(20_000);
        let mut msgs = vec![
            user_text("PROMPT"),
            assistant_tool_use("t1", "Read"),
            user_tool_result("t1", &big),
            assistant_tool_use("t2", "Read"),
            user_tool_result("t2", &big),
            assistant_tool_use("t3", "Read"),
            user_tool_result("t3", &big),
            assistant_text("recent assistant turn"),
        ];
        let elided = cap_messages_for_budget(&mut msgs, 25_000);
        assert!(elided, "should have truncated");
        // First message is the original prompt, intact.
        match &msgs[0].content[0] {
            ProviderContent::Text(t) => assert_eq!(t, "PROMPT"),
            _ => panic!("expected prompt preserved"),
        }
        // Last message stays intact (most recent assistant turn).
        match msgs.last().unwrap().content[0] {
            ProviderContent::Text(ref t) => assert_eq!(t, "recent assistant turn"),
            _ => panic!("expected tail preserved"),
        }
    }

    // Normal: a marker message is inserted right after the prompt so the
    // model sees that some context was elided.
    #[test]
    fn cap_messages_inserts_truncation_marker_normal() {
        let big = "x".repeat(20_000);
        let mut msgs = vec![
            user_text("PROMPT"),
            assistant_tool_use("t1", "Read"),
            user_tool_result("t1", &big),
            assistant_text("recent"),
        ];
        cap_messages_for_budget(&mut msgs, 5_000);
        // Marker lands at index 1.
        match &msgs[1].content[0] {
            ProviderContent::Text(t) => assert!(t.contains("elided")),
            _ => panic!("expected marker text"),
        }
        assert!(matches!(msgs[1].role, ProviderRole::Assistant));
    }

    // Robust: when truncation evicts an assistant tool_use, its matching
    // tool_result must also drop. Otherwise Anthropic's API rejects the
    // turn ("tool_result without tool_use").
    #[test]
    fn cap_messages_drops_orphaned_tool_result_robust() {
        let big = "x".repeat(50_000);
        let mut msgs = vec![
            user_text("PROMPT"),
            assistant_tool_use("t1", "Read"),
            user_tool_result("t1", &big),
            assistant_text("recent"),
        ];
        cap_messages_for_budget(&mut msgs, 1_000);
        // No bare ToolResult should survive — every retained ToolResult
        // must be preceded by its tool_use, but here both tool messages
        // were evicted as a pair.
        let has_orphan_tool_result = msgs.iter().any(|m| {
            m.content
                .iter()
                .any(|c| matches!(c, ProviderContent::ToolResult { .. }))
        });
        assert!(
            !has_orphan_tool_result,
            "tool_result left without its tool_use"
        );
    }

    // Normal: estimate function counts text bytes, tool_use name+JSON
    // length, and tool_result content length.
    #[test]
    fn estimate_provider_message_bytes_counts_each_variant_normal() {
        let t = estimate_provider_message_bytes(&user_text("abcde"));
        assert!(t >= 5 + 16, "got {t}"); // 5 chars + 16 byte overhead floor
        let tu = estimate_provider_message_bytes(&assistant_tool_use("id", "Read"));
        assert!(tu >= 4); // at least name length
        let tr = estimate_provider_message_bytes(&user_tool_result("id", "x".repeat(100).as_str()));
        assert!(tr >= 100);
    }

    // Normal: budget constant is the documented 500KB.
    #[test]
    fn subagent_history_budget_constant_normal() {
        assert_eq!(SUBAGENT_HISTORY_BUDGET_BYTES, 500_000);
    }
}

#[cfg(test)]
mod auto_compact_tests {
    use super::*;
    use crate::provider::{
        EventStream, ModelId, ModelInfo, Provider, ProviderContent, ProviderMessage, ProviderRole,
        StopReason, StreamEvent, StreamOptions,
    };
    use std::sync::{Arc, Mutex};

    /// Stub provider that returns a single canned text reply on every
    /// `stream()` call. Used to verify the compaction wiring without
    /// hitting a real model.
    struct CannedSummaryProvider {
        reply: String,
        calls: Arc<Mutex<usize>>,
    }

    impl CannedSummaryProvider {
        fn new(reply: impl Into<String>) -> Self {
            Self {
                reply: reply.into(),
                calls: Arc::new(Mutex::new(0)),
            }
        }
    }

    #[async_trait::async_trait]
    impl Provider for CannedSummaryProvider {
        fn name(&self) -> &str {
            "canned"
        }
        fn available_models(&self) -> Vec<ModelInfo> {
            vec![ModelInfo::new("stub", "Stub", "canned")]
        }
        async fn stream(
            &self,
            _messages: Vec<ProviderMessage>,
            _options: &StreamOptions,
        ) -> anyhow::Result<EventStream> {
            *self.calls.lock().unwrap() += 1;
            let events = vec![
                Ok(StreamEvent::TextDelta {
                    index: 0,
                    delta: self.reply.clone(),
                }),
                Ok(StreamEvent::Done {
                    stop_reason: StopReason::EndTurn,
                }),
            ];
            Ok(Box::pin(futures::stream::iter(events)))
        }
    }
    impl crate::provider::seal::Sealed for CannedSummaryProvider {}

    /// Provider that always returns an error from `stream()` — used to
    /// verify the compaction call gracefully no-ops on transport errors
    /// rather than corrupting the message list.
    struct ErrorProvider;

    #[async_trait::async_trait]
    impl Provider for ErrorProvider {
        fn name(&self) -> &str {
            "error"
        }
        fn available_models(&self) -> Vec<ModelInfo> {
            vec![]
        }
        async fn stream(
            &self,
            _messages: Vec<ProviderMessage>,
            _options: &StreamOptions,
        ) -> anyhow::Result<EventStream> {
            Err(anyhow::anyhow!("simulated stream failure"))
        }
    }
    impl crate::provider::seal::Sealed for ErrorProvider {}

    fn user_text(s: &str) -> ProviderMessage {
        ProviderMessage {
            role: ProviderRole::User,
            content: vec![ProviderContent::Text(s.to_owned())],
        }
    }
    fn assistant_text(s: &str) -> ProviderMessage {
        ProviderMessage {
            role: ProviderRole::Assistant,
            content: vec![ProviderContent::Text(s.to_owned())],
        }
    }
    fn assistant_tool_use(id: &str, name: &str) -> ProviderMessage {
        ProviderMessage {
            role: ProviderRole::Assistant,
            content: vec![ProviderContent::ToolUse {
                id: id.to_owned(),
                name: name.to_owned(),
                input: serde_json::json!({"path": "x"}),
            }],
        }
    }
    fn user_tool_result(id: &str, content: &str) -> ProviderMessage {
        ProviderMessage {
            role: ProviderRole::User,
            content: vec![ProviderContent::ToolResult {
                tool_use_id: id.to_owned(),
                content: content.to_owned(),
                is_error: false,
            }],
        }
    }

    // Normal: extract_summary_tag pulls content out of a wrapped reply.
    #[test]
    fn extract_summary_tag_finds_content_normal() {
        let s = "preamble <summary>core fact</summary> afterword";
        assert_eq!(extract_summary_tag(s), Some("core fact".to_owned()));
    }

    // Robust: missing tag returns None so callers can decide.
    #[test]
    fn extract_summary_tag_missing_returns_none_robust() {
        assert_eq!(extract_summary_tag("no tags here"), None);
    }

    // Normal: render_message_as_text labels role and inlines text.
    #[test]
    fn render_message_text_basic_normal() {
        let r = render_message_as_text(&user_text("hello"));
        assert!(r.starts_with("[user]"));
        assert!(r.contains("hello"));
    }

    // Normal: tool_result rendering shows byte count + truncated head,
    // not the full content (the whole point of the summary input).
    #[test]
    fn render_message_text_tool_result_summarizes_body_normal() {
        let big = "x".repeat(10_000);
        let r = render_message_as_text(&user_tool_result("id1", &big));
        assert!(r.contains("bytes=\"10000\""));
        assert!(!r.contains(&"x".repeat(1_000)));
    }

    // Normal: under-threshold history is left alone (compaction skipped).
    #[tokio::test(flavor = "current_thread")]
    async fn auto_compact_under_threshold_no_op_normal() {
        let provider = CannedSummaryProvider::new("<summary>x</summary>");
        let mut msgs = vec![user_text("PROMPT"), assistant_text("hi"), user_text("ok")];
        let did = auto_compact_subagent_history(&mut msgs, &provider, ModelId::new("stub")).await;
        assert!(!did);
        assert_eq!(msgs.len(), 3);
        assert_eq!(*provider.calls.lock().unwrap(), 0);
    }

    // Normal: over-threshold transcript triggers an LLM call and is
    // rewritten as [prompt, summary, last pair].
    #[tokio::test(flavor = "current_thread")]
    async fn auto_compact_over_threshold_summarizes_normal() {
        let provider = CannedSummaryProvider::new(
            "<summary>The agent read three files and reported their structure.</summary>",
        );
        let big = "x".repeat(200_000); // ~50k tokens per message at 4 chars/tok
        let mut msgs = vec![
            user_text("PROMPT"),
            assistant_tool_use("t1", "Read"),
            user_tool_result("t1", &big),
            assistant_tool_use("t2", "Read"),
            user_tool_result("t2", &big),
            assistant_tool_use("t3", "Read"),
            user_tool_result("t3", &big),
            assistant_text("recent assistant"),
            user_text("recent user"),
        ];
        let did = auto_compact_subagent_history(&mut msgs, &provider, ModelId::new("stub")).await;
        assert!(did, "expected compaction to fire");
        // [prompt, summary, last 2 messages] = 4
        assert_eq!(msgs.len(), 4);
        match &msgs[0].content[0] {
            ProviderContent::Text(t) => assert_eq!(t, "PROMPT"),
            _ => panic!("prompt not preserved"),
        }
        match &msgs[1].content[0] {
            ProviderContent::Text(t) => {
                assert!(t.contains("auto-compacted"));
                assert!(t.contains("read three files"));
            }
            _ => panic!("expected summary message"),
        }
        assert_eq!(*provider.calls.lock().unwrap(), 1);
    }

    // Robust: provider error during compaction leaves the message list
    // intact so the caller can fall through to byte-budget eviction.
    #[tokio::test(flavor = "current_thread")]
    async fn auto_compact_provider_error_no_op_robust() {
        let provider = ErrorProvider;
        let big = "x".repeat(200_000);
        let mut msgs = vec![
            user_text("PROMPT"),
            assistant_tool_use("t1", "Read"),
            user_tool_result("t1", &big),
            assistant_text("recent"),
            user_text("ok"),
        ];
        let original_len = msgs.len();
        let did = auto_compact_subagent_history(&mut msgs, &provider, ModelId::new("stub")).await;
        assert!(!did);
        assert_eq!(msgs.len(), original_len);
    }

    // Robust: empty summary text returned by the model is treated as a
    // failure so the caller doesn't replace history with an empty
    // <summary> block.
    #[tokio::test(flavor = "current_thread")]
    async fn auto_compact_empty_summary_no_op_robust() {
        let provider = CannedSummaryProvider::new("<summary>   </summary>");
        let big = "x".repeat(200_000);
        let mut msgs = vec![
            user_text("PROMPT"),
            assistant_tool_use("t1", "Read"),
            user_tool_result("t1", &big),
            assistant_tool_use("t2", "Read"),
            user_tool_result("t2", &big),
            assistant_text("recent"),
            user_text("ok"),
        ];
        let original_len = msgs.len();
        let did = auto_compact_subagent_history(&mut msgs, &provider, ModelId::new("stub")).await;
        assert!(!did);
        assert_eq!(msgs.len(), original_len);
    }

    // Robust: when the model omits the <summary> tags, the raw text is
    // used as the summary body (best-effort). v131 prompt asks for tags
    // but a stubborn model may not comply.
    #[tokio::test(flavor = "current_thread")]
    async fn auto_compact_falls_back_to_raw_text_when_no_tags_robust() {
        let provider = CannedSummaryProvider::new("the agent did things and finished.");
        let big = "x".repeat(200_000);
        let mut msgs = vec![
            user_text("PROMPT"),
            assistant_tool_use("t1", "Read"),
            user_tool_result("t1", &big),
            assistant_tool_use("t2", "Read"),
            user_tool_result("t2", &big),
            assistant_text("recent"),
            user_text("ok"),
        ];
        let did = auto_compact_subagent_history(&mut msgs, &provider, ModelId::new("stub")).await;
        assert!(did);
        match &msgs[1].content[0] {
            ProviderContent::Text(t) => assert!(t.contains("the agent did things")),
            _ => panic!("expected summary body"),
        }
    }

    // Normal: constants align with v131 deob figures.
    #[test]
    fn constants_match_v131_normal() {
        assert_eq!(SUBAGENT_AUTO_COMPACT_TOKEN_THRESHOLD, 100_000);
        assert_eq!(BYTES_PER_TOKEN, 4);
        assert!(SUBAGENT_AUTO_COMPACT_PROMPT.contains("Task Overview"));
        assert!(SUBAGENT_AUTO_COMPACT_PROMPT.contains("Current State"));
        assert!(SUBAGENT_AUTO_COMPACT_PROMPT.contains("Important Discoveries"));
        assert!(SUBAGENT_AUTO_COMPACT_PROMPT.contains("Next Steps"));
        assert!(SUBAGENT_AUTO_COMPACT_PROMPT.contains("<summary>"));
    }
}

#[tracing::instrument(
    target = "jfc::stream",
    skip_all,
    fields(
        provider = %provider.name(),
        model = %model,
        messages = messages.len(),
    ),
)]
pub async fn stream_response(
    provider: Arc<dyn Provider>,
    messages: Vec<ProviderMessage>,
    model: ModelId,
    tx: mpsc::Sender<AppEvent>,
    interrupt: std::sync::Arc<std::sync::atomic::AtomicBool>,
    // wg-async pattern: spawned tasks holding critical state need an
    // explicit cancellation handle, not just a polled flag. The token
    // races the SSE stream against `.cancelled()` so ESC×2 unwinds in
    // microseconds instead of waiting for the next STREAM_INTERRUPT_POLL.
    cancel: tokio_util::sync::CancellationToken,
) {
    let cwd = std::env::current_dir()
        .ok()
        .and_then(|p| p.to_str().map(str::to_owned))
        .unwrap_or_default();

    // Build prompt sections (matching Claude Code's structure)
    let skills_listing = if let Ok(cwd_path) = std::env::current_dir() {
        let skills = crate::agents::load_skills(&cwd_path);
        let block = crate::agents::render_skills_section(&skills);
        if block.is_empty() { String::new() } else { block }
    } else {
        String::new()
    };

    // Auto-dispatch nudge — surfaces every agent's `keyTrigger` to
    // the leader so the model proactively fires Explore / Plan /
    // verification without the user having to ask. v132 + oh-my-
    // opencode parity: "Default Bias: DELEGATE" + Intent → Dispatch
    // routing table. Only the built-in agents are consulted here;
    // user-defined `.claude/agents/*.md` already merge into the same
    // list via `load_all_agents`, so their `keyTrigger` frontmatter
    // also takes effect.
    let dispatch_section = {
        let cwd_for_agents = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
        let agents = crate::agents::load_agents(&cwd_for_agents);
        crate::agents::render_dispatch_section(&agents)
    };

    let diagnostics_block = {
        let diags = crate::diagnostics::global_snapshot();
        crate::diagnostics::render_for_prompt(&diags).unwrap_or_default()
    };

    let tool_guidance = "\
## Using your tools\n\
Prefer dedicated tools over Bash when one fits (Read, Write, Edit, Glob, Grep) — reserve Bash for shell-only operations.\n\
You can call multiple tools in a single response. If you intend to call multiple tools and there are no dependencies between the calls, make all of the independent calls in the same block, otherwise you MUST wait for previous calls to finish first to determine the dependent values (do NOT use placeholders or guess missing parameters).\n\
If the user provides a specific value for a parameter (for example provided in quotes), make sure to use that value EXACTLY. DO NOT make up values for or ask about optional parameters.";

    let coding_instructions = "\
## Doing tasks\n\
The user will primarily request software engineering tasks. When given an unclear or generic instruction, consider it in the context of software engineering and the current working directory.\n\
You are highly capable and often allow users to complete ambitious tasks that would otherwise be too complex or take too long. Defer to user judgement about whether a task is too large.\n\
For exploratory questions, respond in 2-3 sentences with a recommendation and the main tradeoff. Don't implement until the user agrees.\n\
Prefer editing existing files to creating new ones.\n\
Be careful not to introduce security vulnerabilities (command injection, XSS, SQL injection). Prioritize writing safe, secure, and correct code.\n\
Don't add features, refactor, or introduce abstractions beyond what the task requires. Three similar lines is better than a premature abstraction.\n\
Don't add error handling or validation for scenarios that can't happen. Trust internal code and framework guarantees. Only validate at system boundaries.\n\
Default to writing no comments. Only add one when the WHY is non-obvious: a hidden constraint, a subtle invariant, a workaround for a specific bug.\n\
When reporting results, be accurate about what you verified vs. what you assumed. Distinguish between what you confirmed (ran a command, read a file) and what you believe but did not check.";

    let safety_instructions = "\
## Executing actions with care\n\
Read, search, and investigate freely — looking is not acting. For actions that are hard to reverse, affect shared systems, or are otherwise risky (deleting data, force-pushing, sending messages, modifying shared infrastructure), confirm with the user before proceeding unless durably authorized. Approval in one context doesn't extend to the next.\n\
When you encounter an obstacle, do not use destructive actions as a shortcut. Try to identify root causes rather than bypassing safety checks. If you discover unexpected state like unfamiliar files or branches, investigate before deleting or overwriting — it may represent in-progress work.";

    let tone_style = "\
## Tone and style\n\
Only use emojis if the user explicitly requests it.\n\
Your responses should be short and concise.\n\
When referencing specific functions or pieces of code include the pattern file_path:line_number to allow the user to easily navigate to the source.\n\
Do not use a colon before tool calls.";
    let mut system_prompt = format!(
        "You are jfc, a coding assistant running as a CLI in the user's terminal. \
         You have direct access to the user's filesystem and shell via tools \
         (Bash, Read, Write, Edit, Glob, Grep). When the user asks you to do \
         something — read a file, run a command, write code — USE the tools to \
         do it directly. Don't describe how the user could do it manually; you \
         are the one doing it. Working directory: {cwd}\n\n\
         ## Task tracking\n\
         For any request with 2 or more distinct steps, use TaskCreate to plan \
         before starting. Call TaskCreate once per step with a short description. \
         Mark each step complete with TaskDone immediately after finishing it — \
         never batch completions. Update a step's description mid-work with \
         TaskUpdate if scope changes. TaskList shows the user your current plan \
         in the sidebar. This is the primary way users track your progress, so \
         use it consistently on all non-trivial work.\n\n\
         ## Available skills\n\n\
         {skills_listing}\n\n\
         {dispatch_section}\n\n\
         ## Current diagnostics\n\n\
         {diagnostics_block}\n\n\
         {tool_guidance}\n\n\
         {coding_instructions}\n\n\
         {safety_instructions}\n\n\
         {tone_style}"
    );

    // v126 CLAUDE.md hierarchy — managed → user → project → .claude/ → local
    // overrides. Each layer is appended with its origin labeled so the model
    // can tell which rule came from which file. We load on every stream call
    // so live edits to CLAUDE.md take effect on the next turn (matching CC).
    if let Ok(cwd_path) = std::env::current_dir() {
        let hierarchy = crate::context::ClaudeMdHierarchy::load(&cwd_path);
        if let Some(layered) = hierarchy.render() {
            system_prompt.push_str("\n\n");
            system_prompt.push_str(&layered);
        }

        // Memory system — load persistent memories from both user-level
        // (~/.config/jfc/memory/) and project-level (.jfc/memory/) and
        // inject them into the system prompt. Re-loaded on every stream
        // call so newly-saved memories take effect on the next turn.
        let memories = crate::memory::load_all_memories(&cwd_path);
        if let Some(memories_section) = crate::memory::render_memories_section(&memories) {
            system_prompt.push_str(&memories_section);
        }

        // Two-phase memory recall (v132 `bt1` / `xt1` / tengu_memory_survey).
        // After the bulk-injected memory listing, we ask the model which
        // memories actually apply to *this* user message and synthesize the
        // hits into a short `<system-reminder>` block. The bulk listing stays
        // — it's the cheap-but-coarse signal — and the recall block adds the
        // expensive-but-targeted layer on top. Configurable via
        // `Config.memory_recall_enabled` (default on); skipped for empty /
        // slash-command queries since neither benefits from recall.
        let recall_enabled =
            crate::memory_recall::is_enabled(crate::config::load().memory_recall_enabled);
        if recall_enabled && !memories.is_empty() {
            let last_user_query = last_user_text(&messages);
            if let Some(query) = last_user_query {
                let trimmed = query.trim();
                if !trimmed.is_empty() && !trimmed.starts_with('/') {
                    let block = crate::memory_recall::run_recall(
                        trimmed,
                        &memories,
                        provider.clone(),
                        model.clone(),
                    )
                    .await;
                    if let Some(b) = block {
                        tracing::debug!(
                            target: "jfc::stream",
                            recall_block_len = b.len(),
                            "appended memory recall block to system prompt"
                        );
                        system_prompt.push_str(&b);
                    }
                }
            }
        }

        // Auto-context: when the previous turn(s) edited files via
        // Edit / Write / symbol_edit, surface the callers of the
        // affected functions so the model doesn't have to re-grep
        // them. v131 plan task 23. Drained on use so a single edit
        // injects exactly once.
        if let Some(block) = crate::tools::render_pending_auto_context(&cwd_path) {
            system_prompt.push_str(&block);
        }

        // v132 auto-context: branch / dirty / recent commits. The git
        // tooling already exists in `git_context.rs`; this is the wire-
        // in so the model doesn't have to spend a turn running git
        // status / git log for trivial orientation. Cheap (<50ms) and
        // bounded (5 commits), so we run it every turn.
        let git_ctx = crate::git_context::get_git_context();
        if git_ctx.current_branch.is_some() || !git_ctx.recent_commits.is_empty() {
            system_prompt.push_str("\n\n");
            system_prompt.push_str(&git_ctx.to_prompt_string());
        }

        // v132 env fingerprint — toolchain versions so the model knows
        // the active rustc/cargo/node/python without burning a turn on
        // `--version` calls. Cached per process, so this is zero-cost
        // after the first stream.
        if let Some(env_block) = crate::env_context::get().to_prompt_string() {
            system_prompt.push_str(&env_block);
        }
    }
    // v132 feature-gate framework: append a section listing any
    // gates that diverge from their default. Suppressed when every
    // gate is at its ship default to avoid prompt churn.
    if let Some(gates) = crate::feature_gates::system_prompt_section() {
        system_prompt.push_str(&gates);
    }

    // v132 Marsh: drain any bash chunks the streaming tool buffered
    // since the last outbound prompt and prepend them as a
    // `<system-reminder>` so the model sees what shell commands
    // printed in real time. Skipped when the gate is off (default).
    if crate::feature_gates::is_enabled(crate::feature_gates::FeatureGate::Marsh) {
        let chunks = crate::feature_gates::marsh_drain();
        if !chunks.is_empty() {
            let body = chunks.join("\n");
            let preview: String = body.chars().take(8_000).collect();
            system_prompt.push_str(&format!(
                "\n\n{}",
                crate::system_reminder::format(&format!(
                    "Bash subprocess output captured since last turn:\n```\n{preview}\n```"
                ))
            ));
        }
    }

    // v132 investigate-first guidance (the `harrier` gate). Nudges
    // the model to spend up to ~1 minute on read-only investigation
    // before asking a clarifying question, when the question is
    // bounded and concrete. Skipped if `harrier` is disabled.
    if crate::feature_gates::is_enabled(crate::feature_gates::FeatureGate::Harrier) {
        system_prompt.push_str(
            "\n\n## Investigate before asking\n\
             When the user's request is concrete and bounded (a specific \
             file, a named symbol, a known feature area), spend up to ~1 \
             minute on read-only investigation (Read / Grep / Glob / git \
             log) **before** asking a clarifying question. The user almost \
             always prefers a self-answered question over a back-and-forth \
             — they brought the question to you to save themselves the \
             investigation. Only escalate to AskUserQuestion when the \
             investigation surfaces multiple incompatible interpretations \
             that would meaningfully change the plan.",
        );
    }

    // OutputStyle suffix (v132 brief/verbose/explanatory/learning).
    // Read from the process-global handle so /output-style takes
    // effect on the next turn without having to thread state through
    // every caller. Default is no-op so existing turns are byte-for-
    // byte identical.
    if let Some(suffix) = crate::output_style::active().system_prompt_suffix() {
        system_prompt.push_str(suffix);
        tracing::debug!(
            target: "jfc::stream",
            style = %crate::output_style::active().name(),
            "appended OutputStyle suffix to system prompt"
        );
    }
    // Thinking is a 3-way choice: adaptive (4.6+ Anthropic-native),
    // legacy budget_tokens (older Anthropic-native + select deployments),
    // or off. Proxy-routed model IDs (bedrock-*, vertex-*, etc.) default
    // to off because those proxies typically reject the field. Mirrors
    // v126's tiered gate (`modelSupportsAdaptiveThinking` →
    // `modelSupportsThinking` → off, claude.ts:1602).
    let supports_adaptive = model_supports_adaptive_thinking(model.as_str());
    let has_thinking_support = supports_adaptive || model_supports_thinking(model.as_str());
    tracing::info!(
        target: "jfc::stream",
        model = %model,
        has_thinking_support,
        supports_adaptive,
        system_prompt_len = system_prompt.len(),
        tool_count = tools::all_tool_defs().len(),
        "preparing stream request"
    );
    let max_out = max_output_tokens_for(model.as_str());
    let mut advertised_tools = tools::all_tool_defs_with_mcp().await;

    // v132 pre-flight permission scan: when the user has explicit Deny
    // rules in `.jfc/permissions.toml`, drop those tools from the
    // catalog before sending. The model never sees them, never tries to
    // call them, and the user never sees a denied tool error after the
    // fact. Skipped when no rules are configured (catalog passes through
    // unchanged) so default behavior is identical.
    #[cfg(feature = "permission-automation")]
    {
        let cwd_for_perms = std::env::current_dir()
            .unwrap_or_else(|_| std::path::PathBuf::from("."));
        let cfg = crate::config::feature_config::FeatureConfig::load(&cwd_for_perms);
        let rules = crate::permissions::RuleSet::from_config(&cfg);
        let before = advertised_tools.len();
        let mut suppressed: Vec<String> = Vec::new();
        advertised_tools.retain(|t| {
            let decision = crate::permissions::check_tool_permission(&rules, &t.name, None);
            if matches!(decision.action, crate::permissions::PermissionAction::Deny) {
                suppressed.push(t.name.clone());
                false
            } else {
                true
            }
        });
        if !suppressed.is_empty() {
            tracing::info!(
                target: "jfc::stream::permissions",
                suppressed_count = suppressed.len(),
                tools = ?suppressed,
                "pre-flight: suppressed denied tools from catalog"
            );
            // Tell the model what's missing so it doesn't waste a turn
            // trying to use it.
            system_prompt.push_str(&format!(
                "\n\n## Tools suppressed by policy\n\nThe following tools \
                 are denied by `.jfc/permissions.toml` and are NOT available \
                 this session: {}.\n",
                suppressed.join(", "),
            ));
        }
        let _ = before;
    }
    let opts = {
        let mut base = StreamOptions::new(model.clone())
            .system(system_prompt)
            .tools(advertised_tools)
            .max_tokens(max_out);
        // v132 reasoning-effort pin. Read from the process-global slot
        // populated by `EffortState::publish_global`. Skipped when no
        // session pin is set (server picks default) or when the model
        // doesn't accept the parameter (provider serializer drops it).
        if let Some(effort) = crate::effort::active_global() {
            base = base.reasoning_effort(effort);
        }
        if supports_adaptive {
            base.adaptive()
        } else if has_thinking_support {
            // Legacy budget_tokens path. Pre-4.6 Anthropic-native models
            // accept the older `{"type": "enabled", "budget_tokens": N}`
            // form. v126 uses 16384 as the default budget.
            base.thinking(16_384)
        } else {
            base
        }
    };

    // v132 BeforeStream hook fires after the prompt is fully assembled
    // but before the network call. Handlers that want to inject system
    // reminders, gate on cost budgets, or pre-compact the context can
    // do so here. Default registry is Logger-only so production behavior
    // is byte-for-byte identical when no user hooks are configured.
    crate::hooks::fire(
        crate::hooks::HookPoint::BeforeStream,
        &crate::hooks::HookContext::for_session("stream")
            .with_extra("model", model.as_str().to_string())
            .with_extra("message_count", messages.len().to_string()),
    );

    let mut stream = match provider.stream(messages.clone(), &opts).await {
        Ok(s) => {
            tracing::debug!(target: "jfc::stream", "stream opened successfully");
            s
        }
        Err(e) => {
            let err_lower = e.to_string().to_lowercase();
            // If the API rejects thinking (adaptive or budget_tokens), retry
            // without it. Mirrors v126's fallback when a model unexpectedly
            // doesn't support the thinking parameter.
            if (err_lower.contains("thinking") && err_lower.contains("not supported"))
                || err_lower.contains("adaptive thinking is not supported")
            {
                tracing::warn!(
                    target: "jfc::stream",
                    model = %model,
                    error = %e,
                    "stream rejected thinking parameter — retrying without thinking"
                );
                let fallback_opts = StreamOptions::new(opts.model.clone())
                    .system(opts.system.clone().unwrap_or_default())
                    .tools(opts.tools.clone())
                    .max_tokens(opts.max_tokens);
                match provider.stream(messages, &fallback_opts).await {
                    Ok(s) => s,
                    Err(e2) => {
                        tracing::error!(target: "jfc::stream", error = %e2, "stream open failed (fallback without thinking)");
                        let _ = tx.send(AppEvent::StreamError(e2.to_string())).await;
                        return;
                    }
                }
            } else if err_lower.contains("prompt is too long")
                || err_lower.contains("prompt_too_long")
                || err_lower.contains("input length")
                || err_lower.contains("max_tokens")
                || err_lower.contains("context window")
            {
                // v132 mid-stream compaction trigger. The pre-submit
                // path catches the obvious cases via `compact_level`,
                // but estimator drift means the API can still reject
                // a turn with prompt_too_long. Surface a system-level
                // signal so the main loop fires `/compact` and re-
                // queues the same prompt; the user sees a brief toast
                // instead of a hard failure.
                tracing::warn!(
                    target: "jfc::stream",
                    error = %e,
                    "stream rejected: prompt too long — requesting auto-compact"
                );
                let _ = tx
                    .send(AppEvent::StreamError(format!(
                        "auto-compact: {e}"
                    )))
                    .await;
                return;
            } else {
                tracing::error!(target: "jfc::stream", error = %e, "stream open failed");
                let _ = tx.send(AppEvent::StreamError(e.to_string())).await;
                return;
            }
        }
    };

    let mut stop_reason = StopReason::EndTurn;
    let mut tool_accum: HashMap<usize, (String, String, String)> = HashMap::new();

    loop {
        // Cooperative cancel: the user pressed ESC twice — drop the
        // stream mid-flight, surface a clean stop, and let the main
        // loop reset state. Two paths: legacy `interrupt` AtomicBool
        // (still set by callers we haven't migrated yet) and the
        // CancellationToken (instantly observed via `.cancelled()` in
        // the select! below — no polling latency).
        if interrupt.load(std::sync::atomic::Ordering::SeqCst) || cancel.is_cancelled() {
            tracing::info!(target: "jfc::stream", "stream interrupted by user (ESC×2)");
            let _ = tx
                .send(AppEvent::StreamError("Interrupted by user".to_owned()))
                .await;
            return;
        }

        let event = tokio::select! {
            biased;
            // wg-async: race the SSE read against the cancel token so a
            // stalled provider (slow first byte, hung TLS) doesn't trap
            // the user in "Interrupting…" until the next interrupt poll.
            _ = cancel.cancelled() => {
                tracing::info!(target: "jfc::stream", "stream cancelled via token");
                let _ = tx
                    .send(AppEvent::StreamError("Interrupted by user".to_owned()))
                    .await;
                return;
            }
            _ = tokio::time::sleep(STREAM_INTERRUPT_POLL) => continue,
            event = stream.next() => event,
        };

        let Some(event) = event else {
            break;
        };

        let event = match event {
            Ok(e) => e,
            Err(e) => {
                tracing::error!(target: "jfc::stream", error = %e, "stream event error");
                let _ = tx.send(AppEvent::StreamError(e.to_string())).await;
                return;
            }
        };

        match event {
            StreamEvent::TextDelta { delta, .. } => {
                // High-frequency token stream; safe to drop — next chunk supersedes.
                if tx
                    .try_send(AppEvent::StreamChunk {
                        text: Some(delta),
                        reasoning: None,
                    })
                    .is_err()
                {
                    tracing::trace!(target: "jfc::stream", "StreamChunk dropped (buffer full)");
                }
            }
            StreamEvent::ThinkingDelta { delta, .. } => {
                // High-frequency token stream; safe to drop — next chunk supersedes.
                if tx
                    .try_send(AppEvent::StreamChunk {
                        text: None,
                        reasoning: Some(delta),
                    })
                    .is_err()
                {
                    tracing::trace!(target: "jfc::stream", "StreamChunk(thinking) dropped (buffer full)");
                }
            }
            StreamEvent::ToolDelta { index, delta } => {
                let byte_len = delta.len();
                tool_accum.entry(index).or_default().2.push_str(&delta);
                // Notify the main loop so:
                // 1. streaming_response_bytes increments (spinner token estimate stays live)
                // 2. streaming_last_token_at resets (stall timer doesn't fire)
                // Matches v126's responseLengthRef accumulation from input_json_delta events.
                // High-frequency tool input delta; safe to drop — next delta supersedes.
                if tx.try_send(AppEvent::ToolInputDelta(byte_len)).is_err() {
                    tracing::trace!(target: "jfc::stream", "ToolInputDelta dropped (buffer full)");
                }
            }
            StreamEvent::ToolDone {
                index,
                tool_name,
                tool_use_id,
                input_json,
            } => {
                // Prefer the input_json the provider assembled (Anthropic SSE
                // builds the full payload before firing ToolDone). When that's
                // empty, fall back to the accumulator we filled from
                // ToolDelta — required by OpenWebUI's OpenAI-compatible
                // streaming, which only ever ships fragments and doesn't
                // assemble the full string itself.
                let assembled = if input_json.is_empty() {
                    tool_accum
                        .get(&index)
                        .map(|(_, _, buf)| buf.clone())
                        .unwrap_or_default()
                } else {
                    input_json
                };
                tracing::debug!(
                    target: "jfc::stream",
                    index,
                    tool_name = %tool_name,
                    tool_use_id = %tool_use_id,
                    input_len = assembled.len(),
                    "tool_done"
                );
                // Two failure modes get the same treatment: emit the tool
                // with `Failed` status and a diagnostic output so the model
                // sees a tool_result it can react to.
                //   1. Outer JSON parse failure (assembled bytes don't even
                //      parse as JSON).
                //   2. Inner shape validation failure (`from_value` returns
                //      `ToolInputError`) — same root problem one layer in:
                //      a malformed payload like `{"content": null}` for
                //      Write would otherwise silently default `content: ""`.
                // In both cases we bail with `ToolInput::Generic` carrying
                // the original payload as the summary so the user can see
                // what the model tried to send.
                let parse_outcome: Result<serde_json::Value, _> =
                    if assembled.trim().is_empty() {
                        Ok(serde_json::Value::Object(serde_json::Map::new()))
                    } else {
                        serde_json::from_str(&assembled)
                    };
                let kind = ToolKind::from_name(&tool_name);
                let make_stub = || ToolInput::Generic {
                    summary: if assembled.is_empty() {
                        format!("(empty input for {tool_name})")
                    } else {
                        assembled.clone()
                    },
                };
                // Build through ToolCall::new_pending / new_failed
                // constructors instead of a struct literal. The
                // typestate guards on ToolCall make "constructed in
                // Pending and later transitioned via mark_*" the
                // primary path; new_failed is the carve-out for the
                // malformed-input case where dispatch never happens.
                let id = crate::ids::ToolId::from(tool_use_id.clone());
                let tool = match parse_outcome {
                    Ok(input_val) => match ToolInput::from_value(&tool_name, input_val) {
                        Ok(parsed) => ToolCall::new_pending(id, kind, parsed),
                        Err(err) => {
                            tracing::warn!(
                                target: "jfc::stream",
                                tool_name = %tool_name,
                                tool_use_id = %tool_use_id,
                                input_len = assembled.len(),
                                error = %err,
                                "tool_done: input shape validation failed — failing tool"
                            );
                            let msg = format!(
                                "{err}\n\n\
                                 The tool input was valid JSON but didn't match the \
                                 tool's required schema. Retry with the correct fields."
                            );
                            ToolCall::new_failed(id, kind, make_stub(), ToolOutput::Text(msg))
                        }
                    },
                    Err(err) => {
                        tracing::warn!(
                            target: "jfc::stream",
                            tool_name = %tool_name,
                            tool_use_id = %tool_use_id,
                            input_len = assembled.len(),
                            error = %err,
                            "tool_done: input JSON parse failed — failing tool"
                        );
                        let msg = format!(
                            "Tool input was not valid JSON ({} bytes received): {}\n\n\
                             The provider stream finished before sending a complete \
                             `input` object. Retry the tool call with a properly-formed \
                             JSON input.",
                            assembled.len(),
                            err,
                        );
                        ToolCall::new_failed(id, kind, make_stub(), ToolOutput::Text(msg))
                    }
                };
                tool_accum.remove(&index);
                let _ = tx.send(AppEvent::StreamTool(tool)).await;
            }
            StreamEvent::Done { stop_reason: r } => {
                // Never downgrade from ToolUse → EndTurn.  The OpenAI SSE
                // protocol sends `[DONE]` after the finish_reason chunk.
                // `push_chunk_events_stateful` already emitted Done{ToolUse}
                // from the finish_reason chunk; the subsequent [DONE] line
                // emits Done{EndTurn}.  If we blindly overwrite we lose the
                // ToolUse signal and pending_tool_calls are silently cleared
                // instead of dispatched.
                tracing::debug!(
                    target: "jfc::stream",
                    incoming = ?r, current = ?stop_reason,
                    "StreamEvent::Done"
                );
                if stop_reason != StopReason::ToolUse {
                    stop_reason = r;
                }
            }
            StreamEvent::TextDone { .. } | StreamEvent::ThinkingDone { .. } => {}
            StreamEvent::Usage {
                input_tokens,
                output_tokens,
                cache_read_tokens,
                cache_write_tokens,
            } => {
                tracing::info!(
                    target: "jfc::stream",
                    input_tokens, output_tokens,
                    cache_read_tokens, cache_write_tokens,
                    "stream usage report"
                );
                // Usage stats are non-critical; safe to drop under backpressure.
                if tx
                    .try_send(AppEvent::StreamUsage {
                        input_tokens,
                        output_tokens,
                        cache_read_tokens,
                        cache_write_tokens,
                    })
                    .is_err()
                {
                    tracing::trace!(target: "jfc::stream", "StreamUsage dropped (buffer full)");
                }
            }
            StreamEvent::Error { message } => {
                tracing::error!(target: "jfc::stream", %message, "stream error event");
                let _ = tx.send(AppEvent::StreamError(message)).await;
                return;
            }
        }
    }

    tracing::info!(
        target: "jfc::stream",
        ?stop_reason,
        "stream finished — sending StreamDone"
    );

    // v132 AfterStream hook — fires after the model finished streaming
    // but before the StreamDone AppEvent is sent. Handlers that want
    // to surface end-of-turn cost, run telemetry batching, or trigger
    // session auto-naming can land here.
    crate::hooks::fire(
        crate::hooks::HookPoint::AfterStream,
        &crate::hooks::HookContext::for_session("stream")
            .with_extra("stop_reason", format!("{stop_reason:?}")),
    );

    let _ = tx.send(AppEvent::StreamDone(stop_reason)).await;
}

#[tracing::instrument(target = "jfc::stream", skip(tx, dedup, task_store, provider, model, teammate_event_tx), fields(n = tool_calls.len()))]
pub fn dispatch_tools_batched(
    tool_calls: Vec<ToolCall>,
    tx: &mpsc::Sender<AppEvent>,
    dedup: Arc<Mutex<ReadDedupCache>>,
    task_store: Option<Arc<crate::tasks::TaskStore>>,
    provider: Arc<dyn crate::provider::Provider>,
    model: crate::provider::ModelId,
    teammate_event_tx: mpsc::UnboundedSender<crate::swarm::runner::TeammateEvent>,
    // wg-async: tool batches can run for minutes (Bash, subagents). Hand
    // the spawned scheduler a cancel handle so ESC×2 races the batch
    // against `.cancelled()` rather than orphaning the work.
    cancel: tokio_util::sync::CancellationToken,
) {
    use crate::types::ToolInput;
    use std::sync::atomic::{AtomicUsize, Ordering};

    let cwd = std::env::current_dir().unwrap_or_default();

    let mut regular_calls: Vec<ToolCall> = Vec::new();
    let mut task_calls: Vec<ToolCall> = Vec::new();
    for tc in tool_calls {
        match &tc.input {
            ToolInput::Task(_) => task_calls.push(tc),
            _ => regular_calls.push(tc),
        }
    }

    let task_count = task_calls.len();
    let regular_count = regular_calls.len();
    tracing::info!(
        target: "jfc::stream",
        task_count, regular_count,
        "dispatch_tools_batched: splitting tool calls"
    );
    let pending = Arc::new(AtomicUsize::new(
        task_count + usize::from(!regular_calls.is_empty()),
    ));
    let tx_done = tx.clone();
    let send_all_complete = move || {
        if pending.fetch_sub(1, Ordering::AcqRel) == 1 {
            let _ = tx_done.try_send(AppEvent::AllToolsComplete);
        }
    };

    // Pre-load agent defs once per dispatch so each spawned task can
    // resolve its `subagent_type` without redoing the directory walk.
    let agents = crate::agents::load_agents(&cwd);

    for tc in task_calls {
        let task_input = match tc.input.clone() {
            ToolInput::Task(ti) => ti,
            _ => unreachable!(),
        };

        // ─── Teammate spawn path ─────────────────────────────────────────
        // When `name` + `team_name` are provided, spawn a persistent
        // teammate instead of a one-shot subagent. The teammate runs
        // in-process and communicates via the mailbox system.
        if task_input.is_teammate_spawn() {
            let tx_task = tx.clone();
            let task_id = tc.id.as_str().to_owned();
            let done = send_all_complete.clone();

            let name = task_input.name.clone().unwrap_or_default();
            let team_name = task_input.team_name.clone().unwrap_or_default();
            let agent_id = crate::swarm::types::make_agent_id(&name, &team_name);
            let color = crate::swarm::runner::assign_teammate_color();

            let config = crate::swarm::runner::TeammateRunnerConfig {
                identity: crate::swarm::TeammateIdentity {
                    agent_id: agent_id.clone(),
                    agent_name: name.clone(),
                    team_name: team_name.clone(),
                    color: Some(color.clone()),
                    plan_mode_required: task_input.mode.as_deref() == Some("plan"),
                    parent_session_id: String::new(),
                },
                prompt: task_input.prompt.clone(),
                description: task_input.description.clone(),
                model: task_input.model.clone(),
                agent_type: task_input.subagent_type.clone(),
                provider: provider.clone(),
                model_id: model.clone(),
                system_prompt: None,
            };

            let teammate_event_tx = teammate_event_tx.clone();
            let (runner_task_id, _abort_tx) =
                crate::swarm::runner::start_teammate(config, teammate_event_tx);
            let _ = runner_task_id;

            // Persist the new member into the team file so the team
            // roster on disk matches the runtime spawn list. Without
            // this, `team_helpers::set_member_active` /
            // `set_member_mode` (which look up by name) silently no-op
            // because members are never actually added.
            let member = crate::swarm::types::TeamMember {
                agent_id: agent_id.clone(),
                name: name.clone(),
                agent_type: task_input.subagent_type.clone(),
                model: task_input.model.clone(),
                color: Some(color.clone()),
                plan_mode_required: Some(task_input.mode.as_deref() == Some("plan")),
                joined_at: std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_millis() as u64)
                    .unwrap_or(0),
                cwd: None,
                worktree_path: None,
                backend_type: Some(crate::swarm::types::BackendType::InProcess),
                is_active: Some(true),
                mode: task_input.mode.clone(),
            };
            {
                let team_name = team_name.clone();
                tokio::spawn(async move {
                    if let Err(e) = crate::swarm::team_helpers::add_member(&team_name, member).await
                    {
                        tracing::warn!(
                            target: "jfc::swarm",
                            error = %e,
                            "failed to register spawned teammate in team file"
                        );
                    }
                });
            }

            // Report spawn as a successful tool result
            let result_json = serde_json::json!({
                "status": "teammate_spawned",
                "teammate_id": agent_id,
                "name": name,
                "team_name": team_name,
                "color": color,
                "message": format!("Spawned successfully.\nagent_id: {agent_id}\nname: {name}\nteam_name: {team_name}\nThe agent is now running and will receive instructions via mailbox.")
            });

            // Two task IDs in play here:
            //   - `task_id` (= `tc.id`, e.g. "tooluse_xOqQ…") is the
            //     wire id the API uses to match the tool_use request
            //     with our tool_result reply. It MUST be on the
            //     ToolResult.
            //   - `runner_task_id` (= "teammate-name@team") is the id
            //     the runner stamps onto every Progress / TextDelta /
            //     Completed / Failed event.
            // Register the BackgroundTask under the *runner* id so
            // when those events arrive their lookups hit. Otherwise
            // the task panel reads "No messages yet" forever even
            // though the runner is streaming.
            let runner_task_id = crate::swarm::runner::teammate_task_id(&agent_id);
            // Notify the leader's main loop that a teammate exists so
            // `app.team_context.team_name` and `app.team_context.teammates`
            // get populated. Previously these stayed empty for the
            // entire session, so the team-mode tree (`team-lead` leader,
            // teammate rows) never activated and we fell through to
            // the generic subagent tree even though we were in a team.
            let _ = tx_task.try_send(AppEvent::TeammateSpawned {
                name: name.clone(),
                team_name: team_name.clone(),
                agent_id: agent_id.clone(),
                color: Some(color.clone()),
                agent_type: task_input.subagent_type.clone(),
                cwd: std::env::current_dir()
                    .ok()
                    .map(|p| p.to_string_lossy().into_owned())
                    .unwrap_or_default(),
            });
            let _ = tx_task.try_send(AppEvent::TaskStarted {
                task_id: crate::ids::TaskId::from(runner_task_id.clone()),
                description: format!("spawn teammate: {name}"),
            });

            let _ = tx_task.try_send(AppEvent::ToolResult {
                tool_id: crate::ids::ToolId::from(task_id),
                result: crate::tools::ExecutionResult::success(
                    serde_json::to_string_pretty(&result_json).unwrap_or_default(),
                ),
            });
            done();
            continue;
        }

        // ─── Normal subagent path ────────────────────────────────────────
        let tx_task = tx.clone();
        let provider_task = provider.clone();
        let model_task = model.clone();
        let task_id = tc.id.as_str().to_owned();
        let description = task_input.description.clone();
        let done = send_all_complete.clone();

        // Resolve `subagent_type` to a concrete `AgentDef`. When unset
        // or unknown, falls back to `None` and `execute_task` runs with
        // no system prompt (mirrors the prior, agent-less behavior).
        // Case-insensitive lookup: the model has historically called
        // Task with `subagent_type: "explore"` while we ship agents
        // named "Explore" (markdown-friendly title-case) and v126 also
        // mixes the two. An exact-match miss silently drops the
        // definition — the subagent then runs without its system
        // prompt or tool restrictions and usually exits in <5s with
        // empty output. Fall through with `eq_ignore_ascii_case` so
        // any reasonable casing routes correctly.
        let agent_def = task_input
            .subagent_type
            .as_deref()
            .and_then(|t| agents.iter().find(|a| a.name.eq_ignore_ascii_case(t)))
            .cloned();
        if agent_def.is_none() {
            if let Some(t) = task_input.subagent_type.as_deref() {
                tracing::warn!(
                    target: "jfc::stream",
                    requested = %t,
                    available = ?agents.iter().map(|a| a.name.as_str()).collect::<Vec<_>>(),
                    "subagent_type did not match any loaded agent — running without definition"
                );
            }
        }

        tokio::spawn(async move {
            // If isolation: "worktree", create a git worktree for this agent
            let worktree_info = if task_input.isolation.as_deref() == Some("worktree") {
                let name = format!(
                    "agent-{}",
                    task_id
                        .replace("toolu_", "")
                        .chars()
                        .take(8)
                        .collect::<String>()
                );
                let repo_root = std::env::current_dir().unwrap_or_default();
                match crate::worktrees::create_worktree(&repo_root, &name) {
                    Ok(info) => {
                        tracing::info!(
                            target: "jfc::stream",
                            worktree = %info.path,
                            "task tool: created worktree for isolated agent"
                        );
                        Some(info)
                    }
                    Err(e) => {
                        tracing::warn!(
                            target: "jfc::stream",
                            error = %e,
                            "task tool: failed to create worktree, running in cwd"
                        );
                        None
                    }
                }
            } else {
                None
            };

            tracing::info!(
                target: "jfc::stream",
                task_id = %task_id,
                subagent_type = ?task_input.subagent_type,
                description = %description,
                has_agent_def = agent_def.is_some(),
                "task tool: spawning execute_task"
            );
            let _ = tx_task
                .send(AppEvent::TaskStarted {
                    task_id: crate::ids::TaskId::from(task_id.clone()),
                    description,
                })
                .await;

            let started = std::time::Instant::now();
            // Forward the subagent's streaming text into the main event
            // loop (`AppEvent::AgentChunk`) so the task view fills live
            // rather than showing "No messages yet" until the agent
            // finishes. tx + task_id are passed through; the producer
            // (`execute_task`) emits one event per `TextDelta`.
            //
            // When isolation requested a worktree, hand its path to the
            // subagent as `cwd_override` so any tools it calls (Read,
            // Bash, Edit, etc.) operate inside the isolated checkout.
            // Without this, "isolation" was a name only — the worktree
            // existed on disk but the agent ran against the parent cwd.
            let cwd_override = worktree_info
                .as_ref()
                .map(|info| std::path::PathBuf::from(&info.path));
            let result = crate::tools::execute_task(
                &task_input,
                provider_task.as_ref(),
                model_task,
                Some(&tx_task),
                Some(&task_id),
                agent_def.as_ref(),
                cwd_override,
            )
            .await;
            let elapsed_ms = started.elapsed().as_millis() as u64;

            if result.is_error() {
                tracing::warn!(
                    target: "jfc::stream",
                    task_id = %task_id,
                    elapsed_ms,
                    output_preview = %&result.output[..result.output.len().min(200)],
                    "task tool: execute_task failed"
                );
                let _ = tx_task
                    .send(AppEvent::TaskFailed {
                        task_id: crate::ids::TaskId::from(task_id.clone()),
                        error: result.output.clone(),
                    })
                    .await;
            } else {
                tracing::info!(
                    target: "jfc::stream",
                    task_id = %task_id,
                    elapsed_ms,
                    output_len = result.output.len(),
                    "task tool: execute_task succeeded"
                );
                let _ = tx_task
                    .send(AppEvent::TaskCompleted {
                        task_id: crate::ids::TaskId::from(task_id.clone()),
                        summary: result.output.clone(),
                        elapsed_ms,
                    })
                    .await;
            }

            // Decide the worktree's fate BEFORE sending the ToolResult so the
            // user-visible message can mention the preserved branch
            // when there are uncommitted changes. Mirrors the Claude
            // Code Agent docs: "the worktree is automatically cleaned
            // up if the agent makes no changes; otherwise the path and
            // branch are returned in the result." `git status
            // --porcelain` is the standard "is the working tree clean"
            // signal — quiet, scriptable, exit-code aware.
            let worktree_outcome: Option<(crate::worktrees::WorktreeInfo, bool)> = if let Some(wt) =
                worktree_info
            {
                let dirty = match tokio::process::Command::new("git")
                    .arg("-C")
                    .arg(&wt.path)
                    .arg("status")
                    .arg("--porcelain")
                    .output()
                    .await
                {
                    Ok(out) if out.status.success() => !out.stdout.is_empty(),
                    Ok(out) => {
                        tracing::warn!(
                            target: "jfc::stream",
                            worktree = %wt.path,
                            stderr = %String::from_utf8_lossy(&out.stderr),
                            "git status in worktree returned non-zero — keeping worktree to be safe"
                        );
                        true
                    }
                    Err(e) => {
                        tracing::warn!(
                            target: "jfc::stream",
                            worktree = %wt.path,
                            error = %e,
                            "git status spawn failed — keeping worktree"
                        );
                        true
                    }
                };
                if !dirty {
                    let repo_root = std::env::current_dir().unwrap_or_default();
                    let wt_name = std::path::Path::new(&wt.path)
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or("");
                    match crate::worktrees::remove_worktree(&repo_root, wt_name) {
                        Ok(_) => tracing::info!(
                            target: "jfc::stream",
                            worktree = %wt.path,
                            "worktree had no changes — removed"
                        ),
                        Err(e) => tracing::warn!(
                            target: "jfc::stream",
                            worktree = %wt.path,
                            error = %e,
                            "worktree cleanup failed"
                        ),
                    }
                    Some((wt, false))
                } else {
                    tracing::info!(
                        target: "jfc::stream",
                        worktree = %wt.path,
                        "worktree has uncommitted changes — preserving"
                    );
                    Some((wt, true))
                }
            } else {
                None
            };

            let _ = tx_task
                .send(AppEvent::ToolResult {
                    tool_id: crate::ids::ToolId::from(task_id),
                    result: match &worktree_outcome {
                        Some((wt, true)) => crate::tools::ExecutionResult::success(format!(
                            "{}\n\n[worktree preserved with uncommitted changes]\n\
                         path: {}\nbranch: {}\n\
                         To inspect: cd {} && git diff\n\
                         To merge:   git merge {}\n\
                         To discard: git worktree remove {} && git branch -D {}",
                            result.output,
                            wt.path,
                            wt.branch,
                            wt.path,
                            wt.branch,
                            wt.path,
                            wt.branch,
                        )),
                        Some((_, false)) | None => result,
                    },
                })
                .await;

            done();
        });
    }

    if !regular_calls.is_empty() {
        let batches = scheduler::schedule_tools(regular_calls);
        tracing::debug!(
            target: "jfc::stream",
            batch_count = batches.len(),
            "dispatch_tools_batched: scheduled regular tool batches"
        );
        let tx_clone = tx.clone();
        let done = send_all_complete.clone();
        // wg-async cancellation: race the batch executor against the
        // turn's cancel token. The scheduler itself runs synchronous
        // tool work; a token-cancel cuts off the *await* between tools
        // and lets the spawn return early so its capture set drops.
        let cancel_batch = cancel.clone();
        tokio::spawn(async move {
            tokio::select! {
                biased;
                _ = cancel_batch.cancelled() => {
                    tracing::info!(target: "jfc::stream", "tool batch cancelled via token");
                }
                _ = scheduler::execute_batches(batches, &tx_clone, cwd, dedup, task_store, None) => {}
            }
            done();
        });
    }
}

pub fn should_continue_loop(messages: &[ChatMessage]) -> bool {
    let last = match messages.iter().rev().find(|m| m.role == Role::Assistant) {
        Some(m) => m,
        None => {
            tracing::trace!(target: "jfc::stream", "should_continue_loop: no assistant message found");
            return false;
        }
    };
    let has_tools = last.parts.iter().any(|p| matches!(p, MessagePart::Tool(_)));
    if !has_tools {
        tracing::trace!(target: "jfc::stream", "should_continue_loop: last assistant has no tools");
        return false;
    }
    let all_done = last.parts.iter().all(|p| match p {
        MessagePart::Tool(tc) => {
            tc.status == ToolStatus::Completed || tc.status == ToolStatus::Failed
        }
        _ => true,
    });
    tracing::debug!(
        target: "jfc::stream",
        has_tools, all_done,
        tool_count = last.parts.iter().filter(|p| matches!(p, MessagePart::Tool(_))).count(),
        "should_continue_loop"
    );
    all_done
}

pub async fn continue_agentic_loop(app: &mut App, tx: &mpsc::Sender<AppEvent>) {
    let assistant_idx = app.messages.len();
    tracing::info!(
        target: "jfc::stream",
        assistant_idx,
        model = %app.model,
        total_messages = app.messages.len(),
        "continue_agentic_loop: starting new sub-stream"
    );
    // Debug-only invariant check BEFORE we stage the next assistant
    // slot. If the caller handed us a broken slice (e.g. trailing
    // assistant from the prior round wasn't merged), surface it in
    // the log instead of silently doubling down. Behind cfg() so
    // release builds skip the walk.
    #[cfg(debug_assertions)]
    if let Err(err) = crate::types::validate_turn_invariants_inner(
        &app.messages,
        /* allow_streaming_tail = */ true,
    ) {
        tracing::warn!(
            target: "jfc::stream::invariants",
            error = %err,
            assistant_idx,
            "continue_agentic_loop: turn-invariant violation BEFORE staging new assistant slot"
        );
    }
    app.messages.push(ChatMessage::assistant(String::new()));
    app.streaming_text.clear();
    app.streaming_reasoning.clear();
    // NOTE: do NOT reset streaming_response_bytes here — it accumulates
    // across the entire user turn (all agentic loop iterations). The spinner
    // shows the cumulative token estimate for the full turn, matching v126's
    // responseLengthRef which persists across sub-streams.
    app.streaming_assistant_idx = Some(assistant_idx);
    app.is_streaming = true;
    // The sub-stream clock restarts (Anthropic restarts `output_tokens`
    // per request) but the *user-turn* clock keeps running — set in
    // `handle_submit_text` and only cleared when the loop concludes.
    let now = std::time::Instant::now();
    app.streaming_started_at = Some(now);
    app.streaming_last_token_at = Some(now);
    app.last_usage_output = 0;
    app.usage_apply_baseline = (0, 0, 0, 0);

    let provider = app.provider.clone();
    let messages = build_provider_messages_with_tool_results(&app.messages[..assistant_idx]);
    let model = app.model.clone();
    let tx = tx.clone();
    let interrupt = app.interrupt_flag.clone();
    let cancel = app.cancel_token.clone();

    // wg-async: the agentic continuation IS critical state — it produces
    // the next sub-stream's events. Hand it the cancel token so a mid-loop
    // ESC unwinds it the same way it unwinds the original turn.
    tokio::spawn(async move {
        stream_response(provider, messages, model, tx, interrupt, cancel).await;
    });
}

/// Returns the max output tokens for `model`. Mirrors the
/// `getMaxOutputTokens` helper in opencode-anthropic-auth's
/// `plugin/constants.ts:195` and v126's MODEL_MAX_OUTPUT table.
///
/// Defaults are conservative; Opus/Sonnet 4.x family supports 128k
/// extended output (with the `output-128k-2025-02-19` beta header
/// already in our `ANTHROPIC_BETA` constant). Pre-4.x and Haiku get
/// 16k. Opus 4.0 dated releases are capped at 8k when not streaming
/// (we always stream so this is moot, but the constant stays as a
/// reference).
pub fn max_output_tokens_for(model: &str) -> u32 {
    let m = model.to_lowercase();
    // Opus/Sonnet 4.x family — extended-output 128k support.
    let extended_4x = m.contains("opus-4")
        || m.contains("sonnet-4")
        || m.contains("opus-5")
        || m.contains("sonnet-5");
    if extended_4x {
        return 128_000;
    }
    // Haiku 4.5 caps at 16k.
    if m.contains("haiku-4-5") {
        return 16_384;
    }
    // Older Opus/Sonnet (3.x, 3.5, 3.7).
    if m.contains("opus") || m.contains("sonnet") {
        return 8_192;
    }
    // Unknown / proxy-routed: keep the safe v126 default.
    16_384
}

/// True only for proxy-routed model IDs (Bedrock through LiteLLM/OWUI,
/// Vertex, etc.). The Anthropic-native `thinking` field is rejected by
/// these proxies even when the underlying model is Claude — Bedrock
/// uses its own `additionalModelRequestFields` schema for extended
/// thinking, and the OWUI/LiteLLM passthrough doesn't translate it. Mirrors
/// v126's provider-aware thinking gate (`shouldSendThinking` in cli.js).
fn is_proxy_routed_model(model: &str) -> bool {
    let m = model.to_lowercase();
    m.starts_with("bedrock-")
        || m.starts_with("aws-")
        || m.starts_with("vertex-")
        || m.starts_with("litellm-")
        || m.starts_with("openrouter-")
        || m.starts_with("openwebui-")
}

/// Returns true for models that require `{"type": "adaptive"}` thinking and
/// reject the legacy `budget_tokens` parameter. Matches v126's
/// `modelSupportsAdaptiveThinking` (claude.ts:1602). Proxy-routed
/// equivalents (bedrock-*, vertex-*) are excluded — adaptive thinking is
/// an Anthropic-native parameter the proxies haven't adopted.
fn model_supports_adaptive_thinking(model: &str) -> bool {
    if is_proxy_routed_model(model) {
        return false;
    }
    let m = model.to_lowercase();
    // Opus 4.6, Opus 4.7, Sonnet 4.6 — all reject budget_tokens.
    // Future models (5.x) will also use adaptive, so default to adaptive
    // for any model whose version segment is >= 4.6.
    m.contains("opus-4-6")
        || m.contains("opus-4-7")
        || m.contains("opus-4-8")
        || m.contains("opus-4-9")
        || m.contains("opus-5")
        || m.contains("sonnet-4-6")
        || m.contains("sonnet-4-7")
        || m.contains("sonnet-4-8")
        || m.contains("sonnet-4-9")
        || m.contains("sonnet-5")
}

/// Returns true if the model supports thinking at all. Haiku 4.5 does NOT
/// support the thinking parameter — sending it causes a 400. Opus 4.x and
/// Sonnet 4.5+ do support thinking. Proxy-routed model IDs (`bedrock-*`,
/// `aws-*`, `vertex-*`, `litellm-*`, `openrouter-*`) default to NOT
/// thinking even when the underlying model is Claude — proxies frequently
/// reject the field with `400 invalid_request_error: adaptive thinking is
/// not supported on this model`. The user must explicitly opt back in via
/// config if a specific deployment supports it.
fn model_supports_thinking(model: &str) -> bool {
    if is_proxy_routed_model(model) {
        tracing::debug!(
            target: "jfc::stream",
            model,
            "model_supports_thinking: false (proxy-routed)"
        );
        return false;
    }
    let m = model.to_lowercase();
    // Opus 4.5 returns 400 "adaptive thinking is not supported on this
    // model" for both adaptive AND legacy budget_tokens — the API
    // rejects the entire `thinking` field for that release. Other Opus
    // versions (4.6+) need adaptive thinking and are routed by the
    // `model_supports_adaptive_thinking` predicate first, so reaching
    // this branch with `opus-4-5` means we'd otherwise send the legacy
    // form and get a 400. Mark it as no-thinking so the request goes
    // through cleanly.
    if m.contains("opus-4-5") {
        return false;
    }
    // Known thinking-capable Anthropic-native families
    let supports = m.contains("opus")
        || m.contains("sonnet-4-5")
        || m.contains("sonnet-4-6")
        || m.contains("sonnet-4-7")
        || m.contains("sonnet-4-8")
        || m.contains("sonnet-4-9")
        || m.contains("sonnet-5");
    tracing::debug!(
        target: "jfc::stream",
        model, supports,
        "model_supports_thinking"
    );
    supports
}

pub fn build_provider_messages(msgs: &[ChatMessage]) -> Vec<ProviderMessage> {
    let out: Vec<ProviderMessage> = msgs
        .iter()
        .filter_map(|m| {
            let role = match m.role {
                Role::User => ProviderRole::User,
                Role::Assistant => ProviderRole::Assistant,
            };
            let text: String = m
                .parts
                .iter()
                .filter_map(|p| match p {
                    MessagePart::Text(t) if !t.is_empty() => Some(t.as_str()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join("\n");
            if text.is_empty() {
                return None;
            }
            Some(ProviderMessage {
                role,
                content: vec![ProviderContent::Text(text)],
            })
        })
        .collect();
    tracing::debug!(
        target: "jfc::stream",
        input_messages = msgs.len(),
        output_messages = out.len(),
        "build_provider_messages (text-only)"
    );
    ensure_user_last(out)
}

/// Ensure the message list ends with a user-role message before sending to
/// the provider.
///
/// ## Why this is needed
///
/// Opus 4.6+ rejects any trailing assistant message with:
///     `"This model does not support assistant message prefill.
///      The conversation must end with a user message."`
///
/// Bedrock-via-LiteLLM returns the same error for any Anthropic model.
///
/// The native pre-4.6 API *silently* treats a trailing assistant as prefill,
/// which is also wrong for the agentic continuation use case (we want a
/// fresh assistant turn, not a continuation of an old one).
///
/// ## What we do
///
/// 1. Strip trailing assistant messages that are empty (only blank text).
/// 2. If the last message is still an assistant with real content (e.g. a
///    compact boundary summary, or a text-only end_turn that ended up last
///    due to filtering), **keep it but append a synthetic empty user turn**
///    so the API sees user-last ordering. This matches v126's behavior:
///    `normalizeMessagesForAPI` never produces a conversation ending in
///    assistant — tool_result blocks always follow tool_use blocks in a
///    trailing user message.
fn ensure_user_last(mut msgs: Vec<ProviderMessage>) -> Vec<ProviderMessage> {
    // First: strip trailing empty assistants (placeholder leak from continue_agentic_loop)
    while msgs
        .last()
        .map(|m| {
            m.role == ProviderRole::Assistant
                && m.content.iter().all(|c| match c {
                    ProviderContent::Text(s) => s.trim().is_empty(),
                    _ => false,
                })
        })
        .unwrap_or(false)
    {
        tracing::info!(
            target: "jfc::stream",
            "stripped trailing empty assistant before send"
        );
        msgs.pop();
    }

    // Second: if the conversation still ends with an assistant (real content),
    // append a minimal user turn. The Anthropic API requires alternating
    // user/assistant roles and user-last ordering.
    if msgs
        .last()
        .map(|m| m.role == ProviderRole::Assistant)
        .unwrap_or(false)
    {
        tracing::info!(
            target: "jfc::stream",
            "appending synthetic user turn to satisfy user-last ordering"
        );
        msgs.push(ProviderMessage {
            role: ProviderRole::User,
            content: vec![ProviderContent::Text(
                "Continue from where you left off.".to_owned(),
            )],
        });
    }

    // Third: merge consecutive same-role messages. The Anthropic API requires
    // strictly alternating user/assistant turns. Consecutive same-role
    // messages happen when: (a) a compact_boundary (assistant) is followed by
    // a text-only assistant, (b) queued prompts produce adjacent user
    // messages, (c) filtering removes messages and collapses the alternation.
    let mut merged: Vec<ProviderMessage> = Vec::with_capacity(msgs.len());
    for msg in msgs {
        if let Some(last) = merged.last_mut() {
            if last.role == msg.role {
                last.content.extend(msg.content);
                continue;
            }
        }
        merged.push(msg);
    }
    merged
}

fn build_provider_messages_with_tool_results(msgs: &[ChatMessage]) -> Vec<ProviderMessage> {
    let mut out = Vec::new();
    let mut tool_use_count = 0usize;
    let mut tool_result_count = 0usize;
    let mut abandoned_count = 0usize;
    for m in msgs {
        let role = match m.role {
            Role::User => ProviderRole::User,
            Role::Assistant => ProviderRole::Assistant,
        };
        let text: String = m
            .parts
            .iter()
            .filter_map(|p| match p {
                MessagePart::Text(t) if !t.is_empty() => Some(t.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n");

        let tool_uses: Vec<ProviderContent> = m
            .parts
            .iter()
            .filter_map(|p| match p {
                MessagePart::Tool(tc) => {
                    tool_use_count += 1;
                    Some(ProviderContent::ToolUse {
                        id: tc.id.as_str().to_owned(),
                        name: tc.kind.api_name().to_owned(),
                        input: tc.input.to_value(),
                    })
                }
                _ => None,
            })
            .collect();

        let tool_results: Vec<ProviderContent> = m
            .parts
            .iter()
            .filter_map(|p| match p {
                MessagePart::Tool(tc) => {
                    // After ExecutionStatus unification, tools can in
                    // principle land in any of six states. In practice
                    // tools never reach Idle (that's a Task-only state
                    // for sub-agents that are alive but quiescent), and
                    // Cancelled is treated as a flavor of "the tool was
                    // never executed" — same back-pressure to the model
                    // as Pending/Running abandonment, just with a
                    // slightly different stub message so the model can
                    // tell why the tool didn't run.
                    let (result_text, is_error) = match tc.status {
                        ToolStatus::Completed | ToolStatus::Failed => {
                            tool_result_count += 1;
                            let text = match &tc.output {
                                ToolOutput::Text(s) => s.clone(),
                                ToolOutput::LargeText(lt) => lt.content.clone(),
                                ToolOutput::Command {
                                    stdout,
                                    stderr,
                                    exit_code,
                                } => format!(
                                    "exit: {}\nstdout: {}\nstderr: {}",
                                    exit_code.unwrap_or(-1),
                                    stdout,
                                    stderr
                                ),
                                ToolOutput::FileContent { content, .. } => content.clone(),
                                ToolOutput::FileList(files) => files.join("\n"),
                                ToolOutput::Diff(d) => {
                                    format!("Applied diff to {}", d.file_path)
                                }
                                ToolOutput::Empty => String::new(),
                            };
                            (text, tc.status == ToolStatus::Failed)
                        }
                        ToolStatus::Cancelled => {
                            abandoned_count += 1;
                            (
                                "Tool was cancelled before it could run. \
                                 No output was produced."
                                    .to_owned(),
                                true,
                            )
                        }
                        ToolStatus::Idle => {
                            // Tools never enter Idle in normal flow —
                            // this is a programmer error, not a runtime
                            // condition. Log and treat as abandoned so
                            // we still ship a well-formed tool_result
                            // (Anthropic 400s on orphaned tool_use).
                            tracing::error!(
                                target: "jfc::stream",
                                tool_id = %tc.id.as_str(),
                                "tool reached Idle state — should not happen"
                            );
                            abandoned_count += 1;
                            (
                                "Tool was abandoned: unexpected Idle state. \
                                 No output was produced."
                                    .to_owned(),
                                true,
                            )
                        }
                        ToolStatus::Pending | ToolStatus::Running => {
                            abandoned_count += 1;
                            (
                                "Tool was abandoned: the user moved on before \
                                 approving or executing it. No output was produced."
                                    .to_owned(),
                                true,
                            )
                        }
                    };
                    Some(ProviderContent::ToolResult {
                        tool_use_id: tc.id.as_str().to_owned(),
                        content: cap_tool_result(&result_text),
                        is_error,
                    })
                }
                _ => None,
            })
            .collect();

        let mut assistant_content = Vec::new();
        if !text.is_empty() {
            assistant_content.push(ProviderContent::Text(text.clone()));
        }
        assistant_content.extend(tool_uses);

        if !assistant_content.is_empty() {
            out.push(ProviderMessage {
                role: role.clone(),
                content: assistant_content,
            });
        } else if !text.is_empty() {
            out.push(ProviderMessage {
                role: role.clone(),
                content: vec![ProviderContent::Text(text)],
            });
        }

        if !tool_results.is_empty() {
            out.push(ProviderMessage {
                role: ProviderRole::User,
                content: tool_results,
            });
        }
    }
    // Drain attachments staged by tool dispatchers (currently: PDFs
    // ingested by Read). Append them to the LAST user-role message
    // — that's the tool_results message we just emitted in the loop
    // above when a Read tool just ran, or the user's prompt when no
    // tool fired. Skipping the append here would silently lose the
    // PDF, so this is the load-bearing wire.
    let pending = crate::tools::take_pending_tool_attachments();
    let pending_count = pending.len();
    if !pending.is_empty() {
        let attached: Vec<ProviderContent> = pending
            .into_iter()
            .map(ProviderContent::Attachment)
            .collect();
        // Find the last user message and append; if none exists,
        // create one (defensive — `ensure_user_last` enforces this
        // anyway, but doing it eagerly keeps the message structure
        // predictable).
        if let Some(last_user) = out
            .iter_mut()
            .rfind(|m| matches!(m.role, ProviderRole::User))
        {
            last_user.content.extend(attached);
        } else {
            out.push(ProviderMessage {
                role: ProviderRole::User,
                content: attached,
            });
        }
    }
    tracing::debug!(
        target: "jfc::stream",
        input_messages = msgs.len(),
        output_messages = out.len(),
        tool_use_count, tool_result_count, abandoned_count, pending_count,
        "build_provider_messages_with_tool_results"
    );
    ensure_user_last(out)
}

#[cfg(test)]
mod pdf_attachment_drain_tests {
    use super::*;
    use crate::types::ChatMessage;

    /// Test-isolation lock so this module's tests serialize their
    /// access to the process-global pending-attachments queue.
    /// Otherwise running two tests in parallel would leak state
    /// between them and break the queue-empty assertions.
    fn drain_test_lock() -> &'static std::sync::Mutex<()> {
        static LOCK: std::sync::OnceLock<std::sync::Mutex<()>> =
            std::sync::OnceLock::new();
        LOCK.get_or_init(|| std::sync::Mutex::new(()))
    }

    /// Normal: when a PDF is staged via push_pending_tool_attachment,
    /// build_provider_messages_with_tool_results drains the queue
    /// and appends an Attachment content block to the LAST user
    /// message. Without this glue the model never sees the PDF.
    #[test]
    fn pending_pdf_lands_in_last_user_message_normal() {
        let _guard = drain_test_lock().lock().unwrap_or_else(|p| p.into_inner());
        let _ = crate::tools::take_pending_tool_attachments();

        let pdf = crate::attachments::Attachment {
            kind: crate::attachments::AttachmentKind::ApplicationPdf,
            bytes: b"%PDF-1.7\nfake".to_vec(),
        };
        crate::tools::push_pending_tool_attachment(pdf);

        let msgs = vec![ChatMessage::user("read this please".to_string())];
        let provider_msgs = build_provider_messages_with_tool_results(&msgs);
        let last_user = provider_msgs
            .iter()
            .rfind(|m| matches!(m.role, ProviderRole::User))
            .expect("must have a user message");
        let attachment_count = last_user
            .content
            .iter()
            .filter(|c| matches!(c, ProviderContent::Attachment(_)))
            .count();
        assert_eq!(
            attachment_count, 1,
            "expected exactly one attachment on the last user message"
        );
    }

    /// Robust: a second build_provider_messages_with_tool_results
    /// call AFTER the drain must NOT see the PDF again — the queue
    /// should be empty so the same attachment doesn't replay every
    /// turn (which would balloon token cost and produce duplicate
    /// document blocks).
    #[test]
    fn drain_clears_pending_queue_robust() {
        let _guard = drain_test_lock().lock().unwrap_or_else(|p| p.into_inner());
        let _ = crate::tools::take_pending_tool_attachments();

        let pdf = crate::attachments::Attachment {
            kind: crate::attachments::AttachmentKind::ApplicationPdf,
            bytes: b"%PDF-1.7\n".to_vec(),
        };
        crate::tools::push_pending_tool_attachment(pdf);

        let msgs = vec![ChatMessage::user("first".to_string())];
        let _first_round = build_provider_messages_with_tool_results(&msgs);
        let second_round = build_provider_messages_with_tool_results(&msgs);
        let attachment_count = second_round
            .iter()
            .flat_map(|m| m.content.iter())
            .filter(|c| matches!(c, ProviderContent::Attachment(_)))
            .count();
        assert_eq!(
            attachment_count, 0,
            "second build must not replay the drained attachment"
        );
    }
}

#[cfg(test)]
mod ensure_user_last_tests {
    use super::*;

    fn user_text(s: &str) -> ProviderMessage {
        ProviderMessage {
            role: ProviderRole::User,
            content: vec![ProviderContent::Text(s.to_owned())],
        }
    }

    fn assistant_text(s: &str) -> ProviderMessage {
        ProviderMessage {
            role: ProviderRole::Assistant,
            content: vec![ProviderContent::Text(s.to_owned())],
        }
    }

    // Normal: the exact bug from the screenshot — `continue_agentic_loop`
    // pushes an empty assistant placeholder, the builder echoes it, Bedrock
    // explodes. After the strip, the conversation ends on the user turn.
    #[test]
    fn strip_drops_trailing_empty_assistant_normal() {
        let input = vec![user_text("hi"), assistant_text("")];
        let out = ensure_user_last(input);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].role, ProviderRole::User);
    }

    // Normal: whitespace-only text counts as empty — a streamed turn that
    // only emitted a newline before being interrupted is still no content.
    #[test]
    fn strip_drops_trailing_whitespace_only_assistant_normal() {
        let input = vec![user_text("hi"), assistant_text("   \n")];
        let out = ensure_user_last(input);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].role, ProviderRole::User);
    }

    // Normal: real assistant text at the end gets a synthetic user turn
    // appended so the API sees user-last ordering. Opus 4.6 rejects trailing
    // assistant even with content.
    #[test]
    fn appends_user_when_assistant_has_real_content() {
        let input = vec![user_text("hi"), assistant_text("hello")];
        let out = ensure_user_last(input);
        assert_eq!(out.len(), 3);
        assert_eq!(out[1].role, ProviderRole::Assistant);
        assert_eq!(out[2].role, ProviderRole::User);
    }

    // Normal: an assistant turn with a tool_use gets a synthetic user turn
    // appended (the tool_result would normally follow, but ensure_user_last
    // acts as a safety net).
    #[test]
    fn appends_user_when_assistant_has_only_toolcall() {
        let assistant_with_tool = ProviderMessage {
            role: ProviderRole::Assistant,
            content: vec![ProviderContent::ToolUse {
                id: "toolu_1".to_owned(),
                name: "Bash".to_owned(),
                input: serde_json::json!({"command": "ls"}),
            }],
        };
        let input = vec![user_text("hi"), assistant_with_tool];
        let out = ensure_user_last(input);
        assert_eq!(out.len(), 3);
        assert_eq!(out[2].role, ProviderRole::User);
    }

    // Normal: if the conversation already ends with a user message (the
    // common tool_result-injection case), the function is a no-op.
    #[test]
    fn no_op_on_user_last_normal() {
        let input = vec![assistant_text("hi"), user_text("ok")];
        let out = ensure_user_last(input);
        assert_eq!(out.len(), 2);
        assert_eq!(out[1].role, ProviderRole::User);
    }

    // Robust: empty input must round-trip — no panic.
    #[test]
    fn no_op_on_empty_input_robust() {
        let out = ensure_user_last(Vec::<ProviderMessage>::new());
        assert!(out.is_empty());
    }

    // Normal: multiple trailing empty assistants are ALL stripped.
    #[test]
    fn strips_multiple_trailing_empty_assistants() {
        let input = vec![user_text("hi"), assistant_text(""), assistant_text("")];
        let out = ensure_user_last(input);
        // Both empties stripped, "hi" remains. User-last already satisfied.
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].role, ProviderRole::User);
    }

    // Normal: consecutive same-role messages get merged.
    #[test]
    fn merges_consecutive_user_messages() {
        let input = vec![user_text("a"), user_text("b"), assistant_text("c")];
        let out = ensure_user_last(input);
        // Two users merged into one, then assistant, then synthetic user
        assert_eq!(out.len(), 3);
        assert_eq!(out[0].role, ProviderRole::User);
        assert_eq!(out[0].content.len(), 2); // merged
        assert_eq!(out[1].role, ProviderRole::Assistant);
        assert_eq!(out[2].role, ProviderRole::User); // synthetic
    }
}

#[cfg(test)]
mod thinking_gate_tests {
    use super::*;

    // Bedrock-routed Claude rejects `thinking` even though the underlying
    // model is Claude. Regression for the user's screenshot showing
    // `Anthropic API error 400: adaptive thinking is not supported on
    // this model` for `bedrock-claude-4-6-opus`.
    #[test]
    fn bedrock_routed_models_skip_thinking_robust() {
        assert!(!model_supports_thinking("bedrock-claude-4-6-opus"));
        assert!(!model_supports_thinking("bedrock-claude-3-5-sonnet"));
        assert!(!model_supports_adaptive_thinking("bedrock-claude-4-6-opus"));
    }

    // Other proxy prefixes also default off — none of them reliably
    // pass the thinking field through.
    #[test]
    fn other_proxy_prefixes_skip_thinking_robust() {
        assert!(!model_supports_thinking("vertex-claude-4-6-opus"));
        assert!(!model_supports_thinking("aws-claude-4-6-opus"));
        assert!(!model_supports_thinking("litellm-claude-4-6-opus"));
        assert!(!model_supports_thinking("openrouter-claude-4-6-opus"));
    }

    // Anthropic-native model IDs unchanged: they keep getting adaptive
    // thinking when version >= 4.6, legacy budget_tokens otherwise.
    #[test]
    fn anthropic_native_models_keep_thinking_normal() {
        assert!(model_supports_adaptive_thinking("claude-opus-4-6"));
        assert!(model_supports_adaptive_thinking("claude-opus-4-7"));
        assert!(model_supports_adaptive_thinking("claude-sonnet-4-6"));
        // Opus 4.5 rejects the entire `thinking` field — see
        // `model_supports_thinking` for context. Excluded explicitly so
        // a regression doesn't silently put the request back into the
        // 400-loop.
        assert!(!model_supports_thinking("claude-opus-4-5"));
        assert!(model_supports_thinking("claude-opus-4-6"));
    }

    // Haiku 4.5 doesn't support thinking at all on either path.
    #[test]
    fn haiku_excluded_robust() {
        assert!(!model_supports_thinking("claude-haiku-4-5"));
        assert!(!model_supports_adaptive_thinking("claude-haiku-4-5"));
    }

    // ── is_proxy_routed_model: each prefix is its own equivalence class ──

    // Normal: every documented proxy prefix returns true. The renderer +
    // stream pipeline decide whether to send `thinking` based on this gate,
    // so adding a new proxy means adding a row here.
    #[test]
    fn is_proxy_routed_recognizes_all_prefixes_normal() {
        for id in [
            "bedrock-claude-4-6-opus",
            "aws-claude-4-6-opus",
            "vertex-claude-4-6-opus",
            "litellm-claude-4-6-opus",
            "openrouter-claude-4-6-opus",
            "openwebui-claude-4-6-opus",
        ] {
            assert!(is_proxy_routed_model(id), "expected proxy match for {id}");
        }
    }

    // Robust: case-insensitive matching — uppercase variants must still hit
    // the proxy rules. v126's gate normalizes via lowercase before checking.
    #[test]
    fn is_proxy_routed_is_case_insensitive_robust() {
        assert!(is_proxy_routed_model("BEDROCK-CLAUDE-4-6-OPUS"));
        assert!(is_proxy_routed_model("Vertex-Claude"));
    }

    // Robust: an Anthropic-native id (no proxy prefix) is NOT classified as
    // proxy-routed even though it contains substrings that look prefix-like.
    #[test]
    fn is_proxy_routed_native_anthropic_negative_robust() {
        assert!(!is_proxy_routed_model("claude-opus-4-7"));
        assert!(!is_proxy_routed_model("claude-sonnet-4-6"));
        assert!(!is_proxy_routed_model("claude-haiku-4-5"));
    }

    // Robust: empty string defaults to false — the unknown-model code paths
    // shouldn't be tricked into the proxy branch by garbage inputs.
    #[test]
    fn is_proxy_routed_empty_returns_false_robust() {
        assert!(!is_proxy_routed_model(""));
    }

    // ── max_output_tokens_for: every model family has a tested branch ────

    // Normal: 4.x extended-output Opus / Sonnet → 128k. The single test
    // validates each variant arm of the lowercase contains() chain.
    #[test]
    fn max_output_4x_extended_normal() {
        assert_eq!(max_output_tokens_for("claude-opus-4-7"), 128_000);
        assert_eq!(max_output_tokens_for("claude-opus-4-6"), 128_000);
        assert_eq!(max_output_tokens_for("claude-sonnet-4-6"), 128_000);
        assert_eq!(max_output_tokens_for("claude-sonnet-4-5"), 128_000);
        // Future-proofing: 5.x lands in the same bucket.
        assert_eq!(max_output_tokens_for("claude-opus-5-0"), 128_000);
        assert_eq!(max_output_tokens_for("claude-sonnet-5-0"), 128_000);
    }

    // Normal: Haiku 4.5 caps at 16k — distinct from Opus/Sonnet 4.x even
    // though both share the "4.5" version segment.
    #[test]
    fn max_output_haiku_4_5_normal() {
        assert_eq!(max_output_tokens_for("claude-haiku-4-5"), 16_384);
        assert_eq!(max_output_tokens_for("claude-haiku-4-5-20251001"), 16_384);
    }

    // Normal: 3.x families get 8k. Distinct from the 4.x branch above.
    #[test]
    fn max_output_legacy_opus_sonnet_normal() {
        assert_eq!(max_output_tokens_for("claude-3-7-sonnet-20250219"), 8_192);
        assert_eq!(max_output_tokens_for("claude-opus-3-5"), 8_192);
    }

    // Robust: an unknown / proxy-routed id falls through to the safe v126
    // default of 16k. Matches the comment-documented contract.
    #[test]
    fn max_output_unknown_falls_back_robust() {
        assert_eq!(max_output_tokens_for("bedrock-claude-mystery"), 16_384);
        assert_eq!(max_output_tokens_for("totally-new-model"), 16_384);
        assert_eq!(max_output_tokens_for(""), 16_384);
    }

    // Robust: case-insensitive — the helper lowercases internally so
    // PascalCase or all-caps ids resolve correctly.
    #[test]
    fn max_output_case_insensitive_robust() {
        assert_eq!(max_output_tokens_for("CLAUDE-OPUS-4-7"), 128_000);
        assert_eq!(max_output_tokens_for("Claude-Haiku-4-5"), 16_384);
    }

    // ── model_supports_thinking edge cases ───────────────────────────────

    // Robust: Sonnet 4.4 (a hypothetical or pre-release) should NOT light up
    // adaptive thinking — only 4.5+ Sonnet families do. Catches off-by-one
    // version bumps.
    #[test]
    fn sonnet_below_4_5_is_not_adaptive_robust() {
        assert!(!model_supports_adaptive_thinking("claude-sonnet-4-0"));
        assert!(!model_supports_adaptive_thinking("claude-sonnet-3-7"));
    }

    // Normal: legacy-thinking sonnet 4.5 returns true on the budget branch
    // (used as the second arm in stream_response after adaptive).
    #[test]
    fn sonnet_4_5_supports_thinking_normal() {
        assert!(model_supports_thinking("claude-sonnet-4-5"));
        assert!(model_supports_thinking("claude-sonnet-4-5-20250929"));
    }
}

#[cfg(test)]
mod build_provider_messages_tests {
    use super::*;

    fn user_msg(text: &str) -> ChatMessage {
        let mut m = ChatMessage::user(text.to_owned());
        m.parts = vec![MessagePart::Text(text.to_owned())];
        m
    }

    fn assistant_msg(text: &str) -> ChatMessage {
        let mut m = ChatMessage::assistant(text.to_owned());
        m.parts = vec![MessagePart::Text(text.to_owned())];
        m
    }

    fn assistant_with_parts(parts: Vec<MessagePart>) -> ChatMessage {
        ChatMessage::assistant_parts(parts)
    }

    fn make_tool_call(
        id: &str,
        kind: ToolKind,
        status: ToolStatus,
        output: ToolOutput,
    ) -> ToolCall {
        ToolCall {
            id: crate::ids::ToolId::from(id),
            kind,
            status,
            input: ToolInput::Generic {
                summary: "x".into(),
            },
            output,
            display: crate::types::ToolDisplayState::DEFAULT,
            elapsed_ms: None,
            started_at: None,
        }
    }

    // Normal: text-only conversation maps 1:1 to ProviderMessage::Text. The
    // ensure_user_last invariant kicks in if the conversation ended with the
    // assistant turn — we exercise that elsewhere.
    #[test]
    fn build_text_only_normal() {
        let msgs = vec![user_msg("hi"), assistant_msg("hello")];
        let out = build_provider_messages(&msgs);
        // Three messages: user, assistant, synthetic-user-trailer.
        assert_eq!(out.len(), 3);
        assert_eq!(out[0].role, ProviderRole::User);
        assert_eq!(out[1].role, ProviderRole::Assistant);
        assert_eq!(out[2].role, ProviderRole::User);
    }

    // Normal: a message with multiple text parts joins them with newlines so
    // the model sees a single coherent block per turn.
    #[test]
    fn build_multi_text_part_joins_with_newlines_normal() {
        let m = ChatMessage {
            role: Role::User,
            parts: vec![
                MessagePart::Text("first".into()),
                MessagePart::Text("second".into()),
            ],
            agent_name: None,
            model_name: None,
            cost_tier: None,
            elapsed: None,
            usage: None,
        };
        let out = build_provider_messages(&[m]);
        assert_eq!(out.len(), 1);
        match &out[0].content[0] {
            ProviderContent::Text(t) => assert_eq!(t, "first\nsecond"),
            _ => panic!("expected text content"),
        }
    }

    // Robust: empty / whitespace-only messages drop out entirely so the API
    // doesn't see a degenerate user turn (which Bedrock rejects with 400).
    #[test]
    fn build_drops_empty_text_messages_robust() {
        let m = ChatMessage {
            role: Role::User,
            parts: vec![MessagePart::Text(String::new())],
            agent_name: None,
            model_name: None,
            cost_tier: None,
            elapsed: None,
            usage: None,
        };
        let out = build_provider_messages(&[m]);
        // Empty input → nothing emitted, ensure_user_last leaves the result
        // empty too because there's no trailing assistant to fix up.
        assert!(out.is_empty());
    }

    // Robust: empty input produces empty output (no synthetic injection on
    // a fully-empty conversation).
    #[test]
    fn build_empty_input_robust() {
        let out = build_provider_messages(&[]);
        assert!(out.is_empty());
    }

    // ── build_provider_messages_with_tool_results ─────────────────────────

    // Normal: assistant turn with a completed tool produces a 2-message pair
    // — the assistant's tool_use, then the user's tool_result.
    #[test]
    fn build_with_tool_results_completed_pair_normal() {
        let tool = make_tool_call(
            "toolu_a",
            ToolKind::Bash,
            ToolStatus::Completed,
            ToolOutput::Text("hello world".into()),
        );
        let msgs = vec![
            user_msg("run ls"),
            assistant_with_parts(vec![MessagePart::Tool(tool)]),
        ];
        let out = build_provider_messages_with_tool_results(&msgs);
        // user, assistant(tool_use), user(tool_result)
        assert_eq!(out.len(), 3);
        assert_eq!(out[1].role, ProviderRole::Assistant);
        match &out[1].content[0] {
            ProviderContent::ToolUse { id, .. } => assert_eq!(id, "toolu_a"),
            _ => panic!("expected ToolUse"),
        }
        assert_eq!(out[2].role, ProviderRole::User);
        match &out[2].content[0] {
            ProviderContent::ToolResult {
                tool_use_id,
                content,
                is_error,
            } => {
                assert_eq!(tool_use_id, "toolu_a");
                assert_eq!(content, "hello world");
                assert!(!is_error);
            }
            _ => panic!("expected ToolResult"),
        }
    }

    // Normal: a Failed tool surfaces as is_error=true so the model can react
    // to the failure on its next turn.
    #[test]
    fn build_with_tool_results_failed_marks_is_error_normal() {
        let tool = make_tool_call(
            "toolu_b",
            ToolKind::Bash,
            ToolStatus::Failed,
            ToolOutput::Text("permission denied".into()),
        );
        let msgs = vec![
            user_msg("run rm -rf /"),
            assistant_with_parts(vec![MessagePart::Tool(tool)]),
        ];
        let out = build_provider_messages_with_tool_results(&msgs);
        let last = out.last().unwrap();
        match &last.content[0] {
            ProviderContent::ToolResult { is_error, .. } => {
                assert!(*is_error, "Failed tool must be flagged is_error");
            }
            _ => panic!("expected ToolResult"),
        }
    }

    // Robust: a Pending / Running tool was abandoned (the user moved on
    // without approving). The builder synthesizes a stub error result so the
    // API sees a well-formed tool_result for every tool_use — Anthropic 400s
    // on orphaned tool_use blocks.
    #[test]
    fn build_with_tool_results_pending_synthesizes_abandoned_stub_robust() {
        let tool = make_tool_call(
            "toolu_orphan",
            ToolKind::Bash,
            ToolStatus::Pending,
            ToolOutput::Empty,
        );
        let msgs = vec![
            user_msg("hi"),
            assistant_with_parts(vec![MessagePart::Tool(tool)]),
        ];
        let out = build_provider_messages_with_tool_results(&msgs);
        let last = out.last().unwrap();
        match &last.content[0] {
            ProviderContent::ToolResult {
                content, is_error, ..
            } => {
                assert!(*is_error, "abandoned tool must be flagged is_error");
                assert!(
                    content.contains("abandoned"),
                    "abandoned-tool stub must mention abandonment, got: {content}"
                );
            }
            _ => panic!("expected ToolResult"),
        }
    }

    // Normal: Command output formats to "exit/stdout/stderr" tri-line.
    #[test]
    fn build_with_tool_results_command_output_formats_normal() {
        let tool = make_tool_call(
            "toolu_c",
            ToolKind::Bash,
            ToolStatus::Completed,
            ToolOutput::Command {
                stdout: "ok\n".into(),
                stderr: String::new(),
                exit_code: Some(0),
            },
        );
        let msgs = vec![
            user_msg("run"),
            assistant_with_parts(vec![MessagePart::Tool(tool)]),
        ];
        let out = build_provider_messages_with_tool_results(&msgs);
        let last = out.last().unwrap();
        match &last.content[0] {
            ProviderContent::ToolResult { content, .. } => {
                assert!(content.contains("exit: 0"));
                assert!(content.contains("stdout: ok"));
                assert!(content.contains("stderr:"));
            }
            _ => panic!("expected ToolResult"),
        }
    }

    // Normal: FileContent → content string passes through untouched.
    #[test]
    fn build_with_tool_results_file_content_normal() {
        let tool = make_tool_call(
            "toolu_d",
            ToolKind::Read,
            ToolStatus::Completed,
            ToolOutput::FileContent {
                path: "/tmp/x.rs".into(),
                content: "fn main() {}".into(),
                language: "rust".into(),
            },
        );
        let msgs = vec![
            user_msg("read x.rs"),
            assistant_with_parts(vec![MessagePart::Tool(tool)]),
        ];
        let out = build_provider_messages_with_tool_results(&msgs);
        match &out.last().unwrap().content[0] {
            ProviderContent::ToolResult { content, .. } => {
                assert_eq!(content, "fn main() {}");
            }
            _ => panic!("expected ToolResult"),
        }
    }

    // Normal: FileList output → joined-with-newlines.
    #[test]
    fn build_with_tool_results_file_list_normal() {
        let tool = make_tool_call(
            "toolu_e",
            ToolKind::Glob,
            ToolStatus::Completed,
            ToolOutput::FileList(vec!["/a".into(), "/b".into(), "/c".into()]),
        );
        let msgs = vec![
            user_msg("glob"),
            assistant_with_parts(vec![MessagePart::Tool(tool)]),
        ];
        let out = build_provider_messages_with_tool_results(&msgs);
        match &out.last().unwrap().content[0] {
            ProviderContent::ToolResult { content, .. } => {
                assert_eq!(content, "/a\n/b\n/c");
            }
            _ => panic!("expected ToolResult"),
        }
    }

    // Normal: an assistant turn that has both prose AND a tool emits both
    // content blocks in order — text first, then tool_use. Anthropic relies
    // on this ordering to render the chain-of-thought.
    #[test]
    fn build_with_tool_results_text_and_tool_in_order_normal() {
        let tool = make_tool_call(
            "toolu_f",
            ToolKind::Bash,
            ToolStatus::Completed,
            ToolOutput::Text("ok".into()),
        );
        let msgs = vec![
            user_msg("hi"),
            assistant_with_parts(vec![
                MessagePart::Text("I'll run it.".into()),
                MessagePart::Tool(tool),
            ]),
        ];
        let out = build_provider_messages_with_tool_results(&msgs);
        // out[1] is the assistant turn — content[0]=text, content[1]=tool_use.
        assert_eq!(out[1].content.len(), 2);
        assert!(matches!(out[1].content[0], ProviderContent::Text(_)));
        assert!(matches!(out[1].content[1], ProviderContent::ToolUse { .. }));
    }

    // Robust: multiple tools in one assistant turn produce one tool_result
    // per tool, all in the same trailing user message — Anthropic requires
    // batched tool_result blocks to share a single message.
    #[test]
    fn build_with_tool_results_batched_results_robust() {
        let t1 = make_tool_call(
            "a",
            ToolKind::Bash,
            ToolStatus::Completed,
            ToolOutput::Text("1".into()),
        );
        let t2 = make_tool_call(
            "b",
            ToolKind::Read,
            ToolStatus::Completed,
            ToolOutput::Text("2".into()),
        );
        let msgs = vec![
            user_msg("hi"),
            assistant_with_parts(vec![MessagePart::Tool(t1), MessagePart::Tool(t2)]),
        ];
        let out = build_provider_messages_with_tool_results(&msgs);
        // Tool-result message is the last (no synthetic-user trailer needed).
        let last = out.last().unwrap();
        assert_eq!(last.role, ProviderRole::User);
        assert_eq!(last.content.len(), 2);
    }
}

#[cfg(test)]
mod truncate_more_tests {
    use super::*;

    // Robust: an exact-cap-length input passes through unchanged (no
    // truncation marker injected). Boundary value test: cap is the threshold
    // at which truncation begins.
    #[test]
    fn truncate_at_exact_cap_passes_through_robust() {
        let s: String = "x".repeat(MAX_TOOL_RESULT_CHARS);
        let out = truncate_tool_result(&s);
        assert_eq!(out, s);
        assert!(!out.contains("<truncated-output"));
    }

    // Robust: a one-byte-over-cap input gets truncated. Verifies the >
    // boundary in `if s.len() <= MAX_TOOL_RESULT_CHARS`.
    #[test]
    fn truncate_one_over_cap_does_truncate_robust() {
        let s: String = "y".repeat(MAX_TOOL_RESULT_CHARS + 1);
        let out = truncate_tool_result(&s);
        assert!(out.contains("<truncated-output"));
    }

    // ─── disk persistence (cap_tool_result / persist_tool_result) ──────

    // Normal: small bodies pass through unchanged (no persist, no
    // truncation marker).
    #[test]
    fn cap_tool_result_small_body_pass_through_normal() {
        let body = "tiny output";
        assert_eq!(cap_tool_result(body), body);
    }

    // Normal: medium bodies (over 50KB but under 400KB) hit the
    // in-memory truncation path, not disk persistence.
    #[test]
    fn cap_tool_result_medium_body_truncates_inline_normal() {
        let body: String = "x".repeat(100_000);
        let out = cap_tool_result(&body);
        assert!(
            out.contains("<truncated-output"),
            "expected inline truncation marker"
        );
        assert!(
            !out.contains("<persisted-output"),
            "should not persist below 400KB threshold"
        );
    }

    // Normal: bodies above 400KB get spilled to disk and the
    // returned string is the v131-style <persisted-output>
    // reference. Verify the file was actually written and contains
    // the original content.
    #[test]
    fn cap_tool_result_large_body_persists_to_disk_normal() {
        let body: String = "y".repeat(TOOL_RESULT_DISK_PERSIST_BYTES + 100);
        let out = cap_tool_result(&body);
        assert!(
            out.contains("<persisted-output"),
            "expected persisted-output reference: {}",
            &out[..200.min(out.len())]
        );
        assert!(out.contains("path=\""));
        assert!(out.contains(&format!("original_bytes=\"{}\"", body.len())));
        // Extract the path from the reference and verify the file
        // was actually written with the full body.
        let path_start = out.find("path=\"").map(|p| p + "path=\"".len()).unwrap();
        let path_end = out[path_start..].find('"').map(|p| path_start + p).unwrap();
        let path = &out[path_start..path_end];
        let on_disk = std::fs::read_to_string(path).expect("spilled file should exist");
        assert_eq!(on_disk.len(), body.len());
        // Cleanup so the temp dir doesn't accumulate test artifacts.
        let _ = std::fs::remove_file(path);
    }

    // Robust: the persisted-output reference includes a head
    // preview so the model gets some context without `Read`-ing the
    // spill file. v131 uses 2000 chars; we mirror.
    #[test]
    fn cap_tool_result_persisted_includes_preview_robust() {
        let head = "HEADMARKER";
        let body = format!("{head}{}", "z".repeat(TOOL_RESULT_DISK_PERSIST_BYTES));
        let out = cap_tool_result(&body);
        assert!(
            out.contains(head),
            "preview missing head marker: {}",
            &out[..200.min(out.len())]
        );
        // Cleanup
        if let Some(s) = out.find("path=\"") {
            let s = s + "path=\"".len();
            if let Some(e) = out[s..].find('"') {
                let _ = std::fs::remove_file(&out[s..s + e]);
            }
        }
    }

    // Normal: floor_char_boundary at byte 0 returns 0; at the end returns len.
    #[test]
    fn floor_char_boundary_endpoints_normal() {
        let s = "hello";
        assert_eq!(floor_char_boundary(s, 0), 0);
        assert_eq!(floor_char_boundary(s, s.len()), s.len());
        assert_eq!(floor_char_boundary(s, 100), s.len());
    }

    // Normal: ceil_char_boundary at byte 0 returns 0; at the end returns len.
    #[test]
    fn ceil_char_boundary_endpoints_normal() {
        let s = "hello";
        assert_eq!(ceil_char_boundary(s, 0), 0);
        assert_eq!(ceil_char_boundary(s, s.len()), s.len());
        assert_eq!(ceil_char_boundary(s, 100), s.len());
    }
}

#[cfg(test)]
mod should_continue_loop_tests {
    use super::*;

    fn assistant_with_tool(status: ToolStatus) -> ChatMessage {
        ChatMessage::assistant_parts(vec![MessagePart::Tool(ToolCall {
            id: "toolu_x".into(),
            kind: ToolKind::Bash,
            status,
            input: ToolInput::Generic {
                summary: "x".into(),
            },
            output: ToolOutput::Empty,
            display: crate::types::ToolDisplayState::DEFAULT,
            elapsed_ms: None,
            started_at: None,
        })])
    }

    // Normal: when the last assistant has all tools Complete, the loop
    // continues so the agent gets a chance to react to the tool results.
    #[test]
    fn continues_when_all_tools_complete_normal() {
        let msgs = vec![assistant_with_tool(ToolStatus::Completed)];
        assert!(should_continue_loop(&msgs));
    }

    // Normal: a Failed tool also signals "go again" — the agent might try
    // a different approach.
    #[test]
    fn continues_when_tool_failed_normal() {
        let msgs = vec![assistant_with_tool(ToolStatus::Failed)];
        assert!(should_continue_loop(&msgs));
    }

    // Robust: a Pending tool means the user hasn't approved yet, so the loop
    // does NOT continue (we'd send a half-assembled state to the model).
    #[test]
    fn does_not_continue_when_tool_pending_robust() {
        let msgs = vec![assistant_with_tool(ToolStatus::Pending)];
        assert!(!should_continue_loop(&msgs));
    }

    // Robust: a Running tool is still in flight — the loop must wait.
    #[test]
    fn does_not_continue_when_tool_running_robust() {
        let msgs = vec![assistant_with_tool(ToolStatus::Running)];
        assert!(!should_continue_loop(&msgs));
    }

    // Normal: assistant turn with no tools (pure prose) → loop terminates.
    // The agent finished its turn cleanly.
    #[test]
    fn does_not_continue_for_text_only_assistant_normal() {
        let msgs = vec![ChatMessage::assistant("done".into())];
        assert!(!should_continue_loop(&msgs));
    }

    // Robust: empty conversation → no continuation. Used when the session is
    // freshly resumed and there's nothing to react to.
    #[test]
    fn does_not_continue_when_no_assistant_robust() {
        let msgs = vec![ChatMessage::user("hi".into())];
        assert!(!should_continue_loop(&msgs));
    }

    // Robust: completely empty message list — defensive check for the
    // resume-from-disk code path.
    #[test]
    fn does_not_continue_on_empty_messages_robust() {
        let msgs: Vec<ChatMessage> = vec![];
        assert!(!should_continue_loop(&msgs));
    }
}

#[cfg(test)]
mod cancellation_token_tests {
    //! Regression tests for the wg-async cancellation pattern.
    //!
    //! Background: spawn sites in `stream.rs` and `event_loop.rs` used
    //! to take an `Arc<AtomicBool>` that the spawned task polled between
    //! iterations. A blocking provider call mid-tick could miss the flag
    //! for seconds. We migrated the long-running spawn sites to also
    //! receive a `tokio_util::sync::CancellationToken` so they can race
    //! their work against `.cancelled()` via `tokio::select!`. These
    //! tests pin that contract: cancelling the token must unwind the
    //! spawned task within a single tokio scheduler tick.
    use super::*;
    use tokio_util::sync::CancellationToken;

    /// Normal: a task that races a long sleep against `.cancelled()`
    /// returns immediately when the token is cancelled, instead of
    /// waiting for the sleep to finish. This is the core latency win
    /// over the AtomicBool-poll pattern.
    #[tokio::test]
    async fn cancel_during_spawn_unwinds_within_one_tick_normal() {
        let cancel = CancellationToken::new();
        let cancel_for_task = cancel.clone();

        // Spawn a task that mirrors the migrated stream_response select!
        // shape: a long fake "stream read" raced against cancellation.
        let handle = tokio::spawn(async move {
            tokio::select! {
                biased;
                _ = cancel_for_task.cancelled() => "cancelled",
                _ = tokio::time::sleep(std::time::Duration::from_secs(60)) => "completed",
            }
        });

        // Cancel after a microsleep so the task has actually started
        // polling its select! arms before the signal lands.
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        cancel.cancel();

        // The whole join must finish well under the 60-second sleep —
        // give it 500ms of headroom for slow CI runners.
        let outcome = tokio::time::timeout(
            std::time::Duration::from_millis(500),
            handle,
        )
        .await
        .expect("spawn must unwind within 500ms after cancel()")
        .expect("spawned task must not panic");

        assert_eq!(outcome, "cancelled");
    }

    /// Robust: cancelling the token BEFORE the task gets to its first
    /// poll still unwinds it — `cancelled()` returns immediately when
    /// the token is already in the cancelled state. Without this, a
    /// task spawned between the user's ESC×2 and the runtime actually
    /// scheduling it could miss the cancel and run to completion.
    #[tokio::test]
    async fn cancel_before_task_starts_still_short_circuits_robust() {
        let cancel = CancellationToken::new();
        // Cancel BEFORE the spawn so the cloned token enters the task
        // already in the cancelled state.
        cancel.cancel();
        let cancel_for_task = cancel.clone();

        let handle = tokio::spawn(async move {
            tokio::select! {
                biased;
                _ = cancel_for_task.cancelled() => "cancelled",
                _ = tokio::time::sleep(std::time::Duration::from_secs(60)) => "completed",
            }
        });

        let outcome = tokio::time::timeout(
            std::time::Duration::from_millis(500),
            handle,
        )
        .await
        .expect("pre-cancelled token must short-circuit the spawn")
        .expect("spawned task must not panic");

        assert_eq!(outcome, "cancelled");
    }

    /// Robust: a fresh token is not poisoned by a previously-cancelled
    /// sibling. The migration mints a new token on every user submit;
    /// if that mint were a no-op the next turn would be DOA. This pins
    /// `CancellationToken::new()` semantics for the post-interrupt
    /// new-turn path.
    #[tokio::test]
    async fn fresh_token_is_not_cancelled_robust() {
        let prior = CancellationToken::new();
        prior.cancel();
        assert!(prior.is_cancelled());

        // `App::handle_submit_text` and the StreamError handler both do
        // `app.cancel_token = CancellationToken::new();` after a cancel.
        let fresh = CancellationToken::new();
        assert!(!fresh.is_cancelled());

        // And cloning the fresh token doesn't observe the prior one's
        // cancelled state — they're independent.
        let fresh_clone = fresh.clone();
        assert!(!fresh_clone.is_cancelled());
    }
}

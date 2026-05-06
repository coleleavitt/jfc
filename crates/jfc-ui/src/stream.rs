use std::{collections::HashMap, sync::Arc};

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

/// Truncate `s` to at most `MAX_TOOL_RESULT_CHARS` bytes by keeping the first
/// half and the last half, with an ellipsis marker in the middle. Slice
/// boundaries are snapped to the nearest UTF-8 char boundary so the function
/// can never panic on multi-byte content (emoji, accented chars, or binary
/// blobs that happen to land in the slice — exactly the panic in the
/// screenshot's stack trace at stream.rs:334:14, fired from inside
/// build_provider_messages_with_tool_results' FilterMap closure).
pub(crate) fn truncate_tool_result(s: &str) -> String {
    if s.len() <= MAX_TOOL_RESULT_CHARS {
        return s.to_owned();
    }
    let half = MAX_TOOL_RESULT_CHARS / 2;
    let head_end = floor_char_boundary(s, half);
    let tail_start = ceil_char_boundary(s, s.len().saturating_sub(half));
    let head = &s[..head_end];
    let tail = &s[tail_start..];
    let omitted = s.len() - head_end - (s.len() - tail_start);
    format!("{head}\n\n... [{omitted} bytes omitted] ...\n\n{tail}")
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

    // Normal: head and tail are preserved across truncation.
    #[test]
    fn truncate_keeps_head_and_tail_normal() {
        let mid: String = "x".repeat(MAX_TOOL_RESULT_CHARS * 2);
        let s = format!("HEAD{mid}TAIL");
        let out = truncate_tool_result(&s);
        assert!(out.starts_with("HEAD"));
        assert!(out.ends_with("TAIL"));
        assert!(out.contains("bytes omitted"));
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
        assert!(!has_orphan_tool_result, "tool_result left without its tool_use");
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
    tx: mpsc::UnboundedSender<AppEvent>,
    interrupt: std::sync::Arc<std::sync::atomic::AtomicBool>,
) {
    let cwd = std::env::current_dir()
        .ok()
        .and_then(|p| p.to_str().map(str::to_owned))
        .unwrap_or_default();
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
         use it consistently on all non-trivial work."
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

        // v126 skills listing — discovery surface for the model. Loaded on
        // every stream call so newly-added skills (or edited descriptions)
        // take effect on the next turn, matching cli.js:151-160's per-stream
        // re-read pattern.
        let skills = crate::agents::load_skills(&cwd_path);
        let block = crate::agents::render_skills_section(&skills);
        if !block.is_empty() {
            system_prompt.push_str(&block);
        }

        // Memory system — load persistent memories from both user-level
        // (~/.config/jfc/memory/) and project-level (.jfc/memory/) and
        // inject them into the system prompt. Re-loaded on every stream
        // call so newly-saved memories take effect on the next turn.
        let memories = crate::memory::load_all_memories(&cwd_path);
        if let Some(memories_section) = crate::memory::render_memories_section(&memories) {
            system_prompt.push_str(&memories_section);
        }

        // Inject the most recent diagnostics snapshot so the model can
        // act on cargo errors / lints without the user pasting them in.
        // Read from the global the `DiagnosticsUpdated` handler keeps in
        // sync — passing the slice through every call site would have
        // forced a wide signature change.
        let diags = crate::diagnostics::global_snapshot();
        if let Some(block) = crate::diagnostics::render_for_prompt(&diags) {
            system_prompt.push_str(&block);
        }
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
    let opts = {
        let base = StreamOptions::new(model.clone())
            .system(system_prompt)
            .tools(tools::all_tool_defs())
            .max_tokens(max_out);
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
                        let _ = tx.send(AppEvent::StreamError(e2.to_string()));
                        return;
                    }
                }
            } else {
                tracing::error!(target: "jfc::stream", error = %e, "stream open failed");
                let _ = tx.send(AppEvent::StreamError(e.to_string()));
                return;
            }
        }
    };

    let mut stop_reason = StopReason::EndTurn;
    let mut tool_accum: HashMap<usize, (String, String, String)> = HashMap::new();

    while let Some(event) = stream.next().await {
        // Cooperative cancel: the user pressed ESC twice — drop the
        // stream mid-flight, surface a clean stop, and let the main
        // loop reset state. Doing this *between* SSE events keeps the
        // partial output the model sent so the user sees what made it
        // through.
        if interrupt.load(std::sync::atomic::Ordering::SeqCst) {
            tracing::info!(target: "jfc::stream", "stream interrupted by user (ESC×2)");
            let _ = tx.send(AppEvent::StreamError(
                "Interrupted by user".to_owned(),
            ));
            return;
        }
        let event = match event {
            Ok(e) => e,
            Err(e) => {
                tracing::error!(target: "jfc::stream", error = %e, "stream event error");
                let _ = tx.send(AppEvent::StreamError(e.to_string()));
                return;
            }
        };

        match event {
            StreamEvent::TextDelta { delta, .. } => {
                let _ = tx.send(AppEvent::StreamChunk {
                    text: Some(delta),
                    reasoning: None,
                });
            }
            StreamEvent::ThinkingDelta { delta, .. } => {
                let _ = tx.send(AppEvent::StreamChunk {
                    text: None,
                    reasoning: Some(delta),
                });
            }
            StreamEvent::ToolDelta { index, delta } => {
                let byte_len = delta.len();
                tool_accum.entry(index).or_default().2.push_str(&delta);
                // Notify the main loop so:
                // 1. streaming_response_bytes increments (spinner token estimate stays live)
                // 2. streaming_last_token_at resets (stall timer doesn't fire)
                // Matches v126's responseLengthRef accumulation from input_json_delta events.
                let _ = tx.send(AppEvent::ToolInputDelta(byte_len));
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
                let input_val: serde_json::Value =
                    serde_json::from_str(&assembled).unwrap_or(serde_json::Value::Null);
                let tool = ToolCall {
                    id: tool_use_id,
                    kind: ToolKind::from_name(&tool_name),
                    status: ToolStatus::Pending,
                    input: ToolInput::from_value(&tool_name, input_val),
                    output: ToolOutput::Empty,
                    is_collapsed: false,
                    expanded: false,
                    elapsed_ms: None,
                    started_at: Some(std::time::Instant::now()),
                    pinned: false,
                };
                tool_accum.remove(&index);
                let _ = tx.send(AppEvent::StreamTool(tool));
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
                let _ = tx.send(AppEvent::StreamUsage {
                    input_tokens,
                    output_tokens,
                    cache_read_tokens,
                    cache_write_tokens,
                });
            }
            StreamEvent::Error { message } => {
                tracing::error!(target: "jfc::stream", %message, "stream error event");
                let _ = tx.send(AppEvent::StreamError(message));
                return;
            }
        }
    }

    tracing::info!(
        target: "jfc::stream",
        ?stop_reason,
        "stream finished — sending StreamDone"
    );
    let _ = tx.send(AppEvent::StreamDone(stop_reason));
}

#[tracing::instrument(target = "jfc::stream", skip(tx, dedup, task_store, provider, model, teammate_event_tx), fields(n = tool_calls.len()))]
pub fn dispatch_tools_batched(
    tool_calls: Vec<ToolCall>,
    tx: &mpsc::UnboundedSender<AppEvent>,
    dedup: Arc<Mutex<ReadDedupCache>>,
    task_store: Option<Arc<crate::tasks::TaskStore>>,
    provider: Arc<dyn crate::provider::Provider>,
    model: crate::provider::ModelId,
    teammate_event_tx: mpsc::UnboundedSender<crate::swarm::runner::TeammateEvent>,
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
            let _ = tx_done.send(AppEvent::AllToolsComplete);
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
            let task_id = tc.id.clone();
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
                    if let Err(e) =
                        crate::swarm::team_helpers::add_member(&team_name, member).await
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
            let _ = tx_task.send(AppEvent::TeammateSpawned {
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
            let _ = tx_task.send(AppEvent::TaskStarted {
                task_id: runner_task_id.clone(),
                description: format!("spawn teammate: {name}"),
            });

            let _ = tx_task.send(AppEvent::ToolResult {
                tool_id: task_id,
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
        let task_id = tc.id.clone();
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
            .and_then(|t| {
                agents
                    .iter()
                    .find(|a| a.name.eq_ignore_ascii_case(t))
            })
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
                let name = format!("agent-{}", task_id.replace("toolu_", "").chars().take(8).collect::<String>());
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
            let _ = tx_task.send(AppEvent::TaskStarted {
                task_id: task_id.clone(),
                description,
            });

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
                let _ = tx_task.send(AppEvent::TaskFailed {
                    task_id: task_id.clone(),
                    error: result.output.clone(),
                });
            } else {
                tracing::info!(
                    target: "jfc::stream",
                    task_id = %task_id,
                    elapsed_ms,
                    output_len = result.output.len(),
                    "task tool: execute_task succeeded"
                );
                let _ = tx_task.send(AppEvent::TaskCompleted {
                    task_id: task_id.clone(),
                    summary: result.output.clone(),
                    elapsed_ms,
                });
            }

            // Decide the worktree's fate BEFORE sending the ToolResult so the
            // user-visible message can mention the preserved branch
            // when there are uncommitted changes. Mirrors the Claude
            // Code Agent docs: "the worktree is automatically cleaned
            // up if the agent makes no changes; otherwise the path and
            // branch are returned in the result." `git status
            // --porcelain` is the standard "is the working tree clean"
            // signal — quiet, scriptable, exit-code aware.
            let worktree_outcome: Option<(crate::worktrees::WorktreeInfo, bool)> =
                if let Some(wt) = worktree_info {
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

            let _ = tx_task.send(AppEvent::ToolResult {
                tool_id: task_id,
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
            });

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
        tokio::spawn(async move {
            scheduler::execute_batches(batches, &tx_clone, cwd, dedup, task_store, None).await;
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
            tc.status == ToolStatus::Complete || tc.status == ToolStatus::Failed
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

pub async fn continue_agentic_loop(app: &mut App, tx: &mpsc::UnboundedSender<AppEvent>) {
    let assistant_idx = app.messages.len();
    tracing::info!(
        target: "jfc::stream",
        assistant_idx,
        model = %app.model,
        total_messages = app.messages.len(),
        "continue_agentic_loop: starting new sub-stream"
    );
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

    tokio::spawn(async move {
        stream_response(provider, messages, model, tx, interrupt).await;
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
                        id: tc.id.clone(),
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
                    let (result_text, is_error) = match tc.status {
                        ToolStatus::Complete | ToolStatus::Failed => {
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
                        tool_use_id: tc.id.clone(),
                        content: truncate_tool_result(&result_text),
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
    tracing::debug!(
        target: "jfc::stream",
        input_messages = msgs.len(),
        output_messages = out.len(),
        tool_use_count, tool_result_count, abandoned_count,
        "build_provider_messages_with_tool_results"
    );
    ensure_user_last(out)
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
        let input = vec![
            user_text("hi"),
            assistant_text(""),
            assistant_text(""),
        ];
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

    fn make_tool_call(id: &str, kind: ToolKind, status: ToolStatus, output: ToolOutput) -> ToolCall {
        ToolCall {
            id: id.to_owned(),
            kind,
            status,
            input: ToolInput::Generic { summary: "x".into() },
            output,
            is_collapsed: false,
            expanded: false,
            elapsed_ms: None,
            started_at: None,
            pinned: false,
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
            ToolStatus::Complete,
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
            ProviderContent::ToolResult { tool_use_id, content, is_error } => {
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
            ProviderContent::ToolResult { content, is_error, .. } => {
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
            ToolStatus::Complete,
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
            ToolStatus::Complete,
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
            ToolStatus::Complete,
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
            ToolStatus::Complete,
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
        let t1 = make_tool_call("a", ToolKind::Bash, ToolStatus::Complete, ToolOutput::Text("1".into()));
        let t2 = make_tool_call("b", ToolKind::Read, ToolStatus::Complete, ToolOutput::Text("2".into()));
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
        assert!(!out.contains("bytes omitted"));
    }

    // Robust: a one-byte-over-cap input gets truncated. Verifies the >
    // boundary in `if s.len() <= MAX_TOOL_RESULT_CHARS`.
    #[test]
    fn truncate_one_over_cap_does_truncate_robust() {
        let s: String = "y".repeat(MAX_TOOL_RESULT_CHARS + 1);
        let out = truncate_tool_result(&s);
        assert!(out.contains("bytes omitted"));
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
            input: ToolInput::Generic { summary: "x".into() },
            output: ToolOutput::Empty,
            is_collapsed: false,
            expanded: false,
            elapsed_ms: None,
            started_at: None,
            pinned: false,
        })])
    }

    // Normal: when the last assistant has all tools Complete, the loop
    // continues so the agent gets a chance to react to the tool results.
    #[test]
    fn continues_when_all_tools_complete_normal() {
        let msgs = vec![assistant_with_tool(ToolStatus::Complete)];
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

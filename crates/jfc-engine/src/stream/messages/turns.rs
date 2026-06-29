use crate::types::{ChatMessage, MessagePart, Role};
use jfc_provider::{ProviderContent, ProviderMessage, ProviderRole};
use std::collections::HashSet;

const INTERRUPTED_TOOL_RESULT: &str =
    "Tool was interrupted before it could run. No output was produced.";
const ORPHANED_TOOL_RESULT_REMOVED: &str =
    "[Orphaned tool result removed due to conversation resume]";
const TOOL_USE_INTERRUPTED_TEXT: &str = "[Tool use interrupted]";

pub fn provider_role(role: Role) -> ProviderRole {
    match role {
        Role::User => ProviderRole::User,
        Role::Assistant => ProviderRole::Assistant,
    }
}

pub fn chat_message_text(m: &ChatMessage) -> String {
    m.parts
        .iter()
        .filter_map(|p| match p {
            MessagePart::Text(t) if !t.is_empty() => Some(t.to_owned()),
            // TaskStatus is UI/session state. Detached worker completions are
            // delivered to the model through a one-shot system reminder, not
            // replayed from every historical assistant turn.
            MessagePart::TaskStatus(_) => None,
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn turns_ago_by_message(msgs: &[ChatMessage]) -> Vec<usize> {
    let mut turns_ago_map: Vec<usize> = vec![0; msgs.len()];
    let mut user_turns_seen = 0usize;
    for i in (0..msgs.len()).rev() {
        if msgs[i].role == Role::User && !msgs[i].queued {
            user_turns_seen += 1;
        }
        turns_ago_map[i] = user_turns_seen;
    }
    turns_ago_map
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
pub fn ensure_user_last(msgs: Vec<ProviderMessage>) -> Vec<ProviderMessage> {
    let msgs = strip_trailing_empty_assistants(msgs);
    let msgs = append_synthetic_user_if_trailing_assistant(msgs);
    merge_consecutive_same_role(msgs)
}

/// Resume-mode message preparation for `pause_turn` continuations.
///
/// Anthropic's `pause_turn` resume protocol (cli.js v142:622686, :623776):
///
/// > To continue, re-send the user message and assistant response and make
/// > another API request — the server will resume where it left off.
/// > **Do NOT add an extra user message like "Continue."** — the API detects
/// > the trailing `server_tool_use` block and knows to resume automatically.
///
/// We still strip empty trailing assistant placeholders (those are
/// `continue_agentic_loop` staging artifacts, not model output), and we
/// still merge consecutive same-role messages, but we deliberately skip
/// the synthetic-user injection that `ensure_user_last` does. The trailing
/// assistant with its `server_tool_use` block IS the resume signal.
///
/// Used only by `build_provider_messages_for_pause_turn_resume`.
pub fn prepare_for_pause_turn_resume(msgs: Vec<ProviderMessage>) -> Vec<ProviderMessage> {
    let msgs = strip_trailing_empty_assistants(msgs);
    // Note: NO synthetic-user step here. The trailing assistant is the
    // resume cue per Anthropic spec.
    merge_consecutive_same_role(msgs)
}

/// Upstream-style pre-send repair for `tool_use` / `tool_result` pairing.
///
/// Claude Code runs an `ensureToolResultPairing` pass immediately before an API
/// request. It repairs resumptions and interrupted turns that would otherwise
/// 400:
///
/// * orphaned `tool_result` user blocks are stripped;
/// * duplicate local `tool_use` IDs are dropped;
/// * local `tool_use` blocks missing a next-user `tool_result` get a synthetic
///   error result;
/// * duplicate/extra `tool_result` blocks are removed;
/// * unpaired server-side tool uses are stripped from normal sends.
///
/// Pause-turn resume deliberately skips this pass: the trailing
/// `server_tool_use` without a result is the resume signal for Anthropic's
/// server-side sampling loop.
pub fn repair_tool_result_pairing(msgs: Vec<ProviderMessage>) -> Vec<ProviderMessage> {
    let original_len = msgs.len();
    // Forensic snapshot of the pre-repair message structure. Cheap to build
    // (one pass over block kinds) and only formatted into a string when a
    // repair or strict-mode abort actually fires.
    let pre_structure = describe_message_structure(&msgs);
    let mut out = Vec::with_capacity(original_len);
    let mut repaired = false;
    let mut seen_tool_use_ids: HashSet<String> = HashSet::new();
    let mut idx = 0usize;

    while idx < msgs.len() {
        let msg = msgs[idx].clone();
        if msg.role != ProviderRole::Assistant {
            if msg.role == ProviderRole::User
                && contains_tool_result(&msg)
                && !out
                    .last()
                    .is_some_and(|prev: &ProviderMessage| prev.role == ProviderRole::Assistant)
            {
                let filtered: Vec<_> = msg
                    .content
                    .into_iter()
                    .filter(|content| !matches!(content, ProviderContent::ToolResult { .. }))
                    .collect();
                if filtered.is_empty() {
                    if out.is_empty() {
                        out.push(ProviderMessage {
                            role: ProviderRole::User,
                            content: vec![ProviderContent::Text(
                                ORPHANED_TOOL_RESULT_REMOVED.to_owned(),
                            )],
                        });
                    }
                } else {
                    out.push(ProviderMessage {
                        role: ProviderRole::User,
                        content: filtered,
                    });
                }
                repaired = true;
                idx += 1;
                continue;
            }
            out.push(msg);
            idx += 1;
            continue;
        }

        let same_message_server_results: HashSet<String> = msg
            .content
            .iter()
            .filter_map(|content| match content {
                ProviderContent::ServerToolResult { tool_use_id, .. } => Some(tool_use_id.clone()),
                _ => None,
            })
            .collect();
        let mut local_tool_use_ids = Vec::new();
        let mut assistant_content = Vec::with_capacity(msg.content.len());
        for content in msg.content {
            match &content {
                ProviderContent::ToolUse { id, .. } => {
                    if seen_tool_use_ids.contains(id) {
                        repaired = true;
                        continue;
                    }
                    seen_tool_use_ids.insert(id.clone());
                    local_tool_use_ids.push(id.clone());
                    assistant_content.push(content);
                }
                ProviderContent::ServerToolUse { id, .. } => {
                    if same_message_server_results.contains(id) {
                        assistant_content.push(content);
                    } else {
                        repaired = true;
                    }
                }
                _ => assistant_content.push(content),
            }
        }
        if assistant_content.is_empty() {
            assistant_content.push(ProviderContent::Text(TOOL_USE_INTERRUPTED_TEXT.to_owned()));
        }
        out.push(ProviderMessage {
            role: ProviderRole::Assistant,
            content: assistant_content,
        });

        if local_tool_use_ids.is_empty() {
            idx += 1;
            continue;
        }

        let next_user = msgs.get(idx + 1).filter(|m| m.role == ProviderRole::User);
        let next_result_ids = next_user
            .map(tool_result_ids_with_duplicate_flag)
            .unwrap_or_default();
        let local_id_set: HashSet<_> = local_tool_use_ids.iter().cloned().collect();
        let missing_ids: Vec<String> = local_tool_use_ids
            .iter()
            .filter(|id| !next_result_ids.ids.contains(*id))
            .cloned()
            .collect();
        let extra_ids: HashSet<String> = next_result_ids
            .ids
            .iter()
            .filter(|id| !local_id_set.contains(*id))
            .cloned()
            .collect();

        if missing_ids.is_empty() && extra_ids.is_empty() && !next_result_ids.has_duplicate {
            idx += 1;
            continue;
        }
        repaired = true;

        let synthetic_results =
            missing_ids
                .into_iter()
                .map(|tool_use_id| ProviderContent::ToolResult {
                    tool_use_id,
                    content: INTERRUPTED_TOOL_RESULT.to_owned(),
                    is_error: true,
                });

        if let Some(next) = next_user {
            let mut seen_results = HashSet::new();
            let filtered_next = next.content.iter().filter_map(|content| match content {
                ProviderContent::ToolResult { tool_use_id, .. } => {
                    if extra_ids.contains(tool_use_id) || !seen_results.insert(tool_use_id.clone())
                    {
                        None
                    } else {
                        Some(content.clone())
                    }
                }
                _ => Some(content.clone()),
            });
            let content: Vec<_> = synthetic_results.chain(filtered_next).collect();
            if content.is_empty() {
                out.push(ProviderMessage {
                    role: ProviderRole::User,
                    content: vec![ProviderContent::Text(INTERRUPTED_TOOL_RESULT.to_owned())],
                });
            } else {
                out.push(ProviderMessage {
                    role: ProviderRole::User,
                    content,
                });
            }
            idx += 2;
        } else {
            out.push(ProviderMessage {
                role: ProviderRole::User,
                content: synthetic_results.collect(),
            });
            idx += 1;
        }
    }

    let (out, reminder_count) = fold_system_reminders_into_tool_results(out);
    if reminder_count > 0 {
        tracing::warn!(
            target: "jfc::stream::invariants",
            reminder_count,
            "moved top-level system-reminder text into adjacent tool_result content before send"
        );
    }

    if repaired {
        let post_structure = describe_message_structure(&out);
        if strict_tool_result_pairing() {
            // Claude 2.1.177 strict mode (inc-4977): refuse to silently inject
            // synthetic placeholders into the model's context. Repairing would
            // mask an upstream bug, so surface it loudly. We log rather than
            // panic — the send path has no Result here and a panic would kill
            // the turn — but the message mirrors upstream's strict-mode error.
            tracing::error!(
                target: "jfc::stream::invariants",
                strict = true,
                original_message_count = original_len,
                repaired_message_count = out.len(),
                pre_normalized_sequence = %pre_structure,
                normalized_sequence = %post_structure,
                "STRICT tool_use/tool_result pairing mismatch detected — repair would inject synthetic placeholders into model context (see inc-4977); set JFC_STRICT_TOOL_RESULT_PAIRING=0 to silence"
            );
        }
        tracing::error!(
            target: "jfc::stream::invariants",
            original_message_count = original_len,
            repaired_message_count = out.len(),
            pre_normalized_sequence = %pre_structure,
            normalized_sequence = %post_structure,
            "tengu_tool_result_pairing_repaired: repaired provider message history before send"
        );
    }
    out
}

/// Strict tool-result pairing mode. When enabled, a detected pairing mismatch
/// is logged as a hard strict-mode violation in addition to the normal repair.
///
/// Mirrors Claude 2.1.177's `strictToolResultPairing` flag (env-gated). Default
/// off so production behavior is unchanged; set `JFC_STRICT_TOOL_RESULT_PAIRING`
/// to a truthy value to opt in (CI / debugging contexts).
pub fn strict_tool_result_pairing() -> bool {
    std::env::var("JFC_STRICT_TOOL_RESULT_PAIRING")
        .ok()
        .map(|v| {
            matches!(
                v.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
        .unwrap_or(false)
}

/// Build a compact, forensic description of a provider message sequence —
/// per-message role plus the kind of each content block (and tool ids for
/// `tool_use` / `tool_result`). Mirrors the `messageTypes` / `normalizedSequence`
/// strings Claude 2.1.177 emits with its pairing telemetry so a 400 can be
/// debugged from logs without reconstructing the request body.
/// Count how many times a tool id appears as a `tool_use` (or server variant)
/// vs a `tool_result` across the entire message sequence. Returns
/// `(tool_use_occurrences, tool_result_occurrences)`. Used by the pairing
/// mismatch forensics to distinguish a duplicate from a true orphan.
fn count_tool_id_occurrences(msgs: &[ProviderMessage], id: &str) -> (usize, usize) {
    let mut uses = 0usize;
    let mut results = 0usize;
    for msg in msgs {
        for content in &msg.content {
            match content {
                ProviderContent::ToolUse { id: cid, .. }
                | ProviderContent::ServerToolUse { id: cid, .. }
                    if cid == id =>
                {
                    uses += 1;
                }
                ProviderContent::ToolResult { tool_use_id, .. }
                | ProviderContent::ServerToolResult { tool_use_id, .. }
                    if tool_use_id == id =>
                {
                    results += 1;
                }
                _ => {}
            }
        }
    }
    (uses, results)
}

fn describe_message_structure(msgs: &[ProviderMessage]) -> String {
    msgs.iter()
        .enumerate()
        .map(|(i, msg)| {
            let role = match msg.role {
                ProviderRole::User => "user",
                ProviderRole::Assistant => "assistant",
            };
            let blocks = msg
                .content
                .iter()
                .map(|c| match c {
                    ProviderContent::ToolUse { id, .. } => format!("tool_use:{id}"),
                    ProviderContent::ToolResult { tool_use_id, .. } => {
                        format!("tool_result:{tool_use_id}")
                    }
                    ProviderContent::ServerToolUse { id, .. } => format!("server_tool_use:{id}"),
                    ProviderContent::ServerToolResult { tool_use_id, .. } => {
                        format!("server_tool_result:{tool_use_id}")
                    }
                    ProviderContent::Text(_) => "text".to_owned(),
                    ProviderContent::Thinking { .. } => "thinking".to_owned(),
                    ProviderContent::Attachment(_) => "attachment".to_owned(),
                    ProviderContent::RedactedThinking { .. } => "redacted_thinking".to_owned(),
                })
                .collect::<Vec<_>>()
                .join(",");
            format!("[{i}] {role}([{blocks}])")
        })
        .collect::<Vec<_>>()
        .join("; ")
}

fn fold_system_reminders_into_tool_results(
    msgs: Vec<ProviderMessage>,
) -> (Vec<ProviderMessage>, usize) {
    let mut moved = 0usize;
    let msgs = msgs
        .into_iter()
        .map(|msg| {
            if msg.role != ProviderRole::User || !contains_tool_result(&msg) {
                return msg;
            }

            let mut reminders = Vec::new();
            let mut content = Vec::with_capacity(msg.content.len());
            for block in msg.content {
                match block {
                    ProviderContent::Text(text) if is_system_reminder_text(&text) => {
                        moved += 1;
                        reminders.push(text);
                    }
                    other => content.push(other),
                }
            }
            if reminders.is_empty() {
                return ProviderMessage {
                    role: ProviderRole::User,
                    content,
                };
            }

            if let Some(ProviderContent::ToolResult {
                content: tool_content,
                ..
            }) = content
                .iter_mut()
                .rev()
                .find(|block| matches!(block, ProviderContent::ToolResult { .. }))
            {
                for reminder in reminders {
                    let reminder = reminder.trim();
                    if reminder.is_empty() {
                        continue;
                    }
                    if !tool_content.trim().is_empty() {
                        tool_content.push_str("\n\n");
                    }
                    tool_content.push_str(reminder);
                }
            }

            ProviderMessage {
                role: ProviderRole::User,
                content,
            }
        })
        .collect();
    (msgs, moved)
}

fn is_system_reminder_text(text: &str) -> bool {
    text.trim_start().starts_with("<system-reminder>")
}

#[derive(Default)]
struct ToolResultIds {
    ids: HashSet<String>,
    has_duplicate: bool,
}

fn tool_result_ids_with_duplicate_flag(msg: &ProviderMessage) -> ToolResultIds {
    let mut out = ToolResultIds::default();
    for content in &msg.content {
        if let ProviderContent::ToolResult { tool_use_id, .. } = content
            && !out.ids.insert(tool_use_id.clone())
        {
            out.has_duplicate = true;
        }
    }
    out
}

fn strip_trailing_empty_assistants(mut msgs: Vec<ProviderMessage>) -> Vec<ProviderMessage> {
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
    msgs
}

fn append_synthetic_user_if_trailing_assistant(
    mut msgs: Vec<ProviderMessage>,
) -> Vec<ProviderMessage> {
    // If the conversation still ends with an assistant (real content),
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
    msgs
}

fn merge_consecutive_same_role(msgs: Vec<ProviderMessage>) -> Vec<ProviderMessage> {
    // Merge consecutive same-role messages. The Anthropic API requires
    // strictly alternating user/assistant turns. Consecutive same-role
    // messages happen when: (a) a compact_boundary (assistant) is followed
    // by a text-only assistant, (b) queued prompts produce adjacent user
    // messages, (c) filtering removes messages and collapses the alternation.
    //
    // CRITICAL: do NOT merge across a `tool_result` boundary. Anthropic's
    // Messages API validates that any user message containing a
    // `tool_result` block contains ONLY tool_result blocks (and that the
    // immediately-preceding assistant message contained matching
    // `tool_use` IDs). Merging a tool_result user message with a normal
    // text user message produces a mixed-content user turn that either
    // gets rejected with `invalid_request_error` or — worse on lenient
    // gateways — teaches the model that tool results are interchangeable
    // with chat text. Same rule for the inverse direction.
    let mut merged: Vec<ProviderMessage> = Vec::with_capacity(msgs.len());
    for msg in msgs {
        if let Some(last) = merged.last_mut()
            && last.role == msg.role
            && !contains_tool_result(last)
            && !contains_tool_result(&msg)
        {
            last.content.extend(msg.content);
            continue;
        }
        merged.push(msg);
    }
    merged
}

/// True iff the message carries any `ProviderContent::ToolResult` block.
/// Used by `ensure_user_last` to enforce Anthropic's tool_result purity
/// rule: a user message that contains tool_result MUST contain only
/// tool_result, and must not be merged with adjacent normal-text user
/// messages.
pub fn contains_tool_result(msg: &ProviderMessage) -> bool {
    msg.content
        .iter()
        .any(|c| matches!(c, ProviderContent::ToolResult { .. }))
}

/// Pre-send validator for the post-merge provider message stream.
///
/// Anthropic's Messages API enforces several invariants that JFC's
/// `build_provider_messages*` mutations could violate:
///
/// 1. A user message containing `tool_result` MUST contain ONLY
///    tool_result blocks (no text, no images, no other types).
/// 2. A `tool_result` user message MUST be immediately preceded by an
///    assistant message that contains the matching `tool_use` IDs.
/// 3. Every `tool_use` ID in an assistant message MUST have a matching
///    `tool_result` in the next user message.
/// 4. A `tool_use` block (or `server_tool_use`) MUST NOT appear in a
///    user message — that produces an immediate API 400
///    "tool_use blocks can only appear in assistant messages".
///
/// Unconditionally on (not `#[cfg(debug_assertions)]`). The walk is
/// O(n+m), negligible vs. the network round-trip we're about to make,
/// and the trace it emits is the only signal we have when release-mode
/// users hit a wire-shape regression. We do NOT block the send — the
/// gateway may be lenient, and a forensic log is more useful than a
/// hard panic.
pub fn validate_provider_messages(msgs: &[ProviderMessage]) {
    // Invariant 4: a tool_use block must NEVER appear in a User message.
    // This is the post-merge view of the same bug as the role-guarded
    // push paths in event_loop.rs — if the build_provider_messages step
    // somehow produces one, log loudly so it's not silent on the next
    // 400.
    for (i, msg) in msgs.iter().enumerate() {
        if !matches!(msg.role, ProviderRole::User) {
            continue;
        }
        let bad: Vec<&'static str> = msg
            .content
            .iter()
            .filter_map(|c| match c {
                ProviderContent::ToolUse { .. } => Some("tool_use"),
                ProviderContent::ServerToolUse { .. } => Some("server_tool_use"),
                ProviderContent::ServerToolResult { .. } => Some("server_tool_result"),
                _ => None,
            })
            .collect();
        if !bad.is_empty() {
            tracing::error!(
                target: "jfc::stream::invariants",
                msg_index = i,
                offending = ?bad,
                "provider message invariant violation: user message carries tool_use/server_tool_* blocks — will produce API 400"
            );
        }
    }
    use std::collections::HashSet;
    for (i, msg) in msgs.iter().enumerate() {
        if matches!(msg.role, ProviderRole::User) && contains_tool_result(msg) {
            // Invariant 1: tool_result purity.
            let non_tool_result = msg
                .content
                .iter()
                .any(|c| !matches!(c, ProviderContent::ToolResult { .. }));
            if non_tool_result {
                tracing::warn!(
                    target: "jfc::stream::invariants",
                    msg_index = i,
                    content_kinds = ?msg.content.iter().map(|c| match c {
                        ProviderContent::Text(_) => "text",
                        ProviderContent::Thinking { .. } => "thinking",
                        ProviderContent::ToolUse { .. } => "tool_use",
                        ProviderContent::ToolResult { .. } => "tool_result",
                        ProviderContent::ServerToolUse { .. } => "server_tool_use",
                        ProviderContent::ServerToolResult { .. } => "server_tool_result",
                        ProviderContent::Attachment(_) => "attachment",
                        ProviderContent::RedactedThinking { .. } => "redacted_thinking",
                    }).collect::<Vec<_>>(),
                    "provider message invariant violation: user message contains tool_result mixed with other content"
                );
            }
            // Invariant 2: must follow an assistant with matching tool_use IDs.
            let tool_result_ids: HashSet<&str> = msg
                .content
                .iter()
                .filter_map(|c| match c {
                    ProviderContent::ToolResult { tool_use_id, .. } => Some(tool_use_id.as_str()),
                    _ => None,
                })
                .collect();
            if let Some(prev) = i.checked_sub(1).and_then(|j| msgs.get(j)) {
                if !matches!(prev.role, ProviderRole::Assistant) {
                    tracing::warn!(
                        target: "jfc::stream::invariants",
                        msg_index = i,
                        prev_role = ?prev.role,
                        "provider message invariant violation: tool_result user message not preceded by assistant"
                    );
                } else {
                    let tool_use_ids: HashSet<&str> = prev
                        .content
                        .iter()
                        .filter_map(|c| match c {
                            ProviderContent::ToolUse { id, .. } => Some(id.as_str()),
                            _ => None,
                        })
                        .collect();
                    let missing: Vec<&str> =
                        tool_result_ids.difference(&tool_use_ids).copied().collect();
                    let unmatched: Vec<&str> =
                        tool_use_ids.difference(&tool_result_ids).copied().collect();
                    if !missing.is_empty() || !unmatched.is_empty() {
                        // Claude 2.1.177 `tengu_tool_use_tool_result_mismatch_error`
                        // forensics: count how many times each offending id appears
                        // as a tool_use vs a tool_result across the whole request so
                        // a duplicate/orphan can be told apart from a true mismatch.
                        let offending_id = unmatched
                            .first()
                            .copied()
                            .or_else(|| missing.first().copied());
                        let (tool_use_occurrences, tool_result_occurrences) = offending_id
                            .map(|id| count_tool_id_occurrences(msgs, id))
                            .unwrap_or((0, 0));
                        tracing::warn!(
                            target: "jfc::stream::invariants",
                            msg_index = i,
                            tool_result_without_use = ?missing,
                            tool_use_without_result = ?unmatched,
                            offending_tool_use_id = offending_id.unwrap_or(""),
                            tool_use_occurrences,
                            tool_result_occurrences,
                            "tengu_tool_use_tool_result_mismatch_error: tool_use ↔ tool_result IDs do not match"
                        );
                    }
                }
            } else {
                tracing::warn!(
                    target: "jfc::stream::invariants",
                    msg_index = i,
                    "provider message invariant violation: leading tool_result user message"
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
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

    fn assistant_tool_use(id: &str) -> ProviderMessage {
        ProviderMessage {
            role: ProviderRole::Assistant,
            content: vec![ProviderContent::ToolUse {
                id: id.to_owned(),
                name: "Bash".to_owned(),
                input: serde_json::json!({"command": "true"}),
                thought_signature: None,
            }],
        }
    }

    fn user_tool_result(id: &str, content: &str) -> ProviderContent {
        ProviderContent::ToolResult {
            tool_use_id: id.to_owned(),
            content: content.to_owned(),
            is_error: false,
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
                thought_signature: None,
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

    #[test]
    fn repair_injects_missing_tool_result_robust() {
        let out = repair_tool_result_pairing(vec![user_text("hi"), assistant_tool_use("toolu_1")]);
        assert_eq!(out.len(), 3);
        let user = out.last().expect("synthetic tool result user message");
        assert_eq!(user.role, ProviderRole::User);
        assert!(matches!(
            &user.content[0],
            ProviderContent::ToolResult { tool_use_id, is_error: true, .. }
                if tool_use_id == "toolu_1"
        ));
    }

    #[test]
    fn repair_removes_leading_orphan_tool_result_robust() {
        let out = repair_tool_result_pairing(vec![ProviderMessage {
            role: ProviderRole::User,
            content: vec![user_tool_result("toolu_orphan", "stale")],
        }]);
        assert_eq!(out.len(), 1);
        assert!(matches!(
            &out[0].content[0],
            ProviderContent::Text(text) if text.contains("Orphaned tool result removed")
        ));
    }

    #[test]
    fn repair_dedupes_and_removes_extra_tool_results_robust() {
        let out = repair_tool_result_pairing(vec![
            user_text("hi"),
            assistant_tool_use("toolu_ok"),
            ProviderMessage {
                role: ProviderRole::User,
                content: vec![
                    user_tool_result("toolu_ok", "first"),
                    user_tool_result("toolu_ok", "duplicate"),
                    user_tool_result("toolu_extra", "extra"),
                ],
            },
        ]);
        let user = out.last().expect("tool result user message");
        let result_ids: Vec<_> = user
            .content
            .iter()
            .filter_map(|content| match content {
                ProviderContent::ToolResult { tool_use_id, .. } => Some(tool_use_id.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(result_ids, vec!["toolu_ok"]);
    }

    #[test]
    fn repair_folds_system_reminder_text_into_last_tool_result_normal() {
        let out = repair_tool_result_pairing(vec![
            user_text("hi"),
            assistant_tool_use("toolu_ok"),
            ProviderMessage {
                role: ProviderRole::User,
                content: vec![
                    user_tool_result("toolu_ok", "first"),
                    ProviderContent::Text(
                        "<system-reminder>files changed</system-reminder>".to_owned(),
                    ),
                ],
            },
        ]);
        let user = out.last().expect("tool result user message");
        assert_eq!(user.content.len(), 1);
        assert!(matches!(
            &user.content[0],
            ProviderContent::ToolResult { content, .. }
                if content.contains("first")
                    && content.contains("<system-reminder>files changed</system-reminder>")
        ));
    }

    #[test]
    fn repair_strips_unpaired_server_tool_use_from_normal_send_robust() {
        let out = repair_tool_result_pairing(vec![ProviderMessage {
            role: ProviderRole::Assistant,
            content: vec![ProviderContent::ServerToolUse {
                id: "srvtoolu_1".to_owned(),
                name: "web_search".to_owned(),
                input: serde_json::json!({"query": "rust"}),
            }],
        }]);
        assert_eq!(out.len(), 1);
        assert!(matches!(
            &out[0].content[0],
            ProviderContent::Text(text) if text == TOOL_USE_INTERRUPTED_TEXT
        ));
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

    // Normal: pause_turn resume DOES strip trailing empty assistants
    // (placeholder leak from `continue_after_pause_turn`'s staging) but
    // does NOT inject the `"Continue from where you left off."` user.
    // Per Anthropic spec (cli.js v142:622686), the trailing assistant's
    // `server_tool_use` block is itself the resume cue.
    #[test]
    fn pause_turn_resume_keeps_trailing_assistant_normal() {
        let input = vec![user_text("hi"), assistant_text("partial reply")];
        let out = prepare_for_pause_turn_resume(input);
        assert_eq!(out.len(), 2);
        assert_eq!(out.last().unwrap().role, ProviderRole::Assistant);
    }

    // Robust: pause_turn resume still strips empty trailing assistants
    // (placeholder slots staged by `continue_after_pause_turn`) — only
    // the synthetic-user injection is skipped.
    #[test]
    fn pause_turn_resume_strips_empty_assistant_robust() {
        let input = vec![
            user_text("hi"),
            assistant_text("partial"),
            assistant_text(""),
        ];
        let out = prepare_for_pause_turn_resume(input);
        assert_eq!(out.len(), 2);
        assert_eq!(out.last().unwrap().role, ProviderRole::Assistant);
        if let ProviderContent::Text(t) = &out.last().unwrap().content[0] {
            assert_eq!(t, "partial");
        } else {
            panic!("expected Text");
        }
    }

    // Robust: pause_turn resume must NEVER inject the "Continue from
    // where you left off." filler — the Anthropic API detects that as
    // a real user turn and breaks the server-side loop resumption.
    #[test]
    fn pause_turn_resume_never_injects_continue_filler_robust() {
        let input = vec![user_text("search"), assistant_text("looking up rust docs")];
        let out = prepare_for_pause_turn_resume(input);
        for msg in &out {
            if msg.role == ProviderRole::User {
                for c in &msg.content {
                    if let ProviderContent::Text(t) = c {
                        assert!(
                            !t.contains("Continue from where you left off"),
                            "pause_turn resume injected forbidden filler: {t}"
                        );
                    }
                }
            }
        }
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

    // Normal: the forensic structure description names each message's role and
    // block kinds, with tool ids inlined — the format Claude 2.1.177 logs for
    // pairing diagnostics.
    #[test]
    fn describe_message_structure_names_blocks_and_ids_normal() {
        let msgs = vec![
            user_text("hi"),
            assistant_tool_use("toolu_1"),
            ProviderMessage {
                role: ProviderRole::User,
                content: vec![user_tool_result("toolu_1", "ok")],
            },
        ];
        let desc = describe_message_structure(&msgs);
        assert_eq!(
            desc,
            "[0] user([text]); [1] assistant([tool_use:toolu_1]); [2] user([tool_result:toolu_1])"
        );
    }

    // Normal: occurrence counts distinguish a use from its paired result.
    #[test]
    fn count_tool_id_occurrences_counts_use_and_result_normal() {
        let msgs = vec![
            assistant_tool_use("toolu_1"),
            ProviderMessage {
                role: ProviderRole::User,
                content: vec![user_tool_result("toolu_1", "ok")],
            },
        ];
        assert_eq!(count_tool_id_occurrences(&msgs, "toolu_1"), (1, 1));
        assert_eq!(count_tool_id_occurrences(&msgs, "toolu_missing"), (0, 0));
    }

    // Robust: a duplicated tool_use id is reflected as 2 occurrences so an
    // orphan (1,0) can be told apart from a duplicate (2,_).
    #[test]
    fn count_tool_id_occurrences_reflects_duplicates_robust() {
        let msgs = vec![
            assistant_tool_use("toolu_dup"),
            assistant_tool_use("toolu_dup"),
        ];
        assert_eq!(count_tool_id_occurrences(&msgs, "toolu_dup"), (2, 0));
    }

    // Robust: strict-mode flag parsing only trips on explicit truthy values.
    #[test]
    fn strict_tool_result_pairing_defaults_off_robust() {
        // SAFETY: single-threaded test; we set then clear the var.
        unsafe { std::env::remove_var("JFC_STRICT_TOOL_RESULT_PAIRING") };
        assert!(!strict_tool_result_pairing());
        unsafe { std::env::set_var("JFC_STRICT_TOOL_RESULT_PAIRING", "1") };
        assert!(strict_tool_result_pairing());
        unsafe { std::env::set_var("JFC_STRICT_TOOL_RESULT_PAIRING", "off") };
        assert!(!strict_tool_result_pairing());
        unsafe { std::env::remove_var("JFC_STRICT_TOOL_RESULT_PAIRING") };
    }
}

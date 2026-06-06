use super::{ModelUsage, TaskStatusPart, ToolCall, ToolStatus};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    User,
    Assistant,
}

impl std::fmt::Display for Role {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::User => f.write_str("user"),
            Self::Assistant => f.write_str("assistant"),
        }
    }
}

#[derive(Clone, Debug)]
pub enum MessagePart {
    Text(String),
    Reasoning(String),
    /// Opaque redacted thinking blob from Anthropic. Round-tripped verbatim.
    RedactedThinking(String),
    Tool(Box<ToolCall>),
    TaskStatus(TaskStatusPart),
    CompactBoundary {
        pre_tokens: usize,
    },
    /// A parallel-advisor reply (see `crate::advisor`). Rendered with a
    /// distinct visual style (italic + secondary text color + "ADVISOR:"
    /// prefix) so the user can tell at a glance that this came from the
    /// out-of-band advisor and not the main agent. Doesn't participate in
    /// the model's normal turn accounting — it's a UI-only side effect of
    /// `/advisor <query>`.
    Advisor(String),
}

impl MessagePart {
    pub fn tool(tool: ToolCall) -> Self {
        Self::Tool(Box::new(tool))
    }

    pub fn tool_boxed(tool: Box<ToolCall>) -> Self {
        Self::Tool(tool)
    }

    pub fn approx_text_len(&self) -> usize {
        match self {
            Self::Text(s) | Self::Reasoning(s) | Self::Advisor(s) => s.len(),
            Self::RedactedThinking(s) => s.len(),
            Self::Tool(tc) => tc.input.summary().len() + tc.output.approx_text_len(),
            Self::TaskStatus(ts) => {
                ts.description.len() + ts.summary.as_deref().map_or(0, |s| s.len())
            }
            Self::CompactBoundary { .. } => 0,
        }
    }

    pub fn text_only(&self) -> String {
        match self {
            Self::Text(s) | Self::Reasoning(s) => s.clone(),
            Self::RedactedThinking(_) => String::new(),
            Self::Advisor(s) => format!("[Advisor: {s}]"),
            Self::Tool(tc) => {
                format!("[Tool: {} → {}]", tc.kind.label(), tc.output.text_only())
            }
            Self::TaskStatus(ts) => {
                format!("[Task {}: {}]", ts.task_id, ts.description)
            }
            Self::CompactBoundary { pre_tokens } => {
                format!("[Compact boundary, pre={pre_tokens} tokens]")
            }
        }
    }

    pub fn to_display_string(&self) -> String {
        match self {
            Self::Text(s) => s.clone(),
            Self::Reasoning(s) => format!("[Reasoning: {}]", s),
            Self::RedactedThinking(data) => {
                format!(
                    "[Redacted thinking: provider withheld plaintext, {} bytes preserved]",
                    data.len()
                )
            }
            Self::Advisor(s) => format!("[Advisor: {s}]"),
            Self::Tool(tc) => {
                format!(
                    "[Tool: {} | Input: {} | Output: {}]",
                    tc.kind.label(),
                    tc.input.summary(),
                    tc.output.to_display_string(),
                )
            }
            Self::TaskStatus(ts) => {
                format!(
                    "[Task {} | {} | {:?}]",
                    ts.task_id, ts.description, ts.status
                )
            }
            Self::CompactBoundary { pre_tokens } => {
                format!("[Compact boundary, pre={pre_tokens} tokens]")
            }
        }
    }
}

#[derive(Clone, Debug)]
pub struct ChatMessage {
    pub role: Role,
    pub parts: Vec<MessagePart>,
    pub agent_name: Option<String>,
    pub model_name: Option<String>,
    pub cost_tier: Option<String>,
    pub elapsed: Option<String>,
    /// Token usage as of the END of this assistant turn. Set on
    /// `StreamUsage` (via `apply_to_last_assistant`) so when the
    /// session is later resumed, `App::recompute_token_estimate` can
    /// walk backwards to the last assistant message with usage and
    /// re-seat the Context gauge at the correct value. Mirrors v126's
    /// `Wd(messages)` (cli.js:197282-197294) which finds the last
    /// usage block and totals input + cache_read + cache_write +
    /// output.
    pub usage: Option<ModelUsage>,
    /// True when this user message is a placeholder for a queued
    /// prompt that hasn't been drained yet. Queued prompts render in
    /// the transcript so the user can see "I queued this", but they
    /// MUST be filtered out of `build_provider_messages*` — otherwise
    /// the agentic continuation that fires while the queue is filling
    /// would send the queued user prompt to the provider as part of
    /// the current turn, polluting the prompt and creating the
    /// "context jumped after queueing a message" symptom. Set to
    /// `false` after `drain_queued_prompts` promotes the message to a
    /// real submission. Default `false` so existing call sites stay
    /// correct; only the queueing path flips this.
    pub queued: bool,
    /// Prompt-local image/PDF attachments owned by this message.
    /// Populated at submit time from `app.pasted_images` by matching
    /// `[Image #N]` markers in the message text. Replaces the old
    /// process-global queue for paste-originated images.
    pub attachments: Vec<crate::attachments::Attachment>,
}

impl ChatMessage {
    pub fn user(content: String) -> Self {
        Self {
            role: Role::User,
            parts: vec![MessagePart::Text(content)],
            agent_name: None,
            model_name: None,
            cost_tier: None,
            elapsed: None,
            usage: None,
            queued: false,
            attachments: Vec::new(),
        }
    }

    /// User message that's a placeholder for a queued prompt. Identical
    /// to `user()` except `queued = true`, which `build_provider_messages*`
    /// uses to skip the message until `drain_queued_prompts` promotes it.
    pub fn user_queued(content: String) -> Self {
        Self {
            role: Role::User,
            parts: vec![MessagePart::Text(content)],
            agent_name: None,
            model_name: None,
            cost_tier: None,
            elapsed: None,
            usage: None,
            queued: true,
            attachments: Vec::new(),
        }
    }

    pub fn assistant(content: String) -> Self {
        // No placeholder values — fields are set authentically by the
        // stream pipeline (`elapsed` at StreamDone via `Cooked for Xs`,
        // `model_name` from the active provider). Earlier hardcoded
        // strings ("Sisyphus - Ultraworker", "$$$$", "3.9s") leaked into
        // session.json files and showed up under loaded sessions before
        // the next turn could overwrite them.
        Self {
            role: Role::Assistant,
            parts: vec![MessagePart::Text(content)],
            agent_name: None,
            model_name: None,
            cost_tier: None,
            elapsed: None,
            usage: None,
            queued: false,
            attachments: Vec::new(),
        }
    }

    pub fn assistant_parts(parts: Vec<MessagePart>) -> Self {
        Self {
            role: Role::Assistant,
            parts,
            agent_name: None,
            model_name: None,
            cost_tier: None,
            elapsed: None,
            usage: None,
            queued: false,
            attachments: Vec::new(),
        }
    }

    pub fn compact_boundary(summary: &str, pre_tokens: usize) -> Self {
        Self {
            role: Role::User,
            parts: vec![
                MessagePart::CompactBoundary { pre_tokens },
                MessagePart::Text(format!(
                    "This session is being continued from a previous conversation that ran out of context. \
                     The summary below covers the earlier portion of the conversation.\n\n\
                     {summary}\n\n\
                     Continue the conversation from where it left off without asking further questions. \
                     Resume directly — do not acknowledge the summary, do not recap what was happening, \
                     do not preface with \"I'll continue\" or similar. Pick up the last task as if the break never happened."
                )),
            ],
            agent_name: Some("system".into()),
            model_name: None,
            cost_tier: None,
            elapsed: None,
            usage: None,
            queued: false,
            attachments: Vec::new(),
        }
    }

    pub fn role_is_user(&self) -> bool {
        self.role == Role::User
    }

    pub fn is_compact_boundary(&self) -> bool {
        self.parts
            .iter()
            .any(|p| matches!(p, MessagePart::CompactBoundary { .. }))
    }
}

/// Variants of the "messages alternate user/assistant" invariant. Each
/// carries enough context (an index, a role, optionally a `ToolId`) for
/// a tracing log to point at the offending entry.
#[derive(thiserror::Error, Debug, PartialEq, Eq)]
pub enum TurnInvariantError {
    #[error("two consecutive user messages at index {at_index}")]
    ConsecutiveUser { at_index: usize },
    #[error("two consecutive assistant messages at index {at_index}")]
    ConsecutiveAssistant { at_index: usize },
    #[error("empty {role} message at index {at_index}")]
    EmptyMessage { at_index: usize, role: Role },
    #[error(
        "orphan tool_use {tool_id} at index {at_index} (no matching tool_result before next turn)"
    )]
    OrphanToolUse {
        tool_id: crate::ids::ToolId,
        at_index: usize,
    },
    #[error("orphan tool_result {tool_id} at index {at_index} (tool part on a user message)")]
    OrphanToolResult {
        tool_id: crate::ids::ToolId,
        at_index: usize,
    },
    #[error("leading assistant message at index 0 (role={role})")]
    LeadingAssistant { role: Role },
}

/// Walk a message slice and report the first `TurnInvariantError` that
/// breaks the user/assistant alternation invariant.
/// Merge consecutive `MessagePart::Text` parts within a single ChatMessage.
/// Fixes the fragmentation bug where streaming deltas or partial appends
/// create N separate text parts that should be one. Preserves non-Text parts
/// (Tool, Reasoning, RedactedThinking, TaskStatus, Attachment) in their order.
pub fn merge_consecutive_text_parts(parts: &mut Vec<MessagePart>) {
    let mut i = 0;
    while i + 1 < parts.len() {
        let merge = matches!(
            (&parts[i], &parts[i + 1]),
            (MessagePart::Text(_), MessagePart::Text(_))
        );
        if merge {
            // Take ownership of the next part's text and append to current.
            let next = parts.remove(i + 1);
            if let (MessagePart::Text(cur), MessagePart::Text(nxt)) = (&mut parts[i], next) {
                cur.push_str(&nxt);
            }
            // Don't advance i — there may be another Text after this.
        } else {
            i += 1;
        }
    }
}

pub fn validate_turn_invariants(messages: &[ChatMessage]) -> Result<(), TurnInvariantError> {
    validate_turn_invariants_inner(messages, /* allow_streaming_tail = */ false)
}

/// Inner form that allows the trailing message to be a (possibly empty)
/// assistant streaming placeholder.
pub(crate) fn validate_turn_invariants_inner(
    messages: &[ChatMessage],
    allow_streaming_tail: bool,
) -> Result<(), TurnInvariantError> {
    if messages.is_empty() {
        return Ok(());
    }

    let first = &messages[0];
    if first.role == Role::Assistant && !first.is_compact_boundary() {
        return Err(TurnInvariantError::LeadingAssistant { role: first.role });
    }

    let last_idx = messages.len() - 1;
    for (i, m) in messages.iter().enumerate() {
        if i > 0 {
            let prev = &messages[i - 1];
            if prev.role == m.role {
                let either_is_boundary = prev.is_compact_boundary() || m.is_compact_boundary();
                if !either_is_boundary {
                    return Err(match m.role {
                        Role::User => TurnInvariantError::ConsecutiveUser { at_index: i },
                        Role::Assistant => TurnInvariantError::ConsecutiveAssistant { at_index: i },
                    });
                }
            }
        }

        let has_content = m.parts.iter().any(|p| match p {
            MessagePart::Text(s) | MessagePart::Reasoning(s) | MessagePart::Advisor(s) => {
                !s.is_empty()
            }
            MessagePart::RedactedThinking(_) => true,
            MessagePart::Tool(_)
            | MessagePart::TaskStatus(_)
            | MessagePart::CompactBoundary { .. } => true,
        });
        let is_streaming_tail = allow_streaming_tail && i == last_idx && m.role == Role::Assistant;
        if !has_content && !is_streaming_tail {
            return Err(TurnInvariantError::EmptyMessage {
                at_index: i,
                role: m.role,
            });
        }

        if m.role == Role::User {
            for part in &m.parts {
                if let MessagePart::Tool(tc) = part {
                    return Err(TurnInvariantError::OrphanToolResult {
                        tool_id: tc.id.clone(),
                        at_index: i,
                    });
                }
            }
        }
    }

    for (i, m) in messages.iter().enumerate() {
        if m.role != Role::Assistant {
            continue;
        }
        let is_tail = i == last_idx;
        if is_tail {
            continue;
        }
        for part in &m.parts {
            if let MessagePart::Tool(tc) = part
                && matches!(tc.status, ToolStatus::Pending | ToolStatus::Running)
            {
                return Err(TurnInvariantError::OrphanToolUse {
                    tool_id: tc.id.clone(),
                    at_index: i,
                });
            }
        }
    }

    Ok(())
}

use crate::types::{ChatMessage, MessagePart};

use super::message_pressure::estimate_transcript_tokens;

const MIN_TAIL_MESSAGES: usize = 12;
const SUMMARY_TOKEN_RESERVE: usize = 4_000;
const MAX_EXCERPT_CHARS: usize = 4_000;
const MAX_EXCERPT_PER_MESSAGE: usize = 500;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct TranscriptBoundaryBudget {
    pub(crate) window_tokens: usize,
    pub(crate) max_output_tokens: Option<usize>,
    pub(crate) overhead_tokens: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TranscriptBoundaryResult {
    pub(crate) omitted_messages: usize,
    pub(crate) kept_messages: usize,
    pub(crate) pre_tokens: usize,
    pub(crate) post_tokens: usize,
    pub(crate) archive_id: Option<String>,
}

pub(crate) fn materialize_transcript_boundary(
    messages: &mut Vec<ChatMessage>,
    budget: TranscriptBoundaryBudget,
) -> Option<TranscriptBoundaryResult> {
    if messages.len() <= MIN_TAIL_MESSAGES {
        return None;
    }
    let pre_tokens = estimate_transcript_tokens(messages);
    if !over_threshold(pre_tokens, budget) {
        return None;
    }

    let tail_start = tail_start_index(messages, budget);
    if tail_start == 0 || tail_start >= messages.len() {
        return None;
    }

    let omitted: Vec<ChatMessage> = messages[..tail_start].to_vec();
    let tail: Vec<ChatMessage> = messages[tail_start..].to_vec();
    let base_summary = boundary_summary(&omitted, pre_tokens, None);
    let archive_id = match crate::compact_archive::archive_current_project(
        &omitted,
        pre_tokens,
        &base_summary,
    ) {
        Ok(Some(meta)) => Some(meta.id),
        Ok(None) => None,
        Err(error) => {
            tracing::warn!(
                target: "jfc::context",
                error = %error,
                "failed to archive transcript prefix before durable boundary"
            );
            None
        }
    };
    let summary = boundary_summary(&omitted, pre_tokens, archive_id.as_deref());
    messages.clear();
    messages.push(ChatMessage::compact_boundary(&summary, pre_tokens));
    messages.extend(tail);

    let post_tokens = estimate_transcript_tokens(messages);
    Some(TranscriptBoundaryResult {
        omitted_messages: omitted.len(),
        kept_messages: messages.len().saturating_sub(1),
        pre_tokens,
        post_tokens,
        archive_id,
    })
}

fn over_threshold(tokens: usize, budget: TranscriptBoundaryBudget) -> bool {
    matches!(
        crate::compact::compact_level_with_output(
            tokens,
            budget.window_tokens,
            budget.max_output_tokens,
        ),
        crate::compact::CompactLevel::Compact | crate::compact::CompactLevel::Blocked
    )
}

fn tail_start_index(messages: &[ChatMessage], budget: TranscriptBoundaryBudget) -> usize {
    let threshold = crate::compact::compact_threshold_with_output(
        budget.window_tokens,
        budget.max_output_tokens,
    );
    let target = threshold
        .saturating_sub(budget.overhead_tokens)
        .saturating_sub(SUMMARY_TOKEN_RESERVE);
    let mut start = messages.len();
    let mut tail_tokens = 0usize;

    while start > 0
        && (tail_tokens < target || messages.len().saturating_sub(start) < MIN_TAIL_MESSAGES)
    {
        start -= 1;
        tail_tokens =
            tail_tokens.saturating_add(estimate_transcript_tokens(&messages[start..=start]));
    }
    start
}

fn boundary_summary(
    messages: &[ChatMessage],
    pre_tokens: usize,
    archive_id: Option<&str>,
) -> String {
    let mut out = format!(
        "JFC materialized an automatic context boundary before this request. \
         The omitted prefix contained {} messages and was estimated at {} tokens. \
         The live transcript now keeps the recent tail verbatim so the next provider request starts below the context threshold.",
        messages.len(),
        pre_tokens
    );
    if let Some(id) = archive_id {
        out.push_str(&format!(
            "\n\nExact omitted transcript archive: `{id}`. Use `/expand {id}` to inspect it."
        ));
    }
    let excerpts = recent_excerpts(messages);
    if !excerpts.is_empty() {
        out.push_str("\n\nRecent omitted excerpts:\n");
        out.push_str(&excerpts);
    }
    out
}

fn recent_excerpts(messages: &[ChatMessage]) -> String {
    let mut out = String::new();
    let mut used = 0usize;
    for message in messages.iter().rev() {
        for part in message.parts.iter().rev() {
            let Some(text) = part_text(part) else {
                continue;
            };
            if used >= MAX_EXCERPT_CHARS {
                break;
            }
            let clipped = truncate_chars(text.trim(), MAX_EXCERPT_PER_MESSAGE);
            if clipped.is_empty() {
                continue;
            }
            used = used.saturating_add(clipped.len());
            out.push_str(&format!(
                "- {}: {}\n",
                message.role,
                clipped.replace('\n', "\\n")
            ));
        }
    }
    out
}

fn part_text(part: &MessagePart) -> Option<&str> {
    match part {
        MessagePart::Text(text)
        | MessagePart::Reasoning(text)
        | MessagePart::Advisor(text)
        | MessagePart::RedactedThinking(text) => Some(text.as_str()),
        MessagePart::ReasoningSignature(_)
        | MessagePart::Tool(_)
        | MessagePart::TaskStatus(_)
        | MessagePart::CompactBoundary { .. } => None,
    }
}

fn truncate_chars(text: &str, max_chars: usize) -> String {
    let mut chars = text.chars();
    let mut out: String = chars.by_ref().take(max_chars).collect();
    if chars.next().is_some() {
        out.push_str("...");
    }
    out
}

#[cfg(test)]
mod tests;

use jfc_provider::{ProviderContent, ProviderMessage, ProviderRole};
use std::fmt::Write as _;

use super::provider_payload::{provider_message_tokens, provider_messages_tokens};
use stats::HistoryStats;

const MIN_TAIL_MESSAGES: usize = 12;
const SUMMARY_TOKEN_RESERVE: u64 = 4_000;
const MAX_EXCERPT_CHARS: usize = 6_000;
const MAX_EXCERPT_PER_BLOCK: usize = 700;

#[derive(Debug, Clone)]
pub(crate) struct ProviderHistoryTransform {
    pub(crate) messages: Vec<ProviderMessage>,
    pub(crate) omitted_messages: usize,
    pub(crate) kept_messages: usize,
    pub(crate) omitted_tokens: u64,
    pub(crate) kept_tokens: u64,
    pub(crate) archive_id: Option<String>,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct ProviderHistoryBudget {
    pub(crate) window_tokens: usize,
    pub(crate) max_output_tokens: Option<usize>,
    pub(crate) overhead_tokens: usize,
}

pub(crate) fn compact_provider_history(
    messages: &[ProviderMessage],
    budget: ProviderHistoryBudget,
) -> Option<ProviderHistoryTransform> {
    compact_provider_history_with_archive(messages, budget, None)
}

pub(crate) fn compact_provider_history_with_archive(
    messages: &[ProviderMessage],
    budget: ProviderHistoryBudget,
    archive_id: Option<&str>,
) -> Option<ProviderHistoryTransform> {
    if messages.len() <= MIN_TAIL_MESSAGES {
        return None;
    }

    let target_tokens = crate::compact::compact_threshold_with_output(
        budget.window_tokens,
        budget.max_output_tokens,
    )
    .try_into()
    .unwrap_or(u64::MAX);
    let tail_target_tokens = target_tokens
        .saturating_sub(budget.overhead_tokens.try_into().unwrap_or(u64::MAX))
        .saturating_sub(SUMMARY_TOKEN_RESERVE);
    let mut tail_start = messages.len();
    let mut tail_tokens = 0u64;

    while tail_start > 0
        && (tail_tokens < tail_target_tokens
            || messages.len().saturating_sub(tail_start) < MIN_TAIL_MESSAGES)
    {
        tail_start -= 1;
        tail_tokens = tail_tokens.saturating_add(provider_message_tokens(&messages[tail_start]));
    }

    tail_start = first_safe_tail_index(messages, tail_start);
    if tail_start == 0 || tail_start >= messages.len() {
        return None;
    }

    let omitted = &messages[..tail_start];
    let tail = &messages[tail_start..];
    let omitted_tokens = provider_messages_tokens(omitted);
    let summary = history_block(omitted, omitted_tokens, tail.len(), archive_id);
    let transformed = splice_history_block(summary, tail);
    let kept_tokens = provider_messages_tokens(&transformed);

    Some(ProviderHistoryTransform {
        messages: transformed,
        omitted_messages: omitted.len(),
        kept_messages: tail.len(),
        omitted_tokens,
        kept_tokens,
        archive_id: archive_id.map(str::to_owned),
    })
}

fn first_safe_tail_index(messages: &[ProviderMessage], start: usize) -> usize {
    let mut idx = start;
    while idx < messages.len() && is_orphan_tool_result_start(&messages[idx]) {
        idx += 1;
    }
    idx
}

fn is_orphan_tool_result_start(message: &ProviderMessage) -> bool {
    message.role == ProviderRole::User
        && message
            .content
            .iter()
            .any(|content| matches!(content, ProviderContent::ToolResult { .. }))
}

fn splice_history_block(summary: String, tail: &[ProviderMessage]) -> Vec<ProviderMessage> {
    let mut out = Vec::with_capacity(tail.len().saturating_add(1));
    let Some(first) = tail.first() else {
        return out;
    };

    if first.role == ProviderRole::User {
        let mut first = first.clone();
        first.content.insert(0, ProviderContent::Text(summary));
        out.push(first);
        out.extend_from_slice(&tail[1..]);
    } else {
        out.push(ProviderMessage {
            role: ProviderRole::User,
            content: vec![ProviderContent::Text(summary)],
        });
        out.extend_from_slice(tail);
    }
    out
}

fn history_block(
    messages: &[ProviderMessage],
    omitted_tokens: u64,
    kept_messages: usize,
    archive_id: Option<&str>,
) -> String {
    let stats = HistoryStats::from_messages(messages);
    let mut block = format!(
        "<session-history compacted=\"true\" omitted_messages=\"{}\" omitted_tokens=\"{}\" kept_live_tail_messages=\"{}\">\n\
         Older provider-visible replay was compacted locally before this request so the live turn can stay inside the model context window. \
         The full transcript remains in JFC session storage; this block is only the bounded provider-visible replacement.\n\
         Counts: user_messages={}, assistant_messages={}, text_blocks={}, tool_use_blocks={}, tool_result_blocks={}, attachment_blocks={}, thinking_blocks={}.\n",
        messages.len(),
        omitted_tokens,
        kept_messages,
        stats.user_messages,
        stats.assistant_messages,
        stats.text_blocks,
        stats.tool_use_blocks,
        stats.tool_result_blocks,
        stats.attachment_blocks,
        stats.thinking_blocks,
    );
    if let Some(archive_id) = archive_id {
        let _ = writeln!(
            &mut block,
            "Provider-visible archive: `{archive_id}`. Use `/expand {archive_id}` to inspect the exact omitted provider replay."
        );
    }
    append_recent_excerpts(&mut block, messages);
    block.push_str("</session-history>");
    block
}

fn append_recent_excerpts(block: &mut String, messages: &[ProviderMessage]) {
    let mut excerpt_chars = 0usize;
    let mut excerpts = Vec::new();
    for message in messages.iter().rev() {
        for content in message.content.iter().rev() {
            let Some(text) = content_excerpt(content) else {
                continue;
            };
            if excerpt_chars >= MAX_EXCERPT_CHARS {
                break;
            }
            let clipped = truncate_chars(text, MAX_EXCERPT_PER_BLOCK);
            excerpt_chars = excerpt_chars.saturating_add(clipped.len());
            excerpts.push(format!(
                "- {:?}: {}",
                message.role,
                clipped.replace('\n', "\\n")
            ));
        }
    }
    if excerpts.is_empty() {
        return;
    }
    block.push_str("Recent omitted excerpts:\n");
    for excerpt in excerpts.into_iter().rev() {
        block.push_str(&excerpt);
        block.push('\n');
    }
}

fn content_excerpt(content: &ProviderContent) -> Option<&str> {
    match content {
        ProviderContent::Text(text)
        | ProviderContent::Thinking { text, .. }
        | ProviderContent::ToolResult { content: text, .. }
        | ProviderContent::RedactedThinking { data: text } => Some(text.as_str()),
        ProviderContent::ToolUse { .. }
        | ProviderContent::ServerToolUse { .. }
        | ProviderContent::ServerToolResult { .. }
        | ProviderContent::Attachment(_) => None,
    }
}

fn truncate_chars(text: &str, max_chars: usize) -> String {
    let mut end = 0usize;
    for (count, (idx, ch)) in text.char_indices().enumerate() {
        if count >= max_chars {
            return format!("{}...", &text[..end]);
        }
        end = idx + ch.len_utf8();
    }
    text.to_owned()
}

mod stats;
#[cfg(test)]
mod tests;

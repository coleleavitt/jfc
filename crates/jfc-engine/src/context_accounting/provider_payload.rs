use jfc_provider::{ProviderContent, ProviderMessage};

pub(crate) fn chars_to_tokens(chars: usize) -> u64 {
    (chars / 4).try_into().unwrap_or(u64::MAX)
}

pub(crate) fn provider_content_chars(content: &ProviderContent) -> usize {
    match content {
        ProviderContent::Text(text) => text.len(),
        ProviderContent::Thinking { text, signature } => {
            text.len() + signature.as_deref().map_or(0, str::len)
        }
        ProviderContent::ToolResult { content, .. } => content.len(),
        ProviderContent::ToolUse { name, input, .. }
        | ProviderContent::ServerToolUse { name, input, .. } => {
            name.len() + input.to_string().len()
        }
        ProviderContent::ServerToolResult { content, .. } => content.to_string().len(),
        ProviderContent::Attachment(attachment) => attachment.bytes.len(),
        ProviderContent::RedactedThinking { data } => data.len(),
    }
}

pub(crate) fn provider_message_tokens(message: &ProviderMessage) -> u64 {
    chars_to_tokens(message.content.iter().map(provider_content_chars).sum())
}

pub(crate) fn provider_messages_tokens(messages: &[ProviderMessage]) -> u64 {
    chars_to_tokens(
        messages
            .iter()
            .flat_map(|message| message.content.iter())
            .map(provider_content_chars)
            .sum(),
    )
}

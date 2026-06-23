use jfc_provider::{ProviderContent, ProviderMessage, ToolDef};

fn chars_to_tokens(chars: usize) -> u64 {
    (chars / 4).try_into().unwrap_or(u64::MAX)
}

fn provider_content_chars(content: &ProviderContent) -> usize {
    match content {
        ProviderContent::Text(text) => text.len(),
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

fn provider_messages_tokens(messages: &[ProviderMessage]) -> u64 {
    chars_to_tokens(
        messages
            .iter()
            .flat_map(|message| message.content.iter())
            .map(provider_content_chars)
            .sum(),
    )
}

fn tool_definition_tokens(tool: &ToolDef) -> u64 {
    chars_to_tokens(
        tool.name
            .len()
            .saturating_add(tool.description.len())
            .saturating_add(tool.input_schema.to_string().len()),
    )
}

pub(crate) fn stream_context_budget(
    system_prompt: &str,
    tools: &[ToolDef],
    memory_context_chars: usize,
    project_instructions_chars: usize,
    messages: &[ProviderMessage],
) -> jfc_core::context_budget::ContextBudget {
    let system_chars = system_prompt
        .len()
        .saturating_sub(memory_context_chars)
        .saturating_sub(project_instructions_chars);
    jfc_core::context_budget::ContextBudget {
        system_prompt_tokens: chars_to_tokens(system_chars),
        tool_definition_tokens: tools.iter().map(tool_definition_tokens).sum(),
        memory_tokens: chars_to_tokens(memory_context_chars),
        project_instructions_tokens: chars_to_tokens(project_instructions_chars),
        user_message_tokens: provider_messages_tokens(messages),
    }
}

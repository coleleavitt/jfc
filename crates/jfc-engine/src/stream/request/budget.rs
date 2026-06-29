use crate::context_accounting::{chars_to_tokens, provider_messages_tokens};
use jfc_provider::{ProviderMessage, ToolDef};

fn tool_definition_tokens(tool: &ToolDef) -> u64 {
    chars_to_tokens(
        tool.name
            .len()
            .saturating_add(tool.description.len())
            .saturating_add(tool.input_schema.to_string().len()),
    )
}

#[linkscope::instrument]
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

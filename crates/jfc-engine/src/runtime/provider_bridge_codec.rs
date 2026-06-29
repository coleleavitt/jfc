use base64::{Engine as _, engine::general_purpose::STANDARD};
use jfc_plugin_sdk::{
    BridgeProviderContent, BridgeProviderMessage, BridgeProviderRole, BridgeProviderStreamOptions,
    BridgeProviderToolDef,
};
use jfc_provider::{ProviderContent, ProviderMessage, ProviderRole, StreamOptions};

pub(crate) fn provider_message_to_bridge(message: ProviderMessage) -> BridgeProviderMessage {
    BridgeProviderMessage {
        role: match message.role {
            ProviderRole::User => BridgeProviderRole::User,
            ProviderRole::Assistant => BridgeProviderRole::Assistant,
        },
        content: message
            .content
            .into_iter()
            .map(provider_content_to_bridge)
            .collect(),
    }
}

pub(crate) fn stream_options_to_bridge(options: &StreamOptions) -> BridgeProviderStreamOptions {
    BridgeProviderStreamOptions {
        model: options.model.as_str().to_owned(),
        system: options.system.clone(),
        max_tokens: options.max_tokens,
        tools: options
            .tools
            .iter()
            .map(|tool| BridgeProviderToolDef {
                name: tool.name.clone(),
                description: tool.description.clone(),
                input_schema: tool.input_schema.clone(),
            })
            .collect(),
        thinking_budget: options.thinking_budget,
        adaptive_thinking: options.adaptive_thinking,
        thinking_display: options.thinking_display.clone(),
        temperature: options.temperature,
        top_p: options.top_p,
        reasoning_effort: options.reasoning_effort.clone(),
        provider_options: options.provider_options.clone(),
        custom_betas: options.custom_betas.clone(),
        fast_mode: options.fast_mode,
        eager_input_streaming: options.eager_input_streaming,
        strict_tool_schemas: options.strict_tool_schemas,
        task_budget_tokens: options.task_budget_tokens,
        previous_message_id: options.previous_message_id.clone(),
        context_hint_tokens_saved: options.context_hint_tokens_saved,
        thinking_token_count: options.thinking_token_count,
        mid_conversation_system: options.mid_conversation_system,
        cache_diagnosis: options.cache_diagnosis,
        prompt_caching_scope: options.prompt_caching_scope,
        session_id: options.session_id.clone(),
        advisor_model: options
            .advisor_model
            .as_ref()
            .map(|model| model.as_str().to_owned()),
        narration_summaries: options.narration_summaries,
    }
}

fn provider_content_to_bridge(content: ProviderContent) -> BridgeProviderContent {
    match content {
        ProviderContent::Text(text) => BridgeProviderContent::Text { text },
        ProviderContent::Thinking { text, signature } => {
            BridgeProviderContent::Thinking { text, signature }
        }
        ProviderContent::ToolResult {
            tool_use_id,
            content,
            is_error,
        } => BridgeProviderContent::ToolResult {
            tool_use_id,
            content,
            is_error,
        },
        ProviderContent::ToolUse {
            id,
            name,
            input,
            thought_signature,
        } => BridgeProviderContent::ToolUse {
            id,
            name,
            input,
            thought_signature,
        },
        ProviderContent::ServerToolUse { id, name, input } => {
            BridgeProviderContent::ServerToolUse { id, name, input }
        }
        ProviderContent::ServerToolResult {
            tool_use_id,
            tool_kind,
            content,
        } => BridgeProviderContent::ServerToolResult {
            tool_use_id,
            tool_kind: tool_kind.wire_type().to_owned(),
            content,
        },
        ProviderContent::Attachment(attachment) => BridgeProviderContent::Attachment {
            id: attachment.id,
            mime_type: attachment.kind.mime_type().to_owned(),
            data_base64: STANDARD.encode(attachment.bytes),
        },
        ProviderContent::RedactedThinking { data } => {
            BridgeProviderContent::RedactedThinking { data }
        }
    }
}

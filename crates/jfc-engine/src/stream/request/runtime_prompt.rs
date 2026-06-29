use std::sync::Arc;

use crate::runtime::StreamRequestOverrides;
use crate::tools;
use jfc_provider::{ModelId, Provider, ProviderMessage, StreamConvention};

use super::behavior_prompt::BehavioralPromptState;
use super::runtime_extensions::append_prompt_context_extensions;
use super::runtime_prompt_context_builtins::{
    BuiltinPromptContextState, TotalTokensPromptContextState,
};

pub(super) struct RuntimePromptState {
    pub(super) hcom_available: bool,
    pub(super) server_advisor_model: Option<ModelId>,
    pub(super) local_advisor_model: Option<ModelId>,
}

pub(super) async fn append_runtime_prompt_sections(
    system_prompt: &mut String,
    provider: &Arc<dyn Provider>,
    messages: &[ProviderMessage],
    overrides: &StreamRequestOverrides,
    behavioral_prompt: BehavioralPromptState,
) -> RuntimePromptState {
    let doc_cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    let server_advisor_model = if matches!(
        provider.stream_convention(),
        StreamConvention::AnthropicNative
    ) && matches!(provider.name(), "anthropic" | "anthropic-oauth")
    {
        crate::advisor::active_server_advisor_model()
    } else {
        None
    };
    let local_advisor_model = crate::advisor::active_local_advisor_model();
    let prompt_context_state = BuiltinPromptContextState {
        server_advisor_model: server_advisor_model.clone(),
        local_advisor_model: local_advisor_model.clone(),
        previous_session_handoff: previous_session_handoff_prompt_context_body(messages),
        background_reminders: &overrides.background_reminders,
        total_tokens_reminder: TotalTokensPromptContextState {
            mode: overrides
                .total_tokens_reminder_mode
                .unwrap_or_else(crate::total_tokens_reminder::active_mode),
            messages,
            last_usage_input_tokens: overrides.last_usage_input_tokens,
            context_window_tokens: overrides.context_window_tokens,
        },
        behavioral_prompt,
    };
    append_prompt_context_extensions(system_prompt, &doc_cwd, &prompt_context_state).await;

    if let Some(model) = &server_advisor_model {
        tracing::info!(
            target: "jfc::advisor",
            advisor_model = %model,
            "server advisor prompt-context descriptor is active"
        );
    }
    if let Some(model) = &local_advisor_model {
        tracing::info!(
            target: "jfc::advisor",
            advisor_model = %model,
            "local advisor prompt-context descriptor is active"
        );
    }
    RuntimePromptState {
        hcom_available: tools::hcom_available(),
        server_advisor_model,
        local_advisor_model,
    }
}

fn previous_session_handoff_prompt_context_body(messages: &[ProviderMessage]) -> Option<String> {
    if messages.len() > 1 {
        return None;
    }
    let root = crate::context::discover_git_root()?;
    let handoff = crate::sprint::HandoffSummary::read_latest(&root)?;
    Some(handoff.chars().take(4000).collect())
}

use std::path::Path;

use jfc_plugin_sdk::RuntimeExtensionDescriptor;
use jfc_provider::{ModelId, ProviderMessage};

use super::behavior_prompt::BehavioralPromptState;

pub(super) struct BuiltinPromptContextState<'a> {
    pub(super) server_advisor_model: Option<ModelId>,
    pub(super) local_advisor_model: Option<ModelId>,
    pub(super) previous_session_handoff: Option<String>,
    pub(super) background_reminders: &'a [String],
    pub(super) total_tokens_reminder: TotalTokensPromptContextState<'a>,
    pub(super) behavioral_prompt: BehavioralPromptState,
}

pub(super) struct TotalTokensPromptContextState<'a> {
    pub(super) mode: crate::total_tokens_reminder::TotalTokensReminderMode,
    pub(super) messages: &'a [ProviderMessage],
    pub(super) last_usage_input_tokens: Option<u64>,
    pub(super) context_window_tokens: Option<u64>,
}

pub(super) fn builtin_prompt_context_body(
    extension: &RuntimeExtensionDescriptor,
    cwd: &Path,
    state: &BuiltinPromptContextState<'_>,
    system_prompt_tokens: usize,
) -> Option<String> {
    if extension.executor.handler == jfc_plugin_host::BUILTIN_BACKGROUND_REMINDERS_PROMPT_HANDLER {
        return background_reminders_prompt_context_body(state);
    }
    if extension.executor.handler == jfc_plugin_host::BUILTIN_BRIEF_MODE_PROMPT_HANDLER {
        return brief_mode_prompt_context_body(state);
    }
    if extension.executor.handler == jfc_plugin_host::BUILTIN_DOCUMENT_FORMATS_PROMPT_HANDLER {
        return crate::document_formats::system_prompt_section(cwd);
    }
    if extension.executor.handler == jfc_plugin_host::BUILTIN_FEATURE_GATES_PROMPT_HANDLER {
        return crate::feature_gates::system_prompt_section();
    }
    if extension.executor.handler == jfc_plugin_host::BUILTIN_HARRIER_PROMPT_HANDLER {
        return harrier_prompt_context_body();
    }
    if extension.executor.handler == jfc_plugin_host::BUILTIN_INTERACTION_MODE_PROMPT_HANDLER {
        return interaction_mode_prompt_context_body(state);
    }
    if extension.executor.handler == jfc_plugin_host::BUILTIN_LOCAL_ADVISOR_PROMPT_HANDLER {
        return local_advisor_prompt_context_body(state);
    }
    if extension.executor.handler == jfc_plugin_host::BUILTIN_MARSH_PROMPT_HANDLER {
        return marsh_prompt_context_body();
    }
    if extension.executor.handler == jfc_plugin_host::BUILTIN_OUTPUT_STYLE_PROMPT_HANDLER {
        let suffix = crate::output_style::active_suffix(cwd);
        if suffix.is_some() {
            tracing::debug!(
                target: "jfc::stream",
                style = %crate::output_style::active().name(),
                "appended OutputStyle suffix through prompt-context runtime extension"
            );
        }
        return suffix;
    }
    if extension.executor.handler == jfc_plugin_host::BUILTIN_PEWTER_OWL_PROMPT_HANDLER {
        return pewter_owl_prompt_context_body(state);
    }
    if extension.executor.handler == jfc_plugin_host::BUILTIN_PREVIOUS_HANDOFF_PROMPT_HANDLER {
        return previous_session_handoff_prompt_context_body(state);
    }
    if extension.executor.handler == jfc_plugin_host::BUILTIN_SERVER_ADVISOR_PROMPT_HANDLER {
        return server_advisor_prompt_context_body(state);
    }
    if extension.executor.handler == jfc_plugin_host::BUILTIN_TOTAL_TOKENS_PROMPT_HANDLER {
        return total_tokens_prompt_context_body(state, system_prompt_tokens);
    }
    tracing::debug!(
        target: "jfc::plugin_runtime",
        plugin_id = %extension.plugin_id,
        extension_id = %extension.id,
        handler = %extension.executor.handler,
        "unknown built-in prompt-context runtime extension handler"
    );
    None
}

fn background_reminders_prompt_context_body(
    state: &BuiltinPromptContextState<'_>,
) -> Option<String> {
    if state.background_reminders.is_empty() {
        return None;
    }
    tracing::debug!(
        target: "jfc::stream",
        count = state.background_reminders.len(),
        "appending background reminders through prompt-context runtime extension"
    );
    Some(
        state
            .background_reminders
            .iter()
            .map(|body| crate::system_reminder::format(body))
            .collect::<Vec<_>>()
            .join("\n\n"),
    )
}

fn brief_mode_prompt_context_body(state: &BuiltinPromptContextState<'_>) -> Option<String> {
    if !state.behavioral_prompt.effective_brief_mode {
        return None;
    }
    Some(
        "## Brief User Messages\n\nPlain assistant text is hidden from the main chat view. Put \
         every substantive user-facing reply in `SendUserMessage`; use normal assistant text only \
         for internal reasoning that can be omitted from the user's visible transcript."
            .to_owned(),
    )
}

fn harrier_prompt_context_body() -> Option<String> {
    if !crate::feature_gates::is_enabled(crate::feature_gates::FeatureGate::Harrier) {
        return None;
    }
    Some(
        "When the user's request is concrete and bounded (a specific file, a named symbol, a \
         known feature area), do a small targeted investigation **only if you would otherwise ask \
         a clarifying question**. Prefer one CodeGraph query or one precise search, then act. Do \
         not use this as permission for a broad Read/Grep/Glob survey before routine edits. \
         Escalate to AskUserQuestion only when that targeted check surfaces multiple incompatible \
         interpretations that would meaningfully change the plan."
            .to_owned(),
    )
}

fn marsh_prompt_context_body() -> Option<String> {
    if !crate::feature_gates::is_enabled(crate::feature_gates::FeatureGate::Marsh) {
        return None;
    }
    let chunks = crate::feature_gates::marsh_drain();
    if chunks.is_empty() {
        return None;
    }
    let body = chunks.join("\n");
    let preview: String = body.chars().take(8_000).collect();
    Some(crate::system_reminder::format(&format!(
        "Bash subprocess output captured since last turn:\n```\n{preview}\n```"
    )))
}

fn interaction_mode_prompt_context_body(state: &BuiltinPromptContextState<'_>) -> Option<String> {
    state
        .behavioral_prompt
        .interaction_mode
        .prompt_section()
        .map(str::to_owned)
}

fn local_advisor_prompt_context_body(state: &BuiltinPromptContextState<'_>) -> Option<String> {
    state.local_advisor_model.as_ref()?;
    Some(
        "You have access to an `Advisor` tool backed by JFC's configured \
         local/client-side advisor model. It takes no parameters. When you call it, JFC snapshots \
         the current conversation, sends that snapshot through the configured advisor \
         provider/model, and returns the advisor's feedback as this tool's result. Call it before \
         substantive work on multi-step tasks, when stuck, when considering a change of approach, \
         and before declaring substantial work done."
            .to_owned(),
    )
}

fn server_advisor_prompt_context_body(state: &BuiltinPromptContextState<'_>) -> Option<String> {
    state.server_advisor_model.as_ref()?;
    Some(crate::advisor::SERVER_ADVISOR_SYSTEM_PROMPT.to_owned())
}

fn pewter_owl_prompt_context_body(state: &BuiltinPromptContextState<'_>) -> Option<String> {
    if state.behavioral_prompt.effective_brief_mode || !state.behavioral_prompt.pewter_owl_tool {
        return None;
    }
    Some(
        "## Pewter Owl Messaging\n\n`SendUserMessage` is available for exact user-visible content \
         between tool calls, such as generated snippets, specific values, and direct replies to \
         mid-task user messages. Routine narration and final answers may remain normal assistant \
         text."
            .to_owned(),
    )
}

fn previous_session_handoff_prompt_context_body(
    state: &BuiltinPromptContextState<'_>,
) -> Option<String> {
    state.previous_session_handoff.clone()
}

fn total_tokens_prompt_context_body(
    state: &BuiltinPromptContextState<'_>,
    system_prompt_tokens: usize,
) -> Option<String> {
    crate::total_tokens_reminder::render_for_request_with_mode(
        state.total_tokens_reminder.mode,
        state.total_tokens_reminder.messages,
        system_prompt_tokens,
        state.total_tokens_reminder.last_usage_input_tokens,
        state.total_tokens_reminder.context_window_tokens,
    )
}

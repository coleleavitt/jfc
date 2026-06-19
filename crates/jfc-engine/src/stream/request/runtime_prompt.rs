use std::sync::Arc;

use crate::runtime::StreamRequestOverrides;
use crate::tools;
use jfc_provider::{ModelId, Provider, ProviderMessage, StreamConvention};

pub(super) struct RuntimePromptState {
    pub(super) hcom_available: bool,
    pub(super) server_advisor_model: Option<ModelId>,
    pub(super) local_advisor_model: Option<ModelId>,
}

pub(super) fn append_runtime_prompt_sections(
    system_prompt: &mut String,
    provider: &Arc<dyn Provider>,
) -> RuntimePromptState {
    if let Some(gates) = crate::feature_gates::system_prompt_section() {
        system_prompt.push_str(&gates);
    }

    let doc_cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    if let Some(doc_rules) = crate::document_formats::system_prompt_section(&doc_cwd) {
        system_prompt.push_str("\n\n");
        system_prompt.push_str(&doc_rules);
    }

    if crate::feature_gates::is_enabled(crate::feature_gates::FeatureGate::Marsh) {
        let chunks = crate::feature_gates::marsh_drain();
        if !chunks.is_empty() {
            let body = chunks.join("\n");
            let preview: String = body.chars().take(8_000).collect();
            system_prompt.push_str(&format!(
                "\n\n{}",
                crate::system_reminder::format(&format!(
                    "Bash subprocess output captured since last turn:\n```\n{preview}\n```"
                ))
            ));
        }
    }

    if crate::feature_gates::is_enabled(crate::feature_gates::FeatureGate::Harrier) {
        system_prompt.push_str(
            "\n\n## Investigate before asking\n\
                 When the user's request is concrete and bounded (a specific \
                 file, a named symbol, a known feature area), do a small targeted \
                 investigation **only if you would otherwise ask a clarifying \
                 question**. Prefer one CodeGraph query or one precise search, \
                 then act. Do not use this as permission for a broad Read/Grep/Glob \
                 survey before routine edits. Escalate to AskUserQuestion only \
                 when that targeted check surfaces multiple incompatible \
                 interpretations that would meaningfully change the plan.",
        );
    }

    if let Some(suffix) = crate::output_style::active_suffix(&doc_cwd) {
        system_prompt.push_str(&suffix);
        tracing::debug!(
            target: "jfc::stream",
            style = %crate::output_style::active().name(),
            "appended OutputStyle suffix to system prompt"
        );
    }

    let server_advisor_model = if matches!(
        provider.stream_convention(),
        StreamConvention::AnthropicNative
    ) && matches!(provider.name(), "anthropic" | "anthropic-oauth")
    {
        crate::advisor::active_server_advisor_model()
    } else {
        None
    };
    if let Some(model) = &server_advisor_model {
        tracing::info!(
            target: "jfc::advisor",
            advisor_model = %model,
            "injecting server advisor prompt/tool"
        );
        system_prompt.push_str("\n\n");
        system_prompt.push_str(crate::advisor::SERVER_ADVISOR_SYSTEM_PROMPT);
    }
    let local_advisor_model = crate::advisor::active_local_advisor_model();
    if let Some(model) = &local_advisor_model {
        tracing::info!(
            target: "jfc::advisor",
            advisor_model = %model,
            "injecting local advisor prompt"
        );
        system_prompt.push_str("\n\n## Local Advisor Tool\n\n");
        system_prompt.push_str(
            "You have access to an `Advisor` tool backed by JFC's configured \
                 local/client-side advisor model. It takes no parameters. When you \
                 call it, JFC snapshots the current conversation, sends that \
                 snapshot through the configured advisor provider/model, and returns \
                 the advisor's feedback as this tool's result. Call it before \
                 substantive work on multi-step tasks, when stuck, when considering \
                 a change of approach, and before declaring substantial work done.",
        );
    }
    RuntimePromptState {
        hcom_available: tools::hcom_available(),
        server_advisor_model,
        local_advisor_model,
    }
}

pub(super) fn append_turn_prompt_sections(
    system_prompt: &mut String,
    overrides: &StreamRequestOverrides,
    messages: &[ProviderMessage],
) {
    // Drain queued background reminders (file watcher / MCP refresh / …)
    // into this request's system prompt. The reminders were posted by
    // FS-event handlers and live wire-only — they never persist in
    // `app.engine.messages`, so re-issuing or compacting the conversation
    // doesn't re-show them. Each reminder is wrapped in the canonical
    // `<system-reminder>` envelope so the model treats it as background
    // context, not a user instruction.
    if !overrides.background_reminders.is_empty() {
        tracing::debug!(
            target: "jfc::stream",
            count = overrides.background_reminders.len(),
            "appending background reminders to system prompt"
        );
        for body in &overrides.background_reminders {
            system_prompt.push_str("\n\n");
            system_prompt.push_str(&crate::system_reminder::format(body));
        }
    }

    // Inject the last session's handoff summary so the model knows where
    // the previous session left off. Only on the first request per session
    // (handoff is static context).
    if messages.len() <= 1
        && let Some(root) = crate::context::discover_git_root()
        && let Some(handoff) = crate::sprint::HandoffSummary::read_latest(&root)
    {
        let truncated: String = handoff.chars().take(4000).collect();
        system_prompt.push_str("\n\n## Previous Session Handoff\n");
        system_prompt.push_str(&truncated);
    }

    // Temporal awareness is now fully implemented in
    // stream/messages/provider_messages.rs — time gap markers (<!-- +Nm -->)
    // are prepended to user messages when the gap exceeds 1 minute.

    let system_prompt_tokens_before_total_reminder = system_prompt.len() / 4;
    let total_tokens_reminder_mode = overrides
        .total_tokens_reminder_mode
        .unwrap_or_else(crate::total_tokens_reminder::active_mode);
    if let Some(reminder) = crate::total_tokens_reminder::render_for_request_with_mode(
        total_tokens_reminder_mode,
        messages,
        system_prompt_tokens_before_total_reminder,
        overrides.last_usage_input_tokens,
        overrides.context_window_tokens,
    ) {
        system_prompt.push_str("\n\n");
        system_prompt.push_str(&reminder);
    }
}

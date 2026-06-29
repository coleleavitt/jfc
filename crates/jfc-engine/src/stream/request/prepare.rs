use std::sync::Arc;

use crate::context_accounting::RequestContextPressure;
use crate::runtime::{StreamRequestMetadata, StreamRequestOverrides};
use crate::tools;
use jfc_provider::{
    DEFAULT_MAX_OUTPUT_TOKENS, ModelId, ModelRequestPolicy, ModelRequestProfile,
    ModelResolutionReason, ModelSpec, Provider, ProviderId, ProviderMessage, ResolvedModel,
    StreamConvention, StreamOptions,
};

use super::PreparedStreamRequest;
use super::behavior_prompt::resolve_behavioral_prompt_state;
use super::budget::stream_context_budget;
use super::project_context::append_project_context;
use super::prompt_seed::{PromptSeed, build_prompt_seed};
use super::rsi_runtime::append_active_rsi_prompt_sections;
use super::runtime_prompt::append_runtime_prompt_sections;
use super::thinking::{enforce_thinking_budget_fits_max_tokens, requested_thinking_display};
use super::tool_catalog::prepare_advertised_tools;
use super::tools::anthropic_tool_choice_value;

pub async fn prepare_stream_request(
    provider: Arc<dyn Provider>,
    messages: &[ProviderMessage],
    model: &ModelId,
    overrides: StreamRequestOverrides,
) -> PreparedStreamRequest {
    let PromptSeed {
        mut system_prompt,
        skills_chars,
        dispatch_chars,
        diagnostics_chars,
    } = build_prompt_seed().await;
    let mut overrides = overrides;
    let project_context = append_project_context(
        &mut system_prompt,
        &mut overrides,
        &provider,
        messages,
        model,
    )
    .await;
    let behavior_prompt = resolve_behavioral_prompt_state(&overrides, model);
    let rsi_runtime = append_active_rsi_prompt_sections(&mut system_prompt).await;
    let runtime_prompt = append_runtime_prompt_sections(
        &mut system_prompt,
        &provider,
        messages,
        &overrides,
        behavior_prompt,
    )
    .await;

    let provider_name = provider.name().to_owned();
    let selected_model_info = provider
        .available_models()
        .into_iter()
        .find(|info| info.id == *model);
    let model_profile = ModelRequestProfile::from_provider_model(
        &provider_name,
        model.as_str(),
        selected_model_info
            .as_ref()
            .and_then(|info| info.context_window_tokens),
        selected_model_info
            .as_ref()
            .and_then(|info| info.max_output_tokens),
    );
    let thinking_mode = model_profile.thinking_mode();
    tracing::debug!(
        target: "jfc::stream::budget",
        skills_chars,
        dispatch_chars,
        diagnostics_chars,
        rsi_prompt_sections = rsi_runtime.prompt_sections,
        rsi_tool_visibility_rules = rsi_runtime.tool_visibility_rules,
        total_system_chars = system_prompt.len(),
        estimated_tokens = system_prompt.len() / 4,
        "system prompt budget breakdown"
    );
    tracing::info!(
        target: "jfc::stream",
        model = %model,
        has_thinking_support = thinking_mode.has_thinking_support(),
        supports_adaptive = thinking_mode.supports_adaptive(),
        system_prompt_len = system_prompt.len(),
        tool_count = tools::model_tool_defs().len(),
        "preparing stream request"
    );
    let max_out = model_profile
        .max_output_tokens()
        .unwrap_or(DEFAULT_MAX_OUTPUT_TOKENS);
    let tool_catalog = prepare_advertised_tools(
        &mut system_prompt,
        messages,
        &overrides,
        runtime_prompt.hcom_available,
        runtime_prompt.local_advisor_model.is_none(),
        behavior_prompt.effective_brief_mode,
        behavior_prompt.pewter_owl_tool,
    )
    .await;
    let advertised_tool_count = tool_catalog.advertised_tool_count;
    let action_expected = tool_catalog.action_expected;
    let advertised_tools = tool_catalog.tools;
    let request_budget = stream_context_budget(
        &system_prompt,
        &advertised_tools,
        project_context.memory_context_chars,
        project_context.project_instructions_chars,
        messages,
    );
    tracing::debug!(
        target: "jfc::stream::budget",
        system_prompt_tokens = request_budget.system_prompt_tokens,
        tool_definition_tokens = request_budget.tool_definition_tokens,
        memory_tokens = request_budget.memory_tokens,
        project_instructions_tokens = request_budget.project_instructions_tokens,
        replay_message_tokens = request_budget.user_message_tokens,
        advertised_tool_count,
        "proof-backed stream context budget estimate"
    );

    let mut base = StreamOptions::new(model.clone())
        .system(system_prompt)
        .tools(advertised_tools)
        .max_tokens(max_out);
    if matches!(
        provider.stream_convention(),
        StreamConvention::AnthropicNative
    ) && !base.tools.is_empty()
    {
        base.provider_options.insert(
            "tool_choice".to_owned(),
            anthropic_tool_choice_value(overrides.tool_choice),
        );
    }
    if let Some(advisor_model) = runtime_prompt.server_advisor_model {
        base = base.advisor_model(advisor_model);
    }
    if crate::effort::active_fast_mode() {
        base = base.fast_mode(true);
    }
    if behavior_prompt.pewter_owl_header {
        base = base.narration_summaries(true);
    }
    let thinking_display = requested_thinking_display(&overrides);
    if !overrides.custom_betas.is_empty() {
        base = base.custom_betas(overrides.custom_betas);
    }
    if overrides.fine_grained_tool_streaming
        || std::env::var("JFC_FINE_GRAINED_TOOL_STREAMING")
            .map(|v| matches!(v.as_str(), "1" | "true" | "yes" | "on"))
            .unwrap_or(false)
    {
        base = base.eager_input_streaming(true);
    }
    if overrides.strict_tool_schemas
        || std::env::var("JFC_STRICT_TOOL_SCHEMAS")
            .map(|v| matches!(v.as_str(), "1" | "true" | "yes" | "on"))
            .unwrap_or(false)
    {
        base = base.strict_tool_schemas(true);
    }
    if let Some(tokens) = overrides.task_budget {
        base = base.task_budget(tokens);
    }
    // Forward the post-compaction savings hint so the API's context-management
    // assist (context-hint-2026-04-09) knows how much we just freed. The body
    // builder gates on a >=20k floor (matching cli.js's `2e4`), so a trivial
    // compaction won't emit the hint.
    base.context_hint_tokens_saved = overrides.context_hint_tokens_saved;

    // Server-side compaction (the non-blocking primary path): let Anthropic
    // reduce old turns API-side before they reach the model, so the user's
    // input never stalls behind a local summarization round-trip. Defaults on
    // in StreamOptions::new; cleared only when the user disables auto-compaction
    // or opts out of the server path specifically.
    {
        let cfg = crate::config::load_arc();
        base.server_side_compaction =
            cfg.auto_compact_enabled && cfg.server_side_compaction_enabled;
    }

    let mut opts = thinking_mode.apply_to(base);
    opts = crate::exploration::apply_to_stream_options(
        opts,
        &model_profile,
        provider.name(),
        provider.stream_convention(),
    );
    opts = model_profile.clamp_options(opts);
    let request_pressure = RequestContextPressure::new(
        request_budget,
        effective_context_window_tokens(
            model_profile.context_window_tokens(),
            overrides
                .context_window_tokens
                .and_then(|tokens| usize::try_from(tokens).ok()),
        ),
        usize::try_from(opts.max_tokens).ok(),
    );
    tracing::debug!(
        target: "jfc::stream::budget",
        raw_tokens = request_pressure.raw_tokens,
        effective_tokens = request_pressure.effective_tokens,
        window_tokens = ?request_pressure.window_tokens,
        max_output_tokens = ?request_pressure.max_output_tokens,
        "resolved request context pressure"
    );
    let context_pressure_nudge = request_pressure.context_pressure_nudge();
    if let Some(nudge) = context_pressure_nudge {
        tracing::warn!(
            target: "jfc::stream::ctx_reduce",
            nudge = nudge.kind.label(),
            level = ?nudge.level,
            raw_tokens = nudge.raw_tokens,
            effective_tokens = nudge.effective_tokens,
            window_tokens = nudge.window_tokens,
            threshold_tokens = nudge.threshold_tokens,
            reclaim_floor_tokens = nudge.reclaim_floor_tokens,
            "request context pressure crossed ctx_reduce nudge threshold"
        );
    }
    // Log the resolved request params after per-model clamping so every spawn's
    // actual reasoning_effort, max_tokens, and thinking mode are observable.
    // Critical for experiments comparing model tiers / effort levels — the
    // post-clamp values are what the model actually sees, not the input strings.
    tracing::debug!(
        target: "jfc::stream",
        model = %model,
        reasoning_effort = ?opts.reasoning_effort,
        max_tokens = opts.max_tokens,
        adaptive_thinking = opts.adaptive_thinking,
        thinking_budget = ?opts.thinking_budget,
        "resolved request after clamp_options"
    );
    if let Some(max) = overrides.max_thinking_tokens
        && let Some(budget) = opts.thinking_budget.as_mut()
    {
        *budget = (*budget).min(max);
    }
    enforce_thinking_budget_fits_max_tokens(&mut opts);
    if opts.adaptive_thinking || opts.thinking_budget.is_some() {
        let display = thinking_display.unwrap_or_else(|| "summarized".into());
        opts = opts.thinking_display(display);
        // Request server-authoritative thinking token estimates so the spinner
        // can show a live thinking-token chip. Only meaningful when thinking is
        // active; the API otherwise streams thinking_delta without estimates.
        opts = opts.thinking_token_count(true);
    }

    PreparedStreamRequest {
        opts,
        context_pressure: request_pressure,
        system_prompt_tokens: request_pressure.overhead_tokens,
        metadata: StreamRequestMetadata {
            advertised_tool_count,
            action_expected,
            tool_choice: overrides.tool_choice,
            resolved_model: Some(ResolvedModel::new(
                ModelSpec::qualified(ProviderId::new(provider.name()), model.clone()),
                ModelSpec::qualified(ProviderId::new(provider.name()), model.clone()),
                ModelResolutionReason::Requested,
                selected_model_info.as_ref(),
            )),
            context_budget: Some(request_budget),
            context_pressure_nudge,
            provider_history_archive_recall_ids: project_context
                .provider_history_archive_recall_ids,
            rsi_prompt_sections: rsi_runtime.prompt_sections,
            rsi_tool_visibility_rules: rsi_runtime.tool_visibility_rules,
        },
        recalled_memory_chars: project_context.fresh_recall_chars,
    }
}

fn effective_context_window_tokens(
    model_profile_tokens: Option<usize>,
    override_tokens: Option<usize>,
) -> Option<usize> {
    match (model_profile_tokens, override_tokens) {
        (Some(profile), Some(override_tokens)) => Some(profile.min(override_tokens)),
        (Some(profile), None) => Some(profile),
        (None, Some(override_tokens)) => Some(override_tokens),
        (None, None) => None,
    }
}

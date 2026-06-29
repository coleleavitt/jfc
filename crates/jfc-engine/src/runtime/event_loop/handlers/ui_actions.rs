//! Engine-side action handlers (plan-mode transitions, notices, stream
//! metadata). The frontend submit pipeline stayed with the TUI.

use crate::app::EngineState;
use crate::toast;

/// Handle `FrontendEvent::PlanModeEntered { reason }`.
pub fn handle_enter_plan_mode(state: &mut EngineState, reason: String) {
    // Model-callable plan mode entry — the EnterPlanMode tool
    // emits this. Flip the leader's permission mode and toast
    // the reason so the user knows what triggered it.
    state.permission_mode = crate::app::PermissionMode::Plan;
    let preview: String = reason.chars().take(120).collect();
    let body = if preview.is_empty() {
        "Entered plan mode (model request)".to_owned()
    } else {
        format!("Plan mode: {preview}")
    };
    toast::push_with_cap(
        &mut state.toasts,
        toast::Toast::new(toast::ToastKind::Info, body),
    );
    // v132 mid-stream system-reminder so the next turn
    // sees the mode flip explicitly. Without this the
    // model only learns about the new permissions when
    // a tool call gets denied — too late.
    crate::system_reminder::append_to_last_user(
        &mut state.messages,
        "Permission mode is now `Plan` (read-only). Use ExitPlanMode \
         with a finalized plan to proceed with edits.",
    );
}

/// Handle `FrontendEvent::GoalSet { condition }` — the model-invocable `SetGoal`
/// tool. Sets (or clears) the session stop-condition the same way `/goal` does,
/// so the goal loop drives the agent until an evaluator says the condition is
/// met. Reuses [`crate::goal`] validation + sidecar persistence so the model
/// path and the user `/goal` path can't diverge.
pub fn handle_set_goal(state: &mut EngineState, condition: String) {
    let arg = condition.trim();
    if arg.is_empty() || crate::goal::is_clear_arg(arg) {
        let prev = state.goal.take();
        crate::runtime::cancel_goal_evaluator(state);
        if let Some(sid) = state.current_session_id.as_ref() {
            crate::goal::save_sidecar(sid.as_str(), None);
        }
        let body = match prev {
            Some(g) => format!("Goal cleared (model) after {} iterations.", g.iterations),
            None => "No goal was set.".to_owned(),
        };
        toast::push_with_cap(
            &mut state.toasts,
            toast::Toast::new(toast::ToastKind::Info, body),
        );
        return;
    }
    match crate::goal::validate_condition(arg) {
        Ok(condition) => {
            state.goal = Some(crate::goal::ActiveGoal::new(condition.clone()));
            crate::runtime::cancel_goal_evaluator(state);
            if let Some(sid) = state.current_session_id.as_ref() {
                crate::goal::save_sidecar(sid.as_str(), state.goal.as_ref());
            }
            let preview: String = condition.chars().take(80).collect();
            toast::push_with_cap(
                &mut state.toasts,
                toast::Toast::new(toast::ToastKind::Success, format!("Goal set: {preview}")),
            );
            // Mid-stream reminder so the next turn sees the new stop-condition
            // explicitly (mirrors the plan-mode reminder pattern).
            crate::system_reminder::append_to_last_user(
                &mut state.messages,
                &format!(
                    "Session goal is now active: \"{condition}\". Keep working until \
                     it is met; it is auto-evaluated after each turn (max {} iterations). \
                     Call SetGoal with an empty/`clear` condition to cancel.",
                    crate::goal::MAX_ITERATIONS
                ),
            );
        }
        Err(reason) => {
            toast::push_with_cap(
                &mut state.toasts,
                toast::Toast::new(
                    toast::ToastKind::Error,
                    format!("SetGoal rejected: {reason}"),
                ),
            );
        }
    }
}

/// Handle `StreamEvent::SystemPromptLen(len)`.
pub fn handle_system_prompt_len(state: &mut EngineState, len: usize) {
    state.last_system_prompt_len = Some(len);
}

/// Handle `StreamEvent::RequestMetadata(meta)`.
pub fn handle_request_metadata(
    state: &mut EngineState,
    meta: crate::runtime::StreamRequestMetadata,
) {
    state.record_stream_activity();
    tracing::debug!(
        target: "jfc::stream",
        advertised_tool_count = meta.advertised_tool_count,
        action_expected = meta.action_expected,
        tool_choice = ?meta.tool_choice,
        provider_history_archive_recall_count = meta.provider_history_archive_recall_ids.len(),
        "stream request metadata"
    );
    if let Some(nudge) = meta.context_pressure_nudge {
        tracing::warn!(
            target: "jfc::stream::ctx_reduce",
            nudge = nudge.kind.label(),
            level = ?nudge.level,
            raw_tokens = nudge.raw_tokens,
            effective_tokens = nudge.effective_tokens,
            window_tokens = nudge.window_tokens,
            threshold_tokens = nudge.threshold_tokens,
            reclaim_floor_tokens = nudge.reclaim_floor_tokens,
            "stream request metadata carried ctx_reduce pressure nudge"
        );
        if let Some(reduction) = crate::context_reduction::queue_pressure_reduction(state, nudge) {
            tracing::warn!(
                target: "jfc::stream::ctx_reduce",
                nudge = nudge.kind.label(),
                queued_tags = reduction.queued_tags,
                queued_ranges = reduction.queued_ranges,
                estimated_reclaim_tokens = reduction.estimated_reclaim_tokens,
                "queued automatic ctx_reduce pressure drops for next cache-safe drain"
            );
            crate::runtime::session_save::request_save(state);
        }
    }
    let previous_seen_count = state.provider_history_archive_seen.len();
    state
        .provider_history_archive_seen
        .extend(meta.provider_history_archive_recall_ids.iter().cloned());
    if state.provider_history_archive_seen.len() != previous_seen_count
        && let Some(session_id) = state.current_session_id.as_ref()
    {
        let session_id = session_id.as_str().to_owned();
        if let Err(err) = crate::context_accounting::persist_session_provider_history_archive_seen(
            &session_id,
            &state.provider_history_archive_seen,
        ) {
            tracing::warn!(
                target: "jfc::stream::provider_history",
                session_id,
                error = %err,
                "failed to persist provider-history archive recall ledger"
            );
        }
    }
    if let Some(budget) = meta.context_budget {
        state.last_context_budget = Some(budget);
    }
    state.current_stream_request = Some(meta);
}

/// Handle `StreamEvent::Lifecycle(status)`.
pub fn handle_stream_lifecycle(
    state: &mut EngineState,
    status: crate::runtime::StreamLifecycleStatus,
) {
    state.record_stream_activity();
    tracing::debug!(
        target: "jfc::stream::lifecycle",
        phase = ?status.phase,
        detail = ?status.detail,
        "stream lifecycle status"
    );
    state.stream_lifecycle = Some(status);
}

/// Handle `ControlEvent::Notice { kind, text }`.
pub fn handle_toast(state: &mut EngineState, kind: toast::ToastKind, text: impl Into<String>) {
    // Push onto the auto-expiring strip with the kind's
    // default TTL. Capped at `MAX_TOASTS` to bound memory
    // when a long-running compaction or classifier spams.
    toast::push_with_cap(&mut state.toasts, toast::Toast::new(kind, text));
}

/// Handle `FrontendEvent::PlanReview { plan }`.
pub fn handle_exit_plan_mode(state: &mut EngineState, plan: String) {
    // Surface the plan as part of the existing assistant message
    // (NOT a new message — that would fool should_continue_loop
    // into thinking the last assistant has no tools, blocking
    // the agentic continuation). Append the plan body to the
    // current streaming assistant message so the turn can
    // continue after tool completion.
    tracing::info!(
        target: "jfc::ui::plan_mode",
        plan_bytes = plan.len(),
        from_mode = ?state.permission_mode,
        "ExitPlanMode: surfacing plan + transitioning out of Plan"
    );
    let body = format!("\n\n**Plan presented (Plan Mode → Accept Edits)**\n\n---\n\n{plan}");
    // Append to the current streaming assistant message if we
    // have one; otherwise fall back to the last assistant msg.
    // Only accept `streaming_assistant_idx` if it still points
    // at an Assistant — otherwise fall through to the
    // rposition scan. Prevents a stale index (Up-recall
    // shifted the slot left onto a user placeholder) from
    // gluing the plan body onto a User message.
    let streaming_assistant = state.streaming_assistant_idx.filter(|&idx| {
        matches!(
            state.messages.get(idx).map(|m| m.role),
            Some(crate::types::Role::Assistant),
        )
    });
    let target_idx = streaming_assistant.or_else(|| {
        state
            .messages
            .iter()
            .rposition(|m| m.role == crate::types::Role::Assistant)
    });
    if let Some(idx) = target_idx {
        // Append as a new Text part to the existing assistant msg.
        state.messages[idx]
            .parts
            .push(crate::types::MessagePart::Text(body));
    } else {
        // Fallback: no assistant message found (shouldn't happen
        // but defensive). Push as a new message.
        state
            .messages
            .push(crate::types::ChatMessage::assistant(body));
    }
    if matches!(state.permission_mode, crate::app::PermissionMode::Plan) {
        state.permission_mode = crate::app::PermissionMode::AcceptEdits;
        crate::toast::push_with_cap(
            &mut state.toasts,
            crate::toast::Toast::new(
                crate::toast::ToastKind::Success,
                "Plan approved — mode: Accept Edits",
            ),
        );
        crate::system_reminder::append_to_last_user(
            &mut state.messages,
            "Permission mode flipped from `Plan` to `AcceptEdits`. \
             Edit/Write/Bash now auto-approve. Continue executing the plan.",
        );
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use jfc_provider::{EventStream, ModelInfo, Provider, ProviderMessage, StreamOptions};

    use super::*;

    struct TestProvider;

    #[async_trait::async_trait]
    impl Provider for TestProvider {
        fn name(&self) -> &str {
            "test"
        }

        fn available_models(&self) -> Vec<ModelInfo> {
            Vec::new()
        }

        async fn stream(
            &self,
            _messages: Vec<ProviderMessage>,
            _options: &StreamOptions,
        ) -> anyhow::Result<EventStream> {
            Ok(Box::pin(futures::stream::empty()))
        }
    }

    impl jfc_provider::seal::Sealed for TestProvider {}

    #[test]
    fn request_metadata_marks_provider_history_archives_seen_regression() {
        let mut state = EngineState::new(Arc::new(TestProvider), "test-model");
        let meta = crate::runtime::StreamRequestMetadata {
            advertised_tool_count: 1,
            action_expected: false,
            tool_choice: crate::runtime::StreamToolChoice::Auto,
            resolved_model: None,
            context_budget: Some(jfc_core::context_budget::ContextBudget {
                system_prompt_tokens: 10,
                tool_definition_tokens: 20,
                memory_tokens: 30,
                project_instructions_tokens: 40,
                user_message_tokens: 50,
            }),
            context_pressure_nudge: None,
            provider_history_archive_recall_ids: vec!["provider-history-1".to_owned()],
            rsi_prompt_sections: 0,
            rsi_tool_visibility_rules: 0,
        };

        handle_request_metadata(&mut state, meta.clone());

        assert!(
            state
                .provider_history_archive_seen
                .contains("provider-history-1")
        );
        assert_eq!(state.last_context_budget, meta.context_budget);
        assert_eq!(state.current_stream_request, Some(meta));
    }
}

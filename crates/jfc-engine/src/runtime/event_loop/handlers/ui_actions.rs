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


/// Handle `StreamEvent::SystemPromptLen(len)`.
pub fn handle_system_prompt_len(state: &mut EngineState, len: usize) {
    state.last_system_prompt_len = Some(len);
}


/// Handle `StreamEvent::RequestMetadata(meta)`.
pub fn handle_request_metadata(state: &mut EngineState, meta: crate::runtime::StreamRequestMetadata) {
    state.record_stream_activity();
    tracing::debug!(
        target: "jfc::stream",
        advertised_tool_count = meta.advertised_tool_count,
        action_expected = meta.action_expected,
        tool_choice = ?meta.tool_choice,
        "stream request metadata"
    );
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
        state.messages
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
        state.messages
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


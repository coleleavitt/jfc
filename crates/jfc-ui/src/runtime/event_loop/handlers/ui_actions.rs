//! UI-action handlers — plan mode entry/exit, submit, toast, load session,
//! and stream metadata.

use crate::app::{self, App};
use crate::runtime::EventSender;
use crate::{input, toast};

/// Handle `UiEvent::EnterPlanModeRequested { reason }`.
pub(crate) fn handle_enter_plan_mode(app: &mut App, reason: String) {
    // Model-callable plan mode entry — the EnterPlanMode tool
    // emits this. Flip the leader's permission mode and toast
    // the reason so the user knows what triggered it.
    app.permission_mode = crate::app::PermissionMode::Plan;
    let preview: String = reason.chars().take(120).collect();
    let body = if preview.is_empty() {
        "Entered plan mode (model request)".to_owned()
    } else {
        format!("Plan mode: {preview}")
    };
    toast::push_with_cap(
        &mut app.toasts,
        toast::Toast::new(toast::ToastKind::Info, body),
    );
    // v132 mid-stream system-reminder so the next turn
    // sees the mode flip explicitly. Without this the
    // model only learns about the new permissions when
    // a tool call gets denied — too late.
    crate::system_reminder::append_to_last_user(
        &mut app.messages,
        "Permission mode is now `Plan` (read-only). Use ExitPlanMode \
         with a finalized plan to proceed with edits.",
    );
}

/// Handle `UiEvent::Submit(text)`.
pub(crate) async fn handle_submit(
    app: &mut App,
    text: String,
    tx: &EventSender,
) -> anyhow::Result<()> {
    app.last_user_activity_at = std::time::Instant::now();
    app.idle_return_shown = false;
    // Re-fire after pre-submit compaction. Reuses the same
    // dispatch path as a typed prompt so message persistence,
    // streaming setup, and session save all run identically.
    tracing::debug!(
        target: "jfc::input",
        text_len = text.len(),
        "UiEvent::Submit (re-queued after compaction)"
    );
    input::handle_submit_text(app, text, tx).await?;
    Ok(())
}

/// Handle `StreamEvent::SystemPromptLen(len)`.
pub(crate) fn handle_system_prompt_len(app: &mut App, len: usize) {
    app.last_system_prompt_len = Some(len);
}

/// Handle `StreamEvent::RequestMetadata(meta)`.
pub(crate) fn handle_request_metadata(app: &mut App, meta: crate::runtime::StreamRequestMetadata) {
    app.record_stream_activity();
    tracing::debug!(
        target: "jfc::stream",
        advertised_tool_count = meta.advertised_tool_count,
        action_expected = meta.action_expected,
        tool_choice = ?meta.tool_choice,
        "stream request metadata"
    );
    app.current_stream_request = Some(meta);
}

/// Handle `UiEvent::Toast { kind, text }`.
pub(crate) fn handle_toast(app: &mut App, kind: toast::ToastKind, text: impl Into<String>) {
    // Push onto the auto-expiring strip with the kind's
    // default TTL. Capped at `MAX_TOASTS` to bound memory
    // when a long-running compaction or classifier spams.
    toast::push_with_cap(&mut app.toasts, toast::Toast::new(kind, text));
}

/// Handle `UiEvent::LoadSession(session_id)`.
pub(crate) async fn handle_load_session(app: &mut App, session_id: crate::ids::SessionId) {
    // Session-picker selected. Reuse the same loader the
    // sidebar's Enter handler calls so we share the
    // cwd-refresh + title-update + scroll-to-bottom
    // semantics. Errors land as a toast so the user
    // doesn't lose the picker context.
    tracing::info!(
        target: "jfc::session_picker",
        session_id = %session_id,
        "LoadSession event received, fetching messages"
    );
    match crate::session::load_session(&session_id).await {
        Some(messages) => {
            app.messages = messages;
            let id_for_toast = session_id.clone();
            app.switch_session(Some(session_id));
            app.streaming_text.clear();
            app.streaming_reasoning.clear();
            app.streaming_response_bytes = 0;
            app.streaming_assistant_idx = None;
            app.scroll_to_bottom();
            toast::push_with_cap(
                &mut app.toasts,
                toast::Toast::new(
                    toast::ToastKind::Success,
                    format!("Loaded session {id_for_toast}"),
                ),
            );
        }
        None => {
            toast::push_with_cap(
                &mut app.toasts,
                toast::Toast::new(
                    toast::ToastKind::Error,
                    format!("Failed to load session {session_id}"),
                ),
            );
        }
    }
}

/// Handle `UiEvent::ExitPlanModeRequested { plan }`.
pub(crate) fn handle_exit_plan_mode(app: &mut App, plan: String) {
    // Surface the plan as part of the existing assistant message
    // (NOT a new message — that would fool should_continue_loop
    // into thinking the last assistant has no tools, blocking
    // the agentic continuation). Append the plan body to the
    // current streaming assistant message so the turn can
    // continue after tool completion.
    tracing::info!(
        target: "jfc::ui::plan_mode",
        plan_bytes = plan.len(),
        from_mode = ?app.permission_mode,
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
    let streaming_assistant = app.streaming_assistant_idx.filter(|&idx| {
        matches!(
            app.messages.get(idx).map(|m| m.role),
            Some(crate::types::Role::Assistant),
        )
    });
    let target_idx = streaming_assistant.or_else(|| {
        app.messages
            .iter()
            .rposition(|m| m.role == crate::types::Role::Assistant)
    });
    if let Some(idx) = target_idx {
        // Append as a new Text part to the existing assistant msg.
        app.messages[idx]
            .parts
            .push(crate::types::MessagePart::Text(body));
    } else {
        // Fallback: no assistant message found (shouldn't happen
        // but defensive). Push as a new message.
        app.messages
            .push(crate::types::ChatMessage::assistant(body));
    }
    if matches!(app.permission_mode, app::PermissionMode::Plan) {
        app.permission_mode = app::PermissionMode::AcceptEdits;
        crate::toast::push_with_cap(
            &mut app.toasts,
            crate::toast::Toast::new(
                crate::toast::ToastKind::Success,
                "Plan approved — mode: Accept Edits",
            ),
        );
        crate::system_reminder::append_to_last_user(
            &mut app.messages,
            "Permission mode flipped from `Plan` to `AcceptEdits`. \
             Edit/Write/Bash now auto-approve. Continue executing the plan.",
        );
    }
}

/// Handle `UiEvent::AdvisorToolRequested { tool_use_id }`.
///
/// The model called the `Advisor` tool. Snapshot the transcript, run
/// `ask_advisor()` against the lazy session, and append the reply as
/// a `MessagePart::Advisor` on the current assistant message so the
/// user sees it inline and the model sees it on the next turn.
pub(crate) async fn handle_advisor_tool_requested(app: &mut App, _tool_use_id: String) {
    if !app.advisor_enabled {
        app.messages
            .push(crate::types::ChatMessage::assistant_parts(vec![
                crate::types::MessagePart::Advisor(
                    "Advisor mode is disabled. Set `JFC_ADVISOR_ENABLED=1` and \
                     restart jfc to enable Advisor tool calls."
                        .into(),
                ),
            ]));
        return;
    }
    let session = app
        .advisor_session
        .get_or_insert_with(|| crate::advisor::AdvisorSession::new(app.model.clone()));
    let snapshot = app.messages.clone();
    let provider = std::sync::Arc::clone(&app.provider);
    // The Advisor tool takes no parameters — the "query" is implicit:
    // "look at this conversation and tell me what I'm missing."
    let auto_query = "Review my conversation so far. Flag anything I'm missing, \
                      any assumption I should verify, and any risk I'm overlooking. \
                      Be specific and terse.";
    match crate::advisor::ask_advisor(
        provider.as_ref(),
        session,
        auto_query.to_string(),
        &snapshot,
    )
    .await
    {
        Ok(reply) => {
            let remaining = session.tokens_remaining();
            let total_budget = session.token_budget;
            app.messages
                .push(crate::types::ChatMessage::assistant_parts(vec![
                    crate::types::MessagePart::Advisor(format!(
                        "{reply}\n\n_(advisor budget: {remaining} of {total_budget} tokens remaining)_"
                    )),
                ]));
        }
        Err(e) => {
            app.messages
                .push(crate::types::ChatMessage::assistant_parts(vec![
                    crate::types::MessagePart::Advisor(format!(
                        "Advisor error: {e}\n\nUse `/clear` to start a fresh session if the budget is exhausted."
                    )),
                ]));
        }
    }
}

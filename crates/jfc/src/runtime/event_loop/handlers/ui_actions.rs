//! UI-action handlers — plan mode entry/exit, submit, toast, load session,
//! and stream metadata.

use crate::app::{self, App};
use crate::runtime::EventSender;
use crate::{input, toast};

/// Handle `FrontendEvent::PlanModeEntered { reason }`.
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

/// Handle `ControlEvent::SubmitPrompt(text)`.
pub(crate) async fn handle_submit(
    app: &mut App,
    text: String,
    tx: &EventSender,
) -> anyhow::Result<()> {
    // A fresh submit dismisses any lingering away-recap from the previous
    // idle period (the away block below may immediately set a new one).
    app.away_recap = None;
    // Away-recap: if the agent worked autonomously for > 5 min while the
    // user was gone, surface an honest user-facing banner of what happened
    // ("Tools: …, Files: …, Errors: N") at the top of the transcript. This
    // is for the *user* to re-orient on return — it is not injected into the
    // model's context (the model already has the full transcript).
    let away = app.last_user_activity_at.elapsed();
    if away >= crate::session_recap::AWAY_THRESHOLD && !app.idle_return_shown {
        use crate::types::{MessagePart, Role, ToolInput};
        let start_idx = app.interaction_message_idx.min(app.messages.len());
        let since: Vec<crate::session_recap::RecapMessage> =
            app.messages[start_idx..]
                .iter()
                .map(|m| {
                    let text_preview = m
                        .parts
                        .iter()
                        .find_map(|p| match p {
                            MessagePart::Text(t) if !t.is_empty() => {
                                Some(t.chars().take(160).collect::<String>())
                            }
                            _ => None,
                        })
                        .unwrap_or_default();
                    let tool_calls: Vec<String> = m
                        .parts
                        .iter()
                        .filter_map(|p| match p {
                            MessagePart::Tool(t) => Some(t.kind.label().to_string()),
                            _ => None,
                        })
                        .collect();
                    // Surface which files the agent actually touched (Edit/Write/
                    // MultiEdit/NotebookEdit carry a `file_path`).
                    let files_changed: Vec<String> = m
                        .parts
                        .iter()
                        .filter_map(|p| match p {
                            MessagePart::Tool(t) => match &t.input {
                                ToolInput::Edit { file_path, .. }
                                | ToolInput::Write { file_path, .. } => Some(file_path.clone()),
                                _ => None,
                            },
                            _ => None,
                        })
                        .collect();
                    let had_error = m.parts.iter().any(|p| matches!(p,
                    MessagePart::Tool(t) if t.status == crate::types::ExecutionStatus::Failed
                ));
                    crate::session_recap::RecapMessage {
                        is_assistant: m.role == Role::Assistant,
                        tool_calls,
                        had_error,
                        files_changed,
                        text_preview,
                    }
                })
                .collect();
        if let Some(recap) = crate::session_recap::generate_recap(&since) {
            app.away_recap = Some(format!(
                "{recap}\n(away {}m · Esc to dismiss)",
                away.as_secs() / 60
            ));
        }
        app.idle_return_shown = true;
    }
    // Update interaction tracking for next away-detection.
    app.last_user_activity_at = std::time::Instant::now();
    app.interaction_message_idx = app.messages.len();
    app.idle_return_shown = false;
    // Re-fire after pre-submit compaction. Reuses the same
    // dispatch path as a typed prompt so message persistence,
    // streaming setup, and session save all run identically.
    tracing::debug!(
        target: "jfc::input",
        text_len = text.len(),
        "ControlEvent::SubmitPrompt (re-queued after compaction)"
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

/// Handle `StreamEvent::Lifecycle(status)`.
pub(crate) fn handle_stream_lifecycle(
    app: &mut App,
    status: crate::runtime::StreamLifecycleStatus,
) {
    app.record_stream_activity();
    tracing::debug!(
        target: "jfc::stream::lifecycle",
        phase = ?status.phase,
        detail = ?status.detail,
        "stream lifecycle status"
    );
    app.stream_lifecycle = Some(status);
}

/// Handle `ControlEvent::Notice { kind, text }`.
pub(crate) fn handle_toast(app: &mut App, kind: toast::ToastKind, text: impl Into<String>) {
    // Push onto the auto-expiring strip with the kind's
    // default TTL. Capped at `MAX_TOASTS` to bound memory
    // when a long-running compaction or classifier spams.
    toast::push_with_cap(&mut app.toasts, toast::Toast::new(kind, text));
}

/// Handle `ControlEvent::LoadSession(session_id)`.
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

/// Handle `FrontendEvent::PlanReview { plan }`.
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

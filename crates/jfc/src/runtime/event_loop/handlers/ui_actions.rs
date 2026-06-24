//! UI-action handlers — plan mode entry/exit, submit, toast, load session,
//! and stream metadata.

use crate::app::App;
use crate::input;
use crate::runtime::EventSender;
use jfc_engine::runtime::PromptSubmission;

/// Handle `ControlEvent::SubmitPrompt(text)`.
pub(crate) async fn handle_submit(
    app: &mut App,
    submission: PromptSubmission,
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
    if away >= jfc_engine::session_recap::AWAY_THRESHOLD && !app.idle_return_shown {
        use jfc_core::{MessagePart, Role, ToolInput};
        let start_idx = app.interaction_message_idx.min(app.engine.messages.len());
        let since: Vec<jfc_engine::session_recap::RecapMessage> = app.engine.messages[start_idx..]
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
                // Surface which files the agent actually touched.
                let files_changed: Vec<String> = m
                    .parts
                    .iter()
                    .filter_map(|p| match p {
                        MessagePart::Tool(t) => match &t.input {
                            ToolInput::Edit { file_path, .. }
                            | ToolInput::Write { file_path, .. }
                            | ToolInput::MultiEdit { file_path, .. } => Some(file_path.clone()),
                            ToolInput::NotebookEdit { path, .. } => Some(path.clone()),
                            _ => None,
                        },
                        _ => None,
                    })
                    .collect();
                let had_error = m.parts.iter().any(|p| {
                    matches!(p,
                        MessagePart::Tool(t) if t.status == jfc_core::ExecutionStatus::Failed
                    )
                });
                jfc_engine::session_recap::RecapMessage {
                    is_assistant: m.role == Role::Assistant,
                    tool_calls,
                    had_error,
                    files_changed,
                    text_preview,
                }
            })
            .collect();
        if let Some(recap) = jfc_engine::session_recap::generate_recap(&since) {
            app.away_recap = Some(format!(
                "{recap}\n(away {}m · Esc to dismiss)",
                away.as_secs() / 60
            ));
        }
        app.idle_return_shown = true;
    }
    // Update interaction tracking for next away-detection.
    app.last_user_activity_at = std::time::Instant::now();
    app.interaction_message_idx = app.engine.messages.len();
    app.idle_return_shown = false;
    // Re-fire after pre-submit compaction. Reuses the same
    // dispatch path as a typed prompt so message persistence,
    // streaming setup, and session save all run identically.
    tracing::debug!(
        target: "jfc::input",
        text_len = submission.text.len(),
        attachments = submission.attachments.len(),
        edit_at = ?submission.edit_at,
        "ControlEvent::SubmitPrompt (re-queued after compaction)"
    );
    if submission.attachments.is_empty() && submission.edit_at.is_none() {
        input::handle_submit_text(app, submission.text, tx).await?;
    } else {
        let _ = crate::runtime::ops::submit_prompt(
            &mut app.engine,
            tx,
            submission.text,
            submission.attachments,
            submission.edit_at,
        )
        .await?;
    }
    Ok(())
}

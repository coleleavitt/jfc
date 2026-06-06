//! UI-action handlers — plan mode entry/exit, submit, toast, load session,
//! and stream metadata.

use crate::app::App;
use crate::runtime::EventSender;
use crate::input;

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
        let start_idx = app.interaction_message_idx.min(app.engine.messages.len());
        let since: Vec<crate::session_recap::RecapMessage> =
            app.engine.messages[start_idx..]
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
    app.interaction_message_idx = app.engine.messages.len();
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


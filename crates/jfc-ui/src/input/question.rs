//! `AskUserQuestion` modal: parsing, key handling, and answer submission.
//!
//! Unlike tool *approval* (which gates a dispatch — see `input/approval.rs`),
//! a question *collects an answer that becomes the tool_result*. The model
//! emits an `AskUserQuestion` tool_use; `handle_stream_tool` diverts it into
//! `app.pending_question` instead of dispatching; this module renders the
//! interaction and, on submit, synthesizes a `ToolEvent::Result` for the
//! tool_use + an `AllComplete` so the existing result-recording and
//! agentic-loop-continuation machinery resumes the turn.

use crossterm::event::{self, KeyCode, KeyModifiers};
use tokio::sync::mpsc;

use crate::app::{App, AppEvent, PendingQuestion, QuestionOption};
use crate::runtime::{ExecutionResult, ToolEvent, send_critical};
use crate::types::{ToolCall, ToolInput};

/// Parse an `AskUserQuestion` tool call into a [`PendingQuestion`]. Returns
/// `None` when the input isn't an `AskUserQuestion` or has no usable options —
/// the caller falls back to recording a failed tool_result so the tool_use
/// stays paired.
pub(crate) fn build_pending_question(tool: &ToolCall) -> Option<PendingQuestion> {
    let ToolInput::AskUserQuestion {
        question,
        options,
        multi_select,
    } = &tool.input
    else {
        return None;
    };
    let parsed: Vec<QuestionOption> = options
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|o| {
                    let label = o.get("label").and_then(|v| v.as_str())?.to_owned();
                    let description = o
                        .get("description")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_owned();
                    let preview = o
                        .get("preview")
                        .and_then(|v| v.as_str())
                        .map(str::to_owned);
                    Some(QuestionOption {
                        label,
                        description,
                        preview,
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    if parsed.is_empty() {
        return None;
    }
    Some(PendingQuestion {
        tool_id: tool.id.clone(),
        question: question.clone(),
        // The single-question schema carries no `header` yet; left empty until
        // the questions[] contract migration adds it.
        header: String::new(),
        options: parsed,
        multi_select: *multi_select,
        selected: 0,
        chosen: std::collections::BTreeSet::new(),
        editing_other: false,
        other_text: String::new(),
    })
}

/// Whether a key event is a Ctrl-modified character `c`.
fn is_ctrl(key: &event::KeyEvent, c: char) -> bool {
    key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char(c)
}

/// Route a key to the active question modal. Returns `true` when a question is
/// pending (the key was consumed by the modal), mirroring
/// `handle_approval_key`'s contract so `handle_key` can short-circuit.
pub(super) fn handle_question_key(
    app: &mut App,
    key: event::KeyEvent,
    tx: &mpsc::Sender<AppEvent>,
) -> bool {
    if app.pending_question.is_none() {
        return false;
    }
    // Let Ctrl-C fall through to the global interrupt handler.
    if key.modifiers.contains(KeyModifiers::CONTROL)
        && matches!(key.code, KeyCode::Char('c') | KeyCode::Char('C'))
    {
        return false;
    }

    // Decided after the borrow on `pending` ends, so submit/decline can re-take
    // `app.pending_question` without a borrow conflict.
    enum Act {
        None,
        Submit,
        Decline,
    }
    let mut act = Act::None;

    {
        let pending = app.pending_question.as_mut().expect("checked above");
        let other_row = pending.other_row();

        if pending.editing_other {
            // Free-text capture for the "Other" row.
            match key.code {
                KeyCode::Backspace => {
                    pending.other_text.pop();
                }
                KeyCode::Esc => {
                    // Cancel text entry only — not the whole modal.
                    pending.editing_other = false;
                }
                KeyCode::Enter => {
                    if pending.multi_select {
                        // Commit the typed "Other" into the chosen set and
                        // return to navigation; the user submits separately.
                        if pending.other_text.trim().is_empty() {
                            pending.chosen.remove(&other_row);
                        } else {
                            pending.chosen.insert(other_row);
                        }
                        pending.editing_other = false;
                    } else if pending.can_submit() {
                        act = Act::Submit;
                    }
                }
                KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                    pending.other_text.push(c);
                }
                _ => {}
            }
        } else {
            match key.code {
                KeyCode::Up => {
                    pending.selected = pending.selected.saturating_sub(1);
                }
                KeyCode::Down => {
                    pending.selected = (pending.selected + 1).min(pending.row_count() - 1);
                }
                _ if is_ctrl(&key, 'p') => {
                    pending.selected = pending.selected.saturating_sub(1);
                }
                _ if is_ctrl(&key, 'n') => {
                    pending.selected = (pending.selected + 1).min(pending.row_count() - 1);
                }
                KeyCode::Char(' ') if pending.multi_select => {
                    if pending.on_other() {
                        // Toggling "Other" means typing into it.
                        pending.editing_other = true;
                    } else if pending.chosen.contains(&pending.selected) {
                        pending.chosen.remove(&pending.selected);
                    } else {
                        pending.chosen.insert(pending.selected);
                    }
                }
                KeyCode::Enter => {
                    if pending.on_other() && pending.other_text.trim().is_empty() {
                        // Focus the free-text input before it can be submitted.
                        pending.editing_other = true;
                    } else if pending.multi_select {
                        if pending.can_submit() {
                            act = Act::Submit;
                        }
                    } else if pending.can_submit() {
                        act = Act::Submit;
                    }
                }
                KeyCode::Esc => {
                    act = Act::Decline;
                }
                _ => {}
            }
        }
    }

    match act {
        Act::Submit => submit_question(app, tx),
        Act::Decline => decline_question(app, tx),
        Act::None => {}
    }
    true
}

/// Synthesize the tool_result from the collected answer and resume the loop.
fn submit_question(app: &mut App, tx: &mpsc::Sender<AppEvent>) {
    let Some(pending) = app.pending_question.take() else {
        return;
    };
    let answer = pending.answer();
    let tool_id = pending.tool_id.clone();
    tracing::info!(
        target: "jfc::ui::question",
        tool_id = %tool_id,
        answer = %answer.chars().take(80).collect::<String>(),
        "AskUserQuestion answered"
    );
    // The result content is framed as direct user intent — the model should
    // treat the answer as the user speaking, not as an untrusted tool payload.
    // (See the `[User answered AskUserQuestion]` transcript rewrite in
    // stream/messages/provider_messages.rs.)
    let result_text = format!(
        "User has answered your question: \"{}\"=\"{}\". \
         You can now continue with the user's answer in mind.",
        pending.question, answer
    );
    let _ = tx.try_send(AppEvent::Tool(ToolEvent::Result {
        tool_id,
        result: ExecutionResult::success(result_text),
    }));
    // AllComplete is critical: dropping it on a full channel would wedge the
    // agentic loop with an answered-but-unprocessed question.
    send_critical(tx, AppEvent::Tool(ToolEvent::AllComplete));
}

/// Esc with no committed answer: record a failed tool_result (so the tool_use
/// stays paired) and resume the loop so the model can react.
fn decline_question(app: &mut App, tx: &mpsc::Sender<AppEvent>) {
    let Some(pending) = app.pending_question.take() else {
        return;
    };
    let tool_id = pending.tool_id.clone();
    tracing::info!(
        target: "jfc::ui::question",
        tool_id = %tool_id,
        "AskUserQuestion declined"
    );
    let _ = tx.try_send(AppEvent::Tool(ToolEvent::Result {
        tool_id,
        result: ExecutionResult::failure("User declined to answer the question."),
    }));
    send_critical(tx, AppEvent::Tool(ToolEvent::AllComplete));
}

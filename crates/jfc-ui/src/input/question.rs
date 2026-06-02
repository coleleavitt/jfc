//! `AskUserQuestion` modal: parsing, key handling, and answer submission.
//!
//! Unlike tool *approval* (which gates a dispatch — see `input/approval.rs`),
//! a question *collects answers that become the tool_result*. The model emits
//! an `AskUserQuestion` tool_use; `handle_stream_tool` diverts it into
//! `app.pending_question` instead of dispatching; this module renders the
//! interaction and, once every question is committed, synthesizes a
//! `ToolEvent::Result` for the tool_use + an `AllComplete` so the existing
//! result-recording and agentic-loop-continuation machinery resumes the turn.
//!
//! 1-4 questions are presented one at a time; the user commits each (Enter)
//! and the modal advances to the next unanswered one, submitting once all are
//! committed. Left/Right (or Tab) revisits earlier questions.

use crossterm::event::{self, KeyCode, KeyModifiers};
use tokio::sync::mpsc;

use crate::app::{App, AppEvent, PendingQuestion, QuestionItem, QuestionOption};
use crate::runtime::{ExecutionResult, ToolEvent, send_critical};
use crate::types::{ToolCall, ToolInput};

/// Parse an option object `{label, description?, preview?}`.
fn parse_option(o: &serde_json::Value) -> Option<QuestionOption> {
    let label = o.get("label").and_then(|v| v.as_str())?.to_owned();
    let description = o
        .get("description")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_owned();
    let preview = o.get("preview").and_then(|v| v.as_str()).map(str::to_owned);
    Some(QuestionOption {
        label,
        description,
        preview,
    })
}

/// Parse one question object into a [`QuestionItem`]. Returns `None` when it
/// has no usable options.
fn parse_question(q: &serde_json::Value) -> Option<QuestionItem> {
    let question = q
        .get("question")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_owned();
    let header = q
        .get("header")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_owned();
    let multi_select = q
        .get("multiSelect")
        .or_else(|| q.get("multi_select"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let options: Vec<QuestionOption> = q
        .get("options")
        .and_then(|v| v.as_array())
        .map(|arr| arr.iter().filter_map(parse_option).collect())
        .unwrap_or_default();
    if options.is_empty() {
        return None;
    }
    Some(QuestionItem {
        question,
        header,
        options,
        multi_select,
        selected: 0,
        chosen: std::collections::BTreeSet::new(),
        other_text: String::new(),
        answer: None,
    })
}

/// Build a [`PendingQuestion`] from an `AskUserQuestion` tool call. Returns
/// `None` when the input isn't an `AskUserQuestion` or has no usable questions
/// — the caller falls back to recording a failed tool_result so the tool_use
/// stays paired. The `questions` value is already normalized to an array by
/// the jfc-core parser (legacy single-question form lifted to 1 element).
pub(crate) fn build_pending_question(tool: &ToolCall) -> Option<PendingQuestion> {
    let ToolInput::AskUserQuestion { questions } = &tool.input else {
        return None;
    };
    let items: Vec<QuestionItem> = questions
        .as_array()
        .map(|arr| arr.iter().filter_map(parse_question).collect())
        .unwrap_or_default();
    if items.is_empty() {
        return None;
    }
    Some(PendingQuestion {
        tool_id: tool.id.clone(),
        items,
        current: 0,
        editing_other: false,
    })
}

/// Whether a key event is a Ctrl-modified character `c`.
fn is_ctrl(key: &event::KeyEvent, c: char) -> bool {
    key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char(c)
}

/// Route a key to the active question modal. Returns `true` when a question is
/// pending (the key was consumed), mirroring `handle_approval_key`'s contract.
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

        if pending.editing_other {
            let other_row = pending.cur().other_row();
            // Free-text capture for the "Other" row. Each arm reborrows
            // `pending` rather than holding a `cur_mut()` across the match.
            match key.code {
                KeyCode::Backspace => {
                    pending.cur_mut().other_text.pop();
                }
                KeyCode::Esc => {
                    // Cancel text entry only — not the whole modal.
                    pending.editing_other = false;
                }
                KeyCode::Enter => {
                    if pending.cur().multi_select {
                        if pending.cur().other_text.trim().is_empty() {
                            pending.cur_mut().chosen.remove(&other_row);
                        } else {
                            pending.cur_mut().chosen.insert(other_row);
                        }
                        pending.editing_other = false;
                    } else if pending.cur().can_commit() {
                        // Commit this single-select "Other" answer and advance.
                        pending.editing_other = false;
                        if commit_current(pending) {
                            act = Act::Submit;
                        }
                    }
                }
                KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                    pending.cur_mut().other_text.push(c);
                }
                _ => {}
            }
        } else {
            match key.code {
                KeyCode::Up => {
                    let item = pending.cur_mut();
                    item.selected = item.selected.saturating_sub(1);
                }
                KeyCode::Down => {
                    let item = pending.cur_mut();
                    item.selected = (item.selected + 1).min(item.row_count() - 1);
                }
                _ if is_ctrl(&key, 'p') => {
                    let item = pending.cur_mut();
                    item.selected = item.selected.saturating_sub(1);
                }
                _ if is_ctrl(&key, 'n') => {
                    let item = pending.cur_mut();
                    item.selected = (item.selected + 1).min(item.row_count() - 1);
                }
                // Switch between questions without committing.
                KeyCode::Left | KeyCode::BackTab => {
                    if pending.current > 0 {
                        pending.current -= 1;
                        pending.editing_other = false;
                    }
                }
                KeyCode::Right | KeyCode::Tab => {
                    if pending.current + 1 < pending.items.len() {
                        pending.current += 1;
                        pending.editing_other = false;
                    }
                }
                KeyCode::Char(' ') if pending.cur().multi_select => {
                    let on_other = pending.cur().on_other();
                    if on_other {
                        // Toggling "Other" means typing into it.
                        pending.editing_other = true;
                    } else {
                        let item = pending.cur_mut();
                        if item.chosen.contains(&item.selected) {
                            item.chosen.remove(&item.selected);
                        } else {
                            item.chosen.insert(item.selected);
                        }
                    }
                }
                KeyCode::Enter => {
                    let on_other = pending.cur().on_other();
                    let other_empty = pending.cur().other_text.trim().is_empty();
                    if on_other && other_empty {
                        // Focus the free-text input before it can be committed.
                        pending.editing_other = true;
                    } else if pending.cur().can_commit() && commit_current(pending) {
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

/// Commit the focused question's current selection, then advance to the next
/// unanswered question. Returns `true` when every question is now committed
/// (the caller should submit).
fn commit_current(pending: &mut PendingQuestion) -> bool {
    let selection = pending.cur().current_selection();
    pending.cur_mut().answer = Some(selection);
    !pending.advance_to_next_unanswered()
}

fn submit_question(app: &mut App, tx: &mpsc::Sender<AppEvent>) {
    let Some(pending) = app.pending_question.take() else {
        return;
    };
    let tool_id = pending.tool_id.clone();
    let answers = pending.combined_result();
    tracing::info!(
        target: "jfc::ui::question",
        tool_id = %tool_id,
        questions = pending.items.len(),
        "AskUserQuestion answered"
    );
    // Framed as direct user intent — the model should treat answers as the user
    // speaking, not as untrusted tool payload (see the
    // `[User answered AskUserQuestion]` transcript rewrite in
    // stream/messages/provider_messages.rs).
    let result_text = format!(
        "User has answered your question(s): {answers}. \
         You can now continue with the user's answers in mind."
    );
    let _ = tx.try_send(AppEvent::Tool(ToolEvent::Result {
        tool_id,
        result: ExecutionResult::success(result_text),
    }));
    send_critical(tx, AppEvent::Tool(ToolEvent::AllComplete));
}

fn decline_question(app: &mut App, tx: &mpsc::Sender<AppEvent>) {
    let Some(pending) = app.pending_question.take() else {
        return;
    };
    let tool_id = pending.tool_id.clone();
    tracing::info!(target: "jfc::ui::question", tool_id = %tool_id, "AskUserQuestion declined");
    let _ = tx.try_send(AppEvent::Tool(ToolEvent::Result {
        tool_id,
        result: ExecutionResult::failure("User declined to answer the question(s)."),
    }));
    send_critical(tx, AppEvent::Tool(ToolEvent::AllComplete));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::ToolId;
    use crate::types::{ToolCall, ToolDisplayState, ToolKind, ToolOutput, ToolStatus};

    fn ask_tool(input: serde_json::Value) -> ToolCall {
        ToolCall {
            id: ToolId::from("toolu_q"),
            kind: ToolKind::AskUserQuestion,
            status: ToolStatus::Pending,
            input: ToolInput::from_value("AskUserQuestion", input).unwrap(),
            output: ToolOutput::Text(String::new()),
            display: ToolDisplayState::DEFAULT,
            elapsed_ms: None,
            started_at: None,
            thought_signature: None,
        }
    }

    // Normal: legacy single-question form parses into one item.
    #[test]
    fn build_single_question_normal() {
        let tool = ask_tool(serde_json::json!({
            "question": "Pick one?",
            "options": [{"label": "A"}, {"label": "B"}],
            "multi_select": false,
        }));
        let pq = build_pending_question(&tool).expect("built");
        assert_eq!(pq.items.len(), 1);
        assert_eq!(pq.items[0].options.len(), 2);
        assert!(!pq.items[0].multi_select);
    }

    // Normal: the questions[] contract parses N items with headers + multiSelect.
    #[test]
    fn build_multi_question_normal() {
        let tool = ask_tool(serde_json::json!({
            "questions": [
                {"question": "Q1?", "header": "One", "options": [{"label": "a"}, {"label": "b"}]},
                {"question": "Q2?", "header": "Two", "multiSelect": true,
                 "options": [{"label": "x"}, {"label": "y"}]},
            ]
        }));
        let pq = build_pending_question(&tool).expect("built");
        assert_eq!(pq.items.len(), 2);
        assert_eq!(pq.items[0].header, "One");
        assert!(pq.items[1].multi_select);
    }

    // Normal: committing each question advances, then the combined result joins
    // every Q="A" pair.
    #[test]
    fn combined_result_joins_all_answers_normal() {
        let mut pq = build_pending_question(&ask_tool(serde_json::json!({
            "questions": [
                {"question": "Q1?", "options": [{"label": "a"}, {"label": "b"}]},
                {"question": "Q2?", "options": [{"label": "x"}, {"label": "y"}]},
            ]
        })))
        .expect("built");
        // Answer Q1 = "a" (selected 0), Q2 = "y" (selected 1).
        pq.items[0].answer = Some("a".into());
        pq.items[1].answer = Some("y".into());
        assert_eq!(pq.combined_result(), "\"Q1?\"=\"a\", \"Q2?\"=\"y\"");
    }

    // Robust: multi-select current_selection comma-joins chosen labels.
    #[test]
    fn multi_select_selection_comma_joins_robust() {
        let pq = build_pending_question(&ask_tool(serde_json::json!({
            "questions": [
                {"question": "Q?", "multiSelect": true,
                 "options": [{"label": "a"}, {"label": "b"}, {"label": "c"}]},
            ]
        })))
        .expect("built");
        let mut item = pq.items.into_iter().next().unwrap();
        item.chosen.insert(0);
        item.chosen.insert(2);
        assert_eq!(item.current_selection(), "a, c");
    }

    // Robust: advance_to_next_unanswered wraps and reports completion.
    #[test]
    fn advance_wraps_and_reports_done_robust() {
        let mut pq = build_pending_question(&ask_tool(serde_json::json!({
            "questions": [
                {"question": "Q1?", "options": [{"label": "a"}, {"label": "b"}]},
                {"question": "Q2?", "options": [{"label": "x"}, {"label": "y"}]},
            ]
        })))
        .expect("built");
        pq.items[0].answer = Some("a".into());
        // From q0, next unanswered is q1.
        assert!(pq.advance_to_next_unanswered());
        assert_eq!(pq.current, 1);
        // Commit q1 too → nothing left.
        pq.items[1].answer = Some("x".into());
        assert!(!pq.advance_to_next_unanswered());
        assert!(pq.all_committed());
    }
}

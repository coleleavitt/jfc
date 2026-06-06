use super::{drain_queued_prompts, maybe_continue_task_factory};
use crate::runtime::{AppEvent, EventSender, GoalEvent};
use crate::{app, stream, types};

pub(crate) fn dispatch_goal_evaluator_if_active(app: &mut app::App, tx: &EventSender) -> bool {
    let Some(goal) = app.goal.as_ref() else {
        return false;
    };
    if app.goal_evaluator_in_flight {
        tracing::debug!(target: "jfc::goal", "evaluator already in flight, skipping");
        return true;
    }
    if goal.is_exhausted() {
        let banner = crate::goal::format_exhaustion_banner(goal);
        app.messages.push(types::ChatMessage::assistant(banner));
        app.goal = None;
        crate::toast::push_with_cap(
            &mut app.toasts,
            crate::toast::Toast::new(
                crate::toast::ToastKind::Error,
                "Goal abandoned — iteration cap reached".to_owned(),
            ),
        );
        return false;
    }

    app.goal_evaluator_in_flight = true;
    let condition = goal.condition.clone();
    let history = app.messages.clone();
    let provider = std::sync::Arc::clone(&app.provider);
    let model = app.model.clone();
    let cancel = app.cancel_token.clone();
    let tx_eval = tx.clone();
    tokio::spawn(async move {
        let verdict = tokio::select! {
            biased;
            _ = cancel.cancelled() => {
                tracing::info!(target: "jfc::goal", "evaluator cancelled before reply");
                return;
            }
            verdict = crate::goal::evaluate(provider.as_ref(), model, &condition, &history) => verdict,
        };
        let event = match verdict {
            Ok(verdict) => AppEvent::Goal(GoalEvent::Verdict {
                ok: verdict.ok,
                reason: verdict.reason,
            }),
            Err(error) => {
                tracing::warn!(
                    target: "jfc::goal",
                    error = %error,
                    "evaluator call failed; surfacing as unmet"
                );
                AppEvent::Goal(GoalEvent::Verdict {
                    ok: false,
                    reason: format!("evaluator error: {error}"),
                })
            }
        };
        let _ = tx_eval.send(event).await;
    });
    true
}

pub(crate) async fn handle_goal_verdict(
    app: &mut app::App,
    tx: &EventSender,
    ok: bool,
    reason: String,
) {
    app.goal_evaluator_in_flight = false;
    let Some(mut goal) = app.goal.take() else {
        persist_goal_for_session(app);
        drain_queued_prompts(app, tx).await;
        maybe_continue_task_factory(app, tx).await;
        return;
    };

    if ok {
        let banner = crate::goal::format_success_banner(&goal, &reason);
        append_to_last_assistant_or_push(&mut app.messages, &banner);
        crate::toast::push_with_cap(
            &mut app.toasts,
            crate::toast::Toast::new(crate::toast::ToastKind::Success, "Goal achieved".to_owned()),
        );
        persist_goal_for_session(app);
        drain_queued_prompts(app, tx).await;
        maybe_continue_task_factory(app, tx).await;
        return;
    }

    goal.iterations += 1;
    goal.last_unmet_reason = Some(reason.clone());
    if goal.is_exhausted() {
        let banner = crate::goal::format_exhaustion_banner(&goal);
        append_to_last_assistant_or_push(&mut app.messages, &banner);
        crate::toast::push_with_cap(
            &mut app.toasts,
            crate::toast::Toast::new(
                crate::toast::ToastKind::Error,
                "Goal abandoned — iteration cap reached".to_owned(),
            ),
        );
        persist_goal_for_session(app);
        drain_queued_prompts(app, tx).await;
        maybe_continue_task_factory(app, tx).await;
        return;
    }

    let iteration = goal.iterations;
    let condition = goal.condition.clone();
    app.goal = Some(goal);
    persist_goal_for_session(app);
    let reminder = crate::goal::format_unmet_reminder(&condition, &reason, iteration);
    let body = crate::system_reminder::format(&reminder);
    app.messages.push(types::ChatMessage::user(body));
    tracing::info!(
        target: "jfc::goal",
        iteration,
        "goal unmet; pushed fresh user turn and continuing agentic loop"
    );
    stream::continue_agentic_loop(app, tx).await;
}

fn append_to_last_assistant_or_push(messages: &mut Vec<types::ChatMessage>, body: &str) {
    use crate::types::{MessagePart, Role};

    let target_idx = messages
        .iter()
        .rposition(|message| message.role == Role::Assistant);
    let appended = format!("\n\n{body}");
    if let Some(idx) = target_idx {
        messages[idx].parts.push(MessagePart::Text(appended));
        return;
    }
    messages.push(types::ChatMessage::assistant(body.to_owned()));
}

fn persist_goal_for_session(app: &app::App) {
    let Some(session_id) = app.current_session_id.as_ref() else {
        return;
    };
    crate::goal::save_sidecar(session_id.as_str(), app.goal.as_ref());
}

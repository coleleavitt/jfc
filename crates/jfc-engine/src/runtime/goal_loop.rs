use super::{drain_queued_prompts, maybe_continue_task_factory};
use crate::runtime::{EngineEvent, EventSender, GoalEvent};
use crate::{app, stream, types};

pub fn dispatch_goal_evaluator_if_active(state: &mut app::EngineState, tx: &EventSender) -> bool {
    let Some(goal) = state.goal.as_ref() else {
        return false;
    };
    if state.goal_evaluator_in_flight {
        tracing::debug!(target: "jfc::goal", "evaluator already in flight, skipping");
        return true;
    }
    if goal.is_exhausted() {
        let banner = crate::goal::format_exhaustion_banner(goal);
        state.messages.push(types::ChatMessage::assistant(banner));
        state.goal = None;
        crate::toast::push_with_cap(
            &mut state.toasts,
            crate::toast::Toast::new(
                crate::toast::ToastKind::Error,
                "Goal abandoned — iteration cap reached".to_owned(),
            ),
        );
        return false;
    }

    state.goal_evaluator_in_flight = true;
    let condition = goal.condition.clone();
    let history = state.messages.clone();
    let provider = std::sync::Arc::clone(&state.provider);
    let model = state.model.clone();
    let cancel = state.cancel_token.clone();
    let tx_eval = tx.clone();

    // Opt-in high-stakes path: when council-verdict is enabled and a distinct
    // advisor model is available, decide "is the goal met?" by Council (active
    // model + advisor) so a single model can't prematurely declare success.
    let council_members = if state.council_verdict_enabled {
        let mut members = vec![crate::council::CouncilMember::new(
            std::sync::Arc::clone(&state.provider),
            model.clone(),
        )
        .with_label(model.as_str().to_owned())];
        if let Some(ctx) = crate::stream::LocalAdvisorDispatchContext::from_state(state) {
            members.push(
                crate::council::CouncilMember::new(ctx.provider, ctx.advisor_model.clone())
                    .with_label(ctx.advisor_model.as_str().to_owned()),
            );
        }
        members
    } else {
        Vec::new()
    };
    let use_council = council_members.len() >= 2;

    tokio::spawn(async move {
        let verdict = tokio::select! {
            biased;
            _ = cancel.cancelled() => {
                tracing::info!(target: "jfc::goal", "evaluator cancelled before reply");
                return;
            }
            verdict = async {
                if use_council {
                    tracing::info!(target: "jfc::goal", members = council_members.len(), "goal verdict via model council");
                    crate::goal::evaluate_with_council(council_members, &condition, &history).await
                } else {
                    crate::goal::evaluate(provider.as_ref(), model, &condition, &history).await
                }
            } => verdict,
        };
        let event = match verdict {
            Ok(verdict) => EngineEvent::Goal(GoalEvent::Verdict {
                ok: verdict.ok,
                reason: verdict.reason,
            }),
            Err(error) => {
                tracing::warn!(
                    target: "jfc::goal",
                    error = %error,
                    "evaluator call failed; surfacing as unmet"
                );
                EngineEvent::Goal(GoalEvent::Verdict {
                    ok: false,
                    reason: format!("evaluator error: {error}"),
                })
            }
        };
        let _ = tx_eval.send(event).await;
    });
    true
}

pub async fn handle_goal_verdict(
    state: &mut app::EngineState,
    tx: &EventSender,
    ok: bool,
    reason: String,
) {
    state.goal_evaluator_in_flight = false;
    let Some(mut goal) = state.goal.take() else {
        persist_goal_for_session(state);
        drain_queued_prompts(state, tx).await;
        maybe_continue_task_factory(state, tx).await;
        return;
    };

    if ok {
        let banner = crate::goal::format_success_banner(&goal, &reason);
        append_to_last_assistant_or_push(&mut state.messages, &banner);
        crate::toast::push_with_cap(
            &mut state.toasts,
            crate::toast::Toast::new(crate::toast::ToastKind::Success, "Goal achieved".to_owned()),
        );
        persist_goal_for_session(state);
        drain_queued_prompts(state, tx).await;
        maybe_continue_task_factory(state, tx).await;
        return;
    }

    goal.iterations += 1;
    goal.last_unmet_reason = Some(reason.clone());
    if goal.is_exhausted() {
        let banner = crate::goal::format_exhaustion_banner(&goal);
        append_to_last_assistant_or_push(&mut state.messages, &banner);
        crate::toast::push_with_cap(
            &mut state.toasts,
            crate::toast::Toast::new(
                crate::toast::ToastKind::Error,
                "Goal abandoned — iteration cap reached".to_owned(),
            ),
        );
        persist_goal_for_session(state);
        drain_queued_prompts(state, tx).await;
        maybe_continue_task_factory(state, tx).await;
        return;
    }

    let iteration = goal.iterations;
    let condition = goal.condition.clone();
    state.goal = Some(goal);
    persist_goal_for_session(state);
    let reminder = crate::goal::format_unmet_reminder(&condition, &reason, iteration);
    let body = crate::system_reminder::format(&reminder);
    state.messages.push(types::ChatMessage::user(body));
    tracing::info!(
        target: "jfc::goal",
        iteration,
        "goal unmet; pushed fresh user turn and continuing agentic loop"
    );
    stream::continue_agentic_loop(state, tx).await;
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

fn persist_goal_for_session(state: &app::EngineState) {
    let Some(session_id) = state.current_session_id.as_ref() else {
        return;
    };
    crate::goal::save_sidecar(session_id.as_str(), state.goal.as_ref());
}

//! `StreamEvent::Done(stop_reason)` handler — end-of-stream lifecycle,
//! session save, continuation logic.

use crate::app::{self, App};
use crate::runtime::{
    EventSender, drain_queued_prompts,
};
use crate::{config, session, stream, types};
use crate::types::*;

use super::super::narration_retry::retry_narration_only_end_turn;

/// Handle `StreamEvent::Done(stop_reason)`.
pub(crate) async fn handle_stream_done(
    app: &mut App,
    tx: &EventSender,
    stop_reason: jfc_provider::StopReason,
) {
    app.record_stream_activity();
    app.network_recovery_status = None;
    app.network_recovery_attempts = 0;
    tracing::info!(
        target: "jfc::stream",
        ?stop_reason,
        pending_tool_count = app.pending_tool_calls.len(),
        pending_approval = app.pending_approval.is_some(),
        approval_queue = app.approval_queue.len(),
        "StreamEvent::Done received"
    );
    app.is_streaming = false;
    app.last_stream_event_at = None;
    app.render_cache.borrow_mut().clear_streaming();

    // OpenWebUI / LiteLLM / some third-party gateways
    // leak `<tool_call>` XML into the assistant text
    // instead of using OpenAI's `tool_calls` array.
    // Detect the leaked markup and surface a toast so
    // the user knows their gateway is misconfigured —
    // jfc's renderer can't currently dispatch from
    // inline markup. Mirrors the pattern v132 uses
    // for `tengu_streaming_*` warnings.
    if let Some(last) = app.messages.last() {
        let text: String = last
            .parts
            .iter()
            .filter_map(|p| {
                if let crate::types::MessagePart::Text(t) = p {
                    Some(t.as_str())
                } else {
                    None
                }
            })
            .collect::<Vec<_>>()
            .join("\n");
        if crate::inline_tools::contains_inline_tools(&text) {
            let segments = crate::inline_tools::parse(&text);
            let tool_calls = segments
                .iter()
                .filter(|s| {
                    matches!(s, crate::inline_tools::Segment::ToolCall { .. })
                })
                .count();
            tracing::warn!(
                target: "jfc::stream::inline_tools",
                tool_calls,
                "assistant text contains inline <tool_call> markup — \
                 the upstream gateway is emitting tool calls as text, \
                 not as the OpenAI `tool_calls` field. They won't \
                 dispatch."
            );
            crate::toast::push_with_cap(
                &mut app.toasts,
                crate::toast::Toast::new(
                    crate::toast::ToastKind::Warning,
                    format!(
                        "Detected {tool_calls} inline `<tool_call>` block(s) \
                         in the response — your OpenWebUI/LiteLLM gateway is \
                         emitting tool calls as text, not via OpenAI tool_calls. \
                         Check the gateway config."
                    ),
                ),
            );
        }
    }
    if retry_narration_only_end_turn(app, tx, &stop_reason) {
        return;
    }
    // v126's "Cooked for Nm Ns" post-turn footer: stamp the
    // assistant message with a randomized past-tense verb +
    // formatted duration the moment the stream resolves. The
    // renderer reads `msg.elapsed` and prints it under the
    // assistant's content. Mirrors cli.js:341376
    // (`${A} for ${w}` where A = past-tense verb, w = duration).
    // Stamp `Cooked for Nm Ns` only on the *final* message of
    // the user turn — i.e. when `stop_reason == EndTurn` with
    // nothing pending. Otherwise every sub-stream of a 5-step
    // agentic loop got its own footer (`Brewed for 2s`,
    // `Brewed for 3s`, ...). v126 stamps once per turn so the
    // user sees the cumulative `Brewed for 5m 10s` on the
    // turn's last message. The duration is read off
    // `turn_started_at` (still set at this point — we only
    // clear it in the next block once the EndTurn condition
    // is verified) so it covers tools + thinking + final text.
    let turn_done = stop_reason == jfc_provider::StopReason::EndTurn
        && app.pending_approval.is_none()
        && app.approval_queue.is_empty()
        && app.pending_tool_calls.is_empty();
    if turn_done {
        // v132 session auto-naming — fire on the first
        // assistant-turn completion if no title is set
        // yet. We dispatch a non-blocking tokio task so
        // the UI doesn't stall waiting on the naming
        // call. Best-effort: failures are logged but
        // don't surface to the user (the fallback title
        // is still readable).
        let user_turn_count = app
            .messages
            .iter()
            .filter(|m| matches!(m.role, types::Role::User))
            .count();
        if user_turn_count == 1 {
            let first_user = app
                .messages
                .iter()
                .find(|m| matches!(m.role, types::Role::User))
                .and_then(|m| {
                    m.parts.iter().find_map(|p| match p {
                        types::MessagePart::Text(t) => Some(t.clone()),
                        _ => None,
                    })
                });
            let first_assistant = app
                .messages
                .iter()
                .find(|m| matches!(m.role, types::Role::Assistant))
                .and_then(|m| {
                    m.parts.iter().find_map(|p| match p {
                        types::MessagePart::Text(t) => Some(t.clone()),
                        _ => None,
                    })
                });
            if let (Some(sid), Some(u), Some(a)) =
                (app.current_session_id.clone(), first_user, first_assistant)
            {
                if let Some((p, m)) = crate::tools::snapshot_active_provider() {
                    tokio::spawn(async move {
                        let _ = crate::session_naming::generate_and_save(
                            sid, p, m, u, a,
                        )
                        .await;
                    });
                }
            }
        }
        if let (Some(start), Some(idx)) =
            (app.turn_started_at, app.streaming_assistant_idx)
        {
            let elapsed = std::time::Instant::now().duration_since(start);
            let label = crate::spinner::format_finished(elapsed);
            // v132 per-turn cost surfacing: append the
            // turn's incremental cost to the elapsed footer
            // so the user sees "Cooked for 2m / $0.04". We
            // approximate per-turn cost from the most-
            // recent message_delta usage (already populated
            // into usage_by_model). Skipped when no model
            // is registered (no pricing match).
            let turn_cost = crate::cost::total_cost(&app.usage_by_model);
            let label = if turn_cost > 0.0 {
                format!("{label} / {}", crate::cost::fmt_cost(turn_cost))
            } else {
                label
            };
            // Pull the assistant's text body for the
            // notification preview before re-borrowing
            // mutably to stamp the elapsed footer.
            let preview = app
                .messages
                .get(idx)
                .and_then(|m| {
                    m.parts.iter().find_map(|p| match p {
                        types::MessagePart::Text(s) if !s.is_empty() => {
                            Some(s.clone())
                        }
                        _ => None,
                    })
                })
                .unwrap_or_default();
            if let Some(msg) = app.messages.get_mut(idx) {
                msg.elapsed = Some(label);
            }
            crate::notifications::notify_turn_complete(elapsed, &preview);
        }
        // Push this turn's total token count onto the
        // sparkline history. `last_usage_input` reflects
        // the API's wire-truth count (cumulative across
        // the turn) and `last_usage_output` is the model's
        // generated count. Together they give a per-turn
        // sense of "how much work did this take."
        let turn_total = (app.last_usage_input as u64)
            .saturating_add(app.last_usage_output as u64);
        if turn_total > 0 {
            if app.token_history.len() >= app::TOKEN_HISTORY_CAP {
                app.token_history.pop_front();
            }
            app.token_history.push_back(turn_total);
        }

        // OpenWebUI outlet-filter notification — fire-and-forget POST to
        // `/api/chat/completed` so server-side filters (rate-limit
        // accounting, audit logs, chat-history persistence) see our
        // completion the same way they see a web-client completion.
        // Without this we look like a desync'd client to admins; on
        // chat.ai2s.org `rate_limit_inlet_filter` is globally active
        // and its outlet half wouldn't fire for us. Spawned as a
        // detached task so a slow OWUI ack never blocks the UI.
        if app.provider.name() == "openwebui" {
            if let Some(sid) = app.current_session_id.as_ref().map(|s| s.as_str().to_string()) {
                let model = app.model.to_string();
                let msg_id = uuid::Uuid::new_v4().to_string();
                // The provider holds its own auth-resolution code path;
                // we need to extract base_url + token. Use the store
                // helpers directly since the provider trait doesn't
                // expose them.
                tokio::spawn(async move {
                    let store_path =
                        crate::providers::openwebui::default_store_path();
                    let store = crate::providers::openwebui::load_store(&store_path);
                    if let Some(account) = crate::providers::openwebui::get_current(&store)
                    {
                        crate::providers::openwebui::notify_chat_completed(
                            &account.base_url,
                            &account.token,
                            &model,
                            &sid,
                            &sid, // session_id = chat_id when no websocket
                            &msg_id,
                        )
                        .await;
                    }
                });
            }
        }
    }
    app.streaming_started_at = None;
    app.streaming_last_token_at = None;

    // v132 cost-budget surfacing. When the user has set a
    // session budget and we cross 80% / 100%, post a toast
    // once per threshold so they can choose to stop or
    // switch to a cheaper model. We never hard-block (an
    // in-flight investigation shouldn't be killed mid-turn
    // by an estimate); the toast is the user's signal.
    if let Some(budget_usd) = config::load().session_cost_budget_usd {
        if budget_usd > 0.0 {
            let spent = crate::cost::total_cost(&app.usage_by_model);
            let pct = ((spent / budget_usd) * 100.0).round() as u8;
            let cross = |th: u8| pct >= th && app.cost_budget_warned_at < th;
            if cross(100) {
                app.cost_budget_warned_at = 100;
                crate::toast::push_with_cap(
                    &mut app.toasts,
                    crate::toast::Toast::new(
                        crate::toast::ToastKind::Error,
                        format!(
                            "Session cost {} exceeds budget {} — consider /quit or switching models",
                            crate::cost::fmt_cost(spent),
                            crate::cost::fmt_cost(budget_usd),
                        ),
                    ),
                );
            } else if cross(80) {
                app.cost_budget_warned_at = 80;
                crate::toast::push_with_cap(
                    &mut app.toasts,
                    crate::toast::Toast::new(
                        crate::toast::ToastKind::Warning,
                        format!(
                            "Session cost {} at {pct}% of {} budget",
                            crate::cost::fmt_cost(spent),
                            crate::cost::fmt_cost(budget_usd),
                        ),
                    ),
                );
            }
        }
    }

    // If thinking started but never transitioned to text
    // (e.g. the assistant only produced thinking + tool calls
    // and no visible text), stamp the end now so the spinner
    // shows `thought for Ns` next iteration instead of a
    // stuck `thinking…` from the last reasoning chunk.
    if app.thinking_started_at.is_some() && app.thinking_ended_at.is_none() {
        app.thinking_ended_at = Some(std::time::Instant::now());
    }
    app.streaming_text = String::new();
    app.streaming_reasoning = String::new();
    // Only reset the cumulative token counter when the turn is
    // truly done. During agentic loops (ToolUse stop_reason), the
    // counter should keep accumulating so the spinner shows the
    // full turn's token estimate.
    if turn_done {
        app.streaming_response_bytes = 0;
    }
    // Clear the user-turn clock only when the loop has
    // genuinely concluded — EndTurn stop reason AND no
    // tools pending. ToolUse means an agentic continuation
    // is about to fire and the turn timer must keep running.
    let turn_genuinely_done = stop_reason == jfc_provider::StopReason::EndTurn
        && app.pending_approval.is_none()
        && app.approval_queue.is_empty()
        && app.pending_tool_calls.is_empty();
    if turn_genuinely_done {
        app.turn_started_at = None;
    }

    // Auto-save session after each assistant turn completes
    if let Some(ref session_id) = app.current_session_id {
        let sid = session_id.clone();
        let msgs = app.messages.clone();
        let cwd = app.cwd.clone();
        let model = app.model.clone();
        tokio::spawn(async move {
            session::save_session(
                &sid,
                &msgs,
                Some(cwd.as_str()),
                Some(model.as_str()),
            )
            .await;
        });
        app.last_session_save_at = Some(std::time::Instant::now());
    }
    // v126 queued-prompt drain on plain end_turn: model finished
    // without tools to call → if anything's queued, fire it now.
    if stop_reason == jfc_provider::StopReason::EndTurn
        && app.pending_approval.is_none()
        && app.approval_queue.is_empty()
        && app.pending_tool_calls.is_empty()
        && !app.queued_prompts.is_empty()
    {
        drain_queued_prompts(app, tx).await;
    }
    // Dispatch any tools that were emitted during streaming,
    // regardless of `stop_reason`. Some providers (OpenWebUI,
    // LiteLLM, Bedrock proxies, even Anthropic on transient
    // fast-paths) return `finish_reason="stop"` while the
    // assistant message actually contains tool_use blocks.
    // Mirrors OpenCode's `prompt.ts:1382` workaround: "Some
    // providers return stop even when the assistant message
    // contains tool calls" — keep the loop alive if tools
    // exist. Previously the `else` branch below cleared
    // pending_tool_calls when stop_reason != ToolUse,
    // silently dropping the user's requested tools and
    // leaving the model's "I'll write the file now" claim
    // unbacked — the "hallucinated Done" symptom.
    let has_pending_tools = !app.pending_tool_calls.is_empty();
    let waiting_on_approval =
        app.pending_approval.is_some() || !app.approval_queue.is_empty();
    // Mixed-mode pause_turn handling. When a response
    // carries BOTH local tool_use AND stop_reason=pause_turn,
    // the `has_pending_tools` branch below shadows the
    // PauseTurn dispatch arm. Without remembering that the
    // turn was paused, the AllToolsComplete handler would
    // route through the NORMAL builder and inject the
    // forbidden "Continue from where you left off." filler
    // (cli.js v142:622686). Latch the bit here so AllComplete
    // can re-route to `continue_after_pause_turn` instead.
    // Also covers waiting_on_approval: the user might
    // approve later, and AllComplete fires after approval.
    if (has_pending_tools || waiting_on_approval)
        && stop_reason == jfc_provider::StopReason::PauseTurn
    {
        tracing::info!(
            target: "jfc::stream",
            n = app.pending_tool_calls.len(),
            approval_pending = waiting_on_approval,
            "mixed-mode pause_turn detected — latching pending_pause_turn_resume for AllComplete"
        );
        app.pending_pause_turn_resume = true;
    }
    if has_pending_tools {
        let calls = std::mem::take(&mut app.pending_tool_calls);
        tracing::info!(
            target: "jfc::stream",
            n = calls.len(),
            ?stop_reason,
            kinds = ?calls.iter().map(|t| t.kind.label()).collect::<Vec<_>>(),
            pause_turn_latched = app.pending_pause_turn_resume,
            "stream_done dispatching auto-routed batch"
        );
        crate::runtime::update_task_activities(app, &calls);
        stream::dispatch_tools_batched(
            calls,
            tx,
            std::sync::Arc::clone(&app.dedup_cache),
            Some(std::sync::Arc::clone(&app.task_store)),
            app.team_context.team_name.clone(),
            app.current_session_id
                .as_ref()
                .map(|id| id.as_str().to_owned()),
            std::sync::Arc::clone(&app.provider),
            app.model.clone(),
            app.teammate_event_tx.clone(),
            app.cancel_token.clone(),
        );
    } else if waiting_on_approval {
        tracing::info!(
            target: "jfc::stream",
            pending_modal = app.pending_approval.is_some(),
            queue_depth = app.approval_queue.len(),
            ?stop_reason,
            "stream_done waiting on approval pipeline"
        );
        // Tool awaiting user approval — keep streaming_assistant_idx
        // alive so the approved/denied tool can be inserted into the
        // correct message. AllToolsComplete fires after approval.
    } else if stop_reason == jfc_provider::StopReason::PauseTurn {
        // Anthropic's server-side sampling loop (web_search,
        // code_execution, etc.) hit its iteration cap. The
        // resume protocol per cli.js v142:622686 is "re-send
        // the conversation; the server picks up where it
        // left off." We must NOT inject a synthetic user
        // message — that breaks the resumption signal. The
        // trailing assistant with its `server_tool_use`
        // block IS the cue. `continue_after_pause_turn`
        // stages a fresh assistant slot to stream the
        // resumed response into and re-sends without the
        // "Continue from where you left off." filler.
        tracing::info!(
            target: "jfc::stream",
            streaming_idx = ?app.streaming_assistant_idx,
            "stream_done PauseTurn — resuming server-side sampling loop"
        );
        stream::continue_after_pause_turn(app, tx).await;
    } else if stop_reason == jfc_provider::StopReason::ToolUse {
        // Upstream returned finish_reason="tool_calls" but sent
        // zero tool_call delta chunks (transient LiteLLM/Bedrock
        // failure). The assistant message that was pre-pushed to
        // history is empty and un-replyable; strip it so the
        // next user turn doesn't send a broken conversation turn.
        tracing::warn!(
            target: "jfc::stream",
            streaming_idx = ?app.streaming_assistant_idx,
            "stream_done ToolUse with no tools — stripping dangling assistant turn"
        );
        if let Some(idx) = app.streaming_assistant_idx {
            if idx < app.messages.len() {
                let msg = &app.messages[idx];
                let is_empty = msg.parts.is_empty()
                    || msg.parts.iter().all(|p| {
                        matches!(p, MessagePart::Text(t) if t.trim().is_empty())
                    });
                if is_empty {
                    app.messages.remove(idx);
                }
            }
        }
        app.streaming_assistant_idx = None;
        app.current_stream_request = None;
        app.scroll_to_bottom();
    } else {
        // Non-standard stop reasons (MaxTokens, StopSequence, Other)
        // mean the response was terminated early. Surface a warning so
        // the user knows their response may be incomplete.
        let reason_label = format!("{stop_reason:?}");
        if !matches!(stop_reason, jfc_provider::StopReason::EndTurn) {
            tracing::warn!(
                target: "jfc::stream",
                stop_reason = %reason_label,
                "stream ended with non-EndTurn stop reason"
            );
            let msg = match &stop_reason {
                jfc_provider::StopReason::MaxTokens => {
                    "Response truncated — max output tokens reached. \
                     The model's reply may be incomplete."
                        .to_string()
                }
                jfc_provider::StopReason::Other(s) if s.contains("refusal") => {
                    "The model refused this request.".to_string()
                }
                jfc_provider::StopReason::Other(s) => {
                    format!("Stream ended unexpectedly: {s}")
                }
                _ => format!("Stream ended: {reason_label}"),
            };
            crate::toast::push_with_cap(
                &mut app.toasts,
                crate::toast::Toast::new(crate::toast::ToastKind::Warning, msg),
            );
        }
        app.streaming_assistant_idx = None;
        app.current_stream_request = None;
        app.scroll_to_bottom();
    }
}

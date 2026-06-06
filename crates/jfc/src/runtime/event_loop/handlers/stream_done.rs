//! `StreamEvent::Done(stop_reason)` handler — end-of-stream lifecycle,
//! session save, continuation logic.

use crate::app::{self, App};
use crate::runtime::{EventSender, drain_queued_prompts};
use crate::types::*;
use crate::{config, session, stream, types};

/// Handle `StreamEvent::Done(stop_reason)`.
pub(crate) async fn handle_stream_done(
    app: &mut App,
    tx: &EventSender,
    stop_reason: jfc_provider::StopReason,
) {
    app.record_stream_activity();
    app.network_recovery_status = None;
    app.network_recovery_attempts = 0;
    app.stream_lifecycle = None;
    tracing::info!(
        target: "jfc::stream",
        ?stop_reason,
        pending_tool_count = app.pending_tool_calls.len(),
        pending_approval = app.pending_approval.is_some(),
        approval_queue = app.approval_queue.len(),
        "StreamEvent::Done received"
    );

    // Bug A — the "$-charged blank turn". Detect an assistant turn that
    // finished with NO meaningful content (no text, no tool calls, no
    // reasoning) yet carries usage (the model billed for it). This renders as
    // a blank `assistant (Brewed 34s / $3.84)` bubble AND, if left in history,
    // trips the `empty_message` turn-invariant on the next save. A
    // discard-and-resend (below) both clears the blank bubble and prevents the
    // invariant violation — one fix, both symptoms.
    if let Some(idx) = app.streaming_assistant_idx
        && let Some(msg) = app.messages.get(idx)
    {
        let text_chars: usize = msg
            .parts
            .iter()
            .filter_map(|p| match p {
                crate::types::MessagePart::Text(t) => Some(t.chars().count()),
                _ => None,
            })
            .sum();
        let tool_parts = msg
            .parts
            .iter()
            .filter(|p| matches!(p, crate::types::MessagePart::Tool(_)))
            .count();
        let reasoning_chars: usize = msg
            .parts
            .iter()
            .filter_map(|p| match p {
                crate::types::MessagePart::Reasoning(t) => Some(t.chars().count()),
                _ => None,
            })
            .sum();
        let out_tokens = msg.usage.as_ref().map(|u| u.output_tokens).unwrap_or(0);
        // `safe_to_discard` is intentionally STRICTER than text+tools alone: a
        // turn that produced reasoning (interleaved thinking) but no text/tools
        // is mid-loop and its `thought_signature` must round-trip — discarding
        // it would break thinking continuity. `assistant_turn_has_no_content`
        // mirrors `validate_turn_invariants`' `has_content` test exactly, so a
        // RedactedThinking / Advisor / TaskStatus / CompactBoundary part also
        // counts as content and is never discarded.
        let safe_to_discard = assistant_turn_has_no_content(msg);
        let thinking_only = !safe_to_discard && text_chars == 0 && tool_parts == 0;
        tracing::debug!(
            target: "jfc::stream::lifecycle",
            assistant_idx = idx,
            ?stop_reason,
            text_chars,
            reasoning_chars,
            tool_parts,
            out_tokens,
            thinking_only,
            safe_to_discard,
            streaming_response_bytes = app.streaming_response_bytes,
            "stream turn finalized"
        );
        // Refusal fallback (adapts Claude Code 2.1.160's "switch models when a
        // message is flagged"): if this turn ended in a refusal and the user
        // configured a fallback model, switch to it and resend once. INERT by
        // default — `refusal_fallback_model` is `None` unless the user opts in
        // — and loop-guarded by `refusal_fallback_attempted` (one swap/turn).
        if !app.refusal_fallback_attempted && stop_reason_is_refusal(&stop_reason) {
            let cfg = crate::config::load_arc();
            if cfg.refusal_fallback_enabled
                && let Some(fb) = cfg.refusal_fallback_model.clone()
                && !fb.is_empty()
                && fb != app.model.as_str()
            {
                app.refusal_fallback_attempted = true;
                let from = app.model.to_string();
                tracing::warn!(
                    target: "jfc::stream::lifecycle",
                    %from, fallback = %fb, ?stop_reason,
                    "refusal — switching to fallback model and resending"
                );
                crate::toast::push_with_cap(
                    &mut app.toasts,
                    crate::toast::Toast::new(
                        crate::toast::ToastKind::Warning,
                        format!("{from} refused — retrying on {fb}"),
                    ),
                );
                app.model = jfc_provider::ModelId::new(&fb);
                // Teardown mirrors the empty-billed DiscardAndResend path below.
                app.is_streaming = false;
                app.active_stream_handle = None;
                app.last_stream_event_at = None;
                app.render_cache.borrow_mut().clear_streaming();
                app.streaming_text = String::new();
                app.streaming_reasoning = String::new();
                app.streaming_assistant_idx = None;
                app.current_stream_request = None;
                app.stream_lifecycle = None;
                if idx < app.messages.len() {
                    app.messages.remove(idx);
                }
                stream::continue_agentic_loop(app, tx).await;
                return;
            }
        }
        let inputs = EmptyBilledInputs {
            safe_to_discard,
            out_tokens,
            resend_eligible_stop_reason: empty_billed_resend_eligible(&stop_reason),
            auto_continue: stream::auto_continue_enabled(),
            plan_mode: matches!(app.permission_mode, app::PermissionMode::Plan),
            resend_count: app.empty_billed_resend_count,
            resend_cap: empty_billed_resend_cap(),
        };
        match decide_empty_billed(&inputs) {
            EmptyBilledAction::DiscardAndResend => {
                app.empty_billed_resend_count += 1;
                tracing::warn!(
                    target: "jfc::stream::lifecycle",
                    assistant_idx = idx,
                    ?stop_reason,
                    out_tokens,
                    resend = app.empty_billed_resend_count,
                    max = inputs.resend_cap,
                    "EMPTY-BUT-BILLED assistant turn (no text/tools/reasoning, but usage \
                     recorded) — discarding the blank message and re-streaming. This also \
                     prevents the empty_message invariant violation on save."
                );
                // Tear down the streaming/turn state the same way the normal
                // cleanup ladder would, THEN drop the empty message so history
                // stays valid (no dangling empty assistant turn → no
                // empty_message invariant on the resend's save), THEN
                // re-stream. `continue_agentic_loop` stages a fresh slot and
                // re-sends the (now-clean) conversation.
                app.is_streaming = false;
                app.active_stream_handle = None;
                app.last_stream_event_at = None;
                app.render_cache.borrow_mut().clear_streaming();
                app.streaming_text = String::new();
                app.streaming_reasoning = String::new();
                app.streaming_assistant_idx = None;
                app.current_stream_request = None;
                app.stream_lifecycle = None;
                if idx < app.messages.len() {
                    app.messages.remove(idx);
                }
                stream::continue_agentic_loop(app, tx).await;
                return;
            }
            EmptyBilledAction::CapReached => {
                tracing::warn!(
                    target: "jfc::stream::lifecycle",
                    assistant_idx = idx,
                    ?stop_reason,
                    out_tokens,
                    resend = app.empty_billed_resend_count,
                    max = inputs.resend_cap,
                    "EMPTY-BUT-BILLED assistant turn: resend cap reached — leaving the turn \
                     and waiting for the user (provider appears persistently degraded)."
                );
            }
            EmptyBilledAction::WarnOnly => {
                tracing::warn!(
                    target: "jfc::stream::lifecycle",
                    assistant_idx = idx,
                    ?stop_reason,
                    out_tokens,
                    reasoning_chars,
                    "EMPTY-BUT-BILLED assistant turn: no text/tools/reasoning, usage recorded \
                     (renders as a blank 'assistant (Brewed …)' bubble). Not auto-resending \
                     (refusal / max-tokens / auto-continue off / plan mode)."
                );
            }
            EmptyBilledAction::ResetBudget => {
                // The turn produced real content (text, tools, reasoning, …) →
                // reset the resend budget so a later genuinely-empty turn gets
                // its full retries rather than inheriting a stale count.
                app.empty_billed_resend_count = 0;
            }
            EmptyBilledAction::None => {}
        }
    }

    app.is_streaming = false;
    app.active_stream_handle = None;
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
                .filter(|s| matches!(s, crate::inline_tools::Segment::ToolCall { .. }))
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
        && app.pending_tool_calls.is_empty()
        && app.pending_classifications == 0
        && app.in_flight_eager_dispatches == 0
        && app.in_flight_tool_batches == 0;
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
                && let Some((p, m)) = crate::tools::snapshot_active_provider()
            {
                tokio::spawn(async move {
                    let _ = crate::session_naming::generate_and_save(sid, p, m, u, a).await;
                });
            }
        }
        if let (Some(start), Some(idx)) = (app.turn_started_at, app.streaming_assistant_idx) {
            let elapsed = std::time::Instant::now().duration_since(start);
            // Honest, minimal footer: just how long the turn took (no
            // decorative past-tense verb), plus the turn's incremental
            // cost when we can price it — e.g. `took 2m04s · $0.04`.
            let label = format!("took {}", crate::spinner::format_finished(elapsed));
            // Per-turn cost = cumulative-now minus the snapshot taken when
            // this user turn began. Without the baseline this showed the whole
            // session's running spend, not the turn's. saturating at 0 guards
            // the rare case where usage_by_model is reset mid-turn.
            let turn_cost =
                (crate::cost::total_cost(&app.usage_by_model) - app.turn_start_cost).max(0.0);
            let label = if turn_cost > 0.0 {
                format!("{label} · {}", crate::cost::fmt_cost(turn_cost))
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
                        types::MessagePart::Text(s) if !s.is_empty() => Some(s.clone()),
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
        let turn_total = (app.last_usage_input as u64).saturating_add(app.last_usage_output as u64);
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
        if app.provider.name() == "openwebui"
            && let Some(sid) = app
                .current_session_id
                .as_ref()
                .map(|s| s.as_str().to_string())
        {
            let model = app.model.to_string();
            let msg_id = uuid::Uuid::new_v4().to_string();
            // The provider holds its own auth-resolution code path;
            // we need to extract base_url + token. Use the store
            // helpers directly since the provider trait doesn't
            // expose them.
            tokio::spawn(async move {
                let store_path = crate::providers::openwebui::default_store_path();
                let store = crate::providers::openwebui::load_store(&store_path);
                if let Some(account) = crate::providers::openwebui::get_current(&store) {
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
    app.streaming_started_at = None;
    app.streaming_last_token_at = None;

    // v132 cost-budget surfacing. When the user has set a
    // session budget and we cross 80% / 100%, post a toast
    // once per threshold so they can choose to stop or
    // switch to a cheaper model. We never hard-block (an
    // in-flight investigation shouldn't be killed mid-turn
    // by an estimate); the toast is the user's signal.
    if let Some(budget_usd) = config::load_arc().session_cost_budget_usd
        && budget_usd > 0.0
    {
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
        app.streaming_thinking_tokens = 0;
    }
    // Clear the user-turn clock only when the loop has
    // genuinely concluded — EndTurn stop reason AND no
    // tools pending. ToolUse means an agentic continuation
    // is about to fire and the turn timer must keep running.
    let turn_genuinely_done = stop_reason == jfc_provider::StopReason::EndTurn
        && app.pending_approval.is_none()
        && app.approval_queue.is_empty()
        && app.pending_tool_calls.is_empty()
        && app.pending_classifications == 0
        && app.in_flight_eager_dispatches == 0
        && app.in_flight_tool_batches == 0;
    let needs_dynamic_loop_keepalive = turn_genuinely_done && dynamic_loop_keepalive_needed(app);
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
            session::save_session(&sid, &msgs, Some(cwd.as_str()), Some(model.as_str())).await;
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
        // If the drain staged a fresh turn (non-meta prompt → new assistant
        // slot + live `streaming_assistant_idx`), bail out now. The cleanup
        // ladder below — specifically the EndTurn `else` arm — would otherwise
        // null out `streaming_assistant_idx`/`current_stream_request`, and the
        // newly-spawned stream's chunks would then arrive with no slot to
        // attach to (stream alive but no visible output). A meta-only drain
        // leaves `is_streaming` false and falls through to the normal cleanup.
        if app.is_streaming {
            return;
        }
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
    let has_pending_tools =
        !app.pending_tool_calls.is_empty() || app.in_flight_eager_dispatches > 0;
    let waiting_on_approval = app.pending_approval.is_some() || !app.approval_queue.is_empty();
    // Auto-mode: one or more tool calls are still awaiting an async classifier
    // verdict. We must NOT finalize the turn (which clears the streaming slot)
    // until every verdict lands — otherwise the late ClassifierDecision finds
    // no slot and the tool is silently dropped. The final verdict
    // (handle_classifier_decision) dispatches the approved batch / continues.
    let awaiting_classifier = app.pending_classifications > 0;
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
    if awaiting_classifier {
        // Hold the turn open: keep streaming_assistant_idx alive so the
        // pending ClassifierDecision events can record their tools and the
        // final one drives dispatch/continuation. This must take priority
        // over has_pending_tools so a partially-classified batch isn't
        // dispatched early (leaving later-approved tools stranded).
        tracing::info!(
            target: "jfc::stream",
            in_flight = app.pending_classifications,
            already_approved = app.pending_tool_calls.len(),
            "stream_done holding turn open for in-flight auto-mode classifier verdicts"
        );
    } else if has_pending_tools {
        if app.in_flight_eager_dispatches > 0 || app.in_flight_tool_batches > 0 {
            tracing::info!(
                target: "jfc::stream",
                pending = app.pending_tool_calls.len(),
                in_flight_eager = app.in_flight_eager_dispatches,
                in_flight_batches = app.in_flight_tool_batches,
                ?stop_reason,
                "stream_done waiting for in-flight eager tool prefix before dispatching remaining tools"
            );
        } else {
            super::stream_tool::dispatch_pending_after_stream(app, tx);
        }
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
        if let Some(idx) = app.streaming_assistant_idx
            && idx < app.messages.len()
        {
            let msg = &app.messages[idx];
            let is_empty = msg.parts.is_empty()
                || msg
                    .parts
                    .iter()
                    .all(|p| matches!(p, MessagePart::Text(t) if t.trim().is_empty()));
            if is_empty {
                app.messages.remove(idx);
            } else if stream::should_continue_loop(&app.messages) {
                // The assistant DID emit tool_use blocks, but every one was
                // recorded terminal *before* dispatch — denied by the active
                // permission mode, or malformed provider input (handle_stream_tool's
                // terminal-on-arrival / denied-by-mode arms record the tool with a
                // Failed status and synthetic tool_result but never push it onto
                // `pending_tool_calls`). No ToolResult event will fire, so no
                // AllToolsComplete drives the loop forward. Without continuing here
                // the model is left staring at its own failed tool_result with no
                // follow-up turn — the "denied tool stalls the turn" symptom.
                // `should_continue_loop` confirms the last assistant ends with
                // all-terminal tools, so resuming is safe (no dangling Pending).
                tracing::info!(
                    target: "jfc::stream",
                    streaming_idx = ?app.streaming_assistant_idx,
                    "stream_done ToolUse with only pre-resolved (denied/malformed) tools — continuing agentic loop"
                );
                stream::continue_agentic_loop(app, tx).await;
                return;
            }
        }
        app.streaming_assistant_idx = None;
        app.current_stream_request = None;
        app.stream_lifecycle = None;
        app.scroll_to_bottom();
    } else if stream::should_continue_loop(&app.messages) {
        // The assistant emitted tool_use blocks that were all recorded
        // terminal *before* dispatch — denied by the active permission mode,
        // or malformed provider input (handle_stream_tool's denied-by-mode /
        // terminal-on-arrival arms mark the tool Failed and synthesize a
        // tool_result but never enqueue it onto `pending_tool_calls`, so no
        // ToolResult/AllToolsComplete event ever drives the loop).
        //
        // The ToolUse stop-reason arm above already handles this when the
        // gateway reports `finish_reason="tool_calls"`. But OpenWebUI/LiteLLM
        // commonly map a denied-tool turn to `finish_reason="stop"` → EndTurn,
        // which lands HERE. Without continuing, the model is frozen staring at
        // its own failed tool_result with no follow-up turn — the "denied tool
        // halts the loop" symptom. `should_continue_loop` confirms the last
        // assistant ends with all-terminal tools, so resuming is safe (no
        // dangling Pending / Running). The mid-tool-loop guard in
        // prepare_stream_request keeps the catalog advertised on the resend.
        tracing::info!(
            target: "jfc::stream",
            ?stop_reason,
            streaming_idx = ?app.streaming_assistant_idx,
            "stream_done with only pre-resolved (denied/malformed) tools — continuing agentic loop"
        );
        stream::continue_agentic_loop(app, tx).await;
        return;
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
                jfc_provider::StopReason::Refusal => "The model refused this request.".to_string(),
                jfc_provider::StopReason::Other(s) if looks_like_refusal_stop_reason(s) => {
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
        app.stream_lifecycle = None;
        app.scroll_to_bottom();
    }

    // ── Self-continuation guard ──────────────────────────────────────────
    // The turn genuinely concluded (EndTurn, nothing pending). If
    // auto-continue is on and the model either (a) stalled on a
    // permission-asking question ("Want me to …?") or (b) left unfinished
    // queued tasks, drive the next step instead of waiting for the user to
    // type "continue". This is the behavioral half of the "factory": the
    // stopping condition is *scope exhausted*, not *finished a sub-step*.
    if should_self_continue_after_stop_reason(&stop_reason) {
        if turn_genuinely_done {
            let stalled = stream::assistant_text_stalls(&app.messages);
            if stalled {
                app.exploration_state
                    .bump_for_signal(crate::exploration::ExplorationSignal::AssistantStall);
            } else {
                let counts = app.task_store.counts();
                if counts.pending == 0 && counts.in_progress == 0 {
                    app.exploration_state.decay_after_progress();
                }
            }
        }
        maybe_self_continue(app, tx).await;
    }
    if needs_dynamic_loop_keepalive && !app.is_streaming {
        schedule_dynamic_loop_keepalive();
    }
}

fn should_self_continue_after_stop_reason(stop_reason: &jfc_provider::StopReason) -> bool {
    !matches!(stop_reason, jfc_provider::StopReason::Refusal)
        && !matches!(stop_reason, jfc_provider::StopReason::Other(s) if looks_like_refusal_stop_reason(s))
}

/// True iff an assistant message carries no content that
/// `validate_turn_invariants` would accept. Kept in lock-step with the
/// `has_content` predicate in `validate_turn_invariants_inner` (message.rs):
/// any non-empty Text / Reasoning / Advisor, or any RedactedThinking / Tool /
/// TaskStatus / CompactBoundary part, is content. A turn that is `true` here
/// is the empty-but-billed shape — safe to discard because nothing
/// (including a `thought_signature`-bearing reasoning part) is lost.
fn assistant_turn_has_no_content(msg: &types::ChatMessage) -> bool {
    !msg.parts.iter().any(|p| match p {
        MessagePart::Text(s) | MessagePart::Reasoning(s) | MessagePart::Advisor(s) => !s.is_empty(),
        MessagePart::RedactedThinking(_)
        | MessagePart::Tool(_)
        | MessagePart::TaskStatus(_)
        | MessagePart::CompactBoundary { .. } => true,
    })
}

/// Stop reasons for which re-streaming an empty-but-billed turn is sane.
/// `Refusal` is eligible only inside this stricter empty-billed gate: the turn
/// had no text, no tools, no reasoning/redacted-thinking, and is capped by the
/// resend budget. A refusal that produced real content still reaches the normal
/// non-EndTurn stop path and never self-continues.
/// `MaxTokens` is excluded — the output budget, not a degraded stream, ended
/// the turn, so a resend won't help. `EndTurn`, empty `Refusal`, and
/// non-refusal `Other(_)` are eligible.
fn empty_billed_resend_eligible(stop_reason: &jfc_provider::StopReason) -> bool {
    match stop_reason {
        jfc_provider::StopReason::EndTurn | jfc_provider::StopReason::Refusal => true,
        jfc_provider::StopReason::Other(s) => !looks_like_refusal_stop_reason(s),
        jfc_provider::StopReason::MaxTokens
        | jfc_provider::StopReason::StopSequence
        | jfc_provider::StopReason::ToolUse
        | jfc_provider::StopReason::PauseTurn => false,
    }
}

/// Max consecutive empty-but-billed resends before we give up and leave the
/// turn for the user — prevents an infinite billed loop against a persistently
/// degraded provider. Reuses the `[continuation] max_self_continuations`
/// budget capped at 3 (an empty turn is a much stronger "something is wrong"
/// signal than a stall, so retry far fewer times).
fn empty_billed_resend_cap() -> u32 {
    stream::max_self_continuations().min(3)
}

/// Value-only snapshot of everything `decide_empty_billed` needs, so the
/// discard-and-resend decision is unit-testable without an `App`, env vars, or
/// a live stream (mirrors the `TurnIdleness` pattern). All fields are resolved
/// at the call site from `App` + the finalized message.
#[derive(Debug, Clone, Copy)]
struct EmptyBilledInputs {
    /// The finalized assistant turn has no content `validate_turn_invariants`
    /// would accept (no text/tools/reasoning/redacted-thinking/…).
    safe_to_discard: bool,
    /// Output tokens the provider billed for the (empty) turn.
    out_tokens: u64,
    /// The stop reason permits a resend (EndTurn / non-refusal Other).
    resend_eligible_stop_reason: bool,
    /// Auto-continue is enabled (we never silently re-spend without it).
    auto_continue: bool,
    /// Plan mode is read-only — never auto-act.
    plan_mode: bool,
    /// Consecutive empty-billed resends already spent this turn.
    resend_count: u32,
    /// Cap on consecutive resends.
    resend_cap: u32,
}

/// What `handle_stream_done` should do about the just-finalized assistant turn
/// with respect to the empty-but-billed condition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EmptyBilledAction {
    /// Remove the blank message and re-stream the conversation.
    DiscardAndResend,
    /// Empty-but-billed but the resend cap is exhausted — warn, leave the turn.
    CapReached,
    /// Empty-but-billed but resend isn't appropriate (refusal / max-tokens /
    /// auto-continue off / plan mode) — warn, leave the turn.
    WarnOnly,
    /// The turn produced real content — clear the resend budget.
    ResetBudget,
    /// Nothing to do (e.g. empty turn that wasn't billed).
    None,
}

/// Pure decision for the empty-but-billed turn. Separated from the handler so
/// the gating logic — the part the user explicitly asked to get right (discard
/// only when text==0 && tools==0 && reasoning==0, i.e. `safe_to_discard`) — is
/// exhaustively unit-testable.
fn decide_empty_billed(i: &EmptyBilledInputs) -> EmptyBilledAction {
    if !i.safe_to_discard {
        return EmptyBilledAction::ResetBudget;
    }
    if i.out_tokens == 0 {
        // An empty, *unbilled* turn (e.g. a freshly-staged placeholder slot
        // that never streamed). Not our concern — the normal cleanup ladder
        // handles it. Don't touch the resend budget.
        return EmptyBilledAction::None;
    }
    if !i.resend_eligible_stop_reason || !i.auto_continue || i.plan_mode {
        return EmptyBilledAction::WarnOnly;
    }
    if i.resend_count < i.resend_cap {
        EmptyBilledAction::DiscardAndResend
    } else {
        EmptyBilledAction::CapReached
    }
}

fn looks_like_refusal_stop_reason(reason: &str) -> bool {
    reason == "content_filter" || reason.contains("refusal")
}

/// Whether the turn ended in a refusal — the first-class `Refusal` stop reason
/// or a refusal-shaped `Other` (content_filter / "refusal"). Drives the
/// refusal-fallback model swap.
fn stop_reason_is_refusal(sr: &jfc_provider::StopReason) -> bool {
    matches!(sr, jfc_provider::StopReason::Refusal)
        || matches!(sr, jfc_provider::StopReason::Other(s) if looks_like_refusal_stop_reason(s))
}

fn dynamic_loop_keepalive_needed(app: &App) -> bool {
    if !crate::autonomous_loop::loop_keepalive_enabled() {
        return false;
    }
    let Some(loop_state) = app.autonomous_loop.as_ref() else {
        return false;
    };
    if loop_state.pacing != crate::autonomous_loop::LoopPacing::Dynamic {
        return false;
    }
    let Some(idx) = app.streaming_assistant_idx else {
        return false;
    };
    let Some(message) = app.messages.get(idx) else {
        return false;
    };
    !message_scheduled_dynamic_loop_wakeup(message)
}

fn message_scheduled_dynamic_loop_wakeup(message: &types::ChatMessage) -> bool {
    message.parts.iter().any(|part| {
        let MessagePart::Tool(tool) = part else {
            return false;
        };
        tool.kind == ToolKind::ScheduleWakeup
            && matches!(
                &tool.input,
                ToolInput::ScheduleWakeup { prompt, .. }
                    if prompt == crate::autonomous_loop::LOOP_SENTINEL_DYNAMIC
            )
    })
}

fn schedule_dynamic_loop_keepalive() {
    let result = crate::tools::execute_schedule_wakeup(
        crate::autonomous_loop::LOOP_KEEPALIVE_DELAY_SECONDS,
        crate::autonomous_loop::LOOP_SENTINEL_DYNAMIC,
        "autonomous loop keepalive: model did not reschedule the dynamic loop",
    );
    if result.is_error() {
        tracing::warn!(
            target: "jfc::autonomous_loop",
            error = %result.output,
            "dynamic loop keepalive schedule failed"
        );
    } else {
        tracing::info!(
            target: "jfc::autonomous_loop",
            output = %result.output,
            "dynamic loop keepalive scheduled"
        );
    }
}

#[cfg(test)]
mod stream_done_lifecycle_tests {
    use std::{sync::Arc, time::Instant};

    use jfc_provider::{EventStream, ModelInfo, Provider, ProviderMessage, StreamOptions};

    use super::*;

    #[test]
    fn stop_reason_is_refusal_classifies_normal() {
        use jfc_provider::StopReason;
        assert!(stop_reason_is_refusal(&StopReason::Refusal));
        assert!(stop_reason_is_refusal(&StopReason::Other(
            "content_filter".into()
        )));
        assert!(stop_reason_is_refusal(&StopReason::Other(
            "model_refusal".into()
        )));
        assert!(!stop_reason_is_refusal(&StopReason::EndTurn));
        assert!(!stop_reason_is_refusal(&StopReason::ToolUse));
        assert!(!stop_reason_is_refusal(&StopReason::Other("stop".into())));
    }

    struct TestProvider;

    #[async_trait::async_trait]
    impl Provider for TestProvider {
        fn name(&self) -> &str {
            "test"
        }

        fn available_models(&self) -> Vec<ModelInfo> {
            Vec::new()
        }

        async fn stream(
            &self,
            _messages: Vec<ProviderMessage>,
            _options: &StreamOptions,
        ) -> anyhow::Result<EventStream> {
            Ok(Box::pin(futures::stream::empty()))
        }
    }

    impl jfc_provider::seal::Sealed for TestProvider {}

    #[tokio::test]
    async fn pending_classifier_keeps_turn_clock_active_robust() {
        let mut app = App::new(Arc::new(TestProvider), "test-model");
        app.task_store = jfc_session::TaskStore::in_memory();
        app.is_streaming = true;
        app.turn_started_at = Some(Instant::now());
        app.pending_classifications = 1;
        let (tx, _rx) = tokio::sync::mpsc::channel(8);

        handle_stream_done(&mut app, &tx, jfc_provider::StopReason::EndTurn).await;

        assert!(
            app.turn_started_at.is_some(),
            "classifier verdicts are still in flight, so the user turn must stay open"
        );
        assert!(!app.is_streaming);
    }

    #[tokio::test]
    async fn stream_done_clears_active_stream_handle_robust() {
        let mut app = App::new(Arc::new(TestProvider), "test-model");
        app.task_store = jfc_session::TaskStore::in_memory();
        app.is_streaming = true;
        let handle = tokio::spawn(async {
            std::future::pending::<()>().await;
        });
        app.active_stream_handle = Some(handle.abort_handle());
        let (tx, _rx) = tokio::sync::mpsc::channel(8);

        handle_stream_done(&mut app, &tx, jfc_provider::StopReason::EndTurn).await;

        assert!(app.active_stream_handle.is_none());
        assert!(!app.has_interruptible_work());
        handle.abort();
    }

    /// Build an `App` whose streaming slot is an empty-but-billed assistant
    /// turn (no text/tools/reasoning, but `output_tokens > 0`) — the exact
    /// shape `handle_stream_done` must discard.
    fn app_with_empty_billed_turn() -> App {
        let mut app = App::new(Arc::new(TestProvider), "test-model");
        app.task_store = jfc_session::TaskStore::in_memory();
        app.messages.push(ChatMessage::user("hello".into()));
        let mut blank = ChatMessage::assistant(String::new());
        blank.usage = Some(ModelUsage {
            output_tokens: 64,
            ..Default::default()
        });
        app.messages.push(blank);
        app.streaming_assistant_idx = Some(1);
        app.is_streaming = true;
        app
    }

    // Normal: with auto-continue on, an empty-but-billed EndTurn is discarded
    // (the blank assistant message is removed → no blank bubble, no
    // empty_message invariant on save) and a resend is staged. Asserting the
    // removal is the cross-cutting fix for BOTH symptoms the user named.
    #[serial_test::serial]
    #[tokio::test]
    async fn empty_billed_turn_is_discarded_and_resent_normal() {
        unsafe { std::env::set_var("JFC_AUTO_CONTINUE", "1") };
        let mut app = app_with_empty_billed_turn();
        let (tx, _rx) = tokio::sync::mpsc::channel(8);

        handle_stream_done(&mut app, &tx, jfc_provider::StopReason::EndTurn).await;

        // The blank assistant turn was removed and a fresh slot staged by
        // continue_agentic_loop. The user turn survives, and the *billed*
        // empty message (output_tokens=64) is gone — only the new empty
        // streaming slot (no usage) remains, so no empty_message invariant
        // can fire on the next save.
        assert_eq!(app.empty_billed_resend_count, 1, "resend budget consumed");
        assert_eq!(
            app.messages.iter().filter(|m| m.role == Role::User).count(),
            1,
            "user turn preserved"
        );
        assert!(
            !app.messages
                .iter()
                .any(|m| m.usage.as_ref().is_some_and(|u| u.output_tokens == 64)),
            "the billed empty assistant message must have been removed"
        );
        unsafe { std::env::remove_var("JFC_AUTO_CONTINUE") };
    }

    // Normal — the realistic degraded-stream class: `stop_reason=Other(_)`
    // (dropped/abandoned stream) dominates the observed empty-billed warnings.
    // It must discard + resend exactly like EndTurn.
    #[serial_test::serial]
    #[tokio::test]
    async fn empty_billed_other_stop_reason_is_discarded_normal() {
        unsafe { std::env::set_var("JFC_AUTO_CONTINUE", "1") };
        let mut app = app_with_empty_billed_turn();
        let (tx, _rx) = tokio::sync::mpsc::channel(8);

        handle_stream_done(
            &mut app,
            &tx,
            jfc_provider::StopReason::Other("stream_error".into()),
        )
        .await;

        assert_eq!(app.empty_billed_resend_count, 1, "Other(_) must resend");
        assert!(
            !app.messages
                .iter()
                .any(|m| m.usage.as_ref().is_some_and(|u| u.output_tokens == 64)),
            "the billed empty assistant message must have been removed"
        );
        unsafe { std::env::remove_var("JFC_AUTO_CONTINUE") };
    }

    // Robust: an empty-billed refusal is usually a provider continuation
    // failure, not a semantic content refusal. It is safe to discard and
    // resend because the assistant turn has no content to preserve and the
    // empty-billed resend cap prevents paid loops.
    #[serial_test::serial]
    #[tokio::test]
    async fn empty_billed_refusal_is_discarded_once_regression() {
        unsafe { std::env::set_var("JFC_AUTO_CONTINUE", "1") };
        let mut app = app_with_empty_billed_turn();
        let (tx, _rx) = tokio::sync::mpsc::channel(8);

        handle_stream_done(&mut app, &tx, jfc_provider::StopReason::Refusal).await;

        assert_eq!(
            app.empty_billed_resend_count, 1,
            "empty-billed refusal should trigger one capped resend"
        );
        assert!(
            !app.messages
                .iter()
                .any(|m| m.usage.as_ref().is_some_and(|u| u.output_tokens == 64)),
            "the billed empty assistant message must have been removed"
        );
        unsafe { std::env::remove_var("JFC_AUTO_CONTINUE") };
    }

    // Robust: with auto-continue OFF, we do NOT auto-resend (no silent
    // re-spend), and the budget stays at 0. The turn is left for the user.
    #[serial_test::serial]
    #[tokio::test]
    async fn empty_billed_turn_not_resent_when_auto_continue_off_robust() {
        unsafe { std::env::set_var("JFC_AUTO_CONTINUE", "0") };
        let mut app = app_with_empty_billed_turn();
        let (tx, _rx) = tokio::sync::mpsc::channel(8);

        handle_stream_done(&mut app, &tx, jfc_provider::StopReason::EndTurn).await;

        assert_eq!(
            app.empty_billed_resend_count, 0,
            "must not resend when auto-continue is disabled"
        );
        assert!(!app.is_streaming, "turn finalized, left for the user");
        unsafe { std::env::remove_var("JFC_AUTO_CONTINUE") };
    }
}

/// Auto-drive the next in-scope step when the model stalls or leaves work
/// queued, instead of forcing a manual "continue". Gated on `auto_continue`
/// (env/config/factory), disabled in plan mode, and capped by
/// `max_self_continuations` to prevent runaway loops.
async fn maybe_self_continue(app: &mut App, tx: &EventSender) {
    // Only fire when the turn is fully settled and idle. The
    // `in_flight_*` guards are load-bearing: when StreamDone arrives
    // with `stop_reason=ToolUse` and pending tools, the `has_pending_tools`
    // arm above *dispatches* the batch (incrementing `in_flight_tool_batches`)
    // and then falls through to here. Those tools are still Pending/Running —
    // their `AllToolsComplete` hasn't landed yet. Without these guards we
    // self-continue immediately, start the next assistant stream, and
    // `build_assistant_and_tool_result_messages` serializes the still-running
    // tool as "abandoned" (tool_wire.rs Pending/Running arm). Mirror the
    // `turn_truly_complete` predicate in tools.rs so continuation is driven by
    // `handle_all_complete` *after* the tool results land, not by this race.
    let idle = TurnIdleness {
        is_streaming: app.is_streaming,
        pending_approval: app.pending_approval.is_some(),
        approval_queue_len: app.approval_queue.len(),
        pending_tool_calls_len: app.pending_tool_calls.len(),
        queued_prompts_len: app.queued_prompts.len(),
        pending_classifications: app.pending_classifications,
        in_flight_eager_dispatches: app.in_flight_eager_dispatches,
        in_flight_tool_batches: app.in_flight_tool_batches,
    };
    if !turn_is_idle_for_self_continue(&idle) {
        return;
    }
    // Plan mode is read-only by contract — never auto-act.
    if matches!(app.permission_mode, app::PermissionMode::Plan) {
        return;
    }
    if !stream::auto_continue_enabled() {
        return;
    }

    // Is there a reason to continue? Either unfinished queued tasks, or the
    // model ended on a permission-asking stall.
    let counts = app.task_store.counts();
    let tasks_remain = counts.pending > 0 || counts.in_progress > 0;
    let stalled = stream::assistant_text_stalls(&app.messages);
    if !tasks_remain && !stalled {
        return;
    }

    // Cap consecutive self-continuations.
    let max = stream::max_self_continuations();
    if app.self_continuation_count >= max {
        tracing::info!(
            target: "jfc::stream",
            count = app.self_continuation_count,
            max,
            "self-continuation cap reached — waiting for user"
        );
        return;
    }
    app.self_continuation_count += 1;

    tracing::info!(
        target: "jfc::stream",
        count = app.self_continuation_count,
        tasks_remain,
        stalled,
        pending_tasks = counts.pending,
        in_progress = counts.in_progress,
        "self-continuing without user nudge"
    );

    // Inject a system-reminder nudge as a fresh user turn. Phrased to match
    // the operating rule: finish the scope, don't ask permission for the next
    // in-scope step.
    let reason = if tasks_remain {
        format!(
            "Continue the remaining work. There are {} pending and {} in-progress task(s) — \
             work through them. Do NOT stop to ask permission for the next in-scope step; \
             only pause for genuine forks (incompatible interpretations, irreversible actions, \
             or missing external input). When the whole scope is done, verify (build/test/commit) \
             and report.",
            counts.pending, counts.in_progress
        )
    } else {
        "Continue — do the next step you just proposed instead of asking whether to. \
         Only pause for genuine forks (incompatible interpretations, irreversible actions, \
         or missing external input). When the full scope is done, verify and report."
            .to_string()
    };
    let body = crate::system_reminder::format(&reason);
    app.messages.push(types::ChatMessage::user(body));
    stream::continue_agentic_loop(app, tx).await;
}

/// Snapshot of the runtime fields `maybe_self_continue` inspects to decide
/// whether the turn is fully idle. Extracted into a value-only struct so the
/// predicate can be unit-tested without spinning up an `App`.
#[derive(Debug, Clone, Copy)]
struct TurnIdleness {
    is_streaming: bool,
    pending_approval: bool,
    approval_queue_len: usize,
    pending_tool_calls_len: usize,
    queued_prompts_len: usize,
    pending_classifications: usize,
    in_flight_eager_dispatches: usize,
    in_flight_tool_batches: usize,
}

/// Pure predicate: returns true iff the runtime is genuinely quiescent and
/// `maybe_self_continue` may start the next assistant stream.
///
/// The `in_flight_*` checks pin the bug fix: when `StreamDone(ToolUse)` arrives
/// the handler dispatches the batch and increments `in_flight_tool_batches`
/// before falling through to here. Returning `true` while those counters are
/// positive starts the next stream over still-running tools, which then
/// serialize as `[abandoned]` (tool_wire.rs Pending/Running arm).
fn turn_is_idle_for_self_continue(idle: &TurnIdleness) -> bool {
    !idle.is_streaming
        && !idle.pending_approval
        && idle.approval_queue_len == 0
        && idle.pending_tool_calls_len == 0
        && idle.queued_prompts_len == 0
        && idle.pending_classifications == 0
        && idle.in_flight_eager_dispatches == 0
        && idle.in_flight_tool_batches == 0
}

#[cfg(test)]
mod self_continue_idleness_tests {
    //! Pins the `turn_is_idle_for_self_continue` predicate. The
    //! `in_flight_*` checks reproduce the abandoned-tool race observed in
    //! `ses_20260528_130646.log`:
    //!
    //! ```text
    //! 20:29:25.570981 stream_done dispatching auto-routed batch n=1 kinds=["TaskUpdate"]
    //! 20:29:25.571227 self-continuing without user nudge   ← BUG: too early
    //! 20:29:25.571409 build_assistant_and_tool_result_messages ... abandoned_count=1
    //! ```
    use super::{
        TurnIdleness, should_self_continue_after_stop_reason, turn_is_idle_for_self_continue,
    };

    fn fully_idle() -> TurnIdleness {
        TurnIdleness {
            is_streaming: false,
            pending_approval: false,
            approval_queue_len: 0,
            pending_tool_calls_len: 0,
            queued_prompts_len: 0,
            pending_classifications: 0,
            in_flight_eager_dispatches: 0,
            in_flight_tool_batches: 0,
        }
    }

    // Normal: fully-quiescent state is the only state that lets
    // self-continue fire.
    #[test]
    fn fully_idle_allows_continue_normal() {
        assert!(turn_is_idle_for_self_continue(&fully_idle()));
    }

    #[test]
    fn refusal_stop_reason_blocks_retry_loop_robust() {
        assert!(!should_self_continue_after_stop_reason(
            &jfc_provider::StopReason::Refusal
        ));
        assert!(should_self_continue_after_stop_reason(
            &jfc_provider::StopReason::EndTurn
        ));
        assert!(!should_self_continue_after_stop_reason(
            &jfc_provider::StopReason::Other("content_filter".to_string())
        ));
    }

    // Normal — REGRESSION: this is the exact state stream_done leaves
    // behind after `dispatch_auto_routed_batch` increments
    // `in_flight_tool_batches`. Pre-fix the predicate returned true and
    // the next stream stomped over the still-running tool.
    #[test]
    fn in_flight_tool_batch_blocks_continue_normal_regression() {
        let mut s = fully_idle();
        s.in_flight_tool_batches = 1;
        assert!(
            !turn_is_idle_for_self_continue(&s),
            "self-continue MUST wait for in-flight tool batches to drain"
        );
    }

    // Normal — REGRESSION: same race for the eager-dispatch path
    // (tool_event.rs schedules tools before StreamDone arrives).
    #[test]
    fn in_flight_eager_dispatch_blocks_continue_normal_regression() {
        let mut s = fully_idle();
        s.in_flight_eager_dispatches = 1;
        assert!(
            !turn_is_idle_for_self_continue(&s),
            "self-continue MUST wait for in-flight eager dispatches to drain"
        );
    }

    // Robust: each pre-existing guard still blocks, so the refactor is
    // behaviour-preserving for the original conditions.
    #[test]
    fn pre_existing_guards_still_block_continue_robust() {
        let cases = [
            (
                "is_streaming",
                TurnIdleness {
                    is_streaming: true,
                    ..fully_idle()
                },
            ),
            (
                "pending_approval",
                TurnIdleness {
                    pending_approval: true,
                    ..fully_idle()
                },
            ),
            (
                "approval_queue",
                TurnIdleness {
                    approval_queue_len: 1,
                    ..fully_idle()
                },
            ),
            (
                "pending_tool_calls",
                TurnIdleness {
                    pending_tool_calls_len: 1,
                    ..fully_idle()
                },
            ),
            (
                "queued_prompts",
                TurnIdleness {
                    queued_prompts_len: 1,
                    ..fully_idle()
                },
            ),
            (
                "pending_classifications",
                TurnIdleness {
                    pending_classifications: 1,
                    ..fully_idle()
                },
            ),
        ];
        for (label, state) in cases {
            assert!(
                !turn_is_idle_for_self_continue(&state),
                "guard `{label}` must block self-continue"
            );
        }
    }
}

#[cfg(test)]
mod empty_billed_tests {
    //! Pins the empty-but-billed discard-and-resend decision. The headline
    //! invariant (the bug the user flagged): a turn is only discarded when it
    //! has NO text AND NO tools AND NO reasoning — a thinking-only turn must
    //! survive so its `thought_signature` round-trips.
    use super::{
        EmptyBilledAction, EmptyBilledInputs, assistant_turn_has_no_content, decide_empty_billed,
        empty_billed_resend_eligible,
    };
    use crate::ids::ToolId;
    use crate::types::{
        ChatMessage, MessagePart, ToolCall, ToolDisplayState, ToolInput, ToolKind, ToolOutput,
        ToolStatus,
    };
    use jfc_provider::StopReason;

    fn eligible() -> EmptyBilledInputs {
        EmptyBilledInputs {
            safe_to_discard: true,
            out_tokens: 128,
            resend_eligible_stop_reason: true,
            auto_continue: true,
            plan_mode: false,
            resend_count: 0,
            resend_cap: 3,
        }
    }

    fn tool_part(id: &str) -> MessagePart {
        MessagePart::tool(ToolCall {
            id: ToolId::from(id),
            kind: ToolKind::Bash,
            status: ToolStatus::Completed,
            input: ToolInput::Generic {
                summary: "x".into(),
            },
            output: ToolOutput::Text("ok".into()),
            display: ToolDisplayState::DEFAULT,
            elapsed_ms: None,
            started_at: None,
            thought_signature: None,
        })
    }

    // Normal: the fully-eligible empty-but-billed turn is discarded + resent.
    #[test]
    fn eligible_empty_billed_discards_and_resends_normal() {
        assert_eq!(
            decide_empty_billed(&eligible()),
            EmptyBilledAction::DiscardAndResend
        );
    }

    // Robust — the headline gate: a thinking-only turn (reasoning>0, so
    // `safe_to_discard=false`) is NEVER discarded; its budget is reset instead.
    #[test]
    fn thinking_only_turn_is_not_discarded_robust() {
        let i = EmptyBilledInputs {
            safe_to_discard: false,
            ..eligible()
        };
        assert_eq!(decide_empty_billed(&i), EmptyBilledAction::ResetBudget);
    }

    // Robust: an empty turn that was NOT billed is left to the normal cleanup
    // ladder and does not perturb the resend budget.
    #[test]
    fn empty_unbilled_turn_is_noop_robust() {
        let i = EmptyBilledInputs {
            out_tokens: 0,
            ..eligible()
        };
        assert_eq!(decide_empty_billed(&i), EmptyBilledAction::None);
    }

    // Robust: max-tokens / auto-continue-off / plan-mode suppress the resend
    // (warn only). Empty refusal is handled by the eligible path above; a
    // contentful refusal never reaches this empty-billed decision.
    #[test]
    fn ineligible_conditions_warn_only_robust() {
        for i in [
            EmptyBilledInputs {
                resend_eligible_stop_reason: false,
                ..eligible()
            },
            EmptyBilledInputs {
                auto_continue: false,
                ..eligible()
            },
            EmptyBilledInputs {
                plan_mode: true,
                ..eligible()
            },
        ] {
            assert_eq!(decide_empty_billed(&i), EmptyBilledAction::WarnOnly);
        }
    }

    // Robust: once the resend budget is spent, stop resending and wait.
    #[test]
    fn resend_cap_reached_stops_robust() {
        let i = EmptyBilledInputs {
            resend_count: 3,
            resend_cap: 3,
            ..eligible()
        };
        assert_eq!(decide_empty_billed(&i), EmptyBilledAction::CapReached);
    }

    // Robust: the stop-reason gate matches the spec — EndTurn, empty Refusal,
    // and generic Other(_) are resend-eligible; max-tokens/tool-use/pause are
    // not. Refusal is eligible only through the stricter empty-billed caller.
    #[test]
    fn resend_eligible_stop_reasons_robust() {
        assert!(empty_billed_resend_eligible(&StopReason::EndTurn));
        assert!(empty_billed_resend_eligible(&StopReason::Other(
            "stream_error".into()
        )));
        assert!(empty_billed_resend_eligible(&StopReason::Refusal));
        assert!(!empty_billed_resend_eligible(&StopReason::Other(
            "content_filter".into()
        )));
        assert!(!empty_billed_resend_eligible(&StopReason::MaxTokens));
        assert!(!empty_billed_resend_eligible(&StopReason::ToolUse));
        assert!(!empty_billed_resend_eligible(&StopReason::PauseTurn));
        assert!(!empty_billed_resend_eligible(&StopReason::StopSequence));
    }

    // Robust: `assistant_turn_has_no_content` mirrors `validate_turn_invariants`
    // — empty/whitespace-free text is "no content", but a reasoning part, a
    // tool part, or a task-status part each counts as content.
    #[test]
    fn has_no_content_matches_invariant_robust() {
        assert!(assistant_turn_has_no_content(&ChatMessage::assistant(
            String::new()
        )));
        assert!(!assistant_turn_has_no_content(&ChatMessage::assistant(
            "hi".into()
        )));
        assert!(!assistant_turn_has_no_content(
            &ChatMessage::assistant_parts(vec![MessagePart::Reasoning("thinking…".into())])
        ));
        assert!(!assistant_turn_has_no_content(
            &ChatMessage::assistant_parts(vec![tool_part("t1")])
        ));
        assert!(!assistant_turn_has_no_content(
            &ChatMessage::assistant_parts(vec![MessagePart::RedactedThinking("blob".into())])
        ));
    }
}

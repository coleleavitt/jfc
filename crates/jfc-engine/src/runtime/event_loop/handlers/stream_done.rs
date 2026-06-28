//! `StreamEvent::Done(stop_reason)` handler — end-of-stream lifecycle,
//! session save, continuation logic.

use crate::app::{self, EngineState};
use crate::runtime::{EventSender, dispatch_goal_evaluator_if_active, drain_queued_prompts};
use crate::types::*;
use crate::{config, stream, types};
use serde::Serialize;

const MALFORMED_TOOL_USE_RETRY_MARKER: &str = "jfc_malformed_tool_use_clean_retry";
const MALFORMED_TOOL_USE_RETRY_REMINDER: &str = "The previous assistant response ended with a \
tool-use stop reason but did not produce a valid tool call. Treat that assistant response as \
invalid and retry cleanly now. If a tool is needed, emit it through the provider tool-use channel \
with valid JSON input. Do not repeat malformed XML or text-form tool calls.";
const REFUSAL_DIAGNOSTIC_KIND: &str = "refusal_diagnostic";
const REFUSAL_DIAGNOSTIC_KEY: &str = "stream_done";
const REFUSAL_INPUT_PREVIEW_CHARS: usize = 400;
const REFUSAL_VISIBLE_PREVIEW_CHARS: usize = 400;

/// Handle `StreamEvent::Done(stop_reason)`.
pub async fn handle_stream_done(
    state: &mut EngineState,
    tx: &EventSender,
    stop_reason: jfc_provider::StopReason,
) {
    state.record_stream_activity();
    state.network_recovery_status = None;
    state.network_recovery_attempts = 0;
    state.stream_lifecycle = None;
    tracing::info!(
        target: "jfc::stream",
        ?stop_reason,
        pending_tool_count = state.pending_tool_calls.len(),
        pending_approval = state.pending_approval.is_some(),
        approval_queue = state.approval_queue.len(),
        "StreamEvent::Done received"
    );
    let mut empty_billed_refusal_cap_reached = false;

    // Bug A — the "$-charged blank turn". Detect an assistant turn that
    // finished with NO meaningful content (no text, no tool calls, no
    // reasoning) yet carries usage (the model billed for it). This renders as
    // a blank `assistant (Brewed 34s / $3.84)` bubble AND, if left in history,
    // trips the `empty_message` turn-invariant on the next save. A
    // discard-and-resend (below) both clears the blank bubble and prevents the
    // invariant violation — one fix, both symptoms.
    if let Some(idx) = state.streaming_assistant_idx
        && let Some(msg) = state.messages.get(idx)
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
        // The visible reasoning/CoT text of the refusing turn, concatenated. Only
        // used (and only ever emitted to an ephemeral debug log) when
        // `refusal_log_reasoning` is opted in; the durable diagnostic keeps counts
        // only — see `record_refusal_diagnostic`.
        let reasoning_text: String = msg
            .parts
            .iter()
            .filter_map(|p| match p {
                crate::types::MessagePart::Reasoning(t) => Some(t.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n");
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
            streaming_response_bytes = state.streaming_response_bytes,
            "stream turn finalized"
        );
        let stop_reason_refusal = stop_reason_is_refusal(&stop_reason);
        let assistant_response_text = if idx < state.messages.len() {
            message_text_parts(&state.messages[idx])
        } else {
            String::new()
        };
        let refusal_candidate_uidx = state.messages[..idx.min(state.messages.len())]
            .iter()
            .rposition(|m| matches!(m.role, crate::types::Role::User));
        let mut model_refusal_assessment = None;
        if !state.refusal_fallback_attempted
            && !stop_reason_refusal
            && state.refusal_rewrite_retry_enabled
            && state.refusal_rewrite_retry_count
                < refusal_rewrite_retry_cap(state.refusal_rewrite_retry_max)
            && state.prompt_rewrite.is_some()
            && !assistant_response_text.trim().is_empty()
            && let Some(uidx) = refusal_candidate_uidx
        {
            let original_prompt = message_text_parts(&state.messages[uidx]);
            if !original_prompt.trim().is_empty() {
                let provider = state.provider.clone();
                let providers = state.providers.clone();
                let advisor = state.local_advisor_model.clone();
                let active = state.model.to_string();
                let pr = state.prompt_rewrite.clone();
                let cancel = state.cancel_token.clone();
                // Race the classification against the cancel token so ESC/interrupt
                // can abort a hung refusal-classifier call (the 218-minute freeze bug).
                let classify_fut = crate::runtime::prompt_rewrite_gate::classify_response_refusal(
                    pr.as_ref(),
                    provider,
                    &providers,
                    advisor.as_ref().map(|m| m.as_str()),
                    &active,
                    &original_prompt,
                    &assistant_response_text,
                );
                model_refusal_assessment = tokio::select! {
                    biased;
                    _ = cancel.cancelled() => {
                        tracing::info!(
                            target: "jfc::stream::lifecycle",
                            "refusal classification cancelled by user interrupt"
                        );
                        None
                    }
                    result = classify_fut => result,
                };
            }
        }
        let model_refusal = model_refusal_assessment
            .as_ref()
            .is_some_and(|assessment| assessment.is_refusal());
        let refusal_detected = stop_reason_refusal || model_refusal;
        if model_refusal {
            if let Some(assessment) = &model_refusal_assessment {
                tracing::warn!(
                    target: "jfc::stream::lifecycle",
                    confidence = assessment.confidence,
                    rationale = %assessment.rationale,
                    "assistant response classified as a refusal despite non-refusal stop reason"
                );
            }
        }
        // Refusal fallback (adapts Claude Code 2.1.160's "switch models when a
        // message is flagged"): if this turn ended in a refusal and the user
        // configured a fallback model, switch to it and resend once. INERT by
        // default — `refusal_fallback_model` is `None` unless the user opts in
        // — and loop-guarded by `refusal_fallback_attempted` (one swap/turn).
        if !state.refusal_fallback_attempted && refusal_detected {
            let cfg = crate::config::load_arc();
            if cfg.refusal_fallback_enabled
                && let Some(fb) = cfg.refusal_fallback_model.clone()
                && !fb.is_empty()
                && fb != state.model.as_str()
            {
                state.refusal_fallback_attempted = true;
                let from = state.model.to_string();
                tracing::warn!(
                    target: "jfc::stream::lifecycle",
                    %from, fallback = %fb, ?stop_reason,
                    "refusal — switching to fallback model and resending"
                );
                crate::toast::push_with_cap(
                    &mut state.toasts,
                    crate::toast::Toast::new(
                        crate::toast::ToastKind::Warning,
                        format!("{from} refused — retrying on {fb}"),
                    ),
                );
                state.model = jfc_provider::ModelId::new(&fb);
                // Teardown mirrors the empty-billed DiscardAndResend path below.
                state.is_streaming = false;
                state.active_stream_handle = None;
                state.clear_active_stream_scope();
                state.last_stream_event_at = None;
                state.push_effect(crate::app::EngineEffect::StreamingFinalized);
                state.streaming_text = String::new();
                state.streaming_reasoning = String::new();
                state.streaming_assistant_idx = None;
                state.current_stream_request = None;
                state.stream_lifecycle = None;
                if idx < state.messages.len() {
                    state.messages.remove(idx);
                }
                stream::continue_agentic_loop(state, tx).await;
                return;
            }

            // No fallback model configured (or already used this turn): a
            // content-bearing refusal still dead-stops below (it has content, so
            // the empty-billed resend doesn't fire, and Refusal is excluded from
            // self-continuation). Under transient provider degradation — the
            // `Rate limited`/`overloaded` mid-stream errors seen in the wild — a
            // refusal stop is frequently spurious and clears on a plain resend.
            // So retry on the SAME model a bounded number of times before
            // surfacing the stall, mirroring the empty-billed resend cap.
            //
            // Only for CONTENT-BEARING refusals: an empty-but-billed refusal is
            // handled by the empty-billed resend ladder below (which has its own
            // budget + teardown), so we must not intercept it here.
            let refusal_has_content =
                idx < state.messages.len() && !assistant_turn_has_no_content(&state.messages[idx]);

            // Opt-in refusal→rewrite→resend (over-refusal mitigation). When a
            // *legitimate* request trips a provider false-positive, run it back
            // through the local rewrite gate and resend a scope-bounded
            // clarification. Bounded by `refusal_rewrite_retry_max`. The gate's
            // policy stage refuses genuinely-disallowed intent, so a real refusal
            // returns `Refused` and is NOT resent — only legitimate prompts get
            // rephrased. Requires the prompt-rewrite gate (`state.prompt_rewrite`)
            // to be configured; otherwise the gate returns `None` and we fall
            // through to the existing resend/dead-stop.
            //
            // NOT gated on `refusal_has_content`: an EMPTY (blank) refusal — the
            // provider returning `stop_reason=Refusal` with no body, which renders
            // as a blank bubble and otherwise only gets the same prompt
            // plain-resent by the empty-billed ladder below — is exactly the case
            // a rephrase is meant to break, so it routes through here too.
            if state.refusal_rewrite_retry_enabled
                && state.refusal_rewrite_retry_count
                    < refusal_rewrite_retry_cap(state.refusal_rewrite_retry_max)
                && let Some(uidx) = refusal_candidate_uidx
            {
                // Pin the pristine original at index 0 on the first retry so every
                // round rewrites the TRUE user intent (and the verifier checks
                // intent against it), not a prior rewrite — avoiding drift.
                if state.refusal_rewrite_attempts.is_empty() {
                    let orig = message_text_parts(&state.messages[uidx]);
                    state.refusal_rewrite_attempts.push(orig);
                }
                let original_prompt = state.refusal_rewrite_attempts[0].clone();
                // Rewrites already tried this turn (fed back so the rewriter
                // produces a *different* clarification each round).
                let prior_attempts: Vec<String> = state.refusal_rewrite_attempts[1..].to_vec();
                // Refusal body for the rewriter's feedback; empty/blank refusals
                // get a synthetic reason so the rewriter still has context.
                let refusal_text: String = if assistant_response_text.trim().is_empty() {
                    "(provider returned an empty/blank refusal — no content)".to_string()
                } else {
                    let mut text: String = assistant_response_text.chars().take(500).collect();
                    if let Some(assessment) = &model_refusal_assessment {
                        if !assessment.rationale.trim().is_empty() {
                            text.push_str("\nClassifier rationale: ");
                            text.push_str(assessment.rationale.trim());
                        }
                        text.push_str(&format!(
                            "\nClassifier confidence: {:.2}",
                            assessment.confidence
                        ));
                    }
                    text
                };
                if !original_prompt.trim().is_empty() {
                    let provider = state.provider.clone();
                    let providers = state.providers.clone();
                    let advisor = state.local_advisor_model.clone();
                    let active = state.model.to_string();
                    let pr = state.prompt_rewrite.clone();
                    let cancel = state.cancel_token.clone();
                    // Race the rewrite gate against the cancel token so ESC/interrupt
                    // can abort a hung rewrite call (the 218-minute freeze bug).
                    let rewrite_fut = crate::runtime::prompt_rewrite_gate::evaluate_with_feedback(
                        pr.as_ref(),
                        provider,
                        &providers,
                        advisor.as_ref().map(|m| m.as_str()),
                        &active,
                        &original_prompt,
                        &[],
                        &prior_attempts,
                        Some(&refusal_text),
                    );
                    let decision = tokio::select! {
                        biased;
                        _ = cancel.cancelled() => {
                            tracing::info!(
                                target: "jfc::stream::lifecycle",
                                "refusal rewrite gate cancelled by user interrupt"
                            );
                            None
                        }
                        result = rewrite_fut => result,
                    };
                    if let Some(jfc_audit::RewriteDecision::Rewritten(rw)) = decision {
                        state.refusal_rewrite_retry_count += 1;
                        state.refusal_rewrite_attempts.push(rw.text.clone());
                        tracing::warn!(
                            target: "jfc::stream::lifecycle",
                            attempt = state.refusal_rewrite_retry_count,
                            max = refusal_rewrite_retry_cap(state.refusal_rewrite_retry_max),
                            original = %original_prompt,
                            rewrite = %rw.text,
                            "refusal — rephrased via rewrite gate (auto-applied, opt-in) and resending"
                        );
                        crate::toast::push_with_cap(
                            &mut state.toasts,
                            crate::toast::Toast::new(
                                crate::toast::ToastKind::Warning,
                                format!(
                                    "Refused — auto-rephrased & retrying ({}/{})",
                                    state.refusal_rewrite_retry_count,
                                    refusal_rewrite_retry_cap(state.refusal_rewrite_retry_max)
                                ),
                            ),
                        );
                        // Substitute the user turn's text with the rephrase (other
                        // parts, e.g. attachments, are preserved). Unlike the
                        // PRE-FLIGHT gate (which proposes a rewrite for the user to
                        // accept/reject), this response-side retry AUTO-APPLIES —
                        // intentionally, since it is opt-in and the whole point is
                        // to recover from a provider false-positive without
                        // blocking. It is surfaced, not silent: the toast above
                        // announces each attempt and the pristine original is
                        // recorded in the warn log. Teardown mirrors the resend
                        // paths.
                        set_message_text_parts(&mut state.messages[uidx], rw.text);
                        state.is_streaming = false;
                        state.active_stream_handle = None;
                        state.clear_active_stream_scope();
                        state.last_stream_event_at = None;
                        state.push_effect(crate::app::EngineEffect::StreamingFinalized);
                        state.streaming_text = String::new();
                        state.streaming_reasoning = String::new();
                        state.streaming_assistant_idx = None;
                        state.current_stream_request = None;
                        state.stream_lifecycle = None;
                        if idx < state.messages.len() {
                            state.messages.remove(idx);
                        }
                        stream::continue_agentic_loop(state, tx).await;
                        return;
                    }
                    // Refused / Pass / None (incl. gate-off or infra error) → fall
                    // through to the existing same-model resend / dead-stop. A
                    // genuinely-disallowed prompt returns Refused and lands here,
                    // so it is never rephrased-and-resent.
                }
            }

            if refusal_has_content && state.refusal_resend_count < refusal_resend_cap() {
                state.refusal_resend_count += 1;
                tracing::warn!(
                    target: "jfc::stream::lifecycle",
                    ?stop_reason,
                    attempt = state.refusal_resend_count,
                    max = refusal_resend_cap(),
                    "content-bearing refusal — resending on the same model (likely \
                     transient degradation) instead of dead-stopping"
                );
                crate::toast::push_with_cap(
                    &mut state.toasts,
                    crate::toast::Toast::new(
                        crate::toast::ToastKind::Warning,
                        format!(
                            "Refusal — retrying ({}/{})",
                            state.refusal_resend_count,
                            refusal_resend_cap()
                        ),
                    ),
                );
                state.is_streaming = false;
                state.active_stream_handle = None;
                state.clear_active_stream_scope();
                state.last_stream_event_at = None;
                state.push_effect(crate::app::EngineEffect::StreamingFinalized);
                state.streaming_text = String::new();
                state.streaming_reasoning = String::new();
                state.streaming_assistant_idx = None;
                state.current_stream_request = None;
                state.stream_lifecycle = None;
                if idx < state.messages.len() {
                    state.messages.remove(idx);
                }
                stream::continue_agentic_loop(state, tx).await;
                return;
            }
        }
        let inputs = EmptyBilledInputs {
            safe_to_discard,
            out_tokens,
            resend_eligible_stop_reason: empty_billed_resend_eligible(&stop_reason),
            auto_continue: stream::auto_continue_enabled(),
            plan_mode: matches!(state.permission_mode, app::PermissionMode::Plan),
            resend_count: state.empty_billed_resend_count,
            resend_cap: empty_billed_resend_cap(),
        };
        let empty_billed_action = decide_empty_billed(&inputs);
        if refusal_detected {
            record_refusal_diagnostic(
                state,
                RefusalDiagnosticInput {
                    stop_reason: &stop_reason,
                    stop_reason_refusal,
                    model_refusal,
                    assistant_idx: idx,
                    text_chars,
                    reasoning_chars,
                    tool_parts,
                    out_tokens,
                    safe_to_discard,
                    thinking_only,
                    empty_billed_action,
                    visible_response_text: &assistant_response_text,
                    reasoning_text: &reasoning_text,
                },
            );
        }
        match empty_billed_action {
            EmptyBilledAction::DiscardAndResend => {
                state.empty_billed_resend_count += 1;
                tracing::warn!(
                    target: "jfc::stream::lifecycle",
                    assistant_idx = idx,
                    ?stop_reason,
                    out_tokens,
                    resend = state.empty_billed_resend_count,
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
                state.is_streaming = false;
                state.active_stream_handle = None;
                state.clear_active_stream_scope();
                state.last_stream_event_at = None;
                state.push_effect(crate::app::EngineEffect::StreamingFinalized);
                state.streaming_text = String::new();
                state.streaming_reasoning = String::new();
                state.streaming_assistant_idx = None;
                state.current_stream_request = None;
                state.stream_lifecycle = None;
                if idx < state.messages.len() {
                    state.messages.remove(idx);
                }
                stream::continue_agentic_loop(state, tx).await;
                return;
            }
            EmptyBilledAction::CapReached => {
                empty_billed_refusal_cap_reached = stop_reason_is_refusal(&stop_reason);
                tracing::warn!(
                    target: "jfc::stream::lifecycle",
                    assistant_idx = idx,
                    ?stop_reason,
                    out_tokens,
                    resend = state.empty_billed_resend_count,
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
                // its full retries rather than inheriting a stale count. A normal
                // (non-refusal) result also clears the refusal retry budget.
                state.empty_billed_resend_count = 0;
                state.refusal_resend_count = 0;
                state.refusal_rewrite_retry_count = 0;
                state.refusal_rewrite_attempts.clear();
            }
            EmptyBilledAction::None => {}
        }
    }

    state.is_streaming = false;
    state.active_stream_handle = None;
    state.clear_active_stream_scope();
    state.last_stream_event_at = None;
    state.push_effect(crate::app::EngineEffect::StreamingFinalized);

    // OpenWebUI / LiteLLM / some third-party gateways
    // leak `<tool_call>` XML into the assistant text
    // instead of using OpenAI's `tool_calls` array.
    // Detect the leaked markup and surface a toast so
    // the user knows their gateway is misconfigured —
    // jfc's renderer can't currently dispatch from
    // inline markup. Mirrors the pattern v132 uses
    // for `tengu_streaming_*` warnings.
    if let Some(last) = state.messages.last() {
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
                &mut state.toasts,
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
        && state.pending_approval.is_none()
        && state.approval_queue.is_empty()
        && state.pending_tool_calls.is_empty()
        && state.pending_classifications == 0
        && state.in_flight_eager_dispatches == 0
        && state.in_flight_tool_batches == 0
        // An open AskUserQuestion modal means the turn isn't done — don't stamp
        // the post-turn footer / auto-name while the user is still answering.
        && state.pending_question.is_none();
    if turn_done {
        // v132 session auto-naming — fire on the first
        // assistant-turn completion if no title is set
        // yet. We dispatch a non-blocking tokio task so
        // the UI doesn't stall waiting on the naming
        // call. Best-effort: failures are logged but
        // don't surface to the user (the fallback title
        // is still readable).
        let user_turn_count = state
            .messages
            .iter()
            .filter(|m| matches!(m.role, types::Role::User))
            .count();
        if user_turn_count == 1 {
            let first_user = state
                .messages
                .iter()
                .find(|m| matches!(m.role, types::Role::User))
                .and_then(|m| {
                    m.parts.iter().find_map(|p| match p {
                        types::MessagePart::Text(t) => Some(t.clone()),
                        _ => None,
                    })
                });
            let first_assistant = state
                .messages
                .iter()
                .find(|m| matches!(m.role, types::Role::Assistant))
                .and_then(|m| {
                    m.parts.iter().find_map(|p| match p {
                        types::MessagePart::Text(t) => Some(t.clone()),
                        _ => None,
                    })
                });
            if let (Some(sid), Some(u), Some(a)) = (
                state.current_session_id.clone(),
                first_user,
                first_assistant,
            ) && let Some((p, m)) = crate::tools::snapshot_active_provider()
            {
                tokio::spawn(async move {
                    let _ = crate::session_naming::generate_and_save(sid, p, m, u, a).await;
                });
            }
        }
        if let (Some(start), Some(idx)) = (state.turn_started_at, state.streaming_assistant_idx) {
            let elapsed = std::time::Instant::now().duration_since(start);
            // Honest, minimal footer: just how long the turn took (no
            // decorative past-tense verb), plus the turn's incremental
            // cost when we can price it — e.g. `took 2m04s · $0.04`.
            let label = format!(
                "took {}",
                crate::runtime::durations::format_finished(elapsed)
            );
            // Per-turn cost = cumulative-now minus the snapshot taken when
            // this user turn began. Without the baseline this showed the whole
            // session's running spend, not the turn's. saturating at 0 guards
            // the rare case where usage_by_model is reset mid-turn.
            let turn_cost =
                (crate::cost::total_cost(&state.usage_by_model) - state.turn_start_cost).max(0.0);
            let label = if turn_cost > 0.0 {
                format!("{label} · {}", crate::cost::fmt_cost(turn_cost))
            } else {
                label
            };
            // Append time-to-first-token when measured this turn — the
            // API-responsiveness signal CC surfaces as `ttft_ms`. e.g.
            // `took 2m04s · $0.04 · ttft 420ms`.
            let label = match state.ttft_ms {
                Some(ttft) => format!(
                    "{label} · ttft {}",
                    crate::runtime::durations::format_ttft(ttft)
                ),
                None => label,
            };
            // Pull the assistant's text body for the
            // notification preview before re-borrowing
            // mutably to stamp the elapsed footer.
            let preview = state
                .messages
                .get(idx)
                .and_then(|m| {
                    m.parts.iter().find_map(|p| match p {
                        types::MessagePart::Text(s) if !s.is_empty() => Some(s.clone()),
                        _ => None,
                    })
                })
                .unwrap_or_default();
            if let Some(msg) = state.messages.get_mut(idx) {
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
        let turn_total =
            (state.last_usage_input as u64).saturating_add(state.last_usage_output as u64);
        if turn_total > 0 {
            if state.token_history.len() >= crate::app::TOKEN_HISTORY_CAP {
                state.token_history.pop_front();
            }
            state.token_history.push_back(turn_total);
        }

        // OpenWebUI outlet-filter notification — fire-and-forget POST to
        // `/api/chat/completed` so server-side filters (rate-limit
        // accounting, audit logs, chat-history persistence) see our
        // completion the same way they see a web-client completion.
        // Without this we look like a desync'd client to admins; on
        // chat.ai2s.org `rate_limit_inlet_filter` is globally active
        // and its outlet half wouldn't fire for us. Spawned as a
        // detached task so a slow OWUI ack never blocks the UI.
        if state.provider.name() == "openwebui"
            && let Some(sid) = state
                .current_session_id
                .as_ref()
                .map(|s| s.as_str().to_string())
        {
            let model = state.model.to_string();
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
    state.streaming_started_at = None;
    state.streaming_last_token_at = None;

    // v132 cost-budget surfacing. When the user has set a
    // session budget and we cross 80% / 100%, post a toast
    // once per threshold so they can choose to stop or
    // switch to a cheaper model. We never hard-block (an
    // in-flight investigation shouldn't be killed mid-turn
    // by an estimate); the toast is the user's signal.
    if let Some(budget_usd) = config::load_arc().session_cost_budget_usd
        && budget_usd > 0.0
    {
        let spent = crate::cost::total_cost(&state.usage_by_model);
        let pct = ((spent / budget_usd) * 100.0).round() as u8;
        let cross = |th: u8| pct >= th && state.cost_budget_warned_at < th;
        if cross(100) {
            state.cost_budget_warned_at = 100;
            crate::toast::push_with_cap(
                &mut state.toasts,
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
            state.cost_budget_warned_at = 80;
            crate::toast::push_with_cap(
                &mut state.toasts,
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
    if state.thinking_started_at.is_some() && state.thinking_ended_at.is_none() {
        state.thinking_ended_at = Some(std::time::Instant::now());
    }
    state.streaming_text = String::new();
    state.streaming_reasoning = String::new();
    // Only reset the cumulative token counter when the turn is
    // truly done. During agentic loops (ToolUse stop_reason), the
    // counter should keep accumulating so the spinner shows the
    // full turn's token estimate.
    if turn_done {
        state.streaming_response_bytes = 0;
        state.streaming_response_baseline = 0;
        state.streaming_thinking_tokens = 0;
        state.token_rate_samples.clear();
        state.token_rate_sample_thinking = None;
    }
    // Clear the user-turn clock only when the loop has
    // genuinely concluded — EndTurn stop reason AND no
    // tools pending. ToolUse means an agentic continuation
    // is about to fire and the turn timer must keep running.
    let terminal_blank_refusal = empty_billed_refusal_cap_reached;
    let turn_genuinely_done = (stop_reason == jfc_provider::StopReason::EndTurn
        || terminal_blank_refusal)
        && state.pending_approval.is_none()
        && state.approval_queue.is_empty()
        && state.pending_tool_calls.is_empty()
        && state.pending_classifications == 0
        && state.in_flight_eager_dispatches == 0
        && state.in_flight_tool_batches == 0
        // Keep the turn clock running while an AskUserQuestion modal is open —
        // the turn only genuinely ends once the user answers/declines.
        && state.pending_question.is_none();
    let needs_dynamic_loop_keepalive = turn_genuinely_done && dynamic_loop_keepalive_needed(state);
    if turn_genuinely_done {
        state.turn_started_at = None;
        crate::runtime::materialize_terminal_transcript_boundary(state);
    }

    // Auto-save session after each assistant turn completes. Turn
    // boundaries always persist immediately (never debounced) — crash
    // safety for completed turns is non-negotiable.
    crate::runtime::session_save::force_save(state);
    if turn_genuinely_done {
        crate::auto_review::maybe_spawn_after_turn(state, tx).await;
    }
    if stop_reason == jfc_provider::StopReason::EndTurn
        && turn_genuinely_done
        && dispatch_goal_evaluator_if_active(state, tx)
    {
        tracing::info!(
            target: "jfc::goal",
            "goal evaluator dispatched on plain EndTurn — deferring drain"
        );
        return;
    }
    // v126 queued-prompt drain on plain end_turn: model finished
    // without tools to call → if anything's queued, fire it now.
    if stop_reason == jfc_provider::StopReason::EndTurn
        && state.pending_approval.is_none()
        && state.approval_queue.is_empty()
        && state.pending_tool_calls.is_empty()
        // A queued prompt must not race ahead of an open AskUserQuestion modal —
        // the user's pending answer is the next input, not the queued prompt.
        && state.pending_question.is_none()
        && !state.queued_prompts.is_empty()
    {
        drain_queued_prompts(state, tx).await;
        // If the drain staged a fresh turn (non-meta prompt → new assistant
        // slot + live `streaming_assistant_idx`), bail out now. The cleanup
        // ladder below — specifically the EndTurn `else` arm — would otherwise
        // null out `streaming_assistant_idx`/`current_stream_request`, and the
        // newly-spawned stream's chunks would then arrive with no slot to
        // attach to (stream alive but no visible output). A meta-only drain
        // leaves `is_streaming` false and falls through to the normal cleanup.
        if state.is_streaming {
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
        !state.pending_tool_calls.is_empty() || state.in_flight_eager_dispatches > 0;
    let waiting_on_approval = state.pending_approval.is_some() || !state.approval_queue.is_empty();
    // Auto-mode: one or more tool calls are still awaiting an async classifier
    // verdict. We must NOT finalize the turn (which clears the streaming slot)
    // until every verdict lands — otherwise the late ClassifierDecision finds
    // no slot and the tool is silently dropped. The final verdict
    // (handle_classifier_decision) dispatches the approved batch / continues.
    let awaiting_classifier = state.pending_classifications > 0;
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
            n = state.pending_tool_calls.len(),
            approval_pending = waiting_on_approval,
            "mixed-mode pause_turn detected — latching pending_pause_turn_resume for AllComplete"
        );
        state.pending_pause_turn_resume = true;
    }
    if awaiting_classifier {
        // Hold the turn open: keep streaming_assistant_idx alive so the
        // pending ClassifierDecision events can record their tools and the
        // final one drives dispatch/continuation. This must take priority
        // over has_pending_tools so a partially-classified batch isn't
        // dispatched early (leaving later-approved tools stranded).
        tracing::info!(
            target: "jfc::stream",
            in_flight = state.pending_classifications,
            already_approved = state.pending_tool_calls.len(),
            "stream_done holding turn open for in-flight auto-mode classifier verdicts"
        );
    } else if has_pending_tools {
        if state.in_flight_eager_dispatches > 0 || state.in_flight_tool_batches > 0 {
            tracing::info!(
                target: "jfc::stream",
                pending = state.pending_tool_calls.len(),
                in_flight_eager = state.in_flight_eager_dispatches,
                in_flight_batches = state.in_flight_tool_batches,
                ?stop_reason,
                "stream_done waiting for in-flight eager tool prefix before dispatching remaining tools"
            );
        } else {
            super::stream_tool::dispatch_pending_after_stream(state, tx);
        }
    } else if waiting_on_approval {
        tracing::info!(
            target: "jfc::stream",
            pending_modal = state.pending_approval.is_some(),
            queue_depth = state.approval_queue.len(),
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
            streaming_idx = ?state.streaming_assistant_idx,
            "stream_done PauseTurn — resuming server-side sampling loop"
        );
        stream::continue_after_pause_turn(state, tx).await;
    } else if stop_reason == jfc_provider::StopReason::ToolUse {
        // Upstream returned finish_reason="tool_calls" but sent
        // zero tool_call delta chunks (transient LiteLLM/Bedrock
        // failure). The assistant message that was pre-pushed to
        // history is empty/malformed and un-replyable; strip it so the
        // retry/next user turn doesn't send a broken conversation turn.
        tracing::warn!(
            target: "jfc::stream",
            streaming_idx = ?state.streaming_assistant_idx,
            "stream_done ToolUse with no tools — stripping dangling assistant turn"
        );
        if let Some(idx) = state.streaming_assistant_idx
            && idx < state.messages.len()
        {
            let msg = &state.messages[idx];
            let is_empty = msg.parts.is_empty()
                || msg
                    .parts
                    .iter()
                    .all(|p| matches!(p, MessagePart::Text(t) if t.trim().is_empty()));
            if malformed_tool_use_clean_retry_enabled()
                && malformed_tool_use_retry_candidate(msg)
                && !malformed_tool_use_retry_already_attempted(&state.messages)
            {
                tracing::warn!(
                    target: "jfc::stream::tool_use",
                    assistant_idx = idx,
                    "malformed tool-use turn — tombstoning assistant response and retrying cleanly"
                );
                crate::toast::push_with_cap(
                    &mut state.toasts,
                    crate::toast::Toast::new(
                        crate::toast::ToastKind::Warning,
                        "Malformed tool call — retrying cleanly".to_owned(),
                    ),
                );
                state.messages.remove(idx);
                state.streaming_assistant_idx = None;
                state.current_stream_request = None;
                state.stream_lifecycle = None;
                let body = crate::system_reminder::format(&format!(
                    "{MALFORMED_TOOL_USE_RETRY_REMINDER}\n\n<{MALFORMED_TOOL_USE_RETRY_MARKER}/>"
                ));
                state.messages.push(types::ChatMessage::user(body));
                stream::continue_agentic_loop(state, tx).await;
                return;
            } else if is_empty {
                state.messages.remove(idx);
            } else if stream::should_continue_loop(&state.messages) {
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
                    streaming_idx = ?state.streaming_assistant_idx,
                    "stream_done ToolUse with only pre-resolved (denied/malformed) tools — continuing agentic loop"
                );
                stream::continue_agentic_loop(state, tx).await;
                return;
            }
        }
        state.streaming_assistant_idx = None;
        state.current_stream_request = None;
        state.stream_lifecycle = None;
        state.push_effect(crate::app::EngineEffect::ScrollToBottom);
    } else if stream::should_continue_loop(&state.messages) {
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
            streaming_idx = ?state.streaming_assistant_idx,
            "stream_done with only pre-resolved (denied/malformed) tools — continuing agentic loop"
        );
        stream::continue_agentic_loop(state, tx).await;
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
                    // We auto-resume from here (see maybe_self_continue's
                    // output_truncated path), so frame it as a continuation, not
                    // a dead end. If auto-continue is off or the cap is hit, the
                    // user can still nudge manually.
                    "Reply hit the max output-token limit — continuing it automatically."
                        .to_string()
                }
                jfc_provider::StopReason::Refusal if empty_billed_refusal_cap_reached => {
                    "Provider returned blank refusal responses repeatedly — waiting for you."
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
                &mut state.toasts,
                crate::toast::Toast::new(crate::toast::ToastKind::Warning, msg),
            );
        }
        state.streaming_assistant_idx = None;
        state.current_stream_request = None;
        state.stream_lifecycle = None;
        state.push_effect(crate::app::EngineEffect::ScrollToBottom);
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
            let stalled = stream::assistant_text_stalls(&state.messages);
            if stalled {
                state
                    .exploration_state
                    .bump_for_signal(crate::exploration::ExplorationSignal::AssistantStall);
            } else {
                let counts = state.task_store.counts();
                if counts.pending == 0 && counts.in_progress == 0 {
                    state.exploration_state.decay_after_progress();
                }
            }
        }
        let truncated = matches!(stop_reason, jfc_provider::StopReason::MaxTokens);
        maybe_self_continue(state, tx, truncated).await;
    }
    if needs_dynamic_loop_keepalive && !state.is_streaming {
        // Keepalive budget gate: the model declined to reschedule the dynamic
        // loop. Fire the fallback heartbeat only while budget remains; once
        // exhausted, end the loop rather than firing forever. Mirrors Claude
        // 2.1.177's `hc$() >= Wz5` guard (`tengu_loop_keepalive_fired` vs
        // `tengu_loop_ended` model_stopped).
        let budget_available = state
            .autonomous_loop
            .as_ref()
            .is_some_and(crate::autonomous_loop::AutonomousLoopState::keepalive_budget_available);
        if budget_available {
            schedule_dynamic_loop_keepalive();
            if let Some(loop_state) = state.autonomous_loop.as_mut() {
                loop_state.record_keepalive_fired();
            }
        } else {
            tracing::info!(
                target: "jfc::autonomous_loop",
                "tengu_loop_ended: keepalive budget exhausted (model declined to reschedule) — ending dynamic loop"
            );
            state.autonomous_loop = None;
        }
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
        MessagePart::ReasoningSignature(_) => false,
        MessagePart::RedactedThinking(_)
        | MessagePart::Tool(_)
        | MessagePart::TaskStatus(_)
        | MessagePart::CompactBoundary { .. } => true,
    })
}

fn malformed_tool_use_clean_retry_enabled() -> bool {
    for key in [
        "JFC_MALFORMED_TOOL_USE_CLEAN_RETRY",
        "CLAUDE_CODE_MALFORMED_TOOL_USE_CLEAN_RETRY",
    ] {
        if let Ok(value) = std::env::var(key) {
            let normalized = value.trim().to_ascii_lowercase();
            return !matches!(normalized.as_str(), "0" | "false" | "no" | "off");
        }
    }
    true
}

fn malformed_tool_use_retry_candidate(msg: &types::ChatMessage) -> bool {
    if msg
        .parts
        .iter()
        .any(|part| matches!(part, MessagePart::Tool(_)))
    {
        return false;
    }
    if assistant_turn_has_no_content(msg) {
        return true;
    }
    let text = assistant_turn_text(msg);
    crate::inline_tools::contains_inline_tools(&text) || !text.trim().is_empty()
}

fn malformed_tool_use_retry_already_attempted(messages: &[types::ChatMessage]) -> bool {
    messages.iter().rev().take(8).any(|message| {
        message.role == Role::User
            && message_text_contains(message, MALFORMED_TOOL_USE_RETRY_MARKER)
    })
}

fn assistant_turn_text(msg: &types::ChatMessage) -> String {
    msg.parts
        .iter()
        .filter_map(|part| match part {
            MessagePart::Text(text) | MessagePart::Reasoning(text) | MessagePart::Advisor(text) => {
                Some(text.as_str())
            }
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn message_text_contains(msg: &types::ChatMessage, needle: &str) -> bool {
    msg.parts.iter().any(|part| match part {
        MessagePart::Text(text) | MessagePart::Reasoning(text) | MessagePart::Advisor(text) => {
            text.contains(needle)
        }
        _ => false,
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

/// Max same-model resends for a content-bearing refusal before giving up and
/// surfacing the stall. Kept small (2) — a refusal that survives two clean
/// resends is probably a real refusal, not transient degradation. Overridable
/// via `JFC_REFUSAL_RESEND_CAP`.
fn refusal_resend_cap() -> u32 {
    std::env::var("JFC_REFUSAL_RESEND_CAP")
        .ok()
        .and_then(|s| s.parse::<u32>().ok())
        .unwrap_or(2)
}

/// Cap for the opt-in refusal→rewrite→resend loop. Configured value
/// (`refusal_rewrite_retry_max`), default 3, hard-clamped to 20 since each round
/// is a full extra request and a real refusal won't clear after a few tries.
fn refusal_rewrite_retry_cap(max: Option<u32>) -> u32 {
    max.unwrap_or(3).min(20)
}

/// Concatenate a message's `Text` parts (the user prompt body or refusal text).
fn message_text_parts(msg: &crate::types::ChatMessage) -> String {
    msg.parts
        .iter()
        .filter_map(|p| match p {
            crate::types::MessagePart::Text(t) => Some(t.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("")
}

/// Replace a message's `Text` parts with `text`, preserving non-text parts (e.g.
/// attachments) so a rephrased resend keeps the turn's other content.
fn set_message_text_parts(msg: &mut crate::types::ChatMessage, text: String) {
    msg.parts
        .retain(|p| !matches!(p, crate::types::MessagePart::Text(_)));
    msg.parts.insert(0, crate::types::MessagePart::Text(text));
}

struct RefusalDiagnosticInput<'a> {
    stop_reason: &'a jfc_provider::StopReason,
    stop_reason_refusal: bool,
    model_refusal: bool,
    assistant_idx: usize,
    text_chars: usize,
    reasoning_chars: usize,
    tool_parts: usize,
    out_tokens: u64,
    safe_to_discard: bool,
    thinking_only: bool,
    empty_billed_action: EmptyBilledAction,
    visible_response_text: &'a str,
    /// Concatenated visible reasoning/CoT of the refusing turn. Emitted ONLY to an
    /// ephemeral debug log and ONLY when `refusal_log_reasoning` is opted in; never
    /// written to the durable diagnostic record.
    reasoning_text: &'a str,
}

#[derive(Debug, Serialize)]
struct RefusalDiagnosticRecord {
    schema_version: u8,
    stop_reason: String,
    stop_reason_refusal: bool,
    model_refusal: bool,
    category: &'static str,
    provider: String,
    model: String,
    assistant_idx: usize,
    text_chars: usize,
    reasoning_chars: usize,
    tool_parts: usize,
    out_tokens: u64,
    safe_to_discard: bool,
    thinking_only: bool,
    empty_billed_action: &'static str,
    empty_billed_resend_count: u32,
    empty_billed_resend_cap: u32,
    refusal_resend_count: u32,
    refusal_resend_cap: u32,
    refusal_rewrite_retry_count: u32,
    refusal_rewrite_retry_cap: u32,
    streaming_response_bytes: usize,
    pending_tool_calls: usize,
    pending_classifications: usize,
    in_flight_eager_dispatches: usize,
    in_flight_tool_batches: usize,
    pending_approval: bool,
    approval_queue_depth: usize,
    input: RefusalInputDiagnostic,
    visible_response_preview: Option<String>,
    privacy_note: &'static str,
}

#[derive(Debug, Serialize)]
struct RefusalInputDiagnostic {
    user_idx: Option<usize>,
    user_text_chars: usize,
    user_part_count: usize,
    user_tool_parts: usize,
    user_attachment_count: usize,
    user_preview: Option<String>,
    conversation_messages: usize,
    advertised_tool_count: Option<usize>,
    action_expected: Option<bool>,
    tool_choice: Option<String>,
    request_resolved_model: Option<String>,
}

/// Whether refusal chain-of-thought debug logging is opted in via config.
/// Separated so the gate is a single, mockable read of the cached config.
fn refusal_reasoning_logging_enabled() -> bool {
    crate::config::load_arc().refusal_log_reasoning
}

/// Emit the refusing turn's reasoning/CoT and full visible response to an
/// ephemeral debug log. Pure side-effect helper; callers gate it on
/// [`refusal_reasoning_logging_enabled`]. Skips the log entirely when there is
/// no reasoning text to show, so a turn that refused without thinking doesn't
/// produce an empty line.
fn emit_refusal_reasoning_debug(input: &RefusalDiagnosticInput<'_>) {
    let cot = input.reasoning_text.trim();
    if cot.is_empty() {
        tracing::debug!(
            target: "jfc::stream::refusal_diagnostic",
            "refusal_log_reasoning=on but the refusing turn carried no reasoning/CoT to log"
        );
        return;
    }
    tracing::debug!(
        target: "jfc::stream::refusal_diagnostic",
        reasoning = %cot,
        visible_response = %input.visible_response_text,
        "refusal chain-of-thought (refusal_log_reasoning=on; ephemeral debug log, not persisted)"
    );
}

fn record_refusal_diagnostic(state: &EngineState, input: RefusalDiagnosticInput<'_>) {
    // Opt-in, local-debug only: surface the refusing turn's chain-of-thought (and
    // full visible response) to an EPHEMERAL debug log so a user can see *why* the
    // turn refused and how the rewrite chain adapts. This never touches the durable
    // record built below, which stays counts-only — preserving the privacy
    // guarantee for everyone who hasn't opted in.
    if refusal_reasoning_logging_enabled() {
        emit_refusal_reasoning_debug(&input);
    }
    let diagnostic = build_refusal_diagnostic(state, input);
    tracing::warn!(
        target: "jfc::stream::refusal_diagnostic",
        stop_reason = %diagnostic.stop_reason,
        category = diagnostic.category,
        provider = %diagnostic.provider,
        model = %diagnostic.model,
        out_tokens = diagnostic.out_tokens,
        text_chars = diagnostic.text_chars,
        reasoning_chars = diagnostic.reasoning_chars,
        input_user_idx = ?diagnostic.input.user_idx,
        input_user_chars = diagnostic.input.user_text_chars,
        input_user_parts = diagnostic.input.user_part_count,
        input_user_attachments = diagnostic.input.user_attachment_count,
        input_user_preview = ?diagnostic.input.user_preview.as_deref(),
        advertised_tool_count = ?diagnostic.input.advertised_tool_count,
        action_expected = ?diagnostic.input.action_expected,
        tool_choice = ?diagnostic.input.tool_choice.as_deref(),
        request_resolved_model = ?diagnostic.input.request_resolved_model.as_deref(),
        input_conversation_messages = diagnostic.input.conversation_messages,
        visible_response_preview = ?diagnostic.visible_response_preview.as_deref(),
        stop_reason_refusal = diagnostic.stop_reason_refusal,
        model_refusal = diagnostic.model_refusal,
        empty_billed_action = diagnostic.empty_billed_action,
        resend_count = diagnostic.empty_billed_resend_count,
        "provider refusal diagnostic captured"
    );
    if cfg!(test) {
        tracing::debug!(
            target: "jfc::stream::refusal_diagnostic",
            "test build; refusal diagnostic kept in logs only"
        );
        return;
    }
    let Some(session_id) = state.current_session_id.as_ref().map(ToString::to_string) else {
        tracing::debug!(
            target: "jfc::stream::refusal_diagnostic",
            "no current session id; refusal diagnostic kept in logs only"
        );
        return;
    };
    let Ok(value_json) = serde_json::to_string(&diagnostic) else {
        tracing::warn!(
            target: "jfc::stream::refusal_diagnostic",
            "failed to serialize refusal diagnostic"
        );
        return;
    };
    tokio::spawn(async move {
        let result: jfc_knowledge::Result<()> = async {
            let store = jfc_knowledge::KnowledgeStore::open_default().await?;
            store
                .append_session_artifact_event(
                    &session_id,
                    REFUSAL_DIAGNOSTIC_KIND,
                    REFUSAL_DIAGNOSTIC_KEY,
                    &value_json,
                )
                .await?;
            Ok(())
        }
        .await;
        if let Err(error) = result {
            tracing::warn!(
                target: "jfc::stream::refusal_diagnostic",
                error = %error,
                "failed to persist refusal diagnostic"
            );
        }
    });
}

fn build_refusal_diagnostic(
    state: &EngineState,
    input: RefusalDiagnosticInput<'_>,
) -> RefusalDiagnosticRecord {
    RefusalDiagnosticRecord {
        schema_version: 1,
        stop_reason: format!("{:?}", input.stop_reason),
        stop_reason_refusal: input.stop_reason_refusal,
        model_refusal: input.model_refusal,
        category: refusal_diagnostic_category(input.safe_to_discard, input.out_tokens),
        provider: state.provider.name().to_owned(),
        model: state.model.to_string(),
        assistant_idx: input.assistant_idx,
        text_chars: input.text_chars,
        reasoning_chars: input.reasoning_chars,
        tool_parts: input.tool_parts,
        out_tokens: input.out_tokens,
        safe_to_discard: input.safe_to_discard,
        thinking_only: input.thinking_only,
        empty_billed_action: empty_billed_action_label(input.empty_billed_action),
        empty_billed_resend_count: state.empty_billed_resend_count,
        empty_billed_resend_cap: empty_billed_resend_cap(),
        refusal_resend_count: state.refusal_resend_count,
        refusal_resend_cap: refusal_resend_cap(),
        refusal_rewrite_retry_count: state.refusal_rewrite_retry_count,
        refusal_rewrite_retry_cap: refusal_rewrite_retry_cap(state.refusal_rewrite_retry_max),
        streaming_response_bytes: state.streaming_response_bytes,
        pending_tool_calls: state.pending_tool_calls.len(),
        pending_classifications: state.pending_classifications,
        in_flight_eager_dispatches: state.in_flight_eager_dispatches,
        in_flight_tool_batches: state.in_flight_tool_batches,
        pending_approval: state.pending_approval.is_some(),
        approval_queue_depth: state.approval_queue.len(),
        input: build_refusal_input_diagnostic(state, input.assistant_idx),
        visible_response_preview: visible_refusal_preview(input.visible_response_text),
        privacy_note: "private reasoning is intentionally not persisted; only counts and visible text preview are stored",
    }
}

fn build_refusal_input_diagnostic(
    state: &EngineState,
    assistant_idx: usize,
) -> RefusalInputDiagnostic {
    let user_idx = state.messages[..assistant_idx.min(state.messages.len())]
        .iter()
        .rposition(|m| matches!(m.role, crate::types::Role::User));
    let (user_text_chars, user_part_count, user_tool_parts, user_attachment_count, user_preview) =
        if let Some(idx) = user_idx {
            let msg = &state.messages[idx];
            let text = message_text_parts(msg);
            (
                text.chars().count(),
                msg.parts.len(),
                msg.parts
                    .iter()
                    .filter(|part| matches!(part, crate::types::MessagePart::Tool(_)))
                    .count(),
                msg.attachments.len(),
                redacted_input_preview(&text),
            )
        } else {
            (0, 0, 0, 0, None)
        };
    let request = state.current_stream_request.as_ref();
    RefusalInputDiagnostic {
        user_idx,
        user_text_chars,
        user_part_count,
        user_tool_parts,
        user_attachment_count,
        user_preview,
        conversation_messages: state.messages.len(),
        advertised_tool_count: request.map(|meta| meta.advertised_tool_count),
        action_expected: request.map(|meta| meta.action_expected),
        tool_choice: request.map(|meta| format!("{:?}", meta.tool_choice)),
        request_resolved_model: request
            .and_then(|meta| meta.resolved_model.as_ref())
            .map(|resolved| resolved.effective.to_string()),
    }
}

fn refusal_diagnostic_category(safe_to_discard: bool, out_tokens: u64) -> &'static str {
    if safe_to_discard && out_tokens > 0 {
        "blank_billed_refusal"
    } else if safe_to_discard {
        "blank_unbilled_refusal"
    } else {
        "content_refusal"
    }
}

fn empty_billed_action_label(action: EmptyBilledAction) -> &'static str {
    match action {
        EmptyBilledAction::DiscardAndResend => "discard_and_resend",
        EmptyBilledAction::CapReached => "cap_reached",
        EmptyBilledAction::WarnOnly => "warn_only",
        EmptyBilledAction::ResetBudget => "reset_budget",
        EmptyBilledAction::None => "none",
    }
}

fn visible_refusal_preview(text: &str) -> Option<String> {
    bounded_preview(text, REFUSAL_VISIBLE_PREVIEW_CHARS)
}

fn redacted_input_preview(text: &str) -> Option<String> {
    let stripped = jfc_core::strip_system_reminders(text);
    let redacted = jfc_knowledge::redact::redact(&stripped, false);
    bounded_preview(&redacted, REFUSAL_INPUT_PREVIEW_CHARS)
}

fn bounded_preview(text: &str, max_chars: usize) -> Option<String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }
    let mut preview = trimmed.chars().take(max_chars).collect::<String>();
    if trimmed.chars().count() > max_chars {
        preview.push_str("...");
    }
    Some(preview)
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

fn dynamic_loop_keepalive_needed(state: &EngineState) -> bool {
    if !crate::autonomous_loop::loop_keepalive_enabled() {
        return false;
    }
    let Some(loop_state) = state.autonomous_loop.as_ref() else {
        return false;
    };
    if loop_state.pacing != crate::autonomous_loop::LoopPacing::Dynamic {
        return false;
    }
    let Some(idx) = state.streaming_assistant_idx else {
        return false;
    };
    let Some(message) = state.messages.get(idx) else {
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
    use crate::app::EngineState;
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

    // Normal: the refusal resend cap is small and positive (bounded recovery,
    // not an infinite retry loop).
    #[test]
    fn refusal_resend_cap_is_bounded_normal() {
        let cap = refusal_resend_cap();
        assert!(cap >= 1, "must allow at least one recovery retry");
        assert!(cap <= 5, "must stay small so a real refusal isn't hammered");
    }

    #[test]
    fn malformed_tool_retry_guard_detects_marker_normal() {
        let body = crate::system_reminder::format(&format!(
            "retrying\n\n<{MALFORMED_TOOL_USE_RETRY_MARKER}/>"
        ));
        let messages = vec![
            ChatMessage::user("run a tool".into()),
            ChatMessage::user(body),
        ];
        assert!(malformed_tool_use_retry_already_attempted(&messages));
    }

    // Robust: JFC_REFUSAL_RESEND_CAP overrides the default, and a bad value
    // falls back to the default (env is restored so parallel tests are safe).
    #[serial_test::serial]
    #[serial_test::serial]
    #[test]
    fn refusal_resend_cap_env_override_robust() {
        const KEY: &str = "JFC_REFUSAL_RESEND_CAP";
        let prev = std::env::var(KEY).ok();
        unsafe { std::env::set_var(KEY, "4") };
        assert_eq!(refusal_resend_cap(), 4);
        unsafe { std::env::set_var(KEY, "not-a-number") };
        assert_eq!(refusal_resend_cap(), 2, "bad value falls back to default");
        unsafe {
            match &prev {
                Some(v) => std::env::set_var(KEY, v),
                None => std::env::remove_var(KEY),
            }
        }
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

    #[serial_test::serial]
    #[tokio::test]
    async fn malformed_tool_use_is_tombstoned_and_retried_once_normal() {
        const KEY: &str = "JFC_MALFORMED_TOOL_USE_CLEAN_RETRY";
        let prev = std::env::var(KEY).ok();
        unsafe { std::env::set_var(KEY, "1") };

        let mut state = EngineState::new(Arc::new(TestProvider), "test-model");
        state.task_store = jfc_session::TaskStore::in_memory();
        state.messages.push(ChatMessage::user("run ls".into()));
        state
            .messages
            .push(ChatMessage::assistant("<tool_use>bad</tool_use>".into()));
        state.streaming_assistant_idx = Some(1);
        state.is_streaming = true;
        let (tx, _rx) = tokio::sync::mpsc::channel(8);

        handle_stream_done(&mut state, &tx, jfc_provider::StopReason::ToolUse).await;

        assert!(
            !state
                .messages
                .iter()
                .any(|message| message_text_contains(message, "<tool_use>bad</tool_use>")),
            "malformed assistant turn must be removed before retry"
        );
        assert!(
            state
                .messages
                .iter()
                .any(|message| message_text_contains(message, MALFORMED_TOOL_USE_RETRY_MARKER)),
            "retry marker user reminder must be present"
        );
        assert!(
            state.is_streaming,
            "clean retry should stage a fresh stream"
        );

        unsafe {
            match prev {
                Some(value) => std::env::set_var(KEY, value),
                None => std::env::remove_var(KEY),
            }
        }
    }

    #[tokio::test]
    async fn pending_classifier_keeps_turn_clock_active_robust() {
        let mut state = EngineState::new(Arc::new(TestProvider), "test-model");
        state.task_store = jfc_session::TaskStore::in_memory();
        state.is_streaming = true;
        state.turn_started_at = Some(Instant::now());
        state.pending_classifications = 1;
        let (tx, _rx) = tokio::sync::mpsc::channel(8);

        handle_stream_done(&mut state, &tx, jfc_provider::StopReason::EndTurn).await;

        assert!(
            state.turn_started_at.is_some(),
            "classifier verdicts are still in flight, so the user turn must stay open"
        );
        assert!(!state.is_streaming);
    }

    #[tokio::test]
    async fn stream_done_clears_active_stream_handle_robust() {
        let mut state = EngineState::new(Arc::new(TestProvider), "test-model");
        state.task_store = jfc_session::TaskStore::in_memory();
        state.is_streaming = true;
        let handle = tokio::spawn(async {
            std::future::pending::<()>().await;
        });
        state.active_stream_handle = Some(handle.abort_handle());
        let (tx, _rx) = tokio::sync::mpsc::channel(8);

        handle_stream_done(&mut state, &tx, jfc_provider::StopReason::EndTurn).await;

        assert!(state.active_stream_handle.is_none());
        assert!(!state.has_interruptible_work());
        handle.abort();
    }

    #[tokio::test]
    async fn end_turn_with_active_goal_dispatches_evaluator_before_stopping_regression() {
        // Given: a text-only assistant turn is ending while a session goal is active.
        let mut state = EngineState::new(Arc::new(TestProvider), "test-model");
        state.task_store = jfc_session::TaskStore::in_memory();
        state.is_streaming = true;
        state.goal = Some(crate::goal::ActiveGoal::new("finish the task".to_owned()));
        state.messages.push(ChatMessage::user("start".into()));
        state
            .messages
            .push(ChatMessage::assistant("working".into()));
        state.streaming_assistant_idx = Some(1);
        let (tx, _rx) = tokio::sync::mpsc::channel(8);

        // When: the provider reports a plain EndTurn with no tools pending.
        handle_stream_done(&mut state, &tx, jfc_provider::StopReason::EndTurn).await;

        // Then: the goal stop-hook evaluator owns the next step instead of the
        // turn stopping as if no goal existed.
        assert!(
            state.goal_evaluator_in_flight,
            "plain EndTurn must dispatch the /goal evaluator"
        );
        assert_eq!(
            state.goal_evaluator_epoch_in_flight,
            state.goal.as_ref().map(|goal| goal.epoch)
        );
    }

    /// Scripted provider whose `complete()` drives the over-refusal rewrite gate
    /// (classifier→rewriter→verifier) to a clarifying rewrite, so the
    /// empty-refusal→rewrite→resend loop is testable end-to-end. `stream()` is the
    /// inert empty stream (the resend's stream body isn't asserted here).
    struct RewriteRetryProvider;
    #[async_trait::async_trait]
    impl Provider for RewriteRetryProvider {
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
        async fn complete(
            &self,
            _messages: Vec<ProviderMessage>,
            options: &StreamOptions,
        ) -> anyhow::Result<jfc_provider::CompletionResponse> {
            let sys = options.system.clone().unwrap_or_default();
            let content = if sys.starts_with("You are a response refusal classifier") {
                r#"{"verdict":"refused","confidence":0.92,"rationale":"declined the request"}"#
            } else if sys.starts_with("You are a safety intent classifier") {
                r#"{"goal_category":"other","verdict":"allowed","confidence":0.9}"#
            } else if sys.starts_with("You rewrite") {
                r#"{"original_intent":"tcp server","text":"Clarified request: write a TCP echo server in Rust using only the standard library.","rationale":"clarified scope"}"#
            } else {
                r#"{"intent_preserved":true,"introduced_harm":false}"#
            };
            Ok(jfc_provider::CompletionResponse {
                content: content.to_string(),
                usage: jfc_provider::TokenUsage::default(),
                context_signals: None,
                reasoning: None,
            })
        }
    }
    impl jfc_provider::seal::Sealed for RewriteRetryProvider {}

    /// State with an empty-but-billed REFUSAL turn (the blank bubble) plus the
    /// over-refusal rewrite gate enabled, so the loop can run end-to-end.
    fn app_with_blank_refusal_and_gate(loop_enabled: bool) -> EngineState {
        let mut state = EngineState::new(Arc::new(RewriteRetryProvider), "test-model");
        state.task_store = jfc_session::TaskStore::in_memory();
        state.refusal_rewrite_retry_enabled = loop_enabled;
        state.prompt_rewrite = Some(jfc_config::PromptRewriteConfig {
            enabled: true,
            model: None,
            threshold: None,
            constitution: None,
        });
        state
            .messages
            .push(ChatMessage::user("write a tcp echo server in rust".into()));
        let mut blank = ChatMessage::assistant(String::new());
        blank.usage = Some(ModelUsage {
            output_tokens: 48,
            ..Default::default()
        });
        state.messages.push(blank);
        state.streaming_assistant_idx = Some(1);
        state.is_streaming = true;
        state
    }

    fn app_with_content_refusal_and_gate(loop_enabled: bool) -> EngineState {
        let mut state = EngineState::new(Arc::new(RewriteRetryProvider), "test-model");
        state.task_store = jfc_session::TaskStore::in_memory();
        state.refusal_rewrite_retry_enabled = loop_enabled;
        state.prompt_rewrite = Some(jfc_config::PromptRewriteConfig {
            enabled: true,
            model: None,
            threshold: None,
            constitution: None,
        });
        state
            .messages
            .push(ChatMessage::user("write a tcp echo server in rust".into()));
        let mut refusal = ChatMessage::assistant("I cannot help with that.".into());
        refusal.usage = Some(ModelUsage {
            output_tokens: 32,
            ..Default::default()
        });
        state.messages.push(refusal);
        state.streaming_assistant_idx = Some(1);
        state.is_streaming = true;
        state
    }

    // The reported bug ("multiple blanks"): the provider returns an EMPTY refusal
    // (blank bubble, stop_reason=Refusal, no body) and the engine plain-resends
    // the same prompt forever. With the opt-in loop enabled, an empty refusal must
    // instead route through the rewrite gate and resend a *clarified* prompt.
    #[tokio::test]
    async fn empty_refusal_rewrites_and_resends_when_enabled() {
        let mut state = app_with_blank_refusal_and_gate(true);
        let (tx, _rx) = tokio::sync::mpsc::channel(8);

        handle_stream_done(&mut state, &tx, jfc_provider::StopReason::Refusal).await;

        assert_eq!(
            state.refusal_rewrite_retry_count, 1,
            "one rewrite-retry consumed"
        );
        assert!(
            state.is_streaming,
            "a fresh stream must be staged for the resend"
        );
        // The user prompt was replaced with the gate's clarification for the resend.
        let user = state
            .messages
            .iter()
            .find(|m| matches!(m.role, crate::types::Role::User))
            .expect("user turn present");
        assert!(
            message_text_contains(user, "Clarified request"),
            "the resent prompt must be the gate's clarification"
        );
        // The pristine original is pinned at index 0 for subsequent rounds.
        assert_eq!(state.refusal_rewrite_attempts.len(), 2);
        assert_eq!(
            state.refusal_rewrite_attempts[0],
            "write a tcp echo server in rust"
        );
    }

    // Regression: with the loop OFF (default), an empty refusal must NOT be
    // rephrased — it falls through to the existing empty-billed ladder and the
    // user's prompt is left untouched (no silent rewrite when not opted in).
    #[tokio::test]
    async fn empty_refusal_not_rewritten_when_disabled() {
        let mut state = app_with_blank_refusal_and_gate(false);
        let (tx, _rx) = tokio::sync::mpsc::channel(8);

        handle_stream_done(&mut state, &tx, jfc_provider::StopReason::Refusal).await;

        assert_eq!(
            state.refusal_rewrite_retry_count, 0,
            "loop off → no rewrite"
        );
        assert!(state.refusal_rewrite_attempts.is_empty());
        let user = state
            .messages
            .iter()
            .find(|m| matches!(m.role, crate::types::Role::User))
            .expect("user turn present");
        assert!(
            message_text_contains(user, "write a tcp echo server in rust"),
            "the user prompt must be unchanged when the loop is off"
        );
    }

    #[tokio::test]
    async fn semantic_content_refusal_rewrites_when_classifier_flags() {
        let mut state = app_with_content_refusal_and_gate(true);
        let (tx, _rx) = tokio::sync::mpsc::channel(8);

        handle_stream_done(&mut state, &tx, jfc_provider::StopReason::EndTurn).await;

        assert_eq!(
            state.refusal_rewrite_retry_count, 1,
            "classifier-detected content refusal consumes one rewrite retry"
        );
        assert!(
            state.is_streaming,
            "a classifier-detected refusal should stage a fresh stream"
        );
        let user = state
            .messages
            .iter()
            .find(|m| matches!(m.role, crate::types::Role::User))
            .expect("user turn present");
        assert!(
            message_text_contains(user, "Clarified request"),
            "the resent prompt must be the gate's clarification"
        );
        assert!(
            !state
                .messages
                .iter()
                .any(|m| message_text_contains(m, "I cannot help with that.")),
            "the refused assistant turn must be removed before retry"
        );
    }

    /// Build an `EngineState` whose streaming slot is an empty-but-billed assistant
    /// turn (no text/tools/reasoning, but `output_tokens > 0`) — the exact
    /// shape `handle_stream_done` must discard.
    fn app_with_empty_billed_turn() -> EngineState {
        let mut state = EngineState::new(Arc::new(TestProvider), "test-model");
        state.task_store = jfc_session::TaskStore::in_memory();
        state.messages.push(ChatMessage::user("hello".into()));
        let mut blank = ChatMessage::assistant(String::new());
        blank.usage = Some(ModelUsage {
            output_tokens: 64,
            ..Default::default()
        });
        state.messages.push(blank);
        state.streaming_assistant_idx = Some(1);
        state.is_streaming = true;
        state
    }

    // Builds a state whose latest assistant turn HAS content (text) + usage —
    // the "content-bearing refusal" shape that previously dead-stopped.
    fn app_with_content_turn() -> EngineState {
        let mut state = EngineState::new(Arc::new(TestProvider), "test-model");
        state.task_store = jfc_session::TaskStore::in_memory();
        state.messages.push(ChatMessage::user("hello".into()));
        let mut msg = ChatMessage::assistant("I can't help with that.".into());
        msg.usage = Some(ModelUsage {
            output_tokens: 32,
            ..Default::default()
        });
        state.messages.push(msg);
        state.streaming_assistant_idx = Some(1);
        state.is_streaming = true;
        state
    }

    #[test]
    fn refusal_diagnostic_persists_counts_not_private_reasoning_regression() {
        let mut state = app_with_content_turn();
        state.messages[0] = ChatMessage::user(
            "please explain why this fails token=sk-proj-0123456789abcdef\n\
             <system-reminder>internal-only hint</system-reminder>"
                .into(),
        );
        state.current_stream_request = Some(crate::runtime::StreamRequestMetadata {
            advertised_tool_count: 7,
            action_expected: true,
            tool_choice: crate::runtime::StreamToolChoice::Auto,
            resolved_model: None,
            context_budget: None,
            provider_history_archive_recall_ids: Vec::new(),
            rsi_prompt_sections: 0,
            rsi_tool_visibility_rules: 0,
        });
        let hidden_reasoning = "private diagnostic chain";
        let (visible, reasoning_chars) = {
            let msg = state.messages.get_mut(1).expect("assistant turn present");
            msg.parts
                .push(MessagePart::Reasoning(hidden_reasoning.into()));
            let visible = message_text_parts(msg);
            let reasoning_chars = msg
                .parts
                .iter()
                .filter_map(|part| match part {
                    MessagePart::Reasoning(text) => Some(text.chars().count()),
                    _ => None,
                })
                .sum();
            (visible, reasoning_chars)
        };

        let diagnostic = build_refusal_diagnostic(
            &state,
            RefusalDiagnosticInput {
                stop_reason: &jfc_provider::StopReason::Refusal,
                stop_reason_refusal: true,
                model_refusal: false,
                assistant_idx: 1,
                text_chars: visible.chars().count(),
                reasoning_chars,
                tool_parts: 0,
                out_tokens: 32,
                safe_to_discard: false,
                thinking_only: false,
                empty_billed_action: EmptyBilledAction::ResetBudget,
                visible_response_text: &visible,
                reasoning_text: hidden_reasoning,
            },
        );
        let json = serde_json::to_string(&diagnostic).expect("diagnostic serializes");

        assert!(
            !json.contains(hidden_reasoning),
            "private reasoning text must never be persisted: {json}"
        );
        assert!(
            json.contains("\"reasoning_chars\":24"),
            "diagnostic should keep only the reasoning length: {json}"
        );
        assert!(
            json.contains("I can't help with that."),
            "visible refusal text preview is safe to persist: {json}"
        );
        assert!(
            json.contains("please explain why this fails token=[REDACTED]"),
            "diagnostic should persist a redacted last-user preview: {json}"
        );
        assert!(
            !json.contains("sk-proj-0123456789abcdef")
                && !json.contains("internal-only hint")
                && !json.contains("<system-reminder>"),
            "input preview must redact secrets and strip system reminders: {json}"
        );
        assert!(
            json.contains("\"advertised_tool_count\":7")
                && json.contains("\"action_expected\":true"),
            "diagnostic should include request-shape metadata: {json}"
        );
        assert!(
            json.contains("private reasoning is intentionally not persisted"),
            "diagnostic should document the privacy boundary: {json}"
        );
    }

    // The durable record stays counts-only regardless of the opt-in flag; the CoT
    // only ever travels the ephemeral debug-log path. This pins the two channels
    // apart: `build_refusal_diagnostic` never reads `reasoning_text`.
    #[test]
    fn refusal_diagnostic_record_ignores_reasoning_text_field() {
        let state = app_with_content_turn();
        let secret = "chain-of-thought that must not be serialized";
        let diagnostic = build_refusal_diagnostic(
            &state,
            RefusalDiagnosticInput {
                stop_reason: &jfc_provider::StopReason::Refusal,
                stop_reason_refusal: true,
                model_refusal: false,
                assistant_idx: 1,
                text_chars: 4,
                reasoning_chars: secret.chars().count(),
                tool_parts: 0,
                out_tokens: 1,
                safe_to_discard: false,
                thinking_only: false,
                empty_billed_action: EmptyBilledAction::None,
                visible_response_text: "I can't help with that.",
                reasoning_text: secret,
            },
        );
        let json = serde_json::to_string(&diagnostic).expect("serializes");
        assert!(
            !json.contains(secret),
            "reasoning_text must never reach the durable record: {json}"
        );
    }

    // The ephemeral CoT log is strictly gated: it fires only when a turn has
    // reasoning AND (at the call site) the opt-in flag is on. `emit_*` is a pure
    // side-effect; here we exercise the empty-CoT branch (no panic, early return)
    // and the populated branch to lock the helper's shape.
    #[test]
    fn emit_refusal_reasoning_debug_handles_empty_and_present() {
        let base = RefusalDiagnosticInput {
            stop_reason: &jfc_provider::StopReason::Refusal,
            stop_reason_refusal: true,
            model_refusal: false,
            assistant_idx: 0,
            text_chars: 0,
            reasoning_chars: 0,
            tool_parts: 0,
            out_tokens: 0,
            safe_to_discard: false,
            thinking_only: false,
            empty_billed_action: EmptyBilledAction::None,
            visible_response_text: "I can't help with that.",
            reasoning_text: "   ",
        };
        // Empty/whitespace CoT: early-returns without emitting the populated line.
        emit_refusal_reasoning_debug(&base);
        // Present CoT: emits the populated line. (Assertion is no-panic; tracing
        // capture is covered by integration-level subscribers, not this unit.)
        let present = RefusalDiagnosticInput {
            reasoning_text: "the model reasoned then declined",
            ..base
        };
        emit_refusal_reasoning_debug(&present);
    }

    // REGRESSION (content-bearing refusal dead-stop): a refusal that produced
    // text used to neither empty-billed-resend (it has content) nor self-
    // continue (Refusal excluded) — so it stalled. It must now auto-resend on
    // the same model, bounded by the cap.
    #[serial_test::serial]
    #[tokio::test]
    async fn content_refusal_auto_resends_bounded_robust() {
        unsafe { std::env::set_var("JFC_AUTO_CONTINUE", "1") };
        let (tx, _rx) = tokio::sync::mpsc::channel(8);

        // First refusal → one resend consumed, the refused message removed and
        // a fresh stream staged (the recovery).
        let mut state = app_with_content_turn();
        handle_stream_done(&mut state, &tx, jfc_provider::StopReason::Refusal).await;
        assert_eq!(
            state.refusal_resend_count, 1,
            "first content refusal resends once"
        );
        assert!(
            !state
                .messages
                .iter()
                .any(|m| m.usage.as_ref().is_some_and(|u| u.output_tokens == 32)),
            "the refused (billed) message was removed for the resend"
        );

        // Already at the cap: must NOT resend again — the refused message is
        // retained and the turn finalizes (bounded recovery, no infinite loop).
        let mut state2 = app_with_content_turn();
        state2.refusal_resend_count = refusal_resend_cap();
        handle_stream_done(&mut state2, &tx, jfc_provider::StopReason::Refusal).await;
        assert!(
            state2.refusal_resend_count <= refusal_resend_cap(),
            "never exceeds the cap"
        );
        assert!(
            state2
                .messages
                .iter()
                .any(|m| m.usage.as_ref().is_some_and(|u| u.output_tokens == 32)),
            "at the cap the refused message is retained (not resent), left for the user"
        );
        unsafe { std::env::remove_var("JFC_AUTO_CONTINUE") };
    }

    // Normal: with auto-continue on, an empty-but-billed EndTurn is discarded
    // (the blank assistant message is removed → no blank bubble, no
    // empty_message invariant on save) and a resend is staged. Asserting the
    // removal is the cross-cutting fix for BOTH symptoms the user named.
    #[serial_test::serial]
    #[tokio::test]
    async fn empty_billed_turn_is_discarded_and_resent_normal() {
        unsafe { std::env::set_var("JFC_AUTO_CONTINUE", "1") };
        let mut state = app_with_empty_billed_turn();
        let (tx, _rx) = tokio::sync::mpsc::channel(8);

        handle_stream_done(&mut state, &tx, jfc_provider::StopReason::EndTurn).await;

        // The blank assistant turn was removed and a fresh slot staged by
        // continue_agentic_loop. The user turn survives, and the *billed*
        // empty message (output_tokens=64) is gone — only the new empty
        // streaming slot (no usage) remains, so no empty_message invariant
        // can fire on the next save.
        assert_eq!(state.empty_billed_resend_count, 1, "resend budget consumed");
        assert_eq!(
            state
                .messages
                .iter()
                .filter(|m| m.role == Role::User)
                .count(),
            1,
            "user turn preserved"
        );
        assert!(
            !state
                .messages
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
        let mut state = app_with_empty_billed_turn();
        let (tx, _rx) = tokio::sync::mpsc::channel(8);

        handle_stream_done(
            &mut state,
            &tx,
            jfc_provider::StopReason::Other("stream_error".into()),
        )
        .await;

        assert_eq!(state.empty_billed_resend_count, 1, "Other(_) must resend");
        assert!(
            !state
                .messages
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
        let mut state = app_with_empty_billed_turn();
        let (tx, _rx) = tokio::sync::mpsc::channel(8);

        handle_stream_done(&mut state, &tx, jfc_provider::StopReason::Refusal).await;

        assert_eq!(
            state.empty_billed_resend_count, 1,
            "empty-billed refusal should trigger one capped resend"
        );
        assert!(
            !state
                .messages
                .iter()
                .any(|m| m.usage.as_ref().is_some_and(|u| u.output_tokens == 64)),
            "the billed empty assistant message must have been removed"
        );
        unsafe { std::env::remove_var("JFC_AUTO_CONTINUE") };
    }

    // Regression: after the bounded resend ladder is exhausted, a blank,
    // billed provider refusal is a provider-output failure, not a semantic
    // content refusal. The toast should name the blank-refusal condition so the
    // user does not chase a nonexistent policy refusal.
    #[serial_test::serial]
    #[tokio::test]
    async fn empty_billed_refusal_cap_toast_names_blank_provider_failure_regression() {
        unsafe { std::env::set_var("JFC_AUTO_CONTINUE", "1") };
        let mut state = app_with_empty_billed_turn();
        state.empty_billed_resend_count = empty_billed_resend_cap();
        state.turn_started_at = Some(Instant::now());
        let (tx, _rx) = tokio::sync::mpsc::channel(8);

        handle_stream_done(&mut state, &tx, jfc_provider::StopReason::Refusal).await;

        assert!(
            state.toasts.iter().any(|t| t
                .text
                .contains("Provider returned blank refusal responses repeatedly")),
            "cap-reached blank refusals need a provider-failure toast, got {:?}",
            state
                .toasts
                .iter()
                .map(|t| t.text.as_str())
                .collect::<Vec<_>>()
        );
        assert!(
            !state
                .toasts
                .iter()
                .any(|t| t.text == "The model refused this request."),
            "blank refusal cap must not surface as a normal semantic refusal"
        );
        assert!(
            state.turn_started_at.is_none(),
            "blank refusal cap is terminal; the TUI must not keep showing \
             `Working between model/tool steps`"
        );
        unsafe { std::env::remove_var("JFC_AUTO_CONTINUE") };
    }

    // Robust: with auto-continue OFF, we do NOT auto-resend (no silent
    // re-spend), and the budget stays at 0. The turn is left for the user.
    #[serial_test::serial]
    #[tokio::test]
    async fn empty_billed_turn_not_resent_when_auto_continue_off_robust() {
        unsafe { std::env::set_var("JFC_AUTO_CONTINUE", "0") };
        let mut state = app_with_empty_billed_turn();
        let (tx, _rx) = tokio::sync::mpsc::channel(8);

        handle_stream_done(&mut state, &tx, jfc_provider::StopReason::EndTurn).await;

        assert_eq!(
            state.empty_billed_resend_count, 0,
            "must not resend when auto-continue is disabled"
        );
        assert!(!state.is_streaming, "turn finalized, left for the user");
        unsafe { std::env::remove_var("JFC_AUTO_CONTINUE") };
    }
}

/// Auto-drive the next in-scope step when the model stalls or leaves work
/// queued, instead of forcing a manual "continue". Gated on `auto_continue`
/// (env/config/factory), disabled in plan mode, and capped by
/// `max_self_continuations` to prevent runaway loops.
async fn maybe_self_continue(state: &mut EngineState, tx: &EventSender, output_truncated: bool) {
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
        is_streaming: state.is_streaming,
        pending_approval: state.pending_approval.is_some(),
        approval_queue_len: state.approval_queue.len(),
        pending_tool_calls_len: state.pending_tool_calls.len(),
        queued_prompts_len: state.queued_prompts.len(),
        pending_classifications: state.pending_classifications,
        in_flight_eager_dispatches: state.in_flight_eager_dispatches,
        in_flight_tool_batches: state.in_flight_tool_batches,
    };
    if !turn_is_idle_for_self_continue(&idle) {
        return;
    }
    // Plan mode is read-only by contract — never auto-act.
    if matches!(state.permission_mode, crate::app::PermissionMode::Plan) {
        return;
    }
    if !stream::auto_continue_enabled() {
        return;
    }

    // Is there a reason to continue? Either unfinished queued tasks, the model
    // ended on a permission-asking stall, OR the response was truncated by the
    // output-token cap (stop_reason=max_tokens). The last case is the "I hit
    // 128k mid-answer and you had to type continue" bug: output-budget
    // truncation unambiguously means "more to write," so we resume from where
    // the reply was cut off rather than waiting for a manual nudge. (Claude Code
    // surfaces max_tokens as a hard error and stops; jfc has a bounded
    // self-continuation loop, so it can safely auto-resume — the same
    // self_continuation_count cap below still bounds it.)
    let counts = state.task_store.counts();
    let tasks_remain = counts.pending > 0 || counts.in_progress > 0;
    let stalled = stream::assistant_text_stalls(&state.messages);
    if !tasks_remain && !stalled && !output_truncated {
        return;
    }

    // Cap consecutive self-continuations.
    let max = stream::max_self_continuations();
    if state.self_continuation_count >= max {
        tracing::info!(
            target: "jfc::stream",
            count = state.self_continuation_count,
            max,
            "self-continuation cap reached — waiting for user"
        );
        return;
    }
    state.self_continuation_count += 1;

    tracing::info!(
        target: "jfc::stream",
        count = state.self_continuation_count,
        tasks_remain,
        stalled,
        output_truncated,
        pending_tasks = counts.pending,
        in_progress = counts.in_progress,
        "self-continuing without user nudge"
    );

    // Inject a system-reminder nudge as a fresh user turn. Phrased to match
    // the operating rule: finish the scope, don't ask permission for the next
    // in-scope step.
    let reason = if output_truncated {
        // The previous reply was cut off at the output-token cap. Resume it
        // seamlessly — the truncated text is already in the transcript, so the
        // model should continue from exactly where it stopped, not restart.
        "Your previous response was cut off because it reached the maximum output \
         length. Continue it from exactly where it stopped — do not repeat what you \
         already wrote, and do not restart. Pick up mid-sentence if needed."
            .to_string()
    } else if tasks_remain {
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
    state.messages.push(types::ChatMessage::user(body));
    stream::continue_agentic_loop(state, tx).await;
}

/// Snapshot of the runtime fields `maybe_self_continue` inspects to decide
/// whether the turn is fully idle. Extracted into a value-only struct so the
/// predicate can be unit-tested without spinning up an `EngineState`.
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

    // Normal — REGRESSION (the "hit 128k mid-answer, had to type continue" bug):
    // MaxTokens must be eligible for self-continuation so a truncated reply
    // auto-resumes instead of stalling for a manual nudge.
    #[test]
    fn max_tokens_stop_reason_self_continues_normal() {
        assert!(
            should_self_continue_after_stop_reason(&jfc_provider::StopReason::MaxTokens),
            "an output-truncated turn must be eligible to auto-resume"
        );
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

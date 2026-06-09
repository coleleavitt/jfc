//! Engine operations — the verbs of the engine API. Each op is a pure
//! `EngineState` mutation + event/effect emission; frontends (TUI key
//! handlers, headless drivers, remote control) call these instead of
//! reaching into engine internals. Carved out of `input/` as stage 4 of
//! the jfc-engine extraction.

use std::sync::Arc;

use crate::app::EngineState;
use crate::runtime::EventSender;
use crate::types::ChatMessage;

/// What `submit_prompt` did with the prompt. Frontends use this to decide
/// follow-up behavior (e.g. the TUI leaves queued prompts to the drain
/// loop; headless treats `CompactingFirst` as "wait for the re-fire").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubmitOutcome {
    /// The turn started: user message pushed, stream spawned.
    Started,
    /// An OnUserPromptSubmit hook vetoed the turn. Note: any staged
    /// attachments passed in are dropped with the veto.
    AbortedByHook,
    /// Context is at/over the compaction threshold — a pre-submit compaction
    /// was spawned and the prompt will re-fire via
    /// `ControlEvent::SubmitPrompt` once it lands.
    CompactingFirst,
}

/// Interrupt the current turn: cancel the stream, abort in-flight tools,
/// deny pending approvals, kill bash subprocesses. The engine half of the
/// TUI's Esc handling and the whole of remote/headless interrupt.
pub fn interrupt(state: &mut EngineState, tx: &EventSender) {
    let already_requested = state.cancel_token.is_cancelled()
        || state
            .interrupt_flag
            .load(std::sync::atomic::Ordering::SeqCst);

    let old_cancel = state.cancel_token.clone();
    state
        .interrupt_flag
        .store(true, std::sync::atomic::Ordering::SeqCst);
    old_cancel.cancel();
    state.cancel_token = tokio_util::sync::CancellationToken::new();

    if let Some(handle) = state.active_stream_handle.take() {
        handle.abort();
    }

    let denied_approvals = crate::runtime::approvals::deny_pending_and_queued(state, tx);

    if state.goal_evaluator_in_flight {
        tracing::info!(
            target: "jfc::input::abort",
            "marking in-flight goal evaluator cancelled"
        );
        state.goal_evaluator_in_flight = false;
    }

    // Zero the in-flight auto-mode classifier counter. Each classifier task
    // races the (now-cancelled) cancel token and returns WITHOUT emitting a
    // ClassifierDecision, so `pending_classifications` is never decremented —
    // it would otherwise stay > 0 forever, wedging `pipeline_busy_for_submit`
    // true so every later submit gets queued behind a turn that will never run
    // (the "queue a message after cancelling and it never fires" bug). The
    // aborted stream's StreamEvent::Error resets the rest of the pipeline; this
    // counter has no such self-clearing event.
    if state.pending_classifications > 0 {
        tracing::info!(
            target: "jfc::input::abort",
            pending = state.pending_classifications,
            "zeroing in-flight classifier counter on interrupt"
        );
        state.pending_classifications = 0;
    }

    let killed = crate::bash_processes::terminate_all();
    if killed > 0 {
        tracing::info!(
            target: "jfc::input::abort",
            killed,
            "SIGTERMed in-flight bash subprocesses"
        );
    }

    if !state.has_interruptible_work() {
        state
            .interrupt_flag
            .store(false, std::sync::atomic::Ordering::SeqCst);
    }

    if already_requested {
        tracing::debug!(
            target: "jfc::input::abort",
            denied_approvals,
            "interrupt request ignored because cancellation is already in progress"
        );
        return;
    }

    crate::toast::push_with_cap(
        &mut state.toasts,
        crate::toast::Toast::new(
            crate::toast::ToastKind::Warning,
            if killed > 0 {
                format!(
                    "⏹ Interrupted (killed {killed} process{})",
                    if killed == 1 { "" } else { "es" }
                )
            } else {
                "⏹ Interrupted".to_owned()
            },
        ),
    );
}

/// Load a session by id: read the transcript from disk, switch the engine
/// session (task store, per-session state), and reset streaming state.
/// View-side resets ride on the `SessionSwitched` effect.
pub async fn load_session(state: &mut EngineState, session_id: crate::ids::SessionId) {
    tracing::info!(
        target: "jfc::session_picker",
        session_id = %session_id,
        "LoadSession: fetching messages"
    );
    match crate::session::load_session(&session_id).await {
        Some(messages) => {
            state.messages = messages;
            let id_for_toast = session_id.clone();
            state.switch_session(Some(session_id));
            state.streaming_text.clear();
            state.streaming_reasoning.clear();
            state.streaming_response_bytes = 0;
            state.streaming_assistant_idx = None;
            state.push_effect(crate::app::EngineEffect::SessionSwitched);
            state.push_effect(crate::app::EngineEffect::ScrollToBottom);
            crate::toast::push_with_cap(
                &mut state.toasts,
                crate::toast::Toast::new(
                    crate::toast::ToastKind::Success,
                    format!("Loaded session {id_for_toast}"),
                ),
            );
        }
        None => {
            crate::toast::push_with_cap(
                &mut state.toasts,
                crate::toast::Toast::new(
                    crate::toast::ToastKind::Error,
                    format!("Failed to load session {session_id}"),
                ),
            );
        }
    }
}

/// Submit a user prompt: fire hooks, resolve @-mentions, (optionally)
/// rewrite history for an edit-resubmit, run the pre-submit compaction
/// gate, push the user message + reminders, reset per-turn state, persist
/// the session, and spawn the stream. The frontend half (textarea drain,
/// paste-chip expansion, pasted-image extraction, slash routing) happens
/// before this is called; `attachments` carries whatever the frontend
/// staged for this turn.
pub async fn submit_prompt(
    state: &mut EngineState,
    tx: &EventSender,
    text: String,
    attachments: Vec<crate::attachments::Attachment>,
    edit_at: Option<usize>,
) -> anyhow::Result<SubmitOutcome> {
    // v132 OnUserPromptSubmit hook — fires before any compaction or
    // stream setup so a registered handler can inject system reminders,
    // veto the turn, or rewrite the text. Default registry has only
    // a Logger so production behavior is unchanged when no user hooks
    // are configured.
    let session_id_for_hook = state
        .current_session_id
        .as_ref()
        .map(|s| s.as_str().to_owned())
        .unwrap_or_else(|| "<no-session>".to_owned());

    // CC 2.1.167 Setup hook — fires once, before the very first model turn.
    // Any shell hook registered on "Setup" can inject additional context into
    // the session. We fire it here (fire-and-forget) so it runs before the
    // OnUserPromptSubmit gate that can abort the turn.
    if state.messages.is_empty() {
        crate::hooks::fire_async(
            crate::hooks::HookPoint::OnSetup,
            &crate::hooks::HookContext::for_session(&session_id_for_hook),
        );
        tracing::debug!(
            target: "jfc::hooks",
            session_id = %session_id_for_hook,
            "fired OnSetup hook (first turn)"
        );
    }

    let hook_action = crate::hooks::fire(
        crate::hooks::HookPoint::OnUserPromptSubmit,
        &crate::hooks::HookContext::for_session(&session_id_for_hook)
            .with_extra("text_len", text.len().to_string()),
    );
    if let crate::hooks::HookAction::Abort(reason) = &hook_action {
        tracing::warn!(target: "jfc::hooks", %reason, "OnUserPromptSubmit aborted turn");
        let _ = tx
            .send(crate::runtime::EngineEvent::Control(
                crate::runtime::ControlEvent::Notice {
                    kind: crate::toast::ToastKind::Error,
                    text: format!("Turn aborted by hook: {reason}"),
                },
            ))
            .await;
        return Ok(SubmitOutcome::AbortedByHook);
    }

    // v132 @-mention auto-attach: scan the prompt for `@path/to/file`
    // tokens. If the path resolves to a real file, read it and stage
    // it as an attachment so the model sees the content alongside the
    // user's text. URLs (containing `://`) are skipped — those are
    // user-supplied references, not local paths.
    //
    // Text @-mentions: collect reminder bodies; inject after the new
    // user message is pushed so they land on the correct turn.
    // Binary @-mentions: collect locally; attach to the user message
    // after it's pushed — per-message ownership, no global queue.
    let mut deferred_text_reminders: Vec<String> = Vec::new();
    let mut mention_attachments: Vec<crate::attachments::Attachment> = Vec::new();
    {
        let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
        for token in text.split_whitespace() {
            // Strip surrounding punctuation: `(@src/foo.rs)` → `src/foo.rs`.
            let stripped = token.trim_matches(|c: char| {
                !c.is_alphanumeric() && c != '/' && c != '.' && c != '_' && c != '-' && c != '@'
            });
            let Some(rest) = stripped.strip_prefix('@') else {
                continue;
            };
            if rest.is_empty() || rest.contains("://") {
                continue;
            }
            if seen.contains(rest) {
                continue;
            }
            let path = std::path::PathBuf::from(rest);
            if !path.is_file() {
                continue;
            }
            let Ok(meta) = path.metadata() else {
                continue;
            };
            // Cap at 1 MB so a runaway @ doesn't OOM the prompt.
            if meta.len() > 1_000_000 {
                tracing::debug!(
                    target: "jfc::input::mention",
                    path = %path.display(),
                    bytes = meta.len(),
                    "@-mention skipped (file too large)"
                );
                continue;
            }
            // Image/PDF: stage as binary attachment via the existing
            // attachments path. Text: just nudge via system reminder
            // (the model can Read it itself if needed; auto-Read'ing
            // would burn tokens on every @ even when the user didn't
            // mean "show me this file").
            let bytes = match std::fs::read(&path) {
                Ok(b) => b,
                Err(_) => continue,
            };
            if let Some(kind) = crate::attachments::detect_kind(&bytes) {
                let att = crate::attachments::Attachment { id: 0, kind, bytes };
                mention_attachments.push(att);
                tracing::info!(
                    target: "jfc::input::mention",
                    path = %path.display(),
                    "@-mention auto-attached image/pdf"
                );
            } else if let Ok(content) = String::from_utf8(bytes) {
                let preview: String = content.chars().take(50_000).collect();
                deferred_text_reminders.push(format!(
                    "User mentioned `@{rest}` — content of `{}` follows:\n\n```\n{preview}\n```",
                    path.display()
                ));
                tracing::info!(
                    target: "jfc::input::mention",
                    path = %path.display(),
                    bytes = preview.len(),
                    "@-mention queued text reminder"
                );
            }
            seen.insert(rest.to_owned());
        }
    }

    // Edit mode: if the user is editing an earlier message, rewrite
    // history at that index and drop everything after before
    // continuing as a fresh submit. The new turn arrives as if the
    // user had typed it just now — agentic loop, tool calls, and
    // streaming all flow normally.
    if let Some(edit_idx) = edit_at {
        if edit_idx < state.messages.len() {
            tracing::info!(
                target: "jfc::input",
                edit_idx,
                kept = edit_idx,
                dropped = state.messages.len() - edit_idx,
                "edit-resubmit: rewriting history"
            );
            state.messages.truncate(edit_idx);
        }
        // Clear streaming-related state that might be tied to the
        // dropped messages (assistant placeholder index, etc.).
        state.streaming_text.clear();
        state.streaming_reasoning.clear();
        state.streaming_response_bytes = 0;
        state.turn_output_tokens = 0;
        state.refusal_fallback_attempted = false;
        state.refusal_resend_count = 0;
        state.streaming_assistant_idx = None;
    }
    // Pre-submit compaction gate (mirrors v126 `Du7` running before the API
    // call rather than only after tool batches). Without this, a long
    // text-only assistant reply pushes the context past 200K — by the time
    // the next user message arrives, the conversation already exceeds the
    // hard limit and the provider returns 400 prompt_too_long. v126 cli.js
    // line 382476 shows the same pre-submit check returning a "blocking_limit"
    // result before queryDirect ever fires.
    //
    // Use `tool_ctx.approx_tokens` (the calibrated wire-truth, kept in sync
    // by `recompute_token_estimate` on resume and by `StreamUsage` during a
    // turn) rather than re-running the chars-based `estimate_tokens`
    // heuristic. The doc comment on `compact::should_compact` warns
    // explicitly that the raw estimator over-counts tool outputs (it sums
    // their full byte length while the wire truncates each tool result to
    // `MAX_TOOL_RESULT_CHARS`), and on prompt-cache-heavy sessions it can
    // also under-count by missing the cache_read contribution. Using the
    // calibrated value makes pre-submit and post-tool compaction agree on
    // when the session is actually full.
    let mut est = state.tool_ctx.approx_tokens;
    let mut level = crate::compact::compact_level(est, state.max_context_tokens);
    if !state.force_compact_pending {
        let saved_tokens = crate::compact::microcompact_if_helpful(
            &mut state.messages,
            &mut state.tool_ctx.approx_tokens,
            level,
        );
        if saved_tokens > 0 {
            est = state.tool_ctx.approx_tokens;
            level = crate::compact::compact_level(est, state.max_context_tokens);
            tracing::info!(
                target: "jfc::compact::micro",
                saved_tokens,
                new_est = est,
                new_level = ?level,
                "pre-submit microcompaction applied"
            );
        }
    }
    let want_compact = matches!(
        level,
        crate::compact::CompactLevel::Compact | crate::compact::CompactLevel::Blocked
    ) || state.force_compact_pending;
    // Respect the suppression flag set by the post-response compact
    // path. Once compaction has permanently failed (provider doesn't
    // support it, breaker latched, retries exhausted), retrying on
    // every user message just re-fires the failing API call and
    // re-warns. The user clears it manually via /compact, which sets
    // `force_compact_pending` and bypasses this guard.
    if want_compact && state.compact_suppressed && !state.force_compact_pending {
        tracing::debug!(
            target: "jfc::compact",
            est, level = ?level,
            "pre-submit compact skipped — compact_suppressed latched"
        );
    } else if want_compact {
        let manual = std::mem::take(&mut state.force_compact_pending);
        tracing::info!(
            target: "jfc::compact",
            est, level = ?level, manual,
            model = %state.model,
            max_context_tokens = state.max_context_tokens,
            message_count = state.messages.len(),
            rapid_refill_count = state.tool_ctx.rapid_refill_count,
            "pre-submit compact triggered"
        );
        let messages = state.messages.clone();
        let provider = Arc::clone(&state.provider);
        let model = state.model.clone();
        let mut tool_ctx = state.tool_ctx.clone();
        let window = state.max_context_tokens;
        let tx_pre = tx.clone();
        let user_text = text.clone();
        let is_blocked = matches!(level, crate::compact::CompactLevel::Blocked);
        let _ = tx_pre
            .send(crate::runtime::EngineEvent::Compaction(
                crate::runtime::CompactionEvent::Started,
            ))
            .await;
        // Progress callback fires on every text_delta from the streaming
        // compact, forwards the cumulative output length as a
        // CompactionProgress event so the spinner shows live token
        // count. Mirrors v126's `addResponseLength` callback in PB7.
        let progress_tx = tx_pre.clone();
        let on_progress: crate::compact::CompactProgressCb = Box::new(move |chars| {
            // CompactionProgress is non-critical; next progress update supersedes.
            let _ = progress_tx.try_send(crate::runtime::EngineEvent::Compaction(
                crate::runtime::CompactionEvent::Progress {
                    output_chars: chars,
                },
            ));
        });
        tokio::spawn(async move {
            let options = jfc_provider::StreamOptions::new(model.clone());
            tracing::debug!(
                target: "jfc::compact",
                model = %model,
                window,
                "spawned pre-submit compaction task"
            );
            let result = crate::compact::compact(
                &messages,
                provider.as_ref(),
                &options,
                &mut tool_ctx,
                window,
                Some(on_progress),
            )
            .await;
            match result {
                crate::compact::CompactResult::Success {
                    messages,
                    pre_tokens,
                    post_tokens,
                } => {
                    tracing::info!(
                        target: "jfc::compact",
                        pre_tokens, post_tokens,
                        saved = pre_tokens.saturating_sub(post_tokens),
                        "pre-submit compaction succeeded — re-queuing user message"
                    );
                    let _ = tx_pre
                        .send(crate::runtime::EngineEvent::Compaction(
                            crate::runtime::CompactionEvent::Done {
                                messages,
                                tool_ctx,
                                pre_tokens,
                                post_tokens,
                            },
                        ))
                        .await;
                    // Re-queue the user's message — it didn't make it into
                    // the conversation before compaction ran.
                    let _ = tx_pre
                        .send(crate::runtime::EngineEvent::Control(
                            crate::runtime::ControlEvent::SubmitPrompt(user_text),
                        ))
                        .await;
                }
                crate::compact::CompactResult::CircuitBreakerTripped => {
                    tracing::warn!(
                        target: "jfc::compact",
                        "pre-submit compaction: circuit breaker tripped"
                    );
                    let _ = tx_pre
                        .send(crate::runtime::EngineEvent::Compaction(
                            crate::runtime::CompactionEvent::Failed {
                                reason: "Circuit breaker tripped — submit again with `/compact` if needed"
                                    .into(),
                                calibrated_tokens: None,
                                transient: false,
                            },
                        ))
                        .await;
                }
                crate::compact::CompactResult::Exhausted { attempts } => {
                    tracing::warn!(
                        target: "jfc::compact",
                        attempts,
                        "pre-submit compaction exhausted all attempts"
                    );
                    let _ = tx_pre
                        .send(crate::runtime::EngineEvent::Compaction(
                            crate::runtime::CompactionEvent::Failed {
                                reason: format!(
                                "Exhausted {attempts} compaction attempts — request is too large"
                            ),
                                calibrated_tokens: Some(tool_ctx.approx_tokens),
                                transient: false,
                            },
                        ))
                        .await;
                }
                _ => {
                    // Unsupported / TooFewGroups: provider can't compact.
                    // If we were merely at Compact level, submit anyway and
                    // let the API handle it. But if Blocked, don't re-submit
                    // (that would re-enter the compaction gate and loop forever).
                    if is_blocked {
                        tracing::warn!(
                            target: "jfc::compact",
                            "pre-submit compaction unsupported and context is Blocked — cannot proceed"
                        );
                        let _ = tx_pre
                            .send(crate::runtime::EngineEvent::Compaction(
                                crate::runtime::CompactionEvent::Failed {
                                    reason: "Context exceeds limit and provider cannot compact — \
                             try switching to a model/provider that supports compaction, \
                             or start a new session."
                                        .into(),
                                    calibrated_tokens: Some(tool_ctx.approx_tokens),
                                    transient: false,
                                },
                            ))
                            .await;
                    } else {
                        tracing::debug!(
                            target: "jfc::compact",
                            "pre-submit compaction skipped (unsupported/too few groups) — submitting anyway"
                        );
                        let _ = tx_pre
                            .send(crate::runtime::EngineEvent::Control(
                                crate::runtime::ControlEvent::SubmitPrompt(user_text),
                            ))
                            .await;
                    }
                }
            }
        });
        return Ok(SubmitOutcome::CompactingFirst);
    }

    // Keyword scan: detect and strip magic keywords (e.g. "ultrawork")
    // from the user's input before it becomes a message. The stripped
    // text is what gets stored in the conversation; the keyword's effect
    // is delivered via a system-reminder injected after the message push.
    let keyword_result = crate::keywords::scan_and_strip(&text);
    let any_keyword = keyword_result.ultrawork
        || keyword_result.ultracode
        || keyword_result.ultrathink
        || keyword_result.explore
        || keyword_result.turn_effort.is_some();
    let display_text = if any_keyword {
        tracing::info!(
            target: "jfc::keywords",
            ultrawork = keyword_result.ultrawork,
            ultracode = keyword_result.ultracode,
            ultrathink = keyword_result.ultrathink,
            explore = keyword_result.explore,
            "detected turn keyword — stripping and injecting reminder"
        );
        keyword_result.text.clone()
    } else {
        text.clone()
    };

    // The `ultracode` keyword turns on the standing session mode (xhigh +
    // workflow-by-default). Once on it persists across turns until `/effort`
    // clears it; the reminder below is injected every turn while active.
    if keyword_result.ultracode && !state.effort_state.is_ultracode() {
        state.effort_state.set_ultracode();
    }

    // `//effort <level>` sets a one-shot per-turn effort override that wins over
    // the session pin for this request only, then reverts.
    if let Some(level) = keyword_result.turn_effort {
        crate::effort::set_turn_effort(Some(level));
        tracing::info!(
            target: "jfc::keywords",
            effort = %level,
            "per-turn effort override set via //effort marker"
        );
    }

    let mut user_msg = ChatMessage::user(display_text.clone());
    // Combine pasted images ([Image #N] refs) with @-mention binary files.
    let mut all_attachments = attachments;
    all_attachments.extend(mention_attachments);
    user_msg.attachments = all_attachments;
    state.messages.push(user_msg);
    state.tool_ctx.total_user_turns += 1;

    // Periodic memory-persist nudge (Hermes parity): every N user turns, queue a
    // background `<system-reminder>` prompting the model to save durable facts
    // via the memory tool. Queued (not appended inline) so it rides the same
    // background-reminder drain as FS/MCP reminders on the next request.
    if let Some(body) = state.memory_nudge.on_user_turn() {
        tracing::debug!(
            target: "jfc::memory",
            interval = state.memory_nudge.interval,
            "memory-persist nudge fired"
        );
        state.queue_background_reminder(body);
    }

    // Ultrawork keyword: inject the system-reminder telling the model
    // to use the Workflow tool.
    if keyword_result.ultrawork {
        crate::system_reminder::append_to_last_user(
            &mut state.messages,
            crate::keywords::ULTRAWORK_REMINDER,
        );
    }
    // Standing ultracode reminder: injected on EVERY turn while the session
    // mode is active, not just the turn that enabled it.
    if state.effort_state.is_ultracode() {
        crate::system_reminder::append_to_last_user(
            &mut state.messages,
            crate::keywords::ULTRACODE_REMINDER,
        );
    }
    if keyword_result.ultrathink {
        state
            .exploration_state
            .force_next(crate::exploration::ExplorationLevel::MAX);
        crate::system_reminder::append_to_last_user(
            &mut state.messages,
            crate::keywords::ULTRATHINK_REMINDER,
        );
    }
    if keyword_result.explore {
        state
            .exploration_state
            .force_next(crate::exploration::ExplorationLevel::new(3));
        crate::system_reminder::append_to_last_user(
            &mut state.messages,
            crate::keywords::EXPLORE_REMINDER,
        );
    }

    // Inject detached background-agent completions as a one-shot reminder.
    // The visible `TaskStatus` parts stay in the transcript for UI/session
    // state, but provider-message construction intentionally does not replay
    // their summaries from historical assistant turns.
    let background_completions = state.take_background_agent_completions();
    let bg_completed = background_completions.len();
    if bg_completed > 0 {
        let plural = if bg_completed == 1 { "" } else { "s" };
        let mut reminder = format!(
            "{bg_completed} detached background task{plural} completed since your last turn. \
             Review the final summar{suffix} before responding:\n\n",
            suffix = if bg_completed == 1 { "y" } else { "ies" }
        );
        for completion in &background_completions {
            let status = completion.status.label();
            let body = if completion.body.len() > 2000 {
                format!(
                    "{}... [truncated {} chars]",
                    &completion.body[..completion.body.floor_char_boundary(2000)],
                    completion.body.len()
                )
            } else {
                completion.body.clone()
            };
            reminder.push_str(&format!(
                "- {} ({status}): {body}\n",
                completion.description
            ));
        }
        reminder.push_str(
            "\nIncorporate or acknowledge these results where they matter to the user's request.",
        );
        crate::system_reminder::append_to_last_user(&mut state.messages, &reminder);
        tracing::info!(
            target: "jfc::background",
            count = bg_completed,
            "injected background-agent completion reminder into user turn"
        );
    }

    // Now that the new user message is the most-recent user message,
    // attach any deferred @-mention text reminders to IT (not the
    // previous turn). See the comment on `deferred_text_reminders` at
    // the scan site for why this had to be split.
    for body in deferred_text_reminders {
        crate::system_reminder::append_to_last_user(&mut state.messages, &body);
    }

    // Task-state-drift nudge: if the previous turn did mutating work while a
    // plan was live but task state wasn't reconciled, remind the model to keep
    // the task list in sync — instead of the user having to ask "update the
    // tasks". Surfaces state back to the agent (SWE-agent ACI principle)
    // rather than silently mutating task semantics the model owns.
    if let Some(body) = crate::runtime::task_drift_reminder(state) {
        crate::system_reminder::append_to_last_user(&mut state.messages, &body);
        tracing::info!(
            target: "jfc::tasks",
            "injected task-state-drift reminder into user turn"
        );
    }

    // Auto graph-context injection: when the prompt smells like an
    // impact-analysis / refactor-risk / dependency-trace / entrypoint
    // question, run a cheap structural query against the workspace
    // graph and append the result as a `<system-reminder>` so the
    // model sees the structural context up-front instead of having to
    // remember to fire `graph_query` itself. Opt out via
    // `JFC_GRAPH_AUTO_CONTEXT=0`. The helper is a no-op for
    // non-graph intents and disabled-flag cases. We do NOT push the
    // assistant placeholder yet — `append_to_last_user` walks
    // `messages.iter_mut().rfind(|m| m.role == Role::User)`, so the
    // freshly-pushed user message at the tail is its target.
    //
    // Gated behind the `intent-gate` cargo feature for symmetry with
    // the `mod intent` declaration in `main.rs`. Without the gate the
    // intent module is configured-out and `crate::intent::...` paths
    // fail to resolve at compile time — see Cargo.toml `[features]`.
    #[cfg(feature = "intent-gate")]
    {
        let classification = crate::intent::classify(&text);
        let intent_for_inject = classification.intent;

        // (1) Doc-request intents → suggest the matching slash command
        // via a toast. We never auto-run the command (writing a file
        // the user didn't explicitly ask for is destructive) — the
        // toast is a one-keystroke nudge. Suppressed via
        // JFC_AUTO_DOC_SUGGEST=0.
        if let Some(cmd) = intent_for_inject.doc_command()
            && crate::intent::auto_doc_suggest_enabled()
        {
            tracing::info!(
                target: "jfc::intent::doc_suggest",
                intent = ?intent_for_inject,
                cmd,
                "doc-request detected — surfacing slash-command suggestion"
            );
            crate::toast::push_with_cap(
                &mut state.toasts,
                crate::toast::Toast::new(
                    crate::toast::ToastKind::Info,
                    format!(
                        "This looks like a doc request — type `{cmd}` to draft \
                             it with the strict format contract."
                    ),
                ),
            );
        }

        // (2) Auto-Plan-Mode: planning-shaped prompts flip the session
        // into Plan (read-only) permission mode — but only when the
        // user opted in via JFC_AUTO_PLAN_MODE=1, and only when we're
        // not already in a more-restrictive-or-equal mode. The user
        // can Shift+Tab back out immediately.
        if intent_for_inject == crate::intent::Intent::AutoPlanModeRequest
            && crate::intent::auto_plan_mode_enabled()
            && !matches!(
                state.permission_mode,
                crate::app::PermissionMode::Plan | crate::app::PermissionMode::Auto
            )
        {
            let from = state.permission_mode;
            state.permission_mode = crate::app::PermissionMode::Plan;
            tracing::info!(
                target: "jfc::intent::auto_plan_mode",
                ?from,
                "planning-shaped prompt — auto-flipped to Plan mode"
            );
            crate::toast::push_with_cap(
                &mut state.toasts,
                crate::toast::Toast::new(
                    crate::toast::ToastKind::Info,
                    "Planning request detected — switched to Plan mode \
                     (read-only). Shift+Tab to change."
                        .to_string(),
                ),
            );
            crate::system_reminder::append_to_last_user(
                &mut state.messages,
                "Permission mode auto-switched to `Plan` (read-only) because \
                 this request reads as planning/design work. Investigate and \
                 produce a plan; use ExitPlanMode with a finalized plan when \
                 you're ready to make edits.",
            );
        }
    }

    start_turn_from_transcript(state, tx, &display_text).await;
    Ok(SubmitOutcome::Started)
}

/// Push a fresh assistant slot, reset per-turn streaming/turn-control state,
/// persist the session, and spawn the stream over the CURRENT transcript.
/// The shared tail of `submit_prompt`; also the entry point for frontends
/// that seed the transcript externally (headless stream-json input,
/// session-mirror resume).
pub async fn start_turn_from_transcript(
    state: &mut EngineState,
    tx: &EventSender,
    turn_text: &str,
) {
    let assistant_idx = state.messages.len();
    state.messages.push(ChatMessage::assistant(String::new()));
    state.streaming_text.clear();
    state.streaming_reasoning.clear();
    state.streaming_response_bytes = 0;
    // New turn — restart the true output-token counter (it accumulates across
    // this turn's agentic sub-streams, but starts fresh per user turn) and the
    // refusal-fallback guard (each user turn gets one fallback attempt).
    state.turn_output_tokens = 0;
    state.refusal_fallback_attempted = false;
    state.refusal_resend_count = 0;
    state.network_recovery_status = None;
    state.network_recovery_attempts = 0;
    state.streaming_assistant_idx = Some(assistant_idx);
    state.is_streaming = true;
    // Defensive: clear any stale mixed-mode pause_turn latch from a
    // previously-cancelled turn. The flag is normally single-shot
    // (cleared at dispatch time in event_loop's AllComplete /
    // CompactionDone handlers) but a user-initiated cancel + fresh
    // submit would leave it sticky otherwise.
    state.pending_pause_turn_resume = false;
    let now = std::time::Instant::now();
    state.streaming_started_at = Some(now);
    state.last_stream_event_at = Some(now);
    state.streaming_last_token_at = Some(now);
    state.turn_started_at = Some(now);
    state.turn_start_cost = crate::cost::total_cost(&state.usage_by_model);
    // Fresh user turn → start a new per-turn edited-files set for `/turn-diff`.
    state.turn_edited_files.clear();
    state.pending_classifications = 0;
    state.agentic_turn_count = 0;
    // A genuine user submit resets the self-continuation budget — the human
    // is back in the loop, so the auto-driver starts fresh.
    state.self_continuation_count = 0;
    // New user turn — the empty-but-billed resend budget starts fresh too.
    state.empty_billed_resend_count = 0;
    // Reset thinking-state for the new turn so the spinner doesn't carry
    state.pre_dispatched_tool_ids.clear();
    state.deferred_tool_uses.clear();
    state.in_progress_tool_use_ids.clear();
    state.in_flight_eager_dispatches = 0;
    state.in_flight_tool_batches = 0;
    // a stale `thought for Ns` from the previous turn.
    state.thinking_started_at = None;
    state.thinking_ended_at = None;
    state.last_usage_output = 0;
    state.usage_apply_baseline = (0, 0, 0, 0);
    state.push_effect(crate::app::EngineEffect::ScrollToBottom);

    // Auto-persist the session so the sidebar shows it. Reuses the existing
    // session id if one was loaded; otherwise mints a fresh one keyed on the
    // current timestamp.
    let session_id = state
        .current_session_id
        .clone()
        .unwrap_or_else(jfc_session::generate_session_id);
    // Fire-and-forget session save — don't block the UI on disk I/O.
    {
        let sid = session_id.clone();
        let msgs = state.messages.clone();
        let cwd = state.cwd.clone();
        let model = state.model.clone();
        tokio::spawn(async move {
            crate::session::save_session(&sid, &msgs, Some(cwd.as_str()), Some(model.as_str()))
                .await;
        });
    }
    state.current_session_id = Some(session_id.clone());

    let provider = state.provider.clone();
    let messages = crate::stream::build_provider_messages(&state.messages[..assistant_idx]);
    // Slate per-turn model selection: when the router is configured (config
    // `slate_enabled = true`), classify the user's text and route to the
    // best-fit model for this turn. When None (default), use the pinned
    // `state.model` — legacy behavior. The pinned model is also the fallback
    // for unmatched classes inside the router itself.
    let model = if let Some(ref router) = state.slate {
        let (routed, class, rule_idx) = router.route_explained(turn_text, state.model.clone());
        tracing::info!(
            target: "jfc::slate",
            class = ?class,
            matched_rule = ?rule_idx,
            routed_model = %routed,
            pinned_model = %state.model,
            "slate routed turn"
        );
        routed
    } else {
        state.model.clone()
    };
    let cfg = crate::config::load_arc();
    state.exploration_state.begin_turn(turn_text, &cfg);
    let tx = tx.clone();
    let interrupt = state.interrupt_flag.clone();
    // Fresh user submission resets any prior interrupt state — the user
    // moved on, so the next stream should run unchecked.
    interrupt.store(false, std::sync::atomic::Ordering::SeqCst);
    // Mint a fresh cancel token. A token's `cancelled` is sticky, so a
    // previously cancelled turn would poison the next one if we reused
    // it. wg-async pattern: each unit of work gets its own token.
    state.cancel_token = tokio_util::sync::CancellationToken::new();
    let cancel = state.cancel_token.clone();
    let overrides = crate::runtime::StreamRequestOverrides {
        background_reminders: state.take_background_reminders(),
        disallowed_tools: state.effective_disallowed_tools(),
        allowed_tools: state.allowed_tools.clone(),
        custom_betas: state.custom_betas.clone(),
        fine_grained_tool_streaming: state.fine_grained_tool_streaming,
        strict_tool_schemas: state.strict_tool_schemas,
        task_budget: state.cli_task_budget,
        max_thinking_tokens: state.cli_max_thinking_tokens,
        thinking_display: state.cli_thinking_display.clone(),
        brief_mode: state.brief_mode,
        ..Default::default()
    };

    tracing::info!(
        target: "jfc::input",
        model = %model,
        provider_message_count = messages.len(),
        assistant_idx,
        session_id = %session_id,
        total_user_turns = state.tool_ctx.total_user_turns,
        "spawning stream_response"
    );

    // wg-async: stream_response holds the SSE connection + tx sender —
    // cancel has to thread through so ESC×2 can drop them coherently.
    // Park the *inner* task's abort handle on App so the watchdog can
    // forcefully abort the actual stream task (see App::active_stream_handle).
    // Previously this path stored no handle at all, so a wedged normal-submit
    // stream was uninterruptible by the watchdog's forceful-abort escalation.
    let tx_guard = tx.clone();
    let inner = tokio::spawn(async move {
        crate::stream::stream_response(
            provider, messages, model, tx, interrupt, cancel, None, overrides,
        )
        .await;
    });
    state.active_stream_handle = Some(inner.abort_handle());
    tokio::spawn(async move {
        if let Err(join_err) = inner.await {
            let msg = if join_err.is_panic() {
                format!("stream task panicked: {join_err}")
            } else {
                format!("stream task cancelled: {join_err}")
            };
            let _ = tx_guard
                .send(crate::runtime::EngineEvent::Stream(
                    crate::runtime::StreamEvent::Error(msg),
                ))
                .await;
        }
    });
}

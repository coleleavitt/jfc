use super::slash_commands::handle_slash_command;
use super::*;
pub async fn handle_submit_text(
    app: &mut App,
    text: String,
    tx: &mpsc::Sender<crate::runtime::AppEvent>,
) -> anyhow::Result<()> {
    handle_submit(app, text, tx).await
}

pub(super) async fn handle_submit(
    app: &mut App,
    text: String,
    tx: &mpsc::Sender<crate::runtime::AppEvent>,
) -> anyhow::Result<()> {
    tracing::info!(
        target: "jfc::input",
        text_len = text.len(),
        text_preview = %text.chars().take(80).collect::<String>(),
        model = %app.model,
        message_count = app.messages.len(),
        editing_idx = ?app.editing_message_idx,
        "handle_submit"
    );

    // v132 OnUserPromptSubmit hook — fires before any compaction or
    // stream setup so a registered handler can inject system reminders,
    // veto the turn, or rewrite the text. Default registry has only
    // a Logger so production behavior is unchanged when no user hooks
    // are configured.
    let session_id_for_hook = app
        .current_session_id
        .as_ref()
        .map(|s| s.as_str().to_owned())
        .unwrap_or_else(|| "<no-session>".to_owned());
    let hook_action = crate::hooks::fire(
        crate::hooks::HookPoint::OnUserPromptSubmit,
        &crate::hooks::HookContext::for_session(&session_id_for_hook)
            .with_extra("text_len", text.len().to_string()),
    );
    if let crate::hooks::HookAction::Abort(reason) = &hook_action {
        tracing::warn!(target: "jfc::hooks", %reason, "OnUserPromptSubmit aborted turn");
        let _ = tx
            .send(crate::runtime::AppEvent::Ui(
                crate::runtime::UiEvent::Toast {
                    kind: crate::toast::ToastKind::Error,
                    text: format!("Turn aborted by hook: {reason}"),
                },
            ))
            .await;
        return Ok(());
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

    // Extract referenced [Image #N] attachments from pasted_images and
    // attach them to the message that will be submitted. Any pasted
    // images whose markers the user deleted are dropped with a log.
    let submit_attachments: Vec<crate::attachments::Attachment> = if !app.pasted_images.is_empty() {
        // Parse all [Image #N] references from the text
        let mut referenced_ids: Vec<u32> = Vec::new();
        let re_pattern = regex::Regex::new(r"\[Image #(\d+)\]").unwrap();
        for cap in re_pattern.captures_iter(&text) {
            if let Ok(id) = cap[1].parse::<u32>() {
                referenced_ids.push(id);
            }
        }

        let mut matched: Vec<crate::attachments::Attachment> = Vec::new();
        let mut remaining: Vec<crate::attachments::PastedContent> = Vec::new();
        for pc in std::mem::take(&mut app.pasted_images) {
            if referenced_ids.contains(&pc.id) {
                matched.push(pc.attachment);
            } else {
                remaining.push(pc);
            }
        }

        // Drop unreferenced (user deleted the marker)
        if !remaining.is_empty() {
            tracing::info!(
                target: "jfc::input::paste",
                dropped = remaining.len(),
                "dropping unreferenced pasted images (markers deleted by user)"
            );
        }

        tracing::info!(
            target: "jfc::input::paste",
            matched = matched.len(),
            "matched [Image #N] attachments for submit"
        );
        matched
    } else {
        Vec::new()
    };

    // Edit mode: if the user is editing an earlier message, rewrite
    // history at that index and drop everything after before
    // continuing as a fresh submit. The new turn arrives as if the
    // user had typed it just now — agentic loop, tool calls, and
    // streaming all flow normally.
    if let Some(edit_idx) = app.editing_message_idx.take() {
        if edit_idx < app.messages.len() {
            tracing::info!(
                target: "jfc::input",
                edit_idx,
                kept = edit_idx,
                dropped = app.messages.len() - edit_idx,
                "edit-resubmit: rewriting history"
            );
            app.messages.truncate(edit_idx);
        }
        // Clear streaming-related state that might be tied to the
        // dropped messages (assistant placeholder index, etc.).
        app.streaming_text.clear();
        app.streaming_reasoning.clear();
        app.streaming_response_bytes = 0;
        app.streaming_assistant_idx = None;
    }
    if text.starts_with('/') {
        // `/check` re-runs the cargo-check producer. Handled here (not in
        // `handle_slash_command`) because it needs the tx channel to emit
        // `DiagnosticsUpdated` from a spawned task.
        if text.trim() == "/check" {
            let tx_diag = tx.clone();
            let cwd = std::env::current_dir().unwrap_or_else(|_| ".".into());
            tokio::spawn(async move {
                crate::diagnostics_producer::run_once(cwd, tx_diag).await;
            });
        }
        handle_slash_command(app, &text, Some(tx)).await;
        return Ok(());
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
    let est = app.tool_ctx.approx_tokens;
    let level = crate::compact::compact_level(est, app.max_context_tokens);
    let want_compact = matches!(
        level,
        crate::compact::CompactLevel::Compact | crate::compact::CompactLevel::Blocked
    ) || app.force_compact_pending;
    // Respect the suppression flag set by the post-response compact
    // path. Once compaction has permanently failed (provider doesn't
    // support it, breaker latched, retries exhausted), retrying on
    // every user message just re-fires the failing API call and
    // re-warns. The user clears it manually via /compact, which sets
    // `force_compact_pending` and bypasses this guard.
    if want_compact && app.compact_suppressed && !app.force_compact_pending {
        tracing::debug!(
            target: "jfc::compact",
            est, level = ?level,
            "pre-submit compact skipped — compact_suppressed latched"
        );
    } else if want_compact {
        let manual = std::mem::take(&mut app.force_compact_pending);
        tracing::info!(
            target: "jfc::compact",
            est, level = ?level, manual,
            model = %app.model,
            max_context_tokens = app.max_context_tokens,
            message_count = app.messages.len(),
            rapid_refill_count = app.tool_ctx.rapid_refill_count,
            "pre-submit compact triggered"
        );
        let messages = app.messages.clone();
        let provider = Arc::clone(&app.provider);
        let model = app.model.clone();
        let mut tool_ctx = app.tool_ctx.clone();
        let window = app.max_context_tokens;
        let tx_pre = tx.clone();
        let user_text = text.clone();
        let is_blocked = matches!(level, crate::compact::CompactLevel::Blocked);
        let _ = tx_pre
            .send(crate::runtime::AppEvent::Compaction(
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
            let _ = progress_tx.try_send(crate::runtime::AppEvent::Compaction(
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
                        .send(crate::runtime::AppEvent::Compaction(
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
                        .send(crate::runtime::AppEvent::Ui(
                            crate::runtime::UiEvent::Submit(user_text),
                        ))
                        .await;
                }
                crate::compact::CompactResult::CircuitBreakerTripped => {
                    tracing::warn!(
                        target: "jfc::compact",
                        "pre-submit compaction: circuit breaker tripped"
                    );
                    let _ = tx_pre
                        .send(crate::runtime::AppEvent::Compaction(
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
                        .send(crate::runtime::AppEvent::Compaction(
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
                            .send(crate::runtime::AppEvent::Compaction(
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
                            .send(crate::runtime::AppEvent::Ui(
                                crate::runtime::UiEvent::Submit(user_text),
                            ))
                            .await;
                    }
                }
            }
        });
        return Ok(());
    }

    // Keyword scan: detect and strip magic keywords (e.g. "ultrawork")
    // from the user's input before it becomes a message. The stripped
    // text is what gets stored in the conversation; the keyword's effect
    // is delivered via a system-reminder injected after the message push.
    let keyword_result = crate::keywords::scan_and_strip(&text);
    let display_text = if keyword_result.ultrawork {
        tracing::info!(
            target: "jfc::keywords",
            keyword = "ultrawork",
            "detected ultrawork keyword — stripping and injecting reminder"
        );
        keyword_result.text.clone()
    } else {
        text.clone()
    };

    let assistant_idx = app.messages.len() + 1;
    let mut user_msg = ChatMessage::user(display_text.clone());
    // Combine pasted images ([Image #N] refs) with @-mention binary files.
    let mut all_attachments = submit_attachments;
    all_attachments.extend(mention_attachments);
    user_msg.attachments = all_attachments;
    app.messages.push(user_msg);
    app.tool_ctx.total_user_turns += 1;

    // Ultrawork keyword: inject the system-reminder telling the model
    // to use the Workflow tool.
    if keyword_result.ultrawork {
        crate::system_reminder::append_to_last_user(
            &mut app.messages,
            crate::keywords::ULTRAWORK_REMINDER,
        );
    }

    // Inject background-agent completion notification if any detached
    // agents finished since the last user turn. The counter is
    // incremented by sync_detached_background_tasks_from_daemon when
    // agent status transitions to terminal. Drain it here so the model
    // sees "N agents finished — their summaries are in the transcript"
    // on this very turn (via the TaskStatus serialization we added to
    // build_provider_messages). Mirrors oh-my-opencode's
    // background-task-notification-template.ts pattern.
    let bg_completed = app.background_tasks_completed_since_last_turn;
    if bg_completed > 0 {
        app.background_tasks_completed_since_last_turn = 0;
        let plural = if bg_completed == 1 { "" } else { "s" };
        crate::system_reminder::append_to_last_user(
            &mut app.messages,
            &format!(
                "{bg_completed} detached background task{plural} completed since your last turn. \
                 Their final summaries are visible in the assistant transcript as \
                 [Background agent: ...] blocks. Review those summaries before responding — \
                 the user expects you to incorporate or acknowledge completed work."
            ),
        );
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
        crate::system_reminder::append_to_last_user(&mut app.messages, &body);
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

        // (1) Graph-flavored intents → auto-inject structural context.
        if crate::intent::is_graph_intent(intent_for_inject) {
            let cwd = std::path::PathBuf::from(&app.cwd);
            let injected = crate::intent::auto_inject_graph_context(
                &mut app.messages,
                intent_for_inject,
                &text,
                &cwd,
            );
            if injected {
                tracing::info!(
                    target: "jfc::intent::auto_ctx",
                    intent = ?intent_for_inject,
                    "auto graph-context injected"
                );
            }
        }

        // (2) Doc-request intents → suggest the matching slash command
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
                &mut app.toasts,
                crate::toast::Toast::new(
                    crate::toast::ToastKind::Info,
                    format!(
                        "This looks like a doc request — type `{cmd}` to draft \
                             it with the strict format contract."
                    ),
                ),
            );
        }

        // (3) Auto-Plan-Mode: planning-shaped prompts flip the session
        // into Plan (read-only) permission mode — but only when the
        // user opted in via JFC_AUTO_PLAN_MODE=1, and only when we're
        // not already in a more-restrictive-or-equal mode. The user
        // can Shift+Tab back out immediately.
        if intent_for_inject == crate::intent::Intent::AutoPlanModeRequest
            && crate::intent::auto_plan_mode_enabled()
            && !matches!(
                app.permission_mode,
                crate::app::PermissionMode::Plan | crate::app::PermissionMode::Auto
            )
        {
            let from = app.permission_mode;
            app.permission_mode = crate::app::PermissionMode::Plan;
            tracing::info!(
                target: "jfc::intent::auto_plan_mode",
                ?from,
                "planning-shaped prompt — auto-flipped to Plan mode"
            );
            crate::toast::push_with_cap(
                &mut app.toasts,
                crate::toast::Toast::new(
                    crate::toast::ToastKind::Info,
                    "Planning request detected — switched to Plan mode \
                     (read-only). Shift+Tab to change."
                        .to_string(),
                ),
            );
            crate::system_reminder::append_to_last_user(
                &mut app.messages,
                "Permission mode auto-switched to `Plan` (read-only) because \
                 this request reads as planning/design work. Investigate and \
                 produce a plan; use ExitPlanMode with a finalized plan when \
                 you're ready to make edits.",
            );
        }
    }

    app.messages.push(ChatMessage::assistant(String::new()));
    app.streaming_text.clear();
    app.streaming_reasoning.clear();
    app.streaming_response_bytes = 0;
    app.network_recovery_status = None;
    app.network_recovery_attempts = 0;
    app.streaming_assistant_idx = Some(assistant_idx);
    app.is_streaming = true;
    // Defensive: clear any stale mixed-mode pause_turn latch from a
    // previously-cancelled turn. The flag is normally single-shot
    // (cleared at dispatch time in event_loop's AllComplete /
    // CompactionDone handlers) but a user-initiated cancel + fresh
    // submit would leave it sticky otherwise.
    app.pending_pause_turn_resume = false;
    let now = std::time::Instant::now();
    app.streaming_started_at = Some(now);
    app.last_stream_event_at = Some(now);
    app.streaming_last_token_at = Some(now);
    app.turn_started_at = Some(now);
    app.turn_start_cost = crate::cost::total_cost(&app.usage_by_model);
    app.pending_classifications = 0;
    app.agentic_turn_count = 0;
    // Reset thinking-state for the new turn so the spinner doesn't carry
    app.pre_dispatched_tool_ids.clear();
    app.in_flight_eager_dispatches = 0;
    // a stale `thought for Ns` from the previous turn.
    app.thinking_started_at = None;
    app.thinking_ended_at = None;
    app.last_usage_output = 0;
    app.usage_apply_baseline = (0, 0, 0, 0);
    app.scroll_to_bottom();

    // Auto-persist the session so the sidebar shows it. Reuses the existing
    // session id if one was loaded; otherwise mints a fresh one keyed on the
    // current timestamp.
    let session_id = app
        .current_session_id
        .clone()
        .unwrap_or_else(jfc_session::generate_session_id);
    // Fire-and-forget session save — don't block the UI on disk I/O.
    {
        let sid = session_id.clone();
        let msgs = app.messages.clone();
        let cwd = app.cwd.clone();
        let model = app.model.clone();
        tokio::spawn(async move {
            crate::session::save_session(&sid, &msgs, Some(cwd.as_str()), Some(model.as_str()))
                .await;
        });
    }
    app.current_session_id = Some(session_id.clone());

    let provider = app.provider.clone();
    let messages = crate::stream::build_provider_messages(&app.messages[..assistant_idx]);
    // Slate per-turn model selection: when the router is configured (config
    // `slate_enabled = true`), classify the user's text and route to the
    // best-fit model for this turn. When None (default), use the pinned
    // `app.model` — legacy behavior. The pinned model is also the fallback
    // for unmatched classes inside the router itself.
    let model = if let Some(ref router) = app.slate {
        let (routed, class, rule_idx) = router.route_explained(&text, app.model.clone());
        tracing::info!(
            target: "jfc::slate",
            class = ?class,
            matched_rule = ?rule_idx,
            routed_model = %routed,
            pinned_model = %app.model,
            "slate routed turn"
        );
        routed
    } else {
        app.model.clone()
    };
    let tx = tx.clone();
    let interrupt = app.interrupt_flag.clone();
    // Fresh user submission resets any prior interrupt state — the user
    // moved on, so the next stream should run unchecked.
    interrupt.store(false, std::sync::atomic::Ordering::SeqCst);
    // Mint a fresh cancel token. A token's `cancelled` is sticky, so a
    // previously cancelled turn would poison the next one if we reused
    // it. wg-async pattern: each unit of work gets its own token.
    app.cancel_token = tokio_util::sync::CancellationToken::new();
    let cancel = app.cancel_token.clone();

    tracing::info!(
        target: "jfc::input",
        model = %model,
        provider_message_count = messages.len(),
        assistant_idx,
        session_id = %session_id,
        total_user_turns = app.tool_ctx.total_user_turns,
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
            provider,
            messages,
            model,
            tx,
            interrupt,
            cancel,
            None,
            crate::runtime::StreamRequestOverrides::default(),
        )
        .await;
    });
    app.active_stream_handle = Some(inner.abort_handle());
    tokio::spawn(async move {
        if let Err(join_err) = inner.await {
            let msg = if join_err.is_panic() {
                format!("stream task panicked: {join_err}")
            } else {
                format!("stream task cancelled: {join_err}")
            };
            let _ = tx_guard
                .send(crate::runtime::AppEvent::Stream(
                    crate::runtime::StreamEvent::Error(msg),
                ))
                .await;
        }
    });

    Ok(())
}

//! Background code review orchestration and durable review-result storage.
//!
//! The trigger is deliberately small: when a user-level turn ends after file
//! edits, JFC runs the existing `code-review` workflow as a background task and
//! fingerprints the changed-file diff so the same turn cannot queue duplicate
//! reviews.

use std::collections::{BTreeSet, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::app::EngineState;
use crate::runtime::{EngineEvent, EventSender, FrontendEvent, TaskEvent};

/// Background auto-review dispatch state.
///
/// The dedup signature lives behind a shared `Arc<Mutex<_>>` so the
/// background task that actually runs the review can *clear* it when the run
/// fails. Without that feedback loop a single failed run (e.g. every agent
/// hitting a 401) would poison the signature forever and silently suppress all
/// future auto-reviews of the same file-set for the rest of the session.
///
/// The remaining fields implement the event-triggered control loop from
/// `docs/auto-review-design.md`:
/// - `accumulated_risk` buffers a scalar risk score across turns so a review
///   fires when buffered risk crosses a barrier (not once per edit).
/// - `last_reviewed_head` records the commit HEAD at the last dispatch so a new
///   commit acts as an upper-bound forcing trigger.
/// - `in_flight` holds the cancel token of a running review so a superseding
///   edit set can abort the stale run (debounce/supersession).
#[derive(Debug, Default, Clone)]
pub struct AutoReviewState {
    last_dispatched_signature: Arc<parking_lot::Mutex<Option<String>>>,
    accumulated_risk: Arc<parking_lot::Mutex<u32>>,
    last_reviewed_head: Arc<parking_lot::Mutex<Option<String>>>,
    in_flight: Arc<parking_lot::Mutex<Option<Arc<tokio_util::sync::CancellationToken>>>>,
}

pub async fn maybe_spawn_after_turn(state: &mut EngineState, tx: &EventSender) {
    let mode = auto_review_mode();
    if matches!(mode, AutoReviewMode::Off | AutoReviewMode::Manual) {
        return;
    }
    if state.turn_edited_files.is_empty() {
        return;
    }

    let cwd = PathBuf::from(&state.cwd);
    let files: Vec<String> = state.turn_edited_files.iter().cloned().collect();

    // Level 1 — deterministic monitor (free): score the diff and bucket it.
    let signal = RiskSignal::measure(&cwd, &files).await;
    let monitor = signal.monitor_outcome();

    // Risk-barrier + commit forcing trigger (step 5): buffer risk across turns
    // and detect a new commit. A review fires when the monitor says Review, OR
    // buffered risk crosses the barrier, OR HEAD moved since the last review.
    let head = current_head(&cwd).await;
    let committed = {
        let last = state.auto_review.last_reviewed_head.lock();
        match (&*last, &head) {
            (Some(prev), Some(now)) => prev != now,
            _ => false,
        }
    };
    let buffered = {
        let mut acc = state.auto_review.accumulated_risk.lock();
        *acc = acc.saturating_add(signal.score);
        *acc
    };
    let barrier = risk_barrier();

    let decision = review_decision(mode, monitor, buffered, barrier, committed);
    let trigger = match decision {
        ReviewDecision::Skip => {
            tracing::debug!(
                target: "jfc::auto_review",
                file_count = files.len(),
                risk = signal.score,
                buffered,
                "auto-review monitor: skip (no review-worthy signal)"
            );
            return;
        }
        ReviewDecision::Fire(reason) => reason,
        ReviewDecision::AskGate => {
            // Level 2 — ambiguous: a single cheap LLM gate decides. Fail-safe
            // (gate error / unparseable) returns Review, never silently drops.
            let gate = review_gate(
                state.provider.as_ref(),
                &gate_model(&state.model),
                &cwd,
                &files,
                &signal,
            )
            .await;
            if !gate.should_review {
                tracing::debug!(
                    target: "jfc::auto_review",
                    reason = %gate.reason,
                    "auto-review gate: skip"
                );
                return;
            }
            format!("gate: {}", gate.reason)
        }
    };

    let Some(workflow) = crate::workflows::resolve(&cwd, "code-review") else {
        tracing::debug!(
            target: "jfc::auto_review",
            "code-review workflow unavailable; skipping auto-review"
        );
        return;
    };
    let perm =
        crate::workflows::permissions::decide(&crate::config::load_arc(), Some("code-review"));
    if perm == crate::workflows::permissions::WorkflowPermission::Deny {
        tracing::debug!(
            target: "jfc::auto_review",
            "code-review workflow denied by permission rules; skipping auto-review"
        );
        return;
    }

    let signature = review_signature(&cwd, &files).await;
    let signature_slot = state.auto_review.last_dispatched_signature.clone();
    if signature_slot.lock().as_deref() == Some(signature.as_str()) {
        tracing::debug!(
            target: "jfc::auto_review",
            signature,
            "auto-review already dispatched for this edit signature"
        );
        return;
    }

    let Ok((_meta, body)) = crate::workflows::parse_meta(&workflow.script) else {
        tracing::warn!(
            target: "jfc::auto_review",
            "failed to parse built-in code-review workflow; skipping auto-review"
        );
        return;
    };

    // Commit the dedup signature only after every early-return path has passed,
    // so a parse/permission skip never poisons the slot. The background task
    // clears it again if the run ends in error.
    *signature_slot.lock() = Some(signature.clone());
    // We are dispatching: reset the buffered risk and record the HEAD so the
    // next forcing trigger is the *next* commit, not this one.
    *state.auto_review.accumulated_risk.lock() = 0;
    *state.auto_review.last_reviewed_head.lock() = head;

    let run_id = crate::workflows::generate_run_id();
    let task_id = format!("bgauto_review_{run_id}");
    let session_id = state
        .current_session_id
        .as_ref()
        .map(|id| id.as_str().to_owned());
    let session_dir = workflow_session_dir(session_id.as_deref(), &run_id);
    let provider = Arc::clone(&state.provider);
    let model = state.model.clone();
    let tx_bg = tx.clone();
    // Derive a child token from the session so a user Ctrl+C / shutdown aborts
    // the background review instead of letting its agents keep hitting the API.
    let cancel = state.cancel_token.child_token();
    // Debounce/supersession (step 4): a fresh dispatch supersedes any review
    // still in flight, so a burst of edit-bearing turns collapses to the latest
    // review instead of stacking N concurrent fan-outs. Swap our token in and
    // cancel the previous one. The token is `Arc`-wrapped so the completion path
    // can clear the slot iff it still holds *this* run's handle.
    let cancel_handle = Arc::new(cancel.clone());
    {
        let mut slot = state.auto_review.in_flight.lock();
        if let Some(prev) = slot.replace(Arc::clone(&cancel_handle)) {
            prev.cancel();
        }
    }
    let in_flight_slot = state.auto_review.in_flight.clone();
    let target = auto_review_target(&cwd, &files);
    // Adaptive level (step 1): the risk signal picks the level unless the env
    // var pins one explicitly.
    let level = std::env::var("JFC_AUTO_REVIEW_LEVEL")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .unwrap_or_else(|| signal.adaptive_level().to_owned());
    let args = serde_json::json!({
        "level": level,
        "target": target,
        "auto": true,
        "source": "auto_review",
        "files": files,
        "mode": mode.as_str(),
        "trigger": trigger,
        "signature": signature,
    });

    let _ = tx
        .send(EngineEvent::Task(TaskEvent::Started {
            task_id: crate::ids::TaskId::from(task_id.clone()),
            description: format!(
                "auto-review: {}",
                args["target"].as_str().unwrap_or("edits")
            ),
            model_used: Some(model.as_str().to_owned()),
            max_input_tokens: None,
            is_detached: false,
            parent_task_id: None,
        }))
        .await;

    // Mirror the lifecycle into the daemon registry so the run survives a
    // session reload. Auto-review previously reported only to the spawning
    // session's event channel: after a restart the run showed as a stuck
    // [Failed]/stale entry in `jfc daemon agents` because nothing ever
    // recorded completion in the shared state file.
    jfc_daemon::record_background_agent_started(
        &task_id,
        &format!(
            "auto-review: {}",
            args["target"].as_str().unwrap_or("edits")
        ),
        Some(model.as_str().to_owned()),
        None,
    );

    tokio::spawn(async move {
        let _ = tokio::fs::create_dir_all(&session_dir).await;
        let started = Instant::now();

        // Deterministic proof routing: run cheap, deterministic oracles
        // (cargo test/clippy) BEFORE the LLM review and attach their observed
        // findings to the workflow args, so the review runs with real
        // compiler/test evidence rather than guessing whether the code builds.
        // Only for cargo projects, and only when enabled (default on).
        // Oracles respect the cancellation token: if interrupted by the user,
        // they early-exit and record findings as "not run" rather than blocking.
        let mut args = args;
        if auto_review_proof_oracles_enabled()
            && crate::proof_oracles::is_cargo_project(&cwd)
            && !cancel.is_cancelled()
        {
            let findings = crate::proof_oracles::run_all(&cwd, &cancel).await;
            let block = crate::proof_oracles::render_findings_block(&findings);
            if let serde_json::Value::Object(map) = &mut args {
                map.insert(
                    "proof_findings".to_owned(),
                    serde_json::to_value(&findings).unwrap_or(serde_json::Value::Null),
                );
                if !block.is_empty() {
                    map.insert(
                        "proof_findings_text".to_owned(),
                        serde_json::Value::String(block),
                    );
                }
            }
        }

        // Finding memoization (step 6): hash each edited file's current content
        // and skip review entirely if every edited file is byte-identical to the
        // last reviewed snapshot (nothing semantically new to verify). Otherwise
        // attach the prior content-hash map so synthesis can mark which findings
        // are *marginal* (newly introduced) vs. carried over.
        let content_hashes = content_hash_map(&cwd, &files).await;
        let prior_hashes = load_content_hashes(&cwd).await;
        let all_unchanged = !content_hashes.is_empty()
            && content_hashes
                .iter()
                .all(|(file, hash)| prior_hashes.get(file) == Some(hash));
        if all_unchanged && !cancel.is_cancelled() {
            tracing::debug!(
                target: "jfc::auto_review",
                "auto-review memoization: all edited files unchanged since last review; skipping"
            );
            clear_in_flight(&in_flight_slot, &cancel_handle);
            {
                let mut slot = signature_slot.lock();
                if slot.as_deref() == Some(signature.as_str()) {
                    *slot = None;
                }
            }
            jfc_daemon::record_background_agent_finished(
                &task_id,
                jfc_daemon::BackgroundAgentStatus::Completed,
                "auto-review skipped: no content change since last review",
            );
            let _ = tx_bg
                .send(EngineEvent::Task(TaskEvent::Completed {
                    task_id: crate::ids::TaskId::from(task_id),
                    summary: "auto-review skipped: no content change since last review".to_owned(),
                    elapsed_ms: started.elapsed().as_millis() as u64,
                }))
                .await;
            return;
        }
        if let serde_json::Value::Object(map) = &mut args {
            if let Ok(prior) = serde_json::to_value(&prior_hashes) {
                map.insert("prior_content_hashes".to_owned(), prior);
            }
        }

        let outcome = crate::workflows::run_workflow(crate::workflows::WorkflowRunConfig {
            run_id: run_id.clone(),
            script_body: body,
            args: args.clone(),
            provider,
            model,
            session_id: session_id.clone(),
            session_dir,
            resume_from_run_id: None,
            cancel: cancel.clone(),
            tx: Some(tx_bg.clone()),
            workflow_task_id: task_id.clone(),
            depth: 0,
            cwd: cwd.clone(),
            token_budget: None,
        })
        .await;
        let elapsed_ms = started.elapsed().as_millis() as u64;
        // Persist the content-hash snapshot so the next run can memoize against
        // it and compute the marginal finding set.
        if !outcome.cancelled {
            save_content_hashes(&cwd, &content_hashes).await;
        }
        clear_in_flight(&in_flight_slot, &cancel_handle);

        let review = persist_code_review_outcome_event(
            &cwd,
            &run_id,
            "auto",
            &args,
            &outcome.result,
            outcome.error.as_deref(),
        )
        .await;

        if outcome.cancelled {
            // The run was deliberately stopped (user Ctrl+C, shutdown, or a
            // superseding turn). This is NOT a failure: the orchestrator
            // teardown error ("workflow orchestrator unavailable") is an
            // artifact of cancellation, not a crash. Always free the dedup
            // signature so the same edits can be reviewed again, and report a
            // `cancelled:`-prefixed terminal event so the UI + daemon record it
            // as Cancelled rather than Failed (see handle_task_failed).
            {
                let mut slot = signature_slot.lock();
                if slot.as_deref() == Some(signature.as_str()) {
                    *slot = None;
                }
            }
            // If the review nonetheless produced findings before cancellation,
            // still surface them so completed work isn't silently discarded.
            if let Some(review) = review {
                let _ = tx_bg
                    .send(EngineEvent::Frontend(FrontendEvent::ReviewCompleted {
                        review,
                    }))
                    .await;
            }
            let reason = outcome
                .error
                .unwrap_or_else(|| "workflow cancelled".to_owned());
            // Mark the daemon entry as Cancelled so has_interruptible_work()
            // stops returning true — otherwise Ctrl+C freezes the UI waiting
            // for a "Running" task that will never finish.
            jfc_daemon::record_background_agent_finished(
                &task_id,
                jfc_daemon::BackgroundAgentStatus::Cancelled,
                &format!("cancelled: {reason}"),
            );
            let _ = tx_bg
                .send(EngineEvent::Task(TaskEvent::Failed {
                    task_id: crate::ids::TaskId::from(task_id),
                    error: format!("cancelled: {reason}"),
                }))
                .await;
        } else if let Some(error) = outcome.error {
            // The run failed (e.g. provider auth/transient errors). Clear the
            // dedup signature so an identical follow-up edit set can re-trigger
            // the review instead of being permanently suppressed. Only clear if
            // it still holds *our* signature — a newer dispatch may have already
            // claimed the slot.
            {
                let mut slot = signature_slot.lock();
                if slot.as_deref() == Some(signature.as_str()) {
                    *slot = None;
                }
            }
            jfc_daemon::record_background_agent_finished(
                &task_id,
                jfc_daemon::BackgroundAgentStatus::Failed,
                &error,
            );
            let _ = tx_bg
                .send(EngineEvent::Task(TaskEvent::Failed {
                    task_id: crate::ids::TaskId::from(task_id),
                    error,
                }))
                .await;
        } else {
            if let Some(review) = review {
                let _ = tx_bg
                    .send(EngineEvent::Frontend(FrontendEvent::ReviewCompleted {
                        review,
                    }))
                    .await;
            }
            let summary = build_auto_review_notification(&task_id, &outcome, elapsed_ms);
            jfc_daemon::record_background_agent_finished(
                &task_id,
                jfc_daemon::BackgroundAgentStatus::Completed,
                &summary,
            );
            let _ = tx_bg
                .send(EngineEvent::Task(TaskEvent::Completed {
                    task_id: crate::ids::TaskId::from(task_id),
                    summary,
                    elapsed_ms,
                }))
                .await;
        }
    });
}

/// Clear the in-flight slot iff it still holds *our* cancel handle. A superseding
/// dispatch may have already replaced it; in that case we must not stomp the
/// newer run's token. `Arc::ptr_eq` gives reliable identity across the clones.
fn clear_in_flight(
    slot: &Arc<parking_lot::Mutex<Option<Arc<tokio_util::sync::CancellationToken>>>>,
    ours: &Arc<tokio_util::sync::CancellationToken>,
) {
    let mut guard = slot.lock();
    if guard.as_ref().is_some_and(|tok| Arc::ptr_eq(tok, ours)) {
        *guard = None;
    }
}

/// Three buckets the deterministic monitor sorts an edit set into.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReviewMonitorOutcome {
    /// Clearly trivial — skip with no LLM and no review.
    Skip,
    /// Clearly review-worthy — dispatch directly.
    Review,
    /// Neither clearly trivial nor clearly risky — escalate to the LLM gate.
    Ambiguous,
}

/// Final decision after folding monitor + risk-barrier + commit + mode.
#[derive(Debug, Clone, PartialEq, Eq)]
enum ReviewDecision {
    Skip,
    Fire(String),
    AskGate,
}

/// Cheap, deterministic risk score for an edit set. Pure given its inputs (the
/// async constructor reads files/git, but `monitor_outcome`/`adaptive_level`/
/// `score` are pure), so the bucketing and level mapping are fully testable.
#[derive(Debug, Clone, Default)]
struct RiskSignal {
    /// Number of edited files.
    file_count: usize,
    /// Added+removed lines across the edited diff (git numstat).
    changed_lines: u64,
    /// A high-signal risk token (unsafe/auth/Mcp/...) is present in content/diff.
    risky_token: bool,
    /// At least one edited file is a Rust/Cargo/workflow-sensitive path.
    code_sensitive: bool,
    /// Aggregate scalar score used for the cross-turn risk barrier.
    score: u32,
}

impl RiskSignal {
    async fn measure(cwd: &Path, files: &[String]) -> Self {
        let file_count = files.len();
        let changed_lines = git_numstat_changed_lines(cwd, files).await;
        let code_sensitive = files.iter().any(|file| is_code_sensitive(cwd, file));
        let mut risky_token = git_diff_has_review_signal(cwd, files).await;
        if !risky_token {
            for file in files {
                if file_content_has_review_signal(&cwd.join(file)).await {
                    risky_token = true;
                    break;
                }
            }
        }
        // Score: tokens dominate, then size, then breadth. Tuned so a lone
        // comment/doc tweak scores ~0-1 and a multi-file unsafe/auth diff scores
        // well over the barrier in one turn.
        let mut score = 0u32;
        if risky_token {
            score += 6;
        }
        if code_sensitive {
            score += 2;
        }
        score += (file_count.min(20) as u32) / 2;
        score += (changed_lines.min(2000) / 40) as u32;
        Self {
            file_count,
            changed_lines,
            risky_token,
            code_sensitive,
            score,
        }
    }

    /// Bucket this edit set deterministically.
    fn monitor_outcome(&self) -> ReviewMonitorOutcome {
        // Clear review: any high-signal risk token, or a large/broad code diff.
        if self.risky_token
            || self.file_count >= 5
            || (self.code_sensitive && self.changed_lines >= 80)
        {
            return ReviewMonitorOutcome::Review;
        }
        // Clear skip: no code-sensitive file and a tiny diff (docs/prose/config
        // touch-ups). Nothing the reviewer would meaningfully act on.
        if !self.code_sensitive && self.changed_lines <= 5 && self.file_count <= 1 {
            return ReviewMonitorOutcome::Skip;
        }
        // Everything else is genuinely uncertain — let the gate decide.
        ReviewMonitorOutcome::Ambiguous
    }

    /// Map the signal to a `code-review` effort level (step 1).
    fn adaptive_level(&self) -> &'static str {
        if self.risky_token && (self.file_count >= 4 || self.changed_lines >= 400) {
            "xhigh"
        } else if self.risky_token || self.file_count >= 4 || self.changed_lines >= 300 {
            "high"
        } else if self.code_sensitive || self.changed_lines >= 60 {
            "medium"
        } else {
            "low"
        }
    }
}

/// Fold the monitor bucket with mode, the cross-turn risk barrier, and the
/// commit forcing trigger into one decision. Pure for testability.
fn review_decision(
    mode: AutoReviewMode,
    monitor: ReviewMonitorOutcome,
    buffered_risk: u32,
    barrier: u32,
    committed: bool,
) -> ReviewDecision {
    // `Always` ignores the monitor entirely.
    if matches!(mode, AutoReviewMode::Always) {
        return ReviewDecision::Fire("mode=always".to_owned());
    }
    // A new commit is an upper-bound forcing trigger regardless of bucket.
    if committed {
        return ReviewDecision::Fire("commit boundary".to_owned());
    }
    match monitor {
        ReviewMonitorOutcome::Review => ReviewDecision::Fire("risk monitor".to_owned()),
        ReviewMonitorOutcome::Ambiguous => {
            if buffered_risk >= barrier {
                ReviewDecision::Fire(format!("risk barrier ({buffered_risk}>={barrier})"))
            } else {
                ReviewDecision::AskGate
            }
        }
        ReviewMonitorOutcome::Skip => {
            if buffered_risk >= barrier {
                ReviewDecision::Fire(format!("risk barrier ({buffered_risk}>={barrier})"))
            } else {
                ReviewDecision::Skip
            }
        }
    }
}

/// Buffered-risk barrier above which accumulated trivial/ambiguous edits force a
/// review even without a commit. Overridable via `JFC_AUTO_REVIEW_RISK_BARRIER`.
fn risk_barrier() -> u32 {
    std::env::var("JFC_AUTO_REVIEW_RISK_BARRIER")
        .ok()
        .and_then(|v| v.trim().parse::<u32>().ok())
        .filter(|v| *v > 0)
        .unwrap_or(12)
}

/// Pick the cheap model for the gate: reuse the configured fast tier when the
/// session model is itself a big model, else just reuse the session model. Kept
/// simple — the gate is one short structured call.
fn gate_model(session_model: &jfc_provider::ModelId) -> String {
    let m = session_model.as_str();
    if m.contains("haiku") {
        m.to_owned()
    } else if m.starts_with("anthropic/") || m.contains("claude") {
        "claude-haiku-4-5".to_owned()
    } else {
        m.to_owned()
    }
}

/// Outcome of the cheap LLM review gate.
#[derive(Debug, Clone)]
struct GateResult {
    should_review: bool,
    reason: String,
}

/// Level 2 — the cheap LLM gate. Mirrors `auto_mode::classify`: a small model
/// with a forced classifier tool returns `{should_review, level, reason}`. Any
/// provider/parse error fails *open* (review), so an errored gate never silently
/// drops a possibly-risky change.
async fn review_gate(
    provider: &dyn jfc_provider::Provider,
    model: &str,
    cwd: &Path,
    files: &[String],
    signal: &RiskSignal,
) -> GateResult {
    use jfc_provider::{ProviderContent, ProviderMessage, ProviderRole, StreamOptions, ToolDef};

    let rels: Vec<String> = files
        .iter()
        .take(20)
        .map(|f| normalize_repo_relative(cwd, f))
        .collect();
    let user = format!(
        "An agent just edited these files in a turn:\n{}\n\n\
         Heuristic signal: files={}, changed_lines={}, code_sensitive={}, risky_token={}.\n\n\
         Decide whether this change warrants a background code review. Review when there is \
         real correctness, security, API, or regression risk. Skip pure formatting, comments, \
         docs, or trivial mechanical edits. Call `review_decision`.",
        rels.join("\n"),
        signal.file_count,
        signal.changed_lines,
        signal.code_sensitive,
        signal.risky_token,
    );
    let tool = ToolDef {
        name: "review_decision".into(),
        description: "Decide whether the edit set warrants a background code review.".into(),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "should_review": { "type": "boolean", "description": "true = run review, false = skip." },
                "level": { "type": "string", "enum": ["low", "medium", "high", "xhigh"], "description": "Suggested review depth." },
                "reason": { "type": "string", "description": "One-sentence rationale." }
            },
            "required": ["should_review", "reason"]
        }),
    };
    let messages = vec![ProviderMessage {
        role: ProviderRole::User,
        content: vec![ProviderContent::Text(user)],
    }];
    let opts = StreamOptions::new(model)
        .system(
            "You are a fast, conservative gate deciding whether an edit set needs a code review. \
             Prefer skipping trivial edits; review anything with real defect or security risk.",
        )
        .max_tokens(512)
        .tools(vec![tool]);
    match provider.complete(messages, &opts).await {
        Ok(resp) => parse_gate(&resp).unwrap_or_else(|| GateResult {
            should_review: true,
            reason: "gate returned no parseable decision (fail-open)".to_owned(),
        }),
        Err(e) => GateResult {
            should_review: true,
            reason: format!("gate error (fail-open): {e}"),
        },
    }
}

fn parse_gate(resp: &jfc_provider::CompletionResponse) -> Option<GateResult> {
    let s = resp.content.trim();
    let v: serde_json::Value = serde_json::from_str(s).ok().or_else(|| {
        let start = s.find('{')?;
        let end = s.rfind('}')?;
        if start < end {
            serde_json::from_str(&s[start..=end]).ok()
        } else {
            None
        }
    })?;
    let obj = v.get("input").or_else(|| v.get("arguments")).unwrap_or(&v);
    let should_review = obj.get("should_review")?.as_bool()?;
    let reason = obj
        .get("reason")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("(no reason)")
        .to_owned();
    Some(GateResult {
        should_review,
        reason,
    })
}

/// Whether a repo-relative path is code/build/workflow sensitive.
fn is_code_sensitive(cwd: &Path, file: &str) -> bool {
    let rel = normalize_repo_relative(cwd, file);
    rel.ends_with(".rs")
        || rel == "Cargo.toml"
        || rel == "Cargo.lock"
        || rel.ends_with("/Cargo.toml")
        || rel.ends_with("/Cargo.lock")
        || rel.starts_with("crates/")
        || rel.starts_with(".github/workflows/")
}

/// Added+removed lines across the edited diff via `git diff --numstat`.
async fn git_numstat_changed_lines(cwd: &Path, files: &[String]) -> u64 {
    let mut cmd = tokio::process::Command::new("git");
    cmd.current_dir(cwd)
        .args(["diff", "--numstat", "HEAD", "--"])
        .args(files);
    let Ok(output) = cmd.output().await else {
        return 0;
    };
    let text = String::from_utf8_lossy(&output.stdout);
    let mut total = 0u64;
    for line in text.lines() {
        let mut cols = line.split_whitespace();
        let added = cols.next().and_then(|s| s.parse::<u64>().ok()).unwrap_or(0);
        let removed = cols.next().and_then(|s| s.parse::<u64>().ok()).unwrap_or(0);
        total += added + removed;
    }
    total
}

/// Current commit HEAD (short hash), or None outside a git repo.
async fn current_head(cwd: &Path) -> Option<String> {
    let output = tokio::process::Command::new("git")
        .current_dir(cwd)
        .args(["rev-parse", "HEAD"])
        .output()
        .await
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    if s.is_empty() { None } else { Some(s) }
}

/// SHA-256 (first 16 bytes) of each edited file's current on-disk content, keyed
/// by repo-relative path. Missing/unreadable files are skipped.
async fn content_hash_map(
    cwd: &Path,
    files: &[String],
) -> std::collections::BTreeMap<String, String> {
    let mut map = std::collections::BTreeMap::new();
    for file in files {
        let rel = normalize_repo_relative(cwd, file);
        if let Ok(body) = tokio::fs::read(&cwd.join(file)).await {
            let mut hasher = Sha256::new();
            hasher.update(&body);
            map.insert(rel, hex::encode(&hasher.finalize()[..16]));
        }
    }
    map
}

async fn load_content_hashes(cwd: &Path) -> std::collections::BTreeMap<String, String> {
    let artifact_key = auto_review_artifact_key(cwd, "content_hashes", "snapshot");
    tokio::task::spawn_blocking(move || {
        jfc_knowledge::block_on_knowledge(async {
            let store = jfc_knowledge::KnowledgeStore::open_default().await.ok()?;
            let row = store
                .get_session_artifact(
                    REVIEW_ARTIFACT_SESSION_ID,
                    REVIEW_ARTIFACT_KIND,
                    &artifact_key,
                )
                .await
                .ok()??;
            serde_json::from_str::<std::collections::BTreeMap<String, String>>(&row.value_json).ok()
        })
    })
    .await
    .ok()
    .flatten()
    .unwrap_or_default()
}

async fn save_content_hashes(cwd: &Path, hashes: &std::collections::BTreeMap<String, String>) {
    if hashes.is_empty() {
        return;
    }
    // Merge into any existing snapshot so unrelated files stay memoized.
    let mut merged = load_content_hashes(cwd).await;
    for (k, v) in hashes {
        merged.insert(k.clone(), v.clone());
    }
    let Ok(value_json) = serde_json::to_string(&merged) else {
        return;
    };
    let artifact_key = auto_review_artifact_key(cwd, "content_hashes", "snapshot");
    let _ = tokio::task::spawn_blocking(move || {
        jfc_knowledge::block_on_knowledge(async {
            let store = jfc_knowledge::KnowledgeStore::open_default()
                .await
                .map_err(std::io::Error::other)?;
            store
                .upsert_session_artifact(
                    REVIEW_ARTIFACT_SESSION_ID,
                    REVIEW_ARTIFACT_KIND,
                    &artifact_key,
                    &value_json,
                )
                .await
                .map_err(std::io::Error::other)
        })
    })
    .await;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AutoReviewMode {
    Off,
    Manual,
    Smart,
    Always,
}

impl AutoReviewMode {
    fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Manual => "manual",
            Self::Smart => "smart",
            Self::Always => "always",
        }
    }
}

fn auto_review_mode() -> AutoReviewMode {
    // Precedence: the `JFC_AUTO_REVIEW` env var always wins (per-invocation
    // override, back-compat). Otherwise fall back to the `[argus_auto_review]`
    // section of config.toml so users can turn auto-review off durably (some
    // people find it slow/noisy). With neither set, the default is Smart —
    // byte-identical to prior behavior.
    if let Ok(value) = std::env::var("JFC_AUTO_REVIEW") {
        return parse_auto_review_mode(value.trim());
    }
    auto_review_mode_from_config(&crate::config::load_arc())
}

/// Parse one mode token (shared by the env var and the config `mode` field).
fn parse_auto_review_mode(value: &str) -> AutoReviewMode {
    match value.to_ascii_lowercase().as_str() {
        "0" | "false" | "off" | "no" | "disabled" => AutoReviewMode::Off,
        "manual" => AutoReviewMode::Manual,
        "always" | "1" | "true" | "on" | "yes" => AutoReviewMode::Always,
        "smart" | "" => AutoReviewMode::Smart,
        other => {
            tracing::warn!(
                target: "jfc::auto_review",
                value = other,
                "unknown auto-review mode; using smart"
            );
            AutoReviewMode::Smart
        }
    }
}

/// Resolve the mode from config when no env override is present.
///
/// `[argus_auto_review]` semantics:
///   - absent section            → Smart (default; unchanged behavior)
///   - `enabled = false`         → Off  (the user opt-out this adds)
///   - `enabled = true`/omitted  → Smart
///   - `mode = "off|manual|always|smart"` → that mode (takes precedence over
///                                          `enabled` so `mode` is the precise knob)
fn auto_review_mode_from_config(cfg: &jfc_config::Config) -> AutoReviewMode {
    let Some(argus) = cfg.argus_auto_review.as_ref() else {
        return AutoReviewMode::Smart;
    };
    if let Some(mode) = argus.mode.as_deref().filter(|m| !m.trim().is_empty()) {
        return parse_auto_review_mode(mode.trim());
    }
    if argus.enabled == Some(false) {
        return AutoReviewMode::Off;
    }
    AutoReviewMode::Smart
}

/// Whether to run deterministic proof oracles (cargo test/clippy) before the
/// LLM review and attach their findings. Default on; set
/// `JFC_AUTO_REVIEW_PROOF_ORACLES=0/false/off/no` to disable (e.g. to avoid
/// contending on the build lock in a tight edit loop).
fn auto_review_proof_oracles_enabled() -> bool {
    match std::env::var("JFC_AUTO_REVIEW_PROOF_ORACLES") {
        Ok(value) => !matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "0" | "false" | "off" | "no"
        ),
        Err(_) => true,
    }
}

/// Normalize an edited-file path to a cwd-relative, forward-slashed form.
///
/// `turn_edited_files` records whatever path the Edit/Write tool saw, which is
/// usually absolute (`/home/u/proj/crates/x/Cargo.toml`). The trigger's
/// prefix checks (`crates/`, `.github/workflows/`) and exact matches
/// (`Cargo.toml`) only fire on repo-relative paths, so strip the cwd prefix
/// first. Paths already relative, or outside cwd, pass through unchanged.
fn normalize_repo_relative(cwd: &Path, file: &str) -> String {
    let path = Path::new(file);
    let rel = path.strip_prefix(cwd).unwrap_or(path);
    rel.to_string_lossy().replace('\\', "/")
}

async fn file_content_has_review_signal(path: &Path) -> bool {
    const TOKENS: &[&str] = &[
        "unsafe",
        "transmute",
        "MaybeUninit",
        "ManuallyDrop",
        "from_raw_parts",
        "set_len",
        "get_unchecked",
        "unwrap_unchecked",
        "extern \"C\"",
        "target_feature",
        "atomic",
        "volatile",
        "auth",
        "token",
        "secret",
        "permission",
        "Mcp",
        "ToolKind",
        "ToolInput",
    ];
    let Ok(body) = tokio::fs::read_to_string(path).await else {
        return false;
    };
    TOKENS.iter().any(|token| body.contains(token))
}

async fn git_diff_has_review_signal(cwd: &Path, files: &[String]) -> bool {
    const TOKENS: &[&str] = &[
        "+pub ",
        "+unsafe",
        "+transmute",
        "+MaybeUninit",
        "+ManuallyDrop",
        "+from_raw_parts",
        "+set_len",
        "+get_unchecked",
        "+unwrap_unchecked",
        "+extern \"C\"",
        "+target_feature",
        "+atomic",
        "+volatile",
        "+auth",
        "+token",
        "+secret",
        "+permission",
        "+Mcp",
        "+ToolKind",
        "+ToolInput",
    ];
    let mut cmd = tokio::process::Command::new("git");
    cmd.current_dir(cwd).args(["diff", "--"]).args(files);
    let Ok(output) = cmd.output().await else {
        return false;
    };
    let diff = String::from_utf8_lossy(&output.stdout);
    TOKENS.iter().any(|token| diff.contains(token))
}

fn auto_review_target(cwd: &Path, files: &[String]) -> String {
    let mut shown = files
        .iter()
        .take(16)
        .map(|file| normalize_repo_relative(cwd, file))
        .collect::<Vec<_>>();
    let suffix = if files.len() > shown.len() {
        format!(" plus {} more", files.len() - shown.len())
    } else {
        String::new()
    };
    if shown.is_empty() {
        "current git diff".to_owned()
    } else {
        shown.sort();
        format!(
            "current git diff limited to edited files: {}{}",
            shown.join(", "),
            suffix
        )
    }
}

async fn review_signature(cwd: &Path, files: &[String]) -> String {
    let mut hasher = Sha256::new();
    let mut sorted = files.to_vec();
    sorted.sort();
    for file in &sorted {
        hasher.update(file.as_bytes());
        hasher.update([0]);
    }
    hash_git_output(&mut hasher, cwd, &["status", "--short", "--"], &sorted).await;
    hash_git_output(
        &mut hasher,
        cwd,
        &["diff", "--numstat", "HEAD", "--"],
        &sorted,
    )
    .await;
    hex::encode(&hasher.finalize()[..16])
}

async fn hash_git_output(hasher: &mut Sha256, cwd: &Path, prefix: &[&str], files: &[String]) {
    let mut cmd = tokio::process::Command::new("git");
    cmd.current_dir(cwd).args(prefix).args(files);
    if let Ok(output) = cmd.output().await {
        hasher.update(output.stdout);
        hasher.update(output.stderr);
    }
}

fn workflow_session_dir(session_id: Option<&str>, run_id: &str) -> PathBuf {
    let base = jfc_session::sessions_dir();
    match session_id {
        Some(session_id) => base.join(session_id).join("workflows").join(run_id),
        None => base.join("workflows").join(run_id),
    }
}

fn build_auto_review_notification(
    task_id: &str,
    outcome: &crate::workflows::WorkflowOutcome,
    elapsed_ms: u64,
) -> String {
    let findings = outcome
        .result
        .get("findings")
        .and_then(serde_json::Value::as_array)
        .map_or(0, Vec::len);
    let dismissed = outcome
        .result
        .get("dismissed")
        .and_then(serde_json::Value::as_array)
        .map_or(0, Vec::len);
    let diagnostics = outcome
        .result
        .get("diagnostics")
        .and_then(serde_json::Value::as_array)
        .map_or(0, Vec::len);
    let result_json = serde_json::to_string(&outcome.result).unwrap_or_default();
    let truncated: String = result_json.chars().take(8000).collect();
    format!(
        "<task-notification>\n<task-id>{task_id}</task-id>\n<status>completed</status>\n\
         <summary>Auto-review completed: {findings} finding(s), {dismissed} dismissed, \
         {diagnostics} diagnostic(s).</summary>\n<result>{truncated}</result>\n\
         <usage><agent_count>{}</agent_count><agents_dispatched>{}</agents_dispatched>\
         <cache_hits>{}</cache_hits><duration_ms>{elapsed_ms}</duration_ms></usage>\n\
         </task-notification>",
        outcome.agent_count, outcome.total_agents_dispatched, outcome.cache_hits
    )
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewRunRecord {
    pub schema_version: u8,
    pub run_id: String,
    pub created_at_ms: u128,
    pub source: String,
    pub level: Option<String>,
    pub target: Option<String>,
    pub files: Vec<String>,
    pub finding_fingerprints: Vec<String>,
    pub duplicate_fingerprints: Vec<String>,
    pub error: Option<String>,
    pub result: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewFindingRecord {
    pub schema_version: u8,
    pub run_id: String,
    pub created_at_ms: u128,
    pub fingerprint: String,
    pub duplicate: bool,
    pub file: Option<String>,
    pub line: Option<u64>,
    pub severity: Option<String>,
    pub category: Option<String>,
    pub summary: Option<String>,
    pub evidence: Option<String>,
    pub confidence: Option<f64>,
}

pub async fn persist_code_review_outcome(
    cwd: &Path,
    run_id: &str,
    source: &str,
    args: &serde_json::Value,
    result: &serde_json::Value,
    error: Option<&str>,
) {
    if let Err(err) = persist_code_review_outcome_inner(cwd, run_id, source, args, result, error)
        .await
        .map(|_| ())
    {
        tracing::warn!(
            target: "jfc::auto_review",
            run_id,
            error = %err,
            "failed to persist code-review outcome"
        );
    }
}

pub async fn persist_code_review_outcome_event(
    cwd: &Path,
    run_id: &str,
    source: &str,
    args: &serde_json::Value,
    result: &serde_json::Value,
    error: Option<&str>,
) -> Option<crate::review::ReviewOutputEvent> {
    match persist_code_review_outcome_inner(cwd, run_id, source, args, result, error).await {
        Ok(review) => Some(review),
        Err(err) => {
            tracing::warn!(
                target: "jfc::auto_review",
                run_id,
                error = %err,
                "failed to persist code-review outcome"
            );
            None
        }
    }
}

async fn persist_code_review_outcome_inner(
    cwd: &Path,
    run_id: &str,
    source: &str,
    args: &serde_json::Value,
    result: &serde_json::Value,
    error: Option<&str>,
) -> std::io::Result<crate::review::ReviewOutputEvent> {
    let created_at_ms = now_ms();
    // Deterministic review-output repair: parse a stringified body and
    // canonicalize review key synonyms (final_report/summary/confidence) before
    // extraction/normalization, so an off-spec-but-recoverable review payload
    // isn't dropped. Findings are traced for observability.
    let repaired = crate::response_processor::review_repair_chain().process(result.clone());
    crate::response_processor::record_processor_findings(run_id, &repaired.findings);
    let result = &repaired.value;
    let existing = load_existing_fingerprints(cwd).await;
    let (findings, duplicates) = extract_finding_records(run_id, created_at_ms, result, &existing);
    let finding_fingerprints = findings
        .iter()
        .map(|finding| finding.fingerprint.clone())
        .collect::<Vec<_>>();
    let duplicate_fingerprints = duplicates.into_iter().collect::<Vec<_>>();
    let record = ReviewRunRecord {
        schema_version: 1,
        run_id: run_id.to_owned(),
        created_at_ms,
        source: source.to_owned(),
        level: args
            .get("level")
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned),
        target: args
            .get("target")
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned),
        files: args
            .get("files")
            .and_then(serde_json::Value::as_array)
            .map(|files| {
                files
                    .iter()
                    .filter_map(serde_json::Value::as_str)
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default(),
        finding_fingerprints,
        duplicate_fingerprints,
        error: error.map(str::to_owned),
        result: result.clone(),
    };

    append_review_artifact(cwd, "runs", run_id, &record).await?;
    for finding in findings {
        append_review_artifact(cwd, "findings", &finding.fingerprint, &finding).await?;
    }
    let review_event =
        crate::review::normalize_review_output(cwd, run_id, source, args, result, &existing);
    crate::review::persist_review_output(cwd, &review_event).await?;
    Ok(review_event)
}

const REVIEW_ARTIFACT_SESSION_ID: &str = "__auto_review__";
const REVIEW_ARTIFACT_KIND: &str = "auto_review";
const REVIEW_FINGERPRINT_LIMIT: usize = 10_000;

fn auto_review_artifact_key(cwd: &Path, stream: &str, key: &str) -> String {
    let project_key = jfc_knowledge::project_key(cwd);
    format!("{project_key}:{stream}:{key}")
}

async fn append_review_artifact<T: Serialize>(
    cwd: &Path,
    stream: &str,
    key: &str,
    value: &T,
) -> std::io::Result<()> {
    let artifact_key = auto_review_artifact_key(cwd, stream, key);
    let value_json = serde_json::to_string(value).map_err(std::io::Error::other)?;
    tokio::task::spawn_blocking(move || {
        jfc_knowledge::block_on_knowledge(async {
            let store = jfc_knowledge::KnowledgeStore::open_default()
                .await
                .map_err(std::io::Error::other)?;
            store
                .append_session_artifact_event(
                    REVIEW_ARTIFACT_SESSION_ID,
                    REVIEW_ARTIFACT_KIND,
                    &artifact_key,
                    &value_json,
                )
                .await
                .map_err(std::io::Error::other)?;
            Ok(())
        })
    })
    .await
    .map_err(std::io::Error::other)?
}

async fn load_existing_fingerprints(cwd: &Path) -> HashSet<String> {
    let project_key = jfc_knowledge::project_key(cwd);
    tokio::task::spawn_blocking(move || {
        jfc_knowledge::block_on_knowledge(async {
            let store = jfc_knowledge::KnowledgeStore::open_default().await.ok()?;
            let rows = store
                .list_recent_session_artifact_events(
                    REVIEW_ARTIFACT_SESSION_ID,
                    REVIEW_ARTIFACT_KIND,
                    None,
                    REVIEW_FINGERPRINT_LIMIT,
                )
                .await
                .ok()?;
            Some(
                rows.into_iter()
                    .filter(|row| row.key.starts_with(&format!("{project_key}:findings:")))
                    .filter_map(|row| {
                        serde_json::from_str::<ReviewFindingRecord>(&row.value_json).ok()
                    })
                    .map(|record| record.fingerprint)
                    .collect::<HashSet<_>>(),
            )
        })
    })
    .await
    .ok()
    .flatten()
    .unwrap_or_default()
}

fn extract_finding_records(
    run_id: &str,
    created_at_ms: u128,
    result: &serde_json::Value,
    existing: &HashSet<String>,
) -> (Vec<ReviewFindingRecord>, BTreeSet<String>) {
    let mut seen = HashSet::new();
    let mut duplicates = BTreeSet::new();
    let mut out = Vec::new();
    let Some(items) = result.get("findings").and_then(serde_json::Value::as_array) else {
        return (out, duplicates);
    };
    for item in items {
        let fingerprint = finding_fingerprint(item);
        let duplicate = existing.contains(&fingerprint) || !seen.insert(fingerprint.clone());
        if duplicate {
            duplicates.insert(fingerprint.clone());
        }
        out.push(ReviewFindingRecord {
            schema_version: 1,
            run_id: run_id.to_owned(),
            created_at_ms,
            fingerprint,
            duplicate,
            file: string_field(item, "file"),
            line: item.get("line").and_then(serde_json::Value::as_u64),
            severity: string_field(item, "severity"),
            category: string_field(item, "category"),
            summary: string_field(item, "summary"),
            evidence: string_field(item, "evidence"),
            confidence: item.get("confidence").and_then(serde_json::Value::as_f64),
        });
    }
    (out, duplicates)
}

fn finding_fingerprint(item: &serde_json::Value) -> String {
    let mut hasher = Sha256::new();
    for key in ["file", "line", "category", "summary"] {
        if let Some(value) = item.get(key) {
            hasher.update(normalized_json(value).as_bytes());
        }
        hasher.update([0]);
    }
    hex::encode(&hasher.finalize()[..16])
}

fn normalized_json(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(s) => s.trim().to_lowercase(),
        other => other.to_string(),
    }
}

fn string_field(value: &serde_json::Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned)
}

fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

pub fn apply_patch_paths(patch: &str) -> BTreeSet<String> {
    let mut paths = BTreeSet::new();
    for line in patch.lines() {
        for prefix in [
            "*** Add File: ",
            "*** Update File: ",
            "*** Delete File: ",
            "*** Move to: ",
        ] {
            if let Some(path) = line.strip_prefix(prefix) {
                let path = path.trim();
                if !path.is_empty() {
                    paths.insert(path.to_owned());
                }
            }
        }
    }
    paths
}

#[cfg(test)]
mod tests {
    use super::*;

    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn auto_review_mode_parses_known_values_normal() {
        // SAFETY: tests run single-threaded per-process for env mutation here;
        // each case sets then restores the var.
        for (val, expect_smart, expect_off, expect_always, expect_manual) in [
            ("smart", true, false, false, false),
            ("", true, false, false, false),
            ("off", false, true, false, false),
            ("0", false, true, false, false),
            ("always", false, false, true, false),
            ("on", false, false, true, false),
            ("manual", false, false, false, true),
            ("garbage", true, false, false, false),
        ] {
            unsafe { std::env::set_var("JFC_AUTO_REVIEW", val) };
            let mode = auto_review_mode();
            assert_eq!(matches!(mode, AutoReviewMode::Smart), expect_smart, "{val}");
            assert_eq!(matches!(mode, AutoReviewMode::Off), expect_off, "{val}");
            assert_eq!(
                matches!(mode, AutoReviewMode::Always),
                expect_always,
                "{val}"
            );
            assert_eq!(
                matches!(mode, AutoReviewMode::Manual),
                expect_manual,
                "{val}"
            );
        }
        unsafe { std::env::remove_var("JFC_AUTO_REVIEW") };
    }

    // Config-driven mode resolution (the new opt-out). Pure function over a
    // `Config` value — no env, no global state, so it's race-free.
    #[test]
    fn auto_review_mode_from_config_resolves_opt_out_normal() {
        use jfc_config::{ArgusAutoReviewConfig, Config};

        // Absent section → Smart (unchanged default).
        let mut cfg = Config::default();
        cfg.argus_auto_review = None;
        assert!(matches!(
            auto_review_mode_from_config(&cfg),
            AutoReviewMode::Smart
        ));

        // enabled = false → Off (the user opt-out).
        cfg.argus_auto_review = Some(ArgusAutoReviewConfig {
            enabled: Some(false),
            ..Default::default()
        });
        assert!(matches!(
            auto_review_mode_from_config(&cfg),
            AutoReviewMode::Off
        ));

        // enabled = true → Smart.
        cfg.argus_auto_review = Some(ArgusAutoReviewConfig {
            enabled: Some(true),
            ..Default::default()
        });
        assert!(matches!(
            auto_review_mode_from_config(&cfg),
            AutoReviewMode::Smart
        ));

        // Setting only an unrelated field (model) must NOT disable the feature
        // (this is why `enabled` is Option<bool>, not bool).
        cfg.argus_auto_review = Some(ArgusAutoReviewConfig {
            model: Some("haiku".into()),
            ..Default::default()
        });
        assert!(matches!(
            auto_review_mode_from_config(&cfg),
            AutoReviewMode::Smart
        ));
    }

    #[test]
    fn auto_review_mode_from_config_mode_field_takes_precedence_robust() {
        use jfc_config::{ArgusAutoReviewConfig, Config};

        // `mode` wins over `enabled`: enabled=true but mode="off" → Off.
        let mut cfg = Config::default();
        cfg.argus_auto_review = Some(ArgusAutoReviewConfig {
            enabled: Some(true),
            mode: Some("off".into()),
            ..Default::default()
        });
        assert!(matches!(
            auto_review_mode_from_config(&cfg),
            AutoReviewMode::Off
        ));

        // Every recognized mode token parses.
        for (m, is_always, is_manual) in [("always", true, false), ("manual", false, true)] {
            cfg.argus_auto_review = Some(ArgusAutoReviewConfig {
                mode: Some(m.into()),
                ..Default::default()
            });
            let mode = auto_review_mode_from_config(&cfg);
            assert_eq!(matches!(mode, AutoReviewMode::Always), is_always, "{m}");
            assert_eq!(matches!(mode, AutoReviewMode::Manual), is_manual, "{m}");
        }

        // An empty/whitespace mode string falls through to the `enabled` check.
        cfg.argus_auto_review = Some(ArgusAutoReviewConfig {
            enabled: Some(false),
            mode: Some("   ".into()),
            ..Default::default()
        });
        assert!(matches!(
            auto_review_mode_from_config(&cfg),
            AutoReviewMode::Off
        ));
    }

    #[test]
    fn normalize_repo_relative_strips_cwd_prefix_robust() {
        let cwd = Path::new("/home/u/proj");
        assert_eq!(
            normalize_repo_relative(cwd, "/home/u/proj/crates/x/Cargo.toml"),
            "crates/x/Cargo.toml"
        );
        // Already-relative passes through.
        assert_eq!(
            normalize_repo_relative(cwd, "crates/x/src/lib.rs"),
            "crates/x/src/lib.rs"
        );
        // Outside cwd is left intact.
        assert_eq!(normalize_repo_relative(cwd, "/etc/hosts"), "/etc/hosts");
    }

    #[test]
    fn is_code_sensitive_recognizes_rust_and_build_paths_robust() {
        let cwd = Path::new("/home/u/proj");
        for p in [
            "/home/u/proj/crates/x/src/lib.rs",
            "/home/u/proj/crates/x/Cargo.toml",
            "/home/u/proj/Cargo.toml",
            "/home/u/proj/crates/x/README.md", // under crates/ prefix
            "/home/u/proj/.github/workflows/ci.yml",
        ] {
            assert!(is_code_sensitive(cwd, p), "should be sensitive: {p}");
        }
        for p in [
            "/home/u/proj/docs/notes.md",
            "/home/u/proj/README.md",
            "/home/u/proj/note.txt",
        ] {
            assert!(!is_code_sensitive(cwd, p), "should be benign: {p}");
        }
    }

    // Normal: a tiny prose/doc edit buckets to Skip (no LLM, no review).
    #[test]
    fn monitor_skips_trivial_doc_edit_normal() {
        let signal = RiskSignal {
            file_count: 1,
            changed_lines: 3,
            risky_token: false,
            code_sensitive: false,
            score: 0,
        };
        assert_eq!(signal.monitor_outcome(), ReviewMonitorOutcome::Skip);
        assert_eq!(signal.adaptive_level(), "low");
    }

    // Normal: a risk-token diff buckets to Review and escalates the level.
    #[test]
    fn monitor_reviews_risky_diff_normal() {
        let signal = RiskSignal {
            file_count: 1,
            changed_lines: 10,
            risky_token: true,
            code_sensitive: true,
            score: 9,
        };
        assert_eq!(signal.monitor_outcome(), ReviewMonitorOutcome::Review);
        assert_eq!(signal.adaptive_level(), "high");
    }

    // Robust: a mid-size code edit with no risk token is genuinely uncertain →
    // Ambiguous, deferring to the gate.
    #[test]
    fn monitor_ambiguous_on_small_code_edit_robust() {
        let signal = RiskSignal {
            file_count: 1,
            changed_lines: 20,
            risky_token: false,
            code_sensitive: true,
            score: 2,
        };
        assert_eq!(signal.monitor_outcome(), ReviewMonitorOutcome::Ambiguous);
    }

    // Normal: review_decision honors Always, commit forcing, and the barrier.
    #[test]
    fn review_decision_folds_signals_normal() {
        // Always ignores the monitor.
        assert!(matches!(
            review_decision(
                AutoReviewMode::Always,
                ReviewMonitorOutcome::Skip,
                0,
                12,
                false
            ),
            ReviewDecision::Fire(_)
        ));
        // Commit forces even a Skip bucket.
        assert!(matches!(
            review_decision(
                AutoReviewMode::Smart,
                ReviewMonitorOutcome::Skip,
                0,
                12,
                true
            ),
            ReviewDecision::Fire(_)
        ));
        // Review bucket fires.
        assert!(matches!(
            review_decision(
                AutoReviewMode::Smart,
                ReviewMonitorOutcome::Review,
                0,
                12,
                false
            ),
            ReviewDecision::Fire(_)
        ));
        // Ambiguous below barrier asks the gate.
        assert_eq!(
            review_decision(
                AutoReviewMode::Smart,
                ReviewMonitorOutcome::Ambiguous,
                5,
                12,
                false
            ),
            ReviewDecision::AskGate
        );
        // Ambiguous at/above barrier fires.
        assert!(matches!(
            review_decision(
                AutoReviewMode::Smart,
                ReviewMonitorOutcome::Ambiguous,
                12,
                12,
                false
            ),
            ReviewDecision::Fire(_)
        ));
        // Skip below barrier skips.
        assert_eq!(
            review_decision(
                AutoReviewMode::Smart,
                ReviewMonitorOutcome::Skip,
                3,
                12,
                false
            ),
            ReviewDecision::Skip
        );
        // Buffered trivial edits eventually cross the barrier.
        assert!(matches!(
            review_decision(
                AutoReviewMode::Smart,
                ReviewMonitorOutcome::Skip,
                12,
                12,
                false
            ),
            ReviewDecision::Fire(_)
        ));
    }

    // Robust: the gate parser accepts a bare object and a tool-wrapped object,
    // and the should_review field drives the decision.
    #[test]
    fn parse_gate_accepts_object_and_wrapped_robust() {
        let bare = jfc_provider::CompletionResponse {
            content: r#"{"should_review": false, "reason": "doc only"}"#.to_owned(),
            usage: jfc_provider::TokenUsage::default(),
            context_signals: None,
            reasoning: None,
        };
        let g = parse_gate(&bare).unwrap();
        assert!(!g.should_review);

        let wrapped = jfc_provider::CompletionResponse {
            content: r#"prefix {"input": {"should_review": true, "reason": "api change"}} suffix"#
                .to_owned(),
            usage: jfc_provider::TokenUsage::default(),
            context_signals: None,
            reasoning: None,
        };
        let g = parse_gate(&wrapped).unwrap();
        assert!(g.should_review);
        assert_eq!(g.reason, "api change");
    }

    // Robust: content-hash memoization round-trips and merges.
    #[tokio::test]
    async fn content_hashes_save_and_load_merge_robust() {
        let tmp = tempfile::TempDir::new().unwrap();
        let _guard = db_env_guard(tmp.path());
        let dir = tmp.path();
        let mut a = std::collections::BTreeMap::new();
        a.insert("src/lib.rs".to_owned(), "hash1".to_owned());
        save_content_hashes(dir, &a).await;
        let mut b = std::collections::BTreeMap::new();
        b.insert("src/other.rs".to_owned(), "hash2".to_owned());
        save_content_hashes(dir, &b).await;
        let loaded = load_content_hashes(dir).await;
        assert_eq!(loaded.get("src/lib.rs").map(String::as_str), Some("hash1"));
        assert_eq!(
            loaded.get("src/other.rs").map(String::as_str),
            Some("hash2")
        );
    }

    #[test]
    fn auto_review_target_normalizes_and_sorts_normal() {
        let cwd = Path::new("/home/u/proj");
        let files = vec![
            "/home/u/proj/crates/z/src/b.rs".to_owned(),
            "/home/u/proj/crates/a/src/a.rs".to_owned(),
        ];
        let target = auto_review_target(cwd, &files);
        assert_eq!(
            target,
            "current git diff limited to edited files: \
             crates/a/src/a.rs, crates/z/src/b.rs"
        );
    }

    #[test]
    fn auto_review_target_empty_is_full_diff_normal() {
        let cwd = Path::new("/home/u/proj");
        assert_eq!(auto_review_target(cwd, &[]), "current git diff");
    }

    #[tokio::test]
    async fn load_existing_fingerprints_reads_matching_db_events_robust() {
        let tmp = tempfile::TempDir::new().unwrap();
        let _guard = db_env_guard(tmp.path());
        let cwd = tmp.path().join("repo");
        let other = tmp.path().join("other");
        tokio::fs::create_dir_all(&cwd).await.unwrap();
        tokio::fs::create_dir_all(&other).await.unwrap();
        let rec = ReviewFindingRecord {
            schema_version: 1,
            run_id: "r".to_owned(),
            created_at_ms: 0,
            fingerprint: "fp-current".to_owned(),
            duplicate: false,
            file: Some("src/lib.rs".to_owned()),
            line: Some(1),
            severity: None,
            category: None,
            summary: None,
            evidence: None,
            confidence: None,
        };
        let other_rec = ReviewFindingRecord {
            fingerprint: "fp-other".to_owned(),
            ..rec.clone()
        };
        append_review_artifact(&cwd, "findings", &rec.fingerprint, &rec)
            .await
            .unwrap();
        append_review_artifact(&other, "findings", &other_rec.fingerprint, &other_rec)
            .await
            .unwrap();

        let fps = load_existing_fingerprints(&cwd).await;
        assert!(fps.contains("fp-current"));
        assert!(!fps.contains("fp-other"));
    }

    #[test]
    fn apply_patch_paths_extracts_changed_files_normal() {
        let patch = "*** Begin Patch\n*** Update File: src/lib.rs\n@@\n test\n*** Add File: src/new.rs\n+test\n*** Move to: src/moved.rs\n*** End Patch\n";
        let paths = apply_patch_paths(patch);
        assert!(paths.contains("src/lib.rs"));
        assert!(paths.contains("src/new.rs"));
        assert!(paths.contains("src/moved.rs"));
        assert_eq!(paths.len(), 3);
    }

    #[test]
    fn finding_fingerprint_is_stable_for_case_and_whitespace_robust() {
        let left = serde_json::json!({
            "file": "src/lib.rs",
            "line": 10,
            "category": "Logic",
            "summary": "  Missing check "
        });
        let right = serde_json::json!({
            "file": "src/lib.rs",
            "line": 10,
            "category": "logic",
            "summary": "missing check"
        });
        assert_eq!(finding_fingerprint(&left), finding_fingerprint(&right));
    }

    #[tokio::test]
    async fn persist_code_review_outcome_dedups_previous_findings_normal() {
        let tmp = tempfile::TempDir::new().unwrap();
        let _guard = db_env_guard(tmp.path());
        let result = serde_json::json!({
            "findings": [{
                "file": "src/lib.rs",
                "line": 10,
                "severity": "high",
                "category": "logic",
                "summary": "missing check",
                "evidence": "branch skips validation",
                "confidence": 0.9
            }]
        });
        let args = serde_json::json!({
            "level": "high",
            "target": "current diff",
            "files": ["src/lib.rs"]
        });
        persist_code_review_outcome_inner(tmp.path(), "run_a", "auto", &args, &result, None)
            .await
            .unwrap();
        persist_code_review_outcome_inner(tmp.path(), "run_b", "auto", &args, &result, None)
            .await
            .unwrap();

        let project_key = jfc_knowledge::project_key(tmp.path());
        let store = jfc_knowledge::KnowledgeStore::open_default().await.unwrap();
        let parsed = store
            .list_recent_session_artifact_events(
                REVIEW_ARTIFACT_SESSION_ID,
                REVIEW_ARTIFACT_KIND,
                None,
                10,
            )
            .await
            .unwrap()
            .into_iter()
            .filter(|row| row.key.starts_with(&format!("{project_key}:findings:")))
            .map(|row| serde_json::from_str::<ReviewFindingRecord>(&row.value_json).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(parsed.len(), 2);
        assert!(!parsed[0].duplicate);
        assert!(parsed[1].duplicate);
    }

    fn db_env_guard(root: &Path) -> DbEnvGuard {
        let guard = ENV_LOCK.lock().unwrap_or_else(|error| error.into_inner());
        let prior = std::env::var("JFC_KNOWLEDGE_DB").ok();
        unsafe {
            std::env::set_var("JFC_KNOWLEDGE_DB", root.join("knowledge.db"));
        }
        DbEnvGuard {
            prior,
            _guard: guard,
        }
    }

    struct DbEnvGuard {
        prior: Option<String>,
        _guard: std::sync::MutexGuard<'static, ()>,
    }

    impl Drop for DbEnvGuard {
        fn drop(&mut self) {
            unsafe {
                match self.prior.take() {
                    Some(prior) => std::env::set_var("JFC_KNOWLEDGE_DB", prior),
                    None => std::env::remove_var("JFC_KNOWLEDGE_DB"),
                }
            }
        }
    }
}

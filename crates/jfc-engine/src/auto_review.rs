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
use tokio::io::AsyncWriteExt;

use crate::app::EngineState;
use crate::runtime::{EngineEvent, EventSender, FrontendEvent, TaskEvent};

/// Background auto-review dispatch state.
///
/// The dedup signature lives behind a shared `Arc<Mutex<_>>` so the
/// background task that actually runs the review can *clear* it when the run
/// fails. Without that feedback loop a single failed run (e.g. every agent
/// hitting a 401) would poison the signature forever and silently suppress all
/// future auto-reviews of the same file-set for the rest of the session.
#[derive(Debug, Default, Clone)]
pub struct AutoReviewState {
    last_dispatched_signature: Arc<parking_lot::Mutex<Option<String>>>,
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
    let trigger = match mode {
        AutoReviewMode::Always => "mode=always".to_owned(),
        AutoReviewMode::Smart => {
            let Some(reason) = smart_auto_review_trigger(&cwd, &files).await else {
                tracing::debug!(
                    target: "jfc::auto_review",
                    file_count = files.len(),
                    "smart auto-review found no review-worthy signal"
                );
                return;
            };
            reason
        }
        AutoReviewMode::Off | AutoReviewMode::Manual => return,
    };

    let Some(workflow) = crate::workflows::resolve(&cwd, "code-review") else {
        tracing::debug!(
            target: "jfc::auto_review",
            "code-review workflow unavailable; skipping auto-review"
        );
        return;
    };
    let decision =
        crate::workflows::permissions::decide(&crate::config::load_arc(), Some("code-review"));
    if decision == crate::workflows::permissions::WorkflowPermission::Deny {
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
    let target = auto_review_target(&files);
    let level = std::env::var("JFC_AUTO_REVIEW_LEVEL").unwrap_or_else(|_| "high".to_owned());
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

        let outcome = crate::workflows::run_workflow(crate::workflows::WorkflowRunConfig {
            run_id: run_id.clone(),
            script_body: body,
            args: args.clone(),
            provider,
            model,
            session_dir,
            resume_from_run_id: None,
            cancel,
            tx: Some(tx_bg.clone()),
            workflow_task_id: task_id.clone(),
            depth: 0,
            cwd: cwd.clone(),
            token_budget: None,
        })
        .await;
        let elapsed_ms = started.elapsed().as_millis() as u64;

        let review = persist_code_review_outcome_event(
            &cwd,
            &run_id,
            "auto",
            &args,
            &outcome.result,
            outcome.error.as_deref(),
        )
        .await;

        if let Some(error) = outcome.error {
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
    match std::env::var("JFC_AUTO_REVIEW") {
        Ok(value) => match value.trim().to_ascii_lowercase().as_str() {
            "0" | "false" | "off" | "no" => AutoReviewMode::Off,
            "manual" => AutoReviewMode::Manual,
            "always" | "1" | "true" | "on" | "yes" => AutoReviewMode::Always,
            "smart" | "" => AutoReviewMode::Smart,
            other => {
                tracing::warn!(
                    target: "jfc::auto_review",
                    value = other,
                    "unknown JFC_AUTO_REVIEW mode; using smart"
                );
                AutoReviewMode::Smart
            }
        },
        Err(_) => AutoReviewMode::Smart,
    }
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

async fn smart_auto_review_trigger(cwd: &Path, files: &[String]) -> Option<String> {
    if files.len() >= 3 {
        return Some(format!("changed {} files", files.len()));
    }

    if files.iter().any(|file| {
        file.ends_with(".rs")
            || file == "Cargo.toml"
            || file == "Cargo.lock"
            || file.starts_with("crates/")
            || file.starts_with(".github/workflows/")
    }) {
        return Some("rust/workflow-sensitive file changed".to_owned());
    }

    for file in files {
        if file_content_has_review_signal(&cwd.join(file)).await {
            return Some(format!("risk token in {file}"));
        }
    }

    if git_diff_has_review_signal(cwd, files).await {
        return Some("risk token in edited diff".to_owned());
    }

    None
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

fn auto_review_target(files: &[String]) -> String {
    let mut shown = files.iter().take(16).cloned().collect::<Vec<_>>();
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
    let dir = cwd.join(".jfc").join("reviews");
    tokio::fs::create_dir_all(&dir).await?;
    let created_at_ms = now_ms();
    // Deterministic review-output repair: parse a stringified body and
    // canonicalize review key synonyms (final_report/summary/confidence) before
    // extraction/normalization, so an off-spec-but-recoverable review payload
    // isn't dropped. Findings are traced for observability.
    let repaired = crate::response_processor::review_repair_chain().process(result.clone());
    crate::response_processor::record_processor_findings(run_id, &repaired.findings);
    let result = &repaired.value;
    let existing = load_existing_fingerprints(&dir).await;
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

    append_jsonl(&dir.join("runs.jsonl"), &record).await?;
    for finding in findings {
        append_jsonl(&dir.join("findings.jsonl"), &finding).await?;
    }
    let review_event =
        crate::review::normalize_review_output(cwd, run_id, source, args, result, &existing);
    crate::review::persist_review_output(cwd, &review_event).await?;
    Ok(review_event)
}

async fn append_jsonl<T: Serialize>(path: &Path, value: &T) -> std::io::Result<()> {
    let mut file = tokio::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .await?;
    let line = serde_json::to_string(value).unwrap_or_else(|_| "{}".to_owned());
    file.write_all(line.as_bytes()).await?;
    file.write_all(b"\n").await
}

async fn load_existing_fingerprints(dir: &Path) -> HashSet<String> {
    let path = dir.join("findings.jsonl");
    let Ok(body) = tokio::fs::read_to_string(path).await else {
        return HashSet::new();
    };
    body.lines()
        .filter_map(|line| serde_json::from_str::<ReviewFindingRecord>(line).ok())
        .map(|record| record.fingerprint)
        .collect()
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

        let findings = tokio::fs::read_to_string(tmp.path().join(".jfc/reviews/findings.jsonl"))
            .await
            .unwrap();
        let parsed = findings
            .lines()
            .map(|line| serde_json::from_str::<ReviewFindingRecord>(line).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(parsed.len(), 2);
        assert!(!parsed[0].duplicate);
        assert!(parsed[1].duplicate);
    }
}

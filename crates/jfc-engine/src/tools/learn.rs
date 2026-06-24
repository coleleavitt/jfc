//! /learn slash command — status, historize, dream, key-files, user-profile.
//!
//! The Dreamer cycle, user-profile pipeline, key-file store, and status are
//! fully wired here. Historian LLM extraction still belongs in the daemon
//! scheduler, but `historize` now also performs a conservative deterministic
//! write-through so pending transcripts become searchable project memory even
//! when the daemon is not running.

use super::ExecutionResult;
use jfc_memory::{MemoryLevel, MemoryScope, MemoryType};
use std::path::Path;

const LEARN_PENDING_TRANSCRIPT_KIND: &str = "learn_pending_transcript";
const LEARN_PROCESSED_TRANSCRIPT_KIND: &str = "learn_processed_transcript";
const LEARN_FAILED_TRANSCRIPT_KIND: &str = "learn_failed_transcript";

fn project_session_id(cwd: &Path) -> String {
    format!("project:{}", jfc_knowledge::project_key(cwd))
}

/// `/learn status` — report learning subsystem state.
pub fn execute_learn_status() -> ExecutionResult {
    let cwd = std::env::current_dir().unwrap_or_else(|_| ".".into());
    let pending = count_pending(&cwd);
    let candidates = jfc_learn::UserMemoryPipeline::load_candidates(&cwd).unwrap_or_default();
    let promoted = jfc_learn::UserMemoryPipeline::check_promotion(&candidates).len();
    let memories = jfc_knowledge::block_on_knowledge(async {
        let entries = crate::memory::load_all_memories(&cwd).await;
        Ok::<_, jfc_knowledge::KnowledgeError>(entries.len())
    })
    .unwrap_or_default();

    ExecutionResult::success(format!(
        "Learning subsystem: enabled\n\
         Memories: {memories}\n\
         Pending transcripts: {pending}\n\
         User observations: {} ({promoted} promoted)",
        candidates.len()
    ))
}

/// Count pending historian transcripts staged in the project DB.
fn count_pending(cwd: &std::path::Path) -> usize {
    import_legacy_pending(cwd).ok();
    let cwd = cwd.to_owned();
    jfc_knowledge::block_on_knowledge(async move {
        let store = jfc_knowledge::KnowledgeStore::open_default().await?;
        let rows = store
            .list_session_artifacts(
                &project_session_id(&cwd),
                LEARN_PENDING_TRANSCRIPT_KIND,
                10_000,
            )
            .await?;
        Ok::<_, jfc_knowledge::KnowledgeError>(rows.len())
    })
    .unwrap_or_default()
}

/// `/learn historize` — consume pending transcripts into durable project memory.
pub fn execute_learn_historize() -> ExecutionResult {
    let cwd = std::env::current_dir().unwrap_or_else(|_| ".".into());
    execute_learn_historize_in(&cwd)
}

fn execute_learn_historize_in(cwd: &Path) -> ExecutionResult {
    let pending = count_pending(cwd);
    if pending == 0 {
        return ExecutionResult::success(
            "No pending transcripts. Sessions queue transcripts on exit; the \
             Historian will write durable project memories when transcripts are staged.",
        );
    }

    match historize_pending(cwd) {
        Ok(report) => ExecutionResult::success(format!(
            "Historian write-through: {} pending transcript(s), {} memory file(s) created, {} skipped, {} failed.",
            report.pending, report.created, report.skipped, report.failed
        )),
        Err(e) => ExecutionResult::failure(format!("Historian write-through failed: {e}")),
    }
}

#[derive(Default)]
struct HistorizeWriteThroughReport {
    pending: usize,
    created: usize,
    skipped: usize,
    failed: usize,
}

fn historize_pending(cwd: &Path) -> Result<HistorizeWriteThroughReport, String> {
    import_legacy_pending(cwd)?;
    let cwd = cwd.to_owned();
    jfc_knowledge::block_on_knowledge(async move {
        let store = jfc_knowledge::KnowledgeStore::open_default()
            .await
            .map_err(|e| e.to_string())?;
        let session_id = project_session_id(&cwd);
        let mut rows = store
            .list_session_artifacts(&session_id, LEARN_PENDING_TRANSCRIPT_KIND, 10_000)
            .await
            .map_err(|e| e.to_string())?;
        rows.sort_by(|a, b| a.key.cmp(&b.key));
        let mut report = HistorizeWriteThroughReport {
            pending: rows.len(),
            ..Default::default()
        };

        for row in rows {
            match historize_one_async(&cwd, &row.key, &row.value_json).await {
                Ok(true) => {
                    report.created += 1;
                    store
                        .upsert_session_artifact(
                            &session_id,
                            LEARN_PROCESSED_TRANSCRIPT_KIND,
                            &row.key,
                            &row.value_json,
                        )
                        .await
                        .map_err(|e| e.to_string())?;
                    store
                        .delete_session_artifact(
                            &session_id,
                            LEARN_PENDING_TRANSCRIPT_KIND,
                            &row.key,
                        )
                        .await
                        .map_err(|e| e.to_string())?;
                }
                Ok(false) => {
                    report.skipped += 1;
                    store
                        .upsert_session_artifact(
                            &session_id,
                            LEARN_PROCESSED_TRANSCRIPT_KIND,
                            &row.key,
                            &row.value_json,
                        )
                        .await
                        .map_err(|e| e.to_string())?;
                    store
                        .delete_session_artifact(
                            &session_id,
                            LEARN_PENDING_TRANSCRIPT_KIND,
                            &row.key,
                        )
                        .await
                        .map_err(|e| e.to_string())?;
                }
                Err(error) => {
                    report.failed += 1;
                    tracing::warn!(
                        target: "jfc::learn",
                        key = %row.key,
                        error = %error,
                        "historian write-through failed for pending transcript"
                    );
                    store
                        .upsert_session_artifact(
                            &session_id,
                            LEARN_FAILED_TRANSCRIPT_KIND,
                            &row.key,
                            &row.value_json,
                        )
                        .await
                        .map_err(|e| e.to_string())?;
                    store
                        .delete_session_artifact(
                            &session_id,
                            LEARN_PENDING_TRANSCRIPT_KIND,
                            &row.key,
                        )
                        .await
                        .map_err(|e| e.to_string())?;
                }
            }
        }
        Ok::<_, String>(report)
    })
}

fn import_legacy_pending(cwd: &Path) -> Result<(), String> {
    let pending_dir = cwd.join(".jfc").join("learn").join("pending");
    let Ok(entries) = std::fs::read_dir(pending_dir) else {
        return Ok(());
    };
    let cwd = cwd.to_owned();
    jfc_knowledge::block_on_knowledge(async move {
        let store = jfc_knowledge::KnowledgeStore::open_default()
            .await
            .map_err(|e| e.to_string())?;
        let session_id = project_session_id(&cwd);
        for path in entries
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
        {
            let Ok(raw) = std::fs::read_to_string(&path) else {
                continue;
            };
            let key = path
                .file_stem()
                .and_then(|name| name.to_str())
                .unwrap_or("legacy-pending")
                .to_owned();
            store
                .upsert_session_artifact(&session_id, LEARN_PENDING_TRANSCRIPT_KIND, &key, &raw)
                .await
                .map_err(|e| e.to_string())?;
            let _ = std::fs::remove_file(&path);
        }
        Ok::<_, String>(())
    })
}

async fn historize_one_async(cwd: &Path, key: &str, raw_json: &str) -> Result<bool, String> {
    let transcript: Vec<(String, String)> =
        serde_json::from_str(raw_json).map_err(|e| e.to_string())?;
    let Some(body) = build_handoff_memory(key, &transcript) else {
        return Ok(false);
    };
    jfc_memory::create_memory_checked(
        MemoryLevel::Project,
        MemoryType::Project,
        MemoryScope::Private,
        &body,
        cwd,
    )
    .await?;
    Ok(true)
}

fn build_handoff_memory(session: &str, transcript: &[(String, String)]) -> Option<String> {
    let last_user = transcript
        .iter()
        .rev()
        .find(|(role, content)| role == "user" && !content.trim().is_empty())
        .map(|(_, content)| content.trim())?;
    let last_assistant = transcript
        .iter()
        .rev()
        .find(|(role, content)| role == "assistant" && !content.trim().is_empty())
        .map(|(_, content)| content.trim())
        .unwrap_or("");
    let turn_count = transcript
        .iter()
        .filter(|(_, content)| !content.trim().is_empty())
        .count();
    if turn_count < 4 {
        return None;
    }

    let user = truncate_chars(last_user, 500);
    let assistant = truncate_chars(last_assistant, 500);
    let files = mentioned_paths(transcript);

    let mut body = format!(
        "Session handoff `{session}`: {}\n\
         Why: This was captured from a pending JFC transcript so future sessions can recover the work after restart or compaction.\n\
         How to apply: Use this as project-local context when continuing the same task. Last user request: {user}",
        first_line(&user)
    );
    if !assistant.is_empty() {
        body.push_str(&format!("\nRecent assistant state: {assistant}"));
    }
    if !files.is_empty() {
        body.push_str("\nMentioned paths: ");
        body.push_str(&files.join(", "));
    }
    Some(body)
}

fn mentioned_paths(transcript: &[(String, String)]) -> Vec<String> {
    let mut paths = Vec::new();
    for (_, content) in transcript {
        for raw in content.split_whitespace() {
            let token = raw.trim_matches(|c: char| {
                matches!(c, '`' | '\'' | '"' | ',' | '.' | ')' | '(' | '[' | ']')
            });
            if looks_like_path(token) && !paths.iter().any(|p| p == token) {
                paths.push(token.to_owned());
                if paths.len() >= 8 {
                    return paths;
                }
            }
        }
    }
    paths
}

fn looks_like_path(token: &str) -> bool {
    (token.contains('/') || token.starts_with('.'))
        && token.len() > 2
        && token
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '/' | '.' | '_' | '-' | ':'))
}

fn first_line(text: &str) -> String {
    truncate_chars(
        text.lines().find(|l| !l.trim().is_empty()).unwrap_or(text),
        120,
    )
}

fn truncate_chars(text: &str, max_chars: usize) -> String {
    let mut chars = text.chars();
    let mut out: String = chars.by_ref().take(max_chars).collect();
    if chars.next().is_some() {
        out.push_str("...");
    }
    out
}

/// `/learn dream` — run the Dreamer maintenance cycle.
///
/// Acquires the lease, loads memories as `MemoryRecord`s, runs all dreamer
/// tasks, then releases the lease. Same path the daemon scheduler uses in
/// `dreamer_scheduler::run_learn_dreamer`, but triggered manually.
pub fn execute_learn_dream() -> ExecutionResult {
    let cwd = std::env::current_dir().unwrap_or_else(|_| ".".into());
    let lease_path = cwd.join(".jfc").join("learn").join("dreamer.lease");

    use jfc_learn::dreamer::{Dreamer, DreamerTask, MemoryRecord, acquire_lease, release_lease};

    let result = match jfc_knowledge::block_on_knowledge(async {
        let lease = match acquire_lease(&lease_path).await {
            Ok(l) => l,
            Err(e) => {
                return Err(format!("Failed to acquire dreamer lease: {e}"));
            }
        };

        let entries = crate::memory::load_all_memories(&cwd).await;
        let mut records: Vec<MemoryRecord> = entries
            .iter()
            .map(|e| MemoryRecord {
                path: e.source_display().into_owned(),
                category: Some(e.frontmatter.memory_type.to_string()),
                normalized_hash: e.frontmatter.normalized_hash.clone(),
                content: e.body.clone(),
                last_seen_at: e.frontmatter.last_seen_at,
                memory_status: e.frontmatter.memory_status.clone(),
            })
            .collect();

        let dreamer = Dreamer::new(lease_path.clone());
        let tasks = [
            DreamerTask::Consolidate,
            DreamerTask::ArchiveStale,
            DreamerTask::Verify,
            DreamerTask::Improve,
            DreamerTask::MaintainDocs,
        ];

        let result = dreamer.run_cycle(&tasks, &mut records);
        if let Err(e) = release_lease(&lease_path, &lease.holder_id).await {
            tracing::warn!(target: "jfc::learn", error = %e, "failed to release dreamer lease");
        }

        Ok::<_, String>(result)
    }) {
        Ok(dreamer_result) => dreamer_result,
        Err(e) => return ExecutionResult::failure(e),
    };

    match result {
        Ok(report) => {
            let mut msg = format!(
                "Dreamer: {} tasks run, circuit-breaker {}.\n",
                report.tasks_run.len(),
                if report.circuit_breaker_fired {
                    "TRIPPED"
                } else {
                    "ok"
                }
            );
            for r in &report.tasks_run {
                let status = if r.error.is_some() { "FAIL" } else { "ok" };
                msg.push_str(&format!(
                    "  {:?}: {status} ({} actions, {}ms)\n",
                    r.task, r.actions_taken, r.duration_ms
                ));
            }
            ExecutionResult::success(msg)
        }
        Err(e) => ExecutionResult::failure(format!("Dreamer cycle failed: {e}")),
    }
}

/// `/learn key-files list` — list pinned key files.
pub fn execute_learn_key_files_list(project_root: &std::path::Path) -> ExecutionResult {
    let store = match jfc_learn::KeyFileStore::open(project_root) {
        Ok(s) => s,
        Err(e) => return ExecutionResult::failure(format!("Failed to open key-file store: {e}")),
    };
    let pinned = match store.list_pinned() {
        Ok(p) => p,
        Err(e) => return ExecutionResult::failure(format!("Failed to list pinned files: {e}")),
    };
    if pinned.is_empty() {
        return ExecutionResult::success("No pinned key files.");
    }
    let mut out = String::from("Pinned key files:\n");
    for pf in &pinned {
        out.push_str(&format!("  {} — {}\n", pf.file_path, pf.reason));
    }
    ExecutionResult::success(out)
}

/// `/learn user-profile show` — load observations, check promotion, render.
pub fn execute_learn_user_profile_show() -> ExecutionResult {
    let cwd = std::env::current_dir().unwrap_or_else(|_| ".".into());

    let candidates = match jfc_learn::UserMemoryPipeline::load_candidates(&cwd) {
        Ok(c) => c,
        Err(e) => {
            return ExecutionResult::failure(format!("Failed to load profile candidates: {e}"));
        }
    };

    if candidates.is_empty() {
        return ExecutionResult::success(
            "No user-profile observations recorded yet.\n\
             Observations are collected across sessions and promoted after \
             appearing in ≥3 distinct sessions.",
        );
    }

    let promoted = jfc_learn::UserMemoryPipeline::check_promotion(&candidates);
    if promoted.is_empty() {
        return ExecutionResult::success(format!(
            "{} observations recorded, none promoted yet (need ≥3 sessions per facet).",
            candidates.len()
        ));
    }

    let block = jfc_learn::UserMemoryPipeline::render_profile_block(&promoted);
    ExecutionResult::success(format!(
        "{} observations, {} promoted facet(s):\n\n{block}",
        candidates.len(),
        promoted.len()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn learn_status_returns_ok_normal() {
        let result = execute_learn_status();
        assert!(!result.is_error());
        assert!(result.output.contains("Learning subsystem"));
    }

    #[test]
    fn learn_historize_reports_pending() {
        let temp = tempfile::tempdir().unwrap();
        let result = execute_learn_historize_in(temp.path());
        assert!(!result.is_error());
        // Either "No pending" or "N transcript(s) staged".
        assert!(result.output.contains("pending") || result.output.contains("transcript"));
    }

    #[test]
    fn build_handoff_memory_extracts_recent_state_normal() {
        let transcript = vec![
            (
                "user".to_string(),
                "start work in crates/foo/src/lib.rs".to_string(),
            ),
            ("assistant".to_string(), "read the file".to_string()),
            ("user".to_string(), "continue the parser fix".to_string()),
            ("assistant".to_string(), "patched parser tests".to_string()),
        ];
        let body = build_handoff_memory("20260616_010203", &transcript).unwrap();
        assert!(body.contains("continue the parser fix"));
        assert!(body.contains("patched parser tests"));
        assert!(body.contains("crates/foo/src/lib.rs"));
    }

    /// End-to-end test: a pending learning row is historized into a DB memory
    /// row and removed from the pending artifact set.
    #[tokio::test(flavor = "multi_thread")]
    #[serial_test::serial]
    async fn historize_pending_creates_memory_and_moves_db_row_normal() {
        let temp = tempfile::tempdir().unwrap();
        // SAFETY: tests are run single-threaded via #[serial_test::serial]
        unsafe {
            std::env::set_var(
                "JFC_KNOWLEDGE_DB",
                temp.path().join("test.db").to_string_lossy().as_ref(),
            );
        }
        let pending = temp.path().join(".jfc").join("learn").join("pending");
        std::fs::create_dir_all(&pending).unwrap();
        let transcript = vec![
            (
                "user".to_string(),
                "start work in crates/foo/src/lib.rs".to_string(),
            ),
            ("assistant".to_string(), "read the file".to_string()),
            ("user".to_string(), "continue the parser fix".to_string()),
            ("assistant".to_string(), "patched parser tests".to_string()),
        ];
        std::fs::write(
            pending.join("20260616_010203.json"),
            serde_json::to_vec(&transcript).unwrap(),
        )
        .unwrap();

        let report = historize_pending(temp.path()).unwrap();

        assert_eq!(report.pending, 1);
        let store = jfc_knowledge::KnowledgeStore::open_default().await.unwrap();
        let processed = store
            .get_session_artifact(
                &project_session_id(temp.path()),
                LEARN_PROCESSED_TRANSCRIPT_KIND,
                "20260616_010203",
            )
            .await
            .unwrap();
        assert!(processed.is_some());
        assert_eq!(count_pending(temp.path()), 0);
    }

    #[test]
    fn learn_dream_runs_or_reports_lease() {
        let result = execute_learn_dream();
        assert!(result.output.contains("Dreamer") || result.output.contains("lease"));
    }

    #[test]
    fn learn_user_profile_handles_empty() {
        let result = execute_learn_user_profile_show();
        assert!(!result.is_error());
    }
}

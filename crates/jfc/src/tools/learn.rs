//! /learn slash command — status, historize, dream, key-files, user-profile.
//!
//! The Dreamer cycle, user-profile pipeline, key-file store, and status are
//! fully wired here. Historian *extraction* needs an LLM provider, which the
//! synchronous tool-dispatch path does not have — so `historize` stages the
//! pending transcripts and reports readiness; the LLM extraction runs from the
//! daemon scheduler (`dreamer_scheduler`) which owns a provider.

use super::ExecutionResult;

/// `/learn status` — report learning subsystem state.
pub(super) fn execute_learn_status() -> ExecutionResult {
    let cwd = std::env::current_dir().unwrap_or_else(|_| ".".into());
    let pending = count_pending(&cwd);
    let candidates = jfc_learn::UserMemoryPipeline::load_candidates(&cwd).unwrap_or_default();
    let promoted = jfc_learn::UserMemoryPipeline::check_promotion(&candidates).len();
    let memories = crate::memory::load_all_memories(&cwd).len();

    ExecutionResult::success(format!(
        "Learning subsystem: enabled\n\
         Memories: {memories}\n\
         Pending transcripts: {pending}\n\
         User observations: {} ({promoted} promoted)",
        candidates.len()
    ))
}

/// Count pending historian transcripts under `.jfc/learn/pending/`.
fn count_pending(cwd: &std::path::Path) -> usize {
    let dir = cwd.join(".jfc").join("learn").join("pending");
    std::fs::read_dir(&dir)
        .map(|rd| {
            rd.filter_map(|e| e.ok())
                .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
                .count()
        })
        .unwrap_or(0)
}

/// `/learn historize` — report pending transcript readiness.
///
/// Extraction requires an LLM provider, which the tool-dispatch path doesn't
/// carry. The daemon's dreamer scheduler runs the actual Historian against a
/// real provider. This command surfaces how many transcripts are staged.
pub(super) fn execute_learn_historize() -> ExecutionResult {
    let cwd = std::env::current_dir().unwrap_or_else(|_| ".".into());
    let pending = count_pending(&cwd);
    if pending == 0 {
        return ExecutionResult::success(
            "No pending transcripts. Sessions queue transcripts on exit; the \
             Historian (run by the daemon scheduler) extracts facts from them.",
        );
    }
    ExecutionResult::success(format!(
        "{pending} transcript(s) staged for historization in \
         `.jfc/learn/pending/`. The daemon's dreamer scheduler runs the \
         Historian against the active provider to extract memories; run \
         `jfc daemon start` if it isn't already running."
    ))
}

/// `/learn dream` — run the Dreamer maintenance cycle.
///
/// Acquires the lease, loads memories as `MemoryRecord`s, runs all dreamer
/// tasks, then releases the lease. Same path the daemon scheduler uses in
/// `dreamer_scheduler::run_learn_dreamer`, but triggered manually.
pub(super) fn execute_learn_dream() -> ExecutionResult {
    let cwd = std::env::current_dir().unwrap_or_else(|_| ".".into());
    let lease_path = cwd.join(".jfc").join("learn").join("dreamer.lease");

    use jfc_learn::dreamer::{Dreamer, DreamerTask, MemoryRecord, acquire_lease, release_lease};

    let lease = match acquire_lease(&lease_path) {
        Ok(l) => l,
        Err(e) => {
            return ExecutionResult::failure(format!("Failed to acquire dreamer lease: {e}"));
        }
    };

    let entries = crate::memory::load_all_memories(&cwd);
    let mut records: Vec<MemoryRecord> = entries
        .iter()
        .map(|e| MemoryRecord {
            path: e.path.display().to_string(),
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
    if let Err(e) = release_lease(&lease_path, &lease.holder_id) {
        tracing::warn!(target: "jfc::learn", error = %e, "failed to release dreamer lease");
    }

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
pub(super) fn execute_learn_key_files_list(project_root: &std::path::Path) -> ExecutionResult {
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
pub(super) fn execute_learn_user_profile_show() -> ExecutionResult {
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
        let result = execute_learn_historize();
        assert!(!result.is_error());
        // Either "No pending" or "N transcript(s) staged".
        assert!(result.output.contains("pending") || result.output.contains("transcript"));
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

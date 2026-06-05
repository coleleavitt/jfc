use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::error::Result;
use crate::state::ChangeState;

/// A single changed file inside the proposal's worktree.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChangedFile {
    /// Path relative to the repo root.
    pub path: String,
    pub insertions: u32,
    pub deletions: u32,
}

/// The result of running one attached test command against the branch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TestRun {
    /// The exact command line that was executed (e.g. `cargo test -p jfc-ui`).
    pub command: String,
    /// Process exit code. `0` is the only passing value.
    pub exit_code: i32,
    /// Wall-clock duration of the run, in milliseconds.
    pub duration_ms: u64,
    /// Unix-epoch millis when the run completed.
    pub finished_at_ms: u64,
}

impl TestRun {
    /// A run passes iff it exited zero.
    pub fn passed(&self) -> bool {
        self.exit_code == 0
    }
}

/// Why/how a change reached the [`ChangeState::Approved`] state — the evidence
/// trail that gates a production apply.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Approval {
    /// A human operator approved the change.
    Human { user: String, at_ms: u64 },
    /// A validator quorum (economy mode) confirmed the change.
    ValidatorQuorum {
        confirmations: u32,
        total: u32,
        at_ms: u64,
    },
}

/// A durable, reviewable, reversible record of one mutating agent run.
///
/// This is the keystone object: worktrees, tool calls, approvals, tests,
/// session history, and revert all hang off a single `AgentChangeSet` so the
/// whole lifecycle (Dolt's branch → diff → test → merge → revert) is coherent
/// instead of scattered across separate primitives.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentChangeSet {
    /// Stable content-addressed id (see [`AgentChangeSet::compute_id`]).
    pub id: String,
    /// Current lifecycle position.
    pub state: ChangeState,

    // ── Provenance ────────────────────────────────────────────────────────
    /// Task this change fulfils, if dispatched from the task graph.
    pub task_id: Option<String>,
    /// Agent that produced the change (sub-agent name or economy AgentId).
    pub agent_id: Option<String>,
    /// Session the agent ran under.
    pub session_id: Option<String>,

    // ── Git isolation ─────────────────────────────────────────────────────
    /// `git rev-parse HEAD` at the moment the worktree/branch was created.
    pub base_head: String,
    /// Isolated branch name (e.g. `jfc/<name>`).
    pub branch: String,
    /// Absolute path of the worktree the agent mutated.
    pub worktree_path: String,

    // ── Change content ────────────────────────────────────────────────────
    pub changed_files: Vec<ChangedFile>,
    /// One-line summary, e.g. `3 files changed, 65 insertions(+), 1 deletion`.
    pub diff_summary: String,
    /// Path to a saved patch (`git diff` output) for offline review, if any.
    pub patch_path: Option<String>,

    // ── Ledger / review / lifecycle evidence ──────────────────────────────
    /// Opaque references into the runtime audit ledger (tool-call/event ids)
    /// produced by this change. Tagging them here makes the audit queryable
    /// per-change.
    pub ledger_refs: Vec<String>,
    /// Test commands attached for the review gate, plus their results.
    pub test_runs: Vec<TestRun>,
    /// Approval evidence; `Some` once the change reached `Approved`.
    pub approval: Option<Approval>,

    // ── Timestamps ────────────────────────────────────────────────────────
    pub created_at_ms: u64,
    pub updated_at_ms: u64,
}

impl AgentChangeSet {
    /// Open a fresh change-set in [`ChangeState::Draft`].
    ///
    /// `base_head`, `branch`, and `worktree_path` pin the git isolation; the
    /// id is content-addressed from those plus `now_ms` so two worktrees of
    /// the same branch at the same head still get distinct ids.
    pub fn open(
        base_head: impl Into<String>,
        branch: impl Into<String>,
        worktree_path: impl Into<String>,
        now_ms: u64,
    ) -> Self {
        let base_head = base_head.into();
        let branch = branch.into();
        let worktree_path = worktree_path.into();
        let id = Self::compute_id(&base_head, &branch, &worktree_path, now_ms);
        Self {
            id,
            state: ChangeState::Draft,
            task_id: None,
            agent_id: None,
            session_id: None,
            base_head,
            branch,
            worktree_path,
            changed_files: Vec::new(),
            diff_summary: String::new(),
            patch_path: None,
            ledger_refs: Vec::new(),
            test_runs: Vec::new(),
            approval: None,
            created_at_ms: now_ms,
            updated_at_ms: now_ms,
        }
    }

    /// Deterministic short id: first 16 hex chars of
    /// `sha256(base_head | branch | worktree_path | now_ms)`.
    pub fn compute_id(base_head: &str, branch: &str, worktree_path: &str, now_ms: u64) -> String {
        let mut hasher = Sha256::new();
        hasher.update(base_head.as_bytes());
        hasher.update(b"\0");
        hasher.update(branch.as_bytes());
        hasher.update(b"\0");
        hasher.update(worktree_path.as_bytes());
        hasher.update(b"\0");
        hasher.update(now_ms.to_le_bytes());
        let digest = hasher.finalize();
        let mut s = String::with_capacity(16);
        for byte in &digest[..8] {
            s.push_str(&format!("{byte:02x}"));
        }
        s
    }

    /// Attempt a lifecycle transition, stamping `updated_at_ms` on success.
    /// Returns the same `IllegalTransition` error the state machine raises so
    /// callers can surface exactly why an apply was refused.
    pub fn transition_to(&mut self, next: ChangeState, now_ms: u64) -> Result<()> {
        self.state.ensure_transition(next)?;
        self.state = next;
        self.updated_at_ms = now_ms;
        Ok(())
    }

    /// Record the agent's output (diff) and advance `Draft → Ready`.
    pub fn mark_ready(
        &mut self,
        changed_files: Vec<ChangedFile>,
        diff_summary: impl Into<String>,
        now_ms: u64,
    ) -> Result<()> {
        self.changed_files = changed_files;
        self.diff_summary = diff_summary.into();
        self.transition_to(ChangeState::Ready, now_ms)
    }

    /// Record a test run and advance `Ready → Tested`. The transition only
    /// fires from `Ready`; subsequent runs append without re-transitioning.
    pub fn record_test_run(&mut self, run: TestRun, now_ms: u64) -> Result<()> {
        self.test_runs.push(run);
        if self.state == ChangeState::Ready {
            self.transition_to(ChangeState::Tested, now_ms)?;
        } else {
            self.updated_at_ms = now_ms;
        }
        Ok(())
    }

    /// True iff at least one test run was recorded and all of them passed.
    pub fn all_tests_passed(&self) -> bool {
        !self.test_runs.is_empty() && self.test_runs.iter().all(TestRun::passed)
    }

    /// Approve the change (`Tested → Approved`). Refuses if no test run passed,
    /// so approval can never rubber-stamp an untested branch.
    pub fn approve(&mut self, approval: Approval, now_ms: u64) -> Result<()> {
        self.transition_to(ChangeState::Approved, now_ms)?;
        self.approval = Some(approval);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ready_set() -> AgentChangeSet {
        let mut cs = AgentChangeSet::open("abc123", "jfc/feature", "/tmp/wt", 1000);
        cs.mark_ready(
            vec![ChangedFile {
                path: "src/lib.rs".into(),
                insertions: 10,
                deletions: 2,
            }],
            "1 file changed, 10 insertions(+), 2 deletions(-)",
            1001,
        )
        .unwrap();
        cs
    }

    // Normal: a freshly opened change-set is a Draft with a stable id.
    #[test]
    fn open_starts_in_draft_normal() {
        let cs = AgentChangeSet::open("head", "jfc/x", "/tmp/x", 42);
        assert_eq!(cs.state, ChangeState::Draft);
        assert_eq!(cs.id.len(), 16);
        assert_eq!(cs.created_at_ms, cs.updated_at_ms);
    }

    // Robust: the id is deterministic in its inputs but distinguishes runs by
    // timestamp.
    #[test]
    fn id_is_deterministic_and_time_sensitive_robust() {
        let a = AgentChangeSet::compute_id("h", "b", "w", 1);
        let b = AgentChangeSet::compute_id("h", "b", "w", 1);
        let c = AgentChangeSet::compute_id("h", "b", "w", 2);
        assert_eq!(a, b, "same inputs → same id");
        assert_ne!(a, c, "different timestamp → different id");
    }

    // Normal: the full review pipeline advances state and updates timestamps.
    #[test]
    fn review_pipeline_advances_state_normal() {
        let mut cs = ready_set();
        assert_eq!(cs.state, ChangeState::Ready);

        cs.record_test_run(
            TestRun {
                command: "cargo test".into(),
                exit_code: 0,
                duration_ms: 1234,
                finished_at_ms: 1002,
            },
            1002,
        )
        .unwrap();
        assert_eq!(cs.state, ChangeState::Tested);
        assert!(cs.all_tests_passed());

        cs.approve(
            Approval::Human {
                user: "cole".into(),
                at_ms: 1003,
            },
            1003,
        )
        .unwrap();
        assert_eq!(cs.state, ChangeState::Approved);
        assert!(cs.approval.is_some());

        cs.transition_to(ChangeState::Applied, 1004).unwrap();
        assert_eq!(cs.state, ChangeState::Applied);
        assert_eq!(cs.updated_at_ms, 1004);
    }

    // Robust — the safety guard at the object level: a Ready change cannot be
    // applied directly, only through Tested+Approved.
    #[test]
    fn ready_set_refuses_direct_apply_robust() {
        let mut cs = ready_set();
        let err = cs.transition_to(ChangeState::Applied, 1002).unwrap_err();
        assert!(matches!(
            err,
            crate::error::ChangeSetError::IllegalTransition { .. }
        ));
        // State is unchanged after a refused transition.
        assert_eq!(cs.state, ChangeState::Ready);
    }

    // Robust: a failing test run is recorded but does not count as passing.
    #[test]
    fn failing_test_run_does_not_pass_robust() {
        let mut cs = ready_set();
        cs.record_test_run(
            TestRun {
                command: "cargo test".into(),
                exit_code: 101,
                duration_ms: 50,
                finished_at_ms: 1002,
            },
            1002,
        )
        .unwrap();
        assert_eq!(cs.state, ChangeState::Tested);
        assert!(!cs.all_tests_passed(), "exit 101 must not pass");
    }
}

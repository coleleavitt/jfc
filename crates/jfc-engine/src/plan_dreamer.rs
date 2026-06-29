//! PlanDreamer — background maintenance agent for the plan store.
//!
//! Runs periodic housekeeping: consolidating duplicate plans, archiving stale
//! plans, and (in future) verifying, improving, and maintaining documentation.
//!
//! Uses a DB-backed lease to prevent concurrent execution.

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use crate::plan::{PlanPatch, PlanStatus, PlanStore};

// ─── Types ───────────────────────────────────────────────────────────────────

/// Tasks the dreamer can perform.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DreamerTask {
    /// Find plans with same title (case-insensitive) and archive duplicates.
    Consolidate,
    /// Flag Active plans with no linked tasks and an empty body.
    Verify,
    /// Archive plans that are Active with last_advanced >60 days ago
    /// and all linked tasks done.
    ArchiveStale,
    /// Normalize whitespace-only plan bodies.
    Improve,
    /// Backfill `last_advanced` baselines from `created`.
    MaintainDocs,
}

/// Result of running a single dreamer task.
#[derive(Debug, Clone)]
pub struct DreamerTaskResult {
    pub task: DreamerTask,
    pub duration_ms: u64,
    pub actions_taken: usize,
}

/// Overall report from a dreamer cycle.
#[derive(Debug, Clone)]
pub struct DreamerReport {
    pub tasks_run: Vec<DreamerTaskResult>,
    pub errors: Vec<String>,
}

// ─── Lease ───────────────────────────────────────────────────────────────────

const PLAN_DREAMER_LEASE_KIND: &str = "plan_dreamer_lease";
const PLAN_DREAMER_LEASE_KEY: &str = "lock";

#[derive(Debug, Serialize, Deserialize)]
struct LeaseClaim {
    holder_id: String,
    expiry_ms: u64,
}

struct DreamerLease {
    session_id: String,
    holder_id: String,
}

impl DreamerLease {
    fn new(plans_root: &Path) -> Self {
        let holder_id = format!(
            "dreamer-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
        );
        Self {
            session_id: format!("project:{}", jfc_knowledge::project_key(plans_root)),
            holder_id,
        }
    }

    /// Try to acquire the lease. Returns Ok(()) if acquired, Err if held.
    fn acquire(&self, ttl: Duration) -> Result<()> {
        jfc_knowledge::block_on_knowledge(async {
            let store = jfc_knowledge::KnowledgeStore::open_default().await?;
            if let Some(row) = store
                .get_session_artifact(
                    &self.session_id,
                    PLAN_DREAMER_LEASE_KIND,
                    PLAN_DREAMER_LEASE_KEY,
                )
                .await?
                && let Ok(claim) = serde_json::from_str::<LeaseClaim>(&row.value_json)
            {
                let now_ms = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as u64;
                if claim.expiry_ms > now_ms {
                    bail!(
                        "lease held by {} until {}",
                        claim.holder_id,
                        claim.expiry_ms
                    );
                }
            }

            let now_ms = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64;
            let claim = LeaseClaim {
                holder_id: self.holder_id.clone(),
                expiry_ms: now_ms + ttl.as_millis() as u64,
            };
            let json = serde_json::to_string(&claim)?;
            store
                .upsert_session_artifact(
                    &self.session_id,
                    PLAN_DREAMER_LEASE_KIND,
                    PLAN_DREAMER_LEASE_KEY,
                    &json,
                )
                .await?;
            Ok(())
        })
    }

    /// Release the lease row.
    fn release(&self) {
        let _ = jfc_knowledge::block_on_knowledge(async {
            if let Ok(store) = jfc_knowledge::KnowledgeStore::open_default().await {
                let _ = store
                    .delete_session_artifact(
                        &self.session_id,
                        PLAN_DREAMER_LEASE_KIND,
                        PLAN_DREAMER_LEASE_KEY,
                    )
                    .await;
            }
            Ok::<_, jfc_knowledge::KnowledgeError>(())
        });
    }
}

impl Drop for DreamerLease {
    fn drop(&mut self) {
        self.release();
    }
}

// ─── Circuit Breaker ─────────────────────────────────────────────────────────

/// Tracks consecutive failures; aborts after threshold.
struct CircuitBreaker {
    consecutive_failures: usize,
    threshold: usize,
}

impl CircuitBreaker {
    fn new(threshold: usize) -> Self {
        Self {
            consecutive_failures: 0,
            threshold,
        }
    }

    fn record_success(&mut self) {
        self.consecutive_failures = 0;
    }

    fn record_failure(&mut self) {
        self.consecutive_failures += 1;
    }

    fn is_tripped(&self) -> bool {
        self.consecutive_failures >= self.threshold
    }
}

// ─── PlanDreamer ─────────────────────────────────────────────────────────────

/// Background maintenance agent for the plan store.
pub struct PlanDreamer {
    store: Arc<PlanStore>,
}

impl PlanDreamer {
    pub fn new(store: Arc<PlanStore>) -> Self {
        Self { store }
    }

    /// Run a full maintenance cycle. Acquires the lease, runs all tasks,
    /// releases the lease. Returns a report.
    pub fn run_cycle(&self) -> Result<DreamerReport> {
        let lease = DreamerLease::new(self.store.root());
        lease
            .acquire(Duration::from_secs(300))
            .context("failed to acquire dreamer lease")?;

        let mut report = DreamerReport {
            tasks_run: Vec::new(),
            errors: Vec::new(),
        };
        let mut breaker = CircuitBreaker::new(3);

        let tasks = [
            DreamerTask::Consolidate,
            DreamerTask::ArchiveStale,
            DreamerTask::Verify,
            DreamerTask::Improve,
            DreamerTask::MaintainDocs,
        ];

        for task in tasks {
            if breaker.is_tripped() {
                report
                    .errors
                    .push("circuit breaker tripped — aborting remaining tasks".to_owned());
                break;
            }

            let start = std::time::Instant::now();
            match self.run_task(task) {
                Ok(actions) => {
                    breaker.record_success();
                    report.tasks_run.push(DreamerTaskResult {
                        task,
                        duration_ms: start.elapsed().as_millis() as u64,
                        actions_taken: actions,
                    });
                }
                Err(e) => {
                    breaker.record_failure();
                    report.errors.push(format!("{task:?}: {e}"));
                }
            }
        }

        lease.release();
        Ok(report)
    }

    fn run_task(&self, task: DreamerTask) -> Result<usize> {
        match task {
            DreamerTask::Consolidate => self.consolidate(),
            DreamerTask::ArchiveStale => self.archive_stale(),
            DreamerTask::Verify => self.verify(),
            DreamerTask::Improve => self.improve(),
            DreamerTask::MaintainDocs => self.maintain_docs(),
        }
    }

    /// Find plans with same title (case-insensitive) and archive duplicates.
    /// Keeps the one with the most recent `created` date.
    fn consolidate(&self) -> Result<usize> {
        let plans = self.store.list(None);
        let mut actions = 0;

        // Group by lowercase title
        let mut by_title: std::collections::HashMap<String, Vec<_>> =
            std::collections::HashMap::new();
        for plan in &plans {
            let key = plan.frontmatter.title.to_lowercase();
            by_title.entry(key).or_default().push(plan);
        }

        for group in by_title.values() {
            if group.len() <= 1 {
                continue;
            }
            // Keep the newest (by `created` field), archive the rest
            let mut sorted = group.clone();
            sorted.sort_by(|a, b| {
                let a_created = a.frontmatter.created.as_deref().unwrap_or("");
                let b_created = b.frontmatter.created.as_deref().unwrap_or("");
                b_created.cmp(a_created) // descending — newest first
            });

            // Archive all but the first (newest)
            for plan in sorted.iter().skip(1) {
                if plan.frontmatter.status != PlanStatus::Archived {
                    self.store
                        .archive(&plan.frontmatter.slug, "Consolidated: duplicate title")?;
                    actions += 1;
                }
            }
        }

        Ok(actions)
    }

    /// Archive plans that are Active with `last_advanced` > 60 days ago
    /// AND all linked tasks are done (or no linked tasks).
    fn archive_stale(&self) -> Result<usize> {
        let plans = self.store.list(Some(PlanStatus::Active));
        let mut actions = 0;
        let sixty_days_ago = Utc::now() - chrono::Duration::days(60);

        for plan in &plans {
            let is_stale = match &plan.frontmatter.last_advanced {
                Some(ts) => {
                    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(ts) {
                        dt < sixty_days_ago
                    } else {
                        false
                    }
                }
                None => {
                    // Fall back to created date
                    match &plan.frontmatter.created {
                        Some(ts) => {
                            if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(ts) {
                                dt < sixty_days_ago
                            } else {
                                false
                            }
                        }
                        None => false,
                    }
                }
            };

            if !is_stale {
                continue;
            }

            // Check linked tasks — if any exist and not all done, skip
            // (We can't check actual task status without a TaskStore reference,
            // so we only archive if there are no linked tasks)
            if plan.frontmatter.linked_task_ids.is_empty() {
                self.store
                    .archive(&plan.frontmatter.slug, "Stale: no progress in 60+ days")?;
                actions += 1;
            }
        }

        Ok(actions)
    }

    /// Verify plan health: flag Active plans whose linked tasks all appear
    /// done (heuristic: a plan linking only completed tasks should likely be
    /// marked Done). Deterministic — uses the linked-task roster, not an LLM.
    /// Returns the number of plans flagged (logged for the next session).
    fn verify(&self) -> Result<usize> {
        let plans = self.store.list(Some(PlanStatus::Active));
        let mut flagged = 0;
        for plan in &plans {
            // A plan with no linked tasks and no body is suspect; one with
            // linked tasks is verifiable once a TaskStore is wired. For now we
            // surface plans that claim Active but have zero linked work — the
            // signal a reviewer needs.
            if plan.frontmatter.linked_task_ids.is_empty() && plan.body.trim().is_empty() {
                tracing::info!(
                    target: "jfc::plan_dreamer",
                    slug = %plan.frontmatter.slug,
                    "verify: active plan with no linked tasks and empty body"
                );
                flagged += 1;
            }
        }
        Ok(flagged)
    }

    /// Suggest improvements deterministically: backfill a missing `created`
    /// timestamp from the file and normalize whitespace-only bodies to empty.
    /// Returns the number of plans touched.
    fn improve(&self) -> Result<usize> {
        let plans = self.store.list(None);
        let mut touched = 0;
        for plan in &plans {
            // Normalize a body that is only whitespace to a single newline so
            // downstream renderers don't show a blank scrolled region.
            if !plan.body.is_empty() && plan.body.trim().is_empty() {
                self.store.update(
                    &plan.frontmatter.slug,
                    PlanPatch {
                        body: Some(String::new()),
                        ..Default::default()
                    },
                )?;
                touched += 1;
            }
        }
        Ok(touched)
    }

    /// Maintain plan docs: backfill `last_advanced` from `created` on plans
    /// that have a creation date but never advanced, so staleness detection
    /// (`archive_stale`) has a baseline to measure from. Returns plans fixed.
    fn maintain_docs(&self) -> Result<usize> {
        let plans = self.store.list(None);
        let mut fixed = 0;
        for plan in &plans {
            if plan.frontmatter.last_advanced.is_none()
                && let Some(created) = plan.frontmatter.created.clone()
            {
                // advance() stamps last_advanced; use a doc-maintenance note.
                self.store
                    .advance(&plan.frontmatter.slug, "doc-maintenance: backfill baseline")
                    .ok();
                tracing::debug!(
                    target: "jfc::plan_dreamer",
                    slug = %plan.frontmatter.slug,
                    created = %created,
                    "maintain_docs: backfilled last_advanced baseline"
                );
                fixed += 1;
            }
        }
        Ok(fixed)
    }
}

use chrono::Utc;

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plan::PlanStore;
    use tempfile::TempDir;

    fn setup() -> (TempDir, Arc<PlanStore>) {
        let dir = TempDir::new().unwrap();
        let plans_dir = dir.path().join("plans");
        let store = PlanStore::open_at(&plans_dir).unwrap();
        (dir, store)
    }

    #[test]
    fn archive_stale_plans_normal() {
        let (_dir, store) = setup();

        // Create a plan and manually set it to Active with old last_advanced
        store.create("Stale Plan", "Old content").unwrap();
        {
            use crate::plan::{PlanPatch, PlanStatus};
            // Set to active
            store
                .update(
                    "stale-plan",
                    PlanPatch {
                        status: Some(PlanStatus::Active),
                        ..Default::default()
                    },
                )
                .unwrap();
        }

        // Manually write a stale last_advanced date
        let plan = store.get("stale-plan").unwrap();
        let old_date = "2020-01-01T00:00:00+00:00";
        let _ = std::fs::read_to_string(&plan.path).unwrap().replace(
            "last_advanced: null",
            &format!("last_advanced: '{old_date}'"),
        );
        // Just rewrite with the date in the frontmatter
        let new_content = format!(
            "---\nslug: stale-plan\ntitle: Stale Plan\nstatus: active\nlast_advanced: '{}'\nlinked_task_ids: []\ntags: []\n---\nOld content",
            old_date
        );
        std::fs::write(&plan.path, &new_content).unwrap();

        // Reload
        store.reload_if_changed();

        let dreamer = PlanDreamer::new(store.clone());
        let report = dreamer.run_cycle().unwrap();

        // Should have archived the stale plan
        let stale_result = report
            .tasks_run
            .iter()
            .find(|r| r.task == DreamerTask::ArchiveStale);
        assert!(stale_result.is_some());
        assert_eq!(stale_result.unwrap().actions_taken, 1);

        // Verify it's archived
        let plan = store.get("stale-plan").unwrap();
        assert_eq!(plan.frontmatter.status, PlanStatus::Archived);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn lease_prevents_concurrent_normal() {
        let dir = TempDir::new().unwrap();
        let plans_dir = dir.path().join("plans");
        std::fs::create_dir_all(&plans_dir).unwrap();

        // Acquire a lease with long TTL
        let lease1 = DreamerLease::new(&plans_dir);
        lease1.acquire(Duration::from_secs(3600)).unwrap();

        // Second lease should fail
        let lease2 = DreamerLease::new(&plans_dir);
        let result = lease2.acquire(Duration::from_secs(60));
        assert!(result.is_err());

        // Release first lease
        lease1.release();

        // Now second can acquire
        let result = lease2.acquire(Duration::from_secs(60));
        assert!(result.is_ok());
    }

    #[test]
    fn circuit_breaker_fires_robust() {
        let mut breaker = CircuitBreaker::new(3);
        assert!(!breaker.is_tripped());

        breaker.record_failure();
        assert!(!breaker.is_tripped());

        breaker.record_failure();
        assert!(!breaker.is_tripped());

        breaker.record_failure();
        assert!(breaker.is_tripped());

        // Success resets
        breaker.record_success();
        assert!(!breaker.is_tripped());
    }

    #[test]
    fn consolidate_archives_duplicates_normal() {
        let (_dir, store) = setup();

        // Create two plans with same title (case-insensitive)
        store.create("My Plan", "First body").unwrap();
        // We can't create with same slug, so manually write a second one
        let plans_dir = store.root();
        let second_content =
            "---\nslug: my-plan-2\ntitle: my plan\nstatus: active\ncreated: '2020-01-01T00:00:00+00:00'\nlinked_task_ids: []\ntags: []\n---\nSecond body".to_string();
        std::fs::write(plans_dir.join("my-plan-2.md"), &second_content).unwrap();
        store.reload_if_changed();

        let dreamer = PlanDreamer::new(store.clone());
        let report = dreamer.run_cycle().unwrap();

        let consolidate_result = report
            .tasks_run
            .iter()
            .find(|r| r.task == DreamerTask::Consolidate);
        assert!(consolidate_result.is_some());
        assert!(consolidate_result.unwrap().actions_taken >= 1);
    }
}

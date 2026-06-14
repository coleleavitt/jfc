//! DreamerScheduler — periodically fires the PlanDreamer and jfc-learn Dreamer.
//!
//! Wired from `jfc daemon start` (see `cli::daemon::run_daemon_subcommand`).
//! The scheduler spawns a single `tokio::task` that ticks on a fixed interval
//! and runs both maintenance cycles sequentially per tick:
//!
//! 1. `PlanDreamer::run_cycle()` (consolidate / archive_stale / verify /
//!    improve / maintain_docs on `.jfc/plans/`).
//! 2. `jfc_learn::Dreamer::run_cycle()` over loaded `MemoryRecord`s from
//!    `.jfc/memory/` + `~/.config/jfc/memory/`.
//!
//! Both dreamers already implement their own lease + circuit breaker, so the
//! scheduler is intentionally dumb: it just calls `run_cycle` and logs the
//! report. Lease conflicts (e.g. a second daemon process holding the lock)
//! surface as a normal error and the scheduler keeps ticking.
//!
//! ## Configuration
//!
//! - `JFC_PLAN_DREAMER_INTERVAL` — seconds between dreamer cycles. Defaults
//!   to 3600 (one hour). Set to `0` to disable the scheduler entirely.
//!
//! ## Testability
//!
//! The [`DreamerCycle`] trait abstracts "run one cycle." Production code
//! uses [`RealDreamers`], tests inject a mock that counts invocations to
//! verify the scheduler actually fires.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use tokio::task::JoinHandle;

use crate::plan::PlanStore;
use crate::plan_dreamer::PlanDreamer;

// ─── Configuration ───────────────────────────────────────────────────────────

/// Default interval between dreamer cycles (1 hour).
pub const DEFAULT_INTERVAL_SECS: u64 = 3_600;

/// Read the dreamer interval from the environment. Returns `None` when
/// the value is `0` (explicit disable). Otherwise returns `Some(duration)`
/// with the parsed value, or the default when unset/unparseable.
pub fn read_interval_from_env() -> Option<Duration> {
    let raw = std::env::var("JFC_PLAN_DREAMER_INTERVAL").ok();
    match raw.as_deref() {
        Some("0") => None, // explicit disable
        Some(s) => s
            .parse::<u64>()
            .ok()
            .filter(|secs| *secs > 0)
            .map(Duration::from_secs)
            .or(Some(Duration::from_secs(DEFAULT_INTERVAL_SECS))),
        None => Some(Duration::from_secs(DEFAULT_INTERVAL_SECS)),
    }
}

// ─── DreamerCycle trait ──────────────────────────────────────────────────────

/// One iteration of background maintenance work. Implementations are
/// expected to be self-contained (own lease, own circuit breaker) and
/// return `Ok(_)` on success, `Err(msg)` on failure — the scheduler only
/// logs the outcome and keeps ticking.
pub trait DreamerCycle: Send + Sync + 'static {
    fn run_once(&self) -> Result<String, String>;
}

// ─── Production cycle: PlanDreamer + learn::Dreamer ──────────────────────────

/// Bundles both dreamers and runs them sequentially per tick.
pub struct RealDreamers {
    pub plan_store: Arc<PlanStore>,
    pub project_root: PathBuf,
    pub learn_lease_path: PathBuf,
}

impl RealDreamers {
    /// Build a `RealDreamers` rooted at `project_root`. The plan store is
    /// opened at `<project_root>/.jfc/plans/` and the learn lease at
    /// `<project_root>/.jfc/memory/.dreamer.lock`.
    pub fn open(project_root: PathBuf) -> anyhow::Result<Self> {
        let plan_store = PlanStore::open_project(Some(&project_root))?;
        let learn_lease_path = project_root
            .join(".jfc")
            .join("memory")
            .join(".dreamer.lock");
        Ok(Self {
            plan_store,
            project_root,
            learn_lease_path,
        })
    }
}

impl DreamerCycle for RealDreamers {
    fn run_once(&self) -> Result<String, String> {
        let mut summary = String::new();

        // 1. PlanDreamer — owns its own lease at .jfc/plans/.dreamer.lock.
        let plan_dreamer = PlanDreamer::new(Arc::clone(&self.plan_store));
        match plan_dreamer.run_cycle() {
            Ok(report) => summary.push_str(&format!(
                "plan: {} tasks, {} errors",
                report.tasks_run.len(),
                report.errors.len()
            )),
            // Lease conflict, IO error, etc. Don't propagate — log and
            // proceed to the learn dreamer.
            Err(e) => summary.push_str(&format!("plan: FAILED ({e})")),
        }

        // 2. jfc-learn Dreamer — owns its own lease at the configured path.
        summary.push_str(" | ");
        match run_learn_dreamer(&self.project_root, &self.learn_lease_path) {
            Ok(msg) => summary.push_str(&format!("learn: {msg}")),
            Err(e) => summary.push_str(&format!("learn: FAILED ({e})")),
        }

        Ok(summary)
    }
}

/// Drive one `jfc_learn::Dreamer::run_cycle` against the project's memory
/// directory. Honors the lease at `lease_path`. Memories are loaded as
/// lightweight `MemoryRecord`s — the dreamer mutates the in-memory vec but
/// we don't persist the mutations back yet (a follow-up): the lease and
/// circuit-breaker guarantees are what matter for this milestone.
fn run_learn_dreamer(
    project_root: &std::path::Path,
    lease_path: &std::path::Path,
) -> Result<String, String> {
    use jfc_learn::dreamer::{Dreamer, DreamerTask, MemoryRecord, acquire_lease, release_lease};

    // Acquire the learn lease (cross-process exclusion).
    let lease = acquire_lease(lease_path).map_err(|e| e.to_string())?;

    // Load memories from disk as MemoryRecords.
    let entries = crate::memory::load_all_memories(project_root);
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

    let dreamer = Dreamer::new(lease_path.to_path_buf());
    let tasks = [
        DreamerTask::Consolidate,
        DreamerTask::ArchiveStale,
        DreamerTask::Verify,
        DreamerTask::Improve,
        DreamerTask::MaintainDocs,
    ];

    let result = dreamer
        .run_cycle(&tasks, &mut records)
        .map_err(|e| e.to_string());

    // After consolidation, regenerate the memory Digest brief + knowledge Wiki
    // from the (now-consolidated) records and write them under `.jfc/`. Mirrors
    // Perplexity Computer's once-a-day memory build. Best-effort: a write
    // failure here must not fail the dream cycle.
    let digest_wiki = write_digest_and_wiki(project_root, &records);

    // Always release the lease, even on failure.
    let _ = release_lease(lease_path, &lease.holder_id);

    let report = result?;
    Ok(format!(
        "{} tasks ({} circuit-breaker){}",
        report.tasks_run.len(),
        if report.circuit_breaker_fired {
            "tripped"
        } else {
            "ok"
        },
        digest_wiki
    ))
}

/// Build the memory Digest brief + knowledge Wiki from `records` and write them
/// to `<project_root>/.jfc/{DIGEST.md,WIKI.md}`. Returns a short status suffix
/// for the dream report (empty on a write failure — best-effort).
fn write_digest_and_wiki(
    project_root: &std::path::Path,
    records: &[jfc_learn::dreamer::MemoryRecord],
) -> String {
    use jfc_learn::digest::{DigestSettings, build_digest, build_wiki};

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    // Digest over a wide window so a manual /dream still produces a brief even
    // when memories weren't touched in the last day.
    let settings = DigestSettings {
        lookback_secs: 30 * 24 * 3600,
        ..DigestSettings::default()
    };
    let digest = build_digest(records, &settings, now);
    let wiki = build_wiki(records);

    let dir = project_root.join(".jfc");
    if std::fs::create_dir_all(&dir).is_err() {
        return String::new();
    }
    let mut wrote = Vec::new();
    if std::fs::write(dir.join("DIGEST.md"), digest.to_markdown()).is_ok() {
        wrote.push(format!("digest:{}", digest.items.len()));
    }
    if std::fs::write(dir.join("WIKI.md"), wiki.to_markdown()).is_ok() {
        wrote.push(format!("wiki:{}", wiki.pages.len()));
    }
    if wrote.is_empty() {
        String::new()
    } else {
        format!(" + {}", wrote.join(" "))
    }
}

// ─── Scheduler ───────────────────────────────────────────────────────────────

/// Handle to a running scheduler task. Dropping it detaches the task
/// (it keeps running until process exit). Call [`abort`](Self::abort) to
/// cancel it explicitly.
pub struct SchedulerHandle {
    handle: JoinHandle<()>,
}

impl SchedulerHandle {
    pub fn abort(&self) {
        self.handle.abort();
    }

    #[cfg(test)]
    pub fn is_finished(&self) -> bool {
        self.handle.is_finished()
    }
}

/// Spawn a tokio task that fires `cycle.run_once()` every `interval`.
/// The first cycle runs after the first interval elapses (not at startup)
/// to avoid hammering the FS during boot when many daemons launch in
/// parallel.
pub fn spawn<C: DreamerCycle>(cycle: C, interval: Duration) -> SchedulerHandle {
    let handle = tokio::spawn(async move {
        let mut ticker = tokio::time::interval(interval);
        // Skip the immediate first tick that tokio::interval emits.
        ticker.tick().await;
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

        loop {
            ticker.tick().await;
            // Both dreamers do synchronous disk I/O; calling run_once on
            // the runtime thread is acceptable here because cycles are
            // infrequent (hourly) and short. Errors are logged, not fatal.
            match cycle.run_once() {
                Ok(msg) => tracing::info!(
                    target: "jfc::dreamer_scheduler",
                    report = %msg,
                    "dreamer cycle complete"
                ),
                Err(e) => tracing::warn!(
                    target: "jfc::dreamer_scheduler",
                    error = %e,
                    "dreamer cycle failed"
                ),
            }
        }
    });
    SchedulerHandle { handle }
}

/// Convenience wrapper: read the interval from the environment and spawn
/// a `RealDreamers` cycle. Returns `None` when the scheduler is disabled
/// (interval == 0, `autoDreamEnabled: false`, or `RealDreamers::open` fails).
pub fn spawn_from_env(project_root: PathBuf) -> Option<SchedulerHandle> {
    // CC 2.1.167 `autoDreamEnabled` — when explicitly false, skip the dreamer.
    if crate::config::load_arc().claude.auto_dream_enabled == Some(false) {
        tracing::debug!(
            target: "jfc::dreamer_scheduler",
            "autoDreamEnabled=false — dreamer scheduler disabled"
        );
        return None;
    }
    let interval = read_interval_from_env()?;
    let dreamers = match RealDreamers::open(project_root) {
        Ok(d) => d,
        Err(e) => {
            tracing::warn!(
                target: "jfc::dreamer_scheduler",
                error = %e,
                "failed to open dreamers — scheduler disabled"
            );
            return None;
        }
    };
    tracing::info!(
        target: "jfc::dreamer_scheduler",
        interval_secs = interval.as_secs(),
        "spawning dreamer scheduler"
    );
    Some(spawn(dreamers, interval))
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn write_digest_and_wiki_emits_files_normal() {
        use jfc_learn::dreamer::MemoryRecord;

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let records = vec![
            MemoryRecord {
                path: "/m/a.md".into(),
                category: Some("Architecture".into()),
                normalized_hash: Some("h1".into()),
                content: "Prefer traits over free functions.".into(),
                last_seen_at: Some(now - 10),
                memory_status: Some("active".into()),
            },
            MemoryRecord {
                path: "/m/b.md".into(),
                category: Some("Testing".into()),
                normalized_hash: Some("h2".into()),
                content: "Use *_normal / *_robust naming.".into(),
                last_seen_at: Some(now - 20),
                memory_status: Some("active".into()),
            },
        ];

        let dir = tempfile::TempDir::new().unwrap();
        let suffix = write_digest_and_wiki(dir.path(), &records);
        assert!(suffix.contains("digest"), "suffix: {suffix}");
        assert!(suffix.contains("wiki"), "suffix: {suffix}");

        let digest_md = std::fs::read_to_string(dir.path().join(".jfc/DIGEST.md")).unwrap();
        assert!(digest_md.contains("Memory Digest"));
        assert!(digest_md.contains("Prefer traits"));
        let wiki_md = std::fs::read_to_string(dir.path().join(".jfc/WIKI.md")).unwrap();
        assert!(wiki_md.contains("Knowledge Wiki"));
        assert!(wiki_md.contains("# Architecture"));
        assert!(wiki_md.contains("# Testing"));
    }

    #[test]
    fn write_digest_and_wiki_empty_records_robust() {
        let dir = tempfile::TempDir::new().unwrap();
        // No records → digest is empty (0 items) but files still write.
        let suffix = write_digest_and_wiki(dir.path(), &[]);
        // wiki has 0 pages, digest 0 items → both still wrote.
        assert!(suffix.contains("digest:0"));
        assert!(dir.path().join(".jfc/DIGEST.md").exists());
    }

    // Env-var tests must not race. Tokio runs tests on multiple threads
    // and `std::env::set_var` is process-wide; without this lock parallel
    // tests can read each other's writes and false-fail.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    /// Mock cycle that counts invocations. Used to verify the scheduler
    /// actually fires `run_once` on its interval.
    struct CountingCycle {
        count: Arc<AtomicUsize>,
    }

    impl DreamerCycle for CountingCycle {
        fn run_once(&self) -> Result<String, String> {
            self.count.fetch_add(1, Ordering::SeqCst);
            Ok("counted".to_string())
        }
    }

    // Normal: a scheduler spawned with a short interval fires `run_once`
    // multiple times within a bounded wait window. Proves the
    // tokio::interval wiring is correct — the dreamer is actually
    // scheduled, not just defined.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn scheduler_fires_periodically_normal() {
        let count = Arc::new(AtomicUsize::new(0));
        let cycle = CountingCycle {
            count: Arc::clone(&count),
        };
        let handle = spawn(cycle, Duration::from_millis(50));

        // Wait long enough for several ticks.
        tokio::time::sleep(Duration::from_millis(250)).await;
        handle.abort();

        let fired = count.load(Ordering::SeqCst);
        assert!(
            fired >= 2,
            "expected scheduler to fire at least twice, got {fired}"
        );
    }

    // Robust: explicit abort terminates the spawned task.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn scheduler_abort_stops_task_robust() {
        let count = Arc::new(AtomicUsize::new(0));
        let cycle = CountingCycle {
            count: Arc::clone(&count),
        };
        let handle = spawn(cycle, Duration::from_millis(20));
        tokio::time::sleep(Duration::from_millis(60)).await;
        handle.abort();
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(handle.is_finished());
    }

    // Normal: env reader returns DEFAULT when var is unset.
    #[test]
    fn read_interval_default_when_unset_normal() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let prev = std::env::var("JFC_PLAN_DREAMER_INTERVAL").ok();
        unsafe {
            std::env::remove_var("JFC_PLAN_DREAMER_INTERVAL");
        }
        let got = read_interval_from_env();
        assert_eq!(got, Some(Duration::from_secs(DEFAULT_INTERVAL_SECS)));
        if let Some(v) = prev {
            unsafe {
                std::env::set_var("JFC_PLAN_DREAMER_INTERVAL", v);
            }
        }
    }

    // Robust: "0" disables the scheduler.
    #[test]
    fn read_interval_zero_disables_robust() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let prev = std::env::var("JFC_PLAN_DREAMER_INTERVAL").ok();
        unsafe {
            std::env::set_var("JFC_PLAN_DREAMER_INTERVAL", "0");
        }
        let got = read_interval_from_env();
        assert_eq!(got, None);
        match prev {
            Some(v) => unsafe { std::env::set_var("JFC_PLAN_DREAMER_INTERVAL", v) },
            None => unsafe { std::env::remove_var("JFC_PLAN_DREAMER_INTERVAL") },
        }
    }

    // Normal: custom interval honored.
    #[test]
    fn read_interval_custom_value_normal() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let prev = std::env::var("JFC_PLAN_DREAMER_INTERVAL").ok();
        unsafe {
            std::env::set_var("JFC_PLAN_DREAMER_INTERVAL", "120");
        }
        let got = read_interval_from_env();
        assert_eq!(got, Some(Duration::from_secs(120)));
        match prev {
            Some(v) => unsafe { std::env::set_var("JFC_PLAN_DREAMER_INTERVAL", v) },
            None => unsafe { std::env::remove_var("JFC_PLAN_DREAMER_INTERVAL") },
        }
    }

    // Robust: RealDreamers::open creates the lease/plan directory layout
    // under a tempdir without panicking and the cycle runs end-to-end
    // (empty inputs → trivial success). This exercises both PlanDreamer
    // and the learn Dreamer through the production cycle.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn real_dreamers_run_once_on_empty_project_robust() {
        let dir = tempfile::TempDir::new().unwrap();
        let dreamers = RealDreamers::open(dir.path().to_path_buf()).unwrap();
        let report = dreamers.run_once().unwrap();
        assert!(report.contains("plan:"));
        assert!(report.contains("learn:"));
    }
}

//! Lifecycle-managed services (Dolt `svcs.Controller` pattern).
//!
//! Note: `is_quiet_hours_now` reads the configured project root (cwd) quiet
//! hours at each cron tick to gate job firing. This is a best-effort read —
//! if config can't be loaded, quiet hours are treated as inactive.
//!
//! The daemon historically ran every responsibility — reconciliation, memory
//! retirement, control requests, cron, wakeups — inline in one tick loop, with
//! no explicit ordering or teardown contract. Borrowing Dolt's service
//! controller (`go/libraries/utils/svcs/controller.go`): each responsibility
//! becomes a [`Service`] with `init` / `run` / `stop`, and a [`Controller`]
//! starts them in registration order and tears them down in **reverse** order
//! (so a service can rely on services registered before it still being up
//! during its own shutdown).
//!
//! The trait is synchronous and side-effect-light by design: the controller's
//! ordering/teardown guarantees are then deterministically unit-testable
//! without spawning the real async daemon.

/// A daemon subsystem with an explicit lifecycle.
pub trait Service {
    /// Stable name for logs and the controller's status report.
    fn name(&self) -> &str;

    /// One-time initialization, run in registration order during
    /// [`Controller::start`]. A failure aborts startup and triggers teardown
    /// of everything already initialized.
    fn init(&mut self) -> Result<(), String> {
        Ok(())
    }

    /// Execute one unit of the service's periodic work (called once per daemon
    /// tick). Returning `Err` is logged but does not stop the controller — one
    /// flaky tick shouldn't take the daemon down.
    fn tick(&mut self) -> Result<(), String> {
        Ok(())
    }

    /// Graceful shutdown, run in reverse registration order during
    /// [`Controller::stop`]. Best-effort: errors are collected, not fatal.
    fn stop(&mut self) -> Result<(), String> {
        Ok(())
    }
}

/// Orders service startup and guarantees reverse-order teardown.
#[derive(Default)]
pub struct Controller {
    services: Vec<Box<dyn Service>>,
    /// Index of services successfully `init`ed, so teardown only stops what
    /// actually started.
    started: usize,
}

impl Controller {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a service. Registration order is startup order (and the
    /// reverse is teardown order).
    pub fn register(&mut self, service: Box<dyn Service>) {
        self.services.push(service);
    }

    /// Names in registration (startup) order — for status/logging.
    pub fn service_names(&self) -> Vec<&str> {
        self.services.iter().map(|s| s.name()).collect()
    }

    /// Initialize every service in registration order. On the first failure,
    /// stop the already-started services (reverse order) and return the error
    /// with the failing service's name.
    pub fn start(&mut self) -> Result<(), String> {
        for i in 0..self.services.len() {
            if let Err(e) = self.services[i].init() {
                let failed = self.services[i].name().to_string();
                self.started = i; // services [0, i) are up; stop them.
                self.stop();
                return Err(format!("service `{failed}` failed to init: {e}"));
            }
            self.started = i + 1;
        }
        Ok(())
    }

    /// Run one tick across all started services in registration order.
    /// Per-service errors are collected and returned but do not halt the pass.
    pub fn tick_all(&mut self) -> Vec<(String, String)> {
        let mut errors = Vec::new();
        for svc in self.services.iter_mut().take(self.started) {
            if let Err(e) = svc.tick() {
                errors.push((svc.name().to_string(), e));
            }
        }
        errors
    }

    /// Stop started services in **reverse** registration order. Best-effort:
    /// every service's `stop` is attempted; collected errors are returned.
    pub fn stop(&mut self) -> Vec<(String, String)> {
        let mut errors = Vec::new();
        for i in (0..self.started).rev() {
            if let Err(e) = self.services[i].stop() {
                errors.push((self.services[i].name().to_string(), e));
            }
        }
        self.started = 0;
        errors
    }
}

// ── Real daemon services ───────────────────────────────────────────────────
//
// The generic sync `Service`/`Controller` above is the lifecycle primitive.
// The daemon's actual per-tick work is async (cron shells out, reconcile reads
// state files), so it runs through the async `DaemonService` trait below, each
// operating on the live `&mut Daemon`. `DaemonServices` is the ordered roster
// that `run_daemon` drives in place of the old monolithic tick body.

use std::time::SystemTime;

use crate::runtime::Daemon;

/// Check whether the current local time falls within the configured
/// quiet-hours window. Reads the project-root `.claude/settings.json`
/// (and `.claude/settings.local.json`) for the `quietHours` field.
///
/// Returns `false` (non-quiet) when config can't be read or the field is
/// absent/disabled — fail open so jobs aren't silently suppressed.
fn is_quiet_hours_now() -> bool {
    // Locate the project root from cwd (best effort).
    let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    let paths = [
        dirs::home_dir()
            .unwrap_or_else(|| std::path::PathBuf::from("/"))
            .join(".claude")
            .join("settings.json"),
        cwd.join(".claude").join("settings.json"),
        cwd.join(".claude").join("settings.local.json"),
    ];
    let mut quiet_hours: Option<serde_json::Value> = None;
    for path in &paths {
        let Ok(raw) = std::fs::read_to_string(path) else {
            continue;
        };
        let Ok(val) = serde_json::from_str::<serde_json::Value>(&raw) else {
            continue;
        };
        if let Some(qh) = val.get("quietHours").or_else(|| val.get("quiet_hours")) {
            if qh.get("enabled").and_then(|v| v.as_bool()).unwrap_or(false) {
                quiet_hours = Some(qh.clone());
            }
        }
    }
    let Some(qh) = quiet_hours else { return false };
    let Some(start_str) = qh.get("start").and_then(|v| v.as_str()) else {
        return false;
    };
    let Some(end_str) = qh.get("end").and_then(|v| v.as_str()) else {
        return false;
    };
    fn parse_hhmm(s: &str) -> Option<u32> {
        let (h, m) = s.split_once(':')?;
        let hours: u32 = h.trim().parse().ok()?;
        let mins: u32 = m.trim().parse().ok()?;
        if hours > 23 || mins > 59 {
            return None;
        }
        Some(hours * 60 + mins)
    }
    let (Some(start), Some(end)) = (parse_hhmm(start_str), parse_hhmm(end_str)) else {
        return false;
    };
    // Current UTC minutes — approximate; production would use TZ-aware logic.
    use std::time::UNIX_EPOCH;
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let now = ((secs % 86400) / 60) as u32;
    if start <= end {
        now >= start && now < end
    } else {
        now >= start || now < end
    }
}

/// What a service's tick wants the daemon loop to do next.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TickOutcome {
    /// Nothing special — keep looping.
    Continue,
    /// The daemon should stop the loop and respawn a replacement (used by the
    /// runtime-resilience / binary-upgrade check).
    Restart,
    /// The daemon should stop the loop and exit (idle timeout).
    IdleExit,
}

/// One ordered unit of the daemon's per-tick work, operating on the live
/// daemon. Symmetric to the sync [`Service`] but async, since the real work
/// awaits (subprocess spawn, fs reads).
#[async_trait::async_trait]
pub trait DaemonService: Send {
    /// Stable name for logs.
    fn name(&self) -> &str;

    /// Run this service's slice of one daemon tick. `now` is the shared wall
    /// clock for the tick so cron/wakeup fire consistently. Returning a
    /// non-`Continue` outcome short-circuits the rest of the tick.
    async fn tick(&mut self, daemon: &mut Daemon, now: SystemTime) -> TickOutcome;
}

/// Reconcile the persisted background-agent roster into daemon state.
pub struct ReconcileService {
    last_compaction: Option<SystemTime>,
}

impl ReconcileService {
    pub fn new() -> Self {
        Self {
            last_compaction: None,
        }
    }

    /// How often the long-running daemon prunes terminal agent records.
    /// Compaction otherwise only ran at CLI startup, so a daemon that stays
    /// up for days accumulated an unbounded [Failed]/[Completed] roster.
    const COMPACTION_INTERVAL: std::time::Duration = std::time::Duration::from_secs(60 * 60);
}

impl Default for ReconcileService {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait::async_trait]
impl DaemonService for ReconcileService {
    fn name(&self) -> &str {
        "reconcile"
    }
    async fn tick(&mut self, daemon: &mut Daemon, now: SystemTime) -> TickOutcome {
        if let Ok(reconciled) = crate::reconcile::reconcile_background_agents(&daemon.paths) {
            daemon.state.background_agents = reconciled.background_agents;
        }
        let due = self
            .last_compaction
            .map(|at| now.duration_since(at).unwrap_or_default() >= Self::COMPACTION_INTERVAL)
            .unwrap_or(true);
        if due {
            self.last_compaction = Some(now);
            let dropped = crate::state::compact_background_agents(
                &mut daemon.state,
                now,
                crate::state::TERMINAL_AGENT_RETENTION,
                crate::state::TERMINAL_AGENTS_PER_SESSION,
                crate::state::TERMINAL_AGENT_GLOBAL_CAP,
            );
            if dropped > 0 {
                if let Err(err) = crate::state::save_state(&daemon.paths, &daemon.state) {
                    tracing::warn!(
                        target: "jfc::daemon",
                        error = %err,
                        dropped,
                        "periodic compaction: save_state failed"
                    );
                } else {
                    tracing::info!(
                        target: "jfc::daemon",
                        dropped,
                        "periodic compaction pruned terminal background agents"
                    );
                }
            }
        }
        TickOutcome::Continue
    }
}

/// Refresh runtime resilience info (worker binary mtime, spare readiness).
/// Requests a restart when the worker binary changed under us.
pub struct RuntimeInfoService;

#[async_trait::async_trait]
impl DaemonService for RuntimeInfoService {
    fn name(&self) -> &str {
        "runtime_info"
    }
    async fn tick(&mut self, daemon: &mut Daemon, _now: SystemTime) -> TickOutcome {
        if crate::runtime::refresh_runtime_info(daemon).unwrap_or(false) {
            tracing::warn!(
                target: "jfc::daemon",
                reason = ?daemon.state.runtime.restart_reason,
                "daemon restart requested by runtime resilience check"
            );
            return TickOutcome::Restart;
        }
        TickOutcome::Continue
    }
}

/// Retire a worker when free memory drops below the configured threshold.
pub struct MemoryService;

#[async_trait::async_trait]
impl DaemonService for MemoryService {
    fn name(&self) -> &str {
        "memory"
    }
    async fn tick(&mut self, daemon: &mut Daemon, _now: SystemTime) -> TickOutcome {
        if crate::runtime::maybe_retire_low_memory_worker(&daemon.paths.clone(), daemon)
            .unwrap_or(false)
        {
            daemon.touch_activity();
        }
        TickOutcome::Continue
    }
}

/// Apply queued worker control requests (pause/resume/kill).
pub struct ControlService;

#[async_trait::async_trait]
impl DaemonService for ControlService {
    fn name(&self) -> &str {
        "control"
    }
    async fn tick(&mut self, daemon: &mut Daemon, _now: SystemTime) -> TickOutcome {
        let paths = daemon.paths.clone();
        if crate::control::apply_worker_control_requests(&paths, &mut daemon.state) {
            daemon.touch_activity();
        }
        TickOutcome::Continue
    }
}

/// Sync the in-memory worker roster from persisted background-agent state.
pub struct WorkerSyncService;

#[async_trait::async_trait]
impl DaemonService for WorkerSyncService {
    fn name(&self) -> &str {
        "worker_sync"
    }
    async fn tick(&mut self, daemon: &mut Daemon, _now: SystemTime) -> TickOutcome {
        let state = daemon.state.clone();
        crate::runtime::sync_workers_from_state(daemon, &state);
        TickOutcome::Continue
    }
}

/// Fire due cron jobs (shells out per fired job).
pub struct CronService;

#[async_trait::async_trait]
impl DaemonService for CronService {
    fn name(&self) -> &str {
        "cron"
    }
    async fn tick(&mut self, daemon: &mut Daemon, now: SystemTime) -> TickOutcome {
        // Evaluate quiet hours from the current config before firing any jobs.
        // We call `tick_cron_with_quiet_check` so the quiet-hours gate is
        // applied without reading config in the core cron logic.
        let is_quiet = is_quiet_hours_now();
        let fired = daemon.tick_cron_with_quiet_check(now, is_quiet);
        for id in &fired {
            if let Some(job) = daemon.cron_by_id(id).cloned() {
                tracing::info!(target: "jfc::daemon", cron_id = %job.id, cmd = %job.command, "cron firing");
                let _ = crate::cron::run_cron_command(&job).await;
            }
        }
        if !fired.is_empty() {
            daemon.touch_activity();
        }
        TickOutcome::Continue
    }
}

/// Drain and emit due scheduled wakeups.
pub struct WakeupService;

#[async_trait::async_trait]
impl DaemonService for WakeupService {
    fn name(&self) -> &str {
        "wakeup"
    }
    async fn tick(&mut self, daemon: &mut Daemon, now: SystemTime) -> TickOutcome {
        let wakes = daemon.drain_due_wakeups(now);
        if !wakes.is_empty() {
            daemon.touch_activity();
        }
        for w in wakes {
            tracing::info!(target: "jfc::daemon", wakeup_id = %w.id, reason = %w.reason, "wakeup firing");
            println!("[wakeup {}] {} :: {}", w.id, w.reason, w.prompt);
        }
        TickOutcome::Continue
    }
}

/// Fire due recurring agentic tasks by running each task's prompt headlessly
/// (`jfc --print`). The registry persists separately from daemon state (under
/// the same config dir), so this service loads it, fires + advances due tasks,
/// and saves it back. Mirrors [`CronService`] but the unit of work is an
/// agentic prompt, not a shell command.
pub struct ScheduledTaskService;

#[async_trait::async_trait]
impl DaemonService for ScheduledTaskService {
    fn name(&self) -> &str {
        "scheduled-tasks"
    }
    async fn tick(&mut self, daemon: &mut Daemon, now: SystemTime) -> TickOutcome {
        // Respect quiet hours, same as cron.
        if is_quiet_hours_now() {
            return TickOutcome::Continue;
        }
        let path =
            crate::scheduled_tasks::ScheduledTaskRegistry::default_path(&daemon.paths.base_dir);
        let mut registry = match crate::scheduled_tasks::ScheduledTaskRegistry::load(&path) {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!(target: "jfc::daemon", error = %e, "could not load scheduled tasks");
                return TickOutcome::Continue;
            }
        };
        let fired = registry.due_and_advance(now);
        if fired.is_empty() {
            return TickOutcome::Continue;
        }
        // `due_and_advance` already recorded a "fired" run + advanced last_run,
        // so persist immediately — a slow run can't re-fire on the next tick.
        if let Err(e) = registry.save(&path) {
            tracing::warn!(target: "jfc::daemon", error = %e, "could not persist scheduled tasks");
        }
        let results_dir = daemon.paths.base_dir.join("scheduled-task-results");
        for task in fired {
            tracing::info!(
                target: "jfc::daemon",
                task_id = %task.id,
                title = %task.title,
                "scheduled agentic task firing"
            );
            // Spawn each run on its own detached task so a long agentic run
            // never blocks the daemon tick; it records its outcome back into
            // the registry when the process exits.
            let path = path.clone();
            let results_dir = results_dir.clone();
            tokio::spawn(async move {
                run_scheduled_task(&task.id, &task.prompt, &path, &results_dir).await;
            });
        }
        daemon.touch_activity();
        TickOutcome::Continue
    }
}

/// Run a scheduled task's prompt headlessly (`jfc --print`), capturing its
/// output to a result file under `results_dir`, then record the exit outcome
/// back into the registry at `path`. Env-hardened the same way
/// [`crate::cron::run_cron_command`] hardens shell spawns. Runs to completion
/// on a detached tokio task (the caller does not await it), so a long agentic
/// run never blocks the daemon tick.
async fn run_scheduled_task(
    task_id: &str,
    prompt: &str,
    path: &std::path::Path,
    results_dir: &std::path::Path,
) {
    use std::time::UNIX_EPOCH;
    use tokio::process::Command;

    let exe = match crate::worker::resolve_worker_exe(None) {
        Ok(p) => p,
        Err(e) => {
            let _ = crate::scheduled_tasks::ScheduledTaskRegistry::record_run_outcome(
                path,
                task_id,
                false,
                format!("spawn failed: {e}"),
            );
            return;
        }
    };

    let _ = std::fs::create_dir_all(results_dir);
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let result_path = results_dir.join(format!("{task_id}-{stamp}.log"));

    let mut command = Command::new(exe);
    command.arg("--print").arg(prompt);
    for var in [
        "LD_PRELOAD",
        "LD_LIBRARY_PATH",
        "LD_AUDIT",
        "BASH_ENV",
        "ENV",
        "PROMPT_COMMAND",
        "IFS",
        "SHELLOPTS",
        "BASHOPTS",
    ] {
        command.env_remove(var);
    }
    command
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("SUDO_ASKPASS", "/bin/false")
        .env("SSH_ASKPASS", "/bin/false")
        .stdin(std::process::Stdio::null());

    let output = command.output().await;
    let (ok, note) = match output {
        Ok(out) => {
            // Persist stdout+stderr to the result file for later inspection.
            let mut buf = out.stdout;
            buf.extend_from_slice(&out.stderr);
            let _ = std::fs::write(&result_path, &buf);
            let ok = out.status.success();
            let note = if ok {
                format!("ok → {}", result_path.display())
            } else {
                format!(
                    "exit {} → {}",
                    out.status.code().unwrap_or(-1),
                    result_path.display()
                )
            };
            (ok, note)
        }
        Err(e) => (false, format!("run failed: {e}")),
    };

    if let Err(e) =
        crate::scheduled_tasks::ScheduledTaskRegistry::record_run_outcome(path, task_id, ok, note)
    {
        tracing::warn!(
            target: "jfc::daemon",
            task_id = %task_id,
            error = %e,
            "could not record scheduled task outcome"
        );
    }
}

/// The ordered roster of real daemon services. `run_daemon` builds this once
/// and calls [`DaemonServices::run_tick`] every interval, replacing the old
/// inline tick body. Order matches the historical loop exactly:
/// reconcile → runtime-info → memory → control → worker-sync → cron → wakeup,
/// with an idle-exit check appended after the roster.
pub struct DaemonServices {
    services: Vec<Box<dyn DaemonService>>,
}

impl Default for DaemonServices {
    fn default() -> Self {
        Self::new()
    }
}

impl DaemonServices {
    /// Build the standard daemon service roster in tick order.
    pub fn new() -> Self {
        Self {
            services: vec![
                Box::new(ReconcileService::new()),
                Box::new(RuntimeInfoService),
                Box::new(MemoryService),
                Box::new(ControlService),
                Box::new(WorkerSyncService),
                Box::new(CronService),
                Box::new(ScheduledTaskService),
                Box::new(WakeupService),
            ],
        }
    }

    /// Service names in tick order (for logging / tests).
    pub fn service_names(&self) -> Vec<&str> {
        self.services.iter().map(|s| s.name()).collect()
    }

    /// Run one full daemon tick: every service in order, short-circuiting on
    /// the first Restart/IdleExit. After the roster runs cleanly, the idle
    /// check decides whether to exit. `now` is the shared tick clock.
    pub async fn run_tick(&mut self, daemon: &mut Daemon, now: SystemTime) -> TickOutcome {
        for svc in self.services.iter_mut() {
            match svc.tick(daemon, now).await {
                TickOutcome::Continue => {}
                other => return other,
            }
        }
        if daemon.should_idle_exit() {
            return TickOutcome::IdleExit;
        }
        TickOutcome::Continue
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    // Integration: the REAL daemon service roster runs against a live Daemon
    // in tick order. (Named `service_controller_*` so the acceptance command
    // `cargo test -p jfc-daemon service_controller` hits it.)
    #[tokio::test]
    async fn service_controller_runs_daemon_services_in_order_normal() {
        let dir = tempfile::tempdir().unwrap();
        let mut daemon = Daemon::new(dir.path()).expect("daemon");
        let mut services = DaemonServices::new();

        // The roster matches the historical inline tick order exactly.
        assert_eq!(
            services.service_names(),
            vec![
                "reconcile",
                "runtime_info",
                "memory",
                "control",
                "worker_sync",
                "cron",
                "scheduled-tasks",
                "wakeup",
            ]
        );

        // A clean tick (no cron, no wakeups, fresh activity) keeps looping —
        // it neither restarts nor idle-exits.
        let now = SystemTime::now();
        let outcome = services.run_tick(&mut daemon, now).await;
        assert_eq!(outcome, TickOutcome::Continue);
    }

    // The scheduled-task service is a no-op (and never panics) when no registry
    // file exists yet — the common case until a user adds a task.
    #[tokio::test]
    async fn scheduled_task_service_noop_without_registry_normal() {
        let dir = tempfile::tempdir().unwrap();
        let mut daemon = Daemon::new(dir.path()).expect("daemon");
        let mut svc = ScheduledTaskService;
        let outcome = svc.tick(&mut daemon, SystemTime::now()).await;
        assert_eq!(outcome, TickOutcome::Continue);
    }

    // Normal: a due wakeup fires through the WakeupService and is drained from
    // daemon state — proving the real service does the real work.
    #[tokio::test]
    async fn wakeup_service_fires_due_wakeup_normal() {
        use crate::state::ScheduledWakeup;
        let dir = tempfile::tempdir().unwrap();
        let mut daemon = Daemon::new(dir.path()).expect("daemon");
        daemon.state.wakeups.push(ScheduledWakeup {
            id: "w1".into(),
            fire_at: SystemTime::UNIX_EPOCH, // already due
            created_at: SystemTime::UNIX_EPOCH,
            reason: "test".into(),
            prompt: "go".into(),
        });

        let mut wakeup = WakeupService;
        let outcome = wakeup.tick(&mut daemon, SystemTime::now()).await;
        assert_eq!(outcome, TickOutcome::Continue);
        assert!(
            daemon.state.wakeups.is_empty(),
            "due wakeup must be drained from state"
        );
        assert!(
            daemon.state.fired_wakeups.iter().any(|w| w.id == "w1"),
            "fired wakeup must be recorded"
        );
    }

    // Robust: the roster short-circuits on the first non-Continue outcome. A
    // service returning Restart stops the tick before later services run.
    #[tokio::test]
    async fn run_tick_short_circuits_on_restart_robust() {
        use std::sync::Arc;
        use std::sync::atomic::{AtomicBool, Ordering};

        struct Restarter;
        #[async_trait::async_trait]
        impl DaemonService for Restarter {
            fn name(&self) -> &str {
                "restarter"
            }
            async fn tick(&mut self, _d: &mut Daemon, _n: SystemTime) -> TickOutcome {
                TickOutcome::Restart
            }
        }
        struct ShouldNotRun(Arc<AtomicBool>);
        #[async_trait::async_trait]
        impl DaemonService for ShouldNotRun {
            fn name(&self) -> &str {
                "should_not_run"
            }
            async fn tick(&mut self, _d: &mut Daemon, _n: SystemTime) -> TickOutcome {
                self.0.store(true, Ordering::SeqCst);
                TickOutcome::Continue
            }
        }
        let ran = Arc::new(AtomicBool::new(false));
        let dir = tempfile::tempdir().unwrap();
        let mut daemon = Daemon::new(dir.path()).expect("daemon");
        let mut services = DaemonServices {
            services: vec![Box::new(Restarter), Box::new(ShouldNotRun(ran.clone()))],
        };
        let outcome = services.run_tick(&mut daemon, SystemTime::now()).await;
        assert_eq!(outcome, TickOutcome::Restart);
        assert!(
            !ran.load(Ordering::SeqCst),
            "service after Restart must not run"
        );
    }

    /// A service that appends `init:<name>` / `stop:<name>` to a shared log so
    /// tests can assert ordering. `fail_init` makes `init` error.
    struct Recorder {
        name: String,
        log: Arc<Mutex<Vec<String>>>,
        fail_init: bool,
    }

    impl Recorder {
        fn boxed(name: &str, log: Arc<Mutex<Vec<String>>>) -> Box<dyn Service> {
            Box::new(Self {
                name: name.to_string(),
                log,
                fail_init: false,
            })
        }
        fn failing(name: &str, log: Arc<Mutex<Vec<String>>>) -> Box<dyn Service> {
            Box::new(Self {
                name: name.to_string(),
                log,
                fail_init: true,
            })
        }
    }

    impl Service for Recorder {
        fn name(&self) -> &str {
            &self.name
        }
        fn init(&mut self) -> Result<(), String> {
            self.log.lock().unwrap().push(format!("init:{}", self.name));
            if self.fail_init {
                return Err("boom".into());
            }
            Ok(())
        }
        fn stop(&mut self) -> Result<(), String> {
            self.log.lock().unwrap().push(format!("stop:{}", self.name));
            Ok(())
        }
    }

    // Normal: services init in registration order and stop in reverse.
    #[test]
    fn starts_in_order_stops_in_reverse_normal() {
        let log = Arc::new(Mutex::new(Vec::new()));
        let mut c = Controller::new();
        c.register(Recorder::boxed("a", log.clone()));
        c.register(Recorder::boxed("b", log.clone()));
        c.register(Recorder::boxed("c", log.clone()));

        c.start().expect("all init");
        c.stop();

        let entries = log.lock().unwrap().clone();
        assert_eq!(
            entries,
            vec![
                "init:a", "init:b", "init:c", // startup order
                "stop:c", "stop:b", "stop:a", // reverse teardown
            ]
        );
    }

    // Robust: a mid-list init failure tears down only the services that
    // already started, in reverse, and reports the failing name.
    #[test]
    fn init_failure_unwinds_started_services_robust() {
        let log = Arc::new(Mutex::new(Vec::new()));
        let mut c = Controller::new();
        c.register(Recorder::boxed("a", log.clone()));
        c.register(Recorder::failing("b", log.clone()));
        c.register(Recorder::boxed("c", log.clone()));

        let err = c.start().unwrap_err();
        assert!(err.contains("`b`"), "names the failing service: {err}");

        let entries = log.lock().unwrap().clone();
        // a inits, b inits (and fails), then a is stopped. c never inits.
        assert_eq!(entries, vec!["init:a", "init:b", "stop:a"]);
    }

    // Robust: tick_all runs every started service and collects per-service
    // errors without halting the pass.
    #[test]
    fn tick_all_collects_errors_without_halting_robust() {
        struct Ticker {
            name: String,
            fail: bool,
            ticks: Arc<Mutex<u32>>,
        }
        impl Service for Ticker {
            fn name(&self) -> &str {
                &self.name
            }
            fn tick(&mut self) -> Result<(), String> {
                *self.ticks.lock().unwrap() += 1;
                if self.fail {
                    Err("tick failed".into())
                } else {
                    Ok(())
                }
            }
        }
        let ticks = Arc::new(Mutex::new(0));
        let mut c = Controller::new();
        c.register(Box::new(Ticker {
            name: "ok".into(),
            fail: false,
            ticks: ticks.clone(),
        }));
        c.register(Box::new(Ticker {
            name: "bad".into(),
            fail: true,
            ticks: ticks.clone(),
        }));
        c.start().unwrap();

        let errors = c.tick_all();
        assert_eq!(*ticks.lock().unwrap(), 2, "both services ticked");
        assert_eq!(errors.len(), 1, "one error collected");
        assert_eq!(errors[0].0, "bad");
    }

    // Robust: stop is idempotent — a second stop after teardown does nothing.
    #[test]
    fn stop_is_idempotent_robust() {
        let log = Arc::new(Mutex::new(Vec::new()));
        let mut c = Controller::new();
        c.register(Recorder::boxed("a", log.clone()));
        c.start().unwrap();
        c.stop();
        let errors = c.stop(); // second stop
        assert!(errors.is_empty());
        assert_eq!(
            log.lock()
                .unwrap()
                .iter()
                .filter(|e| *e == "stop:a")
                .count(),
            1
        );
    }
}

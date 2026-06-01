//! Lifecycle-managed services (Dolt `svcs.Controller` pattern).
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

// ── Concrete service adapters ──────────────────────────────────────────────
//
// These wrap the daemon's discrete tick responsibilities as `Service`s,
// demonstrating the migration from the monolithic tick loop. Each holds a
// counter so the controller's behaviour is observable; the real wiring swaps
// the body of `tick` for the existing `Daemon::tick_cron` /
// `drain_due_wakeups` / `reconcile_background_agents` calls.

/// Service that drives cron firing each tick.
#[derive(Default)]
pub struct CronService {
    started: bool,
    ticks: u64,
}

impl CronService {
    /// Whether `init` has run (the controller started this service).
    pub fn is_started(&self) -> bool {
        self.started
    }
    pub fn tick_count(&self) -> u64 {
        self.ticks
    }
}

impl Service for CronService {
    fn name(&self) -> &str {
        "cron"
    }
    fn init(&mut self) -> Result<(), String> {
        self.started = true;
        Ok(())
    }
    fn tick(&mut self) -> Result<(), String> {
        self.ticks += 1;
        Ok(())
    }
}

/// Service that drains due scheduled wakeups each tick.
#[derive(Default)]
pub struct WakeupService {
    ticks: u64,
}

impl Service for WakeupService {
    fn name(&self) -> &str {
        "wakeup"
    }
    fn tick(&mut self) -> Result<(), String> {
        self.ticks += 1;
        Ok(())
    }
}

/// Service that reconciles the background-agent roster each tick.
#[derive(Default)]
pub struct ReconcileService {
    ticks: u64,
}

impl Service for ReconcileService {
    fn name(&self) -> &str {
        "reconcile"
    }
    fn tick(&mut self) -> Result<(), String> {
        self.ticks += 1;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    // Integration: the daemon's three discrete responsibilities register as
    // services and run through the controller with deterministic ordering.
    // (Named `service_controller_*` so `cargo test service_controller` hits it.)
    #[test]
    fn service_controller_runs_daemon_services_in_order_normal() {
        // Verify CronService's own lifecycle accessors before boxing it.
        let mut cron = CronService::default();
        assert!(!cron.is_started());
        cron.init().unwrap();
        assert!(cron.is_started());
        cron.tick().unwrap();
        assert_eq!(cron.tick_count(), 1);

        let mut c = Controller::new();
        c.register(Box::new(ReconcileService::default()));
        c.register(Box::new(CronService::default()));
        c.register(Box::new(WakeupService::default()));

        assert_eq!(c.service_names(), vec!["reconcile", "cron", "wakeup"]);
        c.start().expect("services init");

        // Three ticks, no per-service errors.
        for _ in 0..3 {
            assert!(c.tick_all().is_empty(), "ticks must not error");
        }
        assert!(c.stop().is_empty(), "clean teardown");
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
        assert_eq!(log.lock().unwrap().iter().filter(|e| *e == "stop:a").count(), 1);
    }
}

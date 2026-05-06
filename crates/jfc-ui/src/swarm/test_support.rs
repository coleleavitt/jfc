//! Shared test helpers for the swarm module.
//!
//! Centralizes the `JFC_SWARM_HOME_OVERRIDE` env-var management so that the
//! per-file `tests` modules in `mailbox`, `permission_sync`, `team_helpers`,
//! and `runner` don't each define their own `Mutex` — those are local to
//! their module, so cargo's parallel runner could let four tests in four
//! modules race each other on the same global env var. One process-wide
//! mutex here makes the override atomic across every swarm test.

use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard};

use tempfile::TempDir;

/// Process-global lock guarding the `JFC_SWARM_HOME_OVERRIDE` env var.
/// Acquired by `HomeOverride::new()`; held until the guard is dropped.
static ENV_LOCK: Mutex<()> = Mutex::new(());

/// RAII guard that points the swarm path helpers at a fresh `TempDir`.
///
/// Drop order: env var first (still under the lock), then the temp dir, then
/// the lock release — keeps another test from observing the override before
/// the temp directory is wiped.
pub(crate) struct HomeOverride {
    _dir: TempDir,
    _guard: MutexGuard<'static, ()>,
}

impl HomeOverride {
    pub(crate) fn new() -> Self {
        // `unwrap_or_else(into_inner)` lets us recover from a poisoned mutex:
        // a previous test may have panicked while holding the guard, but its
        // own `Drop` would still run and unset the env var, so the next test
        // can safely proceed.
        let guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = TempDir::new().expect("tempdir");
        // SAFETY: we hold `ENV_LOCK`, so no other test in this process is
        // touching `JFC_SWARM_HOME_OVERRIDE` concurrently.
        unsafe { std::env::set_var("JFC_SWARM_HOME_OVERRIDE", dir.path()) };
        Self {
            _dir: dir,
            _guard: guard,
        }
    }

    /// Path of the temp directory standing in for `$HOME`.
    #[allow(dead_code)]
    pub(crate) fn home(&self) -> PathBuf {
        self._dir.path().to_path_buf()
    }
}

impl Drop for HomeOverride {
    fn drop(&mut self) {
        // SAFETY: still holding `ENV_LOCK` until this guard's own drop runs.
        unsafe { std::env::remove_var("JFC_SWARM_HOME_OVERRIDE") };
    }
}

//! Process-global tracker for in-flight `bash` tool subprocesses.
//!
//! The model can ESC×2 to interrupt a turn. v132 SIGTERMs any in-flight bash
//! children at that point so the user's resources aren't held by a runaway
//! script after they've already given up on the result. jfc previously let
//! bash children continue in the background — wasteful and surprising.
//!
//! Foreground `execute_bash` calls register their PID before the process runs
//! and deregister when they exit (success, failure, timeout, or after being
//! auto-backgrounded). Explicit `run_in_background=true` calls are not generic
//! abort targets; their own timeout path still terminates the process tree.
//! On user abort, `terminate_all()` walks the registry and signals each PID.

use std::collections::HashSet;
use std::sync::Mutex;

static REGISTRY: Mutex<Option<HashSet<u32>>> = Mutex::new(None);

/// Register an in-flight bash PID. Idempotent on duplicate insert.
pub fn register(pid: u32) {
    let Ok(mut guard) = REGISTRY.lock() else {
        return;
    };
    let set = guard.get_or_insert_with(HashSet::new);
    set.insert(pid);
    tracing::trace!(target: "jfc::bash::pids", pid, registered = set.len(), "bash PID registered");
}

/// Deregister a PID once the child has exited / been killed.
pub fn deregister(pid: u32) {
    let Ok(mut guard) = REGISTRY.lock() else {
        return;
    };
    let set = guard.get_or_insert_with(HashSet::new);
    set.remove(&pid);
    tracing::trace!(target: "jfc::bash::pids", pid, remaining = set.len(), "bash PID deregistered");
}

/// Snapshot the currently-tracked PIDs. Used by tests and by the abort
/// path to compute work-to-do without holding the lock.
pub fn snapshot() -> Vec<u32> {
    let Ok(guard) = REGISTRY.lock() else {
        return Vec::new();
    };
    guard
        .as_ref()
        .map(|s| s.iter().copied().collect())
        .unwrap_or_default()
}

/// Signal a bash subprocess and, on Unix, the process group/session it leads.
///
/// Tool commands are spawned after `setsid()`, so the bash child is also the
/// process-group leader. Signalling `-pid` catches grandchildren that inherited
/// stdout/stderr or ignored the top-level shell's death.
#[cfg(unix)]
pub fn signal_process_tree(pid: u32, signal: libc::c_int) -> bool {
    let target = match libc::pid_t::try_from(pid) {
        Ok(t) => t,
        Err(_) => {
            tracing::warn!(target: "jfc::bash::pids", pid, "skipping out-of-range pid");
            return false;
        }
    };
    if target <= 0 {
        tracing::warn!(target: "jfc::bash::pids", pid, "refusing to signal pid <= 0");
        return false;
    }

    // SAFETY: kill(2) is async-signal-safe. Negative target means process
    // group; because commands run under setsid(), the child pid is the pgid.
    let group_result = unsafe { libc::kill(-target, signal) };
    if group_result == 0 {
        return true;
    }

    // Fallback for platforms/setups where the group no longer exists but the
    // child PID is still signalable.
    let pid_result = unsafe { libc::kill(target, signal) };
    pid_result == 0
}

#[cfg(not(unix))]
pub fn signal_process_tree(_pid: u32, _signal: i32) -> bool {
    false
}

/// SIGTERM every tracked bash subprocess. Returns the count of signals
/// dispatched. Best-effort: if a PID has already exited the kill is a
/// no-op. Linux/Unix only — on other platforms this is a no-op.
#[cfg(unix)]
pub fn terminate_all() -> usize {
    let pids = snapshot();
    let mut count = 0;
    for pid in pids {
        if signal_process_tree(pid, libc::SIGTERM) {
            count += 1;
            tracing::info!(target: "jfc::bash::pids", pid, "SIGTERM dispatched (user abort)");
        }
    }
    count
}

#[cfg(not(unix))]
pub fn terminate_all() -> usize {
    0
}

/// RAII handle: registers `pid` on construction, deregisters on Drop.
/// Use as `let _g = PidGuard::register(pid);` so every exit path is safe.
pub struct PidGuard {
    pid: u32,
}

impl PidGuard {
    pub fn register(pid: u32) -> Self {
        register(pid);
        Self { pid }
    }
}

impl Drop for PidGuard {
    fn drop(&mut self) {
        deregister(self.pid);
    }
}

/// Reset the registry to a known-empty state. Test-support only — callers
/// in downstream crates' test suites need this across the crate boundary,
/// so it cannot be `#[cfg(test)]`-gated here.
#[doc(hidden)]
pub fn clear_for_test() {
    if let Ok(mut guard) = REGISTRY.lock() {
        *guard = Some(HashSet::new());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Tests share the process-global REGISTRY. Serialize them so a
    /// parallel test's bash execution can't register a PID between
    /// `clear_for_test` and the assertion — the "random failure under
    /// `cargo test`" flake that plagued CI.
    fn test_lock() -> &'static std::sync::Mutex<()> {
        static LOCK: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();
        LOCK.get_or_init(|| std::sync::Mutex::new(()))
    }

    #[test]
    fn register_and_deregister_normal() {
        let _guard = test_lock().lock().unwrap_or_else(|p| p.into_inner());
        clear_for_test();
        register(1234);
        register(5678);
        let s = snapshot();
        assert!(s.contains(&1234));
        assert!(s.contains(&5678));
        deregister(1234);
        let s = snapshot();
        assert!(!s.contains(&1234));
        assert!(s.contains(&5678));
    }

    #[test]
    fn deregister_missing_pid_is_noop_robust() {
        let _guard = test_lock().lock().unwrap_or_else(|p| p.into_inner());
        clear_for_test();
        deregister(99999);
        assert!(snapshot().is_empty());
    }

    #[test]
    fn terminate_all_signals_invalid_pids_robust() {
        let _guard = test_lock().lock().unwrap_or_else(|p| p.into_inner());
        clear_for_test();
        // Use a PID that is overwhelmingly unlikely to exist. Avoid u32::MAX
        // because it wraps to pid_t -1 which signals ALL user processes!
        register(4_000_000);
        let _ = terminate_all();
        // Registry is *not* cleared — the dispatcher waits for the process
        // to actually exit and call deregister. Here we verify the entry
        // is still tracked.
        assert_eq!(snapshot().len(), 1);
    }
}

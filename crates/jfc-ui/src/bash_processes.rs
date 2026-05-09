//! Process-global tracker for in-flight `bash` tool subprocesses.
//!
//! The model can ESC×2 to interrupt a turn. v132 SIGTERMs any in-flight bash
//! children at that point so the user's resources aren't held by a runaway
//! script after they've already given up on the result. jfc previously let
//! bash children continue in the background — wasteful and surprising.
//!
//! Each `execute_bash` call registers its PID before the process runs and
//! deregisters when it exits (success, failure, or timeout). On user abort,
//! `terminate_all()` walks the registry and signals each PID with SIGTERM,
//! followed by SIGKILL after a grace period if the child hasn't exited.

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
    guard.as_ref().map(|s| s.iter().copied().collect()).unwrap_or_default()
}

/// SIGTERM every tracked bash subprocess. Returns the count of signals
/// dispatched. Best-effort: if a PID has already exited the kill is a
/// no-op. Linux/Unix only — on other platforms this is a no-op.
#[cfg(unix)]
pub fn terminate_all() -> usize {
    let pids = snapshot();
    let mut count = 0;
    for pid in pids {
        let target = pid as libc::pid_t;
        // Guard: pid 0 signals the process group, pid -1 signals ALL user
        // processes. Never allow either — they'd nuke the session.
        if target <= 0 {
            tracing::warn!(target: "jfc::bash::pids", pid, "refusing to signal pid <= 0");
            continue;
        }
        // SAFETY: kill(2) is async-signal-safe. SIGTERM gives the child a
        // chance to clean up; we don't escalate to SIGKILL here because the
        // tool's own timeout path already handles hard-kill on hang.
        let r = unsafe { libc::kill(target, libc::SIGTERM) };
        if r == 0 {
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

#[cfg(test)]
pub fn clear_for_test() {
    if let Ok(mut guard) = REGISTRY.lock() {
        *guard = Some(HashSet::new());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn register_and_deregister_normal() {
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
        clear_for_test();
        deregister(99999);
        assert!(snapshot().is_empty());
    }

    #[test]
    fn terminate_all_signals_invalid_pids_robust() {
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

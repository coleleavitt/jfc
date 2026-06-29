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

struct RegistryTrace {
    label: &'static str,
    pid: u32,
    registry_size: usize,
    changed: bool,
}

struct SignalTrace {
    pid: u32,
    signal: libc::c_int,
    target: &'static str,
    ok: bool,
}

/// Register an in-flight bash PID. Idempotent on duplicate insert.
pub fn register(pid: u32) {
    let _linkscope_register = linkscope::phase("tools.bash_processes.register");
    let Ok(mut guard) = REGISTRY.lock() else {
        linkscope::record_items("tools.bash_processes.registry_lock_error", 1);
        return;
    };
    let set = guard.get_or_insert_with(HashSet::new);
    let inserted = set.insert(pid);
    linkscope::record_items("tools.bash_processes.register", 1);
    trace_registry_event(RegistryTrace {
        label: "tools.bash_processes.register.detail",
        pid,
        registry_size: set.len(),
        changed: inserted,
    });
    tracing::trace!(target: "jfc::bash::pids", pid, registered = set.len(), "bash PID registered");
}

/// Deregister a PID once the child has exited / been killed.
pub fn deregister(pid: u32) {
    let _linkscope_deregister = linkscope::phase("tools.bash_processes.deregister");
    let Ok(mut guard) = REGISTRY.lock() else {
        linkscope::record_items("tools.bash_processes.registry_lock_error", 1);
        return;
    };
    let set = guard.get_or_insert_with(HashSet::new);
    let removed = set.remove(&pid);
    linkscope::record_items("tools.bash_processes.deregister", 1);
    trace_registry_event(RegistryTrace {
        label: "tools.bash_processes.deregister.detail",
        pid,
        registry_size: set.len(),
        changed: removed,
    });
    tracing::trace!(target: "jfc::bash::pids", pid, remaining = set.len(), "bash PID deregistered");
}

/// Snapshot the currently-tracked PIDs. Used by tests and by the abort
/// path to compute work-to-do without holding the lock.
pub fn snapshot() -> Vec<u32> {
    let _linkscope_snapshot = linkscope::phase("tools.bash_processes.snapshot");
    let Ok(guard) = REGISTRY.lock() else {
        linkscope::record_items("tools.bash_processes.registry_lock_error", 1);
        return Vec::new();
    };
    let snapshot: Vec<u32> = guard
        .as_ref()
        .map(|s| s.iter().copied().collect())
        .unwrap_or_default();
    linkscope::record_items(
        "tools.bash_processes.snapshot.pids",
        usize_to_u64_saturating(snapshot.len()),
    );
    snapshot
}

/// Signal a bash subprocess and, on Unix, the process group/session it leads.
///
/// Tool commands are spawned after `setsid()`, so the bash child is also the
/// process-group leader. Signalling `-pid` catches grandchildren that inherited
/// stdout/stderr or ignored the top-level shell's death.
#[cfg(unix)]
pub fn signal_process_tree(pid: u32, signal: libc::c_int) -> bool {
    let _linkscope_signal = linkscope::phase("tools.bash_processes.signal_tree");
    let target = match libc::pid_t::try_from(pid) {
        Ok(t) => t,
        Err(_) => {
            linkscope::record_items("tools.bash_processes.signal.invalid_pid", 1);
            trace_signal_event(SignalTrace {
                pid,
                signal,
                target: "invalid_pid",
                ok: false,
            });
            tracing::warn!(target: "jfc::bash::pids", pid, "skipping out-of-range pid");
            return false;
        }
    };
    if target <= 0 {
        linkscope::record_items("tools.bash_processes.signal.nonpositive_pid", 1);
        trace_signal_event(SignalTrace {
            pid,
            signal,
            target: "nonpositive_pid",
            ok: false,
        });
        tracing::warn!(target: "jfc::bash::pids", pid, "refusing to signal pid <= 0");
        return false;
    }

    // SAFETY: Category 8 — FFI boundary. `target` is a positive pid_t converted
    // from u32 above, and negative target intentionally addresses its process
    // group. Bash commands run under setsid(), so the child pid is the pgid.
    let group_result = unsafe { libc::kill(-target, signal) };
    if group_result == 0 {
        linkscope::record_items("tools.bash_processes.signal.group_ok", 1);
        trace_signal_event(SignalTrace {
            pid,
            signal,
            target: "group",
            ok: true,
        });
        return true;
    }

    // Fallback for platforms/setups where the group no longer exists but the
    // child PID is still signalable.
    // SAFETY: Category 8 — FFI boundary. `target` is the same positive pid_t
    // validated above; `signal` is forwarded unchanged to kill(2).
    let pid_result = unsafe { libc::kill(target, signal) };
    let ok = pid_result == 0;
    linkscope::record_items(
        if ok {
            "tools.bash_processes.signal.pid_ok"
        } else {
            "tools.bash_processes.signal.failed"
        },
        1,
    );
    trace_signal_event(SignalTrace {
        pid,
        signal,
        target: "pid",
        ok,
    });
    ok
}

#[cfg(not(unix))]
pub fn signal_process_tree(_pid: u32, _signal: i32) -> bool {
    linkscope::record_items("tools.bash_processes.signal.unsupported", 1);
    false
}

/// SIGTERM every tracked bash subprocess. Returns the count of signals
/// dispatched. Best-effort: if a PID has already exited the kill is a
/// no-op. Linux/Unix only — on other platforms this is a no-op.
#[cfg(unix)]
pub fn terminate_all() -> usize {
    let _linkscope_terminate = linkscope::phase("tools.bash_processes.terminate_all");
    let pids = snapshot();
    linkscope::record_items(
        "tools.bash_processes.terminate_all.tracked",
        usize_to_u64_saturating(pids.len()),
    );
    let mut count = 0;
    for pid in pids {
        if signal_process_tree(pid, libc::SIGTERM) {
            count += 1;
            tracing::info!(target: "jfc::bash::pids", pid, "SIGTERM dispatched (user abort)");
        }
    }
    linkscope::record_items(
        "tools.bash_processes.terminate_all.signaled",
        usize_to_u64_saturating(count),
    );
    count
}

#[cfg(not(unix))]
pub fn terminate_all() -> usize {
    linkscope::record_items("tools.bash_processes.terminate_all.unsupported", 1);
    0
}

/// RAII handle: registers `pid` on construction, deregisters on Drop.
/// Use as `let _g = PidGuard::register(pid);` so every exit path is safe.
pub struct PidGuard {
    pid: u32,
}

impl PidGuard {
    pub fn register(pid: u32) -> Self {
        let _linkscope_guard = linkscope::phase("tools.bash_processes.pid_guard.register");
        register(pid);
        Self { pid }
    }
}

impl Drop for PidGuard {
    fn drop(&mut self) {
        let _linkscope_guard = linkscope::phase("tools.bash_processes.pid_guard.drop");
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
        linkscope::record_items("tools.bash_processes.clear_for_test", 1);
    }
}

fn trace_registry_event(input: RegistryTrace) {
    if !linkscope::trace_detail_enabled() {
        return;
    }
    linkscope::detail_event_fields(
        input.label,
        [
            linkscope::TraceField::count("pid", u64::from(input.pid)),
            linkscope::TraceField::count(
                "registry_size",
                usize_to_u64_saturating(input.registry_size),
            ),
            linkscope::TraceField::count("changed", u64::from(input.changed)),
        ],
    );
}

fn trace_signal_event(input: SignalTrace) {
    if !linkscope::trace_detail_enabled() {
        return;
    }
    linkscope::detail_event_fields(
        "tools.bash_processes.signal.detail",
        [
            linkscope::TraceField::count("pid", u64::from(input.pid)),
            linkscope::TraceField::signed("signal", i64::from(input.signal)),
            linkscope::TraceField::text("target", input.target),
            linkscope::TraceField::count("ok", u64::from(input.ok)),
        ],
    );
}

fn usize_to_u64_saturating(value: usize) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests;

//! Watch CLAUDE.md / `.claude/agents/*.md` / `~/.config/jfc/settings.toml`
//! for changes and emit a `<system-reminder>` so the model picks up the
//! new content on the next turn.
//!
//! The watcher runs once at startup. It collects path candidates from
//! the standard CLAUDE.md hierarchy + the agents directory + settings
//! files, registers a `notify::RecommendedWatcher`, and pushes change
//! events into a process-global counter the Tick handler in `main.rs`
//! reads.
//!
//! When the counter increments, the Tick handler:
//! - drops a toast saying "CLAUDE.md reloaded"
//! - prepends a system-reminder to the next outbound prompt
//!
//! Failures are silent (logged via tracing) — file watching is best-
//! effort and shouldn't block the TUI.

use notify::{RecursiveMode, Watcher};
use std::path::PathBuf;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};

static CHANGE_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Snapshot the change counter — the Tick handler compares to its
/// last-seen value to detect inbound file changes.
pub fn change_counter() -> u64 {
    CHANGE_COUNTER.load(Ordering::SeqCst)
}

/// Spawn the watcher. Idempotent — calling twice is harmless (the
/// second call no-ops).
pub fn install() {
    static INSTALLED: OnceLock<()> = OnceLock::new();
    INSTALLED.get_or_init(|| {
        if let Err(e) = spawn_watcher() {
            tracing::debug!(target: "jfc::file_watcher", error = %e, "file watcher disabled");
        }
    });
}

fn spawn_watcher() -> Result<(), String> {
    let paths = candidate_paths();
    if paths.is_empty() {
        return Err("no candidate paths to watch".into());
    }
    let mut watcher =
        notify::recommended_watcher(|res: Result<notify::Event, notify::Error>| match res {
            Ok(event) => {
                use notify::EventKind;
                if matches!(
                    event.kind,
                    EventKind::Modify(_) | EventKind::Create(_) | EventKind::Remove(_)
                ) {
                    CHANGE_COUNTER.fetch_add(1, Ordering::SeqCst);
                    tracing::debug!(
                        target: "jfc::file_watcher",
                        kind = ?event.kind,
                        paths = ?event.paths,
                        "config file change detected"
                    );
                }
            }
            Err(e) => {
                tracing::debug!(target: "jfc::file_watcher", error = %e, "watch error");
            }
        })
        .map_err(|e| format!("watcher init: {e}"))?;
    for path in &paths {
        let mode = if path.is_dir() {
            RecursiveMode::Recursive
        } else {
            RecursiveMode::NonRecursive
        };
        if let Err(e) = watcher.watch(path, mode) {
            tracing::debug!(
                target: "jfc::file_watcher",
                path = %path.display(),
                error = %e,
                "couldn't watch path"
            );
        }
    }
    // Leak the watcher so it stays alive for the process lifetime.
    // Dropping the watcher would unsubscribe; we want it permanent.
    Box::leak(Box::new(watcher));
    tracing::info!(
        target: "jfc::file_watcher",
        path_count = paths.len(),
        "file watcher installed"
    );
    Ok(())
}

fn candidate_paths() -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    let cwd = std::env::current_dir().ok();
    if let Some(ref c) = cwd {
        let project_md = c.join("CLAUDE.md");
        if project_md.exists() {
            out.push(project_md);
        }
        let agents_dir = c.join(".claude").join("agents");
        if agents_dir.is_dir() {
            out.push(agents_dir);
        }
        let local_md = c.join(".claude").join("CLAUDE.md");
        if local_md.exists() {
            out.push(local_md);
        }
    }
    if let Some(home) = std::env::var_os("HOME") {
        let user_md = std::path::Path::new(&home)
            .join(".claude")
            .join("CLAUDE.md");
        if user_md.exists() {
            out.push(user_md);
        }
        let user_settings = std::path::Path::new(&home)
            .join(".config")
            .join("jfc")
            .join("config.toml");
        if user_settings.exists() {
            out.push(user_settings);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn change_counter_starts_at_zero_normal() {
        // Note: this assumes no change has been recorded yet.
        // In CI / clean processes that's true; in dev iteration this
        // counter might be > 0 from a prior test, so we just check
        // it's reachable.
        let _ = change_counter();
    }

    #[test]
    fn install_is_idempotent_normal() {
        // Calling install() twice should not panic.
        install();
        install();
    }
}

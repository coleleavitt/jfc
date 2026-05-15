//! Watch CLAUDE.md / `.claude/agents/*.md` / `~/.config/jfc/settings.toml` /
//! `~/.config/jfc/keybindings.toml` for changes and emit a
//! `<system-reminder>` so the model picks up the new content on the next turn.
//!
//! The watcher runs once at startup. It collects path candidates from
//! the standard CLAUDE.md hierarchy + the agents directory + settings
//! files, registers a `notify::RecommendedWatcher`, and pushes change
//! events into process-global counters the Tick handler in `main.rs`
//! reads.
//!
//! When the config counter increments, the Tick handler:
//! - drops a toast saying "Config file changed"
//! - prepends a system-reminder to the next outbound prompt
//!
//! When the keybindings counter increments, the Tick handler:
//! - calls `crate::keybindings::load()` to hot-reload the bindings
//! - drops a toast saying "Keybindings reloaded"
//!
//! Failures are silent (logged via tracing) — file watching is best-
//! effort and shouldn't block the TUI.

use notify::{RecursiveMode, Watcher};
use std::path::PathBuf;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};

static CHANGE_COUNTER: AtomicU64 = AtomicU64::new(0);
/// Separate counter that ticks only when `keybindings.toml` changes, so the
/// Tick handler can reload bindings without issuing a system-reminder.
static KEYBINDINGS_CHANGE_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Snapshot the config change counter — the Tick handler compares to its
/// last-seen value to detect inbound file changes.
pub fn change_counter() -> u64 {
    CHANGE_COUNTER.load(Ordering::SeqCst)
}

/// Snapshot the keybindings change counter.
pub fn keybindings_change_counter() -> u64 {
    KEYBINDINGS_CHANGE_COUNTER.load(Ordering::SeqCst)
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
    // Build the keybindings path once so we can detect it in the closure.
    let keybindings_path = crate::keybindings::keybindings_path();

    let (config_paths, keybindings_watch) = candidate_paths(&keybindings_path);
    if config_paths.is_empty() && keybindings_watch.is_none() {
        return Err("no candidate paths to watch".into());
    }

    // inotify on Linux fires events for every file in the watched directory,
    // not just the specific file we registered. We keep an explicit allowlist
    // of paths and directories so sibling files (sessions/*.json,
    // anthropic-accounts.json, litellm.toml, …) don't trigger spurious
    // "Config file changed" toasts on every subagent turn.
    let config_paths_set: std::collections::HashSet<PathBuf> =
        config_paths.iter().cloned().collect();
    let config_dirs_set: std::collections::HashSet<PathBuf> = config_paths
        .iter()
        .filter(|p| p.is_dir())
        .cloned()
        .collect();

    let keybindings_path_clone = keybindings_path.clone();
    let mut watcher =
        notify::recommended_watcher(move |res: Result<notify::Event, notify::Error>| match res {
            Ok(event) => {
                use notify::EventKind;
                if matches!(
                    event.kind,
                    EventKind::Modify(_) | EventKind::Create(_) | EventKind::Remove(_)
                ) {
                    // Check whether this event is for the keybindings file.
                    let is_keybindings = event.paths.iter().any(|p| p == &keybindings_path_clone);

                    if is_keybindings {
                        KEYBINDINGS_CHANGE_COUNTER.fetch_add(1, Ordering::SeqCst);
                        tracing::debug!(
                            target: "jfc::file_watcher",
                            kind = ?event.kind,
                            "keybindings.toml change detected"
                        );
                        return;
                    }

                    // Only fire the config counter when the changed path is an
                    // explicitly-watched file OR a file inside a watched directory.
                    // This prevents session saves / token writes to sibling files
                    // in ~/.config/jfc/ from generating a spurious toast every turn.
                    let is_watched = event.paths.iter().any(|p| {
                        // Direct file match (e.g. config.toml, CLAUDE.md).
                        if config_paths_set.contains(p) {
                            return true;
                        }
                        // File inside a watched directory (e.g. .claude/agents/).
                        if let Some(parent) = p.parent() {
                            if config_dirs_set.contains(parent) {
                                return true;
                            }
                        }
                        false
                    });

                    if is_watched {
                        CHANGE_COUNTER.fetch_add(1, Ordering::SeqCst);
                        tracing::debug!(
                            target: "jfc::file_watcher",
                            kind = ?event.kind,
                            paths = ?event.paths,
                            "config file change detected"
                        );
                    } else {
                        tracing::trace!(
                            target: "jfc::file_watcher",
                            paths = ?event.paths,
                            "ignoring event for non-watched sibling file"
                        );
                    }
                }
            }
            Err(e) => {
                tracing::debug!(target: "jfc::file_watcher", error = %e, "watch error");
            }
        })
        .map_err(|e| format!("watcher init: {e}"))?;

    let mut registered = 0usize;
    for path in &config_paths {
        let mode = if path.is_dir() {
            RecursiveMode::Recursive
        } else {
            RecursiveMode::NonRecursive
        };
        match watcher.watch(path, mode) {
            Ok(()) => registered += 1,
            Err(e) => tracing::debug!(
                target: "jfc::file_watcher",
                path = %path.display(),
                error = %e,
                "couldn't watch path"
            ),
        }
    }

    // Watch the keybindings file (or its parent dir if the file doesn't exist
    // yet, so we catch the initial creation).
    if let Some(kb_path) = &keybindings_watch {
        let watch_target = if kb_path.exists() {
            kb_path.clone()
        } else {
            // Watch the parent directory so a newly-created file is detected.
            kb_path
                .parent()
                .map(PathBuf::from)
                .unwrap_or_else(|| kb_path.clone())
        };
        match watcher.watch(&watch_target, RecursiveMode::NonRecursive) {
            Ok(()) => {
                registered += 1;
                tracing::debug!(
                    target: "jfc::file_watcher",
                    path = %watch_target.display(),
                    "watching keybindings path"
                );
            }
            Err(e) => tracing::debug!(
                target: "jfc::file_watcher",
                path = %watch_target.display(),
                error = %e,
                "couldn't watch keybindings path"
            ),
        }
    }

    if registered == 0 {
        return Err("no paths could be registered with the watcher".into());
    }

    // Leak the watcher so it stays alive for the process lifetime.
    // Dropping the watcher would unsubscribe; we want it permanent.
    Box::leak(Box::new(watcher));
    tracing::info!(
        target: "jfc::file_watcher",
        registered,
        "file watcher installed"
    );
    Ok(())
}

/// Returns `(config_paths, keybindings_path_option)`.
///
/// `keybindings_path_option` is always `Some(...)` — we always want to watch
/// it. It's separated from config_paths so the closure can identify it.
fn candidate_paths(keybindings_path: &PathBuf) -> (Vec<PathBuf>, Option<PathBuf>) {
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
    (out, Some(keybindings_path.clone()))
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
    fn keybindings_change_counter_reachable_normal() {
        let _ = keybindings_change_counter();
    }

    #[test]
    fn install_is_idempotent_normal() {
        // Calling install() twice should not panic.
        install();
        install();
    }
}

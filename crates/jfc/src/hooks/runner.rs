//! Script-based hook runner.
//!
//! Scans `.jfc/hooks/` for user-authored scripts (or JSON configs)
//! whose stem matches the event name (e.g. `pre-tool-use.sh`,
//! `pre-tool-use.json`). When a matching script is found, the runner:
//!
//! 1. Spawns the script via `sh -c <script>` (or invokes a JSON
//!    declarative hook directly).
//! 2. Writes the serialized [`HookEvent`] JSON to stdin.
//! 3. Reads stdout, expecting a JSON object that parses into
//!    [`HookDecision`].
//! 4. Bounds the whole interaction by a timeout (default 10s).
//!
//! The decision wire format on stdout is:
//! ```json
//! {"decision": "allow"}
//! {"decision": "deny", "reason": "blocked by policy"}
//! {"decision": "modify", "modified_input": {"command": "ls -la"}}
//! ```
//!
//! Missing scripts → [`HookDecision::Allow`] (zero-cost when no hooks
//! are installed). Script errors (non-zero exit, parse failure,
//! timeout) → [`HookDecision::Deny`] with a diagnostic reason so the
//! caller can surface the failure without silently proceeding.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use super::{HookDecision, HookEvent};

/// Default timeout for a single script invocation.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(10);

/// Directory (relative to cwd) scanned for hook scripts.
pub const HOOKS_DIR: &str = ".jfc/hooks";

/// File extensions recognised, in lookup priority order.
const SCRIPT_EXTENSIONS: &[&str] = &["sh", "bash", "py", "js", "json"];

/// Result of scanning `.jfc/hooks/` for a matching script.
#[derive(Debug, Clone)]
pub struct DiscoveredHook {
    pub path: PathBuf,
    pub kind: HookKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HookKind {
    /// Executable shell/script (run via `sh -c`).
    Script,
    /// Declarative JSON hook (parsed for a static decision).
    JsonConfig,
}

/// Wire-format decision object emitted by script hooks on stdout.
#[derive(Debug, serde::Deserialize)]
#[serde(tag = "decision", rename_all = "lowercase")]
enum WireDecision {
    Allow,
    Deny {
        #[serde(default)]
        reason: String,
    },
    Modify {
        modified_input: serde_json::Value,
    },
}

impl From<WireDecision> for HookDecision {
    fn from(w: WireDecision) -> Self {
        match w {
            WireDecision::Allow => HookDecision::Allow,
            WireDecision::Deny { reason } => HookDecision::Deny {
                reason: if reason.is_empty() {
                    "hook denied".to_string()
                } else {
                    reason
                },
            },
            WireDecision::Modify { modified_input } => HookDecision::Modify { modified_input },
        }
    }
}

/// Find the first script in `root/.jfc/hooks/` matching the event.
///
/// Returns `None` when no matching script exists (the common case
/// when the user has no hooks installed).
pub fn discover_hook(root: &Path, event: &HookEvent) -> Option<DiscoveredHook> {
    let dir = root.join(HOOKS_DIR);
    if !dir.is_dir() {
        return None;
    }
    let stem = event.script_name();
    for ext in SCRIPT_EXTENSIONS {
        let candidate = dir.join(format!("{stem}.{ext}"));
        if candidate.is_file() {
            let kind = if *ext == "json" {
                HookKind::JsonConfig
            } else {
                HookKind::Script
            };
            return Some(DiscoveredHook {
                path: candidate,
                kind,
            });
        }
    }
    None
}

/// Run any matching hook for the given event, scanning `root/.jfc/hooks/`.
///
/// If no hook is registered, returns [`HookDecision::Allow`] immediately.
pub fn run_hook(root: &Path, event: &HookEvent) -> HookDecision {
    run_hook_with_timeout(root, event, DEFAULT_TIMEOUT)
}

/// Same as [`run_hook`] but with a caller-supplied timeout.
pub fn run_hook_with_timeout(root: &Path, event: &HookEvent, timeout: Duration) -> HookDecision {
    let Some(hook) = discover_hook(root, event) else {
        return HookDecision::Allow;
    };
    match hook.kind {
        HookKind::JsonConfig => run_json_hook(&hook.path),
        HookKind::Script => run_script_hook(&hook.path, event, timeout),
    }
}

/// Execute a JSON declarative hook — the file's contents *are* the
/// decision object. Useful for static "always deny X" policies that
/// don't need a script.
fn run_json_hook(path: &Path) -> HookDecision {
    match std::fs::read_to_string(path) {
        Ok(contents) => match serde_json::from_str::<WireDecision>(&contents) {
            Ok(wire) => HookDecision::from(wire),
            Err(e) => HookDecision::Deny {
                reason: format!("hook {path:?} JSON parse error: {e}"),
            },
        },
        Err(e) => HookDecision::Deny {
            reason: format!("hook {path:?} read error: {e}"),
        },
    }
}

/// Execute a shell-script hook with the serialized event on stdin and
/// a timeout. The script's stdout must be a [`WireDecision`] JSON
/// object; otherwise the call is treated as Deny.
fn run_script_hook(path: &Path, event: &HookEvent, timeout: Duration) -> HookDecision {
    let payload = match serde_json::to_string(event) {
        Ok(s) => s,
        Err(e) => {
            return HookDecision::Deny {
                reason: format!("hook event serialize failed: {e}"),
            };
        }
    };

    let mut child = match Command::new("sh")
        .arg("-c")
        .arg(path.as_os_str())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env("JFC_HOOK_EVENT", event.script_name())
        .spawn()
    {
        Ok(c) => c,
        Err(e) => {
            return HookDecision::Deny {
                reason: format!("hook spawn failed ({path:?}): {e}"),
            };
        }
    };

    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(payload.as_bytes());
        // Closing stdin signals EOF to the script.
        drop(stdin);
    }

    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(_status)) => break,
            Ok(None) => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    return HookDecision::Deny {
                        reason: format!("hook {path:?} timed out after {timeout:?}"),
                    };
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            Err(e) => {
                return HookDecision::Deny {
                    reason: format!("hook wait error ({path:?}): {e}"),
                };
            }
        }
    }

    let output = match child.wait_with_output() {
        Ok(o) => o,
        Err(e) => {
            return HookDecision::Deny {
                reason: format!("hook output read error ({path:?}): {e}"),
            };
        }
    };

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let exit = output.status.code().unwrap_or(-1);
        return HookDecision::Deny {
            reason: if stderr.is_empty() {
                format!("hook {path:?} exited {exit}")
            } else {
                format!("hook {path:?} exited {exit}: {stderr}")
            },
        };
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let trimmed = stdout.trim();
    if trimmed.is_empty() {
        // Empty stdout on success = implicit allow. Common when a hook
        // wants to perform a side effect (logging, telemetry) without
        // influencing the decision.
        return HookDecision::Allow;
    }

    match serde_json::from_str::<WireDecision>(trimmed) {
        Ok(wire) => HookDecision::from(wire),
        Err(e) => HookDecision::Deny {
            reason: format!("hook {path:?} stdout not valid decision JSON: {e}"),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;

    fn make_event() -> HookEvent {
        HookEvent::PreToolUse {
            tool_name: "Bash".to_string(),
            tool_input: serde_json::json!({"command": "ls"}),
        }
    }

    #[test]
    fn discover_returns_none_when_dir_missing() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(discover_hook(tmp.path(), &make_event()).is_none());
    }

    #[test]
    fn run_hook_returns_allow_when_no_script() {
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir_all(tmp.path().join(HOOKS_DIR)).unwrap();
        assert_eq!(run_hook(tmp.path(), &make_event()), HookDecision::Allow);
    }

    #[test]
    fn json_config_hook_is_parsed() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join(HOOKS_DIR);
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("pre-tool-use.json"),
            r#"{"decision":"deny","reason":"static policy"}"#,
        )
        .unwrap();
        let d = run_hook(tmp.path(), &make_event());
        assert_eq!(
            d,
            HookDecision::Deny {
                reason: "static policy".into()
            }
        );
    }

    #[cfg(unix)]
    #[test]
    fn script_hook_allow_via_stdout() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join(HOOKS_DIR);
        fs::create_dir_all(&dir).unwrap();
        let script = dir.join("pre-tool-use.sh");
        fs::write(&script, "#!/bin/sh\nprintf '{\"decision\":\"allow\"}\\n'\n").unwrap();
        let mut perm = fs::metadata(&script).unwrap().permissions();
        perm.set_mode(0o755);
        fs::set_permissions(&script, perm).unwrap();

        assert_eq!(run_hook(tmp.path(), &make_event()), HookDecision::Allow);
    }

    #[cfg(unix)]
    #[test]
    fn script_hook_timeout_denies() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join(HOOKS_DIR);
        fs::create_dir_all(&dir).unwrap();
        let script = dir.join("pre-tool-use.sh");
        fs::write(&script, "#!/bin/sh\nsleep 5\n").unwrap();
        let mut perm = fs::metadata(&script).unwrap().permissions();
        perm.set_mode(0o755);
        fs::set_permissions(&script, perm).unwrap();

        let d = run_hook_with_timeout(tmp.path(), &make_event(), Duration::from_millis(150));
        match d {
            HookDecision::Deny { reason } => assert!(reason.contains("timed out")),
            other => panic!("expected Deny, got {other:?}"),
        }
    }
}

//! Persistent, reusable shell sessions (opt-in).
//!
//! A normal `Bash` call spawns a fresh `bash -c` per invocation, so `cd`, env
//! vars, and shell state never persist. A **persistent shell** keeps one
//! long-lived `bash` process per `shell_id`; sequential commands on the same id
//! share working directory and environment — the model can `cd somewhere` in one
//! call and the next call starts there.
//!
//! ## Opt-in
//!
//! This is NOT the default. A command is routed here only when it carries the
//! `shell:<id>\n` prefix (added by the Bash tool path when the caller selects a
//! persistent shell). The default one-shot fresh-shell behavior is unchanged.
//!
//! ## Design (the load-bearing correctness details)
//!
//! - **Per-shell serialization.** Each shell has its own async mutex, so two
//!   commands targeting the same id can't interleave on one stdin (which would
//!   produce garbage). Different ids run concurrently.
//! - **High-entropy per-command sentinel.** After each command we echo a random
//!   marker plus the exit code. The marker is fresh per command, so a command
//!   that happens to print a *previous* marker can't fake a boundary. Output
//!   before the marker line is the command's output; the marker line carries
//!   `$?`.
//! - **Dead-shell detection + respawn.** If the shell process has exited (a
//!   command ran `exit`, or it crashed), the next call transparently respawns a
//!   fresh shell for that id. State is lost (documented), but the tool keeps
//!   working instead of wedging forever.
//! - **Timeout abandons the shell, not just the command.** A persistent shell
//!   can't cleanly abandon a wedged command (a `read`/interactive prompt blocks
//!   stdin) without killing the shell. So on timeout we KILL and drop the shell;
//!   the next call respawns. This trades state-loss for liveness — the right
//!   call for an agent that must not hang.

use std::collections::HashMap;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, Command};
use tokio::sync::Mutex;

/// Prefix that opts a command into a persistent shell: `shell:<id>\n<command>`.
pub const SHELL_PREFIX: &str = "shell:";

/// Split a `shell:<id>\n<command>` payload into `(id, command)`, or `None` if it
/// isn't a persistent-shell request.
pub fn parse_shell_request(raw: &str) -> Option<(String, String)> {
    let rest = raw.strip_prefix(SHELL_PREFIX)?;
    let (id, command) = rest.split_once('\n')?;
    let id = id.trim();
    if id.is_empty() {
        return None;
    }
    Some((id.to_owned(), command.to_owned()))
}

/// Output of one persistent-shell command.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShellResult {
    pub stdout: String,
    pub exit_code: i32,
    /// True when the command was abandoned (timed out) and the shell was killed.
    pub timed_out: bool,
}

struct PersistentShell {
    child: Child,
    stdin: ChildStdin,
    /// Line reader over the merged stdout (stderr is redirected to stdout inside
    /// the shell via `exec 2>&1` so a single stream carries everything in order).
    reader: tokio::io::Lines<BufReader<tokio::process::ChildStdout>>,
}

impl PersistentShell {
    async fn spawn(shell: &str) -> std::io::Result<Self> {
        let mut child = Command::new(shell)
            .arg("-i")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .env("CI", "true")
            .env("TERM", "dumb")
            .env("NO_COLOR", "1")
            .env("PS1", "")
            .env("PS2", "")
            .kill_on_drop(true)
            .spawn()?;
        let stdin = child.stdin.take().expect("piped stdin");
        let stdout = child.stdout.take().expect("piped stdout");
        let mut shell = Self {
            child,
            stdin,
            reader: BufReader::new(stdout).lines(),
        };
        // Merge stderr into stdout in-shell so ordering is preserved and we only
        // read one stream. Also disable history expansion noise.
        shell
            .stdin
            .write_all(b"exec 2>&1\nset +o history\n")
            .await?;
        shell.stdin.flush().await?;
        Ok(shell)
    }

    /// Run one command, returning its output and exit code. The sentinel marks
    /// the boundary; the line carrying it also carries `$?`.
    async fn run(&mut self, command: &str, marker: &str) -> std::io::Result<(String, i32)> {
        // Newline-terminate the user command, then echo the marker + exit code.
        // `printf '%s %d\n'` avoids echo portability quirks.
        let framed = format!("{command}\nprintf '{marker} %d\\n' \"$?\"\n");
        self.stdin.write_all(framed.as_bytes()).await?;
        self.stdin.flush().await?;

        let mut out = String::new();
        loop {
            let Some(line) = self.reader.next_line().await? else {
                // EOF: the shell died mid-command.
                return Err(std::io::Error::new(
                    std::io::ErrorKind::UnexpectedEof,
                    "persistent shell exited during command",
                ));
            };
            if let Some(code_str) = line.strip_prefix(marker) {
                let exit_code = code_str.trim().parse::<i32>().unwrap_or(-1);
                return Ok((out, exit_code));
            }
            out.push_str(&line);
            out.push('\n');
        }
    }

    fn is_alive(&mut self) -> bool {
        // try_wait returns Ok(None) while the child is still running.
        matches!(self.child.try_wait(), Ok(None))
    }
}

type ShellMap = HashMap<String, Arc<Mutex<Option<PersistentShell>>>>;

/// Process-wide registry of persistent shells, one slot per `shell_id`. The
/// outer mutex guards the map; each inner `Arc<Mutex<Option<…>>>` serializes
/// commands on one shell (and lets `None` mean "needs (re)spawn").
fn shells() -> &'static Mutex<ShellMap> {
    static SHELLS: std::sync::OnceLock<Mutex<ShellMap>> = std::sync::OnceLock::new();
    SHELLS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn random_marker() -> String {
    format!("__JFC_SHELL_{}__", uuid::Uuid::new_v4().simple())
}

fn configured_shell() -> String {
    crate::config::load_arc()
        .bash_shell
        .clone()
        .unwrap_or_else(|| "bash".to_string())
}

/// Run `command` in the persistent shell `id`, spawning or respawning it as
/// needed. Serialized per id. On timeout the shell is killed and dropped (the
/// next call respawns), so the agent never hangs.
pub async fn run_in_shell(id: &str, command: &str, timeout: Duration) -> ShellResult {
    let slot = {
        let mut map = shells().lock().await;
        Arc::clone(
            map.entry(id.to_owned())
                .or_insert_with(|| Arc::new(Mutex::new(None))),
        )
    };

    let mut guard = slot.lock().await;

    // (Re)spawn if missing or dead.
    if guard.as_mut().is_none_or(|sh| !sh.is_alive())
        && let Err(err) = ensure_spawned(&mut guard).await
    {
        return ShellResult {
            stdout: format!("failed to start persistent shell: {err}"),
            exit_code: -1,
            timed_out: false,
        };
    }

    let marker = random_marker();
    let shell = guard.as_mut().expect("spawned above");
    let outcome = tokio::time::timeout(timeout, shell.run(command, &marker)).await;
    finish_run(id, &mut guard, outcome, timeout)
}

/// Spawn a shell into the slot. Separated so `run_in_shell` stays flat.
async fn ensure_spawned(guard: &mut Option<PersistentShell>) -> std::io::Result<()> {
    *guard = Some(PersistentShell::spawn(&configured_shell()).await?);
    Ok(())
}

/// Turn a timeout/run result into a `ShellResult`, resetting the shell slot on
/// death-mid-command or timeout so the next call respawns cleanly.
fn finish_run(
    id: &str,
    guard: &mut Option<PersistentShell>,
    outcome: Result<std::io::Result<(String, i32)>, tokio::time::error::Elapsed>,
    timeout: Duration,
) -> ShellResult {
    match outcome {
        Ok(Ok((stdout, exit_code))) => ShellResult {
            stdout,
            exit_code,
            timed_out: false,
        },
        Ok(Err(err)) => {
            // Shell died mid-command; drop it so the next call respawns.
            *guard = None;
            ShellResult {
                stdout: format!("persistent shell error: {err}"),
                exit_code: -1,
                timed_out: false,
            }
        }
        Err(_) => {
            // Timeout: can't cleanly abandon a wedged command — kill the shell.
            if let Some(mut sh) = guard.take()
                && let Err(err) = sh.child.start_kill()
            {
                tracing::debug!(
                    target: "jfc::tools::shell",
                    shell_id = id,
                    error = %err,
                    "failed to kill timed-out persistent shell (likely already exited)"
                );
            }
            ShellResult {
                stdout: format!(
                    "[persistent shell command timed out after {}ms; shell was reset]",
                    timeout.as_millis()
                ),
                exit_code: -1,
                timed_out: true,
            }
        }
    }
}

/// Drop a persistent shell by id (kills the process). Returns true if one
/// existed. Used by tests and a future `/bashes shell-close`.
#[cfg_attr(not(test), allow(dead_code))]
pub async fn close_shell(id: &str) -> bool {
    let slot = {
        let map = shells().lock().await;
        map.get(id).cloned()
    };
    let Some(slot) = slot else {
        return false;
    };
    let mut guard = slot.lock().await;
    let Some(mut sh) = guard.take() else {
        return false;
    };
    if let Err(err) = sh.child.start_kill() {
        tracing::debug!(
            target: "jfc::tools::shell",
            shell_id = id,
            error = %err,
            "close_shell: kill failed (likely already exited)"
        );
    }
    true
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn parse_shell_request_splits_id_and_command_normal() {
        let (id, cmd) = parse_shell_request("shell:work\ncd /tmp && pwd").unwrap();
        assert_eq!(id, "work");
        assert_eq!(cmd, "cd /tmp && pwd");
    }

    #[test]
    fn parse_request_rejects_bad_input_robust() {
        assert!(parse_shell_request("echo hi").is_none());
        assert!(parse_shell_request("shell:\ncmd").is_none());
        assert!(parse_shell_request("shell:noeol").is_none());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn shell_state_persists_normal() {
        let id = format!("test_persist_{}", uuid::Uuid::new_v4().simple());
        let to = Duration::from_secs(10);

        // First command sets a var and cd's.
        let r1 = run_in_shell(&id, "cd /tmp && export FOO=bar && echo set", to).await;
        assert_eq!(r1.exit_code, 0, "stdout: {}", r1.stdout);
        assert!(r1.stdout.contains("set"));

        // Second command sees the persisted cwd and env.
        let r2 = run_in_shell(&id, "echo $FOO @ $(pwd)", to).await;
        assert_eq!(r2.exit_code, 0, "stdout: {}", r2.stdout);
        assert!(
            r2.stdout.contains("bar @ /tmp"),
            "state did not persist: {:?}",
            r2.stdout
        );

        close_shell(&id).await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn persistent_shell_reports_exit_code_normal() {
        let id = format!("test_exit_{}", uuid::Uuid::new_v4().simple());
        let to = Duration::from_secs(10);
        let r = run_in_shell(&id, "false", to).await;
        assert_eq!(r.exit_code, 1, "stdout: {}", r.stdout);
        close_shell(&id).await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn persistent_shell_respawns_after_exit_robust() {
        let id = format!("test_respawn_{}", uuid::Uuid::new_v4().simple());
        let to = Duration::from_secs(10);
        // Kill the shell from inside.
        let _ = run_in_shell(&id, "exit 0", to).await;
        // Next call must transparently respawn and work.
        let r = run_in_shell(&id, "echo alive", to).await;
        assert_eq!(r.exit_code, 0, "stdout: {}", r.stdout);
        assert!(r.stdout.contains("alive"));
        close_shell(&id).await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn persistent_shell_timeout_resets_shell_robust() {
        let id = format!("test_timeout_{}", uuid::Uuid::new_v4().simple());
        // A command that blocks far longer than the timeout.
        let r = run_in_shell(&id, "sleep 30", Duration::from_millis(300)).await;
        assert!(r.timed_out, "expected timeout, got {:?}", r);
        // After the reset, the shell works again.
        let r2 = run_in_shell(&id, "echo recovered", Duration::from_secs(10)).await;
        assert_eq!(r2.exit_code, 0, "stdout: {}", r2.stdout);
        assert!(r2.stdout.contains("recovered"));
        close_shell(&id).await;
    }
}

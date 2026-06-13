//! Runtime launcher and session manager for external ACP agents.
//!
//! [`super::ExternalAgentProfile`] describes *what* an external agent is
//! (executable, args, env, auth, distribution). This module turns a profile
//! into a concrete, spawnable process and owns its lifecycle: resolve the
//! profile's placeholders against a [`LaunchContext`], spawn the child with
//! stdio piped (the transport an ACP JSON-RPC layer would later speak over),
//! track its [`ExternalAgentStatus`], and terminate it cleanly.
//!
//! Scope boundary: this is the process/session layer shared by Junie, Codex,
//! Gemini, and generic ACP. The ACP wire protocol (initialize handshake,
//! session/prompt RPCs) is a separate concern that rides on the stdio pipes
//! exposed here — it is intentionally NOT implemented in this module.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;

use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::Mutex;

use super::{ExternalAgentKind, ExternalAgentProfile};

/// Runtime inputs needed to resolve a profile's placeholders into a concrete
/// command. Keeps the profile itself static/process-agnostic.
#[derive(Debug, Clone)]
pub struct LaunchContext {
    /// Working directory for the spawned agent.
    pub cwd: PathBuf,
    /// JFC-managed home directory for the agent (resolves a profile's
    /// `<jfc-managed-*-home>` placeholders). Spawning creates it if absent.
    pub managed_home: PathBuf,
    /// Local proxy/gateway URL the agent should talk to, when the profile
    /// routes through one (resolves `<local-*-proxy-url>` placeholders).
    pub proxy_url: Option<String>,
}

/// A fully-resolved, spawnable command derived from a profile + context. All
/// placeholders are substituted; this is what [`ExternalAgentSession::spawn`]
/// executes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternalAgentSpec {
    pub kind: ExternalAgentKind,
    pub program: String,
    pub args: Vec<String>,
    pub env: BTreeMap<String, String>,
    pub cwd: PathBuf,
}

impl ExternalAgentSpec {
    /// Resolve a profile against a launch context into a concrete spec.
    ///
    /// Placeholder grammar (mirrors the strings the Air-derived profile uses):
    /// - `<jfc-managed-*-home>` → `context.managed_home`
    /// - `<local-*-proxy-url>`  → `context.proxy_url` (error if the profile
    ///   needs one but none is provided)
    /// - `<empty>`              → empty string
    pub fn resolve(
        profile: &ExternalAgentProfile,
        context: &LaunchContext,
    ) -> Result<Self, ExternalAgentLaunchError> {
        let program = resolve_binary(profile.executable_hint);

        let args = profile
            .default_args
            .iter()
            .map(|arg| resolve_placeholder(arg, context))
            .collect::<Result<Vec<_>, _>>()?;

        let mut env = BTreeMap::new();
        for (key, value) in &profile.default_env {
            env.insert((*key).to_owned(), resolve_placeholder(value, context)?);
        }

        Ok(Self {
            kind: profile.kind,
            program,
            args,
            env,
            cwd: context.cwd.clone(),
        })
    }

    fn to_command(&self) -> Command {
        let mut cmd = Command::new(&self.program);
        cmd.args(&self.args)
            .current_dir(&self.cwd)
            .envs(&self.env)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        cmd
    }
}

/// Lifecycle status of an external-agent process.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExternalAgentStatus {
    /// Spawned and believed running.
    Running,
    /// Exited with the given status code (`None` = killed by signal / unknown).
    Exited(Option<i32>),
    /// Spawn or runtime error.
    Failed(String),
}

/// Errors from resolving or launching an external agent.
#[derive(Debug, thiserror::Error)]
pub enum ExternalAgentLaunchError {
    #[error("profile requires a proxy url for placeholder `{0}` but none was provided")]
    MissingProxyUrl(String),
    #[error("failed to create managed home {path}: {source}")]
    ManagedHome {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to spawn external agent `{program}`: {source}")]
    Spawn {
        program: String,
        #[source]
        source: std::io::Error,
    },
}

/// A running external-agent process plus its resolved spec and stdio. The
/// handle owns the [`Child`]; dropping it kills the process (`kill_on_drop`).
pub struct ExternalAgentHandle {
    pub spec: ExternalAgentSpec,
    child: Child,
    /// Captured stdin pipe — an ACP transport writes JSON-RPC requests here.
    pub stdin: Option<tokio::process::ChildStdin>,
    /// Captured stdout pipe — an ACP transport reads JSON-RPC responses here.
    pub stdout: Option<tokio::process::ChildStdout>,
}

impl ExternalAgentHandle {
    /// Poll whether the process has exited without blocking, updating status.
    pub fn try_status(&mut self) -> ExternalAgentStatus {
        match self.child.try_wait() {
            Ok(Some(status)) => ExternalAgentStatus::Exited(status.code()),
            Ok(None) => ExternalAgentStatus::Running,
            Err(e) => ExternalAgentStatus::Failed(e.to_string()),
        }
    }

    /// Wait for the process to exit and return its terminal status. Reaps the
    /// child so it does not leak as a zombie.
    pub async fn wait(&mut self) -> ExternalAgentStatus {
        match self.child.wait().await {
            Ok(status) => ExternalAgentStatus::Exited(status.code()),
            Err(e) => ExternalAgentStatus::Failed(e.to_string()),
        }
    }

    /// Terminate the process and reap it. A `start_kill` error means the child
    /// already exited (nothing to signal); it is logged, not propagated, and we
    /// still reap via `wait` to surface the real terminal status.
    pub async fn terminate(&mut self) -> ExternalAgentStatus {
        if let Err(e) = self.child.start_kill() {
            tracing::debug!(
                target: "jfc::external_agent",
                error = %e,
                "start_kill failed (process likely already exited)"
            );
        }
        self.wait().await
    }
}

/// A managed external-agent session: resolves and spawns the process, drains
/// stderr to tracing, and tracks status. One session = one process lifetime.
pub struct ExternalAgentSession {
    spec: ExternalAgentSpec,
    status: Arc<Mutex<ExternalAgentStatus>>,
}

impl ExternalAgentSession {
    /// Resolve a profile into a spec without spawning. Useful for previewing
    /// the exact command or for tests.
    pub fn plan(
        profile: &ExternalAgentProfile,
        context: &LaunchContext,
    ) -> Result<ExternalAgentSpec, ExternalAgentLaunchError> {
        ExternalAgentSpec::resolve(profile, context)
    }

    /// Resolve, create the managed home, and spawn the agent. Returns the
    /// session (status holder) and a handle owning the child + stdio.
    pub async fn spawn(
        profile: &ExternalAgentProfile,
        context: &LaunchContext,
    ) -> Result<(Self, ExternalAgentHandle), ExternalAgentLaunchError> {
        let spec = ExternalAgentSpec::resolve(profile, context)?;
        Self::spawn_spec(spec).await
    }

    /// Spawn an already-resolved spec. Split out so tests can drive a concrete
    /// command without a profile.
    pub async fn spawn_spec(
        spec: ExternalAgentSpec,
    ) -> Result<(Self, ExternalAgentHandle), ExternalAgentLaunchError> {
        tokio::fs::create_dir_all(&spec.cwd)
            .await
            .map_err(|source| ExternalAgentLaunchError::ManagedHome {
                path: spec.cwd.clone(),
                source,
            })?;

        let mut child =
            spec.to_command()
                .spawn()
                .map_err(|source| ExternalAgentLaunchError::Spawn {
                    program: spec.program.clone(),
                    source,
                })?;

        // Drain stderr to tracing so handshake/crash output is visible under
        // RUST_LOG=jfc::external_agent=warn, mirroring the LSP client.
        if let Some(stderr) = child.stderr.take() {
            let kind = spec.kind;
            tokio::spawn(async move {
                let mut reader = BufReader::new(stderr).lines();
                while let Ok(Some(line)) = reader.next_line().await {
                    if !line.trim().is_empty() {
                        tracing::warn!(
                            target: "jfc::external_agent",
                            agent = ?kind,
                            stderr = %line,
                            "external agent stderr"
                        );
                    }
                }
            });
        }

        let stdin = child.stdin.take();
        let stdout = child.stdout.take();
        let session = Self {
            spec: spec.clone(),
            status: Arc::new(Mutex::new(ExternalAgentStatus::Running)),
        };
        let handle = ExternalAgentHandle {
            spec,
            child,
            stdin,
            stdout,
        };
        Ok((session, handle))
    }

    pub fn spec(&self) -> &ExternalAgentSpec {
        &self.spec
    }

    pub async fn status(&self) -> ExternalAgentStatus {
        self.status.lock().await.clone()
    }

    /// Record a terminal status on the session (called after wait/terminate).
    pub async fn set_status(&self, status: ExternalAgentStatus) {
        *self.status.lock().await = status;
    }
}

/// Resolve a single placeholder token against the launch context. Non-
/// placeholder strings pass through unchanged.
fn resolve_placeholder(
    raw: &str,
    context: &LaunchContext,
) -> Result<String, ExternalAgentLaunchError> {
    // Handle `key=<placeholder>` forms (e.g. `--auth=<empty>`) by resolving
    // only the value side.
    if let Some((key, value)) = raw.split_once('=')
        && value.starts_with('<')
        && value.ends_with('>')
    {
        let resolved = resolve_token(value, context)?;
        return Ok(format!("{key}={resolved}"));
    }
    if raw.starts_with('<') && raw.ends_with('>') {
        return resolve_token(raw, context);
    }
    Ok(raw.to_owned())
}

fn resolve_token(token: &str, context: &LaunchContext) -> Result<String, ExternalAgentLaunchError> {
    let inner = token.trim_start_matches('<').trim_end_matches('>');
    if inner == "empty" {
        return Ok(String::new());
    }
    if inner.ends_with("-home") {
        return Ok(context.managed_home.display().to_string());
    }
    if inner.ends_with("-proxy-url") || inner.ends_with("api-proxy-url") {
        return context
            .proxy_url
            .clone()
            .ok_or_else(|| ExternalAgentLaunchError::MissingProxyUrl(token.to_owned()));
    }
    // Unknown placeholder: pass through verbatim so the agent surfaces its own
    // error rather than us guessing.
    Ok(token.to_owned())
}

/// Resolve the executable to spawn. A `JFC_<AGENT>_PATH`-style override could
/// hook in here later; for now the profile's hint is used directly.
fn resolve_binary(hint: &str) -> String {
    hint.to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx() -> LaunchContext {
        LaunchContext {
            cwd: std::env::temp_dir(),
            managed_home: PathBuf::from("/tmp/jfc-junie-home"),
            proxy_url: Some("http://127.0.0.1:9999".to_owned()),
        }
    }

    // Normal: the Junie profile resolves its placeholders (managed home, proxy
    // url, empty auth) into a concrete spec.
    #[test]
    fn resolve_junie_substitutes_placeholders_normal() {
        let profile = ExternalAgentProfile::junie_from_air();
        let spec = ExternalAgentSpec::resolve(&profile, &ctx()).unwrap();
        assert_eq!(spec.program, "junie");
        assert!(spec.args.contains(&"--acp=true".to_owned()));
        // `--auth=<empty>` resolves the value side to empty.
        assert!(spec.args.contains(&"--auth=".to_owned()));
        assert_eq!(
            spec.env.get("JUNIE_HOME").map(String::as_str),
            Some("/tmp/jfc-junie-home")
        );
        assert_eq!(
            spec.env.get("INGRAZZIO_URL").map(String::as_str),
            Some("http://127.0.0.1:9999")
        );
    }

    // Robust: a profile that needs a proxy url errors clearly when none is
    // provided, instead of spawning with a broken `<...>` literal.
    #[test]
    fn resolve_errors_without_required_proxy_url_robust() {
        let profile = ExternalAgentProfile::junie_from_air();
        let context = LaunchContext {
            proxy_url: None,
            ..ctx()
        };
        let err = ExternalAgentSpec::resolve(&profile, &context).unwrap_err();
        assert!(matches!(err, ExternalAgentLaunchError::MissingProxyUrl(_)));
    }

    // Normal: spawning a real trivial process tracks Running then Exited(0),
    // proving the session manager observes actual lifecycle transitions.
    #[tokio::test]
    async fn spawn_tracks_real_process_lifecycle_normal() {
        let spec = ExternalAgentSpec {
            kind: ExternalAgentKind::GenericAcp,
            program: "true".to_owned(),
            args: vec![],
            env: BTreeMap::new(),
            cwd: std::env::temp_dir(),
        };
        let (session, mut handle) = ExternalAgentSession::spawn_spec(spec).await.unwrap();
        let status = handle.wait().await;
        assert_eq!(status, ExternalAgentStatus::Exited(Some(0)));
        session.set_status(status.clone()).await;
        assert_eq!(session.status().await, status);
    }

    // Robust: spawning a nonexistent binary surfaces a Spawn error rather than
    // a phantom Running session.
    #[tokio::test]
    async fn spawn_missing_binary_errors_robust() {
        let spec = ExternalAgentSpec {
            kind: ExternalAgentKind::GenericAcp,
            program: "jfc-no-such-binary-xyzzy".to_owned(),
            args: vec![],
            env: BTreeMap::new(),
            cwd: std::env::temp_dir(),
        };
        // Don't .unwrap_err(): the Ok variant (session, handle) isn't Debug
        // (it owns a Child + pipes), so match instead.
        match ExternalAgentSession::spawn_spec(spec).await {
            Err(ExternalAgentLaunchError::Spawn { .. }) => {}
            Err(other) => panic!("expected Spawn error, got {other:?}"),
            Ok(_) => panic!("expected spawn of nonexistent binary to fail"),
        }
    }

    // Robust: terminate() kills a long-running process and reaps it.
    #[tokio::test]
    async fn terminate_kills_long_running_process_robust() {
        let spec = ExternalAgentSpec {
            kind: ExternalAgentKind::GenericAcp,
            program: "sleep".to_owned(),
            args: vec!["30".to_owned()],
            env: BTreeMap::new(),
            cwd: std::env::temp_dir(),
        };
        let (_session, mut handle) = ExternalAgentSession::spawn_spec(spec).await.unwrap();
        assert_eq!(handle.try_status(), ExternalAgentStatus::Running);
        let status = handle.terminate().await;
        // Killed by signal → no exit code.
        assert_eq!(status, ExternalAgentStatus::Exited(None));
    }
}

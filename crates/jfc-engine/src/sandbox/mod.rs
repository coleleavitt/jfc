//! Process sandboxing for economy solver agents and Bash tool isolation.
//!
//! Provides path restriction and network blocking for child processes.
//! Gracefully degrades when kernel support is unavailable.
//!
//! ## Bash sandbox (bwrap)
//!
//! Mirrors Claude Code v2.1.142+'s sandbox configuration:
//! - Linux: Uses bubblewrap (bwrap) for filesystem + network isolation
//! - Configurable allowed/denied domains via `BashSandboxConfig`
//! - Falls back gracefully when bwrap is unavailable
//! - Auto-approves Bash tool calls when sandbox is active

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

#[cfg(target_os = "linux")]
pub mod landlock;

#[cfg(not(target_os = "linux"))]
pub mod landlock {
    //! Stub for non-Linux platforms.

    pub use super::fallback::*;
}

pub mod egress;
#[cfg(not(target_os = "linux"))]
mod fallback;

pub use egress::{EgressDecision, EgressPolicy};
pub use landlock::{SandboxPolicy, SandboxResult};

/// Global flag: set to `true` once the landlock sandbox has been
/// successfully initialized for this process. Read by the permission
/// system to auto-approve tools when sandboxed.
static SANDBOX_ACTIVE: AtomicBool = AtomicBool::new(false);

/// Mark the sandbox as active. Call this after successful landlock
/// initialization at process startup.
pub fn mark_sandbox_active() {
    let _linkscope_active = linkscope::phase("engine.sandbox.mark_active");
    SANDBOX_ACTIVE.store(true, Ordering::Relaxed);
}

/// Returns `true` if the landlock sandbox was successfully initialized
/// for this process. Used by the permission system to skip approval
/// prompts when execution is already containerized.
pub fn is_sandbox_active() -> bool {
    SANDBOX_ACTIVE.load(Ordering::Relaxed)
}

/// Configuration for Bash tool sandbox (bwrap-based network isolation).
/// Mirrors Claude Code's `sandbox.network` and `sandbox.filesystem` settings.
#[derive(Debug, Clone, Default)]
pub struct BashSandboxConfig {
    /// Whether sandbox is enabled for Bash commands.
    pub enabled: bool,
    /// Whether to fail the command if bwrap is unavailable (vs graceful fallback).
    pub fail_if_unavailable: bool,
    /// Auto-approve all Bash tool calls when sandboxed.
    pub auto_allow_bash_if_sandboxed: bool,
    /// Optional explicit path to the bwrap binary.
    pub bwrap_path: Option<String>,
    /// Network isolation settings.
    pub network: NetworkSandboxConfig,
    /// Filesystem isolation settings.
    pub filesystem: FilesystemSandboxConfig,
}

/// Network isolation for sandboxed Bash commands.
#[derive(Debug, Clone, Default)]
pub struct NetworkSandboxConfig {
    /// Domains explicitly allowed through the sandbox.
    pub allowed_domains: Vec<String>,
    /// Domains explicitly blocked (takes precedence over allowed).
    pub denied_domains: Vec<String>,
    /// When true, only managed (admin-configured) domains are allowed.
    pub allow_managed_domains_only: bool,
    /// Local HTTP egress proxy port that enforces the per-domain allowlist.
    /// When set (with `socks_proxy_port`), it is the *only* signal that host
    /// networking may be kept for allowlist mode (CS-JFC-003).
    pub http_proxy_port: Option<u16>,
    /// Local SOCKS egress proxy port that enforces the allowlist.
    pub socks_proxy_port: Option<u16>,
}

impl NetworkSandboxConfig {
    /// Whether an egress proxy is configured to enforce the domain allowlist.
    ///
    /// Without an enforcing proxy, a non-empty allowlist must NOT keep host
    /// networking — bwrap would otherwise grant full unrestricted egress while
    /// the `EgressPolicy` is never consulted per-connection.
    pub fn has_egress_proxy(&self) -> bool {
        self.http_proxy_port.is_some() || self.socks_proxy_port.is_some()
    }
}

/// Filesystem isolation for sandboxed Bash commands.
#[derive(Debug, Clone, Default)]
pub struct FilesystemSandboxConfig {
    /// Paths explicitly allowed for writing in addition to the current cwd.
    pub allow_write: Vec<String>,
    /// Paths denied for reading.
    pub deny_read: Vec<String>,
    /// Paths denied for writing.
    pub deny_write: Vec<String>,
    /// Paths explicitly allowed for reading (when managed-only mode is on).
    pub allow_read: Vec<String>,
    /// When true, only managed (admin-configured) read paths are allowed.
    pub allow_managed_read_paths_only: bool,
}

/// Check if bwrap (bubblewrap) is available on this system.
pub fn is_bwrap_available() -> bool {
    std::process::Command::new("bwrap")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Path to the bwrap binary, checking standard locations.
pub fn find_bwrap() -> Option<String> {
    if let Ok(output) = std::process::Command::new("which").arg("bwrap").output()
        && output.status.success()
    {
        return Some(String::from_utf8_lossy(&output.stdout).trim().to_string());
    }
    None
}

use std::sync::RwLock;

/// Global active sandbox config — set by the event loop when the user
/// toggles via `/sandbox`. Read by bash execution to decide whether to
/// wrap the command in bwrap.
static ACTIVE_BASH_SANDBOX: RwLock<Option<BashSandboxConfig>> = RwLock::new(None);

/// Install the current bash sandbox config (called when `/sandbox` toggles).
pub fn install_bash_sandbox_config(cfg: BashSandboxConfig) {
    let _linkscope_install = linkscope::phase("engine.sandbox.install_bash_config");
    linkscope::event_fields(
        "engine.sandbox.install_bash_config",
        [
            linkscope::TraceField::count("enabled", u64::from(cfg.enabled)),
            linkscope::TraceField::count(
                "allow_write",
                u64::try_from(cfg.filesystem.allow_write.len()).unwrap_or(u64::MAX),
            ),
            linkscope::TraceField::count(
                "allow_domains",
                u64::try_from(cfg.network.allowed_domains.len()).unwrap_or(u64::MAX),
            ),
        ],
    );
    if let Ok(mut guard) = ACTIVE_BASH_SANDBOX.write() {
        *guard = Some(cfg);
    }
}

/// Get a snapshot of the active bash sandbox config.
pub fn active_bash_sandbox_config() -> Option<BashSandboxConfig> {
    let _linkscope_active = linkscope::phase("engine.sandbox.active_bash_config");
    ACTIVE_BASH_SANDBOX.read().ok().and_then(|g| g.clone())
}

/// Clear the process-global sandbox config back to "none installed".
///
/// `install_bash_sandbox_config` mutates a process-global that bash execution
/// reads, so a test exercising `/sandbox` (e.g. the slash-registry drift test)
/// otherwise leaves the sandbox *enabled* for every subsequent bash test in
/// the same process — they then try to `bwrap` against a non-existent cwd and
/// fail with `execvp bash`. Tests that touch or depend on the sandbox state
/// call this (under `#[serial]`) to restore a deterministic baseline.
/// Test-support only — un-gated so downstream crates' suites can call it.
#[doc(hidden)]
pub fn reset_active_bash_sandbox_for_test() {
    let _linkscope_reset = linkscope::phase("engine.sandbox.reset_active_bash_config_for_test");
    if let Ok(mut guard) = ACTIVE_BASH_SANDBOX.write() {
        *guard = None;
    }
}

/// Build the bwrap argv prefix for wrapping a bash command.
///
/// Returns `None` when sandboxing is disabled or bwrap is unavailable.
/// Returns `Some(argv)` where `argv` is the full bwrap command to prepend
/// before `["bash", "-c", command]`.
pub fn build_bwrap_argv(cfg: &BashSandboxConfig, cwd: &std::path::Path) -> Option<Vec<String>> {
    let _linkscope_build = linkscope::phase("engine.sandbox.build_bwrap_argv");
    linkscope::event_fields(
        "engine.sandbox.build_bwrap_argv",
        [
            linkscope::TraceField::count("enabled", u64::from(cfg.enabled)),
            linkscope::TraceField::text("cwd", cwd.display().to_string()),
            linkscope::TraceField::count(
                "allow_write",
                u64::try_from(cfg.filesystem.allow_write.len()).unwrap_or(u64::MAX),
            ),
            linkscope::TraceField::count(
                "deny_read",
                u64::try_from(cfg.filesystem.deny_read.len()).unwrap_or(u64::MAX),
            ),
        ],
    );
    if !cfg.enabled {
        linkscope::event_fields(
            "engine.sandbox.build_bwrap_argv.result",
            [linkscope::TraceField::text("status", "disabled")],
        );
        return None;
    }
    let bwrap_path = cfg.bwrap_path.clone().or_else(find_bwrap)?;
    let mut argv = vec![
        bwrap_path,
        // Read-only bind the entire root so most commands can run normally.
        "--ro-bind".into(),
        "/usr".into(),
        "/usr".into(),
        "--ro-bind".into(),
        "/etc".into(),
        "/etc".into(),
        "--ro-bind".into(),
        "/bin".into(),
        "/bin".into(),
        "--ro-bind".into(),
        "/lib".into(),
        "/lib".into(),
        "--ro-bind-try".into(),
        "/lib64".into(),
        "/lib64".into(),
        // Standard sandbox setup.
        "--proc".into(),
        "/proc".into(),
        "--dev".into(),
        "/dev".into(),
        "--tmpfs".into(),
        "/tmp".into(),
        // Bind the cwd read-write so the agent can edit project files.
        "--bind".into(),
        cwd.display().to_string(),
        cwd.display().to_string(),
        "--chdir".into(),
        cwd.display().to_string(),
    ];
    for path in &cfg.filesystem.allow_read {
        argv.push("--ro-bind-try".into());
        argv.push(path.clone());
        argv.push(path.clone());
    }
    for path in &cfg.filesystem.allow_write {
        argv.push("--bind-try".into());
        argv.push(path.clone());
        argv.push(path.clone());
    }
    for path in &cfg.filesystem.deny_write {
        argv.push("--ro-bind-try".into());
        argv.push(path.clone());
        argv.push(path.clone());
    }
    // Network (CS-JFC-003): fail closed. The command gets no network at all
    // UNLESS outbound is enabled (a non-empty allowlist) AND an egress proxy is
    // configured to actually enforce the per-domain allowlist. Previously an
    // allowlist alone kept full host networking while nothing applied
    // [`egress::EgressPolicy`] per-connection — i.e. an allowlist silently
    // granted unrestricted egress. We only keep host networking when a proxy
    // port is present, and we then route traffic through it via proxy env vars
    // so the allowlist is genuinely enforced.
    let egress = egress::EgressPolicy::from_network_config(&cfg.network);
    let proxy_enforced = egress.outbound_enabled && cfg.network.has_egress_proxy();
    if proxy_enforced {
        if let Some(port) = cfg.network.http_proxy_port {
            let http = format!("http://127.0.0.1:{port}");
            for var in ["HTTP_PROXY", "http_proxy", "HTTPS_PROXY", "https_proxy"] {
                argv.push("--setenv".into());
                argv.push(var.into());
                argv.push(http.clone());
            }
        }
        if let Some(port) = cfg.network.socks_proxy_port {
            let socks = format!("socks5://127.0.0.1:{port}");
            for var in ["ALL_PROXY", "all_proxy"] {
                argv.push("--setenv".into());
                argv.push(var.into());
                argv.push(socks.clone());
            }
        }
    } else {
        argv.push("--unshare-net".into());
    }
    // Apply deny-read paths.
    for path in &cfg.filesystem.deny_read {
        argv.push("--tmpfs".into());
        argv.push(path.clone());
    }
    linkscope::event_fields(
        "engine.sandbox.build_bwrap_argv.result",
        [
            linkscope::TraceField::text(
                "network",
                if proxy_enforced {
                    "proxy_enforced"
                } else {
                    "unshare_net"
                },
            ),
            linkscope::TraceField::count("argv", u64::try_from(argv.len()).unwrap_or(u64::MAX)),
        ],
    );
    Some(argv)
}

/// Convert Claude/JFC persisted sandbox settings into the runtime Bash
/// sandbox config used by command execution.
pub fn bash_sandbox_config_from_settings(
    settings: &crate::config::SandboxConfig,
) -> BashSandboxConfig {
    let _linkscope_config = linkscope::phase("engine.sandbox.config_from_settings");
    BashSandboxConfig {
        enabled: settings.enabled.unwrap_or(true),
        fail_if_unavailable: settings.fail_if_unavailable.unwrap_or(false),
        auto_allow_bash_if_sandboxed: settings.auto_allow_bash_if_sandboxed.unwrap_or(true),
        bwrap_path: settings.bwrap_path.clone(),
        network: NetworkSandboxConfig {
            allowed_domains: settings.network.allowed_domains.clone(),
            denied_domains: settings.network.denied_domains.clone(),
            allow_managed_domains_only: settings
                .network
                .allow_managed_domains_only
                .unwrap_or(false),
            http_proxy_port: settings.network.http_proxy_port,
            socks_proxy_port: settings.network.socks_proxy_port,
        },
        filesystem: FilesystemSandboxConfig {
            allow_write: settings.filesystem.allow_write.clone(),
            deny_read: settings.filesystem.deny_read.clone(),
            deny_write: settings.filesystem.deny_write.clone(),
            allow_read: settings.filesystem.allow_read.clone(),
            allow_managed_read_paths_only: settings
                .filesystem
                .allow_managed_read_paths_only
                .unwrap_or(false),
        },
    }
}

pub fn rsi_external_worker_sandbox(cwd: &Path) -> jfc_learn::RsiSandboxEnforcement {
    let _linkscope_rsi = linkscope::phase("engine.sandbox.rsi_external_worker");
    let cfg = BashSandboxConfig {
        enabled: true,
        fail_if_unavailable: true,
        auto_allow_bash_if_sandboxed: true,
        bwrap_path: None,
        network: NetworkSandboxConfig::default(),
        filesystem: FilesystemSandboxConfig::default(),
    };
    let argv = build_bwrap_argv(&cfg, cwd);
    let bwrap_available = argv.is_some();
    let unshare_net = argv
        .as_ref()
        .is_some_and(|args| args.iter().any(|arg| arg == "--unshare-net"));
    jfc_learn::RsiSandboxEnforcement::bubblewrap_worker(
        &jfc_learn::RsiLoopSandboxPlan::default(),
        bwrap_available,
        unshare_net,
    )
}

pub fn rsi_curator_worker_config(cwd: &Path) -> Option<jfc_learn::RsiCuratorWorkerConfig> {
    let _linkscope_rsi = linkscope::phase("engine.sandbox.rsi_curator_worker_config");
    let explicit = std::env::var_os("JFC_RSI_WORKER_BIN").map(PathBuf::from);
    if cfg!(test) && explicit.is_none() {
        return None;
    }
    let binary = explicit.or_else(|| std::env::current_exe().ok())?;
    Some(jfc_learn::RsiCuratorWorkerConfig {
        binary,
        cwd: cwd.to_path_buf(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn allowlist_cfg(proxy_port: Option<u16>) -> BashSandboxConfig {
        BashSandboxConfig {
            enabled: true,
            fail_if_unavailable: false,
            auto_allow_bash_if_sandboxed: true,
            // Force a bwrap path so the test doesn't depend on bwrap install.
            bwrap_path: Some("/usr/bin/bwrap".into()),
            network: NetworkSandboxConfig {
                allowed_domains: vec!["example.com".into()],
                denied_domains: vec![],
                allow_managed_domains_only: false,
                http_proxy_port: proxy_port,
                socks_proxy_port: None,
            },
            filesystem: FilesystemSandboxConfig::default(),
        }
    }

    // CS-JFC-003: a non-empty allowlist with NO enforcing egress proxy must
    // still unshare the network (fail closed), not silently keep host networking.
    #[test]
    fn allowlist_without_proxy_unshares_net_regression() {
        let temp = tempfile::tempdir().expect("temp dir");
        let argv = build_bwrap_argv(&allowlist_cfg(None), temp.path()).expect("argv");
        assert!(
            argv.iter().any(|a| a == "--unshare-net"),
            "allowlist without proxy must fail closed with --unshare-net: {argv:?}"
        );
    }

    // With an enforcing proxy configured, host networking is kept and traffic is
    // routed through the proxy so the allowlist is actually applied.
    #[test]
    fn allowlist_with_proxy_keeps_net_and_sets_proxy_env_normal() {
        let temp = tempfile::tempdir().expect("temp dir");
        let argv = build_bwrap_argv(&allowlist_cfg(Some(8888)), temp.path()).expect("argv");
        assert!(
            !argv.iter().any(|a| a == "--unshare-net"),
            "proxy mode should keep networking: {argv:?}"
        );
        assert!(argv.iter().any(|a| a == "HTTP_PROXY"));
        assert!(argv.iter().any(|a| a == "http://127.0.0.1:8888"));
    }

    #[test]
    fn rsi_external_worker_sandbox_uses_bwrap_unshare_net_when_available_normal() {
        let temp = tempfile::tempdir().expect("temp dir");
        let sandbox = rsi_external_worker_sandbox(temp.path());

        assert_eq!(sandbox.kernel_backend, "bubblewrap_unshare_net");
        if find_bwrap().is_some() {
            assert_eq!(
                sandbox.status,
                jfc_learn::RsiSandboxEnforcementStatus::KernelEnforced
            );
            assert!(sandbox.egress_isolated);
        } else {
            assert_eq!(
                sandbox.status,
                jfc_learn::RsiSandboxEnforcementStatus::Blocked
            );
            assert!(sandbox.reasons.contains(&"bubblewrap_unavailable"));
        }
    }
}

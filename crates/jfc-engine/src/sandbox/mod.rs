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
    if let Ok(mut guard) = ACTIVE_BASH_SANDBOX.write() {
        *guard = Some(cfg);
    }
}

/// Get a snapshot of the active bash sandbox config.
pub fn active_bash_sandbox_config() -> Option<BashSandboxConfig> {
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
    if !cfg.enabled {
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
    // Network: resolve the egress policy. When outbound is disabled (no
    // allowlisted domains) the network namespace is unshared so the command
    // gets no network at all. When an allowlist is present, bwrap keeps the
    // network and a host-level egress proxy/guard enforces the per-domain
    // [`egress::EgressPolicy`] (wildcards, deny-precedence, default-deny).
    let egress = egress::EgressPolicy::from_network_config(&cfg.network);
    if !egress.outbound_enabled {
        argv.push("--unshare-net".into());
    }
    // Apply deny-read paths.
    for path in &cfg.filesystem.deny_read {
        argv.push("--tmpfs".into());
        argv.push(path.clone());
    }
    Some(argv)
}

/// Convert Claude/JFC persisted sandbox settings into the runtime Bash
/// sandbox config used by command execution.
pub fn bash_sandbox_config_from_settings(
    settings: &crate::config::SandboxConfig,
) -> BashSandboxConfig {
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

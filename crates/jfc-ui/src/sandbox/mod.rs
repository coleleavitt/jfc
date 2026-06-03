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

#[cfg(not(target_os = "linux"))]
mod fallback;
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
#[cfg(test)]
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
    let bwrap_path = find_bwrap()?;
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
    // Network: by default, sandboxed commands have no network. To allow
    // it, the user must explicitly add allowed_domains (not implemented
    // at the bwrap level — needs a proxy). For now, sandbox = no network.
    if cfg.network.allowed_domains.is_empty() {
        argv.push("--unshare-net".into());
    }
    // Apply deny-read paths.
    for path in &cfg.filesystem.deny_read {
        argv.push("--tmpfs".into());
        argv.push(path.clone());
    }
    Some(argv)
}

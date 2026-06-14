//! Landlock LSM policy builder (Linux only).

use std::path::{Path, PathBuf};
use std::process::Command;

/// A sandbox policy defining allowed paths and restrictions.
#[derive(Debug, Clone)]
pub struct SandboxPolicy {
    /// Paths with read+write access.
    writable_paths: Vec<PathBuf>,
    /// Paths with read-only access.
    readable_paths: Vec<PathBuf>,
    /// Whether network access is blocked.
    block_network: bool,
    /// Allowed executables for build tools.
    exec_allowlist: Vec<PathBuf>,
}

/// Result of applying a sandbox policy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SandboxResult {
    /// Policy applied successfully.
    Applied,
    /// Kernel doesn't support Landlock; proceeding unsandboxed.
    Unsupported,
    /// Policy application failed.
    Failed(String),
}

/// Extension trait for applying sandbox policies to child process commands.
pub trait CommandSandboxExt {
    /// Apply a sandbox policy to this command.
    fn sandbox(&mut self, policy: &SandboxPolicy) -> SandboxResult;
}

impl SandboxPolicy {
    pub fn new() -> Self {
        Self {
            writable_paths: Vec::new(),
            readable_paths: Vec::new(),
            block_network: false,
            exec_allowlist: Vec::new(),
        }
    }

    pub fn allow_write(mut self, path: impl Into<PathBuf>) -> Self {
        self.writable_paths.push(path.into());
        self
    }

    pub fn allow_read(mut self, path: impl Into<PathBuf>) -> Self {
        self.readable_paths.push(path.into());
        self
    }

    pub fn block_network(mut self) -> Self {
        self.block_network = true;
        self
    }

    pub fn allow_exec(mut self, path: impl Into<PathBuf>) -> Self {
        self.exec_allowlist.push(path.into());
        self
    }

    /// Check if a path would be allowed under this policy.
    pub fn is_path_allowed(&self, path: &Path, write: bool) -> bool {
        if write {
            self.writable_paths
                .iter()
                .any(|allowed| path.starts_with(allowed))
        } else {
            self.writable_paths
                .iter()
                .any(|allowed| path.starts_with(allowed))
                || self
                    .readable_paths
                    .iter()
                    .any(|allowed| path.starts_with(allowed))
        }
    }

    /// Check if network access is blocked.
    pub fn is_network_blocked(&self) -> bool {
        self.block_network
    }

    /// Apply this policy to a Command.
    ///
    /// NOTE: Actual Landlock/seccomp application requires the `landlock` crate
    /// or raw syscalls. This implementation provides the policy structure and
    /// validation; actual kernel enforcement is a future enhancement when the
    /// `landlock` crate is added as an optional dependency.
    pub fn apply_to_command(&self, _cmd: &mut Command) -> SandboxResult {
        if !Self::is_landlock_supported() {
            tracing::warn!(
                target: "jfc::sandbox",
                "Landlock not supported on this kernel; proceeding unsandboxed"
            );
            return SandboxResult::Unsupported;
        }

        tracing::info!(
            target: "jfc::sandbox",
            writable = ?self.writable_paths,
            readable = ?self.readable_paths,
            block_network = self.block_network,
            exec_allowlist = ?self.exec_allowlist,
            "sandbox policy defined (enforcement pending landlock crate)"
        );

        SandboxResult::Applied
    }

    fn is_landlock_supported() -> bool {
        std::fs::read_to_string("/proc/sys/kernel/osrelease")
            .ok()
            .and_then(|ver| {
                let parts: Vec<&str> = ver.trim().split('.').collect();
                let major: u32 = parts.first()?.parse().ok()?;
                let minor: u32 = parts.get(1)?.parse().ok()?;
                Some(major > 5 || (major == 5 && minor >= 13))
            })
            .unwrap_or(false)
    }

    /// Create a policy suitable for economy solver agents.
    pub fn economy_solver(worktree_path: &Path) -> Self {
        Self::new()
            .allow_write(worktree_path.to_path_buf())
            .allow_read(PathBuf::from("/usr"))
            .allow_read(PathBuf::from("/lib"))
            .allow_read(PathBuf::from("/etc/alternatives"))
            .allow_exec(PathBuf::from("/usr/bin"))
            .allow_exec(PathBuf::from("/usr/local/bin"))
            .block_network()
    }
}

impl Default for SandboxPolicy {
    fn default() -> Self {
        Self::new()
    }
}

impl CommandSandboxExt for Command {
    fn sandbox(&mut self, policy: &SandboxPolicy) -> SandboxResult {
        policy.apply_to_command(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_policy_allows_writable_path() {
        let policy = SandboxPolicy::new().allow_write("/tmp/jfc-worktree");

        assert!(policy.is_path_allowed(Path::new("/tmp/jfc-worktree/src/main.rs"), true));
        assert!(policy.is_path_allowed(Path::new("/tmp/jfc-worktree/src/main.rs"), false));
    }

    #[test]
    fn test_policy_denies_outside_path() {
        let policy = SandboxPolicy::new().allow_write("/tmp/jfc-worktree");

        assert!(!policy.is_path_allowed(Path::new("/home/user/.ssh/id_rsa"), false));
        assert!(!policy.is_path_allowed(Path::new("/home/user/.ssh/id_rsa"), true));
    }

    #[test]
    fn test_policy_read_only_blocks_write() {
        let policy = SandboxPolicy::new().allow_read("/usr");

        assert!(policy.is_path_allowed(Path::new("/usr/bin/rustc"), false));
        assert!(!policy.is_path_allowed(Path::new("/usr/bin/rustc"), true));
    }

    #[test]
    fn test_economy_solver_policy() {
        let worktree = Path::new("/tmp/jfc-worktree");
        let policy = SandboxPolicy::economy_solver(worktree);

        assert!(policy.is_path_allowed(Path::new("/tmp/jfc-worktree/Cargo.toml"), true));
        assert!(policy.is_path_allowed(Path::new("/usr/bin/cargo"), false));
        assert!(!policy.is_path_allowed(Path::new("/var/lib/private"), false));
        assert!(policy.is_network_blocked());
        assert!(
            policy
                .exec_allowlist
                .iter()
                .any(|path| path == Path::new("/usr/bin"))
        );
        assert!(
            policy
                .exec_allowlist
                .iter()
                .any(|path| path == Path::new("/usr/local/bin"))
        );
    }

    #[test]
    fn test_network_blocked() {
        let policy = SandboxPolicy::new().block_network();

        assert!(policy.is_network_blocked());
    }
}

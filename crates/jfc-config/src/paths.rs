//! Well-known `.claude/` directory paths mirroring CC 2.1.167.
//!
//! Centralises the path logic and diagnostics for:
//! - Agent memory (user / project / local tiers)
//! - Teams directory
//! - Worktrees directory
//! - Plans directory (configurable via `plansDirectory` setting)
//! - Remote credential paths (`~/.claude/remote/`)
//!
//! Path constructors are pure and do not panic. Diagnostics may read path
//! metadata, but remain non-destructive.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use crate::ClaudeCompatibilityConfig;

// ── Agent memory ─────────────────────────────────────────────────────────────

/// Which tier of agent memory to address.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentMemoryScope {
    /// `~/.claude/agent-memory/<namespace>/` — shared across all projects.
    User,
    /// `<project>/.claude/agent-memory/<namespace>/` — committed with the repo.
    Project,
    /// `<project>/.claude/agent-memory-local/<namespace>/` — machine-local, not committed.
    Local,
}

/// Resolve the agent-memory base directory for the given scope and namespace.
///
/// Respects `CLAUDE_CODE_REMOTE_MEMORY_DIR` for managed remote sessions:
/// when set, project-scoped paths are rooted there instead of `<project>/.claude/`.
pub fn agent_memory_path(project_root: &Path, scope: AgentMemoryScope, namespace: &str) -> PathBuf {
    match scope {
        AgentMemoryScope::User => dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("/home/claude"))
            .join(".claude")
            .join("agent-memory")
            .join(namespace),

        AgentMemoryScope::Project => {
            // CC checks CLAUDE_CODE_REMOTE_MEMORY_DIR first for managed remote envs.
            if let Ok(remote_dir) = std::env::var("CLAUDE_CODE_REMOTE_MEMORY_DIR") {
                let base = PathBuf::from(remote_dir);
                tracing::trace!(
                    target: "jfc::config::paths",
                    base = %base.display(),
                    "using CLAUDE_CODE_REMOTE_MEMORY_DIR for agent-memory"
                );
                return base.join("projects").join(namespace).join("agent-memory");
            }
            project_root
                .join(".claude")
                .join("agent-memory")
                .join(namespace)
        }

        AgentMemoryScope::Local => {
            if let Ok(remote_dir) = std::env::var("CLAUDE_CODE_REMOTE_MEMORY_DIR") {
                let base = PathBuf::from(remote_dir);
                return base
                    .join("projects")
                    .join(namespace)
                    .join("agent-memory-local");
            }
            project_root
                .join(".claude")
                .join("agent-memory-local")
                .join(namespace)
        }
    }
}

/// List namespace directories present across all three memory tiers.
///
/// Returns `(scope, path)` pairs — callers can filter by scope as needed.
/// Directories that don't exist are silently skipped.
pub fn list_agent_memory_namespaces(project_root: &Path) -> Vec<(AgentMemoryScope, PathBuf)> {
    let mut result = Vec::new();
    for (scope, base) in [
        (
            AgentMemoryScope::User,
            dirs::home_dir()
                .unwrap_or_else(|| PathBuf::from("/home/claude"))
                .join(".claude")
                .join("agent-memory"),
        ),
        (
            AgentMemoryScope::Project,
            project_root.join(".claude").join("agent-memory"),
        ),
        (
            AgentMemoryScope::Local,
            project_root.join(".claude").join("agent-memory-local"),
        ),
    ] {
        let Ok(entries) = std::fs::read_dir(&base) else {
            continue;
        };
        for entry in entries.flatten() {
            if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                result.push((scope, entry.path()));
            }
        }
    }
    result
}

// ── Teams / worktrees / plans ─────────────────────────────────────────────────

/// `<project>/.claude/teams/` — team session directory.
pub fn teams_dir(project_root: &Path) -> PathBuf {
    project_root.join(".claude").join("teams")
}

/// `<project>/.claude/worktrees/` — isolated git worktree checkouts.
pub fn worktrees_dir(project_root: &Path) -> PathBuf {
    project_root.join(".claude").join("worktrees")
}

/// Count the number of worktrees currently in `<project>/.claude/worktrees/`.
///
/// Returns 0 if the directory is absent. Adds 1 for the main checkout.
pub fn worktree_count(project_root: &Path) -> usize {
    let dir = worktrees_dir(project_root);
    let count = std::fs::read_dir(&dir)
        .map(|entries| entries.flatten().count())
        .unwrap_or(0);
    count + 1 // +1 for the main checkout
}

/// Resolve the plans directory.
///
/// Uses `settings.plans_directory` (relative to `project_root`) when set;
/// otherwise falls back to `~/.claude/plans/`.
pub fn plans_dir(project_root: &Path, settings: &ClaudeCompatibilityConfig) -> PathBuf {
    if let Some(ref rel) = settings.plans_directory {
        if !rel.trim().is_empty() {
            return project_root.join(rel.trim());
        }
    }
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("/home/claude"))
        .join(".claude")
        .join("plans")
}

// ── XDG path diagnostics ──────────────────────────────────────────────────────

/// Domain service for the non-destructive `jfc doctor paths` report.
#[derive(Debug, Default, Clone, Copy)]
pub struct XdgPathDiagnosticService;

impl XdgPathDiagnosticService {
    pub fn report(self) -> String {
        xdg_path_diagnostic_report()
    }
}

fn xdg_path_diagnostic_report() -> String {
    let home_dir = dirs::home_dir();
    let config_dir = dirs::config_dir();
    let data_dir = dirs::data_dir();
    let cache_dir = dirs::cache_dir();
    let legacy_keybindings = home_dir
        .as_ref()
        .map(|home| home.join(".claude").join("keybindings.json"))
        .unwrap_or_else(|| PathBuf::from("/nonexistent"));
    let xdg_keybindings = config_dir
        .as_ref()
        .map(|config| config.join("claude").join("keybindings.json"))
        .unwrap_or_else(|| PathBuf::from("/nonexistent"));

    xdg_path_diagnostic_report_in(XdgPathDiagnosticInputs {
        home_dir,
        config_dir,
        data_dir,
        cache_dir,
        legacy_keybindings_exists: legacy_keybindings.exists(),
        xdg_keybindings_exists: xdg_keybindings.exists(),
    })
}

struct XdgPathDiagnosticInputs {
    home_dir: Option<PathBuf>,
    config_dir: Option<PathBuf>,
    data_dir: Option<PathBuf>,
    cache_dir: Option<PathBuf>,
    legacy_keybindings_exists: bool,
    xdg_keybindings_exists: bool,
}

fn xdg_path_diagnostic_report_in(inputs: XdgPathDiagnosticInputs) -> String {
    let legacy_keybindings = inputs
        .home_dir
        .as_ref()
        .map(|home| home.join(".claude").join("keybindings.json"))
        .unwrap_or_else(|| PathBuf::from("/nonexistent"));
    let xdg_keybindings = inputs
        .config_dir
        .as_ref()
        .map(|config| config.join("claude").join("keybindings.json"))
        .unwrap_or_else(|| PathBuf::from("/nonexistent"));
    let sessions_legacy = inputs
        .config_dir
        .as_ref()
        .map(|config| config.join("jfc").join("sessions"))
        .unwrap_or_else(|| PathBuf::from("/nonexistent"));
    let sessions_modern = inputs
        .data_dir
        .as_ref()
        .map(|data| data.join("jfc").join("sessions"))
        .unwrap_or_else(|| PathBuf::from("/nonexistent"));
    let logs_cfg = inputs
        .config_dir
        .as_ref()
        .map(|config| config.join("jfc").join("logs"))
        .unwrap_or_else(|| PathBuf::from("/nonexistent"));
    let logs_cache = inputs
        .cache_dir
        .as_ref()
        .map(|cache| cache.join("jfc").join("logs"))
        .unwrap_or_else(|| PathBuf::from("/nonexistent"));

    let mut report = String::new();
    report.push_str("jfc doctor — XDG path diagnostics\n\n");

    if let Some(config) = &inputs.config_dir {
        let _ = writeln!(report, "• XDG_CONFIG_HOME: {}", config.display());
        let _ = writeln!(report, "  - jfc config dir: {}/jfc", config.display());
    } else {
        report.push_str("• XDG_CONFIG_HOME: (not set; dirs::config_dir() unavailable)\n");
    }

    if let Some(data) = &inputs.data_dir {
        let _ = writeln!(report, "• XDG_DATA_HOME:   {}", data.display());
    } else {
        report.push_str("• XDG_DATA_HOME:   (not set)\n");
    }

    if let Some(cache) = &inputs.cache_dir {
        let _ = writeln!(report, "• XDG_CACHE_HOME:  {}", cache.display());
    } else {
        report.push_str("• XDG_CACHE_HOME:  (not set)\n");
    }

    report.push_str("\nChecks:\n\n");

    if inputs.legacy_keybindings_exists && !inputs.xdg_keybindings_exists {
        let recommendation = xdg_keybindings
            .parent()
            .map(|path| path.display().to_string() + "/keybindings.json")
            .unwrap_or_else(|| "~/.config/claude/keybindings.json".to_string());
        let _ = writeln!(
            report,
            "- Legacy keybindings found at {}\n  Recommendation: move or copy to {}",
            legacy_keybindings.display(),
            recommendation
        );
    } else if inputs.xdg_keybindings_exists {
        let _ = writeln!(
            report,
            "- Keybindings present at modern XDG path: {}",
            xdg_keybindings.display()
        );
    } else {
        report.push_str("- No keybindings file found (optional feature)\n");
    }

    let _ = writeln!(
        report,
        "- Sessions directory (current): {}",
        sessions_legacy.display()
    );
    if !sessions_modern.as_os_str().is_empty() {
        let _ = writeln!(
            report,
            "  Suggested future location: {} (XDG_DATA_HOME)",
            sessions_modern.display()
        );
    }

    let _ = writeln!(report, "- Logs directory (current): {}", logs_cfg.display());
    if !logs_cache.as_os_str().is_empty() {
        let _ = writeln!(
            report,
            "  Suggested cache location: {} (XDG_CACHE_HOME)",
            logs_cache.display()
        );
    }

    report
}

// ── Remote credential paths ───────────────────────────────────────────────────

/// Paths to managed remote session credential files in `~/.claude/remote/`.
///
/// These files are read (never written) by JFC for remote/managed session
/// authentication. Path resolution only — no content is read here.
#[derive(Debug, Clone)]
pub struct RemotePaths {
    /// `~/.claude/remote/.oauth_token`
    pub oauth_token: PathBuf,
    /// `~/.claude/remote/.api_key`
    pub api_key: PathBuf,
    /// `~/.claude/remote/.session_ingress_token`
    pub session_ingress_token: PathBuf,
}

/// Resolve the remote credential paths under `~/.claude/remote/`.
pub fn remote_paths() -> RemotePaths {
    let base = dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("/home/claude"))
        .join(".claude")
        .join("remote");
    RemotePaths {
        oauth_token: base.join(".oauth_token"),
        api_key: base.join(".api_key"),
        session_ingress_token: base.join(".session_ingress_token"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ClaudeCompatibilityConfig;

    #[test]
    fn agent_memory_path_project_normal() {
        let root = Path::new("/tmp/myproject");
        let path = agent_memory_path(root, AgentMemoryScope::Project, "default");
        assert!(path.ends_with("agent-memory/default"));
        assert!(path.starts_with("/tmp/myproject/.claude"));
    }

    #[test]
    fn agent_memory_path_local_normal() {
        let root = Path::new("/tmp/myproject");
        let path = agent_memory_path(root, AgentMemoryScope::Local, "ns");
        assert!(path.to_string_lossy().contains("agent-memory-local"));
    }

    #[test]
    fn plans_dir_custom_path_normal() {
        let root = Path::new("/tmp/proj");
        let settings = ClaudeCompatibilityConfig {
            plans_directory: Some(".plans".to_owned()),
            ..Default::default()
        };
        let dir = plans_dir(root, &settings);
        assert_eq!(dir, Path::new("/tmp/proj/.plans"));
    }

    #[test]
    fn plans_dir_default_fallback_normal() {
        let root = Path::new("/tmp/proj");
        let settings = ClaudeCompatibilityConfig::default();
        let dir = plans_dir(root, &settings);
        assert!(dir.ends_with(".claude/plans"));
    }

    #[test]
    fn worktrees_dir_path_normal() {
        let root = Path::new("/tmp/proj");
        assert_eq!(
            worktrees_dir(root),
            Path::new("/tmp/proj/.claude/worktrees")
        );
    }

    #[test]
    fn teams_dir_path_normal() {
        let root = Path::new("/tmp/proj");
        assert_eq!(teams_dir(root), Path::new("/tmp/proj/.claude/teams"));
    }

    #[test]
    fn remote_paths_structure_normal() {
        let paths = remote_paths();
        assert!(paths.oauth_token.ends_with(".oauth_token"));
        assert!(paths.api_key.ends_with(".api_key"));
        assert!(
            paths
                .session_ingress_token
                .ends_with(".session_ingress_token")
        );
        // All three should be in the same remote/ directory.
        assert_eq!(paths.oauth_token.parent(), paths.api_key.parent());
    }

    #[test]
    fn xdg_path_diagnostic_report_no_keybindings_normal() {
        let report = xdg_path_diagnostic_report_in(XdgPathDiagnosticInputs {
            home_dir: Some(PathBuf::from("/home/tester")),
            config_dir: Some(PathBuf::from("/home/tester/.config")),
            data_dir: Some(PathBuf::from("/home/tester/.local/share")),
            cache_dir: Some(PathBuf::from("/home/tester/.cache")),
            legacy_keybindings_exists: false,
            xdg_keybindings_exists: false,
        });

        assert_eq!(
            report,
            concat!(
                "jfc doctor — XDG path diagnostics\n\n",
                "• XDG_CONFIG_HOME: /home/tester/.config\n",
                "  - jfc config dir: /home/tester/.config/jfc\n",
                "• XDG_DATA_HOME:   /home/tester/.local/share\n",
                "• XDG_CACHE_HOME:  /home/tester/.cache\n\n",
                "Checks:\n\n",
                "- No keybindings file found (optional feature)\n",
                "- Sessions directory (current): /home/tester/.config/jfc/sessions\n",
                "  Suggested future location: /home/tester/.local/share/jfc/sessions (XDG_DATA_HOME)\n",
                "- Logs directory (current): /home/tester/.config/jfc/logs\n",
                "  Suggested cache location: /home/tester/.cache/jfc/logs (XDG_CACHE_HOME)\n"
            )
        );
    }
}

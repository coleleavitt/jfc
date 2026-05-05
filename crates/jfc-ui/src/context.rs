#![allow(dead_code)]

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde::{Deserialize, Serialize};

/// Tracks file mtime+size so re-reads of unchanged files return a short
/// "file unchanged" stub instead of full content, saving tokens.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ReadDedupCache {
    entries: HashMap<PathBuf, ReadEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ReadEntry {
    mtime_secs: u64,
    len: u64,
}

impl ReadDedupCache {
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns `true` if path was previously read and disk mtime+size still match.
    pub fn is_unchanged(&self, path: &Path) -> bool {
        let Some(entry) = self.entries.get(path) else {
            return false;
        };
        match std::fs::metadata(path) {
            Ok(meta) => {
                let mtime = meta
                    .modified()
                    .ok()
                    .and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
                    .map(|d| d.as_secs())
                    .unwrap_or(0);
                let len = meta.len();
                entry.mtime_secs == mtime && entry.len == len
            }
            Err(_) => false,
        }
    }

    pub fn record_read(&mut self, path: PathBuf) {
        tracing::trace!(target: "jfc::context", path = %path.display(), "recording file read");
        if let Ok(meta) = std::fs::metadata(&path) {
            let mtime = meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
                .map(|d| d.as_secs())
                .unwrap_or(0);
            self.entries.insert(
                path,
                ReadEntry {
                    mtime_secs: mtime,
                    len: meta.len(),
                },
            );
        }
    }

    pub fn invalidate(&mut self, path: &Path) {
        tracing::debug!(target: "jfc::context", path = %path.display(), "invalidating cache entry");
        self.entries.remove(path);
    }

    pub fn clear(&mut self) {
        tracing::debug!(target: "jfc::context", entries = self.entries.len(), "clearing read cache");
        self.entries.clear();
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ToolContext {
    pub read_cache: ReadDedupCache,
    /// Approximate token count of the current conversation.
    pub approx_tokens: usize,
    /// Number of consecutive rapid compaction re-fills (circuit breaker state).
    pub rapid_refill_count: u32,
    /// Total user turns since session start (monotonically increasing).
    pub total_user_turns: u32,
    /// The `total_user_turns` value at which the last compaction occurred.
    pub last_compact_turn: u32,
}

impl ToolContext {
    pub fn new() -> Self {
        Self::default()
    }
}

/// Walk from `start` upward to filesystem root looking for CLAUDE.md.
/// Returns (path, content) of the first one found, or None.
pub fn find_claude_md(start: &Path) -> Option<(PathBuf, String)> {
    tracing::debug!(target: "jfc::context", start = %start.display(), "searching for CLAUDE.md");
    let mut dir = start.to_path_buf();
    loop {
        let candidate = dir.join("CLAUDE.md");
        if let Ok(content) = std::fs::read_to_string(&candidate) {
            if !content.trim().is_empty() {
                tracing::info!(
                    target: "jfc::context",
                    path = %candidate.display(),
                    size_bytes = content.len(),
                    "found CLAUDE.md"
                );
                return Some((candidate, content));
            }
        }
        match dir.parent() {
            Some(parent) if parent != dir => dir = parent.to_path_buf(),
            _ => break,
        }
    }
    tracing::debug!(target: "jfc::context", start = %start.display(), "CLAUDE.md not found");
    None
}

/// v126 CLAUDE.md hierarchy. Sources are loaded in precedence order; each one
/// that exists contributes its content as a labeled section in the system
/// prompt. Mirrors v126's merge-all behavior (all five layers can coexist).
#[derive(Debug, Clone, Default)]
pub struct ClaudeMdHierarchy {
    /// `~/.config/claude/CLAUDE.md` — enterprise/managed policy.
    pub managed: Option<(PathBuf, String)>,
    /// `~/.claude/CLAUDE.md` — personal preferences.
    pub user: Option<(PathBuf, String)>,
    /// `<project>/CLAUDE.md` — project-level instructions.
    pub project: Option<(PathBuf, String)>,
    /// `<project>/.claude/CLAUDE.md` — alternative project location.
    pub project_dot: Option<(PathBuf, String)>,
    /// `<project>/CLAUDE.local.md` — gitignored personal overrides.
    pub local: Option<(PathBuf, String)>,
}

impl ClaudeMdHierarchy {
    /// Load every CLAUDE.md layer that exists for the given project root.
    pub fn load(project_root: &Path) -> Self {
        tracing::info!(target: "jfc::context", project_root = %project_root.display(), "loading CLAUDE.md hierarchy");
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"));
        let cfg = dirs::config_dir().unwrap_or_else(|| home.join(".config"));
        let result = Self {
            managed: read_if_exists(&cfg.join("claude/CLAUDE.md")),
            user: read_if_exists(&home.join(".claude/CLAUDE.md")),
            project: read_if_exists(&project_root.join("CLAUDE.md")),
            project_dot: read_if_exists(&project_root.join(".claude/CLAUDE.md")),
            local: read_if_exists(&project_root.join("CLAUDE.local.md")),
        };
        tracing::debug!(
            target: "jfc::context",
            has_managed = result.managed.is_some(),
            has_user = result.user.is_some(),
            has_project = result.project.is_some(),
            has_project_dot = result.project_dot.is_some(),
            has_local = result.local.is_some(),
            "CLAUDE.md hierarchy loaded"
        );
        result
    }

    /// Concatenate all layers into a single system-prompt-ready string with
    /// labeled section headers so the model can tell where each rule came
    /// from. Returns `None` when nothing was found.
    pub fn render(&self) -> Option<String> {
        let mut out = String::new();
        let mut push = |label: &str, layer: &Option<(PathBuf, String)>| {
            if let Some((path, content)) = layer {
                if !content.trim().is_empty() {
                    if !out.is_empty() {
                        out.push_str("\n\n");
                    }
                    out.push_str(&format!(
                        "# {label} ({})\n\n{}",
                        path.display(),
                        content.trim()
                    ));
                }
            }
        };
        push("Managed policy", &self.managed);
        push("User preferences", &self.user);
        push("Project instructions", &self.project);
        push("Project (.claude)", &self.project_dot);
        push("Local overrides", &self.local);
        let result = if out.is_empty() { None } else { Some(out) };
        tracing::trace!(
            target: "jfc::context",
            output_len = result.as_ref().map(|s| s.len()).unwrap_or(0),
            "rendered CLAUDE.md hierarchy"
        );
        result
    }

    /// True if any layer was loaded.
    pub fn any(&self) -> bool {
        self.managed.is_some()
            || self.user.is_some()
            || self.project.is_some()
            || self.project_dot.is_some()
            || self.local.is_some()
    }
}

fn read_if_exists(path: &Path) -> Option<(PathBuf, String)> {
    let content = std::fs::read_to_string(path).ok()?;
    if content.trim().is_empty() {
        return None;
    }
    Some((path.to_path_buf(), content))
}

pub fn build_system_prompt(claude_md: Option<&str>) -> Option<String> {
    let has_claude_md = claude_md.is_some();
    let base = claude_md?.trim();
    if base.is_empty() {
        tracing::debug!(target: "jfc::context", has_claude_md, "build_system_prompt: empty content");
        return None;
    }
    let result = base.to_owned();
    tracing::debug!(
        target: "jfc::context",
        has_claude_md,
        output_len = result.len(),
        "build_system_prompt"
    );
    Some(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Normal: when no CLAUDE.md files exist anywhere, render() returns None.
    #[test]
    fn hierarchy_empty_returns_none_normal() {
        let h = ClaudeMdHierarchy::default();
        assert!(h.render().is_none());
        assert!(!h.any());
    }

    // Normal: rendering preserves source-precedence ordering with labeled
    // section headers so the model knows which layer each rule came from.
    #[test]
    fn hierarchy_render_labels_each_layer_normal() {
        let h = ClaudeMdHierarchy {
            managed: Some((PathBuf::from("/etc/claude/m.md"), "MANAGED".into())),
            user: Some((PathBuf::from("/home/u/.claude/CLAUDE.md"), "USER".into())),
            project: Some((PathBuf::from("/proj/CLAUDE.md"), "PROJECT".into())),
            project_dot: None,
            local: Some((PathBuf::from("/proj/CLAUDE.local.md"), "LOCAL".into())),
        };
        let r = h.render().expect("non-empty render");
        let idx_managed = r.find("MANAGED").expect("managed");
        let idx_user = r.find("USER").expect("user");
        let idx_project = r.find("PROJECT").expect("project");
        let idx_local = r.find("LOCAL").expect("local");
        assert!(idx_managed < idx_user);
        assert!(idx_user < idx_project);
        assert!(idx_project < idx_local);
        // Each section is labeled with its origin.
        assert!(r.contains("# Managed policy"));
        assert!(r.contains("# Local overrides"));
    }

    // Robust: empty / whitespace-only files don't contribute sections.
    #[test]
    fn hierarchy_skips_blank_layers_robust() {
        let h = ClaudeMdHierarchy {
            project: Some((PathBuf::from("/proj/CLAUDE.md"), "   \n  \n".into())),
            user: Some((PathBuf::from("/u/.claude/CLAUDE.md"), "real".into())),
            ..Default::default()
        };
        let r = h.render().expect("user layer");
        assert!(!r.contains("# Project instructions"));
        assert!(r.contains("real"));
    }
}

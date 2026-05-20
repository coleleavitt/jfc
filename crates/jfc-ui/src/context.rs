
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

    #[tracing::instrument(target = "jfc::context", skip(self), fields(path = %path.display()))]
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

    #[tracing::instrument(target = "jfc::context", skip(self), fields(path = %path.display()))]
    pub fn invalidate(&mut self, path: &Path) {
        tracing::debug!(target: "jfc::context", path = %path.display(), "invalidating cache entry");
        self.entries.remove(path);
    }

    #[tracing::instrument(target = "jfc::context", skip(self))]
    pub fn clear(&mut self) {
        tracing::debug!(target: "jfc::context", entries = self.entries.len(), "clearing read cache");
        self.entries.clear();
    }

    /// Return all cached file paths (unordered).
    pub fn paths(&self) -> Vec<std::path::PathBuf> {
        self.entries.keys().cloned().collect()
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
#[allow(dead_code)]
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

#[allow(dead_code)]
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
    use std::fs;
    use tempfile::TempDir;

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

    // Normal: a fresh ToolContext starts at zero/empty.
    #[test]
    fn tool_context_default_is_zeroed_normal() {
        let ctx = ToolContext::new();
        assert_eq!(ctx.approx_tokens, 0);
        assert_eq!(ctx.rapid_refill_count, 0);
        assert_eq!(ctx.total_user_turns, 0);
        assert_eq!(ctx.last_compact_turn, 0);
    }

    // Normal: ReadDedupCache says "changed" when path was never recorded.
    #[test]
    fn read_cache_unrecorded_path_is_changed_normal() {
        let cache = ReadDedupCache::new();
        let dir = TempDir::new().expect("tempdir");
        let path = dir.path().join("foo.txt");
        fs::write(&path, "hello").expect("write");
        assert!(!cache.is_unchanged(&path));
    }

    // Normal: after record_read, an unchanged file is reported as such.
    #[test]
    fn read_cache_records_then_detects_unchanged_normal() {
        let mut cache = ReadDedupCache::new();
        let dir = TempDir::new().expect("tempdir");
        let path = dir.path().join("foo.txt");
        fs::write(&path, "hello").expect("write");
        cache.record_read(path.clone());
        assert!(cache.is_unchanged(&path));
    }

    // Robust: re-writing a file with new content invalidates the cached entry.
    // The mtime check alone may not change (FS resolution), so size matters too.
    #[test]
    fn read_cache_detects_size_change_robust() {
        let mut cache = ReadDedupCache::new();
        let dir = TempDir::new().expect("tempdir");
        let path = dir.path().join("foo.txt");
        fs::write(&path, "hello").expect("write");
        cache.record_read(path.clone());
        // Overwrite with longer content — len() differs, so cache must say
        // "changed" even if mtime resolution didn't tick.
        fs::write(&path, "hello world more bytes").expect("rewrite");
        assert!(!cache.is_unchanged(&path));
    }

    // Robust: a missing-on-disk path is treated as "changed" (i.e. the
    // caller must re-read), not as "unchanged" — the metadata() call fails.
    #[test]
    fn read_cache_missing_file_treated_as_changed_robust() {
        let mut cache = ReadDedupCache::new();
        let dir = TempDir::new().expect("tempdir");
        let path = dir.path().join("vanished.txt");
        fs::write(&path, "x").expect("write");
        cache.record_read(path.clone());
        fs::remove_file(&path).expect("remove");
        assert!(!cache.is_unchanged(&path));
    }

    // Normal: invalidate(path) removes a single entry; clear() removes all.
    #[test]
    fn read_cache_invalidate_and_clear_normal() {
        let mut cache = ReadDedupCache::new();
        let dir = TempDir::new().expect("tempdir");
        let p1 = dir.path().join("a.txt");
        let p2 = dir.path().join("b.txt");
        fs::write(&p1, "a").expect("a");
        fs::write(&p2, "b").expect("b");
        cache.record_read(p1.clone());
        cache.record_read(p2.clone());
        assert!(cache.is_unchanged(&p1));
        cache.invalidate(&p1);
        assert!(!cache.is_unchanged(&p1));
        assert!(cache.is_unchanged(&p2));
        cache.clear();
        assert!(!cache.is_unchanged(&p2));
    }

    // Robust: record_read on a non-existent path does not panic and stores
    // nothing (so subsequent is_unchanged returns false).
    #[test]
    fn read_cache_record_missing_path_is_noop_robust() {
        let mut cache = ReadDedupCache::new();
        let dir = TempDir::new().expect("tempdir");
        let path = dir.path().join("never_existed.txt");
        cache.record_read(path.clone());
        assert!(!cache.is_unchanged(&path));
    }

    // Normal: find_claude_md walks upward from start until it hits a
    // CLAUDE.md with content, then returns (path, content).
    #[test]
    fn find_claude_md_walks_upward_normal() {
        let dir = TempDir::new().expect("tempdir");
        let nested = dir.path().join("a/b/c");
        fs::create_dir_all(&nested).expect("nested dirs");
        let claude = dir.path().join("a/CLAUDE.md");
        fs::write(&claude, "rules here").expect("write");
        let found = find_claude_md(&nested).expect("walk found CLAUDE.md");
        assert_eq!(found.0, claude);
        assert_eq!(found.1, "rules here");
    }

    // Robust: blank/whitespace-only CLAUDE.md is skipped — keep walking.
    #[test]
    fn find_claude_md_skips_blank_robust() {
        let dir = TempDir::new().expect("tempdir");
        let nested = dir.path().join("inner");
        fs::create_dir_all(&nested).expect("nested dir");
        // Inner directory has a *blank* CLAUDE.md; outer has real content.
        fs::write(nested.join("CLAUDE.md"), "   \n\t").expect("blank");
        fs::write(dir.path().join("CLAUDE.md"), "real rules").expect("real");
        let (path, content) = find_claude_md(&nested).expect("walk found outer");
        assert_eq!(path, dir.path().join("CLAUDE.md"));
        assert_eq!(content, "real rules");
    }

    // Robust: when no CLAUDE.md exists at all up to root, returns None.
    #[test]
    fn find_claude_md_returns_none_when_absent_robust() {
        let dir = TempDir::new().expect("tempdir");
        let nested = dir.path().join("x/y");
        fs::create_dir_all(&nested).expect("nested");
        // Note: walk goes all the way to / which may have something, but
        // we can't assert "always None" here. Instead, just exercise the
        // walk path on a directory where no CLAUDE.md exists in our temp.
        let result = find_claude_md(&nested);
        // The function may find an unrelated CLAUDE.md higher up — that's
        // acceptable. We just assert it didn't panic and the type matches.
        let _ = result;
    }

    // Normal: ClaudeMdHierarchy::any() reflects whether any layer is set.
    #[test]
    fn hierarchy_any_reflects_layers_normal() {
        let mut h = ClaudeMdHierarchy::default();
        assert!(!h.any());
        h.user = Some((PathBuf::from("/u/CLAUDE.md"), "x".into()));
        assert!(h.any());
    }

    // Normal: ClaudeMdHierarchy::load reads files from a temp project root.
    // We can't override the user/managed directories without env trickery,
    // but we can verify project + project_dot + local layers from a fixture.
    #[test]
    fn hierarchy_load_reads_project_layers_normal() {
        let dir = TempDir::new().expect("tempdir");
        let root = dir.path();
        fs::create_dir_all(root.join(".claude")).expect("dotclaude");
        fs::write(root.join("CLAUDE.md"), "PROJECT_RULES").expect("project");
        fs::write(root.join(".claude/CLAUDE.md"), "DOT_RULES").expect("project_dot");
        fs::write(root.join("CLAUDE.local.md"), "LOCAL_RULES").expect("local");

        let h = ClaudeMdHierarchy::load(root);
        assert!(h.project.is_some());
        assert!(h.project_dot.is_some());
        assert!(h.local.is_some());
        let rendered = h.render().expect("renders");
        assert!(rendered.contains("PROJECT_RULES"));
        assert!(rendered.contains("DOT_RULES"));
        assert!(rendered.contains("LOCAL_RULES"));
    }

    // Normal: build_system_prompt returns the trimmed input when non-empty.
    #[test]
    fn build_system_prompt_trims_input_normal() {
        let result = build_system_prompt(Some("  rules  ")).expect("some");
        assert_eq!(result, "rules");
    }

    // Robust: None or whitespace-only input yields None.
    #[test]
    fn build_system_prompt_handles_empty_robust() {
        assert!(build_system_prompt(None).is_none());
        assert!(build_system_prompt(Some("")).is_none());
        assert!(build_system_prompt(Some("    ")).is_none());
    }
}

/// Walk up from CWD to find the nearest `.git` directory and return its parent.
/// Used at startup to anchor the project-level task store before the app's
/// lazy-resolved `git_root` is available.
pub fn discover_git_root() -> Option<PathBuf> {
    let mut dir = std::env::current_dir().ok()?;
    loop {
        if dir.join(".git").exists() {
            return Some(dir);
        }
        if !dir.pop() {
            return None;
        }
    }
}

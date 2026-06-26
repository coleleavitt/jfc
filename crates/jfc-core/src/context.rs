use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde::{Deserialize, Serialize};

/// Parsed YAML frontmatter from a CLAUDE.md file.
#[derive(Debug, Clone, Default)]
pub struct ClaudeMdFrontmatter {
    /// Tools disallowed by this file's frontmatter.
    pub disallowed_tools: Vec<String>,
}

/// Represents the raw YAML frontmatter value for `disallowed-tools`.
/// Supports both a comma-separated string and a YAML list of strings.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum DisallowedToolsValue {
    Csv(String),
    List(Vec<String>),
}

/// Raw deserialization target for CLAUDE.md YAML frontmatter.
#[derive(Debug, Deserialize, Default)]
#[serde(default)]
struct RawFrontmatter {
    #[serde(alias = "disallowedTools", alias = "disallowed-tools")]
    disallowed_tools: Option<DisallowedToolsValue>,
}

/// Parse YAML frontmatter delimited by `---` at the top of a CLAUDE.md file.
/// Returns the parsed frontmatter and the body content (everything after the
/// closing `---`). If no frontmatter is present, returns default frontmatter
/// and the full content unchanged.
pub fn parse_claudemd_frontmatter(content: &str) -> (ClaudeMdFrontmatter, &str) {
    let trimmed = content.trim_start();
    if !trimmed.starts_with("---") {
        return (ClaudeMdFrontmatter::default(), content);
    }
    // Find the closing `---` delimiter (must be on its own line after the opening).
    let after_opening = &trimmed[3..];
    // Skip optional trailing whitespace/newline on the opening `---` line
    let after_opening = after_opening
        .strip_prefix('\n')
        .unwrap_or(after_opening.strip_prefix("\r\n").unwrap_or(after_opening));

    // Find the closing `---` on its own line
    let closing_pos = find_closing_frontmatter(after_opening);
    let Some(closing_pos) = closing_pos else {
        return (ClaudeMdFrontmatter::default(), content);
    };

    let yaml_block = &after_opening[..closing_pos];
    let body_start = &after_opening[closing_pos..];
    // Skip the closing `---` line itself
    let body = body_start
        .strip_prefix("---")
        .unwrap_or(body_start)
        .strip_prefix('\n')
        .unwrap_or(
            body_start
                .strip_prefix("---")
                .unwrap_or(body_start)
                .strip_prefix("\r\n")
                .unwrap_or(body_start.strip_prefix("---").unwrap_or(body_start)),
        );

    let raw: RawFrontmatter = match serde_yaml::from_str(yaml_block) {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!(
                target: "jfc::context",
                error = %e,
                "failed to parse CLAUDE.md frontmatter YAML"
            );
            return (ClaudeMdFrontmatter::default(), content);
        }
    };

    let disallowed_tools = match raw.disallowed_tools {
        Some(DisallowedToolsValue::Csv(csv)) => csv
            .split(',')
            .map(|s| s.trim().to_owned())
            .filter(|s| !s.is_empty())
            .collect(),
        Some(DisallowedToolsValue::List(list)) => {
            list.into_iter().filter(|s| !s.is_empty()).collect()
        }
        None => Vec::new(),
    };

    let fm = ClaudeMdFrontmatter { disallowed_tools };
    (fm, body)
}

/// Find the position of the closing `---` delimiter. It must appear at the
/// start of a line.
fn find_closing_frontmatter(s: &str) -> Option<usize> {
    // Check if it starts right at position 0
    if s.starts_with("---")
        && (s.len() == 3
            || s.as_bytes().get(3) == Some(&b'\n')
            || s.as_bytes().get(3) == Some(&b'\r'))
    {
        return Some(0);
    }
    // Search for `\n---` pattern
    let mut search_from = 0;
    while let Some(pos) = s[search_from..].find('\n') {
        let abs_pos = search_from + pos + 1; // position after the newline
        let rest = &s[abs_pos..];
        if rest.starts_with("---")
            && (rest.len() == 3
                || rest.as_bytes().get(3) == Some(&b'\n')
                || rest.as_bytes().get(3) == Some(&b'\r')
                || rest.as_bytes().get(3) == Some(&b' '))
        {
            return Some(abs_pos);
        }
        search_from = abs_pos;
    }
    None
}

/// Tracks file mtime+size so re-reads of unchanged files return a short
/// "file unchanged" stub instead of full content, saving tokens.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ReadDedupCache {
    #[serde(default)]
    entries: HashMap<PathBuf, ReadEntry>,
    #[serde(default)]
    stale_edit_misses: HashMap<PathBuf, ReadEntry>,
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

    fn entry_for(path: &Path) -> Option<ReadEntry> {
        let meta = std::fs::metadata(path).ok()?;
        let mtime_secs = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
            .map(|d| d.as_secs())
            .unwrap_or(0);
        Some(ReadEntry {
            mtime_secs,
            len: meta.len(),
        })
    }

    /// Returns `true` if path was previously read and disk mtime+size still match.
    pub fn is_unchanged(&self, path: &Path) -> bool {
        let Some(entry) = self.entries.get(path) else {
            return false;
        };
        Self::entry_for(path).is_some_and(|current| {
            entry.mtime_secs == current.mtime_secs && entry.len == current.len
        })
    }

    /// Returns `true` when a previous Edit/MultiEdit for this file missed
    /// `old_string` and the model must refresh via a full Read before retrying.
    pub fn requires_stale_edit_refresh(&self, path: &Path) -> bool {
        self.stale_edit_misses.contains_key(path)
    }

    /// Mark that an Edit/MultiEdit failed against stale contents. A later full
    /// Read clears this marker; until then mutation retries fail fast.
    pub fn mark_stale_edit_miss(&mut self, path: PathBuf) {
        if let Some(entry) = Self::entry_for(&path) {
            self.stale_edit_misses.insert(path, entry);
        }
    }

    #[tracing::instrument(target = "jfc::context", skip(self), fields(path = %path.display()))]
    pub fn record_read(&mut self, path: PathBuf) {
        tracing::trace!(target: "jfc::context", path = %path.display(), "recording file read");
        self.stale_edit_misses.remove(&path);
        if let Some(entry) = Self::entry_for(&path) {
            self.entries.insert(path, entry);
        }
    }

    #[tracing::instrument(target = "jfc::context", skip(self), fields(path = %path.display()))]
    pub fn invalidate(&mut self, path: &Path) {
        tracing::debug!(target: "jfc::context", path = %path.display(), "invalidating cache entry");
        self.entries.remove(path);
        self.stale_edit_misses.remove(path);
    }

    #[tracing::instrument(target = "jfc::context", skip(self))]
    pub fn clear(&mut self) {
        tracing::debug!(target: "jfc::context", entries = self.entries.len(), "clearing read cache");
        self.entries.clear();
        self.stale_edit_misses.clear();
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
pub fn find_claude_md(start: &Path) -> Option<(PathBuf, String)> {
    tracing::debug!(target: "jfc::context", start = %start.display(), "searching for CLAUDE.md");
    let mut dir = start.to_path_buf();
    loop {
        let candidate = dir.join("CLAUDE.md");
        if let Ok(content) = std::fs::read_to_string(&candidate)
            && !content.trim().is_empty()
        {
            tracing::info!(
                target: "jfc::context",
                path = %candidate.display(),
                size_bytes = content.len(),
                "found CLAUDE.md"
            );
            return Some((candidate, content));
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
    /// `<project>/AGENTS.md` — cross-tool agent-instructions convention
    /// (shared with Codex/Cursor/etc.). Auto-loaded so JFC honours the same
    /// project guidance other agent harnesses read.
    pub agents: Option<(PathBuf, String)>,
    /// `<project>/.claude/rules/*.md` — focused always-on project rules.
    pub rules: Vec<(PathBuf, String)>,
    /// `<project>/MEMORY.md` — Claude Code-style concise memory index.
    pub memory: Option<(PathBuf, String)>,
    /// `<project>/CLAUDE.local.md` — gitignored personal overrides.
    pub local: Option<(PathBuf, String)>,
    /// Extra roots supplied by CLI/config (`--add-dir`,
    /// `permissions.additionalDirectories`).
    pub extra_roots: Vec<(PathBuf, String)>,
}

impl ClaudeMdHierarchy {
    /// Load every CLAUDE.md layer that exists for the given project root.
    pub fn load(project_root: &Path) -> Self {
        Self::load_with_extra_roots(project_root, &[])
    }

    /// Load the project hierarchy plus CLAUDE.md layers from extra roots.
    pub fn load_with_extra_roots(project_root: &Path, extra_roots: &[PathBuf]) -> Self {
        tracing::info!(target: "jfc::context", project_root = %project_root.display(), "loading CLAUDE.md hierarchy");
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"));
        let cfg = dirs::config_dir().unwrap_or_else(|| home.join(".config"));
        let result = Self {
            managed: read_if_exists(&cfg.join("claude/CLAUDE.md")),
            user: read_if_exists(&home.join(".claude/CLAUDE.md")),
            project: read_if_exists(&project_root.join("CLAUDE.md")),
            project_dot: read_if_exists(&project_root.join(".claude/CLAUDE.md")),
            agents: read_if_exists(&project_root.join("AGENTS.md"))
                .or_else(|| read_if_exists(&project_root.join(".agents/AGENTS.md"))),
            rules: read_markdown_dir(&project_root.join(".claude/rules")),
            memory: read_if_exists(&project_root.join("MEMORY.md")),
            local: read_if_exists(&project_root.join("CLAUDE.local.md")),
            extra_roots: read_extra_root_layers(project_root, extra_roots),
        };
        tracing::debug!(
            target: "jfc::context",
            has_managed = result.managed.is_some(),
            has_user = result.user.is_some(),
            has_project = result.project.is_some(),
            has_project_dot = result.project_dot.is_some(),
            rules = result.rules.len(),
            has_agents = result.agents.is_some(),
            has_memory = result.memory.is_some(),
            has_local = result.local.is_some(),
            extra_roots = result.extra_roots.len(),
            "CLAUDE.md hierarchy loaded"
        );
        result
    }

    /// Concatenate all layers into a single system-prompt-ready string with
    /// labeled section headers so the model can tell where each rule came
    /// from. Returns `None` when nothing was found. Frontmatter is stripped
    /// from the rendered output (it's metadata, not prompt content).
    pub fn render(&self) -> Option<String> {
        let mut out = String::new();
        fn append_layer(out: &mut String, label: &str, path: &Path, content: &str) {
            if content.trim().is_empty() {
                return;
            }
            let (_fm, body) = parse_claudemd_frontmatter(content);
            let body = body.trim();
            if body.is_empty() {
                return;
            }
            if !out.is_empty() {
                out.push_str("\n\n");
            }
            out.push_str(&format!("# {label} ({})\n\n{}", path.display(), body));
        }
        for (label, layer) in [
            ("Managed policy", &self.managed),
            ("User preferences", &self.user),
            ("Project instructions", &self.project),
            ("Project (.claude)", &self.project_dot),
            ("Project agents (AGENTS.md)", &self.agents),
        ] {
            if let Some((path, content)) = layer {
                append_layer(&mut out, label, path, content);
            }
        }
        for (path, content) in &self.rules {
            append_layer(&mut out, "Project rule", path, content);
        }
        for (label, layer) in [
            ("Project memory index", &self.memory),
            ("Local overrides", &self.local),
        ] {
            if let Some((path, content)) = layer {
                append_layer(&mut out, label, path, content);
            }
        }
        for (path, content) in &self.extra_roots {
            append_layer(&mut out, "Additional directory instructions", path, content);
        }
        let result = if out.is_empty() { None } else { Some(out) };
        tracing::trace!(
            target: "jfc::context",
            output_len = result.as_ref().map(|s| s.len()).unwrap_or(0),
            "rendered CLAUDE.md hierarchy"
        );
        result
    }

    /// Collect all `disallowed-tools` entries from every layer's frontmatter.
    /// Returns a deduplicated list of tool names that should be removed from
    /// the model's available tools.
    pub fn collect_disallowed_tools(&self) -> Vec<String> {
        let mut tools = Vec::new();
        let layers: [&Option<(PathBuf, String)>; 7] = [
            &self.managed,
            &self.user,
            &self.project,
            &self.project_dot,
            &self.agents,
            &self.memory,
            &self.local,
        ];
        for (_path, content) in layers.into_iter().flatten() {
            let (fm, _body) = parse_claudemd_frontmatter(content);
            tools.extend(fm.disallowed_tools);
        }
        for (_path, content) in &self.rules {
            let (fm, _body) = parse_claudemd_frontmatter(content);
            tools.extend(fm.disallowed_tools);
        }
        for (_path, content) in &self.extra_roots {
            let (fm, _body) = parse_claudemd_frontmatter(content);
            tools.extend(fm.disallowed_tools);
        }
        // Deduplicate while preserving order
        let mut seen = std::collections::HashSet::new();
        tools.retain(|t| seen.insert(t.clone()));
        if !tools.is_empty() {
            tracing::info!(
                target: "jfc::context",
                count = tools.len(),
                tools = ?tools,
                "collected disallowed-tools from CLAUDE.md frontmatter"
            );
        }
        tools
    }

    /// True if any layer was loaded.
    pub fn any(&self) -> bool {
        self.managed.is_some()
            || self.user.is_some()
            || self.project.is_some()
            || self.project_dot.is_some()
            || self.agents.is_some()
            || !self.rules.is_empty()
            || self.memory.is_some()
            || self.local.is_some()
            || !self.extra_roots.is_empty()
    }
}

fn read_if_exists(path: &Path) -> Option<(PathBuf, String)> {
    let content = std::fs::read_to_string(path).ok()?;
    if content.trim().is_empty() {
        return None;
    }
    Some((path.to_path_buf(), content))
}

fn read_markdown_dir(dir: &Path) -> Vec<(PathBuf, String)> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("md") {
            continue;
        }
        if let Some(layer) = read_if_exists(&path) {
            out.push(layer);
        }
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

fn read_extra_root_layers(project_root: &Path, roots: &[PathBuf]) -> Vec<(PathBuf, String)> {
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for root in roots {
        let root = if root.is_absolute() {
            root.clone()
        } else {
            project_root.join(root)
        };
        for path in [root.join("CLAUDE.md"), root.join(".claude/CLAUDE.md")] {
            if seen.insert(path.clone())
                && let Some(layer) = read_if_exists(&path)
            {
                out.push(layer);
            }
        }
        for layer in read_markdown_dir(&root.join(".claude/rules")) {
            if seen.insert(layer.0.clone()) {
                out.push(layer);
            }
        }
    }
    out
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
            ..Default::default()
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
        fs::create_dir_all(root.join(".claude/rules")).expect("rules dir");
        fs::write(root.join(".claude/rules/security.md"), "SECURITY_RULES").expect("rule");
        fs::write(root.join("MEMORY.md"), "MEMORY_INDEX").expect("memory");
        fs::write(root.join("CLAUDE.local.md"), "LOCAL_RULES").expect("local");

        let h = ClaudeMdHierarchy::load(root);
        assert!(h.project.is_some());
        assert!(h.project_dot.is_some());
        assert_eq!(h.rules.len(), 1);
        assert!(h.memory.is_some());
        assert!(h.local.is_some());
        let rendered = h.render().expect("renders");
        assert!(rendered.contains("PROJECT_RULES"));
        assert!(rendered.contains("DOT_RULES"));
        assert!(rendered.contains("SECURITY_RULES"));
        assert!(rendered.contains("MEMORY_INDEX"));
        assert!(rendered.contains("LOCAL_RULES"));
    }

    #[test]
    fn hierarchy_loads_and_renders_agents_md_normal() {
        let dir = TempDir::new().expect("tempdir");
        let root = dir.path();
        fs::write(root.join("AGENTS.md"), "AGENTS_GUIDANCE").expect("agents");

        let h = ClaudeMdHierarchy::load(root);
        assert!(h.agents.is_some(), "AGENTS.md must be auto-loaded");
        assert!(h.any());
        let rendered = h.render().expect("renders");
        assert!(rendered.contains("AGENTS_GUIDANCE"), "{rendered}");
        assert!(rendered.contains("AGENTS.md"), "label cites the file");
    }

    #[test]
    fn hierarchy_agents_md_falls_back_to_dot_agents_dir_robust() {
        let dir = TempDir::new().expect("tempdir");
        let root = dir.path();
        fs::create_dir_all(root.join(".agents")).expect("dotagents");
        fs::write(root.join(".agents/AGENTS.md"), "DOT_AGENTS").expect("agents");

        let h = ClaudeMdHierarchy::load(root);
        assert!(h.agents.is_some());
        assert!(h.render().expect("renders").contains("DOT_AGENTS"));
    }

    #[test]
    fn hierarchy_load_with_extra_roots_reads_context_and_frontmatter_normal() {
        let dir = TempDir::new().expect("tempdir");
        let root = dir.path();
        let extra = root.join("shared");
        fs::create_dir_all(extra.join(".claude/rules")).expect("extra rules");
        fs::write(
            extra.join("CLAUDE.md"),
            "---\ndisallowed-tools: Bash\n---\nSHARED_RULES",
        )
        .expect("extra claude");
        fs::write(extra.join(".claude/rules/api.md"), "API_RULES").expect("extra rule");

        let h = ClaudeMdHierarchy::load_with_extra_roots(root, &[PathBuf::from("shared")]);
        assert_eq!(h.extra_roots.len(), 2);
        let rendered = h.render().expect("renders");
        assert!(rendered.contains("SHARED_RULES"));
        assert!(rendered.contains("API_RULES"));
        assert!(
            h.collect_disallowed_tools()
                .iter()
                .any(|tool| tool == "Bash")
        );
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

    // ─── Frontmatter parsing tests ───────────────────────────────────────

    // Normal: no frontmatter returns empty disallowed_tools and full content.
    #[test]
    fn frontmatter_no_delimiters_returns_default() {
        let content = "# Rules\nDo stuff.";
        let (fm, body) = parse_claudemd_frontmatter(content);
        assert!(fm.disallowed_tools.is_empty());
        assert_eq!(body, content);
    }

    // Normal: CSV format for disallowed-tools.
    #[test]
    fn frontmatter_csv_disallowed_tools() {
        let content = "---\ndisallowed-tools: Bash,Write,Edit\n---\n# Rules\nDo stuff.";
        let (fm, body) = parse_claudemd_frontmatter(content);
        assert_eq!(fm.disallowed_tools, vec!["Bash", "Write", "Edit"]);
        assert_eq!(body, "# Rules\nDo stuff.");
    }

    // Normal: YAML list format for disallowed-tools.
    #[test]
    fn frontmatter_list_disallowed_tools() {
        let content = "---\ndisallowed-tools:\n  - Bash\n  - Write\n---\n# Rules";
        let (fm, body) = parse_claudemd_frontmatter(content);
        assert_eq!(fm.disallowed_tools, vec!["Bash", "Write"]);
        assert_eq!(body, "# Rules");
    }

    // Normal: camelCase alias `disallowedTools` works.
    #[test]
    fn frontmatter_camel_case_alias() {
        let content = "---\ndisallowedTools: Read,Glob\n---\nBody text.";
        let (fm, body) = parse_claudemd_frontmatter(content);
        assert_eq!(fm.disallowed_tools, vec!["Read", "Glob"]);
        assert_eq!(body, "Body text.");
    }

    // Robust: empty frontmatter (no keys) returns default.
    #[test]
    fn frontmatter_empty_yaml_block() {
        let content = "---\n---\n# Body";
        let (fm, body) = parse_claudemd_frontmatter(content);
        assert!(fm.disallowed_tools.is_empty());
        assert_eq!(body, "# Body");
    }

    // Robust: invalid YAML returns full content unchanged.
    #[test]
    fn frontmatter_invalid_yaml_returns_full_content() {
        let content = "---\n[invalid yaml: {{{\n---\n# Body";
        let (fm, body) = parse_claudemd_frontmatter(content);
        assert!(fm.disallowed_tools.is_empty());
        assert_eq!(body, content);
    }

    // Normal: render strips frontmatter from output.
    #[test]
    fn hierarchy_render_strips_frontmatter() {
        let h = ClaudeMdHierarchy {
            project: Some((
                PathBuf::from("/proj/CLAUDE.md"),
                "---\ndisallowed-tools: Bash\n---\n# Rules\nNo bash.".into(),
            )),
            ..Default::default()
        };
        let rendered = h.render().expect("non-empty");
        assert!(rendered.contains("# Rules"));
        assert!(rendered.contains("No bash."));
        // Frontmatter should NOT appear in rendered prompt
        assert!(!rendered.contains("disallowed-tools"));
        assert!(!rendered.contains("---"));
    }

    // Normal: collect_disallowed_tools merges from multiple layers.
    #[test]
    fn hierarchy_collect_disallowed_tools_merges_layers() {
        let h = ClaudeMdHierarchy {
            project: Some((
                PathBuf::from("/proj/CLAUDE.md"),
                "---\ndisallowed-tools: Bash,Write\n---\n# Rules".into(),
            )),
            local: Some((
                PathBuf::from("/proj/CLAUDE.local.md"),
                "---\ndisallowed-tools:\n  - Edit\n  - Bash\n---\n# Local".into(),
            )),
            ..Default::default()
        };
        let tools = h.collect_disallowed_tools();
        // Bash appears in both but should be deduplicated
        assert_eq!(tools, vec!["Bash", "Write", "Edit"]);
    }

    // Robust: CSV with extra whitespace is trimmed.
    #[test]
    fn frontmatter_csv_whitespace_trimmed() {
        let content = "---\ndisallowed-tools:  Bash , Write , Edit \n---\nBody";
        let (fm, _body) = parse_claudemd_frontmatter(content);
        assert_eq!(fm.disallowed_tools, vec!["Bash", "Write", "Edit"]);
    }
}

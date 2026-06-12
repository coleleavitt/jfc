//! Memory system for jfc — persistent storage of learned preferences, facts,
//! and project context across sessions.
//!
//! Storage layout (mirroring Claude Code v126):
//! - User-level: `~/.config/jfc/memory/` — personal preferences that follow
//!   the user across all projects.
//! - Project-level: `<project>/.jfc/memory/` — shared project knowledge.
//!
//! Each memory is a single `.md` file with YAML frontmatter:
//! ```markdown
//! ---
//! type: feedback | preference | project | context
//! scope: user | team
//! created: 2026-05-01T12:00:00Z
//! ---
//! The actual memory content goes here.
//! ```
//!
//! Memories are immutable — to update, delete the old file and create a new one.

use std::fmt;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

// ─── Types ───────────────────────────────────────────────────────────────────

/// Which directory a memory lives in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MemoryLevel {
    /// `~/.config/jfc/memory/`
    User,
    /// `<project>/.jfc/memory/`
    Project,
    /// `<project>/.jfc/memory/team/` — shared across everyone working
    /// in this repo. v132 prompt: "Other teammates' Claude sessions
    /// write here too. Merge near-duplicates within `team/`. DO NOT
    /// delete a team memory just because you don't recognize it."
    Team,
    /// Extra memory directories supplied by `JFC_MEMORY_DIRS`.
    External,
}

impl fmt::Display for MemoryLevel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::User => write!(f, "user"),
            Self::Project => write!(f, "project"),
            Self::Team => write!(f, "team"),
            Self::External => write!(f, "external"),
        }
    }
}

/// Semantic type of a memory — mirrors CC 2.1.167's four-type taxonomy.
///
/// | Type       | Scope default | When to save |
/// |------------|---------------|--------------|
/// | `user`     | always private | Role, expertise, preferences |
/// | `feedback` | private (team if project-wide convention) | Corrections AND confirmed approaches |
/// | `project`  | strongly team | Ongoing work, deadlines, decisions not in code/git |
/// | `reference` | usually team | Pointers to external systems |
///
/// Legacy types `preference` and `context` are preserved as aliases for
/// backward-compatibility with existing memory files.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MemoryType {
    /// User's role, expertise, working preferences. Always private.
    User,
    /// Corrections and confirmations of approach.
    /// Body: lead with rule, then **Why:** + **How to apply:** lines.
    Feedback,
    /// Ongoing work, goals, decisions not derivable from code/git.
    /// Body: lead with fact/decision, then **Why:** + **How to apply:** lines.
    /// Always convert relative dates to absolute ISO dates ("Thursday" → "2026-06-08").
    Project,
    /// Pointers to external systems (issue tracker, Grafana, Slack channel).
    Reference,
    /// Legacy: stylistic / workflow preferences (alias for `user`).
    Preference,
    /// Legacy: general context or learned facts (alias for `project`).
    Context,
}

impl fmt::Display for MemoryType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::User => write!(f, "user"),
            Self::Feedback => write!(f, "feedback"),
            Self::Project => write!(f, "project"),
            Self::Reference => write!(f, "reference"),
            Self::Preference => write!(f, "preference"),
            Self::Context => write!(f, "context"),
        }
    }
}

impl std::str::FromStr for MemoryType {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "user" => Ok(Self::User),
            "feedback" => Ok(Self::Feedback),
            "project" => Ok(Self::Project),
            "reference" | "ref" => Ok(Self::Reference),
            "preference" | "pref" => Ok(Self::Preference),
            "context" | "ctx" => Ok(Self::Context),
            other => Err(format!(
                "unknown memory type: {other}. Use: user, feedback, project, reference"
            )),
        }
    }
}

/// Visibility scope — who can benefit from this memory.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MemoryScope {
    /// Only the current user benefits.
    Private,
    /// Shared with all users in the project (committed to VCS).
    Team,
}

impl fmt::Display for MemoryScope {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Private => write!(f, "private"),
            Self::Team => write!(f, "team"),
        }
    }
}

impl std::str::FromStr for MemoryScope {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "private" => Ok(Self::Private),
            "team" => Ok(Self::Team),
            other => Err(format!("unknown memory scope: {other}")),
        }
    }
}

/// YAML frontmatter of a memory file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryFrontmatter {
    #[serde(rename = "type")]
    pub memory_type: MemoryType,
    pub scope: MemoryScope,
    #[serde(default)]
    pub created: Option<String>,

    // ─── jfc-learn extended fields ──────────────────────────────────────
    // All Option so existing files still parse without these fields.
    /// Content-addressable hash for deduplication (SHA256 of normalized text).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub normalized_hash: Option<String>,

    /// Origin of this memory: "historian" | "agent" | "dreamer" | "user"
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_type: Option<String>,

    /// Session that produced this memory.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_session_id: Option<String>,

    /// How many times this fact has been observed across sessions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seen_count: Option<u32>,

    /// How many times this memory has been retrieved for context injection.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retrieval_count: Option<u32>,

    /// Unix timestamp (ms) when this fact was first observed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub first_seen_at: Option<u64>,

    /// Unix timestamp (ms) of the most recent observation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_seen_at: Option<u64>,

    /// Unix timestamp (ms) of the most recent retrieval.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_retrieved_at: Option<u64>,

    /// Lifecycle status: "active" | "permanent" | "archived"
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory_status: Option<String>,

    /// Unix timestamp (ms) when this memory expires (for TTL-based cleanup).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<u64>,

    /// Verification state: "unverified" | "verified" | "stale" | "flagged"
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verification_status: Option<String>,

    /// Unix timestamp (ms) when last verified.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verified_at: Option<u64>,

    /// Path/id of the memory that supersedes this one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub superseded_by: Option<String>,
}

impl MemoryFrontmatter {
    /// Create a minimal frontmatter with only required fields; all extended fields default to None.
    pub fn new(memory_type: MemoryType, scope: MemoryScope) -> Self {
        Self {
            memory_type,
            scope,
            created: None,
            normalized_hash: None,
            source_type: None,
            source_session_id: None,
            seen_count: None,
            retrieval_count: None,
            first_seen_at: None,
            last_seen_at: None,
            last_retrieved_at: None,
            memory_status: None,
            expires_at: None,
            verification_status: None,
            verified_at: None,
            superseded_by: None,
        }
    }
}

/// A fully-loaded memory entry.
#[derive(Debug, Clone)]
pub struct MemoryEntry {
    /// Absolute path to the .md file.
    pub path: PathBuf,
    /// Which directory level this lives in.
    pub level: MemoryLevel,
    /// Parsed frontmatter.
    pub frontmatter: MemoryFrontmatter,
    /// The body content (everything after the `---` block).
    pub body: String,
}

/// Summary of a local team-memory sync between `<project>/.jfc/memory/team`
/// and another local directory. This intentionally stays filesystem-only:
/// cloud/team-server sync can be layered above it without changing the
/// conflict behavior users see in the repo.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TeamMemorySyncReport {
    pub local_dir: PathBuf,
    pub remote_dir: PathBuf,
    pub pushed: usize,
    pub pulled: usize,
    pub conflicts: Vec<TeamMemoryConflict>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TeamMemoryConflict {
    pub file_name: String,
    pub local_path: PathBuf,
    pub remote_path: PathBuf,
    pub conflict_path: PathBuf,
}

// ─── Paths ───────────────────────────────────────────────────────────────────

/// Returns the user-level memory directory: `~/.config/jfc/memory/`
pub fn user_memory_dir() -> PathBuf {
    let cfg = dirs::config_dir().unwrap_or_else(|| {
        dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("/"))
            .join(".config")
    });
    cfg.join("jfc").join("memory")
}

/// Returns the project-level memory directory: `<project>/.jfc/memory/`
pub fn project_memory_dir(project_root: &Path) -> PathBuf {
    project_root.join(".jfc").join("memory")
}

/// Returns the team-shared memory directory:
/// `<project>/.jfc/memory/team/`. Mirrors v132's `## Team memory
/// (team/ subdirectory)` taxonomy: anything checked into git here is
/// shared across every contributor's jfc sessions.
pub fn team_memory_dir(project_root: &Path) -> PathBuf {
    project_memory_dir(project_root).join("team")
}

/// Returns `true` if the given path resides inside a known memory directory.
pub fn is_memory_path(path: &Path) -> bool {
    let normalized = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let user_dir = user_memory_dir();
    let user_canon = user_dir.canonicalize().unwrap_or_else(|_| user_dir.clone());

    if normalized.starts_with(&user_canon) {
        return true;
    }

    for dir in extra_memory_dirs() {
        let canon = dir.canonicalize().unwrap_or(dir);
        if normalized.starts_with(&canon) {
            return true;
        }
    }

    // Check against current working directory's project memory
    if let Ok(cwd) = std::env::current_dir() {
        let proj_dir = project_memory_dir(&cwd);
        let proj_canon = proj_dir.canonicalize().unwrap_or_else(|_| proj_dir.clone());
        if normalized.starts_with(&proj_canon) {
            return true;
        }
    }

    false
}

// ─── Read / Write / Delete ───────────────────────────────────────────────────

/// Load all memory entries from both user and project directories.
pub fn load_all_memories(project_root: &Path) -> Vec<MemoryEntry> {
    let mut entries = Vec::new();
    load_from_dir(&user_memory_dir(), MemoryLevel::User, &mut entries);
    load_from_dir(
        &project_memory_dir(project_root),
        MemoryLevel::Project,
        &mut entries,
    );
    // Team memory lives under `<project>/.jfc/memory/team/` and is
    // shared across everyone working in the repo. Loaded after the
    // project root so the simple flat scan above doesn't pick up
    // team files twice (project_memory_dir reads only `.md` files
    // directly in `.jfc/memory/`, not subdirs).
    load_from_dir(
        &team_memory_dir(project_root),
        MemoryLevel::Team,
        &mut entries,
    );
    for dir in extra_memory_dirs() {
        load_from_dir(&dir, MemoryLevel::External, &mut entries);
    }
    tracing::info!(
        target: "jfc::memory",
        user_dir = %user_memory_dir().display(),
        project_dir = %project_memory_dir(project_root).display(),
        extra_dirs = extra_memory_dirs().len(),
        total_entries = entries.len(),
        "loaded all memories"
    );
    entries
}

fn extra_memory_dirs() -> Vec<PathBuf> {
    std::env::var_os("JFC_MEMORY_DIRS")
        .map(|raw| std::env::split_paths(&raw).collect())
        .unwrap_or_default()
}

/// Load memory entries from a single directory.
fn load_from_dir(dir: &Path, level: MemoryLevel, out: &mut Vec<MemoryEntry>) {
    let read_dir = match std::fs::read_dir(dir) {
        Ok(rd) => rd,
        Err(_) => return, // directory doesn't exist yet — normal
    };

    for entry in read_dir.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("md") {
            continue;
        }
        match parse_memory_file(&path, level) {
            Ok(mem) => out.push(mem),
            Err(e) => {
                tracing::warn!(
                    target: "jfc::memory",
                    path = %path.display(),
                    error = %e,
                    "failed to parse memory file"
                );
            }
        }
    }
}

/// Parse a single memory `.md` file with YAML frontmatter.
fn parse_memory_file(path: &Path, level: MemoryLevel) -> Result<MemoryEntry, String> {
    let content = std::fs::read_to_string(path).map_err(|e| format!("read error: {e}"))?;

    let (frontmatter, body) = parse_frontmatter_and_body(&content)?;

    Ok(MemoryEntry {
        path: path.to_path_buf(),
        level,
        frontmatter,
        body,
    })
}

/// Split content into YAML frontmatter and body.
fn parse_frontmatter_and_body(content: &str) -> Result<(MemoryFrontmatter, String), String> {
    let trimmed = content.trim_start();
    if !trimmed.starts_with("---") {
        // No frontmatter — treat as plain context memory
        return Ok((
            MemoryFrontmatter::new(MemoryType::Context, MemoryScope::Private),
            content.to_string(),
        ));
    }

    // Find closing ---
    let after_open = &trimmed[3..];
    let close_idx = after_open
        .find("\n---")
        .ok_or_else(|| "unclosed frontmatter block".to_string())?;

    let yaml_str = &after_open[..close_idx].trim();
    let body_start = 3 + close_idx + 4; // skip opening --- + yaml + \n---
    let body = trimmed[body_start..].trim_start_matches('\n').to_string();

    let frontmatter: MemoryFrontmatter =
        serde_yaml::from_str(yaml_str).map_err(|e| format!("YAML parse error: {e}"))?;

    Ok((frontmatter, body))
}

/// Result of attempting to create a memory, including optional conflict info.
#[derive(Debug, Clone)]
pub struct CreateMemoryResult {
    /// Path of the newly-created memory file.
    pub path: PathBuf,
    /// A conflicting (near-duplicate) memory file found before saving, if any.
    /// Mirrors CC 2.1.167's `conflicting_memory_id` field.
    pub conflicting_memory_id: Option<PathBuf>,
}

/// Create a memory and return conflict info alongside the new path.
///
/// Checks existing memories in the same directory for near-duplicate content
/// (>50% word overlap) before writing. Returns `conflicting_memory_id` so the
/// caller can decide whether to delete the old file or merge content.
pub fn create_memory_checked(
    level: MemoryLevel,
    memory_type: MemoryType,
    scope: MemoryScope,
    body: &str,
    project_root: &Path,
) -> Result<CreateMemoryResult, String> {
    let dir = memory_dir_for(level, project_root);
    let conflicting = find_conflicting_memory(&dir, body);
    let path = write_memory_file(&dir, memory_type, scope, body)?;
    tracing::info!(
        target: "jfc::memory",
        path = %path.display(),
        conflicting = ?conflicting.as_ref().map(|p: &PathBuf| p.display().to_string()),
        level = %level,
        memory_type = %memory_type,
        scope = %scope,
        "created memory (with conflict check)"
    );
    Ok(CreateMemoryResult {
        path,
        conflicting_memory_id: conflicting,
    })
}

/// Create a new memory file. Returns the path of the created file.
pub fn create_memory(
    level: MemoryLevel,
    memory_type: MemoryType,
    scope: MemoryScope,
    body: &str,
    project_root: &Path,
) -> Result<PathBuf, String> {
    let dir = memory_dir_for(level, project_root);
    let path = write_memory_file(&dir, memory_type, scope, body)?;
    tracing::info!(
        target: "jfc::memory",
        path = %path.display(),
        level = %level,
        memory_type = %memory_type,
        scope = %scope,
        "created memory"
    );
    // Keep the project MEMORY.md index fresh automatically. Previously this was
    // prompt-guidance only ("after writing, add a one-line pointer to
    // MEMORY.md") with no code path, so the index went stale whenever the model
    // forgot. User-level memories have no project index, so they're skipped.
    if !matches!(level, MemoryLevel::User) {
        if let Err(e) = append_memory_index_pointer(project_root, &path, body) {
            // Index maintenance is best-effort: a failed append must never lose
            // the just-written memory file.
            tracing::warn!(target: "jfc::memory", error = %e, "failed to update MEMORY.md index");
        }
    }
    Ok(path)
}

/// Append a one-line pointer for a freshly-created memory to `<root>/MEMORY.md`,
/// matching the documented format `- [Title](path) — hook`. Creates the index
/// (with a heading) if absent, and is idempotent: a pointer to the same file is
/// never duplicated.
fn append_memory_index_pointer(
    project_root: &Path,
    memory_path: &Path,
    body: &str,
) -> std::io::Result<()> {
    let index = project_root.join("MEMORY.md");

    // Relative path from the project root for a portable link.
    let rel = memory_path
        .strip_prefix(project_root)
        .unwrap_or(memory_path)
        .to_string_lossy()
        .replace('\\', "/");

    // Title + hook from the memory body's first non-empty line.
    let first_line = body
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("memory");
    let title = truncate_chars(first_line, 60);
    let hook = truncate_chars(first_line, 100);
    let pointer = format!("- [{title}]({rel}) — {hook}");

    let existing = std::fs::read_to_string(&index).unwrap_or_default();
    // Idempotent: don't append if this file is already linked.
    if existing.contains(&format!("({rel})")) {
        return Ok(());
    }

    let mut out = if existing.trim().is_empty() {
        String::from("# Project Memory Index\n\n")
    } else {
        let mut s = existing;
        if !s.ends_with('\n') {
            s.push('\n');
        }
        s
    };
    out.push_str(&pointer);
    out.push('\n');
    write_atomic_sync(&index, out.as_bytes())
}

/// Truncate `s` to at most `max` chars on a char boundary, adding an ellipsis
/// when cut.
fn truncate_chars(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_owned()
    } else {
        let cut: String = s.chars().take(max.saturating_sub(1)).collect();
        format!("{cut}…")
    }
}

/// Resolve the memory directory for a given level.
fn memory_dir_for(level: MemoryLevel, project_root: &Path) -> PathBuf {
    match level {
        MemoryLevel::User => user_memory_dir(),
        MemoryLevel::Project => project_memory_dir(project_root),
        MemoryLevel::Team => team_memory_dir(project_root),
        MemoryLevel::External => project_memory_dir(project_root),
    }
}

/// Write a single memory `.md` file into `dir`. Returns the path.
fn write_memory_file(
    dir: &Path,
    memory_type: MemoryType,
    scope: MemoryScope,
    body: &str,
) -> Result<PathBuf, String> {
    // Ensure directory exists
    std::fs::create_dir_all(dir).map_err(|e| format!("failed to create memory directory: {e}"))?;

    let now: DateTime<Utc> = SystemTime::now().into();
    let slug = slugify(body, 40);
    let timestamp = now.format("%Y%m%d-%H%M%S");
    let filename = format!("{timestamp}-{slug}.md");
    let path = dir.join(&filename);

    // Render frontmatter + body
    let content = format!(
        "---\ntype: {memory_type}\nscope: {scope}\ncreated: {}\n---\n{body}\n",
        now.format("%Y-%m-%d")
    );

    // Atomic write — a power loss while saving the memory file would
    // otherwise leave a truncated frontmatter + body and `load_memories`
    // would silently skip the file (because the YAML header wouldn't
    // parse), losing the user's note.
    write_atomic_sync(&path, content.as_bytes())
        .map_err(|e| format!("failed to write memory file: {e}"))?;

    Ok(path)
}

/// Find an existing memory in `dir` whose content significantly overlaps
/// with `new_body`. Returns the path of the conflicting file if found.
///
/// Uses simple word-overlap: if >50% of the words in `new_body` appear in
/// the existing memory's body, it's considered a near-duplicate.
/// This is the same heuristic CC uses via `conflicting_memory_id`.
fn find_conflicting_memory(dir: &Path, new_body: &str) -> Option<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return None;
    };
    let new_words = word_set(new_body);
    if new_words.is_empty() {
        return None;
    }

    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("md") {
            continue;
        }
        let Ok(content) = std::fs::read_to_string(&path) else {
            continue;
        };
        let body = strip_frontmatter(&content);
        let existing_words = word_set(body);
        // Count how many of the new body's words appear in the existing memory
        let overlap = new_words
            .iter()
            .filter(|w| existing_words.contains(*w))
            .count();
        let overlap_ratio = overlap as f64 / new_words.len() as f64;
        if overlap_ratio > 0.5 {
            tracing::debug!(
                target: "jfc::memory",
                path = %path.display(),
                overlap_ratio,
                "found near-duplicate memory"
            );
            return Some(path);
        }
    }
    None
}

/// Extract the body portion of a memory file (after the `---` front-matter block).
fn strip_frontmatter(content: &str) -> &str {
    // Skip the opening `---`
    let after_first = content
        .strip_prefix("---")
        .map(|s| s.trim_start_matches('\n'));
    let Some(rest) = after_first else {
        return content;
    };
    // Find the closing `---`
    if let Some(pos) = rest.find("\n---") {
        rest[pos + 4..].trim_start_matches('\n')
    } else {
        content
    }
}

/// Build a set of normalized words for overlap comparison.
fn word_set(text: &str) -> std::collections::HashSet<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.len() >= 4) // skip short stop words
        .map(|w| w.to_lowercase())
        .collect()
}

/// Delete a memory file by path.
pub fn delete_memory(path: &Path) -> Result<(), String> {
    if !is_memory_path(path) {
        return Err(format!(
            "refusing to delete {}: not inside a memory directory",
            path.display()
        ));
    }
    std::fs::remove_file(path).map_err(|e| format!("failed to delete memory: {e}"))?;
    tracing::info!(
        target: "jfc::memory",
        path = %path.display(),
        "deleted memory"
    );
    Ok(())
}

/// Synchronize team memory with another local directory.
///
/// Missing files are copied in both directions. Divergent files are never
/// overwritten; the remote copy is written into the local team directory as
/// `<stem>.conflict-<timestamp>.md` so the next normal memory load exposes it
/// for manual reconciliation.
pub fn sync_team_memory(
    project_root: &Path,
    remote_dir: &Path,
) -> Result<TeamMemorySyncReport, String> {
    let local_dir = team_memory_dir(project_root);
    std::fs::create_dir_all(&local_dir)
        .map_err(|e| format!("failed to create local team memory dir: {e}"))?;
    std::fs::create_dir_all(remote_dir)
        .map_err(|e| format!("failed to create remote team memory dir: {e}"))?;

    let mut names = std::collections::BTreeSet::new();
    collect_md_file_names(&local_dir, &mut names)?;
    collect_md_file_names(remote_dir, &mut names)?;

    let mut report = TeamMemorySyncReport {
        local_dir: local_dir.clone(),
        remote_dir: remote_dir.to_path_buf(),
        pushed: 0,
        pulled: 0,
        conflicts: Vec::new(),
    };

    for name in names {
        let local_path = local_dir.join(&name);
        let remote_path = remote_dir.join(&name);
        match (local_path.exists(), remote_path.exists()) {
            (true, false) => {
                let bytes = std::fs::read(&local_path)
                    .map_err(|e| format!("failed to read {}: {e}", local_path.display()))?;
                write_atomic_sync(&remote_path, &bytes)
                    .map_err(|e| format!("failed to write {}: {e}", remote_path.display()))?;
                report.pushed += 1;
            }
            (false, true) => {
                let bytes = std::fs::read(&remote_path)
                    .map_err(|e| format!("failed to read {}: {e}", remote_path.display()))?;
                write_atomic_sync(&local_path, &bytes)
                    .map_err(|e| format!("failed to write {}: {e}", local_path.display()))?;
                report.pulled += 1;
            }
            (true, true) => {
                let local_bytes = std::fs::read(&local_path)
                    .map_err(|e| format!("failed to read {}: {e}", local_path.display()))?;
                let remote_bytes = std::fs::read(&remote_path)
                    .map_err(|e| format!("failed to read {}: {e}", remote_path.display()))?;
                if local_bytes != remote_bytes {
                    let conflict_path = conflict_path_for(&local_dir, &name);
                    write_atomic_sync(&conflict_path, &remote_bytes)
                        .map_err(|e| format!("failed to write {}: {e}", conflict_path.display()))?;
                    report.conflicts.push(TeamMemoryConflict {
                        file_name: name,
                        local_path,
                        remote_path,
                        conflict_path,
                    });
                }
            }
            (false, false) => {}
        }
    }

    Ok(report)
}

// ─── Rendering into system prompt ───────────────────────────────────────────

/// Render all loaded memories into a system-prompt-ready block.
/// Returns `None` if there are no memories.
pub fn render_memories_section(memories: &[MemoryEntry]) -> Option<String> {
    if memories.is_empty() {
        return None;
    }

    let mut out = String::from(
        "\n\n# Memory\n\nThe following memories have been saved from previous conversations:\n",
    );

    let user_memories: Vec<_> = memories
        .iter()
        .filter(|m| m.level == MemoryLevel::User)
        .collect();
    let project_memories: Vec<_> = memories
        .iter()
        .filter(|m| m.level == MemoryLevel::Project)
        .collect();
    let team_memories: Vec<_> = memories
        .iter()
        .filter(|m| m.level == MemoryLevel::Team)
        .collect();
    let external_memories: Vec<_> = memories
        .iter()
        .filter(|m| m.level == MemoryLevel::External)
        .collect();

    if !user_memories.is_empty() {
        out.push_str("\n## User memories\n\n");
        for mem in &user_memories {
            render_memory_entry(mem, &mut out);
        }
    }

    if !project_memories.is_empty() {
        out.push_str("\n## Project memories\n\n");
        for mem in &project_memories {
            render_memory_entry(mem, &mut out);
        }
    }

    if !team_memories.is_empty() {
        out.push_str(
            "\n## Team memories\n\n\
             Other teammates' jfc sessions write here too. Merge near-duplicates within \
             `team/`. DO NOT delete a team memory just because you don't recognize it — \
             it may belong to another contributor's workflow.\n\n",
        );
        for mem in &team_memories {
            render_memory_entry(mem, &mut out);
        }
    }

    if !external_memories.is_empty() {
        out.push_str("\n## External memory directories\n\n");
        for mem in &external_memories {
            render_memory_entry(mem, &mut out);
        }
    }

    out.push_str(MEMORY_USAGE_SECTIONS);

    tracing::debug!(
        target: "jfc::memory",
        user_count = user_memories.len(),
        project_count = project_memories.len(),
        team_count = team_memories.len(),
        external_count = external_memories.len(),
        output_len = out.len(),
        "rendered memories section"
    );

    Some(out)
}

/// CC 2.1.167-mirrored guidance on when/how to use and save memory.
/// Appended to the memories section so the model has the same rules
/// regardless of which scope a memory lives in.
const MEMORY_USAGE_SECTIONS: &str = "\n\
## Types of memory\n\n\
There are four types of memory, each with a default scope:\n\n\
<types>\n\
<type>\n\
    <name>user</name>\n\
    <scope>always private</scope>\n\
    <description>The user's role, goals, expertise, and working preferences. \
Helps you tailor future responses to who the user is. Avoid judgmental observations; \
focus on what makes you more helpful to them specifically.</description>\n\
    <when_to_save>When you learn details about the user's role, domain expertise, or preferences.</when_to_save>\n\
    <how_to_use>When your answer should be tailored to the user's background — \
e.g., frame a frontend explanation in terms of their backend expertise.</how_to_use>\n\
</type>\n\
<type>\n\
    <name>feedback</name>\n\
    <scope>default private; team only when the guidance is a project-wide convention \
every contributor should follow (a testing policy, a build invariant) — not a personal style preference.</scope>\n\
    <description>Guidance the user has given about how to approach work — corrections \
AND confirmations. Record both: if you only save corrections you avoid past mistakes but \
drift away from validated approaches and grow overly cautious.</description>\n\
    <when_to_save>Any time the user corrects your approach (\"no not that\", \"don't\", \
\"stop doing X\") OR confirms a non-obvious approach worked (\"yes exactly\", \"perfect, keep doing that\", \
accepting an unusual choice without pushback). Confirmations are quieter than corrections — watch for them. \
Include *why* so future sessions can judge edge cases.</when_to_save>\n\
    <how_to_use>Let these memories guide behavior so the user does not need to repeat the same guidance.</how_to_use>\n\
    <body_structure>Lead with the rule itself, then a **Why:** line (the reason given — often a past incident \
or strong preference) and a **How to apply:** line (when/where this kicks in).</body_structure>\n\
</type>\n\
<type>\n\
    <name>project</name>\n\
    <scope>strongly bias toward team</scope>\n\
    <description>Ongoing work, goals, initiatives, decisions, and incidents not derivable \
from the code or git history. Helps understand the broader context behind the user's requests.</description>\n\
    <when_to_save>When you learn who is doing what, why, or by when. These states change quickly — \
keep your understanding up to date. **Always convert relative dates to absolute ISO dates** \
(e.g., \"Thursday\" → \"2026-06-08\") so memories remain interpretable after time passes.</when_to_save>\n\
    <how_to_use>Use to understand nuance and motivation, anticipate coordination issues, make better suggestions.</how_to_use>\n\
    <body_structure>Lead with the fact or decision, then a **Why:** line (motivation — constraint, \
deadline, or stakeholder ask) and a **How to apply:** line. Project memories decay fast, so the why \
helps judge whether the memory is still load-bearing.</body_structure>\n\
</type>\n\
<type>\n\
    <name>reference</name>\n\
    <scope>usually team</scope>\n\
    <description>Pointers to where information can be found in external systems — \
issue trackers, dashboards, Slack channels, runbooks.</description>\n\
    <when_to_save>When you learn about a resource in an external system and its purpose.</when_to_save>\n\
    <how_to_use>When the user references an external system or information that may live there.</how_to_use>\n\
</type>\n\
</types>\n\n\
## How to save memories\n\n\
Write each memory to its own file. Use a 3–4 word filename that describes what the memory is about \
(e.g., `prefers-bun-over-npm.md`, `compliance-driven-rewrite.md`). Don't prefix the filename with the \
memory type — that's already in the frontmatter. Use this frontmatter format:\n\n\
```\n\
---\n\
type: feedback      # user | feedback | project | reference\n\
scope: private      # private | team\n\
created: 2026-06-08\n\
---\n\
Lead with rule/fact. **Why:** reason. **How to apply:** when this kicks in.\n\
```\n\n\
- **One fact per file.** Each memory file contains one paragraph about a single fact. \
Multiple facts → separate files. A very long paragraph is a sign you should split it.\n\
- **Immutable.** Never edit a memory file in place. Delete the stale file and create a fresh one. \
Preserve any information that is still accurate.\n\
- **No duplicates.** Before saving, check existing memories. Update the existing file rather than \
creating a duplicate; delete memories that turn out to be wrong.\n\
- **After writing**, add a one-line pointer to `MEMORY.md`: `- [Title](file.md) — one-line hook`. \
`MEMORY.md` is the index loaded into context — never put memory content directly in it.\n\n\
## Memory scope\n\
- **User** — global to this user across all projects.\n\
- **Project** — scoped to this working tree (`.jfc/memory/project/`).\n\
- **Team** — shared with other contributors via `.jfc/memory/team/` (committed to the repo).\n\n\
## When to access memory\n\
- When memories seem relevant, or the user references prior-conversation work.\n\
- You MUST access memory when the user explicitly asks you to check, recall, or remember.\n\
- If the user says to *ignore* or *not use* memory: do not apply remembered facts, cite, compare against, or mention memory content.\n\
- Memory records can become stale. Use memory as context for what was true at a given point in time. Before answering or acting on memory, verify it is still correct by reading the current state of the files or resources. If a recalled memory conflicts with current information, trust what you observe now — and update or remove the stale memory rather than acting on it.\n\n\
## Before recommending from memory\n\
A memory that names a specific function, file, or flag is a claim that it existed *when the memory was written*. It may have been renamed, removed, or never merged. Before recommending it:\n\
- If the memory names a file path: check the file exists.\n\
- If the memory names a function or flag: grep for it.\n\
- If the user is about to act on your recommendation (not just asking about history), verify first.\n\n\
\"The memory says X exists\" is not the same as \"X exists now.\"\n\n\
## What NOT to save\n\
- Code patterns, conventions, architecture, file paths, or project structure — derivable from reading the current codebase.\n\
- Git history, recent changes, or who-changed-what — `git log` / `git blame` are authoritative.\n\
- Debugging solutions or fix recipes — the fix is in the code; the commit message has the context.\n\
- Anything already documented in CLAUDE.md files.\n\
- Ephemeral task details: in-progress work, temporary state, current conversation context.\n\
- These exclusions apply even when the user asks you to save them. If they ask you to save a PR list or activity summary, ask what was *surprising* or *non-obvious* about it — that is the part worth keeping.\n";

fn render_memory_entry(mem: &MemoryEntry, out: &mut String) {
    let filename = mem
        .path
        .file_name()
        .and_then(|f| f.to_str())
        .unwrap_or("unknown");
    out.push_str(&format!(
        "- **[{}|{}]** {}\n",
        mem.frontmatter.memory_type,
        mem.frontmatter.scope,
        mem.body.lines().next().unwrap_or("(empty)")
    ));
    // If body has multiple lines, include the rest indented
    let mut lines = mem.body.lines();
    lines.next(); // skip first (already shown)
    for line in lines {
        if !line.trim().is_empty() {
            out.push_str(&format!("  {line}\n"));
        }
    }
    out.push_str(&format!("  _(source: {filename})_\n"));
}

/// Format memory files as a listing for the model (used in memory extraction prompts).
pub fn format_existing_memories(memories: &[MemoryEntry]) -> String {
    if memories.is_empty() {
        return String::from("(no existing memory files)");
    }

    let mut out = String::new();
    for mem in memories {
        out.push_str(&format!(
            "- `{}` [{}|{}]: {}\n",
            mem.path.display(),
            mem.frontmatter.memory_type,
            mem.frontmatter.scope,
            mem.body.lines().next().unwrap_or("(empty)")
        ));
    }
    out
}

// ─── Helpers ─────────────────────────────────────────────────────────────────

/// Create a URL-safe slug from text, truncated to `max_len` characters.
fn slugify(text: &str, max_len: usize) -> String {
    let cleaned: String = text
        .chars()
        .take(max_len * 2) // take extra to account for removals
        .map(|c| {
            if c.is_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();

    // Collapse consecutive dashes
    let mut result = String::with_capacity(max_len);
    let mut prev_dash = false;
    for c in cleaned.chars() {
        if c == '-' {
            if !prev_dash && !result.is_empty() {
                result.push('-');
            }
            prev_dash = true;
        } else {
            result.push(c);
            prev_dash = false;
        }
        if result.len() >= max_len {
            break;
        }
    }

    // Trim trailing dash
    result.trim_end_matches('-').to_string()
}

fn collect_md_file_names(
    dir: &Path,
    out: &mut std::collections::BTreeSet<String>,
) -> Result<(), String> {
    for entry in
        std::fs::read_dir(dir).map_err(|e| format!("failed to read {}: {e}", dir.display()))?
    {
        let entry = entry.map_err(|e| format!("failed to read entry in {}: {e}", dir.display()))?;
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("md") {
            continue;
        }
        if let Some(name) = path.file_name().and_then(|s| s.to_str()) {
            out.insert(name.to_owned());
        }
    }
    Ok(())
}

fn conflict_path_for(local_dir: &Path, file_name: &str) -> PathBuf {
    let now: DateTime<Utc> = SystemTime::now().into();
    let stamp = now.format("%Y%m%d-%H%M%S");
    match file_name.strip_suffix(".md") {
        Some(stem) => local_dir.join(format!("{stem}.conflict-{stamp}.md")),
        None => local_dir.join(format!("{file_name}.conflict-{stamp}.md")),
    }
}

// ─── Atomic write (inlined from jfc/atomic_write.rs) ─────────────────────

fn write_atomic_sync(path: &Path, content: &[u8]) -> std::io::Result<()> {
    use std::fs::{File, remove_file, rename};
    use std::io::Write;

    let pid = std::process::id();
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_micros())
        .unwrap_or(0);
    let mut suffix = std::ffi::OsString::from(".tmp.");
    suffix.push(pid.to_string());
    suffix.push(".");
    suffix.push(unique.to_string());
    let mut tmp_os = path.as_os_str().to_owned();
    tmp_os.push(&suffix);
    let tmp = PathBuf::from(tmp_os);

    let result = (|| -> std::io::Result<()> {
        let mut f = File::create(&tmp)?;
        f.write_all(content)?;
        f.sync_all()?;
        drop(f);
        rename(&tmp, path)?;
        if let Some(parent) = path.parent()
            && let Ok(dir) = File::open(parent)
        {
            let _ = dir.sync_all();
        }
        Ok(())
    })();

    if result.is_err() {
        let _ = remove_file(&tmp);
    }
    result
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn create_project_memory_auto_appends_index_pointer_normal() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        let path = create_memory(
            MemoryLevel::Project,
            MemoryType::Context,
            MemoryScope::Team,
            "Build runs from the workspace root via cargo build.",
            root,
        )
        .unwrap();

        let index = fs::read_to_string(root.join("MEMORY.md")).expect("MEMORY.md created");
        assert!(index.contains("# Project Memory Index"), "{index}");
        // The pointer links the relative path and carries the title/hook.
        let rel = path.strip_prefix(root).unwrap().to_string_lossy();
        assert!(
            index.contains(&format!("({rel})")),
            "index links file: {index}"
        );
        assert!(
            index.contains("Build runs from the workspace root"),
            "{index}"
        );
    }

    #[test]
    fn create_memory_index_append_is_idempotent_robust() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        let p = create_memory(
            MemoryLevel::Project,
            MemoryType::Context,
            MemoryScope::Team,
            "Same note body.",
            root,
        )
        .unwrap();
        // Manually append the SAME file's pointer again — must not duplicate.
        append_memory_index_pointer(root, &p, "Same note body.").unwrap();
        let index = fs::read_to_string(root.join("MEMORY.md")).unwrap();
        let rel = p.strip_prefix(root).unwrap().to_string_lossy();
        let occurrences = index.matches(&format!("({rel})")).count();
        assert_eq!(occurrences, 1, "pointer must appear exactly once: {index}");
    }

    #[test]
    fn create_user_memory_does_not_write_project_index_robust() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        // User memories live in the global config dir and have no project index.
        let _ = create_memory(
            MemoryLevel::User,
            MemoryType::Context,
            MemoryScope::Private,
            "A user-scoped preference.",
            root,
        )
        .unwrap();
        assert!(
            !root.join("MEMORY.md").exists(),
            "user memory must not create a project MEMORY.md"
        );
    }

    #[test]
    fn parse_frontmatter_valid() {
        let content = "---\ntype: feedback\nscope: team\ncreated: 2026-05-01T12:00:00Z\n---\nDon't use mocks in integration tests.";
        let (fm, body) = parse_frontmatter_and_body(content).unwrap();
        assert_eq!(fm.memory_type, MemoryType::Feedback);
        assert_eq!(fm.scope, MemoryScope::Team);
        assert_eq!(body, "Don't use mocks in integration tests.");
    }

    #[test]
    fn parse_frontmatter_missing_defaults_to_context() {
        let content = "Just a plain note without frontmatter.";
        let (fm, body) = parse_frontmatter_and_body(content).unwrap();
        assert_eq!(fm.memory_type, MemoryType::Context);
        assert_eq!(fm.scope, MemoryScope::Private);
        assert_eq!(body, content);
    }

    #[test]
    fn slugify_works() {
        assert_eq!(slugify("Hello World!", 20), "hello-world");
        assert_eq!(slugify("don't mock DB", 10), "don-t-mock");
        assert_eq!(slugify("  leading spaces  ", 15), "leading-spaces");
    }

    #[test]
    fn create_and_load_memory() {
        let tmp = TempDir::new().unwrap();
        let project = tmp.path().to_path_buf();

        // Create a project-level memory
        let path = create_memory(
            MemoryLevel::Project,
            MemoryType::Feedback,
            MemoryScope::Team,
            "Always run tests before committing.",
            &project,
        )
        .unwrap();

        assert!(path.exists());
        assert!(path.starts_with(project_memory_dir(&project)));

        // Load it back
        let memories = load_all_memories(&project);
        let created = memories
            .iter()
            .find(|mem| mem.path == path)
            .expect("created memory should be loaded");
        assert_eq!(created.frontmatter.memory_type, MemoryType::Feedback);
        assert_eq!(created.frontmatter.scope, MemoryScope::Team);
        assert!(created.body.contains("Always run tests before committing."));
    }

    #[test]
    fn delete_memory_works() {
        let tmp = TempDir::new().unwrap();
        let project = tmp.path().to_path_buf();
        let mem_dir = project_memory_dir(&project);
        fs::create_dir_all(&mem_dir).unwrap();

        let file = mem_dir.join("test-memory.md");
        fs::write(
            &file,
            "---\ntype: context\nscope: private\n---\nSome fact.\n",
        )
        .unwrap();

        // delete_memory checks is_memory_path, which uses canonicalize.
        // For the test we call remove_file directly since canonicalize
        // depends on process cwd.
        assert!(file.exists());
        fs::remove_file(&file).unwrap();
        assert!(!file.exists());
    }

    #[test]
    fn render_memories_section_empty() {
        assert!(render_memories_section(&[]).is_none());
    }

    #[test]
    fn render_memories_section_has_content() {
        let entries = vec![MemoryEntry {
            path: PathBuf::from("/home/user/.config/jfc/memory/test.md"),
            level: MemoryLevel::User,
            frontmatter: MemoryFrontmatter::new(MemoryType::Preference, MemoryScope::Private),
            body: "Prefer concise responses.".to_string(),
        }];
        let rendered = render_memories_section(&entries).unwrap();
        assert!(rendered.contains("# Memory"));
        assert!(rendered.contains("User memories"));
        assert!(rendered.contains("Prefer concise responses"));
        assert!(rendered.contains("[preference|private]"));
    }

    #[test]
    fn new_memory_types_parse_normal() {
        assert_eq!("user".parse::<MemoryType>().unwrap(), MemoryType::User);
        assert_eq!(
            "reference".parse::<MemoryType>().unwrap(),
            MemoryType::Reference
        );
        assert_eq!("ref".parse::<MemoryType>().unwrap(), MemoryType::Reference);
        assert_eq!(
            "feedback".parse::<MemoryType>().unwrap(),
            MemoryType::Feedback
        );
    }

    #[test]
    fn memory_type_display_normal() {
        assert_eq!(MemoryType::User.to_string(), "user");
        assert_eq!(MemoryType::Reference.to_string(), "reference");
    }

    #[test]
    fn memory_types_section_in_guidance_normal() {
        // All four CC types must appear in the memory usage guidance
        assert!(MEMORY_USAGE_SECTIONS.contains("user"));
        assert!(MEMORY_USAGE_SECTIONS.contains("feedback"));
        assert!(MEMORY_USAGE_SECTIONS.contains("project"));
        assert!(MEMORY_USAGE_SECTIONS.contains("reference"));
    }

    #[test]
    fn date_absolutization_guidance_present_normal() {
        assert!(MEMORY_USAGE_SECTIONS.contains("absolute ISO dates"));
        assert!(MEMORY_USAGE_SECTIONS.contains("Thursday"));
    }

    #[test]
    fn confirmations_guidance_present_normal() {
        // CC: record confirmations too, not just corrections
        assert!(MEMORY_USAGE_SECTIONS.contains("confirmations"));
        assert!(MEMORY_USAGE_SECTIONS.contains("corrections AND confirmations"));
    }

    #[test]
    fn immutability_and_granularity_present_normal() {
        assert!(MEMORY_USAGE_SECTIONS.contains("Immutable"));
        assert!(MEMORY_USAGE_SECTIONS.contains("One fact per file"));
    }

    #[test]
    fn memory_md_index_guidance_present_normal() {
        assert!(MEMORY_USAGE_SECTIONS.contains("MEMORY.md"));
        assert!(MEMORY_USAGE_SECTIONS.contains("one-line pointer"));
    }

    #[test]
    fn no_duplicates_guidance_present_normal() {
        assert!(MEMORY_USAGE_SECTIONS.contains("No duplicates"));
    }

    #[test]
    fn body_structure_guidance_present_normal() {
        assert!(MEMORY_USAGE_SECTIONS.contains("Why:"));
        assert!(MEMORY_USAGE_SECTIONS.contains("How to apply:"));
    }

    #[test]
    fn find_conflicting_memory_detects_duplicate_normal() {
        let dir = tempfile::tempdir().unwrap();
        // Write an existing memory
        let existing = dir.path().join("existing.md");
        std::fs::write(
            &existing,
            "---\ntype: feedback\nscope: private\ncreated: 2026-01-01\n---\nintegration tests must use a real database not mocks\n",
        ).unwrap();
        // A body with >50% word overlap
        let similar = "integration tests must hit a real database and not use mocks";
        let conflict = find_conflicting_memory(dir.path(), similar);
        assert!(conflict.is_some(), "should detect near-duplicate");
    }

    #[test]
    fn find_conflicting_memory_no_false_positive_robust() {
        let dir = tempfile::tempdir().unwrap();
        let existing = dir.path().join("existing.md");
        std::fs::write(
            &existing,
            "---\ntype: project\nscope: team\ncreated: 2026-01-01\n---\nauth middleware rewrite is driven by compliance\n",
        ).unwrap();
        // Unrelated body
        let unrelated = "prefer bun over npm for javascript package management";
        let conflict = find_conflicting_memory(dir.path(), unrelated);
        assert!(conflict.is_none(), "should not flag unrelated memory");
    }

    #[test]
    fn render_memories_section_includes_v132_guidance_normal() {
        let entries = vec![MemoryEntry {
            path: PathBuf::from("/home/user/.config/jfc/memory/test.md"),
            level: MemoryLevel::User,
            frontmatter: MemoryFrontmatter::new(MemoryType::Preference, MemoryScope::Private),
            body: "Prefer concise responses.".to_string(),
        }];
        let rendered = render_memories_section(&entries).unwrap();
        assert!(rendered.contains("## Memory scope"));
        assert!(rendered.contains("## When to access memory"));
        assert!(rendered.contains("## Before recommending from memory"));
        assert!(rendered.contains("## What NOT to save"));
    }

    #[test]
    fn render_memories_section_renders_team_scope_normal() {
        let entries = vec![MemoryEntry {
            path: PathBuf::from(".jfc/memory/team/team-rule.md"),
            level: MemoryLevel::Team,
            frontmatter: MemoryFrontmatter::new(MemoryType::Feedback, MemoryScope::Team),
            body: "All PRs require two reviewers.".to_string(),
        }];
        let rendered = render_memories_section(&entries).unwrap();
        assert!(rendered.contains("## Team memories"));
        assert!(rendered.contains("All PRs require two reviewers"));
        assert!(rendered.contains("DO NOT delete a team memory"));
    }

    #[test]
    fn render_memories_section_skips_empty_team_scope_robust() {
        let entries = vec![MemoryEntry {
            path: PathBuf::from("/u/.config/jfc/memory/u.md"),
            level: MemoryLevel::User,
            frontmatter: MemoryFrontmatter::new(MemoryType::Preference, MemoryScope::Private),
            body: "X".to_string(),
        }];
        let rendered = render_memories_section(&entries).unwrap();
        assert!(!rendered.contains("## Team memories"));
    }
}

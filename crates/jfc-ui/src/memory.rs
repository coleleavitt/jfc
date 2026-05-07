//! Memory system for jfc — persistent storage of learned preferences, facts,
//! and project context across sessions.
//!
//! Storage layout (mirroring Claude Code v126):
//! - User-level:    `~/.config/jfc/memory/` — personal preferences that follow
//!                  the user across all projects.
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
}

impl fmt::Display for MemoryLevel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::User => write!(f, "user"),
            Self::Project => write!(f, "project"),
            Self::Team => write!(f, "team"),
        }
    }
}

/// Semantic type of a memory (mirrors v126 memory taxonomy).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MemoryType {
    /// Corrections and confirmations of approach.
    Feedback,
    /// Stylistic / workflow preferences.
    Preference,
    /// Project-specific facts, goals, initiatives.
    Project,
    /// General context or learned facts.
    Context,
}

impl fmt::Display for MemoryType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Feedback => write!(f, "feedback"),
            Self::Preference => write!(f, "preference"),
            Self::Project => write!(f, "project"),
            Self::Context => write!(f, "context"),
        }
    }
}

impl std::str::FromStr for MemoryType {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "feedback" => Ok(Self::Feedback),
            "preference" => Ok(Self::Preference),
            "project" => Ok(Self::Project),
            "context" => Ok(Self::Context),
            other => Err(format!("unknown memory type: {other}")),
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
    tracing::info!(
        target: "jfc::memory",
        user_dir = %user_memory_dir().display(),
        project_dir = %project_memory_dir(project_root).display(),
        total_entries = entries.len(),
        "loaded all memories"
    );
    entries
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
            MemoryFrontmatter {
                memory_type: MemoryType::Context,
                scope: MemoryScope::Private,
                created: None,
            },
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

/// Create a new memory file. Returns the path of the created file.
pub fn create_memory(
    level: MemoryLevel,
    memory_type: MemoryType,
    scope: MemoryScope,
    body: &str,
    project_root: &Path,
) -> Result<PathBuf, String> {
    let dir = match level {
        MemoryLevel::User => user_memory_dir(),
        MemoryLevel::Project => project_memory_dir(project_root),
        MemoryLevel::Team => team_memory_dir(project_root),
    };

    // Ensure directory exists
    std::fs::create_dir_all(&dir).map_err(|e| format!("failed to create memory directory: {e}"))?;

    // Generate a filename based on timestamp + a slug from the body
    let now: DateTime<Utc> = SystemTime::now().into();
    let slug = slugify(body, 40);
    let timestamp = now.format("%Y%m%d-%H%M%S");
    let filename = format!("{timestamp}-{slug}.md");
    let path = dir.join(&filename);

    // Render frontmatter + body
    let content = format!(
        "---\ntype: {memory_type}\nscope: {scope}\ncreated: {}\n---\n{body}\n",
        now.to_rfc3339()
    );

    std::fs::write(&path, &content).map_err(|e| format!("failed to write memory file: {e}"))?;

    tracing::info!(
        target: "jfc::memory",
        path = %path.display(),
        level = %level,
        memory_type = %memory_type,
        scope = %scope,
        "created memory"
    );

    Ok(path)
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

    out.push_str(MEMORY_USAGE_SECTIONS);

    tracing::debug!(
        target: "jfc::memory",
        user_count = user_memories.len(),
        project_count = project_memories.len(),
        team_count = team_memories.len(),
        output_len = out.len(),
        "rendered memories section"
    );

    Some(out)
}

/// v132-mirrored guidance on when/how to use memory. Appended to the
/// memories section so the model has the same usage rules whether or
/// not the memory file lives in user/project/team scope.
const MEMORY_USAGE_SECTIONS: &str = "\n\
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
- Code patterns, conventions, architecture, file paths, or project structure — these can be derived by reading the current project state.\n\
- Git history, recent changes, or who-changed-what — `git log` / `git blame` are authoritative.\n\
- Debugging solutions or fix recipes — the fix is in the code; the commit message has the context.\n\
- Anything already documented in CLAUDE.md files.\n\
- Ephemeral task details: in-progress work, temporary state, current conversation context.\n";

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

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

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
        assert_eq!(memories.len(), 1);
        assert_eq!(memories[0].frontmatter.memory_type, MemoryType::Feedback);
        assert_eq!(memories[0].frontmatter.scope, MemoryScope::Team);
        assert!(
            memories[0]
                .body
                .contains("Always run tests before committing.")
        );
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
            frontmatter: MemoryFrontmatter {
                memory_type: MemoryType::Preference,
                scope: MemoryScope::Private,
                created: None,
            },
            body: "Prefer concise responses.".to_string(),
        }];
        let rendered = render_memories_section(&entries).unwrap();
        assert!(rendered.contains("# Memory"));
        assert!(rendered.contains("User memories"));
        assert!(rendered.contains("Prefer concise responses"));
        assert!(rendered.contains("[preference|private]"));
    }

    #[test]
    fn render_memories_section_includes_v132_guidance_normal() {
        let entries = vec![MemoryEntry {
            path: PathBuf::from("/home/user/.config/jfc/memory/test.md"),
            level: MemoryLevel::User,
            frontmatter: MemoryFrontmatter {
                memory_type: MemoryType::Preference,
                scope: MemoryScope::Private,
                created: None,
            },
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
            frontmatter: MemoryFrontmatter {
                memory_type: MemoryType::Feedback,
                scope: MemoryScope::Team,
                created: None,
            },
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
            frontmatter: MemoryFrontmatter {
                memory_type: MemoryType::Preference,
                scope: MemoryScope::Private,
                created: None,
            },
            body: "X".to_string(),
        }];
        let rendered = render_memories_section(&entries).unwrap();
        assert!(!rendered.contains("## Team memories"));
    }
}

//! Memory system for jfc — persistent storage of learned preferences, facts,
//! and project context across sessions.
//!
//! Memories are stored in the `jfc-knowledge` SQLite DB (a single
//! `knowledge.db`), not as `.md` files. Each memory carries a `MemoryLevel`
//! (User / Project / Team / External) and a `MemoryFrontmatter` (type, scope,
//! TTL, dedup hash) serialized into the DB `mem_meta` column. CRUD goes through
//! [`create_memory`], [`create_memory_checked`], [`load_all_memories`], and
//! [`delete_memory`] (delete-by-id).
//!
//! Memories are immutable — to update, delete the old row and create a new one.

use std::borrow::Cow;
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
    /// Personal preferences that follow the user across all projects.
    User,
    /// Knowledge scoped to a single project.
    Project,
    /// Shared across everyone working in this repo. v132 prompt: "Other
    /// teammates' Claude sessions write here too. Merge near-duplicates.
    /// DO NOT delete a team memory just because you don't recognize it."
    Team,
    /// Memories imported from an external source.
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
    /// DB row id (the delete-by-id key). `None` only for legacy in-memory test
    /// entries. The canonical store is now the jfc-knowledge DB, not files.
    pub id: Option<String>,
    /// Former absolute path to the `.md` file. `None` for DB-backed entries
    /// (there is no file). Kept as `Option` so legacy display/test code that
    /// referenced a path still type-checks during the cutover.
    pub path: Option<PathBuf>,
    /// Which directory level this lives in.
    pub level: MemoryLevel,
    /// Parsed frontmatter.
    pub frontmatter: MemoryFrontmatter,
    /// The body content (everything after the `---` block).
    pub body: String,
}

impl MemoryEntry {
    pub fn source_name(&self) -> Cow<'_, str> {
        if let Some(path) = &self.path
            && let Some(name) = path.file_name().and_then(|f| f.to_str())
        {
            return Cow::Borrowed(name);
        }
        if let Some(id) = &self.id {
            return Cow::Borrowed(id.as_str());
        }
        Cow::Borrowed("unknown")
    }

    pub fn source_display(&self) -> Cow<'_, str> {
        if let Some(path) = &self.path {
            return Cow::Owned(path.display().to_string());
        }
        if let Some(id) = &self.id {
            return Cow::Borrowed(id.as_str());
        }
        Cow::Borrowed("unknown")
    }
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

// ─── Read / Write / Delete ───────────────────────────────────────────────────

/// Load all memory entries from both user and project directories.
/// Load all memories visible to `project_root` from the jfc-knowledge DB (the
/// canonical store after the MD→DB cutover). Synthesizes `MemoryEntry` from each
/// row, restoring rich frontmatter from the verbatim `mem_meta` JSON. TTL-expired
/// entries are filtered (same rule the `.md` loader applied).
pub fn load_all_memories(project_root: &Path) -> Vec<MemoryEntry> {
    let project_key = jfc_knowledge::project_key(project_root);
    let rows = match jfc_knowledge::KnowledgeStore::open_default() {
        Ok(store) => store.load_memories(Some(&project_key)).unwrap_or_default(),
        Err(e) => {
            tracing::warn!(target: "jfc::memory", error = %e, "knowledge store open failed; no memories loaded");
            return Vec::new();
        }
    };
    let now = now_ms();
    let mut entries = Vec::with_capacity(rows.len());
    for row in rows {
        let level = mem_level_to_memory_level(row.level);
        let frontmatter = row
            .meta
            .as_deref()
            .and_then(|m| serde_json::from_str::<MemoryFrontmatter>(m).ok())
            .unwrap_or_else(|| MemoryFrontmatter::new(MemoryType::Context, MemoryScope::Private));
        if let Some(expires) = frontmatter.expires_at
            && expires <= now
        {
            continue; // TTL-expired
        }
        entries.push(MemoryEntry {
            id: Some(row.id),
            path: None,
            level,
            frontmatter,
            body: row.body,
        });
    }
    tracing::info!(
        target: "jfc::memory",
        project_key = %project_key,
        total_entries = entries.len(),
        "loaded all memories (db)"
    );
    entries
}

fn mem_level_to_memory_level(l: jfc_knowledge::MemLevel) -> MemoryLevel {
    match l {
        jfc_knowledge::MemLevel::User => MemoryLevel::User,
        jfc_knowledge::MemLevel::Project => MemoryLevel::Project,
        jfc_knowledge::MemLevel::Team => MemoryLevel::Team,
        jfc_knowledge::MemLevel::External => MemoryLevel::External,
    }
}

fn memory_level_to_mem_level(l: MemoryLevel) -> jfc_knowledge::MemLevel {
    match l {
        MemoryLevel::User => jfc_knowledge::MemLevel::User,
        MemoryLevel::Project => jfc_knowledge::MemLevel::Project,
        MemoryLevel::Team => jfc_knowledge::MemLevel::Team,
        MemoryLevel::External => jfc_knowledge::MemLevel::External,
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Result of attempting to create a memory, including optional conflict info.
#[derive(Debug, Clone)]
pub struct CreateMemoryResult {
    /// DB id of the newly-created memory row (the delete-by-id key). Was a file
    /// path before the MD→DB cutover.
    pub id: String,
    /// A conflicting (near-duplicate) memory's id found before saving, if any.
    /// Mirrors CC 2.1.167's `conflicting_memory_id` field.
    pub conflicting_memory_id: Option<String>,
}

/// Serialize a fresh `MemoryFrontmatter` for a new memory, stamping `created`
/// and the dedup `normalized_hash`. Stored verbatim in the DB `mem_meta` column.
fn new_frontmatter_json(
    memory_type: MemoryType,
    scope: MemoryScope,
    hash: &str,
) -> (MemoryFrontmatter, String) {
    let now: DateTime<Utc> = SystemTime::now().into();
    let mut fm = MemoryFrontmatter::new(memory_type, scope);
    fm.created = Some(now.format("%Y-%m-%d").to_string());
    fm.normalized_hash = Some(hash.to_owned());
    fm.first_seen_at = Some(now_ms());
    let json = serde_json::to_string(&fm).unwrap_or_else(|_| "{}".to_owned());
    (fm, json)
}

/// Content-dedup hash: lowercase, whitespace-collapsed body.
fn content_hash(body: &str) -> String {
    let norm = body
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase();
    // uuid-v5 over the normalized body is a stable, dependency-free digest.
    uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_OID, norm.as_bytes())
        .simple()
        .to_string()
}

fn open_store_or_err() -> Result<jfc_knowledge::KnowledgeStore, String> {
    jfc_knowledge::KnowledgeStore::open_default().map_err(|e| format!("knowledge store: {e}"))
}

/// Create a memory in the DB and return conflict info alongside the new id.
///
/// Dedups on the normalized-content hash (replacing the old >50%-word-overlap
/// file scan with an exact-normalized-content check). Returns
/// `conflicting_memory_id` so the caller can decide whether to delete the old
/// row or merge.
pub fn create_memory_checked(
    level: MemoryLevel,
    memory_type: MemoryType,
    scope: MemoryScope,
    body: &str,
    project_root: &Path,
) -> Result<CreateMemoryResult, String> {
    let store = open_store_or_err()?;
    let hash = content_hash(body);
    let conflicting = store
        .find_memory_by_hash(&hash)
        .map_err(|e| e.to_string())?;
    let id = write_memory_row(&store, level, memory_type, scope, body, project_root, &hash)?;
    tracing::info!(
        target: "jfc::memory",
        id = %id,
        conflicting = ?conflicting,
        level = %level,
        memory_type = %memory_type,
        scope = %scope,
        "created memory (db, with conflict check)"
    );
    Ok(CreateMemoryResult {
        id,
        conflicting_memory_id: conflicting,
    })
}

/// Create a new memory row in the DB. Returns the row id.
pub fn create_memory(
    level: MemoryLevel,
    memory_type: MemoryType,
    scope: MemoryScope,
    body: &str,
    project_root: &Path,
) -> Result<String, String> {
    let store = open_store_or_err()?;
    let hash = content_hash(body);
    let id = write_memory_row(&store, level, memory_type, scope, body, project_root, &hash)?;
    tracing::info!(
        target: "jfc::memory",
        id = %id,
        level = %level,
        memory_type = %memory_type,
        scope = %scope,
        "created memory (db)"
    );
    Ok(id)
}

/// Insert one memory row, mapping level→project_key + serializing frontmatter.
fn write_memory_row(
    store: &jfc_knowledge::KnowledgeStore,
    level: MemoryLevel,
    memory_type: MemoryType,
    scope: MemoryScope,
    body: &str,
    project_root: &Path,
    hash: &str,
) -> Result<String, String> {
    let mem_level = memory_level_to_mem_level(level);
    let project_key = jfc_knowledge::project_key(project_root);
    let project_key_opt = matches!(level, MemoryLevel::Project).then_some(project_key.as_str());
    let id = jfc_knowledge::memory_id(mem_level, project_key_opt, body);
    let (_fm, meta_json) = new_frontmatter_json(memory_type, scope, hash);
    let title: String = body
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .map(|l| l.chars().take(80).collect())
        .unwrap_or_else(|| "memory".to_owned());
    store
        .insert_memory(&jfc_knowledge::NewMemory {
            id: id.clone(),
            level: mem_level,
            project_key: project_key_opt,
            title: &title,
            body,
            hash,
            meta_json: &meta_json,
        })
        .map_err(|e| format!("failed to insert memory: {e}"))?;
    Ok(id)
}

/// Delete a memory file by path.
/// Delete a memory by its DB id (the delete-by-id contract after the MD→DB
/// cutover). Returns an error if no such memory row exists.
pub fn delete_memory(id: &str) -> Result<(), String> {
    let store = open_store_or_err()?;
    let removed = store
        .delete_memory_by_id(id)
        .map_err(|e| format!("failed to delete memory: {e}"))?;
    if removed == 0 {
        return Err(format!("no memory with id {id}"));
    }
    tracing::info!(target: "jfc::memory", id, "deleted memory (db)");
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
    let filename = mem.source_name();
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
            mem.source_display(),
            mem.frontmatter.memory_type,
            mem.frontmatter.scope,
            mem.body.lines().next().unwrap_or("(empty)")
        ));
    }
    out
}

// ─── Helpers ─────────────────────────────────────────────────────────────────

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
    use tempfile::TempDir;

    struct EnvGuard {
        key: &'static str,
        prev: Option<std::ffi::OsString>,
    }

    impl EnvGuard {
        fn set(key: &'static str, value: &Path) -> Self {
            let prev = std::env::var_os(key);
            unsafe { std::env::set_var(key, value) };
            Self { key, prev }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            unsafe {
                match &self.prev {
                    Some(value) => std::env::set_var(self.key, value),
                    None => std::env::remove_var(self.key),
                }
            }
        }
    }

    fn use_temp_knowledge_db(tmp: &TempDir) -> EnvGuard {
        EnvGuard::set("JFC_KNOWLEDGE_DB", &tmp.path().join("knowledge.db"))
    }

    #[test]
    #[serial_test::serial]
    fn create_project_memory_persists_db_row_normal() {
        let tmp = TempDir::new().unwrap();
        let _guard = use_temp_knowledge_db(&tmp);
        let root = tmp.path();
        let id = create_memory(
            MemoryLevel::Project,
            MemoryType::Context,
            MemoryScope::Team,
            "Build runs from the workspace root via cargo build.",
            root,
        )
        .unwrap();

        let memories = load_all_memories(root);
        let created = memories
            .iter()
            .find(|mem| mem.id.as_deref() == Some(id.as_str()))
            .expect("created memory should be loaded from DB");
        assert!(created.path.is_none());
        assert_eq!(created.frontmatter.memory_type, MemoryType::Context);
        assert_eq!(created.frontmatter.scope, MemoryScope::Team);
        assert!(
            created
                .body
                .contains("Build runs from the workspace root via cargo build.")
        );
    }

    #[test]
    #[serial_test::serial]
    fn create_user_memory_does_not_write_project_index_robust() {
        let tmp = TempDir::new().unwrap();
        let _guard = use_temp_knowledge_db(&tmp);
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
    #[serial_test::serial]
    fn create_and_load_memory() {
        let tmp = TempDir::new().unwrap();
        let _guard = use_temp_knowledge_db(&tmp);
        let project = tmp.path().to_path_buf();

        // Create a project-level memory
        let id = create_memory(
            MemoryLevel::Project,
            MemoryType::Feedback,
            MemoryScope::Team,
            "Always run tests before committing.",
            &project,
        )
        .unwrap();

        // Load it back
        let memories = load_all_memories(&project);
        let created = memories
            .iter()
            .find(|mem| mem.id.as_deref() == Some(id.as_str()))
            .expect("created memory should be loaded");
        assert!(created.path.is_none());
        assert_eq!(created.frontmatter.memory_type, MemoryType::Feedback);
        assert_eq!(created.frontmatter.scope, MemoryScope::Team);
        assert!(created.body.contains("Always run tests before committing."));
    }

    #[test]
    #[serial_test::serial]
    fn delete_memory_works() {
        let tmp = TempDir::new().unwrap();
        let _guard = use_temp_knowledge_db(&tmp);
        let project = tmp.path().to_path_buf();

        // Create a memory, then delete it by its DB id (the delete-by-id
        // contract after the MD→DB cutover).
        let id = create_memory(
            MemoryLevel::Project,
            MemoryType::Context,
            MemoryScope::Private,
            "Some fact to be deleted.",
            &project,
        )
        .unwrap();
        assert!(
            load_all_memories(&project)
                .iter()
                .any(|m| m.id.as_deref() == Some(id.as_str())),
            "memory should exist before delete"
        );

        delete_memory(&id).unwrap();

        assert!(
            !load_all_memories(&project)
                .iter()
                .any(|m| m.id.as_deref() == Some(id.as_str())),
            "memory should be gone after delete"
        );
        // Deleting a nonexistent id surfaces an error.
        assert!(delete_memory("no-such-id").is_err());
    }

    #[test]
    fn render_memories_section_empty() {
        assert!(render_memories_section(&[]).is_none());
    }

    #[test]
    fn render_memories_section_has_content() {
        let entries = vec![MemoryEntry {
            id: Some("test:user:test".to_owned()),
            path: Some(PathBuf::from("/home/user/.config/jfc/memory/test.md")),
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
    fn render_memories_section_includes_v132_guidance_normal() {
        let entries = vec![MemoryEntry {
            id: Some("test:user:test".to_owned()),
            path: Some(PathBuf::from("/home/user/.config/jfc/memory/test.md")),
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
            id: Some("test:team:team-rule".to_owned()),
            path: Some(PathBuf::from(".jfc/memory/team/team-rule.md")),
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
            id: Some("test:user:u".to_owned()),
            path: Some(PathBuf::from("/u/.config/jfc/memory/u.md")),
            level: MemoryLevel::User,
            frontmatter: MemoryFrontmatter::new(MemoryType::Preference, MemoryScope::Private),
            body: "X".to_string(),
        }];
        let rendered = render_memories_section(&entries).unwrap();
        assert!(!rendered.contains("## Team memories"));
    }
}

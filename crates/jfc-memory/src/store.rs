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
use std::str::FromStr;
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
pub async fn load_all_memories(project_root: &Path) -> Vec<MemoryEntry> {
    if let Err(e) = import_legacy_memory_dirs(project_root).await {
        tracing::warn!(target: "jfc::memory", error = %e, "legacy memory import failed");
    }
    let project_key = jfc_knowledge::project_key(project_root);
    let rows = match jfc_knowledge::KnowledgeStore::open_default().await {
        Ok(store) => store.load_memories(Some(&project_key)).await.unwrap_or_default(),
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

async fn open_store_or_err() -> Result<jfc_knowledge::KnowledgeStore, String> {
    jfc_knowledge::KnowledgeStore::open_default().await.map_err(|e| format!("knowledge store: {e}"))
}

async fn import_legacy_memory_dirs(project_root: &Path) -> Result<(), String> {
    import_memory_dir_to_db(
        project_root,
        &user_memory_dir(),
        MemoryLevel::User,
        MemoryScope::Private,
    ).await?;
    import_memory_dir_to_db(
        project_root,
        &project_memory_dir(project_root),
        MemoryLevel::Project,
        MemoryScope::Private,
    ).await?;
    import_memory_dir_to_db(
        project_root,
        &team_memory_dir(project_root),
        MemoryLevel::Team,
        MemoryScope::Team,
    ).await?;
    Ok(())
}

/// Create a memory in the DB and return conflict info alongside the new id.
///
/// Dedups on the normalized-content hash (replacing the old >50%-word-overlap
/// file scan with an exact-normalized-content check). Returns
/// `conflicting_memory_id` so the caller can decide whether to delete the old
/// row or merge.
pub async fn create_memory_checked(
    level: MemoryLevel,
    memory_type: MemoryType,
    scope: MemoryScope,
    body: &str,
    project_root: &Path,
) -> Result<CreateMemoryResult, String> {
    let store = open_store_or_err().await?;
    let hash = content_hash(body);
    let conflicting = store
        .find_memory_by_hash(&hash)
        .await
        .map_err(|e| e.to_string())?;
    let id = write_memory_row(&store, level, memory_type, scope, body, project_root, &hash).await?;
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
pub async fn create_memory(
    level: MemoryLevel,
    memory_type: MemoryType,
    scope: MemoryScope,
    body: &str,
    project_root: &Path,
) -> Result<String, String> {
    let store = open_store_or_err().await?;
    let hash = content_hash(body);
    let id = write_memory_row(&store, level, memory_type, scope, body, project_root, &hash).await?;
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
async fn write_memory_row(
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
    let project_key_opt =
        matches!(level, MemoryLevel::Project | MemoryLevel::Team).then_some(project_key.as_str());
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
        .await
        .map_err(|e| format!("failed to insert memory: {e}"))?;
    Ok(id)
}

/// Delete a memory file by path.
/// Delete a memory by its DB id (the delete-by-id contract after the MD→DB
/// cutover). Returns an error if no such memory row exists.
pub async fn delete_memory(id: &str) -> Result<(), String> {
    let store = open_store_or_err().await?;
    let removed = store
        .delete_memory_by_id(id)
        .await
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
pub async fn sync_team_memory(
    project_root: &Path,
    remote_dir: &Path,
) -> Result<TeamMemorySyncReport, String> {
    let local_dir = team_memory_dir(project_root);
    std::fs::create_dir_all(&local_dir)
        .map_err(|e| format!("failed to create local team memory dir: {e}"))?;
    std::fs::create_dir_all(remote_dir)
        .map_err(|e| format!("failed to create remote team memory dir: {e}"))?;
    export_team_db_memories(project_root, &local_dir).await?;

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

    import_memory_dir_to_db(
        project_root,
        &local_dir,
        MemoryLevel::Team,
        MemoryScope::Team,
    ).await?;

    Ok(report)
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

async fn export_team_db_memories(project_root: &Path, local_dir: &Path) -> Result<(), String> {
    for mem in load_all_memories(project_root)
        .await
        .into_iter()
        .filter(|m| m.level == MemoryLevel::Team)
    {
        let name = exported_team_memory_file_name(&mem);
        let body = memory_entry_to_markdown(&mem);
        write_atomic_sync(&local_dir.join(name), body.as_bytes())
            .map_err(|e| format!("failed to export team memory: {e}"))?;
    }
    Ok(())
}

async fn import_memory_dir_to_db(
    project_root: &Path,
    local_dir: &Path,
    level: MemoryLevel,
    default_scope: MemoryScope,
) -> Result<(), String> {
    let Ok(entries) = std::fs::read_dir(local_dir) else {
        return Ok(());
    };
    for entry in entries {
        let entry =
            entry.map_err(|e| format!("failed to read entry in {}: {e}", local_dir.display()))?;
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("md") {
            continue;
        }
        let content = std::fs::read_to_string(&path)
            .map_err(|e| format!("failed to read {}: {e}", path.display()))?;
        let (frontmatter, body) = parse_memory_markdown(&content, default_scope);
        if body.trim().is_empty() {
            continue;
        }
        if let Err(e) = create_memory_checked(
            level,
            frontmatter.memory_type,
            frontmatter.scope,
            body.trim(),
            project_root,
        ).await {
            tracing::warn!(
                target: "jfc::memory",
                path = %path.display(),
                error = %e,
                "memory import skipped"
            );
        }
    }
    Ok(())
}

fn exported_team_memory_file_name(mem: &MemoryEntry) -> String {
    let first_line = mem
        .body
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or("team-memory");
    let stem = slug_file_stem(first_line);
    let suffix = mem
        .id
        .as_deref()
        .map(|id| id.chars().take(8).collect::<String>())
        .unwrap_or_else(|| content_hash(&mem.body).chars().take(8).collect());
    format!("{stem}-{suffix}.md")
}

fn memory_entry_to_markdown(mem: &MemoryEntry) -> String {
    let mut out = format!(
        "---\ntype: {}\nscope: {}\n",
        mem.frontmatter.memory_type, mem.frontmatter.scope
    );
    if let Some(created) = &mem.frontmatter.created {
        out.push_str(&format!("created: {created}\n"));
    }
    out.push_str("---\n");
    out.push_str(mem.body.trim());
    out.push('\n');
    out
}

fn parse_memory_markdown(content: &str, default_scope: MemoryScope) -> (MemoryFrontmatter, &str) {
    let mut frontmatter = MemoryFrontmatter::new(MemoryType::Context, default_scope);
    let Some(rest) = content
        .strip_prefix("---\n")
        .or_else(|| content.strip_prefix("---\r\n"))
    else {
        return (frontmatter, content);
    };
    let mut offset = 0usize;
    for line in rest.split_inclusive('\n') {
        let trimmed = line.trim_end_matches(['\n', '\r']);
        if trimmed == "---" {
            let body = &rest[offset + line.len()..];
            return (frontmatter, body);
        }
        if let Some((key, value)) = trimmed.split_once(':') {
            let key = key.trim().to_ascii_lowercase();
            let value = value.trim().trim_matches('"').trim_matches('\'');
            match key.as_str() {
                "type" => {
                    if let Ok(kind) = MemoryType::from_str(value) {
                        frontmatter.memory_type = kind;
                    }
                }
                "scope" => {
                    if let Ok(scope) = MemoryScope::from_str(value) {
                        frontmatter.scope = scope;
                    }
                }
                "created" if !value.is_empty() => frontmatter.created = Some(value.to_owned()),
                _ => {}
            }
        }
        offset += line.len();
    }
    (frontmatter, content)
}

fn slug_file_stem(text: &str) -> String {
    let mut out = String::new();
    let mut prev_dash = false;
    for ch in text.chars().flat_map(char::to_lowercase) {
        if ch.is_ascii_alphanumeric() {
            out.push(ch);
            prev_dash = false;
        } else if !prev_dash && !out.is_empty() {
            out.push('-');
            prev_dash = true;
        }
        if out.len() >= 48 {
            break;
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    if out.is_empty() {
        "team-memory".to_owned()
    } else {
        out
    }
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

    #[tokio::test]
    #[serial_test::serial]
    async fn create_project_memory_persists_db_row_normal() {
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
        .await
        .unwrap();

        let memories = load_all_memories(root).await;
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

    #[tokio::test]
    #[serial_test::serial]
    async fn create_user_memory_does_not_write_project_index_robust() {
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
        .await
        .unwrap();
        assert!(
            !root.join("MEMORY.md").exists(),
            "user memory must not create a project MEMORY.md"
        );
    }

    #[tokio::test]
    #[serial_test::serial]
    async fn create_and_load_memory() {
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
        .await
        .unwrap();

        // Load it back
        let memories = load_all_memories(&project).await;
        let created = memories
            .iter()
            .find(|mem| mem.id.as_deref() == Some(id.as_str()))
            .expect("created memory should be loaded");
        assert!(created.path.is_none());
        assert_eq!(created.frontmatter.memory_type, MemoryType::Feedback);
        assert_eq!(created.frontmatter.scope, MemoryScope::Team);
        assert!(created.body.contains("Always run tests before committing."));
    }

    #[tokio::test]
    #[serial_test::serial]
    async fn delete_memory_works() {
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
        .await
        .unwrap();
        assert!(
            load_all_memories(&project)
                .await
                .iter()
                .any(|m| m.id.as_deref() == Some(id.as_str())),
            "memory should exist before delete"
        );

        delete_memory(&id).await.unwrap();

        assert!(
            !load_all_memories(&project)
                .await
                .iter()
                .any(|m| m.id.as_deref() == Some(id.as_str())),
            "memory should be gone after delete"
        );
        // Deleting a nonexistent id surfaces an error.
        assert!(delete_memory("no-such-id").await.is_err());
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
}

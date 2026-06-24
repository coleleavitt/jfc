//! DB-backed memory store (MD→DB cutover). The canonical home for user/project/
//! team/external memories — replacing the per-file `.md` layout. Frontmatter is
//! preserved verbatim in `mem_meta` (serialized JSON) so a higher layer can
//! synthesize its rich `MemoryEntry`/`MemoryFrontmatter` losslessly, while
//! `body`/`title`/`tags` stay queryable via the same `knowledge` table + FTS.
//!
//! Delete is **by id** (the row's stable uuid), not by filesystem path — there
//! are no files. Creation dedups on a caller-supplied content hash stored in
//! `tags` (the `.md` layout used `normalized_hash` frontmatter for the same job).

use rusqlite::{OptionalExtension, params};

use crate::error::{KnowledgeError, Result};
use crate::record::now_ms;
use crate::redact::redact;

/// The four memory levels the `.md` layout encoded by directory. Distinct from
/// the coarser knowledge `Scope` (user/project/global), kept in its own column.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemLevel {
    User,
    Project,
    Team,
    External,
}

impl MemLevel {
    pub fn slug(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Project => "project",
            Self::Team => "team",
            Self::External => "external",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "user" => Some(Self::User),
            "project" => Some(Self::Project),
            "team" => Some(Self::Team),
            "external" => Some(Self::External),
            _ => None,
        }
    }
}

/// A memory row read back from the DB. `meta` is the verbatim serialized
/// frontmatter JSON (opaque here; the engine deserializes it).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryRow {
    pub id: String,
    pub level: MemLevel,
    pub project_key: Option<String>,
    pub body: String,
    pub meta: Option<String>,
}

/// What to write. `id` is supplied by the caller (deterministic from content)
/// so creation is idempotent; `hash` dedups (stored in `tags`).
pub struct NewMemory<'a> {
    pub id: String,
    pub level: MemLevel,
    pub project_key: Option<&'a str>,
    pub title: &'a str,
    pub body: &'a str,
    pub hash: &'a str,
    pub meta_json: &'a str,
}

impl super::KnowledgeStore {
    /// Insert (or replace) a memory row. Idempotent on `id`. Memories live in the
    /// `knowledge` table with `kind='preference'`-agnostic semantics: we tag the
    /// row as a memory via `mem_level`/`mem_meta` being non-NULL.
    pub fn insert_memory(&self, m: &NewMemory<'_>) -> Result<()> {
        if redact(m.title, false) != m.title || redact(m.body, false) != m.body {
            return Err(KnowledgeError::InvalidRecord(
                "memory contains sensitive material".into(),
            ));
        }
        // Map level → knowledge Scope for the recall/promotion machinery.
        let scope = match m.level {
            MemLevel::User => "user",
            MemLevel::Team | MemLevel::External => "global",
            MemLevel::Project => "project",
        };
        let project_key = match m.level {
            MemLevel::Project | MemLevel::Team => m.project_key,
            MemLevel::User | MemLevel::External => None,
        };
        self.conn().execute(
            "INSERT INTO knowledge \
               (id, kind, scope, project_key, title, body, tags, source, confidence, \
                created_at_ms, last_used_ms, use_count, superseded_by, promoted, \
                outcome, importance, mem_level, mem_meta) \
             VALUES (?1,'fact',?2,?3,?4,?5,?6,'memory',0.7,?7,NULL,0,NULL,0,'unverified',0.7,?8,?9) \
             ON CONFLICT(id) DO UPDATE SET \
               body=excluded.body, title=excluded.title, tags=excluded.tags, \
               mem_level=excluded.mem_level, mem_meta=excluded.mem_meta",
            params![
                m.id, scope, project_key, m.title, m.body, m.hash,
                now_ms(), m.level.slug(), m.meta_json,
            ],
        )?;
        Ok(())
    }

    /// Whether a memory with this content hash already exists (dedup check that
    /// replaces `find_conflicting_memory`'s filename scan). Returns the row id.
    pub fn find_memory_by_hash(&self, hash: &str) -> Result<Option<String>> {
        Ok(self
            .conn()
            .query_row(
                "SELECT id FROM knowledge WHERE mem_level IS NOT NULL AND tags = ?1 \
                 AND superseded_by IS NULL LIMIT 1",
                [hash],
                |r| r.get::<_, String>(0),
            )
            .optional()?)
    }

    /// Load all live memory rows visible to a project: user + external
    /// (global-ish) plus this project's project/team-scoped memories.
    pub fn load_memories(&self, project_key: Option<&str>) -> Result<Vec<MemoryRow>> {
        let mut stmt = self.conn().prepare(
            "SELECT id, mem_level, project_key, body, mem_meta FROM knowledge \
             WHERE mem_level IS NOT NULL AND superseded_by IS NULL \
               AND (mem_level IN ('user','external') \
                    OR (mem_level IN ('project','team') AND project_key IS ?1)) \
             ORDER BY created_at_ms ASC",
        )?;
        let rows = stmt.query_map([project_key], |r| {
            let level_s: String = r.get(1)?;
            Ok(MemoryRow {
                id: r.get(0)?,
                level: MemLevel::parse(&level_s).unwrap_or(MemLevel::User),
                project_key: r.get(2)?,
                body: r.get(3)?,
                meta: r.get(4)?,
            })
        })?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    /// Delete a memory by id (the delete-by-id contract). Returns rows removed.
    pub fn delete_memory_by_id(&self, id: &str) -> Result<usize> {
        Ok(self.conn().execute(
            "DELETE FROM knowledge WHERE id = ?1 AND mem_level IS NOT NULL",
            [id],
        )?)
    }

    /// Count of live memory rows — tests/metrics.
    pub fn memory_count(&self) -> Result<i64> {
        Ok(self.conn().query_row(
            "SELECT count(*) FROM knowledge WHERE mem_level IS NOT NULL AND superseded_by IS NULL",
            [],
            |r| r.get(0),
        )?)
    }
}

/// Stable id for a memory from its (level, project, normalized body) — so a
/// re-import / re-create of the same content maps to one row.
pub fn memory_id(level: MemLevel, project_key: Option<&str>, body: &str) -> String {
    let norm = body.split_whitespace().collect::<Vec<_>>().join(" ");
    let basis = format!("mem:{}:{}:{norm}", level.slug(), project_key.unwrap_or(""));
    uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_OID, basis.as_bytes())
        .simple()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::KnowledgeStore;

    fn newmem<'a>(
        level: MemLevel,
        proj: Option<&'a str>,
        body: &'a str,
        hash: &'a str,
        meta: &'a str,
    ) -> (String, NewMemory<'a>) {
        let id = memory_id(level, proj, body);
        (
            id.clone(),
            NewMemory {
                id,
                level,
                project_key: proj,
                title: "t",
                body,
                hash,
                meta_json: meta,
            },
        )
    }

    #[test]
    fn insert_load_delete_roundtrip_normal() {
        let store = KnowledgeStore::open_in_memory().unwrap();
        let (uid, um) = newmem(MemLevel::User, None, "prefer ripgrep", "h1", "{\"k\":1}");
        store.insert_memory(&um).unwrap();
        let (_pid, pm) = newmem(MemLevel::Project, Some("P"), "uses vite", "h2", "{}");
        store.insert_memory(&pm).unwrap();

        // From project P: sees user + project-P memory.
        let rows = store.load_memories(Some("P")).unwrap();
        assert_eq!(rows.len(), 2);
        // meta round-trips.
        let user = rows.iter().find(|r| r.id == uid).unwrap();
        assert_eq!(user.meta.as_deref(), Some("{\"k\":1}"));

        // From a different project: project-P memory is NOT visible; user is.
        let other = store.load_memories(Some("Q")).unwrap();
        assert_eq!(other.len(), 1);
        assert_eq!(other[0].level, MemLevel::User);

        // Delete by id.
        assert_eq!(store.delete_memory_by_id(&uid).unwrap(), 1);
        assert_eq!(store.memory_count().unwrap(), 1);
    }

    #[test]
    fn dedup_by_hash_normal() {
        let store = KnowledgeStore::open_in_memory().unwrap();
        let (_id, m) = newmem(MemLevel::User, None, "x", "hashA", "{}");
        store.insert_memory(&m).unwrap();
        assert!(store.find_memory_by_hash("hashA").unwrap().is_some());
        assert!(store.find_memory_by_hash("nope").unwrap().is_none());
    }

    #[test]
    fn team_memory_is_project_scoped_but_external_is_global_regression() {
        let store = KnowledgeStore::open_in_memory().unwrap();
        let (_t, tm) = newmem(MemLevel::Team, Some("P"), "team rule", "ht", "{}");
        let (_e, em) = newmem(MemLevel::External, None, "ext note", "he", "{}");
        store.insert_memory(&tm).unwrap();
        store.insert_memory(&em).unwrap();

        let project_rows = store.load_memories(Some("P")).unwrap();
        assert_eq!(project_rows.len(), 2);
        assert!(
            project_rows
                .iter()
                .any(|row| row.level == MemLevel::Team && row.project_key.as_deref() == Some("P"))
        );

        let other_rows = store.load_memories(Some("Q")).unwrap();
        assert_eq!(other_rows.len(), 1);
        assert_eq!(other_rows[0].level, MemLevel::External);
    }

    #[test]
    fn memory_id_is_stable_and_content_sensitive_robust() {
        assert_eq!(
            memory_id(MemLevel::User, None, "a  b"),
            memory_id(MemLevel::User, None, "a b"),
        );
        assert_ne!(
            memory_id(MemLevel::User, None, "a"),
            memory_id(MemLevel::Project, Some("p"), "a"),
        );
    }
}

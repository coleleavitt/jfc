use serde::{Deserialize, Serialize};
use sqlx::Row;

use crate::record::now_ms;
use crate::{KnowledgeStore, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DefinitionScope {
    Builtin,
    User,
    Project,
    Plugin,
    Global,
}

impl DefinitionScope {
    pub const fn slug(self) -> &'static str {
        match self {
            Self::Builtin => "builtin",
            Self::User => "user",
            Self::Project => "project",
            Self::Plugin => "plugin",
            Self::Global => "global",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DefinitionStatus {
    Active,
    Candidate,
    Rejected,
    Superseded,
}

impl DefinitionStatus {
    pub const fn slug(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Candidate => "candidate",
            Self::Rejected => "rejected",
            Self::Superseded => "superseded",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DefinitionRecord {
    pub id: String,
    pub kind: String,
    pub scope: String,
    pub project_key: Option<String>,
    pub namespace: Option<String>,
    pub name: String,
    pub title: Option<String>,
    pub description: Option<String>,
    pub body: String,
    pub metadata_json: String,
    pub source_path: Option<String>,
    pub source_hash: Option<String>,
    pub status: String,
    pub version: i64,
    pub created_by: String,
    pub created_at_ms: u64,
    pub updated_at_ms: u64,
    pub superseded_by: Option<String>,
}

#[derive(Debug, Clone)]
pub struct NewDefinition {
    pub kind: String,
    pub scope: DefinitionScope,
    pub project_key: Option<String>,
    pub namespace: Option<String>,
    pub name: String,
    pub title: Option<String>,
    pub description: Option<String>,
    pub body: String,
    pub metadata_json: String,
    pub source_path: Option<String>,
    pub source_hash: Option<String>,
    pub status: DefinitionStatus,
    pub created_by: String,
}

impl NewDefinition {
    pub fn id(&self) -> String {
        definition_id(
            &self.kind,
            self.scope.slug(),
            self.project_key.as_deref(),
            self.namespace.as_deref(),
            &self.name,
        )
    }
}

pub fn definition_id(
    kind: &str,
    scope: &str,
    project_key: Option<&str>,
    namespace: Option<&str>,
    name: &str,
) -> String {
    uuid::Uuid::new_v5(
        &uuid::Uuid::NAMESPACE_OID,
        format!(
            "definition:{kind}:{scope}:{}:{}:{name}",
            project_key.unwrap_or(""),
            namespace.unwrap_or("")
        )
        .as_bytes(),
    )
    .simple()
    .to_string()
}

fn row_to_definition(row: &sqlx::sqlite::SqliteRow) -> Result<DefinitionRecord> {
    Ok(DefinitionRecord {
        id: row.try_get(0)?,
        kind: row.try_get(1)?,
        scope: row.try_get(2)?,
        project_key: row.try_get(3)?,
        namespace: row.try_get(4)?,
        name: row.try_get(5)?,
        title: row.try_get(6)?,
        description: row.try_get(7)?,
        body: row.try_get(8)?,
        metadata_json: row.try_get(9)?,
        source_path: row.try_get(10)?,
        source_hash: row.try_get(11)?,
        status: row.try_get(12)?,
        version: row.try_get(13)?,
        created_by: row.try_get(14)?,
        created_at_ms: row.try_get(15)?,
        updated_at_ms: row.try_get(16)?,
        superseded_by: row.try_get(17)?,
    })
}

impl KnowledgeStore {
    /// Update a definition's lifecycle status (Candidate → Active on promotion,
    /// or Active → Candidate/Rejected on rollback). Returns the number of rows
    /// changed — `0` means no such definition (so callers can tell a real
    /// promotion from a no-op).
    pub async fn set_definition_status(&self, id: &str, status: &str) -> Result<u64> {
        let result =
            sqlx::query("UPDATE definitions SET status = ?2, updated_at_ms = ?3 WHERE id = ?1")
                .bind(id)
                .bind(status)
                .bind(now_ms())
                .execute(&self.pool)
                .await?;
        Ok(result.rows_affected())
    }

    pub async fn upsert_definition(&self, def: &NewDefinition) -> Result<String> {
        let id = def.id();
        let now = now_ms();
        sqlx::query(
            "INSERT INTO definitions (
                id, kind, scope, project_key, namespace, name, title, description,
                body, metadata_json, source_path, source_hash, status, version,
                created_by, created_at_ms, updated_at_ms, superseded_by
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, 1, ?14, ?15, ?15, NULL)
            ON CONFLICT(id) DO UPDATE SET
                title = excluded.title,
                description = excluded.description,
                body = excluded.body,
                metadata_json = excluded.metadata_json,
                source_path = excluded.source_path,
                source_hash = excluded.source_hash,
                -- Don't let a re-stage demote a promoted definition. RSI/
                -- self-critique re-`upsert`s the same definition every session
                -- with status=candidate; without this guard that silently
                -- clobbers an already-promoted `active` def back to candidate,
                -- and since the backlog item is already `applied` it is never
                -- re-promoted — so the active RSI count collapsed to 0 every
                -- save. Only the specific candidate←active downgrade is blocked;
                -- an intentional active→superseded rollback still applies.
                status = CASE
                    WHEN definitions.status = 'active' AND excluded.status = 'candidate'
                    THEN definitions.status
                    ELSE excluded.status
                END,
                created_by = excluded.created_by,
                updated_at_ms = excluded.updated_at_ms,
                version = CASE
                    WHEN definitions.body != excluded.body
                         OR definitions.metadata_json != excluded.metadata_json
                         OR COALESCE(definitions.description, '') != COALESCE(excluded.description, '')
                    THEN definitions.version + 1
                    ELSE definitions.version
                END,
                superseded_by = NULL"
        )
            .bind(&id)
            .bind(&def.kind)
            .bind(def.scope.slug())
            .bind(&def.project_key)
            .bind(&def.namespace)
            .bind(&def.name)
            .bind(&def.title)
            .bind(&def.description)
            .bind(&def.body)
            .bind(&def.metadata_json)
            .bind(&def.source_path)
            .bind(&def.source_hash)
            .bind(def.status.slug())
            .bind(&def.created_by)
            .bind(now)
            .execute(&self.pool)
            .await?;
        Ok(id)
    }

    pub async fn get_definition_by_name(
        &self,
        kind: &str,
        scope: DefinitionScope,
        project_key: Option<&str>,
        namespace: Option<&str>,
        name: &str,
    ) -> Result<Option<DefinitionRecord>> {
        let id = definition_id(kind, scope.slug(), project_key, namespace, name);
        let row = sqlx::query(
            "SELECT id, kind, scope, project_key, namespace, name, title, description,
                    body, metadata_json, source_path, source_hash, status, version,
                    created_by, created_at_ms, updated_at_ms, superseded_by
             FROM definitions
             WHERE id = ?1 AND superseded_by IS NULL",
        )
        .bind(&id)
        .fetch_optional(&self.pool)
        .await?;
        row.map(|r| row_to_definition(&r)).transpose()
    }

    pub async fn list_definitions_for_project(
        &self,
        kind: &str,
        project_key: &str,
    ) -> Result<Vec<DefinitionRecord>> {
        let rows = sqlx::query(
            "SELECT id, kind, scope, project_key, namespace, name, title, description,
                    body, metadata_json, source_path, source_hash, status, version,
                    created_by, created_at_ms, updated_at_ms, superseded_by
             FROM definitions
             WHERE kind = ?1
               AND status = 'active'
               AND superseded_by IS NULL
               AND (project_key IS NULL OR project_key = ?2)
             ORDER BY updated_at_ms ASC",
        )
        .bind(kind)
        .bind(project_key)
        .fetch_all(&self.pool)
        .await?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row_to_definition(&row)?);
        }
        Ok(out)
    }

    pub async fn list_definitions_for_project_status(
        &self,
        kind: &str,
        project_key: &str,
        status: &str,
        limit: usize,
    ) -> Result<Vec<DefinitionRecord>> {
        let limit = i64::try_from(limit).unwrap_or(i64::MAX);
        let rows = sqlx::query(
            "SELECT id, kind, scope, project_key, namespace, name, title, description,
                    body, metadata_json, source_path, source_hash, status, version,
                    created_by, created_at_ms, updated_at_ms, superseded_by
             FROM definitions
             WHERE kind = ?1
               AND status = ?3
               AND superseded_by IS NULL
               AND (project_key IS NULL OR project_key = ?2)
             ORDER BY updated_at_ms DESC
             LIMIT ?4",
        )
        .bind(kind)
        .bind(project_key)
        .bind(status)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row_to_definition(&row)?);
        }
        Ok(out)
    }

    /// Like [`list_definitions_for_project_status`], but across ALL projects
    /// (and global scope) — for RSI self-improvements, which are not bound to a
    /// single project_key. Newest first, capped by `limit`.
    ///
    /// [`list_definitions_for_project_status`]: Self::list_definitions_for_project_status
    pub async fn list_definitions_for_project_status_any_project(
        &self,
        kind: &str,
        status: &str,
        limit: usize,
    ) -> Result<Vec<DefinitionRecord>> {
        let limit = i64::try_from(limit).unwrap_or(i64::MAX);
        let rows = sqlx::query(
            "SELECT id, kind, scope, project_key, namespace, name, title, description,
                    body, metadata_json, source_path, source_hash, status, version,
                    created_by, created_at_ms, updated_at_ms, superseded_by
             FROM definitions
             WHERE kind = ?1
               AND status = ?2
               AND superseded_by IS NULL
             ORDER BY updated_at_ms DESC
             LIMIT ?3",
        )
        .bind(kind)
        .bind(status)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row_to_definition(&row)?);
        }
        Ok(out)
    }

    /// Count RSI definitions grouped by `(kind, status)` in one pass. An RSI
    /// definition is one whose `source_path` starts with `rsi:` or whose
    /// metadata carries an `"rsi"` key (mirrors the runtime's `is_rsi_definition`
    /// classifier). Powers the dashboard's candidate→active funnel without
    /// loading every row. Counts across all projects (RSI self-improvements are
    /// global), excluding superseded rows.
    pub async fn rsi_definition_counts(&self) -> Result<Vec<RsiDefinitionCount>> {
        let rows = sqlx::query(
            "SELECT kind, status, COUNT(*) AS n
             FROM definitions
             WHERE superseded_by IS NULL
               AND (source_path LIKE 'rsi:%' OR metadata_json LIKE '%\"rsi\"%')
             GROUP BY kind, status",
        )
        .fetch_all(&self.pool)
        .await?;
        let mut out = Vec::with_capacity(rows.len());
        for row in rows {
            use sqlx::Row;
            out.push(RsiDefinitionCount {
                kind: row.try_get::<String, _>("kind")?,
                status: row.try_get::<String, _>("status")?,
                count: u64::try_from(row.try_get::<i64, _>("n")?).unwrap_or(0),
            });
        }
        Ok(out)
    }
}

/// One `(kind, status)` tally of RSI definitions, from [`rsi_definition_counts`].
///
/// [`rsi_definition_counts`]: KnowledgeStore::rsi_definition_counts
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RsiDefinitionCount {
    pub kind: String,
    pub status: String,
    pub count: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(name: &str, body: &str) -> NewDefinition {
        NewDefinition {
            kind: "skill".to_owned(),
            scope: DefinitionScope::Project,
            project_key: Some("proj".to_owned()),
            namespace: None,
            name: name.to_owned(),
            title: None,
            description: Some("desc".to_owned()),
            body: body.to_owned(),
            metadata_json: "{}".to_owned(),
            source_path: Some("db:definition".to_owned()),
            source_hash: Some("abc".to_owned()),
            status: DefinitionStatus::Active,
            created_by: "test".to_owned(),
        }
    }

    fn sample_with_status(name: &str, status: DefinitionStatus) -> NewDefinition {
        NewDefinition {
            status,
            ..sample(name, "body")
        }
    }

    fn rsi_candidate(name: &str, kind: &str) -> NewDefinition {
        NewDefinition {
            kind: kind.to_owned(),
            scope: DefinitionScope::Global,
            project_key: None,
            namespace: Some("self_critique".to_owned()),
            name: name.to_owned(),
            title: Some(name.to_owned()),
            description: Some("d".to_owned()),
            body: "guidance body".to_owned(),
            metadata_json: r#"{"rsi":{"source":"self_critique"}}"#.to_owned(),
            source_path: Some("rsi:definition:self_critique:system_prompt".to_owned()),
            source_hash: None,
            status: DefinitionStatus::Candidate,
            created_by: "self_critique".to_owned(),
        }
    }

    // Regression: a re-stage (status=candidate) must NOT demote an already
    // promoted (active) definition. This was the RSI "0 active forever" bug:
    // every session save re-upserted the candidate and clobbered the promoted
    // active status back to candidate. The candidate←active downgrade is the
    // only transition blocked; everything else still applies.
    #[tokio::test]
    async fn upsert_does_not_demote_active_to_candidate_regression() {
        let store = KnowledgeStore::open_in_memory().await.unwrap();
        let candidate = rsi_candidate("self-critique-verify-first", "system_prompt");
        let id = store.upsert_definition(&candidate).await.unwrap();
        // Promote it (what promote_evidenced_self_critique does).
        assert_eq!(store.set_definition_status(&id, "active").await.unwrap(), 1);
        // Next session re-stages the SAME definition as a candidate.
        store.upsert_definition(&candidate).await.unwrap();
        // It must remain active, not be clobbered back to candidate.
        let loaded = store
            .get_definition_by_name(
                "system_prompt",
                DefinitionScope::Global,
                None,
                Some("self_critique"),
                "self-critique-verify-first",
            )
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            loaded.status, "active",
            "re-stage must not demote a promoted def back to candidate"
        );
    }

    // Robust: an intentional active→superseded rollback still applies (the guard
    // only blocks candidate-demotes-active, never other transitions).
    #[tokio::test]
    async fn upsert_still_allows_active_to_superseded_rollback_robust() {
        let store = KnowledgeStore::open_in_memory().await.unwrap();
        let mut def = rsi_candidate("self-critique-x", "system_prompt");
        def.status = DefinitionStatus::Active;
        store.upsert_definition(&def).await.unwrap();
        def.status = DefinitionStatus::Superseded;
        store.upsert_definition(&def).await.unwrap();
        let loaded = store
            .get_definition_by_name(
                "system_prompt",
                DefinitionScope::Global,
                None,
                Some("self_critique"),
                "self-critique-x",
            )
            .await
            .unwrap();
        // get_definition_by_name only returns active rows; a superseded def is
        // gone from the active view — proving the rollback applied.
        assert!(
            loaded.is_none() || loaded.unwrap().status != "active",
            "active→superseded rollback must still apply"
        );
    }

    // Normal: the RSI funnel count groups candidates and active by kind, and
    // ignores non-RSI definitions.
    #[tokio::test]
    async fn rsi_definition_counts_groups_by_kind_status_normal() {
        let store = KnowledgeStore::open_in_memory().await.unwrap();
        store
            .upsert_definition(&rsi_candidate("a", "system_prompt"))
            .await
            .unwrap();
        store
            .upsert_definition(&rsi_candidate("b", "system_prompt"))
            .await
            .unwrap();
        let active = rsi_candidate("c", "reasoning_policy");
        let id = store.upsert_definition(&active).await.unwrap();
        store.set_definition_status(&id, "active").await.unwrap();
        // A non-RSI definition must be excluded.
        store
            .upsert_definition(&sample("not-rsi", "x"))
            .await
            .unwrap();

        let counts = store.rsi_definition_counts().await.unwrap();
        let get = |k: &str, s: &str| {
            counts
                .iter()
                .find(|c| c.kind == k && c.status == s)
                .map(|c| c.count)
                .unwrap_or(0)
        };
        assert_eq!(get("system_prompt", "candidate"), 2);
        assert_eq!(get("reasoning_policy", "active"), 1);
        assert_eq!(get("skill", "active"), 0, "non-RSI skill must be excluded");
    }

    #[tokio::test]
    async fn upsert_definition_round_trips_normal() {
        let store = KnowledgeStore::open_in_memory().await.unwrap();
        let id = store
            .upsert_definition(&sample("deploy", "body"))
            .await
            .unwrap();

        let loaded = store
            .get_definition_by_name(
                "skill",
                DefinitionScope::Project,
                Some("proj"),
                None,
                "deploy",
            )
            .await
            .unwrap()
            .unwrap();

        assert_eq!(loaded.id, id);
        assert_eq!(loaded.body, "body");
        assert_eq!(loaded.version, 1);
    }

    #[tokio::test]
    async fn upsert_definition_bumps_version_when_body_changes_normal() {
        let store = KnowledgeStore::open_in_memory().await.unwrap();
        store
            .upsert_definition(&sample("deploy", "body"))
            .await
            .unwrap();
        store
            .upsert_definition(&sample("deploy", "better body"))
            .await
            .unwrap();

        let loaded = store
            .get_definition_by_name(
                "skill",
                DefinitionScope::Project,
                Some("proj"),
                None,
                "deploy",
            )
            .await
            .unwrap()
            .unwrap();

        assert_eq!(loaded.body, "better body");
        assert_eq!(loaded.version, 2);
    }

    #[tokio::test]
    async fn list_definitions_for_project_status_returns_candidates_normal() {
        let store = KnowledgeStore::open_in_memory().await.unwrap();
        store
            .upsert_definition(&sample_with_status("active", DefinitionStatus::Active))
            .await
            .unwrap();
        store
            .upsert_definition(&sample_with_status(
                "candidate",
                DefinitionStatus::Candidate,
            ))
            .await
            .unwrap();

        let candidates = store
            .list_definitions_for_project_status("skill", "proj", "candidate", 10)
            .await
            .unwrap();

        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].name, "candidate");
        assert_eq!(candidates[0].status, DefinitionStatus::Candidate.slug());
    }
}

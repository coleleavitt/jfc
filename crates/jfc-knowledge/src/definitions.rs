use sqlx::Row;
use serde::{Deserialize, Serialize};

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
                status = excluded.status,
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
             WHERE id = ?1 AND superseded_by IS NULL"
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
             ORDER BY updated_at_ms ASC"
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

    #[tokio::test]
    async fn upsert_definition_round_trips_normal() {
        let store = KnowledgeStore::open_in_memory().await.unwrap();
        let id = store.upsert_definition(&sample("deploy", "body")).await.unwrap();

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
        store.upsert_definition(&sample("deploy", "body")).await.unwrap();
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
}

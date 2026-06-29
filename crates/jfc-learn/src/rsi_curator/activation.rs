mod metadata;

use serde_json::{Value, json};

use metadata::{
    mark_promoted_metadata, metadata_value, optional_string, require_rsi_metadata,
    require_verified_candidate, required_string, target_name,
};

use crate::error::LearnError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RsiDefinitionRef {
    pub kind: String,
    pub name: String,
}

impl RsiDefinitionRef {
    pub fn new(kind: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            kind: kind.into(),
            name: name.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RsiActivationReport {
    pub kind: String,
    pub name: String,
    pub status: String,
    pub action: RsiActivationAction,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RsiActivationAction {
    Promoted,
    RestoredPrior,
    Deactivated,
}

impl RsiActivationAction {
    pub const fn slug(self) -> &'static str {
        match self {
            Self::Promoted => "promoted",
            Self::RestoredPrior => "restored_prior",
            Self::Deactivated => "deactivated",
        }
    }
}

/// True if a candidate definition's metadata satisfies the promotion gate
/// (trust=verified, no raw thinking stored, fixtures run and all passed,
/// research verified). This is the SAME predicate [`promote_rsi_definition`]
/// enforces — exposed so callers (e.g. the dashboard funnel) can count how many
/// staged candidates are actually promotion-eligible without attempting a
/// promotion. `metadata` is the parsed `metadata_json` of a definition record.
pub fn is_promotable_candidate(metadata: &Value) -> bool {
    require_verified_candidate(metadata).is_ok()
}

pub async fn promote_rsi_definition(
    store: &jfc_knowledge::KnowledgeStore,
    project_key: &str,
    definition: &RsiDefinitionRef,
) -> Result<RsiActivationReport, LearnError> {
    let candidate = load_definition(store, project_key, definition).await?;
    require_status(&candidate, jfc_knowledge::DefinitionStatus::Candidate)?;
    let mut metadata = metadata_value(&candidate)?;
    require_verified_candidate(&metadata)?;
    let target_name = target_name(&metadata)?;
    let prior = store
        .get_definition_by_name(
            &definition.kind,
            jfc_knowledge::DefinitionScope::Project,
            Some(project_key),
            None,
            &target_name,
        )
        .await?;

    mark_promoted_metadata(&mut metadata, &candidate, prior.as_ref())?;
    let active = jfc_knowledge::NewDefinition {
        kind: definition.kind.clone(),
        scope: jfc_knowledge::DefinitionScope::Project,
        project_key: Some(project_key.to_owned()),
        namespace: None,
        name: target_name.clone(),
        title: candidate.title.clone(),
        description: candidate.description.clone(),
        body: candidate.body.clone(),
        metadata_json: serde_json::to_string_pretty(&metadata)?,
        source_path: candidate.source_path.clone(),
        source_hash: candidate.source_hash.clone(),
        status: jfc_knowledge::DefinitionStatus::Active,
        created_by: "rsi-curator".to_owned(),
    };
    store.upsert_definition(&active).await?;
    mark_candidate_superseded(store, project_key, &candidate, &metadata).await?;
    Ok(RsiActivationReport {
        kind: definition.kind.clone(),
        name: target_name,
        status: jfc_knowledge::DefinitionStatus::Active.slug().to_owned(),
        action: RsiActivationAction::Promoted,
    })
}

pub async fn rollback_rsi_definition(
    store: &jfc_knowledge::KnowledgeStore,
    project_key: &str,
    definition: &RsiDefinitionRef,
) -> Result<RsiActivationReport, LearnError> {
    let active = load_definition(store, project_key, definition).await?;
    require_status(&active, jfc_knowledge::DefinitionStatus::Active)?;
    let metadata = metadata_value(&active)?;
    require_rsi_metadata(&metadata)?;
    if let Some(snapshot) = metadata.pointer("/rsi/rollback/snapshot") {
        restore_snapshot(store, project_key, &active, snapshot).await?;
        return Ok(RsiActivationReport {
            kind: definition.kind.clone(),
            name: definition.name.clone(),
            status: jfc_knowledge::DefinitionStatus::Active.slug().to_owned(),
            action: RsiActivationAction::RestoredPrior,
        });
    }

    deactivate_active_definition(store, project_key, &active, metadata).await?;
    Ok(RsiActivationReport {
        kind: definition.kind.clone(),
        name: definition.name.clone(),
        status: jfc_knowledge::DefinitionStatus::Superseded
            .slug()
            .to_owned(),
        action: RsiActivationAction::Deactivated,
    })
}

async fn load_definition(
    store: &jfc_knowledge::KnowledgeStore,
    project_key: &str,
    definition: &RsiDefinitionRef,
) -> Result<jfc_knowledge::DefinitionRecord, LearnError> {
    store
        .get_definition_by_name(
            &definition.kind,
            jfc_knowledge::DefinitionScope::Project,
            Some(project_key),
            None,
            &definition.name,
        )
        .await?
        .ok_or_else(|| LearnError::ContractViolation {
            message: format!(
                "RSI definition `{}`/`{}` was not found",
                definition.kind, definition.name
            ),
        })
}

fn require_status(
    record: &jfc_knowledge::DefinitionRecord,
    status: jfc_knowledge::DefinitionStatus,
) -> Result<(), LearnError> {
    if record.status == status.slug() {
        return Ok(());
    }
    Err(LearnError::ContractViolation {
        message: format!(
            "RSI definition `{}` has status `{}`, expected `{}`",
            record.name,
            record.status,
            status.slug()
        ),
    })
}

async fn mark_candidate_superseded(
    store: &jfc_knowledge::KnowledgeStore,
    project_key: &str,
    candidate: &jfc_knowledge::DefinitionRecord,
    active_metadata: &Value,
) -> Result<(), LearnError> {
    let mut metadata = active_metadata.clone();
    if let Some(rsi) = metadata.get_mut("rsi").and_then(Value::as_object_mut) {
        rsi.insert("status".to_owned(), json!("superseded"));
    }
    let archived = jfc_knowledge::NewDefinition {
        kind: candidate.kind.clone(),
        scope: jfc_knowledge::DefinitionScope::Project,
        project_key: Some(project_key.to_owned()),
        namespace: candidate.namespace.clone(),
        name: candidate.name.clone(),
        title: candidate.title.clone(),
        description: candidate.description.clone(),
        body: candidate.body.clone(),
        metadata_json: serde_json::to_string_pretty(&metadata)?,
        source_path: candidate.source_path.clone(),
        source_hash: candidate.source_hash.clone(),
        status: jfc_knowledge::DefinitionStatus::Superseded,
        created_by: "rsi-curator".to_owned(),
    };
    store.upsert_definition(&archived).await?;
    Ok(())
}

async fn restore_snapshot(
    store: &jfc_knowledge::KnowledgeStore,
    project_key: &str,
    active: &jfc_knowledge::DefinitionRecord,
    snapshot: &Value,
) -> Result<(), LearnError> {
    let restored = jfc_knowledge::NewDefinition {
        kind: active.kind.clone(),
        scope: jfc_knowledge::DefinitionScope::Project,
        project_key: Some(project_key.to_owned()),
        namespace: active.namespace.clone(),
        name: active.name.clone(),
        title: optional_string(snapshot, "title"),
        description: optional_string(snapshot, "description"),
        body: required_string(snapshot, "body")?,
        metadata_json: optional_string(snapshot, "metadata_json")
            .unwrap_or_else(|| "{}".to_owned()),
        source_path: optional_string(snapshot, "source_path"),
        source_hash: optional_string(snapshot, "source_hash"),
        status: jfc_knowledge::DefinitionStatus::Active,
        created_by: "rsi-rollback".to_owned(),
    };
    store.upsert_definition(&restored).await?;
    Ok(())
}

async fn deactivate_active_definition(
    store: &jfc_knowledge::KnowledgeStore,
    project_key: &str,
    active: &jfc_knowledge::DefinitionRecord,
    mut metadata: Value,
) -> Result<(), LearnError> {
    if let Some(rsi) = metadata.get_mut("rsi").and_then(Value::as_object_mut) {
        rsi.insert("status".to_owned(), json!("rolled_back"));
    }
    let deactivated = jfc_knowledge::NewDefinition {
        kind: active.kind.clone(),
        scope: jfc_knowledge::DefinitionScope::Project,
        project_key: Some(project_key.to_owned()),
        namespace: active.namespace.clone(),
        name: active.name.clone(),
        title: active.title.clone(),
        description: active.description.clone(),
        body: active.body.clone(),
        metadata_json: serde_json::to_string_pretty(&metadata)?,
        source_path: active.source_path.clone(),
        source_hash: active.source_hash.clone(),
        status: jfc_knowledge::DefinitionStatus::Superseded,
        created_by: "rsi-rollback".to_owned(),
    };
    store.upsert_definition(&deactivated).await?;
    Ok(())
}

#[cfg(test)]
mod tests;

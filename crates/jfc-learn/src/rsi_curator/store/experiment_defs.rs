use serde_json::json;

use crate::error::LearnError;

pub(super) async fn upsert_experiment_loop_state(
    store: &jfc_knowledge::KnowledgeStore,
    project_key: &str,
    state: &super::super::RsiExperimentLoopState,
) -> Result<(), LearnError> {
    let body = state.render_summary();
    let definition = jfc_knowledge::NewDefinition {
        kind: super::super::RSI_LOOP_STATE_KIND.to_owned(),
        scope: jfc_knowledge::DefinitionScope::Project,
        project_key: Some(project_key.to_owned()),
        namespace: None,
        name: super::super::RSI_LOOP_STATE_NAME.to_owned(),
        title: Some("RSI experiment loop state".to_owned()),
        description: Some("Durable cadence and run metrics for RSI iterations".to_owned()),
        body: body.clone(),
        metadata_json: serde_json::to_string_pretty(&json!({
            "rsi": {
                "experiment_loop_state": state.to_metadata(),
            }
        }))?,
        source_path: Some("rsi:definition:experiment_loop_state:current".to_owned()),
        source_hash: Some(super::content_hash(&body)),
        status: jfc_knowledge::DefinitionStatus::Active,
        created_by: "rsi-curator".to_owned(),
    };
    store.upsert_definition(&definition).await?;
    Ok(())
}

pub(super) async fn upsert_experiment_job(
    store: &jfc_knowledge::KnowledgeStore,
    project_key: &str,
    job: &super::super::RsiExperimentJobSpec,
) -> Result<(), LearnError> {
    let body = job.render_summary();
    let definition = jfc_knowledge::NewDefinition {
        kind: "rsi_experiment_job".to_owned(),
        scope: jfc_knowledge::DefinitionScope::Project,
        project_key: Some(project_key.to_owned()),
        namespace: None,
        name: "current".to_owned(),
        title: Some("RSI experiment job".to_owned()),
        description: Some("Executable preflight contract for the next RSI iteration".to_owned()),
        body: body.clone(),
        metadata_json: serde_json::to_string_pretty(&json!({
            "rsi": {
                "experiment_job": job.to_metadata(),
            }
        }))?,
        source_path: Some("rsi:definition:experiment_job:current".to_owned()),
        source_hash: Some(super::content_hash(&body)),
        status: jfc_knowledge::DefinitionStatus::Active,
        created_by: "rsi-curator".to_owned(),
    };
    store.upsert_definition(&definition).await?;
    Ok(())
}

pub(super) async fn upsert_experiment_loop(
    store: &jfc_knowledge::KnowledgeStore,
    project_key: &str,
    plan: &super::super::RsiExperimentLoopPlan,
) -> Result<(), LearnError> {
    let body = plan.render_summary();
    let definition = jfc_knowledge::NewDefinition {
        kind: "rsi_experiment_loop".to_owned(),
        scope: jfc_knowledge::DefinitionScope::Project,
        project_key: Some(project_key.to_owned()),
        namespace: None,
        name: "current".to_owned(),
        title: Some("RSI experiment loop".to_owned()),
        description: Some("Controlled next-iteration contract for long-running RSI".to_owned()),
        body: body.clone(),
        metadata_json: serde_json::to_string_pretty(&json!({
            "rsi": {
                "experiment_loop": plan.to_metadata(),
            }
        }))?,
        source_path: Some("rsi:definition:experiment_loop:current".to_owned()),
        source_hash: Some(super::content_hash(&body)),
        status: jfc_knowledge::DefinitionStatus::Active,
        created_by: "rsi-curator".to_owned(),
    };
    store.upsert_definition(&definition).await?;
    Ok(())
}

pub(super) async fn upsert_experiment_dashboard(
    store: &jfc_knowledge::KnowledgeStore,
    project_key: &str,
    dashboard: &super::super::RsiExperimentDashboard,
) -> Result<(), LearnError> {
    let body = dashboard.render_summary();
    let definition = jfc_knowledge::NewDefinition {
        kind: "rsi_experiment_dashboard".to_owned(),
        scope: jfc_knowledge::DefinitionScope::Project,
        project_key: Some(project_key.to_owned()),
        namespace: None,
        name: "current".to_owned(),
        title: Some("RSI experiment dashboard".to_owned()),
        description: Some("Long-running RSI experiment control summary".to_owned()),
        body: body.clone(),
        metadata_json: serde_json::to_string_pretty(&json!({
            "rsi": {
                "experiment_dashboard": dashboard.to_metadata(),
            }
        }))?,
        source_path: Some("rsi:definition:experiment_dashboard:current".to_owned()),
        source_hash: Some(super::content_hash(&body)),
        status: jfc_knowledge::DefinitionStatus::Active,
        created_by: "rsi-curator".to_owned(),
    };
    store.upsert_definition(&definition).await?;
    Ok(())
}

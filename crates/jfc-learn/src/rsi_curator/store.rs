use serde_json::json;

use super::control::assess_control;
use super::metadata::{
    content_hash, control_metadata, definition_metadata, definition_status, graph_metadata,
    research_metadata,
};
use super::{
    CandidateChange, CandidateKind, CandidateStatus, ControlAssessment, RsiCuratorReport,
    RsiJobPreflightStatus,
};
use crate::error::LearnError;

mod experiment_defs;

use experiment_defs::{
    upsert_experiment_dashboard, upsert_experiment_job, upsert_experiment_loop,
    upsert_experiment_loop_state,
};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StoreApplyReport {
    pub learning_events: usize,
    pub definitions: usize,
    pub memories: usize,
    pub experiment_dashboards: usize,
    pub experiment_loops: usize,
    pub experiment_jobs: usize,
    pub experiment_loop_states: usize,
}

impl StoreApplyReport {
    pub fn actions(&self) -> usize {
        self.learning_events
            + self.definitions
            + self.memories
            + self.experiment_dashboards
            + self.experiment_loops
            + self.experiment_jobs
            + self.experiment_loop_states
    }
}

/// Persist a curator report to the knowledge store. An extension trait (not an
/// inherent impl) because `RsiCuratorReport` now lives in the external `rsi-rs`
/// crate, and Rust's orphan rule forbids inherent impls on foreign types.
#[allow(async_fn_in_trait)] // internal-use trait; callers await inline, no Send bound needed
pub trait ApplyToStore {
    async fn apply_to_store(
        &self,
        store: &jfc_knowledge::KnowledgeStore,
        project_key: &str,
    ) -> Result<StoreApplyReport, LearnError>;
}

impl ApplyToStore for RsiCuratorReport {
    async fn apply_to_store(
        &self,
        store: &jfc_knowledge::KnowledgeStore,
        project_key: &str,
    ) -> Result<StoreApplyReport, LearnError> {
        let _linkscope_apply = linkscope::phase("learn.rsi_store.apply_report");
        linkscope::event_fields(
            "learn.rsi_store.apply_report",
            [
                linkscope::TraceField::text("project_key", project_key.to_owned()),
                linkscope::TraceField::count(
                    "candidates",
                    u64::try_from(self.candidates.len()).unwrap_or(u64::MAX),
                ),
                linkscope::TraceField::count(
                    "trace_count",
                    u64::try_from(self.experiment_dashboard.trace_count).unwrap_or(u64::MAX),
                ),
            ],
        );
        let mut report = StoreApplyReport::default();
        let preflight_ready = self.experiment_dashboard.trace_count == 0
            || self.experiment_job.preflight.status == RsiJobPreflightStatus::Ready;
        if preflight_ready {
            for candidate in &self.candidates {
                let control = assess_control(candidate);
                linkscope::detail_event_fields(
                    "learn.rsi_store.candidate",
                    [
                        linkscope::TraceField::text("id", candidate.id.clone()),
                        linkscope::TraceField::text("kind", candidate.kind.slug().to_owned()),
                        linkscope::TraceField::text(
                            "activation",
                            control.activation_status.slug().to_owned(),
                        ),
                    ],
                );
                record_learning_event(
                    store,
                    project_key,
                    candidate,
                    &self.experience_graph,
                    &control,
                )
                .await?;
                report.learning_events += 1;
                match candidate.kind {
                    CandidateKind::MemoryRule => {
                        if control.activation_status == CandidateStatus::Active {
                            insert_memory_rule(store, project_key, candidate).await?;
                            report.memories += 1;
                        }
                    }
                    CandidateKind::SkillDraft
                    | CandidateKind::SystemPromptPatch
                    | CandidateKind::ToolDefinitionPatch
                    | CandidateKind::HarnessPatch
                    | CandidateKind::ContextPlaybookPatch
                    | CandidateKind::BudgetPolicy
                    | CandidateKind::ReasoningPolicy => {
                        upsert_definition(
                            store,
                            project_key,
                            candidate,
                            &self.experience_graph,
                            &control,
                        )
                        .await?;
                        report.definitions += 1;
                    }
                }
            }
        }
        if self.experiment_dashboard.trace_count > 0 {
            upsert_experiment_dashboard(store, project_key, &self.experiment_dashboard).await?;
            report.experiment_dashboards += 1;
            upsert_experiment_loop(store, project_key, &self.experiment_loop).await?;
            report.experiment_loops += 1;
            upsert_experiment_job(store, project_key, &self.experiment_job).await?;
            report.experiment_jobs += 1;
            let candidate_actions = report.learning_events + report.definitions + report.memories;
            let previous = super::load_experiment_loop_state(store, project_key).await?;
            let state = super::build_next_loop_state(
                previous.as_ref(),
                &self.experiment_dashboard,
                &self.experiment_job,
                candidate_actions,
                self.candidates.len(),
                super::current_time_ms(),
            );
            upsert_experiment_loop_state(store, project_key, &state).await?;
            report.experiment_loop_states += 1;
        }
        linkscope::event_fields(
            "learn.rsi_store.apply_report.result",
            [
                linkscope::TraceField::count(
                    "actions",
                    u64::try_from(report.actions()).unwrap_or(u64::MAX),
                ),
                linkscope::TraceField::count(
                    "definitions",
                    u64::try_from(report.definitions).unwrap_or(u64::MAX),
                ),
                linkscope::TraceField::count(
                    "memories",
                    u64::try_from(report.memories).unwrap_or(u64::MAX),
                ),
            ],
        );
        Ok(report)
    }
}

async fn record_learning_event(
    store: &jfc_knowledge::KnowledgeStore,
    project_key: &str,
    candidate: &CandidateChange,
    graph: &super::ExperienceGraph,
    control: &ControlAssessment,
) -> Result<(), LearnError> {
    let _linkscope_event = linkscope::phase("learn.rsi_store.record_learning_event");
    let now = now_ms();
    let row = jfc_knowledge::LearningEventRow {
        id: format!("rsi:{}", candidate.id),
        source_session_id: Some(candidate.source_session_id.clone()),
        source_turn_id: candidate.source_turn_id.clone(),
        source_tool_run_id: None,
        candidate_rule: candidate.body.clone(),
        status: control.activation_status.slug().to_owned(),
        verifier_evidence: serde_json::to_string(&json!({
            "eval": {
                "passed": candidate.eval.passed,
                "score": candidate.eval.score,
                "reason": candidate.eval.reason,
                "fixtures_run": candidate.eval.fixtures_run,
                "fixtures_passed": candidate.eval.fixtures_passed,
                "research": research_metadata(&candidate.eval),
            },
            "evidence": candidate.evidence,
            "candidate_kind": candidate.kind.slug(),
            "project_key": project_key,
            "experience_graph": graph_metadata(candidate, graph),
            "control": control_metadata(control),
        }))?,
        recurrence_count: candidate.recurrence_count,
        created_at_ms: now,
        updated_at_ms: now,
    };
    store.record_learning_event(&row).await?;
    Ok(())
}

async fn insert_memory_rule(
    store: &jfc_knowledge::KnowledgeStore,
    project_key: &str,
    candidate: &CandidateChange,
) -> Result<(), LearnError> {
    let _linkscope_memory = linkscope::phase("learn.rsi_store.insert_memory_rule");
    linkscope::event_fields(
        "learn.rsi_store.insert_memory_rule",
        [
            linkscope::TraceField::text("project_key", project_key.to_owned()),
            linkscope::TraceField::text("candidate_id", candidate.id.clone()),
        ],
    );
    let mut record = jfc_knowledge::KnowledgeRecord::new(
        jfc_knowledge::Kind::Finding,
        jfc_knowledge::Scope::Project,
        Some(project_key.to_owned()),
        candidate.title.clone(),
        candidate.body.clone(),
    )
    .with_confidence(candidate.eval.score)
    .with_importance(candidate.score)
    .with_outcome(jfc_knowledge::Outcome::Verified)
    .with_source(format!("rsi:session:{}", candidate.source_session_id));
    record.id = format!("rsi-memory-{}", &candidate.id[..24]);
    record.tags = format!("rsi,{}", candidate.kind.slug());
    store.insert(&record).await?;
    Ok(())
}

async fn upsert_definition(
    store: &jfc_knowledge::KnowledgeStore,
    project_key: &str,
    candidate: &CandidateChange,
    graph: &super::ExperienceGraph,
    control: &ControlAssessment,
) -> Result<(), LearnError> {
    let _linkscope_definition = linkscope::phase("learn.rsi_store.upsert_definition");
    let Some(kind) = candidate.kind.definition_kind() else {
        linkscope::event_fields(
            "learn.rsi_store.upsert_definition.result",
            [linkscope::TraceField::text("status", "no_definition_kind")],
        );
        return Ok(());
    };
    linkscope::event_fields(
        "learn.rsi_store.upsert_definition",
        [
            linkscope::TraceField::text("project_key", project_key.to_owned()),
            linkscope::TraceField::text("kind", kind.to_owned()),
            linkscope::TraceField::text("candidate_id", candidate.id.clone()),
        ],
    );
    let prior = store
        .get_definition_by_name(
            kind,
            jfc_knowledge::DefinitionScope::Project,
            Some(project_key),
            None,
            &candidate.target.name,
        )
        .await?;
    let definition_name = definition_name_for(candidate, control);
    let metadata = definition_metadata(candidate, prior.as_ref(), graph, control);
    let definition = jfc_knowledge::NewDefinition {
        kind: kind.to_owned(),
        scope: jfc_knowledge::DefinitionScope::Project,
        project_key: Some(project_key.to_owned()),
        namespace: None,
        name: definition_name,
        title: Some(candidate.title.clone()),
        description: Some(candidate.eval.reason.clone()),
        body: candidate.body.clone(),
        metadata_json: metadata,
        source_path: Some(format!("rsi:definition:{}", candidate.id)),
        source_hash: Some(content_hash(&candidate.body)),
        status: definition_status(control.activation_status),
        created_by: "rsi-curator".to_owned(),
    };
    store.upsert_definition(&definition).await?;
    Ok(())
}

fn definition_name_for(candidate: &CandidateChange, control: &ControlAssessment) -> String {
    if control.activation_status == CandidateStatus::Active {
        return candidate.target.name.clone();
    }
    let short_id = candidate.id.get(..12).unwrap_or(&candidate.id);
    format!("rsi-{}-{short_id}", candidate.kind.slug())
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis().min(i64::MAX as u128) as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests;

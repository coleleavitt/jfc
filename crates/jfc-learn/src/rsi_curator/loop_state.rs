use serde::{Deserialize, Serialize};
use serde_json::json;

use super::{RsiExperimentDashboard, RsiExperimentJobSpec, RsiJobPreflightStatus};
use crate::error::LearnError;

pub const RSI_LOOP_STATE_KIND: &str = "rsi_experiment_loop_state";
pub const RSI_LOOP_STATE_NAME: &str = "current";

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RsiExperimentLoopState {
    pub run_count: u64,
    pub last_run_at_ms: u64,
    pub next_due_at_ms: u64,
    pub cadence_seconds: u64,
    pub phase: String,
    pub preflight_status: String,
    pub candidate_actions: usize,
    pub traces_scored: usize,
    pub candidates_seen: usize,
    pub total_estimated_tokens: u64,
    pub latest_score_milli: u64,
    pub best_score_milli: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RsiLoopDueDecision {
    pub due: bool,
    pub reason: &'static str,
    pub next_due_at_ms: Option<u64>,
}

impl RsiExperimentLoopState {
    pub fn to_metadata(&self) -> serde_json::Value {
        json!({
            "run_count": self.run_count,
            "last_run_at_ms": self.last_run_at_ms,
            "next_due_at_ms": self.next_due_at_ms,
            "cadence_seconds": self.cadence_seconds,
            "phase": self.phase,
            "preflight_status": self.preflight_status,
            "candidate_actions": self.candidate_actions,
            "traces_scored": self.traces_scored,
            "candidates_seen": self.candidates_seen,
            "total_estimated_tokens": self.total_estimated_tokens,
            "latest_score_milli": self.latest_score_milli,
            "best_score_milli": self.best_score_milli,
        })
    }

    pub fn render_summary(&self) -> String {
        format!(
            "runs={} last_run_at_ms={} next_due_at_ms={} cadence_seconds={} phase={} preflight={} candidate_actions={} traces={} candidates={} cost_tokens={} latest_score_milli={} best_score_milli={}",
            self.run_count,
            self.last_run_at_ms,
            self.next_due_at_ms,
            self.cadence_seconds,
            self.phase,
            self.preflight_status,
            self.candidate_actions,
            self.traces_scored,
            self.candidates_seen,
            self.total_estimated_tokens,
            self.latest_score_milli,
            self.best_score_milli,
        )
    }
}

pub fn build_next_loop_state(
    previous: Option<&RsiExperimentLoopState>,
    dashboard: &RsiExperimentDashboard,
    job: &RsiExperimentJobSpec,
    candidate_actions: usize,
    candidates_seen: usize,
    now_ms: u64,
) -> RsiExperimentLoopState {
    let run_count = previous.map_or(1, |state| state.run_count.saturating_add(1));
    let cadence_ms = job.schedule.cadence_seconds.saturating_mul(1_000);
    RsiExperimentLoopState {
        run_count,
        last_run_at_ms: now_ms,
        next_due_at_ms: now_ms.saturating_add(cadence_ms),
        cadence_seconds: job.schedule.cadence_seconds,
        phase: job.phase.slug().to_owned(),
        preflight_status: job.preflight.status.slug().to_owned(),
        candidate_actions,
        traces_scored: dashboard.trace_count,
        candidates_seen,
        total_estimated_tokens: dashboard.cost.estimated_tokens,
        latest_score_milli: score_milli(dashboard.plateau.latest_score),
        best_score_milli: score_milli(dashboard.plateau.best_score),
    }
}

pub async fn load_experiment_loop_state(
    store: &jfc_knowledge::KnowledgeStore,
    project_key: &str,
) -> Result<Option<RsiExperimentLoopState>, LearnError> {
    let Some(record) = store
        .get_definition_by_name(
            RSI_LOOP_STATE_KIND,
            jfc_knowledge::DefinitionScope::Project,
            Some(project_key),
            None,
            RSI_LOOP_STATE_NAME,
        )
        .await?
    else {
        return Ok(None);
    };
    let metadata = serde_json::from_str::<serde_json::Value>(&record.metadata_json)?;
    let Some(state) = metadata.pointer("/rsi/experiment_loop_state") else {
        return Ok(None);
    };
    Ok(Some(serde_json::from_value(state.clone())?))
}

pub async fn experiment_loop_due_decision(
    store: &jfc_knowledge::KnowledgeStore,
    project_key: &str,
    now_ms: u64,
) -> Result<RsiLoopDueDecision, LearnError> {
    let Some(state) = load_experiment_loop_state(store, project_key).await? else {
        return Ok(RsiLoopDueDecision {
            due: true,
            reason: "no_prior_loop_state",
            next_due_at_ms: None,
        });
    };
    if state.preflight_status == RsiJobPreflightStatus::Blocked.slug() {
        return Ok(RsiLoopDueDecision {
            due: true,
            reason: "blocked_preflight_recheck",
            next_due_at_ms: Some(state.next_due_at_ms),
        });
    }
    Ok(RsiLoopDueDecision {
        due: now_ms >= state.next_due_at_ms,
        reason: if now_ms >= state.next_due_at_ms {
            "due"
        } else {
            "cadence_wait"
        },
        next_due_at_ms: Some(state.next_due_at_ms),
    })
}

pub fn current_time_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis().min(u64::MAX as u128) as u64)
        .unwrap_or(0)
}

fn score_milli(score: f64) -> u64 {
    if !score.is_finite() || score <= 0.0 {
        return 0;
    }
    (score * 1_000.0).round().min(u64::MAX as f64) as u64
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rsi_curator::{
        RsiOutcome, RsiToolStep, RsiTrace, RsiVerification, build_experiment_dashboard,
        build_experiment_job_spec, build_experiment_loop_plan,
    };

    #[test]
    fn loop_state_records_next_due_and_metrics_normal() {
        let traces = [
            succeeded_trace("s1", 1_000),
            succeeded_trace("s2", 1_100),
            succeeded_trace("s3", 1_200),
            succeeded_trace("s4", 1_300),
        ];
        let dashboard = build_experiment_dashboard(&traces);
        let plan = build_experiment_loop_plan(&dashboard);
        let job = build_experiment_job_spec(&dashboard, &plan);

        let state = build_next_loop_state(None, &dashboard, &job, 7, 9, 1_000);

        assert_eq!(state.run_count, 1);
        assert_eq!(state.last_run_at_ms, 1_000);
        assert_eq!(state.next_due_at_ms, 901_000);
        assert_eq!(state.preflight_status, "ready");
        assert_eq!(state.candidate_actions, 7);
        assert_eq!(state.candidates_seen, 9);
        assert!(state.total_estimated_tokens > 0);
    }

    #[tokio::test]
    async fn due_decision_waits_until_next_due_normal() {
        let store = jfc_knowledge::KnowledgeStore::open_in_memory()
            .await
            .unwrap();
        let state = RsiExperimentLoopState {
            next_due_at_ms: 10_000,
            preflight_status: "ready".to_owned(),
            ..Default::default()
        };
        upsert_state_for_test(&store, "proj", &state).await;

        let decision = experiment_loop_due_decision(&store, "proj", 9_000)
            .await
            .unwrap();

        assert!(!decision.due);
        assert_eq!(decision.reason, "cadence_wait");
        assert_eq!(decision.next_due_at_ms, Some(10_000));
    }

    #[tokio::test]
    async fn due_decision_rechecks_blocked_preflight_robust() {
        let store = jfc_knowledge::KnowledgeStore::open_in_memory()
            .await
            .unwrap();
        let state = RsiExperimentLoopState {
            next_due_at_ms: 10_000,
            preflight_status: "blocked".to_owned(),
            ..Default::default()
        };
        upsert_state_for_test(&store, "proj", &state).await;

        let decision = experiment_loop_due_decision(&store, "proj", 9_000)
            .await
            .unwrap();

        assert!(decision.due);
        assert_eq!(decision.reason, "blocked_preflight_recheck");
    }

    async fn upsert_state_for_test(
        store: &jfc_knowledge::KnowledgeStore,
        project_key: &str,
        state: &RsiExperimentLoopState,
    ) {
        store
            .upsert_definition(&jfc_knowledge::NewDefinition {
                kind: RSI_LOOP_STATE_KIND.to_owned(),
                scope: jfc_knowledge::DefinitionScope::Project,
                project_key: Some(project_key.to_owned()),
                namespace: None,
                name: RSI_LOOP_STATE_NAME.to_owned(),
                title: None,
                description: None,
                body: state.render_summary(),
                metadata_json: json!({"rsi": {"experiment_loop_state": state.to_metadata()}})
                    .to_string(),
                source_path: None,
                source_hash: None,
                status: jfc_knowledge::DefinitionStatus::Active,
                created_by: "test".to_owned(),
            })
            .await
            .unwrap();
    }

    fn succeeded_trace(session: &str, thinking_tokens: u64) -> RsiTrace {
        let mut trace = RsiTrace::new(session);
        trace.outcome = Some(RsiOutcome::Succeeded);
        trace.thinking_tokens = thinking_tokens;
        trace.tool_steps = vec![RsiToolStep::new("Bash", true)];
        trace.verifications = vec![RsiVerification::new("hidden cargo test", true)];
        trace
    }
}

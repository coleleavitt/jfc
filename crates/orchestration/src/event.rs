use crate::{OrchestrationModule, OrchestrationSkeletonError, non_empty_string, trace};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OrchestrationEventKind {
    AgentLaunched,
    SwarmFormed,
    CouncilConcluded,
    WorkflowAdvanced,
    GoalEvaluated,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OrchestrationEvent {
    sequence: u64,
    module: OrchestrationModule,
    kind: OrchestrationEventKind,
    actor: String,
    summary: String,
}

impl OrchestrationEvent {
    pub fn new(
        sequence: u64,
        module: OrchestrationModule,
        kind: OrchestrationEventKind,
        actor: impl Into<String>,
        summary: impl Into<String>,
    ) -> Result<Self, OrchestrationSkeletonError> {
        let _linkscope_event = linkscope::phase("orchestration.event.new");
        let actor = actor.into();
        let summary = summary.into();
        trace::record_event_shape(trace::EventTrace {
            sequence,
            module: module.label(),
            kind: kind.label(),
            actor_bytes: actor.len(),
            summary_bytes: summary.len(),
        });
        Ok(Self {
            sequence,
            module,
            kind,
            actor: non_empty_string(actor, OrchestrationSkeletonError::EmptyEventActor)?,
            summary: non_empty_string(summary, OrchestrationSkeletonError::EmptyEventSummary)?,
        })
    }

    pub fn sequence(&self) -> u64 {
        self.sequence
    }

    pub fn module(&self) -> OrchestrationModule {
        self.module
    }

    pub fn kind(&self) -> OrchestrationEventKind {
        self.kind
    }

    pub fn actor(&self) -> &str {
        &self.actor
    }

    pub fn summary(&self) -> &str {
        &self.summary
    }
}

impl OrchestrationEventKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::AgentLaunched => "agent_launched",
            Self::SwarmFormed => "swarm_formed",
            Self::CouncilConcluded => "council_concluded",
            Self::WorkflowAdvanced => "workflow_advanced",
            Self::GoalEvaluated => "goal_evaluated",
        }
    }
}

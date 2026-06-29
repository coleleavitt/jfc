use crate::{OrchestrationSkeletonError, non_empty_string, trace};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowOrchestration {
    id: String,
    phase: String,
}

impl WorkflowOrchestration {
    pub fn new(
        id: impl Into<String>,
        phase: impl Into<String>,
    ) -> Result<Self, OrchestrationSkeletonError> {
        let _linkscope_workflow = linkscope::phase("orchestration.workflow.new");
        let id = non_empty_string(id, OrchestrationSkeletonError::EmptyWorkflowId)?;
        let phase = non_empty_string(phase, OrchestrationSkeletonError::EmptyWorkflowPhase)?;
        trace::record_named(trace::NamedTrace {
            label: "orchestration.workflow.new",
            id: &id,
            kind: "workflow",
            value_label: "phase_bytes",
            value_bytes: phase.len(),
        });
        Ok(Self { id, phase })
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn phase(&self) -> &str {
        &self.phase
    }
}

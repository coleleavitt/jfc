use crate::{OrchestrationSkeletonError, non_empty_string};
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
        Ok(Self {
            id: non_empty_string(id, OrchestrationSkeletonError::EmptyWorkflowId)?,
            phase: non_empty_string(phase, OrchestrationSkeletonError::EmptyWorkflowPhase)?,
        })
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn phase(&self) -> &str {
        &self.phase
    }
}

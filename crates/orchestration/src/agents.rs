use crate::{OrchestrationSkeletonError, non_empty_string};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentOrchestrationRole {
    Explore,
    Plan,
    Verify,
    Orchestrator,
    Worker,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentOrchestration {
    id: String,
    role: AgentOrchestrationRole,
}

impl AgentOrchestration {
    pub fn new(
        id: impl Into<String>,
        role: AgentOrchestrationRole,
    ) -> Result<Self, OrchestrationSkeletonError> {
        Ok(Self {
            id: non_empty_string(id, OrchestrationSkeletonError::EmptyAgentId)?,
            role,
        })
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn role(&self) -> AgentOrchestrationRole {
        self.role
    }
}

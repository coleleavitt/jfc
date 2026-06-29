use crate::{OrchestrationSkeletonError, non_empty_string, trace};
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
        let _linkscope_agent = linkscope::phase("orchestration.agent.new");
        let id = non_empty_string(id, OrchestrationSkeletonError::EmptyAgentId)?;
        trace::record_named(trace::NamedTrace {
            label: "orchestration.agent.new",
            id: &id,
            kind: role.label(),
            value_label: "role",
            value_bytes: 0,
        });
        Ok(Self { id, role })
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn role(&self) -> AgentOrchestrationRole {
        self.role
    }
}

impl AgentOrchestrationRole {
    pub fn label(self) -> &'static str {
        match self {
            Self::Explore => "explore",
            Self::Plan => "plan",
            Self::Verify => "verify",
            Self::Orchestrator => "orchestrator",
            Self::Worker => "worker",
        }
    }
}

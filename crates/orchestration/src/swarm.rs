use crate::{OrchestrationSkeletonError, non_empty_string};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SwarmOrchestration {
    id: String,
    members: Vec<String>,
}

impl SwarmOrchestration {
    pub fn new(
        id: impl Into<String>,
        members: Vec<String>,
    ) -> Result<Self, OrchestrationSkeletonError> {
        if members.is_empty() {
            return Err(OrchestrationSkeletonError::EmptySwarmMembers);
        }

        Ok(Self {
            id: non_empty_string(id, OrchestrationSkeletonError::EmptySwarmId)?,
            members,
        })
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn members(&self) -> &[String] {
        &self.members
    }
}

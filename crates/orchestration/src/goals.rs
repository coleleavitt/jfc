use crate::{OrchestrationSkeletonError, non_empty_string};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GoalOrchestration {
    id: String,
    condition: String,
}

impl GoalOrchestration {
    pub fn new(
        id: impl Into<String>,
        condition: impl Into<String>,
    ) -> Result<Self, OrchestrationSkeletonError> {
        Ok(Self {
            id: non_empty_string(id, OrchestrationSkeletonError::EmptyGoalId)?,
            condition: non_empty_string(condition, OrchestrationSkeletonError::EmptyGoalCondition)?,
        })
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn condition(&self) -> &str {
        &self.condition
    }
}

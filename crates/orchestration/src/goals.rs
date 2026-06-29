use crate::{OrchestrationSkeletonError, non_empty_string, trace};
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
        let _linkscope_goal = linkscope::phase("orchestration.goal.new");
        let id = non_empty_string(id, OrchestrationSkeletonError::EmptyGoalId)?;
        let condition =
            non_empty_string(condition, OrchestrationSkeletonError::EmptyGoalCondition)?;
        trace::record_named(trace::NamedTrace {
            label: "orchestration.goal.new",
            id: &id,
            kind: "goal",
            value_label: "condition_bytes",
            value_bytes: condition.len(),
        });
        Ok(Self { id, condition })
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn condition(&self) -> &str {
        &self.condition
    }
}

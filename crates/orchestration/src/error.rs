use std::fmt::{Display, Formatter};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OrchestrationSkeletonError {
    EmptyLayout,
    IncompleteLayout,
    EmptyAgentId,
    EmptySwarmId,
    EmptySwarmMembers,
    EmptyCouncilId,
    EmptyCouncilSeats,
    EmptyWorkflowId,
    EmptyWorkflowPhase,
    EmptyGoalId,
    EmptyGoalCondition,
    EmptyEventActor,
    EmptyEventSummary,
}

impl Display for OrchestrationSkeletonError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::EmptyLayout => "orchestration layout must contain at least one module",
            Self::IncompleteLayout => "orchestration layout is missing destination modules",
            Self::EmptyAgentId => "agent orchestration id cannot be empty",
            Self::EmptySwarmId => "swarm orchestration id cannot be empty",
            Self::EmptySwarmMembers => "swarm orchestration must contain at least one member",
            Self::EmptyCouncilId => "council orchestration id cannot be empty",
            Self::EmptyCouncilSeats => "council orchestration must contain at least one seat",
            Self::EmptyWorkflowId => "workflow orchestration id cannot be empty",
            Self::EmptyWorkflowPhase => "workflow orchestration phase cannot be empty",
            Self::EmptyGoalId => "goal orchestration id cannot be empty",
            Self::EmptyGoalCondition => "goal orchestration condition cannot be empty",
            Self::EmptyEventActor => "orchestration actor id cannot be empty",
            Self::EmptyEventSummary => "orchestration event summary cannot be empty",
        })
    }
}

impl std::error::Error for OrchestrationSkeletonError {}

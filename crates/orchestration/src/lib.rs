mod error;
mod trace;

pub mod agents;
pub mod council;
pub mod event;
pub mod goals;
pub mod layout;
pub mod service;
pub mod swarm;
pub mod workflows;

pub use agents::{AgentOrchestration, AgentOrchestrationRole};
pub use council::CouncilOrchestration;
pub use error::OrchestrationSkeletonError;
pub use event::{OrchestrationEvent, OrchestrationEventKind};
pub use goals::GoalOrchestration;
pub use layout::{OrchestrationLayout, OrchestrationModule};
pub use service::{InMemoryOrchestrationEventService, OrchestrationEventService};
pub use swarm::SwarmOrchestration;
pub use workflows::WorkflowOrchestration;

pub(crate) fn non_empty_string(
    value: impl Into<String>,
    error: OrchestrationSkeletonError,
) -> Result<String, OrchestrationSkeletonError> {
    let value = value.into();
    if value.trim().is_empty() {
        return Err(error);
    }

    Ok(value)
}

#[cfg(test)]
mod tests;

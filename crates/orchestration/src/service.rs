use crate::{OrchestrationEvent, OrchestrationLayout, OrchestrationSkeletonError};
use serde::{Deserialize, Serialize};

pub trait OrchestrationEventService {
    fn layout(&self) -> &OrchestrationLayout;

    fn record_event(
        &mut self,
        event: OrchestrationEvent,
    ) -> Result<&OrchestrationEvent, OrchestrationSkeletonError>;

    fn events(&self) -> &[OrchestrationEvent];
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InMemoryOrchestrationEventService {
    layout: OrchestrationLayout,
    events: Vec<OrchestrationEvent>,
}

impl InMemoryOrchestrationEventService {
    pub fn new(layout: OrchestrationLayout) -> Result<Self, OrchestrationSkeletonError> {
        if !layout.is_complete_destination_skeleton() {
            return Err(OrchestrationSkeletonError::IncompleteLayout);
        }

        Ok(Self {
            layout,
            events: Vec::new(),
        })
    }

    pub fn into_events(self) -> Vec<OrchestrationEvent> {
        self.events
    }
}

impl OrchestrationEventService for InMemoryOrchestrationEventService {
    fn layout(&self) -> &OrchestrationLayout {
        &self.layout
    }

    fn record_event(
        &mut self,
        event: OrchestrationEvent,
    ) -> Result<&OrchestrationEvent, OrchestrationSkeletonError> {
        let index = self.events.len();
        self.events.push(event);
        Ok(&self.events[index])
    }

    fn events(&self) -> &[OrchestrationEvent] {
        &self.events
    }
}

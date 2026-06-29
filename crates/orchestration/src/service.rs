use crate::{OrchestrationEvent, OrchestrationLayout, OrchestrationSkeletonError, trace};
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
        let _linkscope_service = linkscope::phase("orchestration.service.new");
        trace::record_layout("orchestration.service.new.layout", layout.modules().len());
        if !layout.is_complete_destination_skeleton() {
            trace::record_error("orchestration.service.new", "incomplete_layout");
            return Err(OrchestrationSkeletonError::IncompleteLayout);
        }

        Ok(Self {
            layout,
            events: Vec::new(),
        })
    }

    pub fn into_events(self) -> Vec<OrchestrationEvent> {
        trace::record_collection(trace::CollectionTrace {
            label: "orchestration.service.into_events",
            id: "service",
            item_label: "events",
            items: self.events.len(),
        });
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
        let _linkscope_record = linkscope::phase("orchestration.service.record_event");
        let index = self.events.len();
        self.events.push(event);
        trace::record_collection(trace::CollectionTrace {
            label: "orchestration.service.record_event",
            id: "service",
            item_label: "events",
            items: self.events.len(),
        });
        Ok(&self.events[index])
    }

    fn events(&self) -> &[OrchestrationEvent] {
        &self.events
    }
}

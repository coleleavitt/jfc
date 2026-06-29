use crate::{OrchestrationSkeletonError, non_empty_string, trace};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CouncilOrchestration {
    id: String,
    seats: Vec<String>,
}

impl CouncilOrchestration {
    pub fn new(
        id: impl Into<String>,
        seats: Vec<String>,
    ) -> Result<Self, OrchestrationSkeletonError> {
        let _linkscope_council = linkscope::phase("orchestration.council.new");
        if seats.is_empty() {
            trace::record_error("orchestration.council.new", "empty_seats");
            return Err(OrchestrationSkeletonError::EmptyCouncilSeats);
        }
        let id = non_empty_string(id, OrchestrationSkeletonError::EmptyCouncilId)?;
        trace::record_collection(trace::CollectionTrace {
            label: "orchestration.council.new",
            id: &id,
            item_label: "seats",
            items: seats.len(),
        });

        Ok(Self { id, seats })
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn seats(&self) -> &[String] {
        &self.seats
    }
}

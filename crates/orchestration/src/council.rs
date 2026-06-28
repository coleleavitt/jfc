use crate::{OrchestrationSkeletonError, non_empty_string};
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
        if seats.is_empty() {
            return Err(OrchestrationSkeletonError::EmptyCouncilSeats);
        }

        Ok(Self {
            id: non_empty_string(id, OrchestrationSkeletonError::EmptyCouncilId)?,
            seats,
        })
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn seats(&self) -> &[String] {
        &self.seats
    }
}

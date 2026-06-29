use crate::{OrchestrationSkeletonError, non_empty_string, trace};
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
        let _linkscope_swarm = linkscope::phase("orchestration.swarm.new");
        if members.is_empty() {
            trace::record_error("orchestration.swarm.new", "empty_members");
            return Err(OrchestrationSkeletonError::EmptySwarmMembers);
        }
        let id = non_empty_string(id, OrchestrationSkeletonError::EmptySwarmId)?;
        trace::record_collection(trace::CollectionTrace {
            label: "orchestration.swarm.new",
            id: &id,
            item_label: "members",
            items: members.len(),
        });

        Ok(Self { id, members })
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn members(&self) -> &[String] {
        &self.members
    }
}

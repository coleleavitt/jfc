use crate::OrchestrationSkeletonError;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OrchestrationModule {
    Agents,
    Swarm,
    Council,
    Workflows,
    Goals,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OrchestrationLayout {
    modules: Vec<OrchestrationModule>,
}

impl OrchestrationLayout {
    pub fn new(
        modules: impl IntoIterator<Item = OrchestrationModule>,
    ) -> Result<Self, OrchestrationSkeletonError> {
        let modules = modules.into_iter().collect::<Vec<_>>();
        if modules.is_empty() {
            return Err(OrchestrationSkeletonError::EmptyLayout);
        }

        Ok(Self { modules })
    }

    pub fn destination_skeleton() -> Self {
        Self {
            modules: destination_modules().to_vec(),
        }
    }

    pub fn modules(&self) -> &[OrchestrationModule] {
        &self.modules
    }

    pub fn is_complete_destination_skeleton(&self) -> bool {
        destination_modules()
            .iter()
            .all(|module| self.modules.contains(module))
    }
}

fn destination_modules() -> &'static [OrchestrationModule; 5] {
    &[
        OrchestrationModule::Agents,
        OrchestrationModule::Swarm,
        OrchestrationModule::Council,
        OrchestrationModule::Workflows,
        OrchestrationModule::Goals,
    ]
}

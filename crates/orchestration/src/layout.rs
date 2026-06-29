use crate::{OrchestrationSkeletonError, trace};
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
        let _linkscope_layout = linkscope::phase("orchestration.layout.new");
        let modules = modules.into_iter().collect::<Vec<_>>();
        trace::record_layout("orchestration.layout.new", modules.len());
        if modules.is_empty() {
            return Err(OrchestrationSkeletonError::EmptyLayout);
        }

        Ok(Self { modules })
    }

    pub fn destination_skeleton() -> Self {
        let _linkscope_layout = linkscope::phase("orchestration.layout.destination_skeleton");
        trace::record_layout(
            "orchestration.layout.destination_skeleton",
            destination_modules().len(),
        );
        Self {
            modules: destination_modules().to_vec(),
        }
    }

    pub fn modules(&self) -> &[OrchestrationModule] {
        &self.modules
    }

    pub fn is_complete_destination_skeleton(&self) -> bool {
        let complete = destination_modules()
            .iter()
            .all(|module| self.modules.contains(module));
        trace::record_layout_complete(self.modules.len(), complete);
        complete
    }
}

impl OrchestrationModule {
    pub fn label(self) -> &'static str {
        match self {
            Self::Agents => "agents",
            Self::Swarm => "swarm",
            Self::Council => "council",
            Self::Workflows => "workflows",
            Self::Goals => "goals",
        }
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

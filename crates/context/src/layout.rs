use crate::ContextSkeletonError;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContextModule {
    Contributors,
    Health,
    Memory,
    History,
    Reduce,
    Search,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContextLayout {
    modules: Vec<ContextModule>,
}

impl ContextLayout {
    pub fn new(
        modules: impl IntoIterator<Item = ContextModule>,
    ) -> Result<Self, ContextSkeletonError> {
        let modules = modules.into_iter().collect::<Vec<_>>();
        if modules.is_empty() {
            return Err(ContextSkeletonError::EmptyLayout);
        }

        Ok(Self { modules })
    }

    pub fn destination_skeleton() -> Self {
        Self {
            modules: destination_modules().to_vec(),
        }
    }

    pub fn modules(&self) -> &[ContextModule] {
        &self.modules
    }

    pub fn is_complete_destination_skeleton(&self) -> bool {
        destination_modules()
            .iter()
            .all(|module| self.modules.contains(module))
    }
}

fn destination_modules() -> &'static [ContextModule; 6] {
    &[
        ContextModule::Contributors,
        ContextModule::Health,
        ContextModule::Memory,
        ContextModule::History,
        ContextModule::Reduce,
        ContextModule::Search,
    ]
}

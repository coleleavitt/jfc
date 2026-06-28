use crate::ContextSkeletonError;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ContributorId(String);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContextContributor {
    id: ContributorId,
    label: String,
}

impl ContributorId {
    pub fn new(id: impl Into<String>) -> Result<Self, ContextSkeletonError> {
        let id = id.into();
        if id.trim().is_empty() {
            return Err(ContextSkeletonError::EmptyContributorId);
        }

        Ok(Self(id))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl ContextContributor {
    pub fn new(id: ContributorId, label: impl Into<String>) -> Self {
        Self {
            id,
            label: label.into(),
        }
    }

    pub fn try_new(
        id: ContributorId,
        label: impl Into<String>,
    ) -> Result<Self, ContextSkeletonError> {
        let label = label.into();
        if label.trim().is_empty() {
            return Err(ContextSkeletonError::EmptyContributorLabel);
        }

        Ok(Self { id, label })
    }

    pub fn id(&self) -> &ContributorId {
        &self.id
    }

    pub fn label(&self) -> &str {
        &self.label
    }
}

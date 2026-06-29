use crate::ContextSkeletonError;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MemoryAnchor(String);

impl MemoryAnchor {
    pub fn new(anchor: impl Into<String>) -> Result<Self, ContextSkeletonError> {
        let anchor = anchor.into();
        if anchor.trim().is_empty() {
            return Err(ContextSkeletonError::EmptyMemoryAnchor);
        }

        Ok(Self(anchor))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

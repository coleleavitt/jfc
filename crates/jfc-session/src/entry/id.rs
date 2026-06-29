use serde::{Deserialize, Serialize};

use super::validation::{self, SessionEntryValidationError};

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SessionEntryId(String);

impl SessionEntryId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    pub fn parse(id: impl Into<String>) -> Result<Self, SessionEntryValidationError> {
        let id = Self::new(id);
        id.validate()?;
        Ok(id)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn validate(&self) -> Result<(), SessionEntryValidationError> {
        validation::validate_non_empty(&self.0, SessionEntryValidationError::EmptySessionEntryId)
    }

    pub(crate) fn validate_as_parent(&self) -> Result<(), SessionEntryValidationError> {
        validation::validate_non_empty(
            &self.0,
            SessionEntryValidationError::EmptyParentSessionEntryId,
        )
    }
}

impl From<&str> for SessionEntryId {
    fn from(value: &str) -> Self {
        Self::new(value)
    }
}

impl From<String> for SessionEntryId {
    fn from(value: String) -> Self {
        Self::new(value)
    }
}

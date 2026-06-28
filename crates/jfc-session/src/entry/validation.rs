use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionEntryValidationError {
    EmptySessionEntryId,
    EmptyParentSessionEntryId,
    EmptyTimestamp,
    EmptyPluginId,
    EmptyCustomType,
}

impl fmt::Display for SessionEntryValidationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::EmptySessionEntryId => "session entry id cannot be empty",
            Self::EmptyParentSessionEntryId => "parent session entry id cannot be empty",
            Self::EmptyTimestamp => "session entry timestamp cannot be empty",
            Self::EmptyPluginId => "custom plugin entry plugin_id cannot be empty",
            Self::EmptyCustomType => "custom plugin entry custom_type cannot be empty",
        })
    }
}

impl std::error::Error for SessionEntryValidationError {}

pub(super) fn validate_non_empty(
    value: &str,
    error: SessionEntryValidationError,
) -> Result<(), SessionEntryValidationError> {
    if value.trim().is_empty() {
        Err(error)
    } else {
        Ok(())
    }
}

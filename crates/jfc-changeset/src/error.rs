use std::io;

use crate::state::ChangeState;

/// Top-level error type for the jfc-changeset crate.
#[derive(Debug, thiserror::Error)]
pub enum ChangeSetError {
    /// I/O error (file system, lock acquisition, etc.)
    #[error("I/O error ({context}): {source}")]
    Io {
        source: io::Error,
        context: String,
    },

    /// JSON serialization / deserialization failure.
    #[error("serialization error ({context}): {source}")]
    Serde {
        source: serde_json::Error,
        context: String,
    },

    /// A lifecycle transition that the state machine forbids. This is the
    /// load-bearing guard behind "reviewed and tested before it touches
    /// production": e.g. `Ready -> Applied` is rejected because it skips
    /// `Tested` and `Approved`.
    #[error("illegal change-set transition: {from:?} -> {to:?} ({reason})")]
    IllegalTransition {
        from: ChangeState,
        to: ChangeState,
        reason: String,
    },

    /// A change-set id was referenced that is not in the store.
    #[error("change-set {id} not found in store")]
    NotFound { id: String },

    /// The store's backing file is corrupt or contains invalid data.
    #[error("change-set store corrupt: {message}")]
    StoreCorrupt { message: String },
}

impl ChangeSetError {
    pub(crate) fn io(source: io::Error, context: impl Into<String>) -> Self {
        Self::Io {
            source,
            context: context.into(),
        }
    }

    pub(crate) fn serde(source: serde_json::Error, context: impl Into<String>) -> Self {
        Self::Serde {
            source,
            context: context.into(),
        }
    }
}

pub type Result<T> = std::result::Result<T, ChangeSetError>;

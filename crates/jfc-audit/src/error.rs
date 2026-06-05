use std::fmt;
use std::io;

/// Top-level error type for the jfc-audit crate.
#[derive(Debug)]
pub enum AuditError {
    /// I/O error (file system, lock acquisition, etc.)
    Io { source: io::Error, context: String },
    /// JSON serialization/deserialization failure
    Serde {
        source: serde_json::Error,
        context: String,
    },
    /// Finding store is corrupt or contains invalid data
    StoreCorrupt { message: String },
    /// A required graph query failed
    GraphQuery { message: String },
    /// Budget exhausted before completion
    BudgetExhausted { tokens_spent: u64, budget: u64 },
    /// Taint spec file is malformed
    MalformedTaintSpecs { message: String },
    /// Generic internal error
    Internal { message: String },
}

impl fmt::Display for AuditError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { source, context } => write!(f, "I/O error ({context}): {source}"),
            Self::Serde { source, context } => {
                write!(f, "serialization error ({context}): {source}")
            }
            Self::StoreCorrupt { message } => write!(f, "store corrupt: {message}"),
            Self::GraphQuery { message } => write!(f, "graph query failed: {message}"),
            Self::BudgetExhausted {
                tokens_spent,
                budget,
            } => write!(f, "budget exhausted: spent {tokens_spent}/{budget} tokens"),
            Self::MalformedTaintSpecs { message } => {
                write!(f, "malformed taint specs: {message}")
            }
            Self::Internal { message } => write!(f, "internal error: {message}"),
        }
    }
}

impl std::error::Error for AuditError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            Self::Serde { source, .. } => Some(source),
            _ => None,
        }
    }
}

impl From<io::Error> for AuditError {
    fn from(e: io::Error) -> Self {
        Self::Io {
            source: e,
            context: "unspecified".to_string(),
        }
    }
}

impl From<serde_json::Error> for AuditError {
    fn from(e: serde_json::Error) -> Self {
        Self::Serde {
            source: e,
            context: "unspecified".to_string(),
        }
    }
}

pub type Result<T> = std::result::Result<T, AuditError>;

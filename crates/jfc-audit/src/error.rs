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
        linkscope::detail_event_fields(
            "audit.error.display",
            [linkscope::TraceField::text("kind", self.kind_label())],
        );
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
        linkscope::detail_event_fields(
            "audit.error.source",
            [linkscope::TraceField::text("kind", self.kind_label())],
        );
        match self {
            Self::Io { source, .. } => Some(source),
            Self::Serde { source, .. } => Some(source),
            _ => None,
        }
    }
}

impl From<io::Error> for AuditError {
    fn from(e: io::Error) -> Self {
        linkscope::event_fields(
            "audit.error.from_io",
            [linkscope::TraceField::text("io_kind", e.kind().to_string())],
        );
        Self::Io {
            source: e,
            context: "unspecified".to_string(),
        }
    }
}

impl From<serde_json::Error> for AuditError {
    fn from(e: serde_json::Error) -> Self {
        linkscope::event_fields(
            "audit.error.from_serde",
            [linkscope::TraceField::text(
                "category",
                format!("{:?}", e.classify()),
            )],
        );
        Self::Serde {
            source: e,
            context: "unspecified".to_string(),
        }
    }
}

impl AuditError {
    fn kind_label(&self) -> &'static str {
        match self {
            Self::Io { .. } => "io",
            Self::Serde { .. } => "serde",
            Self::StoreCorrupt { .. } => "store_corrupt",
            Self::GraphQuery { .. } => "graph_query",
            Self::BudgetExhausted { .. } => "budget_exhausted",
            Self::MalformedTaintSpecs { .. } => "malformed_taint_specs",
            Self::Internal { .. } => "internal",
        }
    }
}

pub type Result<T> = std::result::Result<T, AuditError>;

//! Error types for jfc-learn.

use snafu::Snafu;

#[derive(Debug, Snafu)]
pub enum LearnError {
    #[snafu(display("IO error: {source}"))]
    Io { source: std::io::Error },

    #[snafu(display("JSON serialization error: {source}"))]
    Json { source: serde_json::Error },

    #[snafu(display("Provider error: {message}"))]
    Provider { message: String },

    #[snafu(display("Lease conflict: {message}"))]
    LeaseConflict { message: String },

    #[snafu(display("Circuit breaker fired after {failures} consecutive failures"))]
    CircuitBreaker { failures: usize },

    #[snafu(display("Parse error: {message}"))]
    Parse { message: String },

    #[snafu(display("Contract violation: {message}"))]
    ContractViolation { message: String },
}

impl From<std::io::Error> for LearnError {
    fn from(source: std::io::Error) -> Self {
        LearnError::Io { source }
    }
}

impl From<serde_json::Error> for LearnError {
    fn from(source: serde_json::Error) -> Self {
        LearnError::Json { source }
    }
}

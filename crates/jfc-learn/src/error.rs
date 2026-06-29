//! Error types for jfc-learn.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum LearnError {
    #[error("IO error: {source}")]
    Io {
        #[from]
        source: std::io::Error,
    },

    #[error("JSON serialization error: {source}")]
    Json {
        #[from]
        source: serde_json::Error,
    },

    #[error("Knowledge store error: {source}")]
    Knowledge {
        #[from]
        source: jfc_knowledge::KnowledgeError,
    },

    #[error("Provider error: {message}")]
    Provider { message: String },

    #[error("Lease conflict: {message}")]
    LeaseConflict { message: String },

    #[error("Circuit breaker fired after {failures} consecutive failures")]
    CircuitBreaker { failures: usize },

    #[error("Parse error: {message}")]
    Parse { message: String },

    #[error("Contract violation: {message}")]
    ContractViolation { message: String },

    #[error("RSI curator error: {source}")]
    Rsi {
        #[from]
        source: rsi_rs::RsiError,
    },
}

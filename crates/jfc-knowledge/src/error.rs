//! Error type for the knowledge store.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum KnowledgeError {
    #[error("sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),

    #[error("knowledge store io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("schema migration failed: {0}")]
    Migration(String),

    #[error("invalid record: {0}")]
    InvalidRecord(String),
}

pub type Result<T> = std::result::Result<T, KnowledgeError>;

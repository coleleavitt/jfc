use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum IdentifierError {
    #[error("identifier cannot be empty")]
    Empty,
    #[error("identifier contains whitespace: {value}")]
    ContainsWhitespace { value: String },
}

#[derive(Debug, Error)]
#[non_exhaustive]
pub enum PluginSdkError {
    #[error("invalid identifier")]
    InvalidIdentifier(#[from] IdentifierError),
    #[error("unknown hook name: {0}")]
    UnknownHookName(String),
}

//! Transcript view models.

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TranscriptViewModel {
    pub rows: Vec<TranscriptRow>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TranscriptRow {
    pub text: String,
}

impl TranscriptRow {
    pub fn new(text: impl Into<String>) -> Self {
        Self { text: text.into() }
    }
}

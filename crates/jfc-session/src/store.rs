use async_trait::async_trait;

use crate::{SessionHit, SessionId, SessionMetadata};

mod default_store;

pub type StoredSessionMessage = jfc_knowledge::SessionMessage;

pub use default_store::{DefaultSessionStore, default_session_store};

#[async_trait]
pub trait SessionStore: Send + Sync {
    async fn save_transcript(&self, request: SaveTranscriptRequest<'_>);
    async fn load_transcript(&self, session_id: &SessionId) -> Option<SessionTranscript>;
    async fn set_title(&self, session_id: &SessionId, title: &str);
    async fn list_sessions(&self, request: ListSessionsRequest<'_>) -> Vec<SessionMetadata>;
    fn search_sessions(&self, request: SearchSessionsRequest<'_>) -> Vec<SessionHit>;
    async fn request_autosave(&self, request: AutosaveRequest<'_>) -> AutosaveOutcome;
}

pub struct SaveTranscriptRequest<'a> {
    pub session_id: &'a SessionId,
    pub messages: &'a [StoredSessionMessage],
    pub cwd: Option<&'a str>,
    pub model: Option<&'a str>,
    pub first_prompt: Option<&'a str>,
    pub title: Option<&'a str>,
}

impl<'a> SaveTranscriptRequest<'a> {
    pub const fn new(session_id: &'a SessionId, messages: &'a [StoredSessionMessage]) -> Self {
        Self {
            session_id,
            messages,
            cwd: None,
            model: None,
            first_prompt: None,
            title: None,
        }
    }

    pub const fn with_cwd(mut self, cwd: Option<&'a str>) -> Self {
        self.cwd = cwd;
        self
    }

    pub const fn with_model(mut self, model: Option<&'a str>) -> Self {
        self.model = model;
        self
    }

    pub const fn with_first_prompt(mut self, first_prompt: Option<&'a str>) -> Self {
        self.first_prompt = first_prompt;
        self
    }

    pub const fn with_title(mut self, title: Option<&'a str>) -> Self {
        self.title = title;
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionTranscript {
    pub messages: Vec<StoredSessionMessage>,
    pub model: Option<String>,
}

pub struct ListSessionsRequest<'a> {
    pub cwd_filter: Option<&'a str>,
    pub limit: Option<usize>,
}

impl ListSessionsRequest<'_> {
    pub const fn all() -> Self {
        Self {
            cwd_filter: None,
            limit: None,
        }
    }
}

pub struct SearchSessionsRequest<'a> {
    pub query: &'a str,
    pub limit: usize,
    pub window: usize,
    pub exclude_session: Option<&'a str>,
}

pub struct AutosaveRequest<'a> {
    pub transcript: SaveTranscriptRequest<'a>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AutosaveOutcome {
    Saved,
}

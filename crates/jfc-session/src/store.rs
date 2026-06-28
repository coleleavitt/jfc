use async_trait::async_trait;

use crate::{SessionHit, SessionId, SessionMetadata};

pub type StoredSessionMessage = jfc_knowledge::SessionMessage;

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

#[derive(Debug, Default, Clone, Copy)]
pub struct DefaultSessionStore;

pub const fn default_session_store() -> DefaultSessionStore {
    DefaultSessionStore
}

#[async_trait]
impl SessionStore for DefaultSessionStore {
    async fn save_transcript(&self, request: SaveTranscriptRequest<'_>) {
        let session_id = request.session_id.as_str().to_owned();
        let messages = request.messages.to_vec();
        let cwd = request.cwd.map(str::to_owned);
        let model = request.model.map(str::to_owned);
        let first_prompt = request.first_prompt.map(str::to_owned);
        let title = request.title.map(str::to_owned);
        let now = chrono::Utc::now().to_rfc3339();

        let result = jfc_knowledge::block_on_knowledge(async move {
            let store = crate::open_default_knowledge_store().await?;
            let prior = store.get_session(&session_id).await?.unwrap_or_else(|| {
                jfc_knowledge::SessionRow {
                    id: session_id.clone(),
                    cwd: None,
                    model: None,
                    created_at: None,
                    updated_at: None,
                    first_prompt: None,
                    title: None,
                    message_count: 0,
                }
            });
            let row = jfc_knowledge::SessionRow {
                id: session_id,
                cwd: prior.cwd.or(cwd),
                model: model.or(prior.model),
                created_at: prior.created_at.or_else(|| Some(now.clone())),
                updated_at: Some(now),
                first_prompt: first_prompt.or(prior.first_prompt),
                title: title.or(prior.title),
                message_count: messages.len() as i64,
            };
            store.replace_transcript(&row, &messages).await
        });

        if let Err(err) = result {
            tracing::debug!(target: "jfc::session", error = %err, "session facade transcript save skipped");
        }
    }

    async fn load_transcript(&self, session_id: &SessionId) -> Option<SessionTranscript> {
        let session_id = session_id.as_str().to_owned();
        jfc_knowledge::block_on_knowledge(async move {
            let store = crate::open_default_knowledge_store().await.ok()?;
            let row = store.get_session(&session_id).await.ok().flatten()?;
            let messages = store.load_transcript(&session_id).await.ok()?;
            if messages.is_empty() {
                return None;
            }
            Some(SessionTranscript {
                messages,
                model: row.model,
            })
        })
    }

    async fn set_title(&self, session_id: &SessionId, title: &str) {
        let session_id = session_id.as_str().to_owned();
        let title = title.to_owned();
        let updated_at = chrono::Utc::now().to_rfc3339();
        let result = jfc_knowledge::block_on_knowledge(async move {
            let store = crate::open_default_knowledge_store().await?;
            if let Some(mut row) = store.get_session(&session_id).await? {
                row.title = Some(title);
                row.updated_at = Some(updated_at);
                store.upsert_session(&row).await?;
            }
            Ok::<(), jfc_knowledge::KnowledgeError>(())
        });
        if let Err(err) = result {
            tracing::debug!(target: "jfc::session", error = %err, "session facade title update skipped");
        }
    }

    async fn list_sessions(&self, request: ListSessionsRequest<'_>) -> Vec<SessionMetadata> {
        let mut sessions = crate::list_sessions_filtered(request.cwd_filter).await;
        if let Some(limit) = request.limit {
            sessions.truncate(limit);
        }
        sessions
    }

    fn search_sessions(&self, request: SearchSessionsRequest<'_>) -> Vec<SessionHit> {
        crate::search_sessions_excluding(
            request.query,
            request.limit,
            request.window,
            request.exclude_session,
        )
    }

    async fn request_autosave(&self, request: AutosaveRequest<'_>) -> AutosaveOutcome {
        self.save_transcript(request.transcript).await;
        AutosaveOutcome::Saved
    }
}

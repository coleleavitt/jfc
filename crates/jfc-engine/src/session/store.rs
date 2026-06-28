use async_trait::async_trait;

use crate::ids::SessionId;
use crate::types::ChatMessage;

pub use jfc_session::{
    AutosaveOutcome, ListSessionsRequest, SearchSessionsRequest, SessionStore, SessionTranscript,
    StoredSessionMessage,
};

pub struct SaveTranscriptRequest<'a> {
    pub session_id: &'a SessionId,
    pub messages: &'a [ChatMessage],
    pub cwd: Option<&'a str>,
    pub model: Option<&'a str>,
}

impl<'a> SaveTranscriptRequest<'a> {
    pub const fn new(session_id: &'a SessionId, messages: &'a [ChatMessage]) -> Self {
        Self {
            session_id,
            messages,
            cwd: None,
            model: None,
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
}

pub struct LoadedTranscript {
    pub messages: Vec<ChatMessage>,
    pub model: Option<String>,
}

pub struct AutosaveRequest<'a> {
    pub transcript: SaveTranscriptRequest<'a>,
}

#[derive(Debug, Default, Clone, Copy)]
pub struct DefaultSessionStore;

pub const fn default_session_store() -> DefaultSessionStore {
    DefaultSessionStore
}

impl DefaultSessionStore {
    pub async fn save_transcript(&self, request: SaveTranscriptRequest<'_>) {
        super::core::save_session(
            request.session_id,
            request.messages,
            request.cwd,
            request.model,
        )
        .await;
    }

    pub async fn load_transcript(&self, session_id: &SessionId) -> Option<Vec<ChatMessage>> {
        super::core::load_session(session_id).await
    }

    pub async fn load_transcript_with_model(
        &self,
        session_id: &SessionId,
    ) -> Option<LoadedTranscript> {
        super::core::load_session_with_model(session_id)
            .await
            .map(|(messages, model)| LoadedTranscript { messages, model })
    }

    pub async fn set_title(&self, session_id: &SessionId, title: &str) {
        super::core::set_session_title(session_id, title).await;
    }

    pub async fn list_sessions(
        &self,
        request: ListSessionsRequest<'_>,
    ) -> Vec<jfc_session::SessionMetadata> {
        let mut sessions = jfc_session::list_sessions_filtered(request.cwd_filter).await;
        if let Some(limit) = request.limit {
            sessions.truncate(limit);
        }
        sessions
    }

    pub fn search_sessions(
        &self,
        request: SearchSessionsRequest<'_>,
    ) -> Vec<jfc_session::SessionHit> {
        jfc_session::search_sessions_excluding(
            request.query,
            request.limit,
            request.window,
            request.exclude_session,
        )
    }

    pub async fn request_autosave(&self, request: AutosaveRequest<'_>) -> AutosaveOutcome {
        self.save_transcript(request.transcript).await;
        AutosaveOutcome::Saved
    }
}

#[async_trait]
impl SessionStore for DefaultSessionStore {
    async fn save_transcript(&self, request: jfc_session::SaveTranscriptRequest<'_>) {
        jfc_session::SessionStore::save_transcript(&jfc_session::default_session_store(), request)
            .await;
    }

    async fn load_transcript(&self, session_id: &SessionId) -> Option<SessionTranscript> {
        jfc_session::SessionStore::load_transcript(
            &jfc_session::default_session_store(),
            session_id,
        )
        .await
    }

    async fn set_title(&self, session_id: &SessionId, title: &str) {
        jfc_session::SessionStore::set_title(
            &jfc_session::default_session_store(),
            session_id,
            title,
        )
        .await;
    }

    async fn list_sessions(
        &self,
        request: ListSessionsRequest<'_>,
    ) -> Vec<jfc_session::SessionMetadata> {
        jfc_session::SessionStore::list_sessions(&jfc_session::default_session_store(), request)
            .await
    }

    fn search_sessions(&self, request: SearchSessionsRequest<'_>) -> Vec<jfc_session::SessionHit> {
        jfc_session::SessionStore::search_sessions(&jfc_session::default_session_store(), request)
    }

    async fn request_autosave(&self, request: jfc_session::AutosaveRequest<'_>) -> AutosaveOutcome {
        jfc_session::SessionStore::request_autosave(&jfc_session::default_session_store(), request)
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;
    use std::sync::Mutex;
    use tempfile::TempDir;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    struct TempSessionEnv {
        _dir: TempDir,
        prior_config: Option<String>,
        prior_db: Option<String>,
        _guard: std::sync::MutexGuard<'static, ()>,
    }

    impl TempSessionEnv {
        fn new() -> Self {
            let guard = ENV_LOCK.lock().unwrap_or_else(|err| err.into_inner());
            let dir = TempDir::new().expect("tempdir");
            let prior_config = std::env::var("XDG_CONFIG_HOME").ok();
            let prior_db = std::env::var("JFC_KNOWLEDGE_DB").ok();
            let db_path = dir.path().join("knowledge.db");
            unsafe {
                std::env::set_var("XDG_CONFIG_HOME", dir.path());
                std::env::set_var("JFC_KNOWLEDGE_DB", db_path);
            }
            Self {
                _dir: dir,
                prior_config,
                prior_db,
                _guard: guard,
            }
        }
    }

    impl Drop for TempSessionEnv {
        fn drop(&mut self) {
            unsafe {
                match self.prior_config.take() {
                    Some(value) => std::env::set_var("XDG_CONFIG_HOME", value),
                    None => std::env::remove_var("XDG_CONFIG_HOME"),
                }
                match self.prior_db.take() {
                    Some(value) => std::env::set_var("JFC_KNOWLEDGE_DB", value),
                    None => std::env::remove_var("JFC_KNOWLEDGE_DB"),
                }
            }
        }
    }

    #[serial]
    #[tokio::test]
    async fn session_store_save_load_title_preservation_normal() {
        let _env = TempSessionEnv::new();
        let store = default_session_store();
        let session_id = SessionId::new("ses_20260627_120000");
        let messages = vec![
            ChatMessage::user("Remember this seam".into()),
            ChatMessage::assistant("Seam preserved".into()),
        ];

        store
            .save_transcript(
                SaveTranscriptRequest::new(&session_id, &messages)
                    .with_cwd(Some("/tmp/jfc-session-store"))
                    .with_model(Some("test-model")),
            )
            .await;
        store.set_title(&session_id, "Store seam title").await;

        let loaded = store
            .load_transcript(&session_id)
            .await
            .expect("transcript loads through store seam");
        let loaded_with_model = store
            .load_transcript_with_model(&session_id)
            .await
            .expect("model loads through store seam");
        let listed = store
            .list_sessions(ListSessionsRequest {
                cwd_filter: Some("/tmp/jfc-session-store"),
                limit: Some(1),
            })
            .await;

        assert_eq!(loaded.len(), 2);
        assert_eq!(loaded[0].parts[0].text_only(), "Remember this seam");
        assert_eq!(loaded[1].parts[0].text_only(), "Seam preserved");
        assert_eq!(loaded_with_model.model.as_deref(), Some("test-model"));
        assert_eq!(loaded_with_model.messages.len(), 2);
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].title.as_deref(), Some("Store seam title"));
        assert_eq!(listed[0].display_title(), "Store seam title");
    }
}

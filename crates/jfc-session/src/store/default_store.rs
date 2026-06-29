use async_trait::async_trait;

use super::{
    AutosaveOutcome, AutosaveRequest, ListSessionsRequest, SaveTranscriptRequest,
    SearchSessionsRequest, SessionStore, SessionTranscript,
};
use crate::{SessionHit, SessionId, SessionMetadata};

#[derive(Debug, Default, Clone, Copy)]
pub struct DefaultSessionStore;

pub const fn default_session_store() -> DefaultSessionStore {
    DefaultSessionStore
}

fn bool_to_u64(value: bool) -> u64 {
    u64::from(value)
}

#[async_trait]
impl SessionStore for DefaultSessionStore {
    async fn save_transcript(&self, request: SaveTranscriptRequest<'_>) {
        let _linkscope_save = linkscope::phase("session.store.save_transcript");
        let message_count = request.messages.len();
        linkscope::event_fields(
            "session.store.save_transcript.request",
            [
                linkscope::TraceField::count("messages", usize_to_u64_saturating(message_count)),
                linkscope::TraceField::count("has_cwd", bool_to_u64(request.cwd.is_some())),
                linkscope::TraceField::count("has_model", bool_to_u64(request.model.is_some())),
                linkscope::TraceField::count(
                    "has_first_prompt",
                    bool_to_u64(request.first_prompt.is_some()),
                ),
                linkscope::TraceField::count("has_title", bool_to_u64(request.title.is_some())),
            ],
        );
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
            linkscope::record_items("session.store.save_transcript.error", 1);
            tracing::debug!(target: "jfc::session", error = %err, "session facade transcript save skipped");
        } else {
            linkscope::record_items(
                "session.store.save_transcript.saved",
                usize_to_u64_saturating(message_count),
            );
        }
    }

    async fn load_transcript(&self, session_id: &SessionId) -> Option<SessionTranscript> {
        let _linkscope_load = linkscope::phase("session.store.load_transcript");
        let session_id = session_id.as_str().to_owned();
        let transcript = jfc_knowledge::block_on_knowledge(async move {
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
        });
        linkscope::event_fields(
            "session.store.load_transcript.result",
            [
                linkscope::TraceField::count("hit", bool_to_u64(transcript.is_some())),
                linkscope::TraceField::count(
                    "messages",
                    transcript
                        .as_ref()
                        .map(|value| usize_to_u64_saturating(value.messages.len()))
                        .unwrap_or(0),
                ),
            ],
        );
        transcript
    }

    async fn set_title(&self, session_id: &SessionId, title: &str) {
        let _linkscope_title = linkscope::phase("session.store.set_title");
        linkscope::record_bytes(
            "session.store.set_title.title",
            usize_to_u64_saturating(title.len()),
        );
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
            linkscope::record_items("session.store.set_title.error", 1);
            tracing::debug!(target: "jfc::session", error = %err, "session facade title update skipped");
        } else {
            linkscope::record_items("session.store.set_title.ok", 1);
        }
    }

    async fn list_sessions(&self, request: ListSessionsRequest<'_>) -> Vec<SessionMetadata> {
        let _linkscope_list = linkscope::phase("session.store.list_sessions");
        linkscope::event_fields(
            "session.store.list_sessions.request",
            [
                linkscope::TraceField::count("has_cwd", bool_to_u64(request.cwd_filter.is_some())),
                linkscope::TraceField::count(
                    "limit",
                    request.limit.map(usize_to_u64_saturating).unwrap_or(0),
                ),
            ],
        );
        let mut sessions = crate::list_sessions_filtered(request.cwd_filter).await;
        if let Some(limit) = request.limit {
            sessions.truncate(limit);
        }
        linkscope::record_items(
            "session.store.list_sessions.rows",
            usize_to_u64_saturating(sessions.len()),
        );
        sessions
    }

    fn search_sessions(&self, request: SearchSessionsRequest<'_>) -> Vec<SessionHit> {
        let _linkscope_search = linkscope::phase("session.store.search_sessions");
        linkscope::event_fields(
            "session.store.search_sessions.request",
            [
                linkscope::TraceField::bytes(
                    "query_bytes",
                    usize_to_u64_saturating(request.query.len()),
                ),
                linkscope::TraceField::count("limit", usize_to_u64_saturating(request.limit)),
                linkscope::TraceField::count("window", usize_to_u64_saturating(request.window)),
                linkscope::TraceField::count(
                    "has_exclude",
                    bool_to_u64(request.exclude_session.is_some()),
                ),
            ],
        );
        let hits = crate::search_sessions_excluding(
            request.query,
            request.limit,
            request.window,
            request.exclude_session,
        );
        linkscope::record_items(
            "session.store.search_sessions.hits",
            usize_to_u64_saturating(hits.len()),
        );
        hits
    }

    async fn request_autosave(&self, request: AutosaveRequest<'_>) -> AutosaveOutcome {
        let _linkscope_autosave = linkscope::phase("session.store.autosave");
        self.save_transcript(request.transcript).await;
        linkscope::record_items("session.store.autosave.saved", 1);
        AutosaveOutcome::Saved
    }
}

fn usize_to_u64_saturating(value: usize) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
}

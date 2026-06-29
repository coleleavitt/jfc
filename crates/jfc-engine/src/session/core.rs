//! Public async API for saving and loading sessions.
//!
//! Coordinates DB persistence using helpers from `compaction` (message
//! filtering) and `serialization` (data conversion).

use tracing::{debug, info, warn};

use crate::ids::SessionId;
use crate::types::{ChatMessage, MessagePart, validate_turn_invariants};

use super::compaction::{
    extract_first_prompt, persistent_session_messages, repair_loaded_messages,
};
use super::deserialize::deserialize_message;
use super::serialization::{SerializedMessage, SerializedSession};
use super::serialize::serialize_message;

#[tracing::instrument(target = "jfc::session", skip(messages), fields(n = messages.len()))]
pub async fn save_session(
    session_id: &SessionId,
    messages: &[ChatMessage],
    cwd: Option<&str>,
    model: Option<&str>,
) {
    // Surface invariant breakage at the save boundary. We deliberately
    // do NOT block the save — corrupt state is itself debugging signal,
    // and silently dropping the write would hide the very symptom we
    // want to study post-mortem. The warn lands in the trace log with
    // enough context (session id, error variant) to reconstruct what
    // shape went wrong.
    let session_id_str = session_id.as_str();
    let coalesced = persistent_session_messages(messages);
    if let Err(err) = validate_turn_invariants(&coalesced) {
        // Promote tool-in-user (the API-400 fingerprint) and orphan
        // tool_use to error so a `RUST_LOG=warn` grep still surfaces
        // them — the other variants stay at warn since they don't
        // immediately break the wire shape (consecutive same-role
        // turns may be a v126 microcompact / split-message artifact
        // we tolerate on load).
        let variant = match &err {
            crate::types::TurnInvariantError::OrphanToolResult { .. } => "orphan_tool_in_user",
            crate::types::TurnInvariantError::OrphanToolUse { .. } => "orphan_tool_use_no_result",
            crate::types::TurnInvariantError::ConsecutiveUser { .. } => "consecutive_user",
            crate::types::TurnInvariantError::ConsecutiveAssistant { .. } => {
                "consecutive_assistant"
            }
            crate::types::TurnInvariantError::EmptyMessage { .. } => "empty_message",
            crate::types::TurnInvariantError::LeadingAssistant { .. } => "leading_assistant",
        };
        if matches!(
            err,
            crate::types::TurnInvariantError::OrphanToolResult { .. }
                | crate::types::TurnInvariantError::OrphanToolUse { .. }
        ) {
            tracing::error!(
                target: "jfc::session::invariants",
                session_id = session_id_str,
                error = %err,
                variant,
                message_count = coalesced.len(),
                "save_session: API-400 fingerprint (saving anyway for forensics)"
            );
        } else {
            warn!(
                target: "jfc::session::invariants",
                session_id = session_id_str,
                error = %err,
                variant,
                message_count = coalesced.len(),
                "save_session: turn-invariant violation (saving anyway for forensics)"
            );
        }
    }
    let now = chrono::Utc::now();
    let prior_db = load_session_header_from_db(session_id_str).await;

    let created_at = prior_db
        .as_ref()
        .and_then(|s| s.created_at.clone())
        .unwrap_or_else(|| now.to_rfc3339());
    let stored_cwd = prior_db
        .as_ref()
        .and_then(|s| s.cwd.clone())
        .or_else(|| cwd.map(str::to_owned))
        .or_else(|| {
            std::env::current_dir()
                .ok()
                .map(|p| p.display().to_string())
        });
    let title = prior_db.as_ref().and_then(|s| s.title.clone());
    let stored_model = model
        .map(str::to_owned)
        .or_else(|| prior_db.as_ref().and_then(|s| s.model.clone()));

    // Drop any queued-prompt placeholders before serializing. They're a
    // runtime-only construct used to render "⏳ I queued this" in the
    // transcript; persisting them would make resume re-display unsent
    // prompts and (worse) `recompute_token_estimate` count their bytes
    // against the context budget on the next launch.
    //
    // Then coalesce consecutive same-role messages (sub-stream split
    // artifacts) into one logical turn per persisted message. See
    // `coalesce_consecutive_same_role` for the rationale — the file
    // on disk becomes the "alternating user/assistant" shape that
    // `validate_turn_invariants` enforces, and the renderer stops
    // emitting one "assistant:" header per agentic sub-stream.
    tracing::debug!(
        target: "jfc::session",
        session_id = session_id_str,
        runtime_messages = messages.len(),
        coalesced_messages = coalesced.len(),
        "session save: filtering runtime placeholders and coalescing sub-stream message splits"
    );
    let serialized = SerializedSession {
        id: session_id_str.to_owned(),
        created_at,
        updated_at: Some(now.to_rfc3339()),
        first_prompt: extract_first_prompt(messages),
        model: stored_model,
        cwd: stored_cwd,
        title,
        messages: coalesced.iter().map(serialize_message).collect(),
    };

    let row = jfc_knowledge::SessionRow {
        id: serialized.id.clone(),
        cwd: serialized.cwd.clone(),
        model: serialized.model.clone(),
        created_at: Some(serialized.created_at.clone()),
        updated_at: serialized.updated_at.clone(),
        first_prompt: serialized.first_prompt.clone(),
        title: serialized.title.clone(),
        message_count: coalesced.len() as i64,
    };
    let db_messages = crate::to_session_messages(&serialized.messages);

    // Autonomous self-critique ("Claude improves Claude"): critique this
    // session's reasoning + outputs (not just tool errors) and fold any
    // improvement proposals into the knowledge store so they shape FUTURE turns
    // via recall. Built from `db_messages` before it moves into the writer; runs
    // detached + best-effort so it never blocks or fails the save, and only
    // touches the DB when there are actual proposals (dedup handles repeats).
    let critique_samples = crate::self_critique_samples(&serialized.id, &db_messages);
    if let Some(cwd) = serialized.cwd.as_deref()
        && !critique_samples.is_empty()
    {
        let project_key = jfc_knowledge::project_key(std::path::Path::new(cwd));
        tokio::spawn(async move {
            let proposals = jfc_learn::self_critique::critique_turns(
                &jfc_learn::self_critique::HeuristicJudge,
                &critique_samples,
            );
            if proposals.is_empty() {
                return;
            }
            let lessons = crate::self_critique_proposals_to_lessons(&proposals);
            let definitions = crate::self_critique_proposals_to_definitions(&proposals);
            if let Ok(store) = jfc_knowledge::KnowledgeStore::open_default().await {
                let inserted = store
                    .ingest_mined(&project_key, &lessons)
                    .await
                    .map(|(inserted, _)| inserted)
                    .unwrap_or(0);
                // Stage prompt/skill/tool/reasoning candidates (Candidate status:
                // visible to the promotion machinery, NOT applied to the live
                // prompt until proven).
                let mut staged = 0usize;
                for def in &definitions {
                    if store.upsert_definition(def).await.is_ok() {
                        staged += 1;
                    }
                }
                // Track every suggestion on the queryable self-improvement
                // backlog (status proposed → proven → applied; recurrence bumps
                // on re-proposal as evidence accrues).
                for item in &crate::self_critique_proposals_to_backlog(&proposals) {
                    let _ = store.upsert_backlog_item(item).await;
                }
                // Evidence-gated promotion: graduate well-recurring candidates to
                // ACTIVE so they actually take effect in the prompt (self-heals
                // the per-save status reset; conservative threshold + assembler
                // safety guards).
                let promoted = crate::promote_evidenced_self_critique(
                    &store,
                    crate::SELF_CRITIQUE_PROMOTE_MIN_RECURRENCE,
                )
                .await;
                tracing::info!(
                    target: "jfc::learn::self_critique",
                    proposals = proposals.len(),
                    lessons = inserted,
                    definitions_staged = staged,
                    promoted_active = promoted,
                    "autonomous self-critique: folded lessons + staged candidates + backlog + promoted"
                );
            }
        });
    }

    if tokio::task::spawn_blocking(move || {
        crate::save_session_transcript_to_db(row, db_messages);
    })
    .await
    .is_err()
    {
        warn!(
            target: "jfc::session",
            session_id = session_id_str,
            "session DB write task failed"
        );
    }

    // Cutover #2 (additive, fire-and-forget): mirror the transcript into a typed
    // SessionEntry JSONL sidecar. The DB save above and the DB-only load path are
    // untouched and nothing reads this sidecar yet — a detached task, so it can
    // never block or fail the real save.
    tokio::spawn(super::entry_log::write_sidecar(
        session_id_str.to_owned(),
        messages.to_vec(),
    ));

    info!(
        target: "jfc::session",
        session_id = session_id_str,
        message_count = messages.len(),
        "session saved to DB"
    );

    // Frontend post-save hook (e.g. the TUI persists its highlight-height
    // cache alongside the session). Lives behind a registration so the
    // session layer stays free of render-stack dependencies.
    if let Some(hook) = POST_SAVE_HOOK.get() {
        hook();
    }
}

/// Optional frontend callback fired after every successful session save.
static POST_SAVE_HOOK: std::sync::OnceLock<fn()> = std::sync::OnceLock::new();

/// Register a frontend post-save hook (first registration wins; called on
/// every save). The TUI uses this to persist its highlight-height cache.
pub fn set_post_save_hook(hook: fn()) {
    let _ = POST_SAVE_HOOK.set(hook);
}

fn scrub_loaded_thinking_poison(messages: &mut [ChatMessage]) -> usize {
    let mut removed = 0usize;
    for message in messages {
        let before = message.parts.len();
        message
            .parts
            .retain(|part| !matches!(part, MessagePart::RedactedThinking(_)));
        removed += before.saturating_sub(message.parts.len());
    }
    removed
}

async fn load_session_header_from_db(session_id_str: &str) -> Option<jfc_knowledge::SessionRow> {
    let id = session_id_str.to_owned();
    jfc_knowledge::block_on_knowledge(async move {
        let store = jfc_knowledge::KnowledgeStore::open_default().await?;
        let result = store.get_session(&id).await.ok().flatten();
        Ok::<_, jfc_knowledge::KnowledgeError>(result)
    })
    .unwrap_or_default()
}

struct LoadedSessionFromStore {
    messages: Vec<ChatMessage>,
    model: Option<String>,
}

async fn load_session_from_store<S>(
    store: &S,
    session_id: &SessionId,
) -> Option<LoadedSessionFromStore>
where
    S: jfc_session::SessionStore + ?Sized,
{
    let transcript = jfc_session::SessionStore::load_transcript(store, session_id).await?;
    let mut serialized = Vec::with_capacity(transcript.messages.len());
    for row in transcript.messages {
        let meta = row.meta?;
        match serde_json::from_str::<SerializedMessage>(&meta) {
            Ok(message) => serialized.push(message),
            Err(_) => return None,
        }
    }

    Some(LoadedSessionFromStore {
        messages: serialized.into_iter().map(deserialize_message).collect(),
        model: transcript.model,
    })
}

pub async fn load_session(session_id: &SessionId) -> Option<Vec<ChatMessage>> {
    let session_id_str = session_id.as_str();
    debug!(target: "jfc::session", session_id = session_id_str, "loading session");

    let store = jfc_session::default_session_store();
    let loaded = if let Some(loaded) = load_session_from_store(&store, session_id).await {
        debug!(target: "jfc::session", session_id = session_id_str, count = loaded.messages.len(), "loaded session from facade transcript");
        loaded
    } else {
        return None;
    };
    let message_count = loaded.messages.len();
    let messages = loaded.messages;
    // Record any pre-existing invariant violation BEFORE callers run
    // their own sanitizers. The plan-continuation phantom-assistant
    // bug only surfaced after the renderer composed two layers of
    // truth — the validator gives us a single tracing line that says
    // "this session arrived broken from disk."
    if let Err(err) = validate_turn_invariants(&messages) {
        warn!(
            target: "jfc::session::invariants",
            session_id = session_id_str,
            error = %err,
            message_count,
            "load_session: persisted transcript violates turn invariants — repairing via coalesce"
        );
        // Repair: strip stale empty assistant placeholders and coalesce
        // consecutive same-role messages that were persisted before the
        // coalesce-on-save fix. This prevents the resume path from reviving
        // placeholder-only turns after a watchdog cancel or process exit.
    }
    let mut messages = repair_loaded_messages(messages);
    let scrubbed_thinking_blocks = scrub_loaded_thinking_poison(&mut messages);
    if scrubbed_thinking_blocks > 0 {
        warn!(
            target: "jfc::session::repair",
            session_id = session_id_str,
            scrubbed_thinking_blocks,
            "load_session: stripped persisted redacted-thinking blocks before resume"
        );
        messages = repair_loaded_messages(messages);
    }
    if let Err(err) = validate_turn_invariants(&messages) {
        warn!(
            target: "jfc::session::invariants",
            session_id = session_id_str,
            error = %err,
            message_count = messages.len(),
            "load_session: transcript still violates turn invariants after repair"
        );
    }
    debug!(target: "jfc::session", session_id = session_id_str, message_count, "session loaded");
    Some(messages)
}

/// Load session messages AND the model that was active. Used by `/continue`
/// to restore the model selection.
pub async fn load_session_with_model(
    session_id: &SessionId,
) -> Option<(Vec<ChatMessage>, Option<String>)> {
    let store = jfc_session::default_session_store();
    if let Some(loaded) = load_session_from_store(&store, session_id).await {
        let model = loaded.model;
        let mut messages = repair_loaded_messages(loaded.messages);
        let _ = scrub_loaded_thinking_poison(&mut messages);
        return Some((messages, model));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{MessagePart, Role};
    use async_trait::async_trait;

    struct FacadeLoadStore {
        messages: Vec<jfc_session::StoredSessionMessage>,
        model: Option<String>,
    }

    #[async_trait]
    impl jfc_session::SessionStore for FacadeLoadStore {
        async fn save_transcript(&self, _request: jfc_session::SaveTranscriptRequest<'_>) {}

        async fn load_transcript(
            &self,
            _session_id: &SessionId,
        ) -> Option<jfc_session::SessionTranscript> {
            Some(jfc_session::SessionTranscript {
                messages: self.messages.clone(),
                model: self.model.clone(),
            })
        }

        async fn set_title(&self, _session_id: &SessionId, _title: &str) {}

        async fn list_sessions(
            &self,
            _request: jfc_session::ListSessionsRequest<'_>,
        ) -> Vec<jfc_session::SessionMetadata> {
            Vec::new()
        }

        fn search_sessions(
            &self,
            _request: jfc_session::SearchSessionsRequest<'_>,
        ) -> Vec<jfc_session::SessionHit> {
            Vec::new()
        }

        async fn request_autosave(
            &self,
            _request: jfc_session::AutosaveRequest<'_>,
        ) -> jfc_session::AutosaveOutcome {
            jfc_session::AutosaveOutcome::Saved
        }
    }

    #[test]
    fn scrub_loaded_thinking_poison_removes_redacted_blocks_regression() {
        let mut messages = vec![
            ChatMessage::user("prompt".into()),
            ChatMessage::assistant_parts(vec![
                MessagePart::RedactedThinking("opaque".into()),
                MessagePart::Text("answer".into()),
            ]),
        ];

        let removed = scrub_loaded_thinking_poison(&mut messages);

        assert_eq!(removed, 1);
        assert_eq!(messages[1].role, Role::Assistant);
        assert!(
            messages[1]
                .parts
                .iter()
                .all(|part| !matches!(part, MessagePart::RedactedThinking(_)))
        );
        assert!(
            messages[1]
                .parts
                .iter()
                .any(|part| matches!(part, MessagePart::Text(text) if text == "answer"))
        );
    }

    #[tokio::test]
    async fn load_session_uses_session_store_facade_unit_normal() {
        let serialized = SerializedMessage {
            role: "user".to_owned(),
            agent_name: None,
            model_name: None,
            cost_tier: None,
            elapsed: None,
            usage: None,
            created_at: 0,
            parts: vec![super::super::serialization::SerializedPart::Text {
                content: "from facade".to_owned(),
            }],
        };
        let rows = crate::to_session_messages(&[serialized]);
        let store = FacadeLoadStore {
            messages: rows,
            model: Some("facade-model".to_owned()),
        };
        let id = SessionId::new("ses_20260628_facade_unit");

        let loaded = load_session_from_store(&store, &id)
            .await
            .expect("facade store load succeeds");

        assert_eq!(loaded.model.as_deref(), Some("facade-model"));
        assert_eq!(loaded.messages.len(), 1);
        assert_eq!(loaded.messages[0].parts[0].text_only(), "from facade");
    }
}

/// Set the user-defined title on a session (`/rename` slash). Returns silently
/// on DB failures because title is cosmetic and should not block chat.
pub async fn set_session_title(session_id: &SessionId, title: &str) {
    let session_id_str = session_id.as_str();
    debug!(target: "jfc::session", session_id = session_id_str, "setting session title");
    let db_session_id = session_id_str.to_owned();
    let db_title = title.to_owned();
    let db_updated = chrono::Utc::now().to_rfc3339();
    match jfc_knowledge::block_on_knowledge(async move {
        let store = jfc_knowledge::KnowledgeStore::open_default().await?;
        if let Some(mut row) = store.get_session(&db_session_id).await? {
            row.title = Some(db_title);
            row.updated_at = Some(db_updated);
            store.upsert_session(&row).await?;
        }
        Ok::<(), jfc_knowledge::KnowledgeError>(())
    }) {
        Ok(()) => {}
        Err(err) => warn!(
            target: "jfc::session",
            session_id = session_id_str,
            error = %err,
            "session title DB update failed"
        ),
    }

    info!(target: "jfc::session", session_id = session_id_str, "session title updated in DB");
}

#[cfg(test)]
mod disk_io_tests {
    use super::super::deserialize::{
        deserialize_part, deserialize_tool_input, deserialize_tool_input_for_kind,
    };
    use super::super::serialization::{SerializedPart, SerializedToolInput, SerializedToolOutput};
    use super::super::serialize::serialize_tool_input;
    use super::*;
    use crate::ids::SessionId;
    use crate::types::{
        ChatMessage, TaskInput, ToolCall, ToolInput, ToolKind, ToolOutput, ToolStatus,
    };
    use jfc_session::{
        list_sessions, list_sessions_filtered, load_session_metadata, most_recent_session,
        most_recent_session_for_cwd,
    };
    use serial_test::serial;
    use std::sync::Mutex;
    use tempfile::TempDir;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    /// RAII guard that points test session state at temp storage for the
    /// lifetime of one test. Restores previous values on drop so a later
    /// test in the same process doesn't see a dangling override.
    struct TempConfigHome {
        _dir: TempDir,
        prior_config: Option<String>,
        prior_db: Option<String>,
        _guard: std::sync::MutexGuard<'static, ()>,
    }

    impl TempConfigHome {
        fn new() -> Self {
            // Poison-tolerant lock: a panic in one test shouldn't take
            // out every subsequent disk-I/O test.
            let guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
            let dir = TempDir::new().expect("tempdir");
            let prior_config = std::env::var("XDG_CONFIG_HOME").ok();
            let prior_db = std::env::var("JFC_KNOWLEDGE_DB").ok();
            let db_path = dir.path().join("knowledge.db");
            // Safety: env mutation is serialized through ENV_LOCK.
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

    impl Drop for TempConfigHome {
        fn drop(&mut self) {
            // Safety: env mutation is serialized through the held guard.
            unsafe {
                match self.prior_config.take() {
                    Some(prev) => std::env::set_var("XDG_CONFIG_HOME", prev),
                    None => std::env::remove_var("XDG_CONFIG_HOME"),
                }
                match self.prior_db.take() {
                    Some(prev) => std::env::set_var("JFC_KNOWLEDGE_DB", prev),
                    None => std::env::remove_var("JFC_KNOWLEDGE_DB"),
                }
            }
        }
    }

    // Normal: round-trip a session through save/load with a few common
    // message variants. Verifies DB persistence reconstructs the messages
    // with the same shape.
    #[serial]
    #[tokio::test]
    async fn save_load_roundtrip_normal() {
        let _g = TempConfigHome::new();
        let messages = vec![
            ChatMessage::user("first user prompt".into()),
            ChatMessage::assistant("first reply".into()),
        ];
        let id = SessionId::new("ses_20260506_120000");
        save_session(&id, &messages, Some("/tmp/test"), Some("test-model")).await;

        let loaded = load_session(&id).await.expect("loadable");
        assert_eq!(loaded.len(), 2);
        assert!(loaded[0].role_is_user());
    }

    // Normal: load_session_with_model returns the persisted model id.
    #[serial]
    #[tokio::test]
    async fn load_session_with_model_normal() {
        let _g = TempConfigHome::new();
        let messages = vec![ChatMessage::user("hi".into())];
        let id = SessionId::new("ses_20260506_120100");
        save_session(&id, &messages, Some("/tmp/proj"), Some("opus-4-7")).await;
        let (loaded, model) = load_session_with_model(&id).await.expect("loadable");
        assert_eq!(loaded.len(), 1);
        assert_eq!(model.as_deref(), Some("opus-4-7"));
    }

    // Robust: load_session for a non-existent id returns None instead of
    // panicking.
    #[serial]
    #[tokio::test]
    async fn load_session_missing_returns_none_robust() {
        let _g = TempConfigHome::new();
        let missing = SessionId::new("ses_does_not_exist");
        assert!(load_session(&missing).await.is_none());
        assert!(load_session_with_model(&missing).await.is_none());
        assert!(load_session_metadata(&missing).await.is_none());
    }

    // Normal: load_session_metadata reports the same first_prompt and
    // message_count we saved.
    #[serial]
    #[tokio::test]
    async fn load_session_metadata_picks_up_first_prompt_normal() {
        let _g = TempConfigHome::new();
        let messages = vec![
            ChatMessage::user("Refactor the renderer".into()),
            ChatMessage::assistant("Plan: …".into()),
        ];
        let id = SessionId::new("ses_20260506_120200");
        save_session(&id, &messages, Some("/tmp/proj"), None).await;
        let meta = load_session_metadata(&id).await.expect("metadata loads");
        assert_eq!(meta.id, id);
        assert_eq!(meta.first_prompt.as_deref(), Some("Refactor the renderer"));
        assert_eq!(meta.message_count, 2);
        assert_eq!(meta.cwd.as_deref(), Some("/tmp/proj"));
    }

    // Robust: stray legacy JSON is ignored by DB-only metadata reads.
    #[serial]
    #[tokio::test]
    async fn load_session_metadata_ignores_legacy_json_robust() {
        let _g = TempConfigHome::new();
        let dir = jfc_session::sessions_dir();
        std::fs::create_dir_all(&dir).expect("dir");
        let path = dir.join("ses_json_only.json");
        std::fs::write(
            &path,
            r#"{"id":"ses_json_only","created_at":"2026-01-01T00:00:00Z","messages":[]}"#,
        )
        .expect("write legacy json");
        assert!(
            load_session_metadata(&SessionId::new("ses_json_only"))
                .await
                .is_none()
        );
    }

    #[serial]
    #[tokio::test]
    async fn list_sessions_returns_all_sorted_newest_first_normal() {
        let _g = TempConfigHome::new();
        let m = vec![ChatMessage::user("hi".into())];
        save_session(&SessionId::new("ses_20260101_000000"), &m, None, None).await;
        save_session(&SessionId::new("ses_20260601_000000"), &m, None, None).await;
        save_session(&SessionId::new("ses_20260301_000000"), &m, None, None).await;
        let ids = list_sessions().await;
        assert_eq!(
            ids,
            vec![
                SessionId::new("ses_20260301_000000"),
                SessionId::new("ses_20260601_000000"),
                SessionId::new("ses_20260101_000000"),
            ],
        );
    }

    // Robust: list_sessions with an empty DB returns an empty vec rather than
    // panicking.
    #[serial]
    #[tokio::test]
    async fn list_sessions_missing_dir_is_empty_robust() {
        let _g = TempConfigHome::new();
        // No save_session calls — directory doesn't even exist yet.
        assert!(list_sessions().await.is_empty());
    }

    // Normal: list_sessions_filtered with a cwd filter returns only that
    // project's DB sessions.
    #[serial]
    #[tokio::test]
    async fn list_sessions_filtered_includes_matching_and_legacy_normal() {
        let _g = TempConfigHome::new();
        let m = vec![ChatMessage::user("hi".into())];
        save_session(
            &SessionId::new("ses_20260101_000000"),
            &m,
            Some("/projA"),
            None,
        )
        .await;
        save_session(
            &SessionId::new("ses_20260201_000000"),
            &m,
            Some("/projB"),
            None,
        )
        .await;
        save_session(
            &SessionId::new("ses_20260301_000000"),
            &m,
            Some("/projA"),
            None,
        )
        .await;

        let only_a = list_sessions_filtered(Some("/projA")).await;
        let ids: Vec<&str> = only_a.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(ids, vec!["ses_20260301_000000", "ses_20260101_000000"]);

        // No filter (None) returns all sessions sorted newest-first by
        // updated_at.
        let all = list_sessions_filtered(None).await;
        assert_eq!(all.len(), 3);
    }

    // Normal: most_recent_session_for_cwd returns the matching-cwd session
    // with the greatest `updated_at`. Saved in creation order here (each
    // save stamps updated_at=now), so the last-saved /proj session wins and
    // the /other-cwd session is excluded.
    #[serial]
    #[tokio::test]
    async fn most_recent_session_for_cwd_returns_top_normal() {
        let _g = TempConfigHome::new();
        let m = vec![ChatMessage::user("hi".into())];
        save_session(
            &SessionId::new("ses_20260101_000000"),
            &m,
            Some("/proj"),
            None,
        )
        .await;
        save_session(
            &SessionId::new("ses_20260201_000000"),
            &m,
            Some("/other"),
            None,
        )
        .await;
        save_session(
            &SessionId::new("ses_20260301_000000"),
            &m,
            Some("/proj"),
            None,
        )
        .await;
        let top = most_recent_session_for_cwd(Some("/proj")).await;
        assert_eq!(
            top.as_ref().map(|s| s.as_str()),
            Some("ses_20260301_000000")
        );
    }

    // Normal — REGRESSION (the "--continue resumed the wrong session" bug):
    // ranking is by `updated_at`, NOT by filename/creation order. Create an
    // OLD-id session, then a NEWER-id session, then re-save (touch) the OLD
    // one. The old session now has the latest updated_at, so --continue must
    // resume IT — even though its filename sorts earlier. Pre-fix, filename
    // order wrongly picked the newer-created-but-untouched session.
    #[serial]
    #[tokio::test]
    async fn most_recent_session_for_cwd_ranks_by_updated_at_regression() {
        let _g = TempConfigHome::new();
        let m = vec![ChatMessage::user("hi".into())];
        let old = SessionId::new("ses_20260101_000000");
        let new = SessionId::new("ses_20260901_000000");
        save_session(&old, &m, Some("/proj"), None).await;
        save_session(&new, &m, Some("/proj"), None).await;
        // Touch the OLDER session last → its updated_at is now the greatest.
        // A coarse-clock filesystem could stamp identical rfc3339 seconds, so
        // nudge to guarantee a strictly-later timestamp.
        tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
        save_session(&old, &m, Some("/proj"), None).await;
        let top = most_recent_session_for_cwd(Some("/proj")).await;
        assert_eq!(
            top.as_ref().map(|s| s.as_str()),
            Some("ses_20260101_000000"),
            "the most-recently-worked-in session must win, not the newest filename"
        );
    }

    // Robust: a cwd with no matching sessions returns None (the caller's
    // global fallback + foreign-cwd warning lives in the event loop). The
    // /other session must not leak through as a match for /proj.
    #[serial]
    #[tokio::test]
    async fn most_recent_session_for_cwd_no_match_returns_none_robust() {
        let _g = TempConfigHome::new();
        let m = vec![ChatMessage::user("hi".into())];
        save_session(
            &SessionId::new("ses_20260101_000000"),
            &m,
            Some("/other"),
            None,
        )
        .await;
        assert!(most_recent_session_for_cwd(Some("/proj")).await.is_none());
    }

    // Robust: most_recent_session (global) returns the newest id regardless
    // of cwd.
    #[serial]
    #[tokio::test]
    async fn most_recent_session_global_robust() {
        let _g = TempConfigHome::new();
        let m = vec![ChatMessage::user("hi".into())];
        save_session(&SessionId::new("ses_20260101_000000"), &m, None, None).await;
        save_session(&SessionId::new("ses_20260601_000000"), &m, None, None).await;
        let top = most_recent_session().await;
        assert_eq!(
            top.as_ref().map(|s| s.as_str()),
            Some("ses_20260601_000000")
        );
    }

    // Normal: set_session_title writes a custom title that overrides
    // first_prompt in display.
    #[serial]
    #[tokio::test]
    async fn set_session_title_persists_and_overrides_first_prompt_normal() {
        let _g = TempConfigHome::new();
        let m = vec![ChatMessage::user("Original prompt".into())];
        let id = SessionId::new("ses_20260506_140000");
        save_session(&id, &m, Some("/tmp"), None).await;
        set_session_title(&id, "My custom title").await;
        let meta = load_session_metadata(&id).await.expect("loaded");
        assert_eq!(meta.title.as_deref(), Some("My custom title"));
        assert_eq!(meta.display_title(), "My custom title");
    }

    // Robust: set_session_title on a non-existent id is a no-op (does not
    // panic, does not create files).
    #[serial]
    #[tokio::test]
    async fn set_session_title_missing_session_is_noop_robust() {
        let _g = TempConfigHome::new();
        // Don't save — target doesn't exist.
        let nope = SessionId::new("ses_nope");
        set_session_title(&nope, "ignored").await;
        assert!(load_session_metadata(&nope).await.is_none());
    }

    // Normal: when re-saving an existing session, the original created_at
    // and cwd are preserved (cwd is pinned at first save).
    #[serial]
    #[tokio::test]
    async fn save_session_preserves_created_at_and_cwd_normal() {
        let _g = TempConfigHome::new();
        let m = vec![ChatMessage::user("first".into())];
        let id = SessionId::new("ses_20260506_141500");
        save_session(&id, &m, Some("/orig"), None).await;
        let meta1 = load_session_metadata(&id).await.expect("first save");
        let created_at = meta1.created_at.clone();

        // Re-save with a different cwd — should NOT migrate.
        let m2 = vec![
            ChatMessage::user("first".into()),
            ChatMessage::assistant("reply".into()),
        ];
        save_session(&id, &m2, Some("/elsewhere"), None).await;
        let meta2 = load_session_metadata(&id).await.expect("second save");
        assert_eq!(meta2.created_at, created_at);
        assert_eq!(meta2.cwd.as_deref(), Some("/orig"));
        assert_eq!(meta2.message_count, 2);
    }

    // Normal: round-trip a tool message with full input + output content.
    // Exercises the serialize_part / deserialize_part / serialize_tool_input
    // / deserialize_tool_input paths for a non-trivial tool variant.
    #[serial]
    #[tokio::test]
    async fn save_load_with_tool_message_round_trips_normal() {
        let _g = TempConfigHome::new();
        let tool = ToolCall {
            id: "tool-1".into(),
            kind: ToolKind::Bash,
            status: ToolStatus::Completed,
            input: ToolInput::Bash {
                command: "echo hi".into(),
                timeout: Some(30_000),
                workdir: Some("/tmp".into()),
                run_in_background: None,
                suppress_output: None,
            },
            output: ToolOutput::Command {
                stdout: "hi\n".into(),
                stderr: String::new(),
                exit_code: Some(0),
            },
            display: crate::types::ToolDisplayState::Collapsed,
            elapsed_ms: Some(123),
            started_at: None,
            thought_signature: None,
        };
        let messages = vec![
            ChatMessage::user("run a command".into()),
            ChatMessage::assistant_parts(vec![crate::types::MessagePart::tool_boxed(Box::new(
                tool,
            ))]),
        ];
        let id = SessionId::new("ses_20260506_142000");
        save_session(&id, &messages, Some("/tmp"), Some("opus")).await;
        let loaded = load_session(&id).await.expect("loaded");
        assert_eq!(loaded.len(), 2);
        let tool_part = loaded[1]
            .parts
            .iter()
            .find(|p| matches!(p, crate::types::MessagePart::Tool(_)))
            .expect("tool part");
        match tool_part {
            crate::types::MessagePart::Tool(tc) => {
                assert_eq!(tc.kind, ToolKind::Bash);
                match &tc.input {
                    ToolInput::Bash {
                        command,
                        timeout,
                        workdir,
                        ..
                    } => {
                        assert_eq!(command, "echo hi");
                        assert_eq!(*timeout, Some(30_000));
                        assert_eq!(workdir.as_deref(), Some("/tmp"));
                    }
                    other => panic!("expected Bash input, got {other:?}"),
                }
                match &tc.output {
                    ToolOutput::Command {
                        stdout, exit_code, ..
                    } => {
                        assert_eq!(stdout, "hi\n");
                        assert_eq!(*exit_code, Some(0));
                    }
                    _ => panic!("expected Command output"),
                }
                // Collapsed survives, expanded/pinned do not (per design).
                assert!(tc.display.is_collapsed());
                assert!(!tc.display.is_expanded());
                assert!(!tc.display.is_pinned());
            }
            _ => unreachable!(),
        }
    }

    // Robust: deserialize_tool_status maps unknown statuses to Complete
    // (graceful fallback when an old/foreign status string lands).
    #[test]
    fn deserialize_tool_status_unknown_falls_back_robust() {
        // The function is private, but we can exercise it through
        // deserializing a SerializedPart::Tool with an unknown status.
        let part = SerializedPart::Tool {
            tool: Box::new(crate::session::serialization::SerializedToolPart {
                id: "x".into(),
                kind: "bash".into(),
                status: "exotic".into(),
                is_collapsed: false,
                input: None,
                output: None,
                thought_signature: None,
            }),
        };
        let mp = deserialize_part(part);
        match mp {
            crate::types::MessagePart::Tool(tc) => {
                assert_eq!(tc.status, ToolStatus::Completed);
                // Default reconstructed Bash stub — empty command.
                assert!(matches!(tc.input, ToolInput::Bash { .. }));
                assert!(matches!(tc.output, ToolOutput::Empty));
            }
            _ => panic!("expected Tool"),
        }
    }

    // Robust: deserialize_task_lifecycle maps unknown variants to Pending.
    #[test]
    fn deserialize_task_lifecycle_unknown_falls_back_robust() {
        let part = SerializedPart::TaskStatus {
            task_id: "t1".into(),
            description: "x".into(),
            status: "wat".into(),
            summary: None,
            error: None,
            elapsed_ms: None,
        };
        let mp = deserialize_part(part);
        match mp {
            crate::types::MessagePart::TaskStatus(ts) => {
                assert_eq!(ts.status, crate::types::TaskLifecycle::Pending);
            }
            _ => panic!("expected TaskStatus"),
        }
    }

    // Normal: SerializedToolOutput's custom deserializer accepts a plain
    // string (legacy v0 format) and produces a Text variant.
    #[test]
    fn serialized_tool_output_accepts_legacy_string_normal() {
        let parsed: SerializedToolOutput =
            serde_json::from_str(r#""legacy plaintext output""#).expect("ok");
        assert!(matches!(parsed, SerializedToolOutput::Text { .. }));
    }

    // Robust: a null in the output slot deserializes to Empty (not error).
    #[test]
    fn serialized_tool_output_null_to_empty_robust() {
        let parsed: SerializedToolOutput = serde_json::from_str("null").expect("ok");
        assert!(matches!(parsed, SerializedToolOutput::Empty));
    }

    #[test]
    fn generic_tool_input_json_rehydrates_by_kind_normal() {
        let input = deserialize_tool_input_for_kind(
            "WebSearch",
            SerializedToolInput::Generic {
                summary: serde_json::json!({
                    "query": "streaming parser",
                    "max_results": 3,
                })
                .to_string(),
            },
        );
        match input {
            ToolInput::WebSearch { query, max_results } => {
                assert_eq!(query, "streaming parser");
                assert_eq!(max_results, Some(3));
            }
            other => panic!("expected WebSearch, got {}", other.summary()),
        }
    }

    #[test]
    fn generic_tool_input_legacy_multi_edit_uses_valid_shape_robust() {
        let input = deserialize_tool_input_for_kind(
            "MultiEdit",
            SerializedToolInput::Generic {
                summary: "MultiEdit: /tmp/file.rs (2 edits)".into(),
            },
        );
        match input {
            ToolInput::MultiEdit { file_path, edits } => {
                assert_eq!(file_path, "/tmp/file.rs");
                assert_eq!(edits, serde_json::json!([]));
            }
            other => panic!("expected MultiEdit, got {}", other.summary()),
        }
    }

    #[test]
    fn serialized_task_input_preserves_teammate_fields_normal() {
        let input = ToolInput::Task(TaskInput {
            description: "review auth".into(),
            prompt: "review auth carefully".into(),
            subagent_type: Some("reviewer".into()),
            category: Some("code".into()),
            run_in_background: true,
            model: Some("anthropic/claude-sonnet-4-7".into()),
            launcher: Some("variant-agent".into()),
            effort: None,
            name: Some("alice".into()),
            team_name: Some("core".into()),
            mode: Some("plan".into()),
            isolation: Some("worktree".into()),
            parent_task_id: Some("t20".into()),
            schema: None,
            allowed_tools: Vec::new(),
            disallowed_tools: Vec::new(),
            cwd: None,
        });

        let encoded = serialize_tool_input(&input);
        assert!(matches!(
            encoded,
            SerializedToolInput::Task {
                ref name,
                ref team_name,
                ref launcher,
                ref mode,
                ref isolation,
                ..
            } if name.as_deref() == Some("alice")
                && team_name.as_deref() == Some("core")
                && launcher.as_deref() == Some("variant-agent")
                && mode.as_deref() == Some("plan")
                && isolation.as_deref() == Some("worktree")
        ));

        let decoded = deserialize_tool_input(encoded);
        match decoded {
            ToolInput::Task(task) => {
                assert_eq!(task.name.as_deref(), Some("alice"));
                assert_eq!(task.team_name.as_deref(), Some("core"));
                assert_eq!(task.mode.as_deref(), Some("plan"));
                assert_eq!(task.isolation.as_deref(), Some("worktree"));
                assert_eq!(task.parent_task_id.as_deref(), Some("t20"));
            }
            other => panic!("expected Task, got {}", other.summary()),
        }
    }

    #[test]
    fn serialized_task_metadata_preserves_extended_fields_normal() {
        let create = ToolInput::TaskCreate {
            subject: "map parser".into(),
            description: "write the parser".into(),
            active_form: Some("parsing".into()),
            blocked_by: vec!["t1".into()],
            acceptance_criteria: Some("round-trip fixtures".into()),
            verification_command: Some("cargo test parser".into()),
            risk: Some("medium".into()),
            parent_id: Some("t0".into()),
            kind: Some("implementation".into()),
            tags: vec![],
            priority: None,
            effort: None,
            model: None,
        };
        let decoded = deserialize_tool_input(serialize_tool_input(&create));
        match decoded {
            ToolInput::TaskCreate {
                acceptance_criteria,
                verification_command,
                risk,
                parent_id,
                kind,
                ..
            } => {
                assert_eq!(acceptance_criteria.as_deref(), Some("round-trip fixtures"));
                assert_eq!(verification_command.as_deref(), Some("cargo test parser"));
                assert_eq!(risk.as_deref(), Some("medium"));
                assert_eq!(parent_id.as_deref(), Some("t0"));
                assert_eq!(kind.as_deref(), Some("implementation"));
            }
            other => panic!("expected TaskCreate, got {}", other.summary()),
        }
    }

    #[test]
    fn previously_generic_tool_inputs_serialize_as_typed_variants_normal() {
        let samples = vec![
            ToolInput::WebSearch {
                query: "stream parser".into(),
                max_results: Some(3),
            },
            ToolInput::WebFetch {
                url: "https://example.invalid".into(),
                prompt: Some("summarize".into()),
            },
            ToolInput::AskUserQuestion {
                questions: serde_json::json!([{
                    "question": "choose one",
                    "options": [{"label": "a"}, {"label": "b"}],
                    "multiSelect": false,
                }]),
            },
            ToolInput::Mcp {
                name: "mcp__fs__read".into(),
                arguments: serde_json::json!({"path": "Cargo.toml"}),
            },
            ToolInput::ScratchpadWrite {
                key: "note".into(),
                value: "body".into(),
            },
        ];

        for input in samples {
            assert!(
                !matches!(
                    serialize_tool_input(&input),
                    SerializedToolInput::Generic { .. }
                ),
                "{} should not fall back to Generic session input",
                input.summary()
            );
        }
    }

    #[test]
    fn generic_tool_input_legacy_ask_user_question_rehydrates_robust() {
        let input = deserialize_tool_input_for_kind(
            "AskUserQuestion",
            SerializedToolInput::Generic {
                summary: "AskUserQuestion: Pick a target: prod or staging?".into(),
            },
        );

        match input {
            ToolInput::AskUserQuestion { questions } => {
                let first = &questions.as_array().expect("array")[0];
                assert_eq!(
                    first.get("question").and_then(|v| v.as_str()),
                    Some("Pick a target: prod or staging?")
                );
                assert_eq!(
                    first.get("multiSelect").and_then(|v| v.as_bool()),
                    Some(false)
                );
            }
            other => panic!("expected AskUserQuestion, got {}", other.summary()),
        }
    }
}

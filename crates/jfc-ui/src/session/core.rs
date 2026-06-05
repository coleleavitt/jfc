//! Public async API for saving and loading session files.
//!
//! Coordinates disk I/O (atomic writes, JSON serialization) using helpers
//! from `compaction` (message filtering) and `serialization` (data conversion).

use tracing::{debug, info, warn};

use crate::ids::SessionId;
use crate::types::{ChatMessage, validate_turn_invariants};

use jfc_session::sessions_dir;

use super::compaction::{
    extract_first_prompt, persistent_session_messages, repair_loaded_messages,
};
use super::deserialize::deserialize_message;
use super::serialization::SerializedSession;
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
    let dir = sessions_dir();
    if tokio::fs::create_dir_all(&dir).await.is_err() {
        warn!(target: "jfc::session", "failed to create sessions directory");
        return;
    }

    let now = chrono::Utc::now();
    let path = dir.join(format!("{session_id_str}.json"));

    // Try to load existing session to preserve created_at + cwd + title
    // (so resaving doesn't reset them on every turn). cwd is pinned at
    // first save; subsequent saves don't migrate the session even if the
    // user `cd`s elsewhere — that would conflate two projects' work into
    // one session, and would also defeat the cwd-mismatch warning on
    // resume (codex-rs `tui/src/session_resume.rs:99-111`).
    let prior = tokio::fs::read_to_string(&path)
        .await
        .ok()
        .and_then(|content| serde_json::from_str::<SerializedSession>(&content).ok());
    let created_at = prior
        .as_ref()
        .map(|s| s.created_at.clone())
        .unwrap_or_else(|| now.to_rfc3339());
    // Precedence: prior session's cwd (immutable for session lifetime) →
    // explicit `cwd` arg from caller → current_dir() fallback.
    let stored_cwd = prior
        .as_ref()
        .and_then(|s| s.cwd.clone())
        .or_else(|| cwd.map(str::to_owned))
        .or_else(|| {
            std::env::current_dir()
                .ok()
                .map(|p| p.display().to_string())
        });
    let title = prior.as_ref().and_then(|s| s.title.clone());
    let stored_model = model
        .map(str::to_owned)
        .or_else(|| prior.as_ref().and_then(|s| s.model.clone()));

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

    if let Ok(json) = serde_json::to_string_pretty(&serialized) {
        // Atomic write: a SIGKILL or power loss between writeFile()
        // chunks would otherwise leave the session JSON truncated
        // (e.g. half a `messages` array with no closing brace), and
        // every subsequent load would fail to deserialize and the
        // user would lose the whole transcript. temp + fsync + rename
        // keeps the old contents in place until the new payload is
        // fully on disk. See crate::atomic_write for the recipe.
        if let Err(e) = crate::atomic_write::write_atomic(&path, json.as_bytes()).await {
            warn!(
                target: "jfc::session",
                session_id = session_id_str,
                error = %e,
                "atomic session write failed — previous on-disk contents preserved"
            );
        } else {
            info!(target: "jfc::session", session_id = session_id_str, message_count = messages.len(), path = %path.display(), "session saved");
        }
    } else {
        warn!(target: "jfc::session", session_id = session_id_str, "failed to serialize session");
    }

    // Persist theme-independent highlight line counts alongside the session so
    // future startups can resolve highlighted tool heights without running
    // syntect for known code blocks.
    let hl_cache_path = std::env::current_dir()
        .unwrap_or_default()
        .join(".jfc/highlight-heights.json");
    jfc_markdown::persist_highlight_line_counts(&hl_cache_path);
}

pub async fn load_session(session_id: &SessionId) -> Option<Vec<ChatMessage>> {
    let session_id_str = session_id.as_str();
    debug!(target: "jfc::session", session_id = session_id_str, "loading session");
    let path = sessions_dir().join(format!("{session_id_str}.json"));
    let content = tokio::fs::read_to_string(&path).await.ok()?;
    let session: SerializedSession = match serde_json::from_str(&content) {
        Ok(s) => s,
        Err(e) => {
            warn!(target: "jfc::session", session_id = session_id_str, error = %e, "failed to parse session file");
            return None;
        }
    };
    let message_count = session.messages.len();
    let messages: Vec<ChatMessage> = session
        .messages
        .into_iter()
        .map(deserialize_message)
        .collect();
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
    let messages = repair_loaded_messages(messages);
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
    let session_id_str = session_id.as_str();
    let path = sessions_dir().join(format!("{session_id_str}.json"));
    let content = tokio::fs::read_to_string(&path).await.ok()?;
    let session: SerializedSession = serde_json::from_str(&content).ok()?;
    let model = session.model.clone();
    let messages: Vec<ChatMessage> = session
        .messages
        .into_iter()
        .map(deserialize_message)
        .collect();
    if let Err(err) = validate_turn_invariants(&messages) {
        warn!(
            target: "jfc::session::invariants",
            session_id = session_id_str,
            error = %err,
            message_count = messages.len(),
            "load_session_with_model: persisted transcript violates turn invariants — repairing"
        );
    }
    let messages = repair_loaded_messages(messages);
    if let Err(err) = validate_turn_invariants(&messages) {
        warn!(
            target: "jfc::session::invariants",
            session_id = session_id_str,
            error = %err,
            message_count = messages.len(),
            "load_session_with_model: transcript still violates turn invariants after repair"
        );
    }
    Some((messages, model))
}

/// Set the user-defined title on a session (`/rename` slash). Returns
/// silently on I/O failures — title is cosmetic, shouldn't block the
/// chat. Mirrors v126's `customTitle` field (cli.js:39786) which sits
/// atop the title precedence chain.
pub async fn set_session_title(session_id: &SessionId, title: &str) {
    let session_id_str = session_id.as_str();
    debug!(target: "jfc::session", session_id = session_id_str, "setting session title");
    let path = sessions_dir().join(format!("{session_id_str}.json"));
    let Ok(content) = tokio::fs::read_to_string(&path).await else {
        warn!(target: "jfc::session", session_id = session_id_str, "cannot read session file for title update");
        return;
    };
    let Ok(mut session) = serde_json::from_str::<SerializedSession>(&content) else {
        warn!(target: "jfc::session", session_id = session_id_str, "cannot parse session file for title update");
        return;
    };
    session.title = Some(title.to_owned());
    if let Ok(json) = serde_json::to_string_pretty(&session) {
        // Atomic write — see save_session() above for the rationale.
        // Title updates are cosmetic but they overwrite the entire
        // session file, so a torn write here loses the whole transcript.
        if let Err(e) = crate::atomic_write::write_atomic(&path, json.as_bytes()).await {
            warn!(
                target: "jfc::session",
                session_id = session_id_str,
                error = %e,
                "atomic title write failed — previous session preserved"
            );
        } else {
            info!(target: "jfc::session", session_id = session_id_str, "session title updated");
        }
    }
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
    use std::sync::Mutex;
    use tempfile::TempDir;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    /// RAII guard that points `XDG_CONFIG_HOME` at a tempdir for the
    /// lifetime of one test. Restores the previous value on drop so a
    /// later test in the same process doesn't see a dangling override.
    struct TempConfigHome {
        _dir: TempDir,
        prior: Option<String>,
        _guard: std::sync::MutexGuard<'static, ()>,
    }

    impl TempConfigHome {
        fn new() -> Self {
            // Poison-tolerant lock: a panic in one test shouldn't take
            // out every subsequent disk-I/O test.
            let guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
            let dir = TempDir::new().expect("tempdir");
            let prior = std::env::var("XDG_CONFIG_HOME").ok();
            // Safety: env mutation is serialized through ENV_LOCK.
            unsafe {
                std::env::set_var("XDG_CONFIG_HOME", dir.path());
            }
            Self {
                _dir: dir,
                prior,
                _guard: guard,
            }
        }
    }

    impl Drop for TempConfigHome {
        fn drop(&mut self) {
            // Safety: env mutation is serialized through the held guard.
            unsafe {
                match self.prior.take() {
                    Some(prev) => std::env::set_var("XDG_CONFIG_HOME", prev),
                    None => std::env::remove_var("XDG_CONFIG_HOME"),
                }
            }
        }
    }

    // Normal: round-trip a session through save/load with a few common
    // message variants. Verifies the file lands under sessions_dir() and
    // load_session reconstructs the messages with the same shape.
    #[tokio::test]
    async fn save_load_roundtrip_normal() {
        let _g = TempConfigHome::new();
        let messages = vec![
            ChatMessage::user("first user prompt".into()),
            ChatMessage::assistant("first reply".into()),
        ];
        let id = SessionId::new("ses_20260506_120000");
        save_session(&id, &messages, Some("/tmp/test"), Some("test-model")).await;
        // The file should exist on disk now.
        let path = sessions_dir().join(format!("{}.json", id.as_str()));
        assert!(path.exists(), "session file written");

        let loaded = load_session(&id).await.expect("loadable");
        assert_eq!(loaded.len(), 2);
        assert!(loaded[0].role_is_user());
    }

    // Normal: load_session_with_model returns the persisted model id.
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

    // Robust: corrupted JSON in a session file makes load_session_metadata
    // return None without aborting (parse errors are logged and swallowed).
    #[tokio::test]
    async fn load_session_metadata_handles_corrupted_robust() {
        let _g = TempConfigHome::new();
        let dir = sessions_dir();
        std::fs::create_dir_all(&dir).expect("dir");
        let path = dir.join("ses_corrupted.json");
        std::fs::write(&path, "{ this is not json").expect("write garbage");
        assert!(
            load_session_metadata(&SessionId::new("ses_corrupted"))
                .await
                .is_none()
        );
    }

    // Normal: list_sessions returns all known ids, newest-first by id sort
    // (which is also chronological for the `ses_YYYYMMDD_HHMMSS` shape).
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
                SessionId::new("ses_20260601_000000"),
                SessionId::new("ses_20260301_000000"),
                SessionId::new("ses_20260101_000000"),
            ],
        );
    }

    // Robust: list_sessions on a non-existent sessions directory returns
    // an empty vec rather than panicking.
    #[tokio::test]
    async fn list_sessions_missing_dir_is_empty_robust() {
        let _g = TempConfigHome::new();
        // No save_session calls — directory doesn't even exist yet.
        assert!(list_sessions().await.is_empty());
    }

    // Normal: list_sessions_filtered with a cwd filter returns only that
    // project's sessions plus any legacy (cwd=None) entries.
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

    // Robust: legacy Generic summaries like `GraphQuery(...): ...` are
    // repaired on load so resumed history replays structured tool_use
    // inputs instead of `{ "input": "GraphQuery(...): ..." }`.
    #[test]
    fn generic_tool_input_legacy_graph_query_rehydrates_robust() {
        let input = deserialize_tool_input_for_kind(
            "GraphQuery",
            SerializedToolInput::Generic {
                summary: r#"GraphQuery(budget=3000): fn("main") | callees"#.into(),
            },
        );
        match input {
            ToolInput::GraphQuery {
                query, max_tokens, ..
            } => {
                assert_eq!(query, r#"fn("main") | callees"#);
                assert_eq!(max_tokens, Some(3000));
            }
            other => panic!("expected GraphQuery, got {}", other.summary()),
        }
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
            effort: None,
            name: Some("alice".into()),
            team_name: Some("core".into()),
            mode: Some("plan".into()),
            isolation: Some("worktree".into()),
            parent_task_id: Some("t20".into()),
            schema: None,
        });

        let encoded = serialize_tool_input(&input);
        assert!(matches!(
            encoded,
            SerializedToolInput::Task {
                ref name,
                ref team_name,
                ref mode,
                ref isolation,
                ..
            } if name.as_deref() == Some("alice")
                && team_name.as_deref() == Some("core")
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
            ToolInput::CodeIndex {
                path: Some("src".into()),
                query: Some("parser".into()),
                kind: Some("function".into()),
                max_entries: Some(10),
            },
            ToolInput::GraphQuery {
                query: "entrypoints".into(),
                max_tokens: Some(4000),
                include_handles: Some(true),
                format: None,
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

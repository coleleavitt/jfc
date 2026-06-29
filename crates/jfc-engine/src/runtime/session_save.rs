//! Debounced session autosave.
//!
//! `save_session` requires cloning the ENTIRE `state.messages` vec to hand
//! to the async disk-write task. On long sessions (1000+ messages, tens of
//! MB) the per-tool-batch save sites turned that into a multi-MB deep clone
//! many times per minute — observed live as sustained allocator churn and a
//! major RSS contributor. This module centralizes the policy:
//!
//!   * `request_save` — debounced. If a save happened less than
//!     `MIN_SAVE_INTERVAL` ago, set a pending flag instead of cloning;
//!     the trailing save fires from `flush_pending_save` (driven by the
//!     frontend's ~1s housekeeping tick) so the newest state still lands
//!     on disk shortly after the burst ends.
//!   * `force_save` — immediate. Turn boundaries (end_turn) and explicit
//!     user actions always persist, debounce never drops them.
//!   * `flush_pending_save` — fires the trailing save once the interval
//!     has elapsed.
//!
//! Crash safety is unchanged in practice: the worst case is losing the last
//! `MIN_SAVE_INTERVAL` of *mid-turn* tool results; every completed turn is
//! still saved synchronously-on-event via `force_save`.

use std::time::{Duration, Instant};

use crate::app::EngineState;
use crate::session;

/// Minimum spacing between full-clone session saves on debounced paths.
pub const MIN_SAVE_INTERVAL: Duration = Duration::from_secs(2);

/// Debounced save: clone + spawn only if the interval has elapsed,
/// otherwise mark a trailing save pending.
pub fn request_save(state: &mut EngineState) {
    let recently_saved = state
        .last_session_save_at
        .is_some_and(|t| t.elapsed() < MIN_SAVE_INTERVAL);
    if recently_saved {
        state.session_save_pending = true;
        return;
    }
    force_save(state);
}

/// Immediate save (turn boundaries, explicit user actions). Clears any
/// pending trailing save since this one supersedes it.
pub fn force_save(state: &mut EngineState) {
    let Some(ref session_id) = state.current_session_id else {
        return;
    };
    state.session_save_pending = false;
    let sid = session_id.clone();
    let msgs = state.messages.clone();
    let cwd = state.cwd.clone();
    let model = state.model.clone();
    let context_reduction_queue = state.context_reduction_queue.clone();
    tokio::spawn(async move {
        let store = session::default_session_store();
        store
            .save_transcript(
                session::SaveTranscriptRequest::new(&sid, &msgs)
                    .with_cwd(Some(cwd.as_str()))
                    .with_model(Some(model.as_str())),
            )
            .await;
        session::save_context_reduction_queue(sid, context_reduction_queue).await;
    });
    state.last_session_save_at = Some(Instant::now());
}

/// Fire the trailing save if one is pending and the interval has elapsed.
/// Called from the frontend's ~1s housekeeping tick. Returns true if a
/// save was dispatched.
pub fn flush_pending_save(state: &mut EngineState) -> bool {
    if !state.session_save_pending {
        return false;
    }
    let ready = state
        .last_session_save_at
        .is_none_or(|t| t.elapsed() >= MIN_SAVE_INTERVAL);
    if !ready {
        return false;
    }
    force_save(state);
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use jfc_context::{ContextDropRange, ContextDropReplayMode, QueuedContextDrop};
    use std::ffi::OsString;
    use std::sync::Arc;
    use std::sync::Mutex;

    use jfc_provider::{EventStream, ModelInfo, Provider, ProviderMessage, StreamOptions};
    use serial_test::serial;
    use tempfile::TempDir;

    struct TestProvider;

    #[async_trait::async_trait]
    impl Provider for TestProvider {
        fn name(&self) -> &str {
            "test"
        }
        fn available_models(&self) -> Vec<ModelInfo> {
            Vec::new()
        }
        async fn stream(
            &self,
            _messages: Vec<ProviderMessage>,
            _options: &StreamOptions,
        ) -> anyhow::Result<EventStream> {
            Ok(Box::pin(futures::stream::empty()))
        }
    }

    impl jfc_provider::seal::Sealed for TestProvider {}

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    struct TempSessionEnv {
        _dir: TempDir,
        prior_config: Option<OsString>,
        prior_db: Option<OsString>,
        _guard: std::sync::MutexGuard<'static, ()>,
    }

    impl TempSessionEnv {
        fn new() -> Self {
            let guard = ENV_LOCK.lock().unwrap_or_else(|err| err.into_inner());
            let dir = TempDir::new().expect("tempdir");
            let prior_config = std::env::var_os("XDG_CONFIG_HOME");
            let prior_db = std::env::var_os("JFC_KNOWLEDGE_DB");
            let db_path = dir.path().join("knowledge.db");
            // SAFETY: Category 2 - process environment mutation can race readers.
            // This guard serializes env mutation in this module and the env-dependent
            // tests using it are marked serial.
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
            // SAFETY: Category 2 - restoration is serialized by the same ENV_LOCK
            // guard acquired in TempSessionEnv::new, so this module cannot restore
            // while another guarded test mutates the process environment.
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

    fn state_with_session() -> EngineState {
        let mut state = EngineState::new(Arc::new(TestProvider), "test-model");
        state.current_session_id = Some(jfc_session::generate_session_id());
        state
    }

    #[tokio::test]
    async fn request_save_debounces_within_interval_normal() {
        let mut state = state_with_session();
        // First request saves immediately.
        request_save(&mut state);
        assert!(state.last_session_save_at.is_some());
        assert!(!state.session_save_pending);
        // Second request inside the window only marks pending — no new
        // save (timestamp unchanged).
        let first_stamp = state.last_session_save_at;
        request_save(&mut state);
        assert!(state.session_save_pending, "burst save must defer");
        assert_eq!(state.last_session_save_at, first_stamp);
    }

    #[tokio::test]
    async fn flush_fires_trailing_save_after_interval_normal() {
        let mut state = state_with_session();
        state.session_save_pending = true;
        // Stale enough timestamp → flush dispatches.
        state.last_session_save_at = Some(Instant::now() - MIN_SAVE_INTERVAL);
        assert!(flush_pending_save(&mut state));
        assert!(!state.session_save_pending);
        // Nothing pending → flush declines.
        assert!(!flush_pending_save(&mut state));
    }

    #[tokio::test]
    async fn flush_waits_out_the_interval_robust() {
        let mut state = state_with_session();
        state.session_save_pending = true;
        state.last_session_save_at = Some(Instant::now());
        assert!(!flush_pending_save(&mut state), "too soon — must wait");
        assert!(state.session_save_pending, "pending flag survives");
    }

    #[tokio::test]
    async fn force_save_always_fires_normal() {
        let mut state = state_with_session();
        state.last_session_save_at = Some(Instant::now());
        state.session_save_pending = true;
        force_save(&mut state);
        assert!(!state.session_save_pending, "force clears pending");
    }

    #[tokio::test]
    async fn no_session_id_is_noop_robust() {
        // EngineState::new mints a session id by default — clear it to
        // model the no-session-persistence path.
        let mut state = EngineState::new(Arc::new(TestProvider), "test-model");
        state.current_session_id = None;
        request_save(&mut state);
        assert!(state.last_session_save_at.is_none());
        assert!(!state.session_save_pending);
    }

    #[serial]
    #[tokio::test]
    async fn flush_pending_save_persists_load_title_model_shape_normal() {
        let _env = TempSessionEnv::new();
        let mut state = state_with_session();
        let session_id = state
            .current_session_id
            .clone()
            .expect("test state has session id");
        state.cwd = "/tmp/jfc-session-save-facade".to_owned();
        state.model = "facade-autosave-model".to_owned().into();
        state.messages = vec![
            crate::types::ChatMessage::user("autosave through facade".into()),
            crate::types::ChatMessage::assistant("loaded through facade".into()),
        ];
        state.session_save_pending = true;
        state.last_session_save_at = Some(Instant::now() - MIN_SAVE_INTERVAL);

        assert!(flush_pending_save(&mut state));

        let mut loaded = None;
        for _ in 0..20 {
            if let Some(result) = crate::session::load_session_with_model(&session_id).await {
                loaded = Some(result);
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        let (messages, model) = loaded.expect("autosave transcript persisted");
        crate::session::set_session_title(&session_id, "Autosave facade title").await;
        let metadata = jfc_session::load_session_metadata(&session_id)
            .await
            .expect("metadata persisted");

        assert_eq!(model.as_deref(), Some("facade-autosave-model"));
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].parts[0].text_only(), "autosave through facade");
        assert_eq!(messages[1].parts[0].text_only(), "loaded through facade");
        assert_eq!(metadata.title.as_deref(), Some("Autosave facade title"));
        assert_eq!(
            metadata.cwd.as_deref(),
            Some("/tmp/jfc-session-save-facade")
        );
    }

    #[serial]
    #[tokio::test]
    async fn force_save_persists_context_reduction_queue_normal() {
        let _env = TempSessionEnv::new();
        let mut state = state_with_session();
        let session_id = state
            .current_session_id
            .clone()
            .expect("test state has session id");
        state
            .context_reduction_queue
            .extend([QueuedContextDrop::new(
                ContextDropRange::new(2, 4).expect("valid range"),
                ContextDropReplayMode::Skeleton,
            )
            .expect("valid queued drop")]);

        force_save(&mut state);

        let mut loaded = jfc_context::ContextReductionQueue::default();
        for _ in 0..20 {
            loaded = crate::session::load_context_reduction_queue(&session_id).await;
            if !loaded.is_empty() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        assert_eq!(loaded.drops().len(), 1);
        assert_eq!(
            loaded.drops()[0].replay_mode(),
            ContextDropReplayMode::Skeleton
        );

        let mut restored = state_with_session();
        crate::runtime::ops::restore_session_context_state(&mut restored, session_id.as_str())
            .await;
        assert_eq!(restored.context_reduction_queue.drops(), loaded.drops());
    }
}

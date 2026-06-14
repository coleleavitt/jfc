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
    tokio::spawn(async move {
        session::save_session(&sid, &msgs, Some(cwd.as_str()), Some(model.as_str())).await;
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
    use std::sync::Arc;

    use jfc_provider::{EventStream, ModelInfo, Provider, ProviderMessage, StreamOptions};

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
}

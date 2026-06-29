use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use jfc_context::ContextReductionQueue;
use serde::{Deserialize, Serialize};

use crate::ids::SessionId;

const STATE_VERSION: u8 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct SerializedContextReductionState {
    version: u8,
    queue: ContextReductionQueue,
}

impl SerializedContextReductionState {
    const fn new(queue: ContextReductionQueue) -> Self {
        Self {
            version: STATE_VERSION,
            queue,
        }
    }
}

pub async fn save_context_reduction_queue(session_id: SessionId, queue: ContextReductionQueue) {
    let path = state_path(&session_id);
    if queue.is_empty() {
        remove_state_file(&path).await;
        return;
    }

    let Some(parent) = path.parent() else {
        return;
    };
    if let Err(error) = tokio::fs::create_dir_all(parent).await {
        tracing::debug!(
            target: "jfc::session::ctx_reduce",
            %error,
            path = %parent.display(),
            "ctx_reduce state directory create failed"
        );
        return;
    }

    let state = SerializedContextReductionState::new(queue);
    let body = match serde_json::to_vec_pretty(&state) {
        Ok(body) => body,
        Err(error) => {
            tracing::debug!(
                target: "jfc::session::ctx_reduce",
                %error,
                "ctx_reduce state serialize failed"
            );
            return;
        }
    };

    if let Err(error) = tokio::fs::write(&path, body).await {
        tracing::debug!(
            target: "jfc::session::ctx_reduce",
            %error,
            path = %path.display(),
            "ctx_reduce state write failed"
        );
    }
}

pub async fn load_context_reduction_queue(session_id: &SessionId) -> ContextReductionQueue {
    let path = state_path(session_id);
    let body = match tokio::fs::read(&path).await {
        Ok(body) => body,
        Err(error) => {
            if error.kind() != ErrorKind::NotFound {
                tracing::debug!(
                    target: "jfc::session::ctx_reduce",
                    %error,
                    path = %path.display(),
                    "ctx_reduce state read failed"
                );
            }
            return ContextReductionQueue::default();
        }
    };

    match serde_json::from_slice::<SerializedContextReductionState>(&body) {
        Ok(state) if state.version == STATE_VERSION => state.queue,
        Ok(state) => {
            tracing::debug!(
                target: "jfc::session::ctx_reduce",
                version = state.version,
                supported = STATE_VERSION,
                "ctx_reduce state version unsupported"
            );
            ContextReductionQueue::default()
        }
        Err(error) => {
            tracing::debug!(
                target: "jfc::session::ctx_reduce",
                %error,
                path = %path.display(),
                "ctx_reduce state parse failed"
            );
            ContextReductionQueue::default()
        }
    }
}

fn state_path(session_id: &SessionId) -> PathBuf {
    jfc_session::sessions_dir().join(format!("{}.ctx-reduce.json", session_id.as_str()))
}

async fn remove_state_file(path: &Path) {
    if let Err(error) = tokio::fs::remove_file(path).await
        && error.kind() != ErrorKind::NotFound
    {
        tracing::debug!(
            target: "jfc::session::ctx_reduce",
            %error,
            path = %path.display(),
            "ctx_reduce state remove failed"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use jfc_context::{ContextDropRange, ContextDropReplayMode, QueuedContextDrop};
    use serial_test::serial;
    use std::ffi::OsString;
    use std::sync::Mutex;
    use tempfile::TempDir;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    struct TempSessionEnv {
        _dir: TempDir,
        prior_config: Option<OsString>,
        _guard: std::sync::MutexGuard<'static, ()>,
    }

    impl TempSessionEnv {
        fn new() -> Self {
            let guard = ENV_LOCK.lock().unwrap_or_else(|err| err.into_inner());
            let dir = TempDir::new().expect("tempdir");
            let prior_config = std::env::var_os("XDG_CONFIG_HOME");
            // SAFETY: Category 2 - process environment mutation can race readers.
            // This test guard serializes all env mutation in this module via ENV_LOCK,
            // and every test that uses it also runs under serial_test.
            unsafe {
                std::env::set_var("XDG_CONFIG_HOME", dir.path());
            }
            Self {
                _dir: dir,
                prior_config,
                _guard: guard,
            }
        }
    }

    impl Drop for TempSessionEnv {
        fn drop(&mut self) {
            // SAFETY: Category 2 - restore is serialized by the same ENV_LOCK guard
            // acquired in TempSessionEnv::new, so this module does not concurrently
            // mutate XDG_CONFIG_HOME while restoring the previous process value.
            unsafe {
                match self.prior_config.take() {
                    Some(value) => std::env::set_var("XDG_CONFIG_HOME", value),
                    None => std::env::remove_var("XDG_CONFIG_HOME"),
                }
            }
        }
    }

    #[serial]
    #[tokio::test]
    async fn context_reduction_queue_round_trips_replay_modes_normal() {
        let _env = TempSessionEnv::new();
        let session_id = SessionId::new("ses_20260628_ctx_reduce_round_trip");
        let queue = ContextReductionQueue::new([
            QueuedContextDrop::new(
                ContextDropRange::new(2, 3).expect("valid range"),
                ContextDropReplayMode::Skeleton,
            )
            .expect("valid skeleton drop"),
            QueuedContextDrop::protected_tail_skip(
                ContextDropRange::new(7, 9).expect("valid range"),
                7,
            )
            .expect("valid protected-tail skip"),
        ]);

        save_context_reduction_queue(session_id.clone(), queue.clone()).await;

        let loaded = load_context_reduction_queue(&session_id).await;
        assert_eq!(loaded.drops(), queue.drops());
    }

    #[serial]
    #[tokio::test]
    async fn empty_context_reduction_queue_removes_stale_state_normal() {
        let _env = TempSessionEnv::new();
        let session_id = SessionId::new("ses_20260628_ctx_reduce_empty");
        let queue = ContextReductionQueue::new([QueuedContextDrop::new(
            ContextDropRange::new(4, 4).expect("valid range"),
            ContextDropReplayMode::Full,
        )
        .expect("valid drop")]);

        save_context_reduction_queue(session_id.clone(), queue).await;
        save_context_reduction_queue(session_id.clone(), ContextReductionQueue::default()).await;

        let loaded = load_context_reduction_queue(&session_id).await;
        assert!(loaded.is_empty());
    }
}

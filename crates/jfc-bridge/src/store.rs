use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;
use tokio::sync::broadcast;
use uuid::Uuid;

use crate::model::{
    BridgeEvent, CreateSessionRequest, DeliveryAckRequest, EventUpload, SessionRecord,
    SessionStatus, WorkerRecord, WorkerStatus, WorkerUpdateRequest,
};
use crate::time::now_ms;

#[derive(Debug, Error)]
pub enum StoreError {
    #[error("session not found: {0}")]
    SessionNotFound(String),
    #[error("active session already exists for environment: {0}")]
    SessionConflict(String),
    #[error("worker epoch mismatch: current={current} request={request}")]
    WorkerEpochMismatch { current: u64, request: u64 },
    #[error("worker identity mismatch: current={current} request={request}")]
    WorkerIdentityMismatch { current: String, request: String },
    #[error("store lock poisoned")]
    Poisoned,
    #[error("failed to persist bridge state")]
    Persistence,
}

pub type StoreResult<T> = Result<T, StoreError>;

pub trait BridgeStore: Send + Sync + 'static {
    fn create_session(&self, req: CreateSessionRequest) -> StoreResult<SessionRecord>;
    fn get_session(&self, session_id: &str) -> StoreResult<Option<SessionRecord>>;
    fn archive_session(&self, session_id: &str) -> StoreResult<SessionRecord>;
    fn upsert_worker(
        &self,
        session_id: &str,
        worker_id: &str,
        req: WorkerUpdateRequest,
    ) -> StoreResult<WorkerRecord>;
    fn get_worker(&self, session_id: &str) -> StoreResult<Option<WorkerRecord>>;
    fn heartbeat(
        &self,
        session_id: &str,
        worker_id: &str,
        worker_epoch: u64,
        status: Option<WorkerStatus>,
    ) -> StoreResult<WorkerRecord>;
    fn append_events(
        &self,
        session_id: &str,
        internal: bool,
        events: Vec<EventUpload>,
    ) -> StoreResult<Vec<BridgeEvent>>;
    fn list_events(
        &self,
        session_id: &str,
        internal: bool,
        after: Option<&str>,
    ) -> StoreResult<Vec<BridgeEvent>>;
    fn ack_delivery(&self, session_id: &str, req: DeliveryAckRequest) -> StoreResult<BridgeEvent>;
    fn subscribe(&self) -> broadcast::Receiver<BridgeEvent>;
}

#[derive(Debug, Clone)]
pub struct MemoryBridgeStore {
    inner: Arc<RwLock<MemoryInner>>,
    events_tx: broadcast::Sender<BridgeEvent>,
    persist_path: Option<PathBuf>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct MemoryInner {
    sessions: HashMap<String, SessionRecord>,
    workers: HashMap<String, WorkerRecord>,
    events: Vec<BridgeEvent>,
}

impl Default for MemoryBridgeStore {
    fn default() -> Self {
        Self::new()
    }
}

impl MemoryBridgeStore {
    pub fn new() -> Self {
        let (events_tx, _) = broadcast::channel(4096);
        Self {
            inner: Arc::new(RwLock::new(MemoryInner::default())),
            events_tx,
            persist_path: None,
        }
    }

    pub fn with_state_file(path: impl AsRef<Path>) -> std::io::Result<Self> {
        let path = path.as_ref().to_path_buf();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let inner = match std::fs::read_to_string(&path) {
            Ok(data) if data.trim().is_empty() => MemoryInner::default(),
            Ok(data) => serde_json::from_str(&data).map_err(std::io::Error::other)?,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => MemoryInner::default(),
            Err(err) => return Err(err),
        };
        let (events_tx, _) = broadcast::channel(4096);
        Ok(Self {
            inner: Arc::new(RwLock::new(inner)),
            events_tx,
            persist_path: Some(path),
        })
    }

    fn read(&self) -> StoreResult<std::sync::RwLockReadGuard<'_, MemoryInner>> {
        self.inner.read().map_err(|_| StoreError::Poisoned)
    }

    fn write(&self) -> StoreResult<std::sync::RwLockWriteGuard<'_, MemoryInner>> {
        self.inner.write().map_err(|_| StoreError::Poisoned)
    }

    fn persist(&self, inner: &MemoryInner) -> StoreResult<()> {
        let Some(path) = self.persist_path.as_ref() else {
            return Ok(());
        };
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|_| StoreError::Persistence)?;
        }
        let tmp = path.with_extension("json.tmp");
        let json = serde_json::to_vec_pretty(inner).map_err(|_| StoreError::Persistence)?;
        std::fs::write(&tmp, json).map_err(|_| StoreError::Persistence)?;
        std::fs::rename(&tmp, path).map_err(|_| StoreError::Persistence)?;
        Ok(())
    }
}

impl BridgeStore for MemoryBridgeStore {
    fn create_session(&self, req: CreateSessionRequest) -> StoreResult<SessionRecord> {
        let _linkscope_create = linkscope::phase("bridge.store.create_session");
        let now = now_ms();
        let environment_id = req.environment_id;
        let title = req.title;
        let tags = req.tags;
        let metadata = req.metadata;
        let mut inner = self.write()?;
        if let Some(environment_id) = environment_id.as_deref() {
            if inner.sessions.values().any(|session| {
                session.environment_id.as_deref() == Some(environment_id)
                    && !matches!(
                        session.status,
                        SessionStatus::Archived | SessionStatus::Terminated
                    )
            }) {
                return Err(StoreError::SessionConflict(environment_id.to_owned()));
            }
        }
        let record = SessionRecord {
            id: format!("ses_{}", Uuid::new_v4().simple()),
            environment_id,
            title,
            status: SessionStatus::Idle,
            tags,
            metadata,
            created_at_ms: now,
            updated_at_ms: now,
            archived_at_ms: None,
        };
        inner.sessions.insert(record.id.clone(), record.clone());
        self.persist(&inner)?;
        linkscope::record_items("bridge.store.session.created", 1);
        Ok(record)
    }

    fn get_session(&self, session_id: &str) -> StoreResult<Option<SessionRecord>> {
        let _linkscope_get = linkscope::phase("bridge.store.get_session");
        let session = self.read()?.sessions.get(session_id).cloned();
        linkscope::record_items("bridge.store.session.get", 1);
        if session.is_none() {
            linkscope::record_items("bridge.store.session.miss", 1);
        }
        Ok(session)
    }

    fn archive_session(&self, session_id: &str) -> StoreResult<SessionRecord> {
        let _linkscope_archive = linkscope::phase("bridge.store.archive_session");
        let mut inner = self.write()?;
        let Some(session) = inner.sessions.get_mut(session_id) else {
            return Err(StoreError::SessionNotFound(session_id.to_owned()));
        };
        let now = now_ms();
        session.status = SessionStatus::Archived;
        session.updated_at_ms = now;
        session.archived_at_ms = Some(now);
        let session = session.clone();
        self.persist(&inner)?;
        linkscope::record_items("bridge.store.session.archived", 1);
        Ok(session)
    }

    fn upsert_worker(
        &self,
        session_id: &str,
        worker_id: &str,
        req: WorkerUpdateRequest,
    ) -> StoreResult<WorkerRecord> {
        let _linkscope_upsert = linkscope::phase("bridge.store.upsert_worker");
        let mut inner = self.write()?;
        if !inner.sessions.contains_key(session_id) {
            return Err(StoreError::SessionNotFound(session_id.to_owned()));
        }
        let now = now_ms();
        let worker_id = worker_id.to_owned();
        let worker_epoch = if req.worker_epoch == 0 {
            inner.workers.get(session_id).map_or(1, |w| w.worker_epoch)
        } else {
            req.worker_epoch
        };
        if let Some(current) = inner.workers.get(session_id) {
            if current.worker_id != worker_id {
                return Err(StoreError::WorkerIdentityMismatch {
                    current: current.worker_id.clone(),
                    request: worker_id,
                });
            }
            if req.worker_epoch != 0 && current.worker_epoch > req.worker_epoch {
                return Err(StoreError::WorkerEpochMismatch {
                    current: current.worker_epoch,
                    request: req.worker_epoch,
                });
            }
        }
        let record = WorkerRecord {
            session_id: session_id.to_owned(),
            worker_id,
            worker_epoch,
            worker_status: req.worker_status,
            external_metadata: req.external_metadata,
            internal_metadata: req.internal_metadata,
            updated_at_ms: now,
            last_heartbeat_at_ms: now,
        };
        inner.workers.insert(session_id.to_owned(), record.clone());
        if let Some(session) = inner.sessions.get_mut(session_id) {
            session.status = match record.worker_status {
                WorkerStatus::Busy => SessionStatus::Running,
                WorkerStatus::Idle => SessionStatus::Idle,
                WorkerStatus::Draining | WorkerStatus::Offline => session.status.clone(),
            };
            session.updated_at_ms = now;
        }
        self.persist(&inner)?;
        linkscope::record_items("bridge.store.worker.upserted", 1);
        Ok(record)
    }

    fn get_worker(&self, session_id: &str) -> StoreResult<Option<WorkerRecord>> {
        let _linkscope_get = linkscope::phase("bridge.store.get_worker");
        let worker = self.read()?.workers.get(session_id).cloned();
        linkscope::record_items("bridge.store.worker.get", 1);
        if worker.is_none() {
            linkscope::record_items("bridge.store.worker.miss", 1);
        }
        Ok(worker)
    }

    fn heartbeat(
        &self,
        session_id: &str,
        worker_id: &str,
        worker_epoch: u64,
        status: Option<WorkerStatus>,
    ) -> StoreResult<WorkerRecord> {
        let _linkscope_heartbeat = linkscope::phase("bridge.store.heartbeat");
        let mut inner = self.write()?;
        let now = now_ms();
        let Some(worker) = inner.workers.get_mut(session_id) else {
            return Err(StoreError::SessionNotFound(session_id.to_owned()));
        };
        if worker.worker_id != worker_id || worker.worker_epoch != worker_epoch {
            return Err(StoreError::WorkerEpochMismatch {
                current: worker.worker_epoch,
                request: worker_epoch,
            });
        }
        if let Some(status) = status {
            worker.worker_status = status;
        }
        worker.updated_at_ms = now;
        worker.last_heartbeat_at_ms = now;
        let worker = worker.clone();
        self.persist(&inner)?;
        linkscope::record_items("bridge.store.worker.heartbeat", 1);
        Ok(worker)
    }

    fn append_events(
        &self,
        session_id: &str,
        internal: bool,
        events: Vec<EventUpload>,
    ) -> StoreResult<Vec<BridgeEvent>> {
        let _linkscope_append = linkscope::phase("bridge.store.append_events");
        let event_count = events.len();
        let mut inner = self.write()?;
        if !inner.sessions.contains_key(session_id) {
            return Err(StoreError::SessionNotFound(session_id.to_owned()));
        }
        let now = now_ms();
        let records = events
            .into_iter()
            .map(|event| BridgeEvent {
                id: event
                    .id
                    .unwrap_or_else(|| format!("evt_{}", Uuid::new_v4().simple())),
                session_id: session_id.to_owned(),
                kind: event.kind,
                payload: event.payload,
                created_at_ms: now,
                internal,
                delivery_status: None,
            })
            .collect::<Vec<_>>();
        for event in &records {
            inner.events.push(event.clone());
            let _ = self.events_tx.send(event.clone());
        }
        self.persist(&inner)?;
        linkscope::record_items(
            "bridge.store.events.appended",
            usize_to_u64_saturating(event_count),
        );
        Ok(records)
    }

    fn list_events(
        &self,
        session_id: &str,
        internal: bool,
        after: Option<&str>,
    ) -> StoreResult<Vec<BridgeEvent>> {
        let _linkscope_list = linkscope::phase("bridge.store.list_events");
        if !self.read()?.sessions.contains_key(session_id) {
            return Err(StoreError::SessionNotFound(session_id.to_owned()));
        }
        let inner = self.read()?;
        let mut seen_after = after.is_none();
        let mut out = Vec::new();
        for event in inner
            .events
            .iter()
            .filter(|event| event.session_id == session_id && event.internal == internal)
        {
            if !seen_after {
                seen_after = after == Some(event.id.as_str());
                continue;
            }
            out.push(event.clone());
        }
        linkscope::record_items(
            "bridge.store.events.listed",
            usize_to_u64_saturating(out.len()),
        );
        Ok(out)
    }

    fn ack_delivery(&self, session_id: &str, req: DeliveryAckRequest) -> StoreResult<BridgeEvent> {
        let _linkscope_ack = linkscope::phase("bridge.store.ack_delivery");
        let mut inner = self.write()?;
        let Some(event) = inner
            .events
            .iter_mut()
            .find(|event| event.session_id == session_id && event.id == req.event_id)
        else {
            return Err(StoreError::SessionNotFound(session_id.to_owned()));
        };
        event.delivery_status = Some(req.status);
        let event = event.clone();
        self.persist(&inner)?;
        linkscope::record_items("bridge.store.delivery.acked", 1);
        Ok(event)
    }

    fn subscribe(&self) -> broadcast::Receiver<BridgeEvent> {
        linkscope::record_items("bridge.store.subscribe", 1);
        self.events_tx.subscribe()
    }
}

fn usize_to_u64_saturating(value: usize) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
}

impl From<Value> for EventUpload {
    fn from(payload: Value) -> Self {
        Self {
            id: None,
            kind: "event".to_owned(),
            payload,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::CreateSessionRequest;

    #[test]
    fn memory_store_roundtrip() {
        let store = MemoryBridgeStore::new();
        let session = store
            .create_session(CreateSessionRequest {
                environment_id: Some("env".to_owned()),
                title: Some("test".to_owned()),
                tags: vec![],
                metadata: Default::default(),
            })
            .unwrap();
        let worker = store
            .upsert_worker(
                &session.id,
                "worker",
                WorkerUpdateRequest {
                    worker_id: None,
                    worker_epoch: 1,
                    worker_status: WorkerStatus::Idle,
                    external_metadata: Value::Null,
                    internal_metadata: Value::Null,
                },
            )
            .unwrap();
        assert_eq!(worker.worker_epoch, 1);
        let events = store
            .append_events(&session.id, false, vec![Value::String("hi".into()).into()])
            .unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(
            store.list_events(&session.id, false, None).unwrap().len(),
            1
        );
    }

    #[test]
    fn upsert_worker_uses_authenticated_worker_id_regression() {
        let store = MemoryBridgeStore::new();
        let session = store
            .create_session(CreateSessionRequest {
                environment_id: None,
                title: None,
                tags: vec![],
                metadata: Default::default(),
            })
            .unwrap();

        let worker = store
            .upsert_worker(
                &session.id,
                "auth-worker",
                WorkerUpdateRequest {
                    worker_id: Some("body-worker".to_owned()),
                    worker_epoch: 1,
                    worker_status: WorkerStatus::Idle,
                    external_metadata: Value::Null,
                    internal_metadata: Value::Null,
                },
            )
            .unwrap();

        assert_eq!(worker.worker_id, "auth-worker");
    }

    #[test]
    fn upsert_worker_rejects_competing_worker_regression() {
        let store = MemoryBridgeStore::new();
        let session = store
            .create_session(CreateSessionRequest {
                environment_id: None,
                title: None,
                tags: vec![],
                metadata: Default::default(),
            })
            .unwrap();
        store
            .upsert_worker(
                &session.id,
                "worker-a",
                WorkerUpdateRequest {
                    worker_id: None,
                    worker_epoch: 1,
                    worker_status: WorkerStatus::Idle,
                    external_metadata: Value::Null,
                    internal_metadata: Value::Null,
                },
            )
            .unwrap();

        let error = store
            .upsert_worker(
                &session.id,
                "worker-b",
                WorkerUpdateRequest {
                    worker_id: None,
                    worker_epoch: 2,
                    worker_status: WorkerStatus::Busy,
                    external_metadata: Value::Null,
                    internal_metadata: Value::Null,
                },
            )
            .unwrap_err();

        assert!(matches!(error, StoreError::WorkerIdentityMismatch { .. }));
    }

    #[test]
    fn create_session_rejects_duplicate_active_environment_regression() {
        let store = MemoryBridgeStore::new();
        store
            .create_session(CreateSessionRequest {
                environment_id: Some("env-1".to_owned()),
                title: None,
                tags: vec![],
                metadata: Default::default(),
            })
            .unwrap();

        let error = store
            .create_session(CreateSessionRequest {
                environment_id: Some("env-1".to_owned()),
                title: Some("duplicate".to_owned()),
                tags: vec![],
                metadata: Default::default(),
            })
            .unwrap_err();

        assert!(matches!(error, StoreError::SessionConflict(env) if env == "env-1"));
    }

    #[test]
    fn file_store_survives_reopen() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("bridge-state.json");
        let session_id = {
            let store = MemoryBridgeStore::with_state_file(&path).unwrap();
            let session = store
                .create_session(CreateSessionRequest {
                    environment_id: None,
                    title: Some("persisted".to_owned()),
                    tags: vec![],
                    metadata: Default::default(),
                })
                .unwrap();
            store
                .append_events(&session.id, false, vec![Value::String("hi".into()).into()])
                .unwrap();
            session.id
        };
        let store = MemoryBridgeStore::with_state_file(&path).unwrap();
        assert_eq!(
            store
                .get_session(&session_id)
                .unwrap()
                .unwrap()
                .title
                .as_deref(),
            Some("persisted")
        );
        assert_eq!(
            store.list_events(&session_id, false, None).unwrap().len(),
            1
        );
    }
}

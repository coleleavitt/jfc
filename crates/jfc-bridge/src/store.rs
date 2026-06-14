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
    #[error("worker epoch mismatch: current={current} request={request}")]
    WorkerEpochMismatch { current: u64, request: u64 },
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
        let now = now_ms();
        let record = SessionRecord {
            id: format!("ses_{}", Uuid::new_v4().simple()),
            environment_id: req.environment_id,
            title: req.title,
            status: SessionStatus::Idle,
            tags: req.tags,
            metadata: req.metadata,
            created_at_ms: now,
            updated_at_ms: now,
            archived_at_ms: None,
        };
        let mut inner = self.write()?;
        inner.sessions.insert(record.id.clone(), record.clone());
        self.persist(&inner)?;
        Ok(record)
    }

    fn get_session(&self, session_id: &str) -> StoreResult<Option<SessionRecord>> {
        Ok(self.read()?.sessions.get(session_id).cloned())
    }

    fn archive_session(&self, session_id: &str) -> StoreResult<SessionRecord> {
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
        Ok(session)
    }

    fn upsert_worker(
        &self,
        session_id: &str,
        worker_id: &str,
        req: WorkerUpdateRequest,
    ) -> StoreResult<WorkerRecord> {
        let mut inner = self.write()?;
        if !inner.sessions.contains_key(session_id) {
            return Err(StoreError::SessionNotFound(session_id.to_owned()));
        }
        let now = now_ms();
        // worker_epoch == 0 means "unspecified": preserve the existing
        // worker's epoch instead of resetting it to 1.
        let worker_epoch = if req.worker_epoch == 0 {
            inner.workers.get(session_id).map_or(1, |w| w.worker_epoch)
        } else {
            req.worker_epoch
        };
        let worker_id = req.worker_id.unwrap_or_else(|| worker_id.to_owned());
        if let Some(current) = inner.workers.get(session_id)
            && req.worker_epoch != 0
            && current.worker_epoch > req.worker_epoch
        {
            return Err(StoreError::WorkerEpochMismatch {
                current: current.worker_epoch,
                request: req.worker_epoch,
            });
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
        Ok(record)
    }

    fn get_worker(&self, session_id: &str) -> StoreResult<Option<WorkerRecord>> {
        Ok(self.read()?.workers.get(session_id).cloned())
    }

    fn heartbeat(
        &self,
        session_id: &str,
        worker_id: &str,
        worker_epoch: u64,
        status: Option<WorkerStatus>,
    ) -> StoreResult<WorkerRecord> {
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
        Ok(worker)
    }

    fn append_events(
        &self,
        session_id: &str,
        internal: bool,
        events: Vec<EventUpload>,
    ) -> StoreResult<Vec<BridgeEvent>> {
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
        Ok(records)
    }

    fn list_events(
        &self,
        session_id: &str,
        internal: bool,
        after: Option<&str>,
    ) -> StoreResult<Vec<BridgeEvent>> {
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
        Ok(out)
    }

    fn ack_delivery(&self, session_id: &str, req: DeliveryAckRequest) -> StoreResult<BridgeEvent> {
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
        Ok(event)
    }

    fn subscribe(&self) -> broadcast::Receiver<BridgeEvent> {
        self.events_tx.subscribe()
    }
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

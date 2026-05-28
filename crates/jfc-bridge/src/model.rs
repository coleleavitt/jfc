use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Default, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WorkerStatus {
    #[default]
    Idle,
    Busy,
    Draining,
    Offline,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SessionStatus {
    Running,
    #[default]
    Idle,
    Archived,
    Terminated,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BridgeRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub environment_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub metadata: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worker_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BridgeResponse {
    pub session_id: String,
    pub worker_id: String,
    pub worker_jwt: String,
    pub expires_in: u64,
    pub api_base_url: String,
    pub worker_epoch: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateSessionRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub environment_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub metadata: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionRecord {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub environment_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub status: SessionStatus,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub metadata: BTreeMap<String, String>,
    pub created_at_ms: u64,
    pub updated_at_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub archived_at_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkerUpdateRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worker_id: Option<String>,
    #[serde(default)]
    pub worker_epoch: u64,
    #[serde(default)]
    pub worker_status: WorkerStatus,
    #[serde(default)]
    pub external_metadata: Value,
    #[serde(default)]
    pub internal_metadata: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HeartbeatRequest {
    #[serde(default)]
    pub worker_epoch: u64,
    #[serde(default)]
    pub worker_status: Option<WorkerStatus>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkerRecord {
    pub session_id: String,
    pub worker_id: String,
    pub worker_epoch: u64,
    pub worker_status: WorkerStatus,
    #[serde(default)]
    pub external_metadata: Value,
    #[serde(default)]
    pub internal_metadata: Value,
    pub updated_at_ms: u64,
    pub last_heartbeat_at_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventUpload {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(default = "default_event_kind")]
    pub kind: String,
    #[serde(default)]
    pub payload: Value,
}

fn default_event_kind() -> String {
    "event".to_owned()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventUploadRequest {
    #[serde(default)]
    pub events: Vec<EventUpload>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BridgeEvent {
    pub id: String,
    pub session_id: String,
    pub kind: String,
    #[serde(default)]
    pub payload: Value,
    pub created_at_ms: u64,
    #[serde(default)]
    pub internal: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delivery_status: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventList {
    #[serde(default)]
    pub events: Vec<BridgeEvent>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeliveryAckRequest {
    pub event_id: String,
    #[serde(default = "default_delivery_status")]
    pub status: String,
}

fn default_delivery_status() -> String {
    "delivered".to_owned()
}

#[derive(Debug, Clone, Deserialize)]
pub struct EventQuery {
    #[serde(default)]
    pub after: Option<String>,
}

#[derive(Debug, Clone)]
pub struct TokenClaims {
    pub session_id: String,
    pub worker_id: String,
    pub exp_ms: u64,
}

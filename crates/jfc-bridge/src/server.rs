use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use futures_util::Stream;
use serde_json::json;
use thiserror::Error;
use uuid::Uuid;

use crate::model::{
    BridgeRequest, BridgeResponse, CreateSessionRequest, DeliveryAckRequest, EventList, EventQuery,
    EventUploadRequest, HeartbeatRequest, TokenClaims, WorkerRecord, WorkerStatus,
    WorkerUpdateRequest,
};
use crate::store::{BridgeStore, MemoryBridgeStore, StoreError};
use crate::time::now_ms;
use crate::token::{TokenError, mint_worker_token, verify_worker_token};

#[derive(Clone)]
pub struct BridgeConfig {
    pub api_base_url: String,
    pub secret: Vec<u8>,
    pub bootstrap_token: Option<String>,
    pub token_ttl: Duration,
}

impl BridgeConfig {
    pub fn new(api_base_url: impl Into<String>, secret: impl Into<Vec<u8>>) -> Self {
        Self {
            api_base_url: api_base_url.into(),
            secret: secret.into(),
            bootstrap_token: None,
            token_ttl: Duration::from_secs(12 * 60 * 60),
        }
    }
}

#[derive(Clone)]
pub struct BridgeState {
    store: Arc<dyn BridgeStore>,
    config: Arc<BridgeConfig>,
}

impl BridgeState {
    pub fn new(store: Arc<dyn BridgeStore>, config: BridgeConfig) -> Self {
        Self {
            store,
            config: Arc::new(config),
        }
    }

    pub fn memory(config: BridgeConfig) -> Self {
        Self::new(Arc::new(MemoryBridgeStore::new()), config)
    }
}

pub fn router(state: BridgeState) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/bridge", post(create_bridge))
        .route("/worker", get(get_worker).put(update_worker))
        .route("/worker/heartbeat", post(worker_heartbeat))
        .route("/worker/events", post(post_worker_events))
        .route("/worker/internal-events", get(get_internal_events))
        .route("/worker/internal-events", post(post_internal_events))
        .route("/worker/events/delivery", post(post_event_delivery))
        .route("/sessions", post(create_session))
        .route("/sessions/:session_id", get(get_session))
        .route("/sessions/:session_id/archive", post(archive_session))
        .route("/sessions/:session_id/events", get(get_session_events))
        .route(
            "/sessions/:session_id/events/stream",
            get(stream_session_events),
        )
        .with_state(state)
}

pub async fn serve(addr: SocketAddr, state: BridgeState) -> std::io::Result<()> {
    let listener = tokio::net::TcpListener::bind(addr).await?;
    tracing::info!(
        target: "jfc::bridge",
        addr = %listener.local_addr()?,
        "bridge server listening"
    );
    axum::serve(listener, router(state)).await
}

async fn health() -> Json<serde_json::Value> {
    Json(json!({ "ok": true }))
}

async fn create_bridge(
    State(state): State<BridgeState>,
    headers: HeaderMap,
    Json(req): Json<BridgeRequest>,
) -> Result<Json<BridgeResponse>, ApiError> {
    require_bootstrap(&state, &headers)?;
    let session = state.store.create_session(CreateSessionRequest {
        environment_id: req.environment_id,
        title: req.title,
        tags: req.tags,
        metadata: req.metadata,
    })?;
    let worker_id = req
        .worker_id
        .unwrap_or_else(|| format!("wrk_{}", Uuid::new_v4().simple()));
    let worker_epoch = 1;
    state.store.upsert_worker(
        &session.id,
        &worker_id,
        WorkerUpdateRequest {
            worker_id: Some(worker_id.clone()),
            worker_epoch,
            worker_status: WorkerStatus::Idle,
            external_metadata: serde_json::Value::Null,
            internal_metadata: serde_json::Value::Null,
        },
    )?;
    Ok(Json(BridgeResponse {
        session_id: session.id.clone(),
        worker_id: worker_id.clone(),
        worker_jwt: mint_token(&state, &session.id, &worker_id)?,
        expires_in: state.config.token_ttl.as_secs(),
        api_base_url: state.config.api_base_url.clone(),
        worker_epoch,
    }))
}

async fn create_session(
    State(state): State<BridgeState>,
    headers: HeaderMap,
    Json(req): Json<CreateSessionRequest>,
) -> Result<Json<crate::model::SessionRecord>, ApiError> {
    require_bootstrap(&state, &headers)?;
    Ok(Json(state.store.create_session(req)?))
}

async fn get_session(
    State(state): State<BridgeState>,
    headers: HeaderMap,
    Path(session_id): Path<String>,
) -> Result<Json<crate::model::SessionRecord>, ApiError> {
    require_session_access(&state, &headers, &session_id)?;
    let session = state
        .store
        .get_session(&session_id)?
        .ok_or_else(|| ApiError::NotFound(format!("session not found: {session_id}")))?;
    Ok(Json(session))
}

async fn archive_session(
    State(state): State<BridgeState>,
    headers: HeaderMap,
    Path(session_id): Path<String>,
) -> Result<Json<crate::model::SessionRecord>, ApiError> {
    require_session_access(&state, &headers, &session_id)?;
    Ok(Json(state.store.archive_session(&session_id)?))
}

async fn get_worker(
    State(state): State<BridgeState>,
    headers: HeaderMap,
) -> Result<Json<WorkerRecord>, ApiError> {
    let claims = require_worker(&state, &headers)?;
    let worker = state
        .store
        .get_worker(&claims.session_id)?
        .ok_or_else(|| ApiError::NotFound("worker not found".to_owned()))?;
    Ok(Json(worker))
}

async fn update_worker(
    State(state): State<BridgeState>,
    headers: HeaderMap,
    Json(req): Json<WorkerUpdateRequest>,
) -> Result<Json<WorkerRecord>, ApiError> {
    let claims = require_worker(&state, &headers)?;
    Ok(Json(state.store.upsert_worker(
        &claims.session_id,
        &claims.worker_id,
        req,
    )?))
}

async fn worker_heartbeat(
    State(state): State<BridgeState>,
    headers: HeaderMap,
    Json(req): Json<HeartbeatRequest>,
) -> Result<Json<WorkerRecord>, ApiError> {
    let claims = require_worker(&state, &headers)?;
    let epoch = if req.worker_epoch == 0 {
        claims_worker_epoch(&state, &claims.session_id)?
    } else {
        req.worker_epoch
    };
    Ok(Json(state.store.heartbeat(
        &claims.session_id,
        &claims.worker_id,
        epoch,
        req.worker_status,
    )?))
}

async fn post_worker_events(
    State(state): State<BridgeState>,
    headers: HeaderMap,
    Json(req): Json<EventUploadRequest>,
) -> Result<Json<EventList>, ApiError> {
    let claims = require_worker(&state, &headers)?;
    let events = state
        .store
        .append_events(&claims.session_id, false, req.events)?;
    Ok(Json(EventList { events }))
}

async fn post_internal_events(
    State(state): State<BridgeState>,
    headers: HeaderMap,
    Json(req): Json<EventUploadRequest>,
) -> Result<Json<EventList>, ApiError> {
    let claims = require_worker(&state, &headers)?;
    let events = state
        .store
        .append_events(&claims.session_id, true, req.events)?;
    Ok(Json(EventList { events }))
}

async fn get_internal_events(
    State(state): State<BridgeState>,
    headers: HeaderMap,
    Query(query): Query<EventQuery>,
) -> Result<Json<EventList>, ApiError> {
    let claims = require_worker(&state, &headers)?;
    let events = state
        .store
        .list_events(&claims.session_id, true, query.after.as_deref())?;
    Ok(Json(EventList { events }))
}

async fn post_event_delivery(
    State(state): State<BridgeState>,
    headers: HeaderMap,
    Json(req): Json<DeliveryAckRequest>,
) -> Result<Json<crate::model::BridgeEvent>, ApiError> {
    let claims = require_worker(&state, &headers)?;
    Ok(Json(state.store.ack_delivery(&claims.session_id, req)?))
}

async fn get_session_events(
    State(state): State<BridgeState>,
    headers: HeaderMap,
    Path(session_id): Path<String>,
    Query(query): Query<EventQuery>,
) -> Result<Json<EventList>, ApiError> {
    require_session_access(&state, &headers, &session_id)?;
    let events = state
        .store
        .list_events(&session_id, false, query.after.as_deref())?;
    Ok(Json(EventList { events }))
}

async fn stream_session_events(
    State(state): State<BridgeState>,
    headers: HeaderMap,
    Path(session_id): Path<String>,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, ApiError> {
    require_session_access(&state, &headers, &session_id)?;
    let mut rx = state.store.subscribe();
    let stream_session_id = session_id.clone();
    let stream = async_stream::stream! {
        loop {
            match rx.recv().await {
                Ok(event) if event.session_id == stream_session_id && !event.internal => {
                    let data = serde_json::to_string(&event).unwrap_or_else(|_| "{}".to_owned());
                    yield Ok(Event::default().id(event.id).event(event.kind).data(data));
                }
                Ok(_) => {}
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                    yield Ok(Event::default().event("bridge_lagged").data("{}"));
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    };
    Ok(Sse::new(stream).keep_alive(
        KeepAlive::new()
            .interval(Duration::from_secs(20))
            .text("keepalive"),
    ))
}

fn mint_token(state: &BridgeState, session_id: &str, worker_id: &str) -> Result<String, ApiError> {
    let exp_ms = now_ms().saturating_add(state.config.token_ttl.as_millis() as u64);
    Ok(mint_worker_token(
        &state.config.secret,
        &TokenClaims {
            session_id: session_id.to_owned(),
            worker_id: worker_id.to_owned(),
            exp_ms,
        },
    )?)
}

fn require_bootstrap(state: &BridgeState, headers: &HeaderMap) -> Result<(), ApiError> {
    let Some(expected) = state.config.bootstrap_token.as_deref() else {
        return Ok(());
    };
    let Some(actual) = bearer(headers) else {
        return Err(ApiError::Unauthorized);
    };
    (actual == expected)
        .then_some(())
        .ok_or(ApiError::Unauthorized)
}

fn require_worker(state: &BridgeState, headers: &HeaderMap) -> Result<TokenClaims, ApiError> {
    let token = bearer(headers).ok_or(ApiError::Unauthorized)?;
    Ok(verify_worker_token(&state.config.secret, token)?)
}

fn require_session_access(
    state: &BridgeState,
    headers: &HeaderMap,
    session_id: &str,
) -> Result<(), ApiError> {
    if require_bootstrap(state, headers).is_ok() && state.config.bootstrap_token.is_some() {
        return Ok(());
    }
    let claims = require_worker(state, headers)?;
    (claims.session_id == session_id)
        .then_some(())
        .ok_or(ApiError::Unauthorized)
}

fn claims_worker_epoch(state: &BridgeState, session_id: &str) -> Result<u64, ApiError> {
    let worker = state
        .store
        .get_worker(session_id)?
        .ok_or_else(|| ApiError::NotFound("worker not found".to_owned()))?;
    Ok(worker.worker_epoch)
}

fn bearer(headers: &HeaderMap) -> Option<&str> {
    let value = headers
        .get(axum::http::header::AUTHORIZATION)?
        .to_str()
        .ok()?;
    value
        .strip_prefix("Bearer ")
        .or_else(|| value.strip_prefix("bearer "))
}

#[derive(Debug, Error)]
pub enum ApiError {
    #[error("unauthorized")]
    Unauthorized,
    #[error("{0}")]
    NotFound(String),
    #[error("{0}")]
    BadRequest(String),
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error(transparent)]
    Token(#[from] TokenError),
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let status = match &self {
            Self::Unauthorized | Self::Token(_) => StatusCode::UNAUTHORIZED,
            Self::NotFound(_) | Self::Store(StoreError::SessionNotFound(_)) => {
                StatusCode::NOT_FOUND
            }
            Self::BadRequest(_) | Self::Store(StoreError::WorkerEpochMismatch { .. }) => {
                StatusCode::BAD_REQUEST
            }
            Self::Store(StoreError::Poisoned | StoreError::Persistence) => {
                StatusCode::INTERNAL_SERVER_ERROR
            }
        };
        let body = Json(json!({
            "error": self.to_string(),
        }));
        (status, body).into_response()
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    #[tokio::test]
    async fn bridge_bootstrap_creates_worker_token() {
        let state = BridgeState::memory(BridgeConfig {
            api_base_url: "http://127.0.0.1:1".to_owned(),
            secret: b"secret".to_vec(),
            bootstrap_token: None,
            token_ttl: Duration::from_secs(60),
        });
        let headers = HeaderMap::new();
        let Json(resp) = create_bridge(
            State(state),
            headers,
            Json(BridgeRequest {
                environment_id: None,
                title: Some("test".to_owned()),
                tags: vec![],
                metadata: Default::default(),
                worker_id: Some("worker".to_owned()),
            }),
        )
        .await
        .unwrap();
        assert_eq!(resp.worker_id, "worker");
        assert_eq!(resp.worker_epoch, 1);
        assert!(!resp.worker_jwt.is_empty());
    }
}

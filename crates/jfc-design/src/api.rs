//! Axum API for the design workspace.
//!
//! This is the Phase-A surface: project CRUD, sandboxed file APIs, static preview,
//! SSE change events, and native tools that do not require a browser. Browser-host
//! endpoints are present with honest `501` responses so a frontend can integrate
//! against stable routes while the Chromium backend is implemented behind them.

use std::collections::HashMap;
use std::convert::Infallible;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use axum::extract::{Path as AxumPath, Query, Request, State};
use axum::http::header::{CONTENT_DISPOSITION, CONTENT_TYPE};
use axum::http::{HeaderValue, StatusCode};
use axum::middleware::{self, Next};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use base64::Engine as _;
use futures_util::Stream;
use hmac::{Hmac, KeyInit, Mac};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::Sha256;
use tokio::sync::broadcast;

use crate::project::{DesignProject, ProjectMeta, ProjectStore};
use crate::{
    DesignError, Result as DesignResult, browser, design_system, handoff, inline, io_err, media,
    mime, source_map,
};

const DIRECT_EDIT_OVERRIDES_PATH: &str = "__om-edit-overrides.json";
const DIRECT_EDIT_LOG_PATH: &str = "__om-direct-edits.json";
const CHAT_HISTORY_PATH: &str = "chat.json";
const PUBLIC_SHARES_PATH: &str = "__jfc-public-shares.json";
const TWEAKS_PATH: &str = "tweaks.json";
const PUBLIC_TOKEN_TTL_MS: u64 = 7 * 24 * 60 * 60 * 1000;
type HmacSha256 = Hmac<Sha256>;

#[derive(Clone)]
pub struct DesignServerState {
    store: Arc<ProjectStore>,
    events: broadcast::Sender<DesignEvent>,
    next_event_id: Arc<AtomicU64>,
}

impl DesignServerState {
    pub fn new(store: ProjectStore) -> Self {
        let (events, _) = broadcast::channel(256);
        Self {
            store: Arc::new(store),
            events,
            next_event_id: Arc::new(AtomicU64::new(1)),
        }
    }

    pub fn default_in(cwd: impl AsRef<std::path::Path>) -> DesignResult<Self> {
        Ok(Self::new(ProjectStore::default_in(cwd)?))
    }

    pub fn store(&self) -> &ProjectStore {
        &self.store
    }

    fn publish(
        &self,
        project_id: Option<String>,
        kind: &'static str,
        path: Option<String>,
        meta: Option<ProjectMeta>,
    ) {
        self.publish_data(project_id, kind, path, meta, None);
    }

    fn publish_data(
        &self,
        project_id: Option<String>,
        kind: &'static str,
        path: Option<String>,
        meta: Option<ProjectMeta>,
        data: Option<Value>,
    ) {
        let id = self.next_event_id.fetch_add(1, Ordering::Relaxed);
        let event = DesignEvent {
            id: id.to_string(),
            ts_ms: now_ms(),
            project_id,
            kind: kind.to_owned(),
            path,
            meta,
            data,
        };
        let _ = self.events.send(event);
    }
}

pub fn router(state: DesignServerState) -> Router {
    let protected = Router::new()
        .route("/design/projects", get(list_projects).post(create_project))
        .route(
            "/design/projects/:project_id",
            get(get_project).patch(update_project),
        )
        .route("/design/projects/:project_id/files", get(list_files))
        .route(
            "/design/projects/:project_id/file",
            get(read_file).put(write_file).delete(delete_file),
        )
        .route("/design/projects/:project_id/files/copy", post(copy_file))
        .route("/design/projects/:project_id/assets", post(register_asset))
        .route(
            "/design/projects/:project_id/assets/by-path",
            delete(unregister_asset),
        )
        .route(
            "/design/projects/:project_id/serve",
            get(serve_project_index),
        )
        .route(
            "/design/projects/:project_id/serve/*path",
            get(serve_project_file),
        )
        .route("/design/projects/:project_id/events", get(stream_events))
        .route(
            "/design/projects/:project_id/tools/super-inline-html",
            post(bundle_html),
        )
        .route(
            "/design/projects/:project_id/tools/handoff",
            post(handoff_project),
        )
        .route(
            "/design/projects/:project_id/tools/check-design-system",
            post(check_design_system),
        )
        .route("/design/projects/:project_id/tools/eval-js", post(eval_js))
        .route(
            "/design/projects/:project_id/tools/screenshot",
            post(save_screenshot),
        )
        .route(
            "/design/projects/:project_id/tools/multi-screenshot",
            post(multi_screenshot),
        )
        .route(
            "/design/projects/:project_id/tools/gen-pptx",
            post(gen_pptx),
        )
        .route(
            "/design/projects/:project_id/tools/save-pdf",
            post(save_pdf),
        )
        .route(
            "/design/projects/:project_id/tools/direct-edit-inspect",
            post(direct_edit_inspect),
        )
        .route(
            "/design/projects/:project_id/tools/direct-edit-apply",
            post(direct_edit_apply),
        )
        .route("/design/projects/:project_id/tools/verify", post(verify))
        .route(
            "/design/projects/:project_id/tools/verify-orchestrate",
            post(verify_orchestrate),
        )
        .route("/design/projects/:project_id/tools/done", post(done))
        .route(
            "/design/projects/:project_id/tools/direct-edit-overrides",
            get(read_direct_edit_overrides).put(write_direct_edit_overrides),
        )
        .route(
            "/design/projects/:project_id/tools/tweaks",
            get(read_tweaks).put(write_tweaks),
        )
        .route(
            "/design/projects/:project_id/tools/dc-write",
            post(dc_write),
        )
        .route(
            "/design/projects/:project_id/tools/dc-stream",
            get(dc_stream),
        )
        .route(
            "/design/projects/:project_id/tools/chat",
            get(read_chat).post(chat),
        )
        .route(
            "/design/projects/:project_id/tools/generate-image",
            post(generate_image),
        )
        .route(
            "/design/projects/:project_id/tools/generate-sound",
            post(generate_sound),
        )
        .route("/design/projects/:project_id/download", get(download_file))
        .route("/design/projects/:project_id/print", get(print_file))
        .route(
            "/design/projects/:project_id/public-token",
            post(create_public_token),
        )
        .route(
            "/design/projects/:project_id/public-shares",
            get(list_public_shares),
        )
        .route(
            "/design/projects/:project_id/public-shares/:token",
            delete(revoke_public_share),
        )
        .route(
            "/design/projects/:project_id/public/:token",
            get(public_file),
        )
        .layer(middleware::from_fn(require_design_api_token));

    Router::new()
        .route("/health", get(health))
        .route("/design/capabilities", get(capabilities))
        .route("/design/public/:token", get(public_share_entry))
        .route("/design/public/:token/", get(public_share_entry))
        .route("/design/public/:token/*path", get(public_share_file))
        .merge(protected)
        .with_state(state)
}

pub async fn serve(addr: SocketAddr, state: DesignServerState) -> std::io::Result<()> {
    let listener = tokio::net::TcpListener::bind(addr).await?;
    tracing::info!(
        target: "jfc::design",
        addr = %listener.local_addr()?,
        "design API server listening"
    );
    axum::serve(listener, router(state)).await
}

#[derive(Debug, Clone, Serialize)]
pub struct DesignEvent {
    pub id: String,
    pub ts_ms: u64,
    pub project_id: Option<String>,
    pub kind: String,
    pub path: Option<String>,
    pub meta: Option<ProjectMeta>,
    pub data: Option<Value>,
}

#[derive(Debug, Serialize)]
struct ProjectResponse {
    meta: ProjectMeta,
    root: String,
}

#[derive(Debug, Deserialize)]
struct CreateProjectRequest {
    title: String,
}

#[derive(Debug, Deserialize)]
struct UpdateProjectRequest {
    title: Option<String>,
    is_design_system: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct FileQuery {
    path: Option<String>,
}

#[derive(Debug, Serialize)]
struct FileReadResponse {
    path: String,
    encoding: &'static str,
    content: String,
}

#[derive(Debug, Deserialize)]
struct WriteFileRequest {
    content: String,
    encoding: Option<String>,
    asset_name: Option<String>,
}

#[derive(Debug, Deserialize)]
struct CopyFileRequest {
    from_path: String,
    to_path: String,
}

#[derive(Debug, Deserialize)]
struct AssetRequest {
    name: String,
    path: String,
}

#[derive(Debug, Deserialize)]
struct BundleHtmlRequest {
    input: String,
    output: Option<String>,
    require_thumbnail: Option<bool>,
}

#[derive(Debug, Serialize)]
struct BundleHtmlResponse {
    output: String,
    bytes: usize,
    misses: Vec<String>,
    summary: String,
}

#[derive(Debug, Deserialize)]
struct HandoffRequest {
    feature: String,
    files: Vec<String>,
}

#[derive(Debug, Serialize)]
struct HandoffResponse {
    dir: String,
    readme: String,
    copied: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct PublicTokenRequest {
    path: String,
    ttl_ms: Option<u64>,
    ttl_seconds: Option<u64>,
    title: Option<String>,
    allow_download: Option<bool>,
}

#[derive(Debug, Serialize, Deserialize)]
struct PublicTokenPayload {
    project_id: String,
    path: String,
    issued_at_ms: u64,
    expires_at_ms: u64,
    scope_dir: String,
}

#[derive(Debug, Serialize)]
struct PublicTokenResponse {
    path: String,
    token: String,
    url: String,
    public_url: String,
    embed_url: String,
    expires_at_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PublicShareRecord {
    path: String,
    token: String,
    public_url: String,
    embed_url: String,
    issued_at_ms: u64,
    expires_at_ms: u64,
    scope_dir: String,
    title: Option<String>,
    allow_download: bool,
    revoked_at_ms: Option<u64>,
}

#[derive(Debug, Serialize)]
struct DirectEditResponse {
    path: String,
    overrides: Value,
}

#[derive(Debug, Deserialize)]
struct DirectEditRequest {
    path: Option<String>,
    overrides: Value,
}

#[derive(Debug, Serialize)]
struct DirectEditApplyResponse {
    path: String,
    selector: String,
    bytes: usize,
    runtime: String,
    overrides_path: String,
    source_rewritten: bool,
    source_path: Option<String>,
    fallback_reason: Option<String>,
    overrides: Value,
}

#[derive(Debug, Deserialize)]
struct DirectEditApplyRequest {
    path: Option<String>,
    selector: String,
    text: Option<String>,
    html: Option<String>,
    attributes: Option<Value>,
    styles: Option<Value>,
    source_path: Option<String>,
    source_start: Option<usize>,
    source_end: Option<usize>,
    source_line: Option<usize>,
    source_column: Option<usize>,
    source_kind: Option<String>,
    generated_path: Option<String>,
    generated_line: Option<usize>,
    generated_column: Option<usize>,
    source_map_path: Option<String>,
    previous_text: Option<String>,
    fallback_overlay: Option<bool>,
}

#[derive(Debug, Serialize)]
struct TweaksResponse {
    path: String,
    values: Value,
}

#[derive(Debug, Deserialize)]
struct TweaksRequest {
    path: Option<String>,
    values: Value,
}

#[derive(Debug, Deserialize)]
struct DcWriteRequest {
    path: String,
    content: String,
    append: Option<bool>,
    name: Option<String>,
    kind: Option<String>,
    streaming: Option<bool>,
}

#[derive(Debug, Serialize)]
struct DcWriteResponse {
    path: String,
    bytes: usize,
    appended: bool,
    streaming: bool,
    name: Option<String>,
    kind: Option<String>,
}

#[derive(Debug, Deserialize)]
struct VerifyOrchestrateRequest {
    path: Option<String>,
    output_dir: Option<String>,
    selector: Option<String>,
    wait_ms: Option<u64>,
    timeout_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ChatMessage {
    role: String,
    content: String,
    ts_ms: u64,
    path: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ChatRequest {
    message: String,
    path: Option<String>,
}

#[derive(Debug, Serialize)]
struct ChatResponse {
    path: String,
    reply: String,
    actions: Vec<String>,
    messages: Vec<ChatMessage>,
}

#[derive(Debug)]
pub struct ApiError {
    status: StatusCode,
    message: String,
}

impl ApiError {
    fn new(status: StatusCode, message: impl Into<String>) -> Self {
        Self {
            status,
            message: message.into(),
        }
    }

    fn bad_request(message: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, message)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let status = self.status;
        let body = Json(json!({ "error": self.message }));
        (status, body).into_response()
    }
}

impl From<DesignError> for ApiError {
    fn from(err: DesignError) -> Self {
        match err {
            DesignError::PathEscape(path) => {
                Self::bad_request(format!("path escapes design sandbox: {path}"))
            }
            DesignError::ProjectNotFound(id) => {
                Self::new(StatusCode::NOT_FOUND, format!("project not found: {id}"))
            }
            DesignError::BadMetadata(msg) => Self::new(StatusCode::INTERNAL_SERVER_ERROR, msg),
            DesignError::Bundle(msg) => Self::bad_request(msg),
            DesignError::Browser(msg) => Self::new(StatusCode::BAD_GATEWAY, msg),
            DesignError::Json(e) => Self::bad_request(e.to_string()),
            DesignError::Io { path, source } => {
                let status = if source.kind() == std::io::ErrorKind::NotFound {
                    StatusCode::NOT_FOUND
                } else {
                    StatusCode::INTERNAL_SERVER_ERROR
                };
                Self::new(status, format!("io error at {path}: {source}"))
            }
        }
    }
}

async fn require_design_api_token(req: Request, next: Next) -> Response {
    let expected = std::env::var("JFC_DESIGN_API_TOKEN")
        .ok()
        .filter(|token| !token.trim().is_empty());
    let Some(expected) = expected else {
        return next.run(req).await;
    };

    let headers = req.headers();
    let bearer_ok = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .is_some_and(|token| constant_time_eq(token.as_bytes(), expected.as_bytes()));
    let token_ok = headers
        .get("x-jfc-design-token")
        .and_then(|value| value.to_str().ok())
        .is_some_and(|token| constant_time_eq(token.as_bytes(), expected.as_bytes()));
    if bearer_ok || token_ok {
        next.run(req).await
    } else {
        ApiError::new(StatusCode::UNAUTHORIZED, "design API token required").into_response()
    }
}

async fn health() -> Json<serde_json::Value> {
    Json(json!({ "ok": true }))
}

async fn capabilities() -> Json<serde_json::Value> {
    Json(json!({
        "matrix": crate::capabilities::matrix(),
        "markdown": crate::capabilities::markdown_table()
    }))
}

async fn list_projects(
    State(state): State<DesignServerState>,
) -> std::result::Result<Json<Vec<ProjectMeta>>, ApiError> {
    Ok(Json(state.store.list()?))
}

async fn create_project(
    State(state): State<DesignServerState>,
    Json(req): Json<CreateProjectRequest>,
) -> std::result::Result<Json<ProjectResponse>, ApiError> {
    if req.title.trim().is_empty() {
        return Err(ApiError::bad_request("title is required"));
    }
    let project = state.store.create(req.title)?;
    let meta = project.meta().clone();
    state.publish(Some(meta.id.clone()), "project.created", None, Some(meta));
    Ok(Json(project_response(project)))
}

async fn get_project(
    State(state): State<DesignServerState>,
    AxumPath(project_id): AxumPath<String>,
) -> std::result::Result<Json<ProjectResponse>, ApiError> {
    Ok(Json(project_response(state.store.open(&project_id)?)))
}

async fn update_project(
    State(state): State<DesignServerState>,
    AxumPath(project_id): AxumPath<String>,
    Json(req): Json<UpdateProjectRequest>,
) -> std::result::Result<Json<ProjectResponse>, ApiError> {
    let mut project = state.store.open(&project_id)?;
    if let Some(title) = req.title {
        if title.trim().is_empty() {
            return Err(ApiError::bad_request("title must not be empty"));
        }
        project.set_title(title)?;
    }
    if let Some(yes) = req.is_design_system {
        project.set_is_design_system(yes)?;
    }
    let meta = project.meta().clone();
    state.publish(Some(meta.id.clone()), "project.updated", None, Some(meta));
    Ok(Json(project_response(project)))
}

async fn list_files(
    State(state): State<DesignServerState>,
    AxumPath(project_id): AxumPath<String>,
) -> std::result::Result<Json<Vec<String>>, ApiError> {
    let project = state.store.open(&project_id)?;
    Ok(Json(project.list_files()))
}

async fn read_file(
    State(state): State<DesignServerState>,
    AxumPath(project_id): AxumPath<String>,
    Query(query): Query<FileQuery>,
) -> std::result::Result<Json<FileReadResponse>, ApiError> {
    let rel = require_file_path(query)?;
    let project = state.store.open(&project_id)?;
    let bytes = project.read_file(&rel)?;
    let (encoding, content) = match String::from_utf8(bytes) {
        Ok(text) => ("utf-8", text),
        Err(e) => (
            "base64",
            base64::engine::general_purpose::STANDARD.encode(e.into_bytes()),
        ),
    };
    Ok(Json(FileReadResponse {
        path: rel,
        encoding,
        content,
    }))
}

async fn write_file(
    State(state): State<DesignServerState>,
    AxumPath(project_id): AxumPath<String>,
    Query(query): Query<FileQuery>,
    Json(req): Json<WriteFileRequest>,
) -> std::result::Result<Json<ProjectResponse>, ApiError> {
    let rel = require_file_path(query)?;
    let bytes = decode_content(req.content, req.encoding.as_deref())?;
    let mut project = state.store.open(&project_id)?;
    project.write_file(&rel, &bytes, req.asset_name.as_deref())?;
    let meta = project.meta().clone();
    state.publish(Some(project_id), "file.written", Some(rel), Some(meta));
    Ok(Json(project_response(project)))
}

async fn delete_file(
    State(state): State<DesignServerState>,
    AxumPath(project_id): AxumPath<String>,
    Query(query): Query<FileQuery>,
) -> std::result::Result<Json<ProjectResponse>, ApiError> {
    let rel = require_file_path(query)?;
    let mut project = state.store.open(&project_id)?;
    project.delete_path(&rel)?;
    let meta = project.meta().clone();
    state.publish(Some(project_id), "file.deleted", Some(rel), Some(meta));
    Ok(Json(project_response(project)))
}

async fn copy_file(
    State(state): State<DesignServerState>,
    AxumPath(project_id): AxumPath<String>,
    Json(req): Json<CopyFileRequest>,
) -> std::result::Result<Json<ProjectResponse>, ApiError> {
    let project = state.store.open(&project_id)?;
    project.copy_file(&req.from_path, &req.to_path)?;
    let meta = project.meta().clone();
    state.publish(
        Some(project_id),
        "file.copied",
        Some(req.to_path),
        Some(meta),
    );
    Ok(Json(project_response(project)))
}

async fn register_asset(
    State(state): State<DesignServerState>,
    AxumPath(project_id): AxumPath<String>,
    Json(req): Json<AssetRequest>,
) -> std::result::Result<Json<ProjectResponse>, ApiError> {
    let mut project = state.store.open(&project_id)?;
    project.register_asset(&req.name, &req.path)?;
    let meta = project.meta().clone();
    state.publish(
        Some(project_id),
        "asset.registered",
        Some(req.path),
        Some(meta),
    );
    Ok(Json(project_response(project)))
}

async fn unregister_asset(
    State(state): State<DesignServerState>,
    AxumPath(project_id): AxumPath<String>,
    Query(query): Query<FileQuery>,
) -> std::result::Result<Json<ProjectResponse>, ApiError> {
    let rel = require_file_path(query)?;
    let mut project = state.store.open(&project_id)?;
    project.unregister_asset(&rel)?;
    let meta = project.meta().clone();
    state.publish(
        Some(project_id),
        "asset.unregistered",
        Some(rel),
        Some(meta),
    );
    Ok(Json(project_response(project)))
}

async fn serve_project_index(
    State(state): State<DesignServerState>,
    AxumPath(project_id): AxumPath<String>,
) -> std::result::Result<Response, ApiError> {
    serve_project_rel(&state, &project_id, "index.html")
}

async fn serve_project_file(
    State(state): State<DesignServerState>,
    AxumPath((project_id, path)): AxumPath<(String, String)>,
) -> std::result::Result<Response, ApiError> {
    let rel = if path.is_empty() {
        "index.html"
    } else {
        path.as_str()
    };
    serve_project_rel(&state, &project_id, rel)
}

async fn stream_events(
    State(state): State<DesignServerState>,
    AxumPath(project_id): AxumPath<String>,
) -> std::result::Result<Sse<impl Stream<Item = std::result::Result<Event, Infallible>>>, ApiError>
{
    state.store.open(&project_id)?;
    let mut rx = state.events.subscribe();
    let stream_project_id = project_id;
    let stream = async_stream::stream! {
        loop {
            match rx.recv().await {
                Ok(event) if event.project_id.as_deref() == Some(stream_project_id.as_str()) => {
                    let data = serde_json::to_string(&event).unwrap_or_else(|_| "{}".to_owned());
                    yield Ok(Event::default().id(event.id).event(event.kind).data(data));
                }
                Ok(_) => {}
                Err(broadcast::error::RecvError::Lagged(_)) => {
                    yield Ok(Event::default().event("design_lagged").data("{}"));
                }
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
    };
    Ok(Sse::new(stream).keep_alive(
        KeepAlive::new()
            .interval(Duration::from_secs(20))
            .text("keepalive"),
    ))
}

async fn bundle_html(
    State(state): State<DesignServerState>,
    AxumPath(project_id): AxumPath<String>,
    Json(req): Json<BundleHtmlRequest>,
) -> std::result::Result<Json<BundleHtmlResponse>, ApiError> {
    let project = state.store.open(&project_id)?;
    let input = canonical_project_file(&project, &req.input)?;
    let output = match req.output.as_deref() {
        Some(out) => project.resolve(out)?,
        None => input.with_extension("standalone.html"),
    };
    let report = inline::bundle(&input, &output, req.require_thumbnail.unwrap_or(true))?;
    let output_rel = path_relative_to(project.root(), &report.output);
    let response = BundleHtmlResponse {
        output: output_rel.clone(),
        bytes: report.bytes,
        misses: report.misses.clone(),
        summary: report.summary(),
    };
    state.publish(Some(project_id), "tool.bundle_html", Some(output_rel), None);
    Ok(Json(response))
}

async fn handoff_project(
    State(state): State<DesignServerState>,
    AxumPath(project_id): AxumPath<String>,
    Json(req): Json<HandoffRequest>,
) -> std::result::Result<Json<HandoffResponse>, ApiError> {
    let project = state.store.open(&project_id)?;
    let pkg = handoff::scaffold(project.root(), &req.feature, &req.files)?;
    state.publish(
        Some(project_id),
        "tool.handoff",
        Some(path_relative_to(project.root(), &pkg.dir)),
        None,
    );
    Ok(Json(HandoffResponse {
        dir: path_relative_to(project.root(), &pkg.dir),
        readme: path_relative_to(project.root(), &pkg.readme),
        copied: pkg.copied,
    }))
}

async fn check_design_system(
    State(state): State<DesignServerState>,
    AxumPath(project_id): AxumPath<String>,
) -> std::result::Result<Json<design_system::DsManifest>, ApiError> {
    let project = state.store.open(&project_id)?;
    let manifest = design_system::index_and_write(project.root())?;
    state.publish(
        Some(project_id),
        "tool.check_design_system",
        Some("_ds_manifest.json".to_owned()),
        None,
    );
    Ok(Json(manifest))
}

async fn eval_js(
    State(state): State<DesignServerState>,
    AxumPath(project_id): AxumPath<String>,
    Json(req): Json<browser::EvalJsRequest>,
) -> std::result::Result<Json<Value>, ApiError> {
    let rel = req.path.clone().unwrap_or_else(|| "index.html".to_owned());
    let project = state.store.open(&project_id)?;
    let input = canonical_project_file(&project, &rel)?;
    let mut response = browser::eval_js(req, &input).await?;
    add_response_path(&mut response, &rel);
    state.publish(Some(project_id), "tool.eval_js", Some(rel), None);
    Ok(Json(response))
}

async fn save_screenshot(
    State(state): State<DesignServerState>,
    AxumPath(project_id): AxumPath<String>,
    Json(req): Json<browser::ScreenshotRequest>,
) -> std::result::Result<Json<Value>, ApiError> {
    let rel = req.path.clone().unwrap_or_else(|| "index.html".to_owned());
    let output_rel = req
        .output
        .clone()
        .unwrap_or_else(|| default_capture_path(&rel, "screenshots", "png"));
    let mut project = state.store.open(&project_id)?;
    let input = canonical_project_file(&project, &rel)?;
    let output = project.resolve(&output_rel)?;
    let mut response = browser::screenshot(req, &input, Some(&output)).await?;
    let stored_rel = path_relative_to(project.root(), &output);
    project.register_asset("Screenshot", &stored_rel)?;
    add_response_path(&mut response, &rel);
    add_response_output(&mut response, &stored_rel);
    state.publish(
        Some(project_id),
        "tool.screenshot",
        Some(stored_rel),
        Some(project.meta().clone()),
    );
    Ok(Json(response))
}

async fn multi_screenshot(
    State(state): State<DesignServerState>,
    AxumPath(project_id): AxumPath<String>,
    Json(req): Json<browser::MultiScreenshotRequest>,
) -> std::result::Result<Json<Value>, ApiError> {
    let rel = req.path.clone().unwrap_or_else(|| "index.html".to_owned());
    let output_rel = req
        .output_dir
        .clone()
        .unwrap_or_else(|| default_capture_dir(&rel, "screenshots"));
    let mut project = state.store.open(&project_id)?;
    let input = canonical_project_file(&project, &rel)?;
    let output_dir = project.resolve(&output_rel)?;
    std::fs::create_dir_all(&output_dir).map_err(|e| io_err(&output_dir, e))?;
    let mut response = browser::multi_screenshot(req, &input, Some(&output_dir)).await?;
    let stored_rel = path_relative_to(project.root(), &output_dir);
    project.register_asset("Screenshot set", &stored_rel)?;
    add_response_path(&mut response, &rel);
    add_response_output(&mut response, &stored_rel);
    state.publish(
        Some(project_id),
        "tool.multi_screenshot",
        Some(stored_rel),
        Some(project.meta().clone()),
    );
    Ok(Json(response))
}

async fn gen_pptx(
    State(state): State<DesignServerState>,
    AxumPath(project_id): AxumPath<String>,
    Json(req): Json<browser::GenPptxRequest>,
) -> std::result::Result<Json<Value>, ApiError> {
    let rel = req.path.clone().unwrap_or_else(|| "index.html".to_owned());
    let output_rel = req
        .output
        .clone()
        .unwrap_or_else(|| default_capture_path(&rel, "exports", "pptx"));
    let mut project = state.store.open(&project_id)?;
    let input = canonical_project_file(&project, &rel)?;
    let output = project.resolve(&output_rel)?;
    let mut response = browser::gen_pptx(req, &input, &output).await?;
    let stored_rel = path_relative_to(project.root(), &output);
    project.register_asset("PPTX export", &stored_rel)?;
    add_response_path(&mut response, &rel);
    add_response_output(&mut response, &stored_rel);
    state.publish(
        Some(project_id),
        "tool.gen_pptx",
        Some(stored_rel),
        Some(project.meta().clone()),
    );
    Ok(Json(response))
}

async fn save_pdf(
    State(state): State<DesignServerState>,
    AxumPath(project_id): AxumPath<String>,
    Json(req): Json<browser::PrintPdfRequest>,
) -> std::result::Result<Json<Value>, ApiError> {
    let rel = req.path.clone().unwrap_or_else(|| "index.html".to_owned());
    let output_rel = req
        .output
        .clone()
        .unwrap_or_else(|| default_capture_path(&rel, "exports", "pdf"));
    let mut project = state.store.open(&project_id)?;
    let input = canonical_project_file(&project, &rel)?;
    let output = project.resolve(&output_rel)?;
    let mut response = browser::print_pdf(req, &input, &output).await?;
    let stored_rel = path_relative_to(project.root(), &output);
    project.register_asset("PDF export", &stored_rel)?;
    add_response_path(&mut response, &rel);
    add_response_output(&mut response, &stored_rel);
    state.publish(
        Some(project_id),
        "tool.save_pdf",
        Some(stored_rel),
        Some(project.meta().clone()),
    );
    Ok(Json(response))
}

async fn direct_edit_inspect(
    State(state): State<DesignServerState>,
    AxumPath(project_id): AxumPath<String>,
    Json(req): Json<browser::DirectEditInspectRequest>,
) -> std::result::Result<Json<Value>, ApiError> {
    let rel = req.path.clone().unwrap_or_else(|| "index.html".to_owned());
    let project = state.store.open(&project_id)?;
    let input = canonical_project_file(&project, &rel)?;
    let mut response = browser::direct_edit_inspect(req, &input).await?;
    add_response_path(&mut response, &rel);
    state.publish(
        Some(project_id),
        "tool.direct_edit_inspect",
        Some(rel),
        None,
    );
    Ok(Json(response))
}

async fn direct_edit_apply(
    State(state): State<DesignServerState>,
    AxumPath(project_id): AxumPath<String>,
    Json(req): Json<DirectEditApplyRequest>,
) -> std::result::Result<Json<DirectEditApplyResponse>, ApiError> {
    let rel = req.path.unwrap_or_else(|| "index.html".to_owned());
    if !rel.ends_with(".html") && !rel.ends_with(".htm") {
        return Err(ApiError::bad_request(
            "direct-edit-apply path must be an HTML file",
        ));
    }
    if req.selector.trim().is_empty() {
        return Err(ApiError::bad_request("selector is required"));
    }

    let mut project = state.store.open(&project_id)?;
    let raw = project.read_to_string(&rel)?;
    let edit = json!({
        "selector": req.selector,
        "text": req.text,
        "html": req.html,
        "attributes": req.attributes.unwrap_or_else(|| json!({})),
        "styles": req.styles.unwrap_or_else(|| json!({})),
        "source_path": req.source_path,
        "source_start": req.source_start,
        "source_end": req.source_end,
        "source_line": req.source_line,
        "source_column": req.source_column,
        "source_kind": req.source_kind,
        "generated_path": req.generated_path,
        "generated_line": req.generated_line,
        "generated_column": req.generated_column,
        "source_map_path": req.source_map_path,
        "previous_text": req.previous_text,
        "updated_at_ms": now_ms(),
    });
    let rewrite = try_apply_direct_edit_source(&project, &rel, &raw, &edit)?;
    let (updated, runtime, source_rewritten, source_path, fallback_reason, overrides) =
        if let Some(rewrite) = rewrite {
            (
                rewrite.content,
                "source_rewrite".to_owned(),
                true,
                Some(rewrite.path),
                None,
                read_json_file_or_empty(&project, DIRECT_EDIT_OVERRIDES_PATH)?,
            )
        } else if req.fallback_overlay.unwrap_or(true) {
            let mut overrides = read_json_file_or_empty(&project, DIRECT_EDIT_OVERRIDES_PATH)?;
            merge_direct_edit_override(&mut overrides, edit.clone());
            let updated = apply_direct_edit_overlay(&raw, &overrides)?;
            write_json_file(&mut project, DIRECT_EDIT_OVERRIDES_PATH, &overrides)?;
            (
                updated,
                "html_overlay".to_owned(),
                false,
                Some(rel.clone()),
                Some("selector did not map to a single static source element".to_owned()),
                overrides,
            )
        } else {
            return Err(ApiError::bad_request(
                "selector did not map to a static source element",
            ));
        };
    let write_rel = source_path.clone().unwrap_or(rel);
    project.write_file(&write_rel, updated.as_bytes(), None)?;
    append_direct_edit_log(&mut project, &edit, &runtime, &source_path)?;
    let bytes = updated.len();
    state.publish_data(
        Some(project_id),
        "tool.direct_edit_apply",
        Some(write_rel.clone()),
        None,
        Some(json!({
            "selector": edit.get("selector"),
            "bytes": bytes,
            "overrides_path": DIRECT_EDIT_OVERRIDES_PATH,
            "runtime": runtime,
            "source_rewritten": source_rewritten,
            "source_path": source_path,
        })),
    );
    Ok(Json(DirectEditApplyResponse {
        path: write_rel,
        selector: edit
            .get("selector")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
        bytes,
        runtime,
        overrides_path: DIRECT_EDIT_OVERRIDES_PATH.to_owned(),
        source_rewritten,
        source_path,
        fallback_reason,
        overrides,
    }))
}

async fn verify(
    State(state): State<DesignServerState>,
    AxumPath(project_id): AxumPath<String>,
    Json(req): Json<browser::VerifyRequest>,
) -> std::result::Result<Json<Value>, ApiError> {
    let rel = req.path.clone().unwrap_or_else(|| "index.html".to_owned());
    let output_rel = req
        .output
        .clone()
        .unwrap_or_else(|| default_capture_path(&rel, "verifier", "png"));
    let mut project = state.store.open(&project_id)?;
    let input = canonical_project_file(&project, &rel)?;
    let output = project.resolve(&output_rel)?;
    let mut response = browser::verify(req, &input, Some(&output)).await?;
    let stored_rel = path_relative_to(project.root(), &output);
    project.register_asset("Verifier screenshot", &stored_rel)?;
    add_response_path(&mut response, &rel);
    add_response_output(&mut response, &stored_rel);
    state.publish(
        Some(project_id),
        "tool.verify",
        Some(stored_rel),
        Some(project.meta().clone()),
    );
    Ok(Json(response))
}

async fn verify_orchestrate(
    State(state): State<DesignServerState>,
    AxumPath(project_id): AxumPath<String>,
    Json(req): Json<VerifyOrchestrateRequest>,
) -> std::result::Result<Json<Value>, ApiError> {
    let rel = req.path.clone().unwrap_or_else(|| "index.html".to_owned());
    let output_dir_rel = req
        .output_dir
        .clone()
        .unwrap_or_else(|| default_capture_dir(&rel, "verifier"));
    let mut project = state.store.open(&project_id)?;
    let input = canonical_project_file(&project, &rel)?;
    let output_dir = project.resolve(&output_dir_rel)?;
    std::fs::create_dir_all(&output_dir).map_err(|e| io_err(&output_dir, e))?;

    let viewports = [
        ("desktop", 1440, 900),
        ("tablet", 900, 1100),
        ("mobile", 390, 844),
    ];
    let mut runs = Vec::new();
    for (name, width, height) in viewports {
        let output = output_dir.join(format!("{name}.png"));
        let verify_req = browser::VerifyRequest {
            path: Some(rel.clone()),
            output: Some(path_relative_to(project.root(), &output)),
            selector: req.selector.clone(),
            max_screenshots: Some(8),
            include_data: Some(false),
            wait_ms: req.wait_ms.or(Some(250)),
            timeout_ms: req.timeout_ms,
            font_timeout_ms: Some(5_000),
            viewport: Some(browser::BrowserViewport { width, height }),
        };
        let mut response = browser::verify(verify_req, &input, Some(&output)).await?;
        add_response_path(&mut response, &rel);
        add_response_output(&mut response, &path_relative_to(project.root(), &output));
        runs.push(json!({
            "name": name,
            "viewport": { "width": width, "height": height },
            "result": response,
        }));
    }

    let ok = runs.iter().all(|run| {
        run.get("result")
            .and_then(|result| result.get("ok"))
            .and_then(Value::as_bool)
            .unwrap_or(false)
    });
    let warnings = runs
        .iter()
        .filter_map(|run| run.get("result").and_then(|result| result.get("warnings")))
        .flat_map(|warnings| warnings.as_array().into_iter().flatten())
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect::<Vec<_>>();
    let report_rel = format!("{}/report.json", output_dir_rel.trim_end_matches('/'));
    let report = json!({
        "ok": ok,
        "path": rel,
        "output_dir": output_dir_rel,
        "runs": runs,
        "warnings": warnings,
        "duration_ms": 0,
    });
    write_json_file(&mut project, &report_rel, &report)?;
    project.register_asset("Verifier report", &report_rel)?;
    state.publish_data(
        Some(project_id),
        "tool.verify_orchestrate",
        Some(report_rel.clone()),
        Some(project.meta().clone()),
        Some(json!({ "ok": ok, "report": report_rel })),
    );
    Ok(Json(report))
}

async fn done(
    State(state): State<DesignServerState>,
    AxumPath(project_id): AxumPath<String>,
    Json(req): Json<VerifyOrchestrateRequest>,
) -> std::result::Result<Json<Value>, ApiError> {
    let Json(mut report) =
        verify_orchestrate(State(state), AxumPath(project_id), Json(req)).await?;
    let verdict = if report.get("ok").and_then(Value::as_bool).unwrap_or(false) {
        "ready"
    } else {
        "needs_attention"
    };
    if let Some(obj) = report.as_object_mut() {
        obj.insert("verdict".to_owned(), Value::String(verdict.to_owned()));
    }
    Ok(Json(report))
}

async fn read_direct_edit_overrides(
    State(state): State<DesignServerState>,
    AxumPath(project_id): AxumPath<String>,
    Query(query): Query<FileQuery>,
) -> std::result::Result<Json<DirectEditResponse>, ApiError> {
    let rel = query
        .path
        .filter(|p| !p.trim().is_empty())
        .unwrap_or_else(|| DIRECT_EDIT_OVERRIDES_PATH.to_owned());
    let project = state.store.open(&project_id)?;
    Ok(Json(DirectEditResponse {
        path: rel.clone(),
        overrides: read_json_file_or_empty(&project, &rel)?,
    }))
}

async fn write_direct_edit_overrides(
    State(state): State<DesignServerState>,
    AxumPath(project_id): AxumPath<String>,
    Json(req): Json<DirectEditRequest>,
) -> std::result::Result<Json<DirectEditResponse>, ApiError> {
    let rel = req
        .path
        .filter(|p| !p.trim().is_empty())
        .unwrap_or_else(|| DIRECT_EDIT_OVERRIDES_PATH.to_owned());
    let mut project = state.store.open(&project_id)?;
    write_json_file(&mut project, &rel, &req.overrides)?;
    state.publish(
        Some(project_id),
        "tool.direct_edit_overrides",
        Some(rel.clone()),
        None,
    );
    Ok(Json(DirectEditResponse {
        path: rel,
        overrides: req.overrides,
    }))
}

async fn read_tweaks(
    State(state): State<DesignServerState>,
    AxumPath(project_id): AxumPath<String>,
    Query(query): Query<FileQuery>,
) -> std::result::Result<Json<TweaksResponse>, ApiError> {
    let rel = query
        .path
        .filter(|p| !p.trim().is_empty())
        .unwrap_or_else(|| TWEAKS_PATH.to_owned());
    let project = state.store.open(&project_id)?;
    Ok(Json(TweaksResponse {
        path: rel.clone(),
        values: read_json_file_or_empty(&project, &rel)?,
    }))
}

async fn write_tweaks(
    State(state): State<DesignServerState>,
    AxumPath(project_id): AxumPath<String>,
    Json(req): Json<TweaksRequest>,
) -> std::result::Result<Json<TweaksResponse>, ApiError> {
    let rel = req
        .path
        .filter(|p| !p.trim().is_empty())
        .unwrap_or_else(|| TWEAKS_PATH.to_owned());
    let mut project = state.store.open(&project_id)?;
    write_json_file(&mut project, &rel, &req.values)?;
    state.publish(Some(project_id), "tool.tweaks", Some(rel.clone()), None);
    Ok(Json(TweaksResponse {
        path: rel,
        values: req.values,
    }))
}

async fn dc_write(
    State(state): State<DesignServerState>,
    AxumPath(project_id): AxumPath<String>,
    Json(req): Json<DcWriteRequest>,
) -> std::result::Result<Json<DcWriteResponse>, ApiError> {
    if !req.path.ends_with(".dc.html") && !req.path.ends_with(".html") {
        return Err(ApiError::bad_request(
            "dc-write path must end with .dc.html or .html",
        ));
    }
    let mut project = state.store.open(&project_id)?;
    let mut bytes = Vec::new();
    let appended = req.append.unwrap_or(false);
    if appended {
        match project.read_file(&req.path) {
            Ok(existing) => bytes.extend(existing),
            Err(DesignError::Io { source, .. })
                if source.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
    }
    bytes.extend(req.content.as_bytes());
    project.write_file(&req.path, &bytes, None)?;
    let streaming = req.streaming.unwrap_or(false);
    let name = req.name.clone();
    let kind = req.kind.clone();
    state.publish_data(
        Some(project_id),
        "tool.dc_write",
        Some(req.path.clone()),
        None,
        Some(json!({
            "name": name,
            "kind": kind,
            "streaming": streaming,
            "bytes": bytes.len(),
            "appended": appended,
            "content": if streaming { Some(req.content.clone()) } else { None },
        })),
    );
    Ok(Json(DcWriteResponse {
        path: req.path,
        bytes: bytes.len(),
        appended,
        streaming,
        name: req.name,
        kind: req.kind,
    }))
}

async fn dc_stream(
    State(state): State<DesignServerState>,
    AxumPath(project_id): AxumPath<String>,
    Query(query): Query<FileQuery>,
) -> std::result::Result<Sse<impl Stream<Item = std::result::Result<Event, Infallible>>>, ApiError>
{
    let rel = require_file_path(query)?;
    let project = state.store.open(&project_id)?;
    let content = project.read_to_string(&rel)?;
    let stream_path = rel;
    let stream = async_stream::stream! {
        let mut offset = 0usize;
        for chunk in content.as_bytes().chunks(768) {
            let text = String::from_utf8_lossy(chunk).into_owned();
            let data = json!({
                "path": stream_path,
                "offset": offset,
                "append": offset != 0,
                "content": text,
                "done": false,
            });
            offset += chunk.len();
            yield Ok(Event::default().event("dc_html_str_replace").data(data.to_string()));
        }
        let data = json!({ "path": stream_path, "offset": offset, "append": true, "content": "", "done": true });
        yield Ok(Event::default().event("dc_done").data(data.to_string()));
    };
    Ok(Sse::new(stream).keep_alive(
        KeepAlive::new()
            .interval(Duration::from_secs(20))
            .text("keepalive"),
    ))
}

async fn read_chat(
    State(state): State<DesignServerState>,
    AxumPath(project_id): AxumPath<String>,
) -> std::result::Result<Json<ChatResponse>, ApiError> {
    let project = state.store.open(&project_id)?;
    let messages = read_chat_messages(&project)?;
    Ok(Json(ChatResponse {
        path: CHAT_HISTORY_PATH.to_owned(),
        reply: String::new(),
        actions: Vec::new(),
        messages,
    }))
}

async fn chat(
    State(state): State<DesignServerState>,
    AxumPath(project_id): AxumPath<String>,
    Json(req): Json<ChatRequest>,
) -> std::result::Result<Json<ChatResponse>, ApiError> {
    let message = req.message.trim();
    if message.is_empty() {
        return Err(ApiError::bad_request("chat message is required"));
    }
    let mut project = state.store.open(&project_id)?;
    let mut messages = read_chat_messages(&project)?;
    messages.push(ChatMessage {
        role: "user".to_owned(),
        content: message.to_owned(),
        ts_ms: now_ms(),
        path: req.path.clone(),
    });
    let (reply, actions) = design_chat_reply(message, req.path.as_deref());
    messages.push(ChatMessage {
        role: "assistant".to_owned(),
        content: reply.clone(),
        ts_ms: now_ms(),
        path: req.path,
    });
    write_json_file(&mut project, CHAT_HISTORY_PATH, &json!(messages))?;
    state.publish_data(
        Some(project_id),
        "tool.chat",
        Some(CHAT_HISTORY_PATH.to_owned()),
        None,
        Some(json!({ "actions": actions, "reply": reply })),
    );
    Ok(Json(ChatResponse {
        path: CHAT_HISTORY_PATH.to_owned(),
        reply,
        actions,
        messages,
    }))
}

async fn generate_image(
    State(state): State<DesignServerState>,
    AxumPath(project_id): AxumPath<String>,
    Json(req): Json<media::GenerateImageRequest>,
) -> std::result::Result<Json<media::GeneratedMedia>, ApiError> {
    let mut project = state.store.open(&project_id)?;
    let generated = media::generate_image(project.root(), req)?;
    project.register_asset("Generated image", &generated.output)?;
    state.publish(
        Some(project_id),
        "tool.generate_image",
        Some(generated.output.clone()),
        Some(project.meta().clone()),
    );
    Ok(Json(generated))
}

async fn generate_sound(
    State(state): State<DesignServerState>,
    AxumPath(project_id): AxumPath<String>,
    Json(req): Json<media::GenerateSoundRequest>,
) -> std::result::Result<Json<media::GeneratedMedia>, ApiError> {
    let mut project = state.store.open(&project_id)?;
    let generated = media::generate_sound(project.root(), req)?;
    project.register_asset("Generated sound", &generated.output)?;
    state.publish(
        Some(project_id),
        "tool.generate_sound",
        Some(generated.output.clone()),
        Some(project.meta().clone()),
    );
    Ok(Json(generated))
}

async fn download_file(
    State(state): State<DesignServerState>,
    AxumPath(project_id): AxumPath<String>,
    Query(query): Query<FileQuery>,
) -> std::result::Result<Response, ApiError> {
    let rel = require_file_path(query)?;
    let mut response = serve_project_rel(&state, &project_id, &rel)?;
    let filename = rel.rsplit('/').next().unwrap_or("download");
    let disposition = format!("attachment; filename=\"{}\"", filename.replace('"', ""));
    let header = HeaderValue::from_str(&disposition)
        .unwrap_or_else(|_| HeaderValue::from_static("attachment"));
    response.headers_mut().insert(CONTENT_DISPOSITION, header);
    Ok(response)
}

async fn print_file(
    State(state): State<DesignServerState>,
    AxumPath(project_id): AxumPath<String>,
    Query(query): Query<FileQuery>,
) -> std::result::Result<Response, ApiError> {
    let rel = require_file_path(query)?;
    let project = state.store.open(&project_id)?;
    canonical_project_file(&project, &rel)?;
    let src = format!(
        "/design/projects/{}/serve/{}",
        url_segment(&project_id),
        rel.split('/')
            .map(url_segment)
            .collect::<Vec<_>>()
            .join("/")
    );
    let html = format!(
        r#"<!doctype html><html><head><meta charset="utf-8"><title>Print {title}</title><style>html,body,iframe{{margin:0;width:100%;height:100%;border:0;background:white}}</style></head><body><iframe id="artifact" src="{src}"></iframe><script>const frame=document.getElementById('artifact');frame.addEventListener('load',()=>setTimeout(()=>frame.contentWindow?.print(),250),{{once:true}});</script></body></html>"#,
        title = escape_html_text(&rel),
        src = src
    );
    let mut response = html.into_response();
    response.headers_mut().insert(
        CONTENT_TYPE,
        HeaderValue::from_static("text/html; charset=utf-8"),
    );
    Ok(response)
}

async fn create_public_token(
    State(state): State<DesignServerState>,
    AxumPath(project_id): AxumPath<String>,
    Json(req): Json<PublicTokenRequest>,
) -> std::result::Result<Json<PublicTokenResponse>, ApiError> {
    let project = state.store.open(&project_id)?;
    canonical_project_file(&project, &req.path)?;
    let now = now_ms();
    let ttl = req
        .ttl_ms
        .or_else(|| req.ttl_seconds.map(|seconds| seconds.saturating_mul(1000)))
        .unwrap_or(PUBLIC_TOKEN_TTL_MS)
        .clamp(60_000, 30 * 24 * 60 * 60 * 1000);
    let scope_dir = scope_dir_for(&req.path);
    let payload = PublicTokenPayload {
        project_id: project_id.clone(),
        path: req.path,
        issued_at_ms: now,
        expires_at_ms: now.saturating_add(ttl),
        scope_dir,
    };
    let token = sign_public_payload(&payload)?;
    let legacy_url = format!(
        "/design/projects/{}/public/{}",
        url_segment(&project_id),
        url_segment(&token)
    );
    let public_url = public_url_for(&token);
    let mut project = state.store.open(&project_id)?;
    let record = PublicShareRecord {
        path: payload.path.clone(),
        token: token.clone(),
        public_url: public_url.clone(),
        embed_url: public_url.clone(),
        issued_at_ms: payload.issued_at_ms,
        expires_at_ms: payload.expires_at_ms,
        scope_dir: payload.scope_dir.clone(),
        title: req.title.filter(|title| !title.trim().is_empty()),
        allow_download: req.allow_download.unwrap_or(true),
        revoked_at_ms: None,
    };
    upsert_public_share(&mut project, record)?;
    state.publish_data(
        Some(project_id),
        "public_share.created",
        Some(payload.path.clone()),
        None,
        Some(json!({ "token": token, "public_url": public_url })),
    );
    Ok(Json(PublicTokenResponse {
        path: payload.path,
        token,
        url: legacy_url,
        public_url: public_url.clone(),
        embed_url: public_url,
        expires_at_ms: payload.expires_at_ms,
    }))
}

async fn list_public_shares(
    State(state): State<DesignServerState>,
    AxumPath(project_id): AxumPath<String>,
) -> std::result::Result<Json<Vec<PublicShareRecord>>, ApiError> {
    let project = state.store.open(&project_id)?;
    Ok(Json(read_public_shares(&project)?))
}

async fn revoke_public_share(
    State(state): State<DesignServerState>,
    AxumPath((project_id, token)): AxumPath<(String, String)>,
) -> std::result::Result<Json<Vec<PublicShareRecord>>, ApiError> {
    let mut project = state.store.open(&project_id)?;
    let mut shares = read_public_shares(&project)?;
    let mut found = false;
    for share in &mut shares {
        if share.token == token {
            share.revoked_at_ms = Some(now_ms());
            found = true;
        }
    }
    if !found {
        return Err(ApiError::new(
            StatusCode::NOT_FOUND,
            "public share not found",
        ));
    }
    write_json_file(&mut project, PUBLIC_SHARES_PATH, &json!(shares))?;
    state.publish_data(
        Some(project_id),
        "public_share.revoked",
        None,
        None,
        Some(json!({ "token": token })),
    );
    Ok(Json(shares))
}

async fn public_file(
    State(state): State<DesignServerState>,
    AxumPath((project_id, token)): AxumPath<(String, String)>,
) -> std::result::Result<Response, ApiError> {
    let payload = verify_public_token(&token)?;
    if payload.project_id != project_id {
        return Err(ApiError::bad_request("public token project mismatch"));
    }
    reject_revoked_public_share(&state, &payload.project_id, &token)?;
    serve_public_rel(&state, &payload.project_id, &payload.path, &payload)
}

async fn public_share_entry(
    State(state): State<DesignServerState>,
    AxumPath(token): AxumPath<String>,
) -> std::result::Result<Response, ApiError> {
    let payload = verify_public_token(&token)?;
    reject_revoked_public_share(&state, &payload.project_id, &token)?;
    serve_public_rel(&state, &payload.project_id, &payload.path, &payload)
}

async fn public_share_file(
    State(state): State<DesignServerState>,
    AxumPath((token, path)): AxumPath<(String, String)>,
) -> std::result::Result<Response, ApiError> {
    let payload = verify_public_token(&token)?;
    reject_revoked_public_share(&state, &payload.project_id, &token)?;
    let rel = scoped_public_path(&payload, &path)?;
    serve_public_rel(&state, &payload.project_id, &rel, &payload)
}

fn project_response(project: DesignProject) -> ProjectResponse {
    ProjectResponse {
        root: project.root().display().to_string(),
        meta: project.meta().clone(),
    }
}

fn require_file_path(query: FileQuery) -> std::result::Result<String, ApiError> {
    query
        .path
        .filter(|p| !p.trim().is_empty())
        .ok_or_else(|| ApiError::bad_request("path query parameter is required"))
}

fn decode_content(
    content: String,
    encoding: Option<&str>,
) -> std::result::Result<Vec<u8>, ApiError> {
    match encoding.unwrap_or("utf-8") {
        "utf-8" => Ok(content.into_bytes()),
        "base64" => base64::engine::general_purpose::STANDARD
            .decode(content.as_bytes())
            .map_err(|e| ApiError::bad_request(format!("invalid base64 content: {e}"))),
        other => Err(ApiError::bad_request(format!(
            "unsupported content encoding: {other}"
        ))),
    }
}

fn serve_project_rel(
    state: &DesignServerState,
    project_id: &str,
    rel: &str,
) -> std::result::Result<Response, ApiError> {
    let project = state.store.open(project_id)?;
    let path = canonical_project_file(&project, rel)?;
    let mut bytes = std::fs::read(&path).map_err(|e| io_err(&path, e))?;
    let content_type = mime::guess(&path);
    if content_type.starts_with("text/html")
        && let Ok(html) = String::from_utf8(bytes.clone())
    {
        bytes = inject_preview_runtime(&html, project_id, rel, false)?.into_bytes();
    }
    let mut response = bytes.into_response();
    let header = HeaderValue::from_str(&content_type)
        .unwrap_or_else(|_| HeaderValue::from_static("application/octet-stream"));
    response.headers_mut().insert(CONTENT_TYPE, header);
    response.headers_mut().insert(
        axum::http::header::HeaderName::from_static("x-content-type-options"),
        HeaderValue::from_static("nosniff"),
    );
    response.headers_mut().insert(
        axum::http::header::CACHE_CONTROL,
        HeaderValue::from_static("no-store"),
    );
    Ok(response)
}

fn serve_public_rel(
    state: &DesignServerState,
    project_id: &str,
    rel: &str,
    payload: &PublicTokenPayload,
) -> std::result::Result<Response, ApiError> {
    let project = state.store.open(project_id)?;
    let path = canonical_project_file(&project, rel)?;
    let mut bytes = std::fs::read(&path).map_err(|e| io_err(&path, e))?;
    let content_type = mime::guess(&path);
    if content_type.starts_with("text/html")
        && let Ok(html) = String::from_utf8(bytes.clone())
    {
        bytes = inject_preview_runtime(&html, project_id, rel, true)?.into_bytes();
    }
    let mut response = bytes.into_response();
    let header = HeaderValue::from_str(&content_type)
        .unwrap_or_else(|_| HeaderValue::from_static("application/octet-stream"));
    response.headers_mut().insert(CONTENT_TYPE, header);
    response.headers_mut().insert(
        axum::http::header::HeaderName::from_static("x-content-type-options"),
        HeaderValue::from_static("nosniff"),
    );
    response.headers_mut().insert(
        axum::http::header::CACHE_CONTROL,
        HeaderValue::from_static("public, max-age=60"),
    );
    response.headers_mut().insert(
        axum::http::header::HeaderName::from_static("x-jfc-share-expires-at-ms"),
        HeaderValue::from_str(&payload.expires_at_ms.to_string())
            .unwrap_or_else(|_| HeaderValue::from_static("0")),
    );
    Ok(response)
}

fn sign_public_payload(payload: &PublicTokenPayload) -> std::result::Result<String, ApiError> {
    let json = serde_json::to_vec(payload).map_err(DesignError::from)?;
    let payload_b64 = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(json);
    let sig = public_signature(payload_b64.as_bytes())?;
    let sig_b64 = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(sig);
    Ok(format!("v1.{payload_b64}.{sig_b64}"))
}

fn verify_public_token(token: &str) -> std::result::Result<PublicTokenPayload, ApiError> {
    let parts = token.split('.').collect::<Vec<_>>();
    if parts.len() != 3 || parts[0] != "v1" {
        return Err(ApiError::bad_request("invalid public token format"));
    }
    let expected = public_signature(parts[1].as_bytes())?;
    let provided = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(parts[2].as_bytes())
        .map_err(|e| ApiError::bad_request(format!("invalid public token signature: {e}")))?;
    if !constant_time_eq(&expected, &provided) {
        return Err(ApiError::bad_request("invalid public token signature"));
    }
    let decoded = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(parts[1].as_bytes())
        .map_err(|e| ApiError::bad_request(format!("invalid public token payload: {e}")))?;
    let payload: PublicTokenPayload =
        serde_json::from_slice(&decoded).map_err(DesignError::from)?;
    if payload.expires_at_ms < now_ms() {
        return Err(ApiError::new(StatusCode::GONE, "public token expired"));
    }
    Ok(payload)
}

fn public_signature(payload: &[u8]) -> std::result::Result<Vec<u8>, ApiError> {
    let secret = std::env::var("JFC_DESIGN_SHARE_SECRET")
        .unwrap_or_else(|_| "jfc-design-local-dev-share-secret".to_owned());
    let mut mac = HmacSha256::new_from_slice(secret.as_bytes())
        .map_err(|e| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    mac.update(payload);
    Ok(mac.finalize().into_bytes().to_vec())
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (left, right) in a.iter().zip(b) {
        diff |= left ^ right;
    }
    diff == 0
}

fn scope_dir_for(path: &str) -> String {
    path.rsplit_once('/')
        .map(|(dir, _)| dir.to_owned())
        .unwrap_or_default()
}

fn scoped_public_path(
    payload: &PublicTokenPayload,
    requested: &str,
) -> std::result::Result<String, ApiError> {
    let clean = requested.trim_start_matches('/');
    if clean.is_empty() {
        return Ok(payload.path.clone());
    }
    if clean.contains("..") {
        return Err(ApiError::bad_request("public path escapes share scope"));
    }
    if payload.scope_dir.is_empty() {
        Ok(clean.to_owned())
    } else {
        Ok(format!("{}/{}", payload.scope_dir, clean))
    }
}

fn public_url_for(token: &str) -> String {
    let path = format!("/design/public/{}/", url_segment(token));
    std::env::var("JFC_DESIGN_PUBLIC_BASE_URL")
        .ok()
        .filter(|base| !base.trim().is_empty())
        .map(|base| format!("{}{}", base.trim_end_matches('/'), path))
        .unwrap_or(path)
}

fn read_public_shares(project: &DesignProject) -> DesignResult<Vec<PublicShareRecord>> {
    match project.read_file(PUBLIC_SHARES_PATH) {
        Ok(bytes) => Ok(serde_json::from_slice(&bytes)?),
        Err(DesignError::Io { source, .. }) if source.kind() == std::io::ErrorKind::NotFound => {
            Ok(Vec::new())
        }
        Err(e) => Err(e),
    }
}

fn upsert_public_share(project: &mut DesignProject, record: PublicShareRecord) -> DesignResult<()> {
    let mut shares = read_public_shares(project)?;
    shares.retain(|share| share.token != record.token);
    shares.push(record);
    shares.sort_by_key(|share| share.issued_at_ms);
    write_json_file(project, PUBLIC_SHARES_PATH, &json!(shares))
}

fn reject_revoked_public_share(
    state: &DesignServerState,
    project_id: &str,
    token: &str,
) -> std::result::Result<(), ApiError> {
    let project = state.store.open(project_id)?;
    let shares = read_public_shares(&project)?;
    if shares
        .iter()
        .any(|share| share.token == token && share.revoked_at_ms.is_some())
    {
        return Err(ApiError::new(StatusCode::GONE, "public share revoked"));
    }
    Ok(())
}

fn read_chat_messages(project: &DesignProject) -> DesignResult<Vec<ChatMessage>> {
    match project.read_file(CHAT_HISTORY_PATH) {
        Ok(bytes) => Ok(serde_json::from_slice(&bytes)?),
        Err(DesignError::Io { source, .. }) if source.kind() == std::io::ErrorKind::NotFound => {
            Ok(Vec::new())
        }
        Err(e) => Err(e),
    }
}

fn design_chat_reply(message: &str, path: Option<&str>) -> (String, Vec<String>) {
    let lower = message.to_ascii_lowercase();
    let mut actions = Vec::new();
    if lower.contains("verify") || lower.contains("done") {
        actions.push("verify_orchestrate".to_owned());
    }
    if lower.contains("screenshot") || lower.contains("shot") {
        actions.push("screenshot".to_owned());
    }
    if lower.contains("pdf") || lower.contains("print") {
        actions.push("save_pdf".to_owned());
    }
    if lower.contains("share") || lower.contains("public") || lower.contains("link") {
        actions.push("public_token".to_owned());
    }
    if lower.contains("bundle") || lower.contains("standalone") {
        actions.push("super_inline_html".to_owned());
    }
    if actions.is_empty() {
        actions.push("inspect".to_owned());
    }
    let target = path.unwrap_or("the active artifact");
    let reply = format!(
        "Queued design review context for {target}. Suggested actions: {}.",
        actions.join(", ")
    );
    (reply, actions)
}

fn inject_preview_runtime(
    html: &str,
    project_id: &str,
    path: &str,
    public: bool,
) -> DesignResult<String> {
    if html.contains("__jfc-design-preview-runtime") {
        return Ok(html.to_owned());
    }
    let config = json_for_script(&json!({
        "projectId": project_id,
        "path": path,
        "public": public,
    }))?;
    let runtime = include_str!("../assets/preview-runtime.js")
        .replace("__JFC_DESIGN_CONFIG__", &config)
        .replace("</script", "<\\/script");
    let block = format!("<script id=\"__jfc-design-preview-runtime\">\n{runtime}\n</script>\n");
    if let Some(pos) = html.rfind("</head>") {
        let mut out = html.to_owned();
        out.insert_str(pos, &block);
        return Ok(out);
    }
    if let Some(pos) = html.rfind("</body>") {
        let mut out = html.to_owned();
        out.insert_str(pos, &block);
        return Ok(out);
    }
    Ok(format!("{html}{block}"))
}

fn read_json_file_or_empty(project: &DesignProject, rel: &str) -> DesignResult<Value> {
    match project.read_file(rel) {
        Ok(bytes) => Ok(serde_json::from_slice(&bytes)?),
        Err(DesignError::Io { source, .. }) if source.kind() == std::io::ErrorKind::NotFound => {
            Ok(json!({}))
        }
        Err(e) => Err(e),
    }
}

fn write_json_file(project: &mut DesignProject, rel: &str, value: &Value) -> DesignResult<()> {
    let bytes = serde_json::to_vec_pretty(value)?;
    project.write_file(rel, &bytes, None)
}

#[derive(Debug)]
struct SourceRewrite {
    path: String,
    content: String,
}

#[derive(Debug, Clone)]
struct HtmlElement {
    tag: String,
    attrs: HashMap<String, String>,
    open_start: usize,
    open_end: usize,
    close_start: usize,
    close_end: usize,
    parent: Option<usize>,
    nth_of_type: usize,
}

#[derive(Debug)]
struct SelectorPart {
    tag: Option<String>,
    id: Option<String>,
    class_name: Option<String>,
    attr: Option<(String, String)>,
    nth_of_type: Option<usize>,
}

fn try_apply_direct_edit_source(
    project: &DesignProject,
    default_rel: &str,
    html: &str,
    edit: &Value,
) -> DesignResult<Option<SourceRewrite>> {
    if let Some(rewrite) = try_apply_explicit_source_range(project, default_rel, edit)? {
        return Ok(Some(rewrite));
    }
    if let Some(rewrite) = try_apply_sourcemap_source_hint(project, default_rel, edit)? {
        return Ok(Some(rewrite));
    }
    if let Some(rewrite) = try_apply_line_column_source_hint(project, default_rel, edit)? {
        return Ok(Some(rewrite));
    }
    if let Some(rewrite) = try_apply_unique_project_text_hint(project, default_rel, edit)? {
        return Ok(Some(rewrite));
    }
    let Some(selector) = edit.get("selector").and_then(Value::as_str) else {
        return Ok(None);
    };
    let Some(parts) = parse_selector_chain(selector) else {
        return Ok(None);
    };
    let elements = parse_html_elements(html);
    let matches = elements
        .iter()
        .enumerate()
        .filter(|(index, _)| selector_chain_matches(*index, &elements, &parts))
        .map(|(index, _)| index)
        .collect::<Vec<_>>();
    if matches.len() != 1 {
        return Ok(None);
    }
    let index = matches[0];
    if has_element_children(index, &elements)
        && (edit.get("text").is_some() || edit.get("html").is_some())
    {
        return Ok(None);
    }
    let content = apply_edit_to_element(html, &elements[index], edit)?;
    Ok(Some(SourceRewrite {
        path: default_rel.to_owned(),
        content,
    }))
}

fn try_apply_explicit_source_range(
    project: &DesignProject,
    default_rel: &str,
    edit: &Value,
) -> DesignResult<Option<SourceRewrite>> {
    let source_path = edit
        .get("source_path")
        .and_then(Value::as_str)
        .filter(|p| !p.trim().is_empty())
        .unwrap_or(default_rel);
    let Some(start) = edit.get("source_start").and_then(Value::as_u64) else {
        return Ok(None);
    };
    let Some(end) = edit.get("source_end").and_then(Value::as_u64) else {
        return Ok(None);
    };
    let start = start as usize;
    let end = end as usize;
    let mut content = project.read_to_string(source_path)?;
    if start > end
        || end > content.len()
        || !content.is_char_boundary(start)
        || !content.is_char_boundary(end)
    {
        return Ok(None);
    }
    let kind = edit
        .get("source_kind")
        .and_then(Value::as_str)
        .unwrap_or("text");
    let replacement = if kind == "html" {
        edit.get("html")
            .and_then(Value::as_str)
            .or_else(|| edit.get("text").and_then(Value::as_str))
            .unwrap_or_default()
            .to_owned()
    } else {
        escape_html_text(
            edit.get("text")
                .and_then(Value::as_str)
                .or_else(|| edit.get("html").and_then(Value::as_str))
                .unwrap_or_default(),
        )
    };
    content.replace_range(start..end, &replacement);
    Ok(Some(SourceRewrite {
        path: source_path.to_owned(),
        content,
    }))
}

fn try_apply_sourcemap_source_hint(
    project: &DesignProject,
    default_rel: &str,
    edit: &Value,
) -> DesignResult<Option<SourceRewrite>> {
    let Some(generated_path) = edit
        .get("generated_path")
        .and_then(Value::as_str)
        .filter(|path| !path.trim().is_empty())
    else {
        return Ok(None);
    };
    let Some(generated_line) = edit.get("generated_line").and_then(Value::as_u64) else {
        return Ok(None);
    };
    let generated_column = edit
        .get("generated_column")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let source_map_path = edit
        .get("source_map_path")
        .and_then(Value::as_str)
        .filter(|path| !path.trim().is_empty());
    let Some(mapped) = source_map::map_generated_position(
        project,
        generated_path,
        source_map_path,
        generated_line as usize,
        generated_column as usize,
    )?
    else {
        return Ok(None);
    };

    let mut mapped_edit = edit.clone();
    if let Some(obj) = mapped_edit.as_object_mut() {
        obj.insert(
            "source_path".to_owned(),
            Value::String(mapped.source_path.clone()),
        );
        obj.insert(
            "source_line".to_owned(),
            Value::Number(serde_json::Number::from(mapped.line)),
        );
        obj.insert(
            "source_column".to_owned(),
            Value::Number(serde_json::Number::from(mapped.column)),
        );
        obj.insert(
            "source_map_resolved".to_owned(),
            Value::String(generated_path.to_owned()),
        );
    }
    try_apply_line_column_source_hint(project, default_rel, &mapped_edit)
}

fn try_apply_line_column_source_hint(
    project: &DesignProject,
    default_rel: &str,
    edit: &Value,
) -> DesignResult<Option<SourceRewrite>> {
    let source_path = edit
        .get("source_path")
        .and_then(Value::as_str)
        .filter(|p| !p.trim().is_empty())
        .unwrap_or(default_rel);
    let Some(line) = edit.get("source_line").and_then(Value::as_u64) else {
        return Ok(None);
    };
    let Some(previous_text) = edit
        .get("previous_text")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return Ok(None);
    };
    let mut content = match project.read_to_string(source_path) {
        Ok(content) => content,
        Err(DesignError::PathEscape(_)) => return Ok(None),
        Err(DesignError::Io { source, .. }) if source.kind() == std::io::ErrorKind::NotFound => {
            return Ok(None);
        }
        Err(err) => return Err(err),
    };
    let line_index = usize::try_from(line.saturating_sub(1)).unwrap_or(usize::MAX);
    let offsets = line_start_offsets(&content);
    if line_index >= offsets.len().saturating_sub(1) {
        return Ok(None);
    }
    let window_start = offsets[line_index.saturating_sub(5)];
    let window_end = offsets
        .get((line_index + 6).min(offsets.len().saturating_sub(1)))
        .copied()
        .unwrap_or(content.len());
    let window = &content[window_start..window_end];
    let mut matches = Vec::new();
    let mut cursor = 0;
    while let Some(pos) = window[cursor..].find(previous_text) {
        let start = window_start + cursor + pos;
        matches.push((start, start + previous_text.len()));
        cursor += pos + previous_text.len();
    }
    if matches.len() != 1 {
        return Ok(None);
    }
    let kind = edit
        .get("source_kind")
        .and_then(Value::as_str)
        .unwrap_or("text");
    let replacement = if kind == "html" {
        edit.get("html")
            .and_then(Value::as_str)
            .or_else(|| edit.get("text").and_then(Value::as_str))
            .unwrap_or_default()
            .to_owned()
    } else {
        escape_html_text(
            edit.get("text")
                .and_then(Value::as_str)
                .or_else(|| edit.get("html").and_then(Value::as_str))
                .unwrap_or_default(),
        )
    };
    let (start, end) = matches[0];
    content.replace_range(start..end, &replacement);
    Ok(Some(SourceRewrite {
        path: source_path.to_owned(),
        content,
    }))
}

fn try_apply_unique_project_text_hint(
    project: &DesignProject,
    default_rel: &str,
    edit: &Value,
) -> DesignResult<Option<SourceRewrite>> {
    let Some(previous_text) = edit
        .get("previous_text")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| value.len() >= 3)
    else {
        return Ok(None);
    };
    let replacement = direct_edit_replacement(edit);
    let mut matches = Vec::new();
    for rel in project.list_files() {
        if !is_direct_edit_source_candidate(&rel) {
            continue;
        }
        let content = match project.read_to_string(&rel) {
            Ok(content) => content,
            Err(_) => continue,
        };
        let mut cursor = 0;
        while let Some(pos) = content[cursor..].find(previous_text) {
            let start = cursor + pos;
            matches.push((rel.clone(), start, start + previous_text.len()));
            cursor = start + previous_text.len();
            if matches.len() > 1 {
                return Ok(None);
            }
        }
    }
    let Some((path, start, end)) = matches.pop() else {
        return Ok(None);
    };
    let mut content = project.read_to_string(&path)?;
    content.replace_range(start..end, &replacement);
    Ok(Some(SourceRewrite {
        path: if path.is_empty() {
            default_rel.to_owned()
        } else {
            path
        },
        content,
    }))
}

fn is_direct_edit_source_candidate(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    !lower.starts_with("__")
        && matches!(
            lower.rsplit_once('.').map(|(_, ext)| ext),
            Some("html" | "htm" | "jsx" | "tsx" | "js" | "ts" | "css" | "md" | "mdx")
        )
}

fn direct_edit_replacement(edit: &Value) -> String {
    let kind = edit
        .get("source_kind")
        .and_then(Value::as_str)
        .unwrap_or("text");
    if kind == "html" {
        edit.get("html")
            .and_then(Value::as_str)
            .or_else(|| edit.get("text").and_then(Value::as_str))
            .unwrap_or_default()
            .to_owned()
    } else {
        escape_html_text(
            edit.get("text")
                .and_then(Value::as_str)
                .or_else(|| edit.get("html").and_then(Value::as_str))
                .unwrap_or_default(),
        )
    }
}

fn line_start_offsets(content: &str) -> Vec<usize> {
    let mut offsets = vec![0];
    for (index, byte) in content.bytes().enumerate() {
        if byte == b'\n' {
            offsets.push(index + 1);
        }
    }
    offsets.push(content.len());
    offsets
}

fn apply_edit_to_element(html: &str, element: &HtmlElement, edit: &Value) -> DesignResult<String> {
    let mut changes: Vec<(usize, usize, String)> = Vec::new();
    let mut attrs = element.attrs.clone();
    if let Some(new_attrs) = edit.get("attributes").and_then(Value::as_object) {
        for (name, value) in new_attrs {
            if !html_attr_name_is_safe(name) {
                continue;
            }
            if value.is_null() {
                attrs.remove(&name.to_ascii_lowercase());
            } else if let Some(value) = value.as_str() {
                attrs.insert(name.to_ascii_lowercase(), value.to_owned());
            } else {
                attrs.insert(name.to_ascii_lowercase(), value.to_string());
            }
        }
    }
    if let Some(styles) = edit.get("styles").and_then(Value::as_object) {
        let mut style_map = parse_style_attr(attrs.get("style").map(String::as_str).unwrap_or(""));
        for (name, value) in styles {
            if !css_property_name_is_safe(name) {
                continue;
            }
            if value.is_null() {
                style_map.remove(name);
            } else {
                let css_value = value
                    .as_str()
                    .map(str::to_owned)
                    .unwrap_or_else(|| value.to_string());
                if !css_value.bytes().any(|b| matches!(b, b';' | b'{' | b'}')) {
                    style_map.insert(name.to_owned(), css_value);
                }
            }
        }
        let style = style_map
            .iter()
            .map(|(name, value)| format!("{name}: {value}"))
            .collect::<Vec<_>>()
            .join("; ");
        if style.is_empty() {
            attrs.remove("style");
        } else {
            attrs.insert("style".to_owned(), style);
        }
    }
    if attrs != element.attrs {
        changes.push((
            element.open_start,
            element.open_end,
            render_open_tag(&element.tag, &attrs),
        ));
    }
    if let Some(raw_html) = edit.get("html").and_then(Value::as_str) {
        changes.push((element.open_end, element.close_start, raw_html.to_owned()));
    } else if let Some(text) = edit.get("text").and_then(Value::as_str) {
        changes.push((
            element.open_end,
            element.close_start,
            escape_html_text(text),
        ));
    }
    if changes.is_empty() {
        return Ok(html.to_owned());
    }
    changes.sort_by_key(|(start, _, _)| *start);
    let mut out = html.to_owned();
    for (start, end, replacement) in changes.into_iter().rev() {
        out.replace_range(start..end, &replacement);
    }
    Ok(out)
}

fn parse_html_elements(html: &str) -> Vec<HtmlElement> {
    let mut out: Vec<HtmlElement> = Vec::new();
    let mut stack: Vec<usize> = Vec::new();
    let mut pos = 0;
    while let Some(rel) = html[pos..].find('<') {
        let start = pos + rel;
        if html[start..].starts_with("<!--") {
            if let Some(end_rel) = html[start + 4..].find("-->") {
                pos = start + 4 + end_rel + 3;
            } else {
                break;
            }
            continue;
        }
        let Some(end_rel) = html[start..].find('>') else {
            break;
        };
        let end = start + end_rel + 1;
        let raw = &html[start + 1..end - 1];
        let trimmed = raw.trim();
        if trimmed.is_empty() || trimmed.starts_with('!') || trimmed.starts_with('?') {
            pos = end;
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix('/') {
            let name = rest
                .split_whitespace()
                .next()
                .unwrap_or_default()
                .to_ascii_lowercase();
            while let Some(index) = stack.pop() {
                if out[index].close_start == out[index].open_end {
                    out[index].close_start = start;
                    out[index].close_end = end;
                }
                if out[index].tag == name {
                    break;
                }
            }
            pos = end;
            continue;
        }
        let self_closing = trimmed.ends_with('/') || is_void_html_tag(trimmed);
        let (tag, attrs) = parse_open_tag(trimmed);
        if tag.is_empty() {
            pos = end;
            continue;
        }
        let parent = stack.last().copied();
        let nth_of_type = parent
            .map(|p| {
                out.iter()
                    .filter(|el: &&HtmlElement| el.parent == Some(p) && el.tag == tag)
                    .count()
                    + 1
            })
            .unwrap_or_else(|| {
                out.iter()
                    .filter(|el: &&HtmlElement| el.parent.is_none() && el.tag == tag)
                    .count()
                    + 1
            });
        let index = out.len();
        out.push(HtmlElement {
            tag: tag.clone(),
            attrs,
            open_start: start,
            open_end: end,
            close_start: end,
            close_end: end,
            parent,
            nth_of_type,
        });
        if !self_closing {
            stack.push(index);
        }
        if matches!(tag.as_str(), "script" | "style" | "textarea") {
            if let Some(close_rel) = html[end..].to_ascii_lowercase().find(&format!("</{tag}>")) {
                let close_start = end + close_rel;
                let close_end = close_start + tag.len() + 3;
                out[index].close_start = close_start;
                out[index].close_end = close_end;
                stack.retain(|i| *i != index);
                pos = close_end;
                continue;
            }
        }
        pos = end;
    }
    out
}

fn parse_open_tag(raw: &str) -> (String, HashMap<String, String>) {
    let mut i = 0;
    let bytes = raw.as_bytes();
    while i < bytes.len() && bytes[i].is_ascii_whitespace() {
        i += 1;
    }
    let tag_start = i;
    while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || matches!(bytes[i], b'-' | b':')) {
        i += 1;
    }
    let tag = raw[tag_start..i].to_ascii_lowercase();
    let mut attrs = HashMap::new();
    while i < bytes.len() {
        while i < bytes.len() && (bytes[i].is_ascii_whitespace() || bytes[i] == b'/') {
            i += 1;
        }
        let name_start = i;
        while i < bytes.len()
            && (bytes[i].is_ascii_alphanumeric() || matches!(bytes[i], b'-' | b':' | b'_'))
        {
            i += 1;
        }
        if name_start == i {
            break;
        }
        let name = raw[name_start..i].to_ascii_lowercase();
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        let mut value = String::new();
        if i < bytes.len() && bytes[i] == b'=' {
            i += 1;
            while i < bytes.len() && bytes[i].is_ascii_whitespace() {
                i += 1;
            }
            if i < bytes.len() && (bytes[i] == b'"' || bytes[i] == b'\'') {
                let quote = bytes[i];
                i += 1;
                let value_start = i;
                while i < bytes.len() && bytes[i] != quote {
                    i += 1;
                }
                value = html_unescape(&raw[value_start..i]);
                if i < bytes.len() {
                    i += 1;
                }
            } else {
                let value_start = i;
                while i < bytes.len() && !bytes[i].is_ascii_whitespace() && bytes[i] != b'/' {
                    i += 1;
                }
                value = html_unescape(&raw[value_start..i]);
            }
        }
        attrs.insert(name, value);
    }
    (tag, attrs)
}

fn parse_selector_chain(selector: &str) -> Option<Vec<SelectorPart>> {
    let mut parts = Vec::new();
    for raw in selector.split('>').map(str::trim).filter(|s| !s.is_empty()) {
        parts.push(parse_selector_part(raw)?);
    }
    if parts.is_empty() { None } else { Some(parts) }
}

fn parse_selector_part(raw: &str) -> Option<SelectorPart> {
    let mut input = raw.trim();
    let mut nth_of_type = None;
    if let Some(pos) = input.find(":nth-of-type(") {
        let rest = &input[pos + ":nth-of-type(".len()..];
        let end = rest.find(')')?;
        nth_of_type = rest[..end].trim().parse::<usize>().ok();
        input = &input[..pos];
    }
    let mut attr = None;
    if let Some(start) = input.find('[') {
        let end = input[start..].find(']')? + start;
        let body = &input[start + 1..end];
        if let Some((name, value)) = body.split_once('=') {
            attr = Some((
                name.trim().to_ascii_lowercase(),
                value.trim().trim_matches('"').trim_matches('\'').to_owned(),
            ));
        }
        input = input[..start].trim();
    }
    let mut tag = None;
    let mut id = None;
    let mut class_name = None;
    let mut rest = input;
    if let Some(hash) = rest.find('#') {
        let before = rest[..hash].trim();
        if !before.is_empty() {
            tag = Some(before.to_ascii_lowercase());
        }
        rest = &rest[hash + 1..];
        let end = rest.find('.').unwrap_or(rest.len());
        id = Some(rest[..end].to_owned());
        rest = &rest[end..];
    } else if let Some(dot) = rest.find('.') {
        let before = rest[..dot].trim();
        if !before.is_empty() {
            tag = Some(before.to_ascii_lowercase());
        }
        rest = &rest[dot..];
    } else if !rest.trim().is_empty() {
        tag = Some(rest.trim().to_ascii_lowercase());
        rest = "";
    }
    if let Some(dot) = rest.find('.') {
        let cls = &rest[dot + 1..];
        if !cls.is_empty() {
            class_name = Some(cls.to_owned());
        }
    }
    Some(SelectorPart {
        tag,
        id,
        class_name,
        attr,
        nth_of_type,
    })
}

fn selector_chain_matches(index: usize, elements: &[HtmlElement], parts: &[SelectorPart]) -> bool {
    let mut current = Some(index);
    for part in parts.iter().rev() {
        let Some(i) = current else {
            return false;
        };
        if !selector_part_matches(&elements[i], part) {
            return false;
        }
        current = elements[i].parent;
    }
    true
}

fn selector_part_matches(element: &HtmlElement, part: &SelectorPart) -> bool {
    if let Some(tag) = &part.tag
        && &element.tag != tag
    {
        return false;
    }
    if let Some(id) = &part.id
        && element.attrs.get("id") != Some(id)
    {
        return false;
    }
    if let Some(class_name) = &part.class_name {
        let classes = element.attrs.get("class").map(String::as_str).unwrap_or("");
        if !classes.split_whitespace().any(|cls| cls == class_name) {
            return false;
        }
    }
    if let Some((name, value)) = &part.attr
        && element.attrs.get(name) != Some(value)
    {
        return false;
    }
    if let Some(n) = part.nth_of_type
        && element.nth_of_type != n
    {
        return false;
    }
    true
}

fn has_element_children(index: usize, elements: &[HtmlElement]) -> bool {
    elements.iter().any(|element| element.parent == Some(index))
}

fn render_open_tag(tag: &str, attrs: &HashMap<String, String>) -> String {
    let mut out = format!("<{tag}");
    let mut keys = attrs.keys().collect::<Vec<_>>();
    keys.sort();
    for key in keys {
        if !html_attr_name_is_safe(key) {
            continue;
        }
        out.push(' ');
        out.push_str(key);
        out.push_str("=\"");
        out.push_str(&escape_html_attr(
            attrs.get(key).map(String::as_str).unwrap_or(""),
        ));
        out.push('"');
    }
    out.push('>');
    out
}

fn parse_style_attr(style: &str) -> HashMap<String, String> {
    let mut out = HashMap::new();
    for declaration in style.split(';') {
        if let Some((name, value)) = declaration.split_once(':') {
            let name = name.trim();
            let value = value.trim();
            if css_property_name_is_safe(name) && !value.is_empty() {
                out.insert(name.to_owned(), value.to_owned());
            }
        }
    }
    out
}

fn html_attr_name_is_safe(name: &str) -> bool {
    !name.is_empty()
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b':' | b'_'))
}

fn escape_html_text(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn escape_html_attr(value: &str) -> String {
    escape_html_text(value).replace('"', "&quot;")
}

fn html_unescape(value: &str) -> String {
    value
        .replace("&quot;", "\"")
        .replace("&#34;", "\"")
        .replace("&apos;", "'")
        .replace("&#39;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
}

fn is_void_html_tag(raw: &str) -> bool {
    let tag = raw
        .split_whitespace()
        .next()
        .unwrap_or_default()
        .trim_end_matches('/')
        .to_ascii_lowercase();
    matches!(
        tag.as_str(),
        "area"
            | "base"
            | "br"
            | "col"
            | "embed"
            | "hr"
            | "img"
            | "input"
            | "link"
            | "meta"
            | "param"
            | "source"
            | "track"
            | "wbr"
    )
}

fn append_direct_edit_log(
    project: &mut DesignProject,
    edit: &Value,
    runtime: &str,
    source_path: &Option<String>,
) -> DesignResult<()> {
    let mut log = read_json_file_or_empty(project, DIRECT_EDIT_LOG_PATH)?;
    if !log.is_object() {
        log = json!({});
    }
    if let Some(obj) = log.as_object_mut() {
        let entries = obj.entry("edits").or_insert_with(|| json!([]));
        if !entries.is_array() {
            *entries = json!([]);
        }
        if let Some(entries) = entries.as_array_mut() {
            entries.push(json!({
                "edit": edit,
                "runtime": runtime,
                "source_path": source_path,
                "updated_at_ms": now_ms(),
            }));
            if entries.len() > 200 {
                entries.drain(0..entries.len() - 200);
            }
        }
    }
    write_json_file(project, DIRECT_EDIT_LOG_PATH, &log)
}

fn merge_direct_edit_override(overrides: &mut Value, edit: Value) {
    if !overrides.is_object() {
        *overrides = json!({});
    }
    let Some(obj) = overrides.as_object_mut() else {
        return;
    };
    let selector = edit
        .get("selector")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    let entry = obj.entry("edits").or_insert_with(|| json!([]));
    if !entry.is_array() {
        *entry = json!([]);
    }
    let Some(edits) = entry.as_array_mut() else {
        return;
    };
    if let Some(existing) = edits.iter_mut().find(|item| {
        item.get("selector")
            .and_then(Value::as_str)
            .is_some_and(|s| s == selector)
    }) {
        *existing = edit;
    } else {
        edits.push(edit);
    }
}

fn apply_direct_edit_overlay(html: &str, overrides: &Value) -> DesignResult<String> {
    const START: &str = "<!-- jfc-direct-edit-overrides:start -->";
    const END: &str = "<!-- jfc-direct-edit-overrides:end -->";

    let mut cleaned = html.to_owned();
    if let Some(start) = cleaned.find(START)
        && let Some(end_rel) = cleaned[start..].find(END)
    {
        let end = start + end_rel + END.len();
        cleaned.replace_range(start..end, "");
    }

    let json = json_for_script(overrides)?;
    let css = direct_edit_css(overrides);
    let block = format!(
        r#"{START}
<style id="__om-edit-overrides">
{css}
</style>
<script id="__om-direct-edit-runtime">
(() => {{
  const state = {json};
  const edits = Array.isArray(state.edits) ? state.edits : [];
  const apply = () => {{
    for (const edit of edits) {{
      if (!edit || !edit.selector) continue;
      let nodes = [];
      try {{ nodes = Array.from(document.querySelectorAll(edit.selector)); }} catch {{ continue; }}
      for (const node of nodes) {{
        if (edit.text !== null && edit.text !== undefined) node.textContent = String(edit.text);
        if (edit.html !== null && edit.html !== undefined) node.innerHTML = String(edit.html);
        if (edit.attributes && typeof edit.attributes === 'object') {{
          for (const [name, value] of Object.entries(edit.attributes)) {{
            if (value === null || value === undefined) node.removeAttribute(name);
            else node.setAttribute(name, String(value));
          }}
        }}
        if (edit.styles && typeof edit.styles === 'object' && node.style) {{
          for (const [name, value] of Object.entries(edit.styles)) {{
            if (value === null || value === undefined) node.style.removeProperty(name);
            else node.style.setProperty(name, String(value));
          }}
        }}
      }}
    }}
  }};
  window.__jfcDirectEdits = {{ edits, apply }};
  if (document.readyState === 'loading') document.addEventListener('DOMContentLoaded', apply, {{ once: true }});
  else apply();
}})();
</script>
{END}
"#
    );

    if let Some(pos) = cleaned.rfind("</head>") {
        cleaned.insert_str(pos, &block);
        return Ok(cleaned);
    }
    if let Some(pos) = cleaned.rfind("</body>") {
        cleaned.insert_str(pos, &block);
        return Ok(cleaned);
    }
    cleaned.push_str(&block);
    Ok(cleaned)
}

fn json_for_script(value: &Value) -> DesignResult<String> {
    Ok(serde_json::to_string(value)?.replace("</script", "<\\/script"))
}

fn direct_edit_css(overrides: &Value) -> String {
    let Some(edits) = overrides.get("edits").and_then(Value::as_array) else {
        return String::new();
    };
    let mut out = String::new();
    for edit in edits {
        let Some(selector) = edit.get("selector").and_then(Value::as_str) else {
            continue;
        };
        if !selector_is_safe_for_css(selector) {
            continue;
        }
        let Some(styles) = edit.get("styles").and_then(Value::as_object) else {
            continue;
        };
        let mut declarations = Vec::new();
        for (name, value) in styles {
            if !css_property_name_is_safe(name) {
                continue;
            }
            let Some(value) = value.as_str().or_else(|| {
                if value.is_number() {
                    value.as_f64().map(|_| "")
                } else {
                    None
                }
            }) else {
                continue;
            };
            let css_value = if value.is_empty() {
                styles
                    .get(name)
                    .map(Value::to_string)
                    .unwrap_or_default()
                    .trim_matches('"')
                    .to_owned()
            } else {
                value.to_owned()
            };
            if css_value.bytes().any(|b| matches!(b, b';' | b'{' | b'}')) {
                continue;
            }
            declarations.push(format!("{name}: {css_value} !important;"));
        }
        if !declarations.is_empty() {
            out.push_str(selector);
            out.push_str(" {\n  ");
            out.push_str(&declarations.join("\n  "));
            out.push_str("\n}\n");
        }
    }
    out
}

fn selector_is_safe_for_css(selector: &str) -> bool {
    !selector
        .bytes()
        .any(|b| matches!(b, b'{' | b'}' | b'<' | b'>'))
}

fn css_property_name_is_safe(name: &str) -> bool {
    !name.is_empty()
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
}

fn canonical_project_file(
    project: &DesignProject,
    rel: &str,
) -> std::result::Result<PathBuf, ApiError> {
    let mut path = project.resolve(rel)?;
    if path.is_dir() {
        path.push("index.html");
    }
    let canonical = path.canonicalize().map_err(|e| io_err(&path, e))?;
    let root = project
        .root()
        .canonicalize()
        .map_err(|e| io_err(project.root(), e))?;
    if !canonical.starts_with(&root) {
        return Err(DesignError::PathEscape(rel.to_owned()).into());
    }
    Ok(canonical)
}

fn path_relative_to(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

fn default_capture_path(input_rel: &str, dir: &str, ext: &str) -> String {
    let stem = input_rel
        .rsplit('/')
        .next()
        .unwrap_or("artifact")
        .rsplit_once('.')
        .map(|(base, _)| base)
        .unwrap_or("artifact");
    let safe = stem
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect::<String>()
        .trim_matches('-')
        .to_owned();
    format!(
        "{dir}/{}.{}",
        if safe.is_empty() { "artifact" } else { &safe },
        ext
    )
}

fn default_capture_dir(input_rel: &str, dir: &str) -> String {
    let stem = input_rel
        .rsplit('/')
        .next()
        .unwrap_or("artifact")
        .rsplit_once('.')
        .map(|(base, _)| base)
        .unwrap_or("artifact");
    let safe = stem
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect::<String>()
        .trim_matches('-')
        .to_owned();
    format!("{dir}/{}", if safe.is_empty() { "artifact" } else { &safe })
}

fn add_response_path(response: &mut Value, path: &str) {
    if let Some(obj) = response.as_object_mut() {
        obj.insert("path".to_owned(), Value::String(path.to_owned()));
    }
}

fn add_response_output(response: &mut Value, output: &str) {
    if let Some(obj) = response.as_object_mut() {
        obj.insert("output".to_owned(), Value::String(output.to_owned()));
    }
}

fn url_segment(value: &str) -> String {
    value
        .bytes()
        .flat_map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                vec![byte as char]
            }
            other => format!("%{other:02X}").chars().collect(),
        })
        .collect()
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_project() -> (PathBuf, DesignProject) {
        let n = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("jfc_design_api_test_{n}"));
        std::fs::create_dir_all(&root).unwrap();
        (root.clone(), DesignProject::at(root))
    }

    #[test]
    fn decode_content_accepts_utf8_and_base64_normal() {
        assert_eq!(
            decode_content("hello".to_owned(), None).unwrap(),
            b"hello".to_vec()
        );
        assert_eq!(
            decode_content("aGVsbG8=".to_owned(), Some("base64")).unwrap(),
            b"hello".to_vec()
        );
    }

    #[test]
    fn decode_content_rejects_unknown_encoding_robust() {
        let err = decode_content("hello".to_owned(), Some("gzip")).unwrap_err();
        assert_eq!(err.status, StatusCode::BAD_REQUEST);
    }

    #[test]
    fn default_capture_path_is_sandbox_relative_normal() {
        assert_eq!(
            default_capture_path("slides/main deck.html", "exports", "pptx"),
            "exports/main-deck.pptx"
        );
    }

    #[test]
    fn default_capture_dir_is_sandbox_relative_normal() {
        assert_eq!(
            default_capture_dir("slides/main deck.html", "screenshots"),
            "screenshots/main-deck"
        );
    }

    #[test]
    fn direct_edit_overlay_replaces_previous_block_normal() {
        let mut overrides = json!({});
        merge_direct_edit_override(
            &mut overrides,
            json!({
                "selector": "h1",
                "text": "Hello",
                "styles": { "color": "#123456" },
                "attributes": { "data-edited": "yes" }
            }),
        );
        let html = "<!doctype html><html><head></head><body><h1>Old</h1></body></html>";
        let once = apply_direct_edit_overlay(html, &overrides).unwrap();
        let twice = apply_direct_edit_overlay(&once, &overrides).unwrap();
        assert!(twice.contains("__om-direct-edit-runtime"));
        assert!(twice.contains("h1 {\n  color: #123456 !important;"));
        assert_eq!(twice.matches("jfc-direct-edit-overrides:start").count(), 1);
    }

    #[test]
    fn direct_edit_css_skips_unsafe_values_robust() {
        let overrides = json!({
            "edits": [
                { "selector": "h1", "styles": { "color": "red; body { display: none }", "font-weight": "700" } },
                { "selector": "h2{}", "styles": { "color": "blue" } }
            ]
        });
        let css = direct_edit_css(&overrides);
        assert!(css.contains("font-weight: 700 !important"));
        assert!(!css.contains("display: none"));
        assert!(!css.contains("h2{}"));
    }

    #[test]
    fn direct_edit_source_rewrites_static_leaf_normal() {
        let (root, project) = tmp_project();
        let html = r#"<main><h1 id="hero">Old</h1><p>Copy</p></main>"#;
        let edit = json!({
            "selector": "#hero",
            "text": "New & safe",
            "styles": { "color": "#123456" }
        });
        let rewrite = try_apply_direct_edit_source(&project, "index.html", html, &edit)
            .unwrap()
            .unwrap();
        assert_eq!(rewrite.path, "index.html");
        assert!(rewrite.content.contains(">New &amp; safe</h1>"));
        assert!(rewrite.content.contains(r#"id="hero""#));
        assert!(rewrite.content.contains(r#"style="color: #123456""#));
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn direct_edit_source_uses_explicit_byte_range_normal() {
        let (root, mut project) = tmp_project();
        let html = r#"<main><h1>Old</h1></main>"#;
        project
            .write_file("index.html", html.as_bytes(), None)
            .unwrap();
        let start = html.find("Old").unwrap();
        let edit = json!({
            "source_path": "index.html",
            "source_start": start,
            "source_end": start + 3,
            "source_kind": "text",
            "text": "New & safe"
        });
        let rewrite = try_apply_direct_edit_source(&project, "index.html", html, &edit)
            .unwrap()
            .unwrap();
        assert_eq!(rewrite.content, r#"<main><h1>New &amp; safe</h1></main>"#);
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn direct_edit_source_uses_unique_line_hint_normal() {
        let (root, mut project) = tmp_project();
        let html = "<main>\n  <h1>Old headline</h1>\n  <p>Copy</p>\n</main>";
        project
            .write_file("index.html", html.as_bytes(), None)
            .unwrap();
        let edit = json!({
            "source_path": "index.html",
            "source_line": 2,
            "source_column": 7,
            "previous_text": "Old headline",
            "text": "New & safe"
        });
        let rewrite = try_apply_direct_edit_source(&project, "index.html", html, &edit)
            .unwrap()
            .unwrap();
        assert!(rewrite.content.contains("<h1>New &amp; safe</h1>"));
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn direct_edit_source_uses_sourcemap_generated_hint_normal() {
        let (root, mut project) = tmp_project();
        let source = "export function App(){return <h1>Old headline</h1>}\n";
        let bundle =
            "React.createElement('h1', null, 'Old headline');\n//# sourceMappingURL=app.js.map\n";
        let map = json!({
            "version": 3,
            "sources": ["src/App.jsx"],
            "mappings": "AAAA"
        });
        project
            .write_file("src/App.jsx", source.as_bytes(), None)
            .unwrap();
        project
            .write_file("dist/app.js", bundle.as_bytes(), None)
            .unwrap();
        project
            .write_file("dist/app.js.map", map.to_string().as_bytes(), None)
            .unwrap();
        let edit = json!({
            "generated_path": "dist/app.js",
            "generated_line": 1,
            "generated_column": 0,
            "previous_text": "Old headline",
            "text": "New & safe"
        });
        let rewrite = try_apply_direct_edit_source(&project, "index.html", "", &edit)
            .unwrap()
            .unwrap();
        assert_eq!(rewrite.path, "src/App.jsx");
        assert!(rewrite.content.contains("<h1>New &amp; safe</h1>"));
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn direct_edit_source_uses_unique_project_text_without_metadata_normal() {
        let (root, mut project) = tmp_project();
        project
            .write_file(
                "src/App.jsx",
                b"export function App(){return <h1>Only headline</h1>}\n",
                None,
            )
            .unwrap();
        project
            .write_file("dist/app.js", b"minified bundle without maps", None)
            .unwrap();
        let edit = json!({
            "selector": "h1",
            "previous_text": "Only headline",
            "text": "Better headline"
        });
        let rewrite = try_apply_direct_edit_source(&project, "index.html", "", &edit)
            .unwrap()
            .unwrap();
        assert_eq!(rewrite.path, "src/App.jsx");
        assert!(rewrite.content.contains("<h1>Better headline</h1>"));
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn public_token_round_trips_and_rejects_tampering_normal() {
        let payload = PublicTokenPayload {
            project_id: "project-a".to_owned(),
            path: "index.html".to_owned(),
            issued_at_ms: now_ms(),
            expires_at_ms: now_ms() + 60_000,
            scope_dir: String::new(),
        };
        let token = sign_public_payload(&payload).unwrap();
        let decoded = verify_public_token(&token).unwrap();
        assert_eq!(decoded.project_id, "project-a");
        assert_eq!(decoded.path, "index.html");
        assert!(verify_public_token(&(token + "x")).is_err());
    }

    #[test]
    fn public_share_manifest_upserts_and_marks_revoked_normal() {
        let (root, mut project) = tmp_project();
        let record = PublicShareRecord {
            path: "index.html".to_owned(),
            token: "token-a".to_owned(),
            public_url: "/design/public/token-a/".to_owned(),
            embed_url: "/design/public/token-a/".to_owned(),
            issued_at_ms: 10,
            expires_at_ms: 20,
            scope_dir: String::new(),
            title: Some("Demo".to_owned()),
            allow_download: true,
            revoked_at_ms: None,
        };
        upsert_public_share(&mut project, record).unwrap();
        let mut shares = read_public_shares(&project).unwrap();
        assert_eq!(shares.len(), 1);
        shares[0].revoked_at_ms = Some(30);
        write_json_file(&mut project, PUBLIC_SHARES_PATH, &json!(shares)).unwrap();
        let shares = read_public_shares(&project).unwrap();
        assert_eq!(shares[0].revoked_at_ms, Some(30));
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn design_chat_reply_routes_actions_normal() {
        let (reply, actions) = design_chat_reply("verify and make a pdf link", Some("index.html"));
        assert!(reply.contains("index.html"));
        assert!(actions.contains(&"verify_orchestrate".to_owned()));
        assert!(actions.contains(&"save_pdf".to_owned()));
        assert!(actions.contains(&"public_token".to_owned()));
    }

    #[test]
    fn preview_runtime_injection_is_idempotent_normal() {
        let html = "<!doctype html><html><head><title>x</title></head><body></body></html>";
        let injected = inject_preview_runtime(html, "project-a", "index.html", false).unwrap();
        let reinjected =
            inject_preview_runtime(&injected, "project-a", "index.html", false).unwrap();
        assert!(injected.contains("__jfc-design-preview-runtime"));
        assert!(injected.contains("__OM_MSG__"));
        assert_eq!(injected, reinjected);
    }
}

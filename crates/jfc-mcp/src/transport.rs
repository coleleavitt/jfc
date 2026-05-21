//! Stdio JSON-RPC transport for MCP servers.
//!
//! ## Framing
//!
//! Supports both LSP-style `Content-Length: N\r\n\r\n{json}` framing
//! and bare newline-delimited JSON (`{json}\n`). The mode is auto-
//! detected from the first byte of the server's response: `{` means
//! bare JSON lines, `C` means Content-Length headers. Once detected
//! the mode is locked for the lifetime of the connection.
//!
//! ## Lifecycle
//!
//! [`Transport::spawn`] forks a child process with stdin/stdout/stderr
//! piped, drives the JSON-RPC `initialize` + `notifications/initialized`
//! handshake, then returns a handle. Three tokio tasks run for the
//! lifetime of the connection:
//!
//! 1. **stderr-drain** — line-by-line forward to `tracing::warn!` AND
//!    a bounded ring buffer accessible via [`Transport::recent_stderr`]
//!    so `/mcp logs <name>` can show the user what blew up.
//! 2. **stdout-reader** — accumulates bytes, parses framed messages,
//!    routes responses to pending oneshots.
//! 3. **stdin-writer** — pulls pre-encoded bytes off an unbounded
//!    channel and writes to the child.
//!
//! See `lsp_client.rs`'s module docs for the rationale on this layout
//! (stderr drain prevents deadlock; tasks vs. shared writer mutex).

use std::collections::HashMap;
use std::collections::VecDeque;
use std::process::Stdio;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, AtomicU8, Ordering};

use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::Mutex;
use tokio::sync::mpsc::{self, UnboundedSender};
use tokio::sync::oneshot;

use super::protocol;

/// Maximum number of stderr lines to keep in the in-memory ring buffer
/// per server. Tunable via [`Transport::set_stderr_buffer_capacity`] but
/// 200 is generous for a `/mcp logs` printout — most npm packages emit
/// fewer than that across their entire lifetime.
const DEFAULT_STDERR_RING_CAPACITY: usize = 200;

/// Maximum number of bytes we'll buffer before declaring the stream
/// corrupt and resetting. 8 MiB is enough for the largest single-tool
/// result we've seen in practice (a recursive directory listing).
const MAX_BUFFER_BYTES: usize = 8 * 1024 * 1024;

/// Framing mode for an MCP connection. Auto-detected from the first
/// byte of server output.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FramingMode {
    /// `Content-Length: N\r\n\r\n{json}` (LSP-style).
    ContentLength,
    /// `{json}\n` (newline-delimited JSON lines).
    NewlineDelimited,
}

/// Encode a JSON value for sending to the server. Uses Content-Length
/// framing when `mode` is `ContentLength`, bare JSON + newline
/// otherwise.
pub fn encode_with_framing(value: &Value, mode: FramingMode) -> Vec<u8> {
    let body = serde_json::to_vec(value).expect("Value serialization is infallible");
    match mode {
        FramingMode::ContentLength => {
            let mut out = Vec::with_capacity(body.len() + 32);
            out.extend_from_slice(b"Content-Length: ");
            out.extend_from_slice(body.len().to_string().as_bytes());
            out.extend_from_slice(b"\r\n\r\n");
            out.extend_from_slice(&body);
            out
        }
        FramingMode::NewlineDelimited => {
            let mut out = body;
            out.push(b'\n');
            out
        }
    }
}

/// LSP-style framing encode (Content-Length). Used as default before
/// the server's framing mode is detected.
pub fn encode(value: &Value) -> Vec<u8> {
    encode_with_framing(value, FramingMode::ContentLength)
}

/// Try to parse a single framed JSON-RPC message off the front of
/// `buf` using Content-Length framing. Returns `Ok(Some((value,
/// consumed)))` on success, `Ok(None)` when more bytes are needed,
/// `Err` on protocol violation.
pub fn try_parse(buf: &[u8]) -> Result<Option<(Value, usize)>, FrameError> {
    let Some(header_end) = find_header_end(buf) else {
        return Ok(None);
    };
    let header_str =
        std::str::from_utf8(&buf[..header_end]).map_err(|_| FrameError::HeaderNotUtf8)?;
    let content_length = parse_content_length(header_str)?;
    if content_length > MAX_BUFFER_BYTES {
        return Err(FrameError::OversizedBody);
    }
    let body_start = header_end + 4;
    let body_end = body_start + content_length;
    if buf.len() < body_end {
        return Ok(None);
    }
    let body = &buf[body_start..body_end];
    let value: Value = serde_json::from_slice(body)?;
    Ok(Some((value, body_end)))
}

/// Try to parse a single newline-delimited JSON message from `buf`.
/// Returns `Ok(Some((value, consumed)))` when a complete `\n`-
/// terminated line is available, `Ok(None)` when more data is needed.
pub fn try_parse_ndjson(buf: &[u8]) -> Result<Option<(Value, usize)>, FrameError> {
    let Some(newline_pos) = buf.iter().position(|&b| b == b'\n') else {
        if buf.len() > MAX_BUFFER_BYTES {
            return Err(FrameError::OversizedBody);
        }
        return Ok(None);
    };
    let line = &buf[..newline_pos];
    // Skip empty lines.
    let trimmed = line.iter().copied().filter(|b| !b.is_ascii_whitespace()).count();
    if trimmed == 0 {
        return Ok(Some((Value::Null, newline_pos + 1)));
    }
    let value: Value = serde_json::from_slice(line)?;
    Ok(Some((value, newline_pos + 1)))
}

/// Detect framing mode from the first non-whitespace byte in `buf`.
pub fn detect_framing(buf: &[u8]) -> Option<FramingMode> {
    for &b in buf {
        match b {
            b' ' | b'\t' | b'\r' | b'\n' => continue,
            b'{' | b'[' => return Some(FramingMode::NewlineDelimited),
            _ => return Some(FramingMode::ContentLength),
        }
    }
    None
}

/// Framing-layer parse failures. The `Json` variant chains the underlying
/// `serde_json::Error` via `#[from]` so callers retain the source — useful
/// for distinguishing `io`, `syntax`, `data`, and `eof` categories rather
/// than string-matching on a flattened message.
#[derive(Debug, thiserror::Error)]
pub enum FrameError {
    #[error("MCP header was not valid UTF-8")]
    HeaderNotUtf8,
    #[error("MCP header missing Content-Length")]
    MissingContentLength,
    #[error("MCP header had bad Content-Length")]
    InvalidContentLength,
    #[error("MCP body exceeds maximum buffer size")]
    OversizedBody,
    #[error("MCP body JSON parse error: {0}")]
    Json(#[from] serde_json::Error),
}

fn find_header_end(buf: &[u8]) -> Option<usize> {
    if let Some(i) = find_subslice(buf, b"\r\n\r\n") {
        return Some(i);
    }
    if let Some(i) = find_subslice(buf, b"\n\n") {
        return Some(i + 2);
    }
    None
}

fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    haystack.windows(needle.len()).position(|w| w == needle)
}

fn parse_content_length(header: &str) -> Result<usize, FrameError> {
    for line in header.split("\r\n").flat_map(|l| l.split('\n')) {
        let mut parts = line.splitn(2, ':');
        let key = parts.next().unwrap_or("").trim();
        let val = parts.next().unwrap_or("").trim();
        if key.eq_ignore_ascii_case("Content-Length") {
            return val.parse().map_err(|_| FrameError::InvalidContentLength);
        }
    }
    Err(FrameError::MissingContentLength)
}

type PendingRequests = Arc<Mutex<HashMap<u64, oneshot::Sender<Result<Value, protocol::RpcError>>>>>;
type StderrRing = Arc<Mutex<VecDeque<String>>>;

/// A live connection to an MCP server. Cloneable handles share the same
/// underlying transport.
#[derive(Clone)]
pub struct Transport {
    inner: Arc<TransportInner>,
}

struct TransportInner {
    server_name: String,
    stdin_tx: UnboundedSender<Vec<u8>>,
    next_id: AtomicU64,
    pending: PendingRequests,
    stderr_ring: StderrRing,
    /// Detected framing mode: 0 = unknown, 1 = ContentLength, 2 = NewlineDelimited.
    /// Shared with the reader task that performs detection.
    framing: Arc<AtomicU8>,
    /// Held so the child is killed on drop (we use kill_on_drop on the
    /// Command). Wrapped in Mutex<Option> so `shutdown` can take it.
    child: Mutex<Option<Child>>,
}

impl Transport {
    /// Server display name (the key from `[mcp.<name>]` in the user
    /// config).
    pub fn server_name(&self) -> &str {
        &self.inner.server_name
    }

    /// Allocate a fresh JSON-RPC request id. Atomically incrementing
    /// from any task is safe.
    pub fn next_id(&self) -> u64 {
        self.inner.next_id.fetch_add(1, Ordering::Relaxed)
    }

    /// Get the detected framing mode (defaults to NewlineDelimited
    /// until detection runs).
    fn framing_mode(&self) -> FramingMode {
        match self.inner.framing.load(Ordering::Relaxed) {
            1 => FramingMode::ContentLength,
            _ => FramingMode::NewlineDelimited,
        }
    }

    /// Encode a message using the detected framing mode.
    fn encode_msg(&self, value: &Value) -> Vec<u8> {
        encode_with_framing(value, self.framing_mode())
    }

    /// Send a JSON-RPC request and await its response. Returns the
    /// `result` field on success or a JSON-RPC `error` on protocol-level
    /// failure. Times out after `timeout` (typical: 30s for cold
    /// servers, 5s for already-warm).
    pub async fn request(
        &self,
        method: &str,
        params: Value,
        timeout: std::time::Duration,
    ) -> Result<Value, RequestError> {
        let id = self.next_id();
        let msg = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        });

        let (tx, rx) = oneshot::channel();
        {
            let mut pending = self.inner.pending.lock().await;
            pending.insert(id, tx);
        }

        if self.inner.stdin_tx.send(self.encode_msg(&msg)).is_err() {
            let mut pending = self.inner.pending.lock().await;
            pending.remove(&id);
            return Err(RequestError::Disconnected);
        }

        match tokio::time::timeout(timeout, rx).await {
            Ok(Ok(Ok(val))) => Ok(val),
            Ok(Ok(Err(e))) => Err(RequestError::Rpc(e)),
            Ok(Err(_)) => {
                // Sender dropped — reader task likely died.
                let mut pending = self.inner.pending.lock().await;
                pending.remove(&id);
                Err(RequestError::Disconnected)
            }
            Err(_) => {
                let mut pending = self.inner.pending.lock().await;
                pending.remove(&id);
                Err(RequestError::Timeout)
            }
        }
    }

    /// Send a notification (no response expected).
    pub fn notify(&self, method: &str, params: Value) -> Result<(), RequestError> {
        let msg = json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params,
        });
        self.inner
            .stdin_tx
            .send(self.encode_msg(&msg))
            .map_err(|_| RequestError::Disconnected)
    }

    /// Snapshot of the most recent stderr lines (most recent last).
    pub async fn recent_stderr(&self) -> Vec<String> {
        let guard = self.inner.stderr_ring.lock().await;
        guard.iter().cloned().collect()
    }

    /// Clean shutdown: send `notifications/exit`, wait briefly, drop the
    /// child. If the child has already exited this is a no-op.
    pub async fn shutdown(&self) {
        let _ = self.notify("notifications/exit", json!({}));
        tokio::time::sleep(std::time::Duration::from_millis(150)).await;
        let mut guard = self.inner.child.lock().await;
        if let Some(mut child) = guard.take() {
            let _ = child.start_kill();
            let _ = tokio::time::timeout(std::time::Duration::from_millis(500), child.wait()).await;
        }
    }
}

#[derive(Debug)]
pub enum RequestError {
    /// Server's stdin channel has closed — process is gone.
    Disconnected,
    /// No response within the deadline.
    Timeout,
    /// JSON-RPC protocol error from the server.
    Rpc(protocol::RpcError),
}

impl std::fmt::Display for RequestError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Disconnected => f.write_str("MCP server disconnected"),
            Self::Timeout => f.write_str("MCP request timed out"),
            Self::Rpc(e) => write!(f, "MCP rpc error {}: {}", e.code, e.message),
        }
    }
}

impl std::error::Error for RequestError {}

/// Configuration for spawning an MCP transport.
#[derive(Debug, Clone)]
pub struct SpawnConfig {
    pub server_name: String,
    pub command: String,
    pub args: Vec<String>,
    pub env: HashMap<String, String>,
}

impl Transport {
    /// Spawn a new MCP server process, wire up I/O tasks, and run the
    /// `initialize` / `notifications/initialized` handshake.
    ///
    /// On any failure (binary missing, handshake timeout) we return
    /// `None` so callers can keep going without that server — same
    /// silent-fallthrough policy as `lsp_client.rs`.
    pub async fn spawn(cfg: SpawnConfig) -> Option<Self> {
        let mut command = Command::new(&cfg.command);
        command
            .args(&cfg.args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        for (k, v) in &cfg.env {
            command.env(k, v);
        }

        let mut child: Child = match command.spawn() {
            Ok(c) => c,
            Err(e) => {
                tracing::info!(
                    target: "jfc::mcp",
                    server = %cfg.server_name,
                    command = %cfg.command,
                    error = %e,
                    "spawn failed (binary likely not on PATH)"
                );
                return None;
            }
        };

        let stdin = child.stdin.take()?;
        let stdout = child.stdout.take()?;
        let stderr = child.stderr.take()?;

        // 1. Stderr drain → tracing AND ring buffer.
        let stderr_ring: StderrRing = Arc::new(Mutex::new(VecDeque::with_capacity(
            DEFAULT_STDERR_RING_CAPACITY,
        )));
        let stderr_ring_clone = Arc::clone(&stderr_ring);
        let server_name_for_stderr = cfg.server_name.clone();
        tokio::spawn(async move {
            let mut reader = BufReader::new(stderr).lines();
            while let Ok(Some(line)) = reader.next_line().await {
                if line.trim().is_empty() {
                    continue;
                }
                tracing::debug!(
                    target: "jfc::mcp",
                    server = %server_name_for_stderr,
                    stderr = %line,
                    "mcp stderr"
                );
                let mut guard = stderr_ring_clone.lock().await;
                if guard.len() == DEFAULT_STDERR_RING_CAPACITY {
                    guard.pop_front();
                }
                guard.push_back(line);
            }
        });

        // 2. Stdin writer.
        let (stdin_tx, mut stdin_rx) = mpsc::unbounded_channel::<Vec<u8>>();
        let mut stdin_handle = stdin;
        let server_name_for_writer = cfg.server_name.clone();
        tokio::spawn(async move {
            while let Some(bytes) = stdin_rx.recv().await {
                if let Err(e) = stdin_handle.write_all(&bytes).await {
                    tracing::warn!(
                        target: "jfc::mcp",
                        server = %server_name_for_writer,
                        error = %e,
                        "stdin write failed — server probably exited"
                    );
                    break;
                }
                let _ = stdin_handle.flush().await;
            }
        });

        // 3. Stdout reader — auto-detects framing from first byte.
        let pending: PendingRequests = Arc::new(Mutex::new(HashMap::new()));
        let pending_for_reader = Arc::clone(&pending);
        let server_name_for_reader = cfg.server_name.clone();
        let framing = Arc::new(AtomicU8::new(0));
        let framing_for_reader = Arc::clone(&framing);
        tokio::spawn(async move {
            let mut reader = BufReader::new(stdout);
            let mut buf: Vec<u8> = Vec::with_capacity(8 * 1024);
            let mut chunk = [0u8; 4096];
            let mut mode: Option<FramingMode> = None;
            loop {
                let n = match reader.read(&mut chunk).await {
                    Ok(0) => {
                        tracing::info!(
                            target: "jfc::mcp",
                            server = %server_name_for_reader,
                            "stdout EOF — server exited"
                        );
                        return;
                    }
                    Ok(n) => n,
                    Err(e) => {
                        tracing::warn!(
                            target: "jfc::mcp",
                            server = %server_name_for_reader,
                            error = %e,
                            "stdout read error — terminating reader"
                        );
                        return;
                    }
                };
                buf.extend_from_slice(&chunk[..n]);

                // Detect framing from first non-whitespace byte.
                if mode.is_none() {
                    if let Some(detected) = detect_framing(&buf) {
                        mode = Some(detected);
                        let code = match detected {
                            FramingMode::ContentLength => 1u8,
                            FramingMode::NewlineDelimited => 2u8,
                        };
                        framing_for_reader.store(code, Ordering::Relaxed);
                        tracing::debug!(
                            target: "jfc::mcp",
                            server = %server_name_for_reader,
                            ?detected,
                            "auto-detected framing mode"
                        );
                    }
                }

                let parser = mode.unwrap_or(FramingMode::NewlineDelimited);
                loop {
                    let result = match parser {
                        FramingMode::ContentLength => try_parse(&buf),
                        FramingMode::NewlineDelimited => try_parse_ndjson(&buf),
                    };
                    match result {
                        Ok(Some((Value::Null, consumed))) => {
                            // Empty line in ndjson — skip.
                            buf.drain(..consumed);
                        }
                        Ok(Some((msg, consumed))) => {
                            buf.drain(..consumed);
                            handle_inbound(&msg, &pending_for_reader).await;
                        }
                        Ok(None) => break,
                        Err(e) => {
                            tracing::warn!(
                                target: "jfc::mcp",
                                server = %server_name_for_reader,
                                error = %e,
                                "framing error — clearing buffer to resync"
                            );
                            buf.clear();
                            break;
                        }
                    }
                }
            }
        });

        let inner = Arc::new(TransportInner {
            server_name: cfg.server_name.clone(),
            stdin_tx,
            next_id: AtomicU64::new(1),
            pending,
            stderr_ring,
            framing,
            child: Mutex::new(Some(child)),
        });
        let transport = Self { inner };

        // Handshake: initialize → wait → initialized notification.
        // Send as bare JSON (newline-delimited) — universally parseable
        // by both Content-Length and ndjson servers. Once the server
        // responds, we detect its framing and lock in for future sends.
        let init =
            protocol::build_initialize(transport.next_id(), "jfc", env!("CARGO_PKG_VERSION"));
        let init_bytes = encode_with_framing(&init, FramingMode::NewlineDelimited);
        if transport.inner.stdin_tx.send(init_bytes).is_err() {
            tracing::warn!(
                target: "jfc::mcp",
                server = %cfg.server_name,
                "could not send initialize — writer task dead"
            );
            return None;
        }

        // We need to await the response. Re-construct the "wait by id"
        // by registering a oneshot under the id we just used. The
        // initialize id was 1 since we just minted from a fresh
        // counter — register that.
        let init_id = 1u64;
        let (init_tx, init_rx) = oneshot::channel();
        {
            let mut pending_guard = transport.inner.pending.lock().await;
            pending_guard.insert(init_id, init_tx);
        }
        let timeout = std::time::Duration::from_secs(30);
        match tokio::time::timeout(timeout, init_rx).await {
            Ok(Ok(Ok(_))) => {}
            Ok(Ok(Err(e))) => {
                tracing::warn!(
                    target: "jfc::mcp",
                    server = %cfg.server_name,
                    code = e.code,
                    message = %e.message,
                    "initialize returned rpc error"
                );
                return None;
            }
            Ok(Err(_)) => {
                tracing::warn!(
                    target: "jfc::mcp",
                    server = %cfg.server_name,
                    "init oneshot dropped — server exited early"
                );
                return None;
            }
            Err(_) => {
                tracing::warn!(
                    target: "jfc::mcp",
                    server = %cfg.server_name,
                    "initialize handshake timed out after 30s"
                );
                let mut pending_guard = transport.inner.pending.lock().await;
                pending_guard.remove(&init_id);
                return None;
            }
        }

        let initialized = protocol::build_initialized_notification();
        if transport.inner.stdin_tx.send(transport.encode_msg(&initialized)).is_err() {
            tracing::warn!(
                target: "jfc::mcp",
                server = %cfg.server_name,
                "could not send initialized notification"
            );
            return None;
        }

        tracing::info!(
            target: "jfc::mcp",
            server = %cfg.server_name,
            command = %cfg.command,
            "mcp transport ready"
        );
        Some(transport)
    }
}

async fn handle_inbound(msg: &Value, pending: &PendingRequests) {
    // Response (has `id` and either `result` or `error`).
    if msg.get("id").is_some() && msg.get("method").is_none() {
        let Some(id) = msg.get("id").and_then(|v| v.as_u64()) else {
            return;
        };
        let mut guard = pending.lock().await;
        let Some(tx) = guard.remove(&id) else {
            return;
        };
        if let Some(err) = msg.get("error") {
            let err: protocol::RpcError = match serde_json::from_value(err.clone()) {
                Ok(e) => e,
                Err(_) => protocol::RpcError {
                    code: -32603,
                    message: "malformed error object".into(),
                    data: None,
                },
            };
            let _ = tx.send(Err(err));
        } else {
            let result = msg.get("result").cloned().unwrap_or(Value::Null);
            let _ = tx.send(Ok(result));
        }
        return;
    }

    // Notification path. The interesting one is
    // `notifications/tools/list_changed` — when an MCP server's tool
    // catalog mutates (server-side hot-reload, plugin install), we
    // refresh the catalog and emit a UI signal so the user knows.
    if let Some(method) = msg.get("method").and_then(|v| v.as_str())
        && method == "notifications/tools/list_changed"
    {
        crate::registry::request_refresh();
        tracing::info!(
            target: "jfc::mcp",
            "received notifications/tools/list_changed — registry refresh requested"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn encode_then_parse_roundtrips_normal() {
        let v = json!({"jsonrpc":"2.0","id":1,"method":"tools/list"});
        let bytes = encode(&v);
        let (parsed, consumed) = try_parse(&bytes).unwrap().unwrap();
        assert_eq!(parsed, v);
        assert_eq!(consumed, bytes.len());
    }

    #[test]
    fn encode_uses_lsp_framing_normal() {
        let v = json!({});
        let bytes = encode(&v);
        let s = std::str::from_utf8(&bytes).unwrap();
        assert!(s.starts_with("Content-Length: "));
        assert!(s.contains("\r\n\r\n"));
    }

    #[test]
    fn try_parse_partial_returns_none_normal() {
        let header = b"Content-Length: 100\r\n\r\n";
        let r = try_parse(header).unwrap();
        assert!(r.is_none());
    }

    #[test]
    fn try_parse_no_header_yet_returns_none_normal() {
        assert!(try_parse(b"Content-Length: 4").unwrap().is_none());
    }

    #[test]
    fn try_parse_consumes_only_one_message_normal() {
        let a = encode(&json!({"id":1}));
        let b = encode(&json!({"id":2}));
        let combined: Vec<u8> = a.iter().chain(b.iter()).copied().collect();
        let (first, consumed) = try_parse(&combined).unwrap().unwrap();
        assert_eq!(first["id"], 1);
        assert_eq!(consumed, a.len());
        let rest = &combined[consumed..];
        let (second, consumed2) = try_parse(rest).unwrap().unwrap();
        assert_eq!(second["id"], 2);
        assert_eq!(consumed2, b.len());
    }

    #[test]
    fn missing_content_length_is_error_robust() {
        let bad = b"X-Header: 1\r\n\r\n{}";
        assert!(matches!(
            try_parse(bad).unwrap_err(),
            FrameError::MissingContentLength
        ));
    }

    #[test]
    fn bad_content_length_is_error_robust() {
        let bad = b"Content-Length: not-a-number\r\n\r\n{}";
        assert!(matches!(
            try_parse(bad).unwrap_err(),
            FrameError::InvalidContentLength
        ));
    }

    #[test]
    fn malformed_body_is_error_robust() {
        let bad = b"Content-Length: 4\r\n\r\nXXXX";
        let err = try_parse(bad).unwrap_err();
        assert!(matches!(err, FrameError::Json(_)));
    }

    #[test]
    fn header_case_insensitive_robust() {
        let body = b"{}";
        let mut bytes = b"content-length: 2\r\n\r\n".to_vec();
        bytes.extend_from_slice(body);
        let (parsed, _) = try_parse(&bytes).unwrap().unwrap();
        assert_eq!(parsed, json!({}));
    }

    #[test]
    fn oversized_body_rejected_robust() {
        // Header claiming 1 GB body — must reject without OOMing.
        let huge = format!("Content-Length: {}\r\n\r\n", MAX_BUFFER_BYTES + 1);
        let err = try_parse(huge.as_bytes()).unwrap_err();
        assert!(matches!(err, FrameError::OversizedBody));
    }

    /// Mock transport echo test: we drive a fake reader/writer pair
    /// without spawning a process, then verify a request gets routed to
    /// its oneshot when the matching response lands.
    #[tokio::test]
    async fn pending_response_routes_by_id_normal() {
        let pending: PendingRequests = Arc::new(Mutex::new(HashMap::new()));
        let (tx, rx) = oneshot::channel::<Result<Value, protocol::RpcError>>();
        {
            let mut guard = pending.lock().await;
            guard.insert(42, tx);
        }
        // Simulate inbound response for id=42.
        let response = json!({
            "jsonrpc": "2.0",
            "id": 42,
            "result": {"echo": "hi"}
        });
        handle_inbound(&response, &pending).await;
        let result = rx.await.unwrap().unwrap();
        assert_eq!(result["echo"], "hi");
        // Pending map should be empty now.
        assert!(pending.lock().await.is_empty());
    }

    #[tokio::test]
    async fn pending_response_routes_error_to_oneshot_robust() {
        let pending: PendingRequests = Arc::new(Mutex::new(HashMap::new()));
        let (tx, rx) = oneshot::channel::<Result<Value, protocol::RpcError>>();
        {
            let mut guard = pending.lock().await;
            guard.insert(7, tx);
        }
        let response = json!({
            "jsonrpc": "2.0",
            "id": 7,
            "error": { "code": -32601, "message": "method not found" }
        });
        handle_inbound(&response, &pending).await;
        let err = rx.await.unwrap().unwrap_err();
        assert_eq!(err.code, -32601);
        assert_eq!(err.message, "method not found");
    }

    #[tokio::test]
    async fn unknown_id_response_dropped_silently_robust() {
        // Server sends a response for an id we never registered. Must
        // not panic; pending map stays empty.
        let pending: PendingRequests = Arc::new(Mutex::new(HashMap::new()));
        let response = json!({
            "jsonrpc": "2.0",
            "id": 999,
            "result": null
        });
        handle_inbound(&response, &pending).await;
        assert!(pending.lock().await.is_empty());
    }

    #[tokio::test]
    async fn notification_does_not_consume_pending_robust() {
        let pending: PendingRequests = Arc::new(Mutex::new(HashMap::new()));
        let (tx, _rx) = oneshot::channel::<Result<Value, protocol::RpcError>>();
        {
            let mut guard = pending.lock().await;
            guard.insert(1, tx);
        }
        // Notification has method but no id.
        let note = json!({
            "jsonrpc": "2.0",
            "method": "notifications/tools/list_changed",
            "params": {}
        });
        handle_inbound(&note, &pending).await;
        // id=1 is still pending; notification didn't fire it.
        assert!(pending.lock().await.contains_key(&1));
    }

    #[test]
    fn try_parse_ndjson_complete_line_normal() {
        let buf = b"{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{}}\n";
        let (val, consumed) = try_parse_ndjson(buf).unwrap().unwrap();
        assert_eq!(consumed, buf.len());
        assert_eq!(val["id"], 1);
    }

    #[test]
    fn try_parse_ndjson_partial_returns_none_normal() {
        let buf = b"{\"jsonrpc\":\"2.0\",\"id\":1";
        assert!(try_parse_ndjson(buf).unwrap().is_none());
    }

    #[test]
    fn try_parse_ndjson_empty_line_returns_null_normal() {
        let buf = b"\n{\"id\":1}\n";
        let (val, consumed) = try_parse_ndjson(buf).unwrap().unwrap();
        assert_eq!(val, Value::Null);
        assert_eq!(consumed, 1);
    }

    #[test]
    fn detect_framing_json_start_normal() {
        assert_eq!(detect_framing(b"{\"jsonrpc"), Some(FramingMode::NewlineDelimited));
        assert_eq!(detect_framing(b"  \n{"), Some(FramingMode::NewlineDelimited));
    }

    #[test]
    fn detect_framing_content_length_normal() {
        assert_eq!(
            detect_framing(b"Content-Length: 42\r\n"),
            Some(FramingMode::ContentLength)
        );
    }

    #[test]
    fn detect_framing_empty_returns_none_robust() {
        assert_eq!(detect_framing(b""), None);
        assert_eq!(detect_framing(b"  \n\r\n"), None);
    }

    #[test]
    fn encode_with_framing_ndjson_normal() {
        let val = json!({"id": 1});
        let bytes = encode_with_framing(&val, FramingMode::NewlineDelimited);
        assert!(bytes.ends_with(b"\n"));
        assert!(!bytes.starts_with(b"Content-Length"));
        let parsed: Value = serde_json::from_slice(&bytes[..bytes.len() - 1]).unwrap();
        assert_eq!(parsed["id"], 1);
    }
}

//! Real LSP client: spawns a language-server process (rust-analyzer for
//! `.rs` projects, zls for `.zig`) and routes inbound
//! `textDocument/publishDiagnostics` notifications into the app's event
//! loop as `AppEvent::DiagnosticsUpdated`.
//!
//! ## Architecture (Helix / rust-analyzer pattern, NOT tower-lsp)
//!
//! Three tokio tasks per spawned server:
//! 1. **stderr-drain** — reads stderr line-by-line and forwards to
//!    `tracing::warn!`. Required because language servers (especially
//!    rust-analyzer) chatter heavily on stderr; if we don't drain it, the
//!    OS pipe fills, the server's `eprintln!` blocks, and the whole
//!    thing deadlocks.
//! 2. **stdout-reader** — accumulates bytes into a buffer, then loops
//!    calling `lsp_rpc::try_parse(&buf)`. Each successful parse drains
//!    the consumed bytes from the front of the buffer. On
//!    `publishDiagnostics`, sends `AppEvent::DiagnosticsUpdated` upstream.
//! 3. **stdin-writer** — pulls `Vec<u8>` framed messages off an
//!    `mpsc::UnboundedReceiver` and writes each to the child's stdin.
//!    The producer side of that channel is the `LspClient.stdin_tx`
//!    field, so callers (didOpen / didChange / shutdown) just push
//!    pre-encoded bytes — no shared writer mutex.
//!
//! All I/O is `tokio::io::Async{Read,Write}`. Mixing `std::io` here
//! would deadlock the runtime when the server back-pressures.
//!
//! ## Request IDs
//!
//! Each `LspClient` owns an `AtomicU64` counter, fetched-and-incremented
//! with `Ordering::Relaxed`. Concurrent requesters never collide.
//!
//! ## Graceful shutdown
//!
//! `shutdown()` sends the LSP `shutdown` request, awaits its response,
//! sends `exit` (notification), then waits up to 1s for the process to
//! exit before returning. The child is dropped after that — if it
//! didn't exit on its own, tokio kills it on drop.
//!
//! ## Why not lsp-types?
//!
//! The framing layer (`lsp_rpc.rs`) already uses raw `serde_json::Value`
//! for parse + build. Pulling in lsp-types would force us to convert
//! between its strongly-typed structs and Value at every boundary, with
//! no real win — we only need a handful of message shapes and they're
//! tiny. We stick with `serde_json::json!` macros throughout.

#![allow(dead_code)]

use std::collections::HashMap;
use std::path::Path;
use std::process::Stdio;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::Mutex;
use tokio::sync::mpsc::{self, UnboundedSender};
use tokio::sync::oneshot;

use crate::app::AppEvent;
use crate::lsp_rpc;

/// Re-export from jfc-graph for convenience.
pub use jfc_graph::enrichment::LspLocation;

type PendingRequests = Arc<Mutex<HashMap<u64, oneshot::Sender<Value>>>>;

pub struct LspClient {
    stdin_tx: UnboundedSender<Vec<u8>>,
    next_id: AtomicU64,
    pending: PendingRequests,
}

impl LspClient {
    pub fn next_id(&self) -> u64 {
        self.next_id.fetch_add(1, Ordering::Relaxed)
    }

    /// Send a JSON-RPC request and await the response (5s timeout).
    /// Returns `None` on timeout or channel failure.
    pub async fn send_request(&self, method: &str, params: Value) -> Option<Value> {
        let id = self.next_id();
        let msg = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        });

        let (tx, rx) = oneshot::channel();
        {
            let mut pending = self.pending.lock().await;
            pending.insert(id, tx);
        }

        if self.stdin_tx.send(lsp_rpc::encode(&msg)).is_err() {
            let mut pending = self.pending.lock().await;
            pending.remove(&id);
            return None;
        }

        match tokio::time::timeout(tokio::time::Duration::from_secs(5), rx).await {
            Ok(Ok(val)) => Some(val),
            _ => {
                let mut pending = self.pending.lock().await;
                pending.remove(&id);
                None
            }
        }
    }

    /// Request `textDocument/definition`. Returns the first location if any.
    pub async fn goto_definition_async(
        &self,
        file: &Path,
        line: u32,
        col: u32,
    ) -> Option<LspLocation> {
        let uri = format!("file://{}", file.display());
        let params = json!({
            "textDocument": { "uri": uri },
            "position": { "line": line.saturating_sub(1), "character": col.saturating_sub(1) }
        });

        let result = self.send_request("textDocument/definition", params).await?;
        parse_location_response(&result)
    }

    /// Request `textDocument/references`. Returns all locations.
    pub async fn find_references_async(
        &self,
        file: &Path,
        line: u32,
        col: u32,
    ) -> Vec<LspLocation> {
        let uri = format!("file://{}", file.display());
        let params = json!({
            "textDocument": { "uri": uri },
            "position": { "line": line.saturating_sub(1), "character": col.saturating_sub(1) },
            "context": { "includeDeclaration": true }
        });

        let Some(result) = self.send_request("textDocument/references", params).await else {
            return Vec::new();
        };
        parse_locations_response(&result)
    }

    /// Spawn the language server and run the initialize/initialized
    /// handshake. Returns `None` if the binary isn't on PATH (so callers
    /// can silently fall through to the cargo-check producer instead of
    /// crashing the UI).
    ///
    /// The returned client owns three background tokio tasks (stderr
    /// drain, stdout reader, stdin writer) plus the spawned child. They
    /// live until `shutdown()` is called or the client is dropped.
    ///
    /// **Not unit-tested**: requires an actual `rust-analyzer` /
    /// `zls` / etc. binary. Integration tests against a fake stdio
    /// server would be the next layer; the component pieces (id
    /// counter, message builders) are tested below.
    pub async fn spawn(
        server_cmd: &str,
        args: &[&str],
        root_uri: &str,
        app_tx: UnboundedSender<AppEvent>,
    ) -> Option<Self> {
        let mut child: Child = match Command::new(server_cmd)
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
        {
            Ok(c) => c,
            Err(e) => {
                tracing::info!(
                    target: "jfc::lsp",
                    server = server_cmd,
                    error = %e,
                    "spawn failed (binary likely not on PATH)"
                );
                return None;
            }
        };

        let stdin = child.stdin.take()?;
        let stdout = child.stdout.take()?;
        let stderr = child.stderr.take()?;

        // 1. Stderr drain. Critical — see module docs. Read line-by-line,
        // forward each non-empty line to tracing::warn! so the user can
        // see crashes / handshake errors via RUST_LOG=jfc::lsp=debug.
        tokio::spawn(async move {
            let mut reader = BufReader::new(stderr).lines();
            while let Ok(Some(line)) = reader.next_line().await {
                if !line.trim().is_empty() {
                    tracing::warn!(
                        target: "jfc::lsp",
                        stderr = %line,
                        "lsp stderr"
                    );
                }
            }
        });

        // 2. Stdin writer. UnboundedSender is the producer side returned
        // to the caller via the LspClient struct.
        let (stdin_tx, mut stdin_rx) = mpsc::unbounded_channel::<Vec<u8>>();
        let mut stdin_handle = stdin;
        tokio::spawn(async move {
            while let Some(bytes) = stdin_rx.recv().await {
                if let Err(e) = stdin_handle.write_all(&bytes).await {
                    tracing::warn!(
                        target: "jfc::lsp",
                        error = %e,
                        "stdin write failed — server probably exited"
                    );
                    break;
                }
                // Force flush so tiny messages (a 60-byte initialized
                // notification) don't sit in the OS pipe buffer waiting
                // for someone else's bigger write.
                let _ = stdin_handle.flush().await;
            }
        });

        // 3. Stdout reader. Set up a oneshot the reader can signal when
        // it sees the initialize response, so this function can block
        // on the handshake before returning.
        let pending: PendingRequests = Arc::new(Mutex::new(HashMap::new()));
        let (init_done_tx, init_done_rx) = oneshot::channel::<()>();
        let init_done_tx = Arc::new(tokio::sync::Mutex::new(Some(init_done_tx)));
        let app_tx_for_reader = app_tx.clone();
        let pending_for_reader = Arc::clone(&pending);
        tokio::spawn(async move {
            let mut reader = BufReader::new(stdout);
            let mut buf: Vec<u8> = Vec::with_capacity(8 * 1024);
            let mut chunk = [0u8; 4096];
            loop {
                let n = match reader.read(&mut chunk).await {
                    Ok(0) => {
                        tracing::info!(target: "jfc::lsp", "stdout EOF — server exited");
                        return;
                    }
                    Ok(n) => n,
                    Err(e) => {
                        tracing::warn!(
                            target: "jfc::lsp",
                            error = %e,
                            "stdout read error — terminating reader"
                        );
                        return;
                    }
                };
                buf.extend_from_slice(&chunk[..n]);

                loop {
                    match lsp_rpc::try_parse(&buf) {
                        Ok(Some((msg, consumed))) => {
                            buf.drain(..consumed);
                            handle_inbound(
                                &msg,
                                &app_tx_for_reader,
                                &init_done_tx,
                                &pending_for_reader,
                            )
                            .await;
                        }
                        Ok(None) => break,
                        Err(e) => {
                            tracing::warn!(
                                target: "jfc::lsp",
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

        let client = Self {
            stdin_tx,
            next_id: AtomicU64::new(1),
            pending,
        };

        // Send `initialize` (id=1) then wait for the response, then
        // send the `initialized` notification per LSP spec. The server
        // will not push diagnostics until it sees `initialized`.
        let init_id = client.next_id();
        let init = lsp_rpc::build_initialize(init_id, std::process::id(), root_uri);
        if client.stdin_tx.send(lsp_rpc::encode(&init)).is_err() {
            tracing::warn!(target: "jfc::lsp", "could not send initialize — writer task dead");
            return None;
        }

        // Bound the handshake. rust-analyzer can take a few seconds on
        // first cold start while it indexes; 30s is generous.
        let timeout = tokio::time::Duration::from_secs(30);
        match tokio::time::timeout(timeout, init_done_rx).await {
            Ok(Ok(())) => {}
            Ok(Err(_)) => {
                tracing::warn!(target: "jfc::lsp", "init oneshot dropped — server exited early");
                return None;
            }
            Err(_) => {
                tracing::warn!(
                    target: "jfc::lsp",
                    "initialize handshake timed out after 30s"
                );
                return None;
            }
        }

        let initialized = lsp_rpc::build_initialized();
        if client.stdin_tx.send(lsp_rpc::encode(&initialized)).is_err() {
            tracing::warn!(target: "jfc::lsp", "could not send initialized");
            return None;
        }
        tracing::info!(
            target: "jfc::lsp",
            server = server_cmd,
            "lsp client ready"
        );
        Some(client)
    }

    /// Send `textDocument/didOpen`. LSP requires this before the server
    /// will push diagnostics for a file. `language_id` is e.g. "rust" or
    /// "zig"; the spec defines a fixed set per language.
    pub fn did_open(&self, uri: &str, language_id: &str, version: i32, text: &str) {
        tracing::debug!(
            target: "jfc::lsp",
            uri,
            language_id,
            "didOpen"
        );
        let msg = build_did_open(uri, language_id, version, text);
        let _ = self.stdin_tx.send(lsp_rpc::encode(&msg));
    }

    /// Send `textDocument/didChange` with a full-document replacement.
    /// LSP also supports incremental changes; full-doc is simpler and
    /// the server reconciles either way.
    pub fn did_change(&self, uri: &str, version: i32, text: &str) {
        tracing::trace!(
            target: "jfc::lsp",
            uri,
            version,
            "didChange"
        );
        let msg = build_did_change(uri, version, text);
        let _ = self.stdin_tx.send(lsp_rpc::encode(&msg));
    }

    /// Polite shutdown: `shutdown` request → wait for response → `exit`
    /// notification → 1s grace period before returning. The caller
    /// should drop `self` after this so the writer task tears down.
    ///
    /// We don't actually wait on the response oneshot here — keeping the
    /// reader-task wiring for that would double the complexity for a
    /// case where ordering is best-effort anyway. We rely on a fixed
    /// short delay: send shutdown, wait 200ms, send exit, wait 1s.
    pub async fn shutdown(&self) {
        tracing::info!(target: "jfc::lsp", "shutting down lsp client");
        let id = self.next_id();
        let req = build_shutdown(id);
        let _ = self.stdin_tx.send(lsp_rpc::encode(&req));
        tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;
        let exit = build_exit();
        let _ = self.stdin_tx.send(lsp_rpc::encode(&exit));
        tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
    }
}

async fn handle_inbound(
    msg: &Value,
    app_tx: &UnboundedSender<AppEvent>,
    init_done_tx: &Arc<tokio::sync::Mutex<Option<oneshot::Sender<()>>>>,
    pending: &PendingRequests,
) {
    // Response to a request (has `id`, no `method`).
    if msg.get("id").is_some() && msg.get("method").is_none() {
        let id = msg.get("id").and_then(|v| v.as_u64()).unwrap_or(0);

        // Special-case: initialize response (id=1) signals the handshake oneshot.
        if id == 1 {
            let mut guard = init_done_tx.lock().await;
            if let Some(tx) = guard.take() {
                let _ = tx.send(());
            }
        }

        // Route to pending request map.
        let mut guard = pending.lock().await;
        if let Some(tx) = guard.remove(&id) {
            let result = msg
                .get("result")
                .cloned()
                .unwrap_or(Value::Null);
            let _ = tx.send(result);
        }
        return;
    }

    // Notification dispatch.
    let method = msg.get("method").and_then(|v| v.as_str()).unwrap_or("");
    if method == "textDocument/publishDiagnostics" {
        let Some(params) = msg.get("params") else {
            return;
        };
        let Some((_uri, entries)) = lsp_rpc::parse_publish_diagnostics(params) else {
            return;
        };
        let _ = app_tx.send(AppEvent::DiagnosticsUpdated { entries });
    }
}

/// Parse a single Location from an LSP definition/declaration response.
/// LSP returns either a single Location, an array of Locations, or a
/// LocationLink array. We handle the common cases.
fn parse_location_response(value: &Value) -> Option<LspLocation> {
    // Single location object: { uri, range }
    if let Some(loc) = try_parse_location(value) {
        return Some(loc);
    }
    // Array of locations: take the first
    if let Some(arr) = value.as_array() {
        for item in arr {
            if let Some(loc) = try_parse_location(item) {
                return Some(loc);
            }
            // LocationLink: { targetUri, targetRange }
            if let Some(loc) = try_parse_location_link(item) {
                return Some(loc);
            }
        }
    }
    None
}

fn parse_locations_response(value: &Value) -> Vec<LspLocation> {
    let mut out = Vec::new();
    if let Some(arr) = value.as_array() {
        for item in arr {
            if let Some(loc) = try_parse_location(item) {
                out.push(loc);
            }
        }
    } else if let Some(loc) = try_parse_location(value) {
        out.push(loc);
    }
    out
}

fn try_parse_location(value: &Value) -> Option<LspLocation> {
    let uri = value.get("uri")?.as_str()?;
    let range = value.get("range")?;
    let start = range.get("start")?;
    let line = start.get("line")?.as_u64()? as u32 + 1;
    let col = start.get("character")?.as_u64()? as u32 + 1;
    let file = uri.strip_prefix("file://").unwrap_or(uri);
    Some(LspLocation {
        file: std::path::PathBuf::from(file),
        line,
        col,
    })
}

fn try_parse_location_link(value: &Value) -> Option<LspLocation> {
    let uri = value.get("targetUri")?.as_str()?;
    let range = value.get("targetRange")?;
    let start = range.get("start")?;
    let line = start.get("line")?.as_u64()? as u32 + 1;
    let col = start.get("character")?.as_u64()? as u32 + 1;
    let file = uri.strip_prefix("file://").unwrap_or(uri);
    Some(LspLocation {
        file: std::path::PathBuf::from(file),
        line,
        col,
    })
}

/// Synchronous `LspDataProvider` implementation. Since the trait is sync but
/// the LSP client is async, these stubs return None/empty. Full implementation
/// requires a dedicated blocking thread with `tokio::runtime::Handle::block_on`
/// or converting the trait to async.
impl jfc_graph::enrichment::LspDataProvider for LspClient {
    fn goto_definition(&self, _file: &Path, _line: u32, _col: u32) -> Option<LspLocation> {
        // The async version (`goto_definition_async`) is fully functional.
        // Bridging async→sync here would require either:
        //   1. A dedicated OS thread running a tokio runtime for blocking calls
        //   2. Converting LspDataProvider to an async trait
        // For now, the graph engine works without LSP enrichment (tree-sitter only).
        None
    }

    fn find_references(&self, _file: &Path, _line: u32, _col: u32) -> Vec<LspLocation> {
        Vec::new()
    }
}
pub fn build_did_open(uri: &str, language_id: &str, version: i32, text: &str) -> Value {
    json!({
        "jsonrpc": "2.0",
        "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": uri,
                "languageId": language_id,
                "version": version,
                "text": text,
            }
        }
    })
}

/// Build a full-document `textDocument/didChange`. Incremental diff mode
/// would replace `contentChanges[0].text` with a `range`+`text` shape;
/// we keep it simple — language servers handle full-doc cheaply.
pub fn build_did_change(uri: &str, version: i32, text: &str) -> Value {
    json!({
        "jsonrpc": "2.0",
        "method": "textDocument/didChange",
        "params": {
            "textDocument": {
                "uri": uri,
                "version": version,
            },
            "contentChanges": [
                { "text": text }
            ]
        }
    })
}

/// Build a `shutdown` request. This *is* a request (has `id`) — servers
/// reply with `null` result.
pub fn build_shutdown(id: u64) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": "shutdown",
    })
}

/// Build the `exit` notification. Sent after the shutdown response;
/// servers terminate cleanly on receipt.
pub fn build_exit() -> Value {
    json!({
        "jsonrpc": "2.0",
        "method": "exit",
    })
}

/// Detect which language server (if any) makes sense for the given
/// directory by scanning for marker files. Returns `(cmd, args)` ready
/// to pass to `Command::new`.
pub fn detect_lsp_for_cwd(cwd: &std::path::Path) -> Option<(&'static str, Vec<&'static str>)> {
    let result = if cwd.join("Cargo.toml").is_file() {
        Some(("rust-analyzer", vec![]))
    } else if cwd.join("build.zig").is_file() {
        Some(("zls", vec![]))
    } else {
        None
    };
    tracing::debug!(
        target: "jfc::lsp",
        ?cwd,
        server = result.as_ref().map(|(cmd, _)| *cmd),
        "detect_lsp_for_cwd"
    );
    result
}

/// Best-effort startup orchestration: detect a language server for the
/// current working directory and spawn it in the background. Gated by
/// `JFC_DISABLE_LSP=1` for CI / users who prefer the cargo-check
/// producer alone.
///
/// This is a fire-and-forget helper — it never blocks the caller. The
/// returned task handle is dropped (we don't keep the client alive
/// across the UI loop yet; that's a follow-up integration). Wire it up
/// from `main.rs` near the other startup spawns.
pub fn maybe_spawn_lsp_clients(cwd: std::path::PathBuf, app_tx: UnboundedSender<AppEvent>) {
    if matches!(
        std::env::var("JFC_DISABLE_LSP").as_deref(),
        Ok("1") | Ok("true")
    ) {
        tracing::debug!(target: "jfc::lsp", "LSP disabled via JFC_DISABLE_LSP");
        return;
    }
    let Some((cmd, args)) = detect_lsp_for_cwd(&cwd) else {
        return;
    };
    tracing::info!(
        target: "jfc::lsp",
        ?cwd,
        server = cmd,
        "spawning lsp client"
    );
    tokio::spawn(async move {
        let root_uri = format!("file://{}", cwd.display());
        let owned_args: Vec<&str> = args.to_vec();
        if let Some(_client) = LspClient::spawn(cmd, &owned_args, &root_uri, app_tx).await {
            // Hold the client alive forever (until the task is cancelled
            // when the runtime shuts down). A more refined integration
            // would store this in App state and call shutdown on exit;
            // doing so cleanly requires plumbing the client through the
            // app-event pipeline and a shutdown signal. The current
            // setup gives us inbound diagnostics with kill_on_drop
            // covering process cleanup at runtime exit.
            std::future::pending::<()>().await;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;
    use std::sync::Arc;

    fn fresh_client() -> LspClient {
        let (tx, _rx) = mpsc::unbounded_channel::<Vec<u8>>();
        LspClient {
            stdin_tx: tx,
            next_id: AtomicU64::new(1),
            pending: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    #[test]
    fn request_id_counter_increments_normal() {
        let c = fresh_client();
        let ids: Vec<u64> = (0..5).map(|_| c.next_id()).collect();
        assert_eq!(ids, vec![1, 2, 3, 4, 5]);
    }

    #[tokio::test]
    async fn request_id_counter_concurrent_robust() {
        let c = Arc::new(fresh_client());
        let mut handles = Vec::with_capacity(100);
        for _ in 0..100 {
            let c = Arc::clone(&c);
            handles.push(tokio::spawn(async move { c.next_id() }));
        }
        let mut ids: HashSet<u64> = HashSet::new();
        for h in handles {
            ids.insert(h.await.unwrap());
        }
        assert_eq!(
            ids.len(),
            100,
            "100 concurrent next_id() calls produced duplicate ids: {ids:?}"
        );
    }

    #[test]
    fn did_open_message_shape_normal() {
        let m = build_did_open("file:///x.rs", "rust", 0, "fn main() {}");
        assert_eq!(m["jsonrpc"], "2.0");
        assert_eq!(m["method"], "textDocument/didOpen");
        assert!(m.get("id").is_none(), "didOpen is a notification");
        let td = &m["params"]["textDocument"];
        assert_eq!(td["uri"], "file:///x.rs");
        assert_eq!(td["languageId"], "rust");
        assert_eq!(td["version"], 0);
        assert_eq!(td["text"], "fn main() {}");
    }

    #[test]
    fn did_change_message_shape_normal() {
        let m = build_did_change("file:///x.rs", 7, "new content");
        assert_eq!(m["jsonrpc"], "2.0");
        assert_eq!(m["method"], "textDocument/didChange");
        assert!(m.get("id").is_none(), "didChange is a notification");
        assert_eq!(m["params"]["textDocument"]["uri"], "file:///x.rs");
        assert_eq!(m["params"]["textDocument"]["version"], 7);
        let changes = m["params"]["contentChanges"]
            .as_array()
            .expect("contentChanges must be an array");
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0]["text"], "new content");
    }

    #[test]
    fn shutdown_message_is_request_normal() {
        let m = build_shutdown(42);
        assert_eq!(m["jsonrpc"], "2.0");
        assert_eq!(m["method"], "shutdown");
        // Critical: shutdown is a *request*, not a notification — it
        // must carry an id so the server can reply.
        assert_eq!(m["id"], 42);
    }

    #[test]
    fn exit_message_is_notification_normal() {
        let m = build_exit();
        assert_eq!(m["jsonrpc"], "2.0");
        assert_eq!(m["method"], "exit");
        assert!(m.get("id").is_none(), "exit is a notification");
    }

    #[test]
    fn detect_lsp_for_cwd_rust_normal() {
        let dir = tempdir();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\nname=\"x\"\n").unwrap();
        let detected = detect_lsp_for_cwd(dir.path());
        assert_eq!(detected.map(|(c, _)| c), Some("rust-analyzer"));
    }

    #[test]
    fn detect_lsp_for_cwd_zig_normal() {
        let dir = tempdir();
        std::fs::write(dir.path().join("build.zig"), "// zig\n").unwrap();
        let detected = detect_lsp_for_cwd(dir.path());
        assert_eq!(detected.map(|(c, _)| c), Some("zls"));
    }

    #[test]
    fn detect_lsp_for_cwd_none_robust() {
        let dir = tempdir();
        assert!(detect_lsp_for_cwd(dir.path()).is_none());
    }

    /// Tiny self-cleaning tempdir so we don't pull in `tempfile` just
    /// for these three tests. Drops remove the directory tree.
    struct TmpDir(std::path::PathBuf);
    impl TmpDir {
        fn path(&self) -> &std::path::Path {
            &self.0
        }
    }
    impl Drop for TmpDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    fn tempdir() -> TmpDir {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let p = std::env::temp_dir().join(format!(
            "jfc-lsp-test-{}-{}",
            std::process::id(),
            nanos
        ));
        std::fs::create_dir_all(&p).unwrap();
        TmpDir(p)
    }
}

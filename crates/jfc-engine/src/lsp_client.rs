//! Real LSP client: spawns a language-server process (rust-analyzer for
//! `.rs` projects, zls for `.zig`) and routes inbound
//! `textDocument/publishDiagnostics` notifications into the app's event
//! loop as `EngineEvent::Provider(ProviderEvent::DiagnosticsUpdated)`.
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
//!    `publishDiagnostics`, sends `ProviderEvent::DiagnosticsUpdated` upstream.
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

use std::collections::HashMap;
use std::path::Path;
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::mpsc::{self, UnboundedSender};
use tokio::sync::oneshot;

use crate::lsp_rpc;
use crate::runtime::{EngineEvent, ProviderEvent};

/// Re-export from jfc-graph for convenience.
pub use jfc_graph::enrichment::LspLocation;

/// Pending request map. Uses a `std::sync::Mutex` (not `tokio::sync::Mutex`)
/// because the only critical sections are HashMap insert / remove — no
/// awaits while the lock is held — and we need to lock from a sync `Drop`
/// impl on `PendingGuard` for exception-safe cleanup. Switching to the std
/// mutex avoids spawning a runtime task per drop.
type PendingRequests = Arc<Mutex<HashMap<u64, oneshot::Sender<Value>>>>;

/// RAII guard that removes its `(id)` entry from the pending-requests
/// map on drop. Constructed by `send_request` immediately after inserting
/// the oneshot sender. Cleanup runs **exactly once** regardless of which
/// `tokio::select!` arm wins (response arrived, timeout fired, future
/// cancelled, etc.) and regardless of which order the reader and
/// `send_request` see the response — `HashMap::remove` is idempotent on
/// a missing key.
///
/// Without this guard, the previous code had two separate cleanup paths
/// (the `is_err()` branch on send-failure, and the timeout branch in the
/// outer match) and a third implicit path (the reader removing the entry
/// when the response arrived). A late response arriving *after* the
/// caller's timeout used to leave a stale entry until the reader saw the
/// matching id; if no matching id ever arrived (e.g. server crash), the
/// `oneshot::Sender` would leak inside the map until the whole client
/// dropped.
struct PendingGuard {
    pending: PendingRequests,
    id: u64,
}

impl Drop for PendingGuard {
    fn drop(&mut self) {
        // Best-effort cleanup. A poisoned mutex means a different thread
        // panicked while holding the lock — we don't have a recovery
        // story here, so we just skip cleanup. The map will be dropped
        // when the whole client tears down.
        if let Ok(mut pending) = self.pending.lock() {
            pending.remove(&self.id);
        }
    }
}

pub struct LspClient {
    _child: Option<Child>,
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
    ///
    /// Cleanup of the `pending` map entry is owned by a `PendingGuard`
    /// RAII handle so it runs exactly once regardless of which exit path
    /// fires:
    /// - send-failure return,
    /// - response received,
    /// - timeout,
    /// - future cancelled by the caller's runtime.
    ///
    /// This eliminates the previous race where a response arriving
    /// *after* timeout and a reader-task removal could either double-
    /// process or leave a stale `oneshot::Sender` sitting in the map.
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
            let mut pending = match self.pending.lock() {
                Ok(g) => g,
                Err(_) => return None,
            };
            pending.insert(id, tx);
        }
        // Guard takes ownership of the (id) entry's lifecycle. Once
        // created, every return path below — including panics — will run
        // its `Drop` and clean up the map exactly once.
        let _guard = PendingGuard {
            pending: Arc::clone(&self.pending),
            id,
        };

        if self.stdin_tx.send(lsp_rpc::encode(&msg)).is_err() {
            return None;
        }

        // `select!` between the response and the timeout. Either arm
        // returns the value (or `None`) directly; the guard cleans up
        // the map on drop. Important: the reader task will *also* try
        // to `remove()` the entry when the matching response arrives
        // — that's idempotent and ordering-independent because
        // `HashMap::remove` on a missing key is a no-op. A late
        // response landing after timeout finds the slot already gone
        // (the guard removed it during unwind) and the reader's send
        // silently fails on the closed oneshot, which is the desired
        // outcome.
        tokio::select! {
            recv = rx => recv.ok(),
            _ = tokio::time::sleep(tokio::time::Duration::from_secs(5)) => None,
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
        cwd: &std::path::Path,
        root_uri: &str,
        app_tx: mpsc::Sender<EngineEvent>,
    ) -> Option<Self> {
        // Resolve the actual binary to exec. On rustup-managed setups the
        // `rust-analyzer` on PATH (`~/.cargo/bin/rust-analyzer`) is a symlink
        // to `rustup`, so every spawn pays a rustup proxy hop that re-resolves
        // the directory's toolchain. `JFC_<SERVER>_PATH` lets the user point
        // straight at the real toolchain binary and skip the shim.
        let resolved = resolve_server_binary(server_cmd);
        let mut child: Child = match Command::new(&resolved)
            .args(args)
            .current_dir(cwd)
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
                    binary = %resolved,
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
            // Per-file diagnostics accumulator. LSP `publishDiagnostics` is
            // *per-file* — each notification carries the complete (and
            // authoritative) set for ONE uri, and an EMPTY array is the
            // server's signal that that file is now clean. But the
            // `DiagnosticsUpdated` event contract is a *full snapshot* that
            // replaces `app.engine.diagnostics` wholesale (events.rs). Without this
            // map, each per-file push clobbered every other file's
            // diagnostics, so: opening a 2nd file erased the 1st file's
            // errors, and fixing one file (empty array) either wiped the
            // whole panel or left other files' stale entries frozen. Keep the
            // latest set per uri here and emit their union so the wholesale
            // replace receives a correct cross-file snapshot. Owned by the
            // single reader task → no lock needed.
            let mut diagnostics_by_uri: std::collections::HashMap<
                String,
                Vec<crate::diagnostics::DiagnosticEntry>,
            > = std::collections::HashMap::new();
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
                                &mut diagnostics_by_uri,
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
            _child: Some(child),
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
    app_tx: &mpsc::Sender<EngineEvent>,
    init_done_tx: &Arc<tokio::sync::Mutex<Option<oneshot::Sender<()>>>>,
    pending: &PendingRequests,
    diagnostics_by_uri: &mut std::collections::HashMap<
        String,
        Vec<crate::diagnostics::DiagnosticEntry>,
    >,
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

        // Route to pending request map. Lock failure (poisoned mutex)
        // means another thread panicked while holding it; in that case
        // we drop the response — the caller's `send_request` will hit
        // its timeout, the `PendingGuard` won't be able to clean up
        // either, but the whole client is in trouble at that point.
        if let Ok(mut guard) = pending.lock()
            && let Some(tx) = guard.remove(&id)
        {
            let result = msg.get("result").cloned().unwrap_or(Value::Null);
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
        let Some((uri, entries)) = lsp_rpc::parse_publish_diagnostics(params) else {
            return;
        };
        let merged = apply_publish_diagnostics(diagnostics_by_uri, uri, entries);
        let _ = app_tx
            .send(EngineEvent::Provider(ProviderEvent::DiagnosticsUpdated {
                entries: merged,
            }))
            .await;
    }
}

/// Fold one file's `publishDiagnostics` into the per-file accumulator and
/// return the cross-file union to ship as the full-snapshot
/// `DiagnosticsUpdated` payload.
///
/// LSP semantics: each notification is the complete, authoritative set for
/// ONE `uri`; an empty array means that file is now clean. So an empty array
/// drops the key (the file stops contributing to the union — this is what
/// makes "fix the issue → it disappears" work), and a non-empty array
/// replaces whatever we held for that file. The union is what the consumer
/// needs because `DiagnosticsUpdated` replaces `app.engine.diagnostics` wholesale.
fn apply_publish_diagnostics(
    diagnostics_by_uri: &mut std::collections::HashMap<
        String,
        Vec<crate::diagnostics::DiagnosticEntry>,
    >,
    uri: String,
    entries: Vec<crate::diagnostics::DiagnosticEntry>,
) -> Vec<crate::diagnostics::DiagnosticEntry> {
    if entries.is_empty() {
        diagnostics_by_uri.remove(&uri);
    } else {
        diagnostics_by_uri.insert(uri, entries);
    }
    diagnostics_by_uri.values().flatten().cloned().collect()
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

/// Resolve the logical server name (`rust-analyzer`, `zls`) to the binary
/// actually spawned. A per-server env override —
/// `JFC_RUST_ANALYZER_PATH` / `JFC_ZLS_PATH`, or the generic
/// `JFC_<UPPER_SNAKE>_PATH` — wins when set and non-empty; otherwise the
/// bare command is returned for normal PATH resolution.
///
/// The motivating case: with rustup, `~/.cargo/bin/rust-analyzer` is a
/// symlink to `rustup`, so each spawn is really a `rustup` proxy invocation
/// that re-resolves the cwd's toolchain. Pointing the override at
/// `~/.rustup/toolchains/<tk>/bin/rust-analyzer` skips that hop.
fn resolve_server_binary(server_cmd: &str) -> String {
    let env_key = format!(
        "JFC_{}_PATH",
        server_cmd.to_ascii_uppercase().replace('-', "_")
    );
    match std::env::var(&env_key) {
        Ok(path) if !path.trim().is_empty() => {
            tracing::debug!(
                target: "jfc::lsp",
                server = server_cmd,
                env = %env_key,
                binary = %path,
                "using LSP binary override"
            );
            path
        }
        _ => server_cmd.to_owned(),
    }
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
pub fn maybe_spawn_lsp_clients(cwd: std::path::PathBuf, app_tx: mpsc::Sender<EngineEvent>) {
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
        let lsp_tx = app_tx.clone();
        let server_name = cmd.to_owned();
        if let Some(_client) = LspClient::spawn(cmd, &owned_args, &cwd, &root_uri, app_tx).await {
            // Notify the sidebar that this LSP is active.
            let _ = lsp_tx
                .send(EngineEvent::Provider(ProviderEvent::LspUpdated {
                    servers: vec![crate::types::LspServerInfo {
                        name: server_name,
                        status: crate::types::LspStatus::Active,
                    }],
                }))
                .await;
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
            _child: None,
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

    #[test]
    fn resolve_server_binary_passes_through_without_override_normal() {
        // No env set → bare command for normal PATH resolution. Use a unique
        // server name so a stray real env var can't perturb the assertion.
        assert_eq!(resolve_server_binary("zzz-fake-lsp"), "zzz-fake-lsp");
    }

    #[test]
    fn resolve_server_binary_honors_env_override_robust() {
        // SAFETY: single-threaded test; key is unique to this test.
        let key = "JFC_RA_OVERRIDE_TEST_LSP_PATH";
        unsafe { std::env::set_var(key, "/opt/toolchain/bin/ra-override-test-lsp") };
        assert_eq!(
            resolve_server_binary("ra-override-test-lsp"),
            "/opt/toolchain/bin/ra-override-test-lsp",
            "JFC_<SERVER>_PATH should override the bare command",
        );
        // Empty override is ignored (falls back to bare command).
        unsafe { std::env::set_var(key, "   ") };
        assert_eq!(
            resolve_server_binary("ra-override-test-lsp"),
            "ra-override-test-lsp",
            "blank override must not shadow PATH resolution",
        );
        unsafe { std::env::remove_var(key) };
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
        let p = std::env::temp_dir().join(format!("jfc-lsp-test-{}-{}", std::process::id(), nanos));
        std::fs::create_dir_all(&p).unwrap();
        TmpDir(p)
    }

    // ──────────────────────────────────────────────────────────────────
    // Pending-map cleanup race regression tests
    //
    // These exercise the invariant that `pending` never leaks an entry,
    // regardless of which order the reader's "response arrived" path and
    // the caller's "timeout" path run. The previous code had three
    // separate cleanup sites (send-failure, timeout, reader); the
    // RAII `PendingGuard` consolidates them.
    // ──────────────────────────────────────────────────────────────────

    /// Helper: insert a oneshot sender into `pending` under `id`,
    /// mirroring what `send_request` does just before constructing its
    /// `PendingGuard`.
    fn insert_pending(pending: &PendingRequests, id: u64) -> oneshot::Receiver<Value> {
        let (tx, rx) = oneshot::channel();
        pending.lock().unwrap().insert(id, tx);
        rx
    }

    /// Race order A: response arrives FIRST, then the request future is
    /// cancelled / dropped (simulating "timeout fires after the value is
    /// already received but before the caller polls again").
    /// Expected: map is empty (reader removed it), receiver got the
    /// value, dropping the guard is a no-op (idempotent remove).
    #[tokio::test]
    async fn pending_cleanup_response_then_timeout_robust() {
        let pending: PendingRequests = Arc::new(Mutex::new(HashMap::new()));
        let id = 42;
        let rx = insert_pending(&pending, id);
        let guard = PendingGuard {
            pending: Arc::clone(&pending),
            id,
        };

        // Reader path: response arrives, removes entry, sends value.
        {
            let mut g = pending.lock().unwrap();
            if let Some(tx) = g.remove(&id) {
                let _ = tx.send(json!({"ok": true}));
            }
        }

        let val = rx.await.expect("response should be delivered");
        assert_eq!(val, json!({"ok": true}));

        // Map is already empty; dropping the guard must not panic and
        // must leave the map empty.
        drop(guard);
        assert!(
            pending.lock().unwrap().is_empty(),
            "pending must remain empty after guard drop following an arrived response"
        );
    }

    /// Race order B: timeout fires FIRST (caller's future is dropped),
    /// then the response arrives late from the reader.
    /// Expected: guard's `Drop` removed the entry on cancellation, so
    /// the late reader finds `None` for `id` and silently does nothing.
    /// No leaked sender, no double-removal panic.
    #[tokio::test]
    async fn pending_cleanup_timeout_then_late_response_robust() {
        let pending: PendingRequests = Arc::new(Mutex::new(HashMap::new()));
        let id = 99;
        let _rx = insert_pending(&pending, id);
        // Sanity: entry exists.
        assert_eq!(pending.lock().unwrap().len(), 1);

        // Caller's future drops (simulating timeout / cancellation):
        // the guard's Drop runs and clears the entry.
        {
            let _guard = PendingGuard {
                pending: Arc::clone(&pending),
                id,
            };
            // _guard goes out of scope here, triggering cleanup.
        }
        assert!(
            pending.lock().unwrap().is_empty(),
            "PendingGuard::drop must remove the entry on the cancellation path"
        );

        // Reader path runs late: response with same id arrives. It
        // tries `remove`, finds nothing, no panic.
        let mut g = pending.lock().unwrap();
        assert!(g.remove(&id).is_none(), "late remove must be a no-op");
    }

    /// Stress: many concurrent (insert + guard-drop) cycles must leave
    /// the map empty and never panic, even when interleaved with reader
    /// removals on the same ids.
    #[tokio::test]
    async fn pending_cleanup_concurrent_races_robust() {
        let pending: PendingRequests = Arc::new(Mutex::new(HashMap::new()));
        let mut handles = Vec::with_capacity(50);
        for id in 0..50u64 {
            let pending = Arc::clone(&pending);
            handles.push(tokio::spawn(async move {
                let _rx = insert_pending(&pending, id);
                // Half the tasks simulate a reader removing first,
                // half let the guard remove on drop.
                if id % 2 == 0 {
                    let _ = pending.lock().unwrap().remove(&id);
                }
                let _guard = PendingGuard {
                    pending: Arc::clone(&pending),
                    id,
                };
                // Guard drops at end of scope.
            }));
        }
        for h in handles {
            h.await.unwrap();
        }
        assert!(
            pending.lock().unwrap().is_empty(),
            "all 50 entries must be cleaned up"
        );
    }

    // ── per-file diagnostics merge (stale-diagnostics fix) ─────────────────

    use crate::diagnostics::{DiagnosticEntry, Severity};

    fn diag(file: &str, message: &str) -> DiagnosticEntry {
        DiagnosticEntry {
            file: file.to_owned(),
            line: 1,
            col: 1,
            message: message.to_owned(),
            code: None,
            source: Some("rust-analyzer".to_owned()),
            severity: Severity::Error,
        }
    }

    // Normal: two files each publish diagnostics. The union carries BOTH —
    // the second file's push must not erase the first file's entries
    // (the original wholesale-replace bug).
    #[test]
    fn publish_two_files_unions_both_normal() {
        let mut acc = std::collections::HashMap::new();
        let merged_a = apply_publish_diagnostics(
            &mut acc,
            "file:///a.rs".to_owned(),
            vec![diag("/a.rs", "err A1")],
        );
        assert_eq!(merged_a.len(), 1);

        let merged_b = apply_publish_diagnostics(
            &mut acc,
            "file:///b.rs".to_owned(),
            vec![diag("/b.rs", "err B1"), diag("/b.rs", "err B2")],
        );
        // Union: A's 1 + B's 2 = 3. Pre-fix this would have been just B's 2.
        assert_eq!(
            merged_b.len(),
            3,
            "union must keep file A after B publishes"
        );
        assert!(merged_b.iter().any(|d| d.message == "err A1"));
        assert!(merged_b.iter().any(|d| d.message == "err B2"));
    }

    // Normal — REGRESSION (the reported bug): a file is fixed and the server
    // re-publishes an EMPTY array for it. That file's diagnostics must
    // disappear from the union, while OTHER files' diagnostics survive.
    #[test]
    fn empty_publish_clears_only_that_file_normal_regression() {
        let mut acc = std::collections::HashMap::new();
        apply_publish_diagnostics(
            &mut acc,
            "file:///a.rs".to_owned(),
            vec![diag("/a.rs", "err A1")],
        );
        apply_publish_diagnostics(
            &mut acc,
            "file:///b.rs".to_owned(),
            vec![diag("/b.rs", "err B1")],
        );
        // Fix file A → server sends empty array for a.rs.
        let merged = apply_publish_diagnostics(&mut acc, "file:///a.rs".to_owned(), vec![]);
        assert_eq!(merged.len(), 1, "only B should remain");
        assert_eq!(merged[0].message, "err B1");
    }

    // Robust: re-publishing a file REPLACES its prior set rather than
    // appending — the server's latest set for a uri is authoritative.
    #[test]
    fn republish_replaces_prior_set_for_same_file_robust() {
        let mut acc = std::collections::HashMap::new();
        apply_publish_diagnostics(
            &mut acc,
            "file:///a.rs".to_owned(),
            vec![diag("/a.rs", "old1"), diag("/a.rs", "old2")],
        );
        let merged = apply_publish_diagnostics(
            &mut acc,
            "file:///a.rs".to_owned(),
            vec![diag("/a.rs", "new1")],
        );
        assert_eq!(merged.len(), 1, "re-publish replaces, not appends");
        assert_eq!(merged[0].message, "new1");
    }

    // Robust: fixing the last file (empty array, nothing else tracked)
    // yields an empty snapshot — the panel goes fully clean.
    #[test]
    fn empty_publish_on_last_file_yields_empty_snapshot_robust() {
        let mut acc = std::collections::HashMap::new();
        apply_publish_diagnostics(
            &mut acc,
            "file:///a.rs".to_owned(),
            vec![diag("/a.rs", "err")],
        );
        let merged = apply_publish_diagnostics(&mut acc, "file:///a.rs".to_owned(), vec![]);
        assert!(
            merged.is_empty(),
            "clearing the only file empties the panel"
        );
    }
}

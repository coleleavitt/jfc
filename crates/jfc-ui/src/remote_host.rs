//! Host-side remote control bridge.
//!
//! `RemoteHost` manages the WebSocket server + connected clients and
//! provides two integrations:
//!
//! 1. **Mirror**: the event loop calls [`RemoteHost::mirror`] for each
//!    relevant `AppEvent`, translating it into a `RemoteEnvelope` and
//!    broadcasting to all connected clients via a `tokio::broadcast`
//!    channel. Non-blocking — a slow or disconnected client never stalls
//!    the host, and multiple clients fan out from one send.
//!
//! 2. **Inject**: each client has a spawned task that reads inbound
//!    envelopes and translates them into `AppEvent`s injected via the
//!    event-loop `tx: EventSender`.

use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

use tokio::sync::broadcast;
use tracing::{debug, info, warn};

use jfc_remote::auth;
use jfc_remote::protocol::{RemoteEnvelope, RemoteFrame, SessionState};
use jfc_remote::transport::{TransportReceiver, TransportSender};
use jfc_remote::ws::WsServer;

use crate::runtime::{AppEvent, EventSender, UiEvent};

/// Broadcast backlog. Frames buffer here until each client's forwarder
/// drains them; a client that lags by more than this many frames drops the
/// oldest (acceptable for a live mirror — the next full status re-syncs).
const BROADCAST_BACKLOG: usize = 1024;

/// State for the remote-control host side.
pub struct RemoteHost {
    /// The running WS server (owns the accept loop).
    server: WsServer,
    /// The pairing token for this session.
    pub token: String,
    /// Next outbound sequence number (monotonically increasing across all
    /// clients — every client shares the same ordered frame stream).
    out_seq: AtomicU64,
    /// Broadcast sender; each connected client subscribes a receiver.
    mirror_tx: broadcast::Sender<RemoteFrame>,
    /// Number of connected clients.
    pub client_count: Arc<AtomicUsize>,
    /// Tool-use id of the most recently mirrored permission request, so the
    /// event loop doesn't re-mirror the same pending approval every burst.
    last_mirrored_approval: std::sync::Mutex<Option<String>>,
    /// Last mirrored session status, so we only emit on transitions.
    last_status: std::sync::Mutex<Option<SessionState>>,
}

impl RemoteHost {
    /// Start the remote-control server. Spawns the client acceptor and a
    /// heartbeat task, and returns the host handle.
    pub async fn start(port: u16, event_tx: EventSender) -> std::io::Result<Arc<Self>> {
        let token = auth::generate_token();
        let addr: SocketAddr = format!("127.0.0.1:{port}").parse().unwrap();
        let (server, client_rx) = WsServer::bind(addr, token.clone()).await?;

        info!(target: "jfc::remote", addr = %server.addr, "remote-control server started");

        let (mirror_tx, _) = broadcast::channel(BROADCAST_BACKLOG);
        let client_count = Arc::new(AtomicUsize::new(0));

        let host = Arc::new(Self {
            server,
            token: token.clone(),
            out_seq: AtomicU64::new(1),
            mirror_tx: mirror_tx.clone(),
            client_count: Arc::clone(&client_count),
            last_mirrored_approval: std::sync::Mutex::new(None),
            last_status: std::sync::Mutex::new(None),
        });

        // Accept clients: each gets a broadcast subscription (outbound) and an
        // inbound forwarder task.
        tokio::spawn(accept_clients(
            client_rx,
            mirror_tx.clone(),
            event_tx,
            token,
            client_count,
        ));

        // Heartbeat: keep idle connections + NAT mappings alive.
        let host_for_hb = Arc::downgrade(&host);
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(std::time::Duration::from_secs(20));
            tick.tick().await; // skip the immediate first tick
            loop {
                tick.tick().await;
                match host_for_hb.upgrade() {
                    Some(h) => h.mirror(RemoteEnvelope::Heartbeat),
                    None => break, // host dropped — stop the heartbeat
                }
            }
        });

        Ok(host)
    }

    /// Mirror an envelope to all connected clients. Non-blocking — silently
    /// drops if no client is subscribed.
    pub fn mirror(&self, envelope: RemoteEnvelope) {
        if self.client_count.load(Ordering::Relaxed) == 0 {
            return;
        }
        let seq = self.out_seq.fetch_add(1, Ordering::Relaxed);
        let frame = auth::build_signed_frame(&self.token, seq, envelope);
        // `send` only errors when there are zero receivers, which races the
        // client_count guard above (a client disconnecting mid-mirror). That's
        // benign — the frame simply has no audience.
        if self.mirror_tx.send(frame).is_err() {
            debug!(target: "jfc::remote", "mirror: no subscribers (client disconnected mid-send)");
        }
    }

    /// Mirror a `PermissionRequest` for the app's current pending approval,
    /// at most once per distinct tool. Called from the event loop after each
    /// burst so a remote client learns a tool is awaiting approval. `diff` is
    /// a plain-text preview of the pending change (Edit/Write/patch/Bash).
    pub fn mirror_pending_approval(
        &self,
        tool_use_id: &str,
        tool_name: &str,
        summary: String,
        diff: Option<String>,
    ) {
        {
            let mut last = self.last_mirrored_approval.lock().unwrap();
            if last.as_deref() == Some(tool_use_id) {
                return; // already mirrored this one
            }
            *last = Some(tool_use_id.to_string());
        }
        self.mirror(RemoteEnvelope::PermissionRequest {
            tool_use_id: tool_use_id.to_string(),
            tool_name: tool_name.to_string(),
            summary,
            diff,
        });
    }

    /// Mirror a session status, but only when it differs from the last one
    /// sent (so we don't flood Running on every chunk). Called post-burst.
    pub fn mirror_status(&self, status: SessionState) {
        {
            let mut last = self.last_status.lock().unwrap();
            if *last == Some(status) {
                return;
            }
            *last = Some(status);
        }
        self.mirror(RemoteEnvelope::SessionStatus {
            status,
            message: None,
        });
    }

    /// Clear the mirrored-approval marker when the pending approval resolves,
    /// so a future approval of the same tool id re-mirrors.
    pub fn clear_pending_approval(&self) {
        *self.last_mirrored_approval.lock().unwrap() = None;
    }

    /// Shut down the WS server.
    pub fn shutdown(&self) {
        self.server.shutdown();
    }

    /// The address the server is listening on.
    pub fn addr(&self) -> SocketAddr {
        self.server.addr
    }
}

// ─── Client acceptor ─────────────────────────────────────────────────────────

/// Accept incoming client connections. Each client subscribes to the
/// broadcast for outbound frames and runs an inbound forwarder. Supports
/// multiple simultaneous clients.
async fn accept_clients(
    mut client_rx: tokio::sync::mpsc::Receiver<(TransportSender, TransportReceiver)>,
    mirror_tx: broadcast::Sender<RemoteFrame>,
    event_tx: EventSender,
    token: String,
    client_count: Arc<AtomicUsize>,
) {
    while let Some((client_out_tx, client_in_rx)) = client_rx.recv().await {
        let n = client_count.fetch_add(1, Ordering::Relaxed) + 1;
        info!(target: "jfc::remote", clients = n, "client connected");

        let mirror_rx = mirror_tx.subscribe();
        let cc = Arc::clone(&client_count);
        let etx = event_tx.clone();
        let tok = token.clone();

        tokio::spawn(run_client_bridge(
            client_out_tx,
            client_in_rx,
            mirror_rx,
            etx,
            tok,
            cc,
        ));
    }
}

/// Per-client bridge: outbound broadcast → WS, inbound WS → event bus.
async fn run_client_bridge(
    client_out_tx: TransportSender,
    mut client_in_rx: TransportReceiver,
    mut mirror_rx: broadcast::Receiver<RemoteFrame>,
    event_tx: EventSender,
    token: String,
    client_count: Arc<AtomicUsize>,
) {
    // Outbound forwarder: broadcast → this client's WS.
    let out_task = tokio::spawn(async move {
        loop {
            match mirror_rx.recv().await {
                Ok(frame) => {
                    if client_out_tx.send(frame).await.is_err() {
                        break;
                    }
                }
                Err(broadcast::error::RecvError::Lagged(skipped)) => {
                    warn!(target: "jfc::remote", skipped, "client lagged — dropping frames");
                }
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
    });

    // Inbound forwarder: client WS → event bus.
    client_inbound_loop(&mut client_in_rx, &event_tx, &token).await;
    out_task.abort();

    let remaining = client_count.fetch_sub(1, Ordering::Relaxed) - 1;
    info!(target: "jfc::remote", clients = remaining, "client disconnected");
}

/// Inbound loop for one client: reads frames, verifies HMAC + seq,
/// translates envelopes into AppEvents.
async fn client_inbound_loop(rx: &mut TransportReceiver, event_tx: &EventSender, token: &str) {
    let mut seq_tracker = auth::SeqTracker::new();

    while let Some(frame) = rx.recv().await {
        if !auth::verify_frame(token, &frame) {
            warn!(target: "jfc::remote", seq = frame.seq, "HMAC verification failed");
            continue;
        }
        if !seq_tracker.accept(frame.seq) {
            warn!(target: "jfc::remote", seq = frame.seq, "sequence replay rejected");
            continue;
        }
        if let Some(app_event) = translate_inbound(&frame.payload)
            && event_tx.send(app_event).await.is_err()
        {
            debug!(target: "jfc::remote", "event bus closed — stopping inbound loop");
            break;
        }
    }
}

/// Translate an inbound client envelope into an AppEvent.
fn translate_inbound(envelope: &RemoteEnvelope) -> Option<AppEvent> {
    match envelope {
        RemoteEnvelope::UserPrompt { text } => {
            debug!(target: "jfc::remote", len = text.len(), "remote user prompt");
            Some(AppEvent::Ui(UiEvent::Submit(text.clone())))
        }
        RemoteEnvelope::Interrupt => {
            debug!(target: "jfc::remote", "remote interrupt");
            Some(AppEvent::Ui(UiEvent::Term(key_event(
                crossterm::event::KeyCode::Esc,
            ))))
        }
        RemoteEnvelope::ApprovalResponse {
            tool_use_id,
            approved,
        } => Some(AppEvent::Ui(UiEvent::RemoteApprovalResponse {
            tool_use_id: tool_use_id.clone(),
            approved: *approved,
        })),
        RemoteEnvelope::PlanApprovalResponse { approve, .. } => {
            let code = if *approve {
                crossterm::event::KeyCode::Char('y')
            } else {
                crossterm::event::KeyCode::Char('n')
            };
            Some(AppEvent::Ui(UiEvent::Term(key_event(code))))
        }
        RemoteEnvelope::Ping => None,
        other => {
            debug!(target: "jfc::remote", ?other, "ignoring outbound envelope from client");
            None
        }
    }
}

/// Build a `crossterm` key event with no modifiers.
fn key_event(code: crossterm::event::KeyCode) -> crossterm::event::Event {
    crossterm::event::Event::Key(crossterm::event::KeyEvent::new(
        code,
        crossterm::event::KeyModifiers::NONE,
    ))
}

// ─── AppEvent → RemoteEnvelope conversion ────────────────────────────────────

/// Try to convert an `AppEvent` into a `RemoteEnvelope` for mirroring.
/// Returns `None` for events that aren't meaningful to remote clients
/// (ticks, internal provider state, etc.).
pub fn mirror_event(ev: &AppEvent) -> Option<RemoteEnvelope> {
    use crate::runtime::*;

    match ev {
        AppEvent::Stream(StreamEvent::Chunk { text, reasoning }) => {
            Some(RemoteEnvelope::AssistantDelta {
                text: text.clone(),
                reasoning: reasoning.clone(),
            })
        }
        AppEvent::Stream(StreamEvent::Tool(tool)) => Some(RemoteEnvelope::ToolUse {
            id: tool.id.to_string(),
            name: tool.kind.label().to_string(),
            input_preview: Some(tool.input.summary()),
        }),
        AppEvent::Tool(ToolEvent::Result { tool_id, result }) => Some(RemoteEnvelope::ToolResult {
            id: tool_id.to_string(),
            output_preview: Some(result.output.chars().take(500).collect()),
            is_error: result.is_error(),
        }),
        AppEvent::Tool(ToolEvent::SetInProgressToolUseIds { action, ids }) => {
            Some(RemoteEnvelope::SetInProgressToolUseIds {
                action: action.clone(),
                ids: ids.clone(),
            })
        }
        AppEvent::Tool(ToolEvent::DeferredToolUse {
            id,
            name,
            input_preview,
            reason,
        }) => Some(RemoteEnvelope::DeferredToolUse {
            id: id.clone(),
            name: name.clone(),
            input_preview: input_preview.clone(),
            reason: reason.clone(),
        }),
        AppEvent::Tool(ToolEvent::UseSummary {
            summary,
            preceding_tool_use_ids,
        }) => Some(RemoteEnvelope::ToolUseSummary {
            summary: summary.clone(),
            preceding_tool_use_ids: preceding_tool_use_ids.clone(),
            timestamp: Some(chrono::Utc::now().to_rfc3339()),
        }),
        // Done/Idle transitions are derived post-burst from `app.is_streaming`
        // in the event loop (see `mirror_status`). Errors carry a message, so
        // they're mirrored directly here.
        AppEvent::Stream(StreamEvent::Error(e)) => Some(RemoteEnvelope::SessionStatus {
            status: SessionState::Error,
            message: Some(e.clone()),
        }),
        AppEvent::Ui(UiEvent::Toast { kind, text }) => Some(RemoteEnvelope::Toast {
            kind: format!("{kind:?}").to_lowercase(),
            text: text.clone(),
        }),
        AppEvent::Ui(UiEvent::ExitPlanModeRequested { plan }) => {
            Some(RemoteEnvelope::PlanApprovalRequest { plan: plan.clone() })
        }
        _ => None,
    }
}

/// Build a plain-text diff preview for a pending tool (the same content the
/// approval modal shows, but as a string for the RC wire). Returns `None`
/// for tools with nothing to preview (Read, Grep, etc.).
pub fn tool_diff_preview(tool: &crate::types::ToolCall) -> Option<String> {
    use jfc_core::ToolInput;
    match &tool.input {
        ToolInput::Edit {
            file_path,
            old_string,
            new_string,
            ..
        } => {
            let mut s = format!("--- {file_path}\n+++ {file_path}\n");
            for ln in old_string.lines().take(30) {
                s.push_str(&format!("- {ln}\n"));
            }
            for ln in new_string.lines().take(30) {
                s.push_str(&format!("+ {ln}\n"));
            }
            Some(s)
        }
        ToolInput::Write { file_path, content } => {
            let mut s = format!("+++ {file_path} ({} bytes)\n", content.len());
            for ln in content.lines().take(40) {
                s.push_str(&format!("+ {ln}\n"));
            }
            Some(s)
        }
        ToolInput::ApplyPatch { patch } => Some(patch.chars().take(2000).collect()),
        ToolInput::MultiEdit { file_path, edits } => {
            let count = edits.as_array().map(|a| a.len()).unwrap_or(0);
            let mut s = format!("MultiEdit {file_path} ({count} edits)\n");
            if let Some(arr) = edits.as_array() {
                for (i, edit) in arr.iter().take(5).enumerate() {
                    let old = edit
                        .get("old_string")
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    let new = edit
                        .get("new_string")
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    s.push_str(&format!("@@ edit {}/{count} @@\n", i + 1));
                    for ln in old.lines().take(8) {
                        s.push_str(&format!("- {ln}\n"));
                    }
                    for ln in new.lines().take(8) {
                        s.push_str(&format!("+ {ln}\n"));
                    }
                }
            }
            Some(s)
        }
        ToolInput::Bash { command, .. } => Some(format!("$ {command}")),
        _ => None,
    }
}

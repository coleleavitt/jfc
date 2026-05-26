//! Host-side remote control bridge.
//!
//! `RemoteHost` manages the WebSocket server + per-client transport and
//! provides two integrations:
//!
//! 1. **Mirror**: the event loop calls [`RemoteHost::mirror`] for each
//!    relevant `AppEvent`, translating it into a `RemoteEnvelope` and
//!    sending to the connected client. Non-blocking (`try_send`) so a
//!    slow or disconnected client never stalls the host.
//!
//! 2. **Inject**: a spawned task reads inbound envelopes from the client
//!    and translates them into `AppEvent`s injected via the existing
//!    event-loop `tx: EventSender`.

use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

use tokio::sync::mpsc;
use tracing::{debug, info, warn};

use jfc_remote::auth;
use jfc_remote::protocol::{RemoteEnvelope, SessionState};
use jfc_remote::transport::TransportReceiver;
use jfc_remote::ws::WsServer;

use crate::runtime::{AppEvent, EventSender, UiEvent};

/// State for the remote-control host side.
pub struct RemoteHost {
    /// The running WS server (owns the accept loop).
    server: WsServer,
    /// The pairing token for this session.
    pub token: String,
    /// Next outbound sequence number (monotonically increasing).
    out_seq: AtomicU64,
    /// Outbound mirror channel — frames sent here are forwarded to the
    /// connected client by a bridge task.
    mirror_tx: mpsc::Sender<jfc_remote::protocol::RemoteFrame>,
    /// Number of connected clients.
    pub client_count: Arc<AtomicUsize>,
}

impl RemoteHost {
    /// Start the remote-control server. Spawns the client acceptor
    /// and returns the host handle.
    pub async fn start(port: u16, event_tx: EventSender) -> std::io::Result<Arc<Self>> {
        let token = auth::generate_token();
        let addr: SocketAddr = format!("127.0.0.1:{port}").parse().unwrap();
        let (server, client_rx) = WsServer::bind(addr, token.clone()).await?;

        info!(target: "jfc::remote", addr = %server.addr, "remote-control server started");

        // Mirror channel: host mirror() → bridge task → client WS.
        let (mirror_tx, mirror_rx) = mpsc::channel(512);
        let client_count = Arc::new(AtomicUsize::new(0));

        let host = Arc::new(Self {
            server,
            token: token.clone(),
            out_seq: AtomicU64::new(1),
            mirror_tx,
            client_count: Arc::clone(&client_count),
        });

        // Spawn the client-acceptor: handles one client at a time (MVP).
        tokio::spawn(accept_clients(
            client_rx,
            mirror_rx,
            event_tx,
            token,
            client_count,
        ));

        Ok(host)
    }

    /// Mirror an envelope to the connected client. Non-blocking — silently
    /// drops if the channel is full or no client is connected.
    pub fn mirror(&self, envelope: RemoteEnvelope) {
        if self.client_count.load(Ordering::Relaxed) == 0 {
            return;
        }
        let seq = self.out_seq.fetch_add(1, Ordering::Relaxed);
        let frame = auth::build_signed_frame(&self.token, seq, envelope);
        if self.mirror_tx.try_send(frame).is_err() {
            debug!(target: "jfc::remote", "mirror channel full or closed — dropping frame");
        }
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

/// Accept incoming client connections. For the MVP, supports one client at a
/// time — a new connection replaces the previous one.
async fn accept_clients(
    mut client_rx: mpsc::Receiver<(
        jfc_remote::transport::TransportSender,
        jfc_remote::transport::TransportReceiver,
    )>,
    mut mirror_rx: mpsc::Receiver<jfc_remote::protocol::RemoteFrame>,
    event_tx: EventSender,
    token: String,
    client_count: Arc<AtomicUsize>,
) {
    while let Some((client_out_tx, mut client_in_rx)) = client_rx.recv().await {
        let n = client_count.fetch_add(1, Ordering::Relaxed) + 1;
        info!(target: "jfc::remote", clients = n, "client connected");

        // Spawn inbound forwarder: client → host event bus.
        let etx = event_tx.clone();
        let tok = token.clone();
        let cc = Arc::clone(&client_count);
        tokio::spawn(async move {
            client_inbound_loop(&mut client_in_rx, &etx, &tok).await;
            let remaining = cc.fetch_sub(1, Ordering::Relaxed) - 1;
            info!(target: "jfc::remote", clients = remaining, "client disconnected");
        });

        // Bridge mirror channel → this client's WS sender.
        while let Some(frame) = mirror_rx.recv().await {
            if client_out_tx.send(frame).await.is_err() {
                debug!(target: "jfc::remote", "client WS send failed — client disconnected");
                break;
            }
        }
    }
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
        if let Some(app_event) = translate_inbound(&frame.payload) {
            if event_tx.send(app_event).await.is_err() {
                debug!(target: "jfc::remote", "event bus closed — stopping inbound loop");
                break;
            }
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
            let esc = crossterm::event::Event::Key(crossterm::event::KeyEvent::new(
                crossterm::event::KeyCode::Esc,
                crossterm::event::KeyModifiers::NONE,
            ));
            Some(AppEvent::Ui(UiEvent::Term(esc)))
        }
        RemoteEnvelope::ApprovalResponse { approved, .. } => {
            let key = if *approved {
                crossterm::event::KeyCode::Char('y')
            } else {
                crossterm::event::KeyCode::Char('n')
            };
            let ev = crossterm::event::Event::Key(crossterm::event::KeyEvent::new(
                key,
                crossterm::event::KeyModifiers::NONE,
            ));
            Some(AppEvent::Ui(UiEvent::Term(ev)))
        }
        RemoteEnvelope::PlanApprovalResponse { approve, .. } => {
            let key = if *approve {
                crossterm::event::KeyCode::Char('y')
            } else {
                crossterm::event::KeyCode::Char('n')
            };
            let ev = crossterm::event::Event::Key(crossterm::event::KeyEvent::new(
                key,
                crossterm::event::KeyModifiers::NONE,
            ));
            Some(AppEvent::Ui(UiEvent::Term(ev)))
        }
        RemoteEnvelope::Ping => None,
        other => {
            debug!(target: "jfc::remote", ?other, "ignoring outbound envelope from client");
            None
        }
    }
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
        AppEvent::Stream(StreamEvent::Done(_)) => Some(RemoteEnvelope::SessionStatus {
            status: SessionState::Idle,
            message: None,
        }),
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

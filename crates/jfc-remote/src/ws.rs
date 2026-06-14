//! WebSocket transport: server + client over `tokio-tungstenite`.
//!
//! The server binds to `127.0.0.1:<port>` and upgrades incoming HTTP
//! connections to WebSocket. Bearer-token auth is enforced on the HTTP
//! upgrade: the client must send `Sec-WebSocket-Protocol: bearer.<token>`.
//!
//! TLS is **not** handled here — the assumption is that the WS runs on
//! localhost and the user exposes it via an encrypted tunnel (Tailscale,
//! SSH -L, or cloudflared).

use std::net::SocketAddr;
use std::sync::Arc;

use futures_util::{SinkExt, StreamExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, watch};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::handshake::server::{ErrorResponse, Request, Response};
use tokio_tungstenite::tungstenite::http::StatusCode;

use crate::protocol::RemoteFrame;
use crate::transport::{TransportReceiver, TransportSender};

/// Channel buffer for WS ↔ transport bridging.
const WS_BUF: usize = 256;

// ─── Server ──────────────────────────────────────────────────────────────────

/// A running WebSocket remote-control server.
pub struct WsServer {
    /// The local address the server is listening on.
    pub addr: SocketAddr,
    /// Signal to shut the server down.
    shutdown_tx: watch::Sender<bool>,
}

impl WsServer {
    /// Start a WebSocket server on `addr`, authenticating connections against
    /// `token`.
    ///
    /// Returns the `WsServer` handle (for shutdown) plus a future that
    /// receives `(TransportSender, TransportReceiver)` for each accepted
    /// client. The caller should spawn the accept loop.
    pub async fn bind(
        addr: SocketAddr,
        token: String,
    ) -> std::io::Result<(Self, mpsc::Receiver<(TransportSender, TransportReceiver)>)> {
        let listener = TcpListener::bind(addr).await?;
        let local_addr = listener.local_addr()?;
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let (client_tx, client_rx) = mpsc::channel(8);

        let token = Arc::new(token);
        tokio::spawn(accept_loop(listener, token, client_tx, shutdown_rx));

        Ok((
            Self {
                addr: local_addr,
                shutdown_tx,
            },
            client_rx,
        ))
    }

    /// Shut down the server (stops accepting new connections).
    pub fn shutdown(&self) {
        let _ = self.shutdown_tx.send(true);
    }
}

async fn accept_loop(
    listener: TcpListener,
    token: Arc<String>,
    client_tx: mpsc::Sender<(TransportSender, TransportReceiver)>,
    mut shutdown_rx: watch::Receiver<bool>,
) {
    loop {
        let accept_result = tokio::select! {
            result = listener.accept() => result,
            _ = shutdown_rx.changed() => {
                if *shutdown_rx.borrow() {
                    tracing::info!(target: "jfc::remote", "WS server shutting down");
                }
                return;
            }
        };
        let (stream, peer) = match accept_result {
            Ok(x) => x,
            Err(e) => {
                tracing::warn!(target: "jfc::remote", error = %e, "accept failed");
                continue;
            }
        };
        let token = Arc::clone(&token);
        let tx = client_tx.clone();
        tokio::spawn(async move {
            if let Err(e) = handle_connection(stream, peer, &token, tx).await {
                tracing::debug!(target: "jfc::remote", %peer, error = %e, "connection rejected");
            }
        });
    }
}

async fn handle_connection(
    stream: TcpStream,
    peer: SocketAddr,
    token: &str,
    client_tx: mpsc::Sender<(TransportSender, TransportReceiver)>,
) -> Result<(), crate::transport::TransportError> {
    let token_owned = token.to_string();

    let ws_stream = tokio_tungstenite::accept_hdr_async(
        stream,
        // The callback signature (and thus the large ErrorResponse Err
        // variant) is dictated by tungstenite's accept_hdr_async API.
        #[allow(clippy::result_large_err)]
        move |req: &Request, mut resp: Response| -> Result<Response, ErrorResponse> {
            // Check bearer token in Sec-WebSocket-Protocol header.
            let expected = format!("bearer.{token_owned}");
            let protocols = req
                .headers()
                .get("Sec-WebSocket-Protocol")
                .and_then(|v| v.to_str().ok())
                .unwrap_or("");

            if protocols.split(',').any(|p| p.trim() == expected) {
                // Echo back the accepted subprotocol.
                resp.headers_mut()
                    .insert("Sec-WebSocket-Protocol", expected.parse().unwrap());
                Ok(resp)
            } else {
                let mut err = ErrorResponse::new(Some("unauthorized".into()));
                *err.status_mut() = StatusCode::UNAUTHORIZED;
                Err(err)
            }
        },
    )
    .await
    .map_err(|e| crate::transport::TransportError::WebSocket(e.to_string()))?;

    tracing::info!(target: "jfc::remote", %peer, "client connected");

    let (mut ws_tx, mut ws_rx) = ws_stream.split();

    // Bridge WS ↔ transport channels.
    let (outbound_tx, mut outbound_rx) = mpsc::channel::<RemoteFrame>(WS_BUF);
    let (inbound_tx, inbound_rx) = mpsc::channel::<RemoteFrame>(WS_BUF);

    // Outbound: transport → WS
    tokio::spawn(async move {
        while let Some(frame) = outbound_rx.recv().await {
            let json = match serde_json::to_string(&frame) {
                Ok(j) => j,
                Err(_) => continue,
            };
            if ws_tx.send(Message::Text(json)).await.is_err() {
                break;
            }
        }
    });

    // Inbound: WS → transport
    tokio::spawn(async move {
        while let Some(Ok(msg)) = ws_rx.next().await {
            let text = match msg {
                Message::Text(t) => t,
                Message::Ping(_) | Message::Pong(_) => continue,
                Message::Close(_) => break,
                _ => continue,
            };
            let frame: RemoteFrame = match serde_json::from_str(&text) {
                Ok(f) => f,
                Err(_) => continue,
            };
            if inbound_tx.send(frame).await.is_err() {
                break;
            }
        }
    });

    // Hand the transport pair to the caller. If the receiver is gone the
    // host is shutting down — nothing actionable, but record it.
    if client_tx
        .send((
            TransportSender::new(outbound_tx),
            TransportReceiver::new(inbound_rx),
        ))
        .await
        .is_err()
    {
        tracing::debug!(target: "jfc::remote", %peer, "host dropped client receiver");
    }

    Ok(())
}

// ─── Client ──────────────────────────────────────────────────────────────────

/// Connect to a remote-control server as a client.
///
/// The `url` should be `ws://host:port` (or `wss://` if going through a
/// TLS-terminating tunnel). The `token` is sent as a WS subprotocol.
pub async fn connect(
    url: &str,
    token: &str,
) -> Result<(TransportSender, TransportReceiver), crate::transport::TransportError> {
    use tokio_tungstenite::tungstenite::http::Request as WsRequest;

    let subprotocol = format!("bearer.{token}");

    let request = WsRequest::builder()
        .uri(url)
        .header("Sec-WebSocket-Protocol", &subprotocol)
        .header("Host", extract_host(url))
        .header("Connection", "Upgrade")
        .header("Upgrade", "websocket")
        .header("Sec-WebSocket-Version", "13")
        .header(
            "Sec-WebSocket-Key",
            tokio_tungstenite::tungstenite::handshake::client::generate_key(),
        )
        .body(())
        .map_err(|e| crate::transport::TransportError::Other(e.to_string()))?;

    let (ws_stream, _resp) = tokio_tungstenite::connect_async(request)
        .await
        .map_err(|e| crate::transport::TransportError::WebSocket(e.to_string()))?;

    let (mut ws_tx, mut ws_rx) = ws_stream.split();

    let (outbound_tx, mut outbound_rx) = mpsc::channel::<RemoteFrame>(WS_BUF);
    let (inbound_tx, inbound_rx) = mpsc::channel::<RemoteFrame>(WS_BUF);

    // Outbound: transport → WS
    tokio::spawn(async move {
        while let Some(frame) = outbound_rx.recv().await {
            let json = match serde_json::to_string(&frame) {
                Ok(j) => j,
                Err(_) => continue,
            };
            if ws_tx.send(Message::Text(json)).await.is_err() {
                break;
            }
        }
    });

    // Inbound: WS → transport
    tokio::spawn(async move {
        while let Some(Ok(msg)) = ws_rx.next().await {
            let text = match msg {
                Message::Text(t) => t,
                Message::Ping(_) | Message::Pong(_) => continue,
                Message::Close(_) => break,
                _ => continue,
            };
            let frame: RemoteFrame = match serde_json::from_str(&text) {
                Ok(f) => f,
                Err(_) => continue,
            };
            if inbound_tx.send(frame).await.is_err() {
                break;
            }
        }
    });

    Ok((
        TransportSender::new(outbound_tx),
        TransportReceiver::new(inbound_rx),
    ))
}

fn extract_host(url: &str) -> String {
    url.trim_start_matches("ws://")
        .trim_start_matches("wss://")
        .split('/')
        .next()
        .unwrap_or("localhost")
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth;
    use crate::protocol::RemoteEnvelope;

    #[tokio::test]
    async fn ws_server_client_roundtrip() {
        let token = auth::generate_token();
        let addr: SocketAddr = "127.0.0.1:0".parse().unwrap();

        let (server, mut clients) = WsServer::bind(addr, token.clone()).await.unwrap();
        let url = format!("ws://127.0.0.1:{}", server.addr.port());

        // Connect as client.
        let (client_tx, mut client_rx) = connect(&url, &token).await.unwrap();

        // Accept on server side.
        let (host_tx, mut host_rx) = clients.recv().await.unwrap();

        // Host → client.
        let frame = auth::build_signed_frame(&token, 1, RemoteEnvelope::Heartbeat);
        host_tx.send(frame).await.unwrap();
        let received = client_rx.recv().await.unwrap();
        assert_eq!(received.seq, 1);
        assert!(auth::verify_frame(&token, &received));

        // Client → host.
        let frame = auth::build_signed_frame(
            &token,
            1,
            RemoteEnvelope::UserPrompt {
                text: "hello".into(),
            },
        );
        client_tx.send(frame).await.unwrap();
        let received = host_rx.recv().await.unwrap();
        assert_eq!(
            received.payload,
            RemoteEnvelope::UserPrompt {
                text: "hello".into()
            }
        );

        server.shutdown();
    }

    #[tokio::test]
    async fn ws_wrong_token_rejected() {
        let token = auth::generate_token();
        let addr: SocketAddr = "127.0.0.1:0".parse().unwrap();

        let (server, _clients) = WsServer::bind(addr, token.clone()).await.unwrap();
        let url = format!("ws://127.0.0.1:{}", server.addr.port());

        let wrong_token = auth::generate_token();
        let result = connect(&url, &wrong_token).await;
        assert!(result.is_err());

        server.shutdown();
    }
}

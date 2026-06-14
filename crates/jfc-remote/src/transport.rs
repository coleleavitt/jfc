//! Transport abstraction for the remote-control protocol.
//!
//! `RemoteTransport` is the seam that makes localhost-WS, SSH-tunnel, and
//! relay all just different implementations. The protocol layer (auth, framing,
//! envelope) is transport-agnostic.

use tokio::sync::mpsc;

use crate::protocol::RemoteFrame;

/// Errors from the transport layer.
#[derive(Debug, thiserror::Error)]
pub enum TransportError {
    #[error("connection closed")]
    Closed,
    #[error("send failed: {0}")]
    Send(String),
    #[error("receive failed: {0}")]
    Recv(String),
    #[error("authentication failed: {0}")]
    Auth(String),
    #[error("websocket error: {0}")]
    WebSocket(String),
    #[error("{0}")]
    Other(String),
}

/// Send half of a transport.
pub struct TransportSender {
    tx: mpsc::Sender<RemoteFrame>,
}

impl TransportSender {
    pub fn new(tx: mpsc::Sender<RemoteFrame>) -> Self {
        Self { tx }
    }

    /// Send a frame. Returns `Err(TransportError::Closed)` if the peer
    /// has disconnected.
    pub async fn send(&self, frame: RemoteFrame) -> Result<(), TransportError> {
        self.tx
            .send(frame)
            .await
            .map_err(|_| TransportError::Closed)
    }

    /// Non-blocking send. Returns `Err` if the channel is full or closed.
    pub fn try_send(&self, frame: RemoteFrame) -> Result<(), TransportError> {
        self.tx.try_send(frame).map_err(|e| match e {
            mpsc::error::TrySendError::Full(_) => TransportError::Send("channel full".into()),
            mpsc::error::TrySendError::Closed(_) => TransportError::Closed,
        })
    }
}

/// Receive half of a transport.
pub struct TransportReceiver {
    rx: mpsc::Receiver<RemoteFrame>,
}

impl TransportReceiver {
    pub fn new(rx: mpsc::Receiver<RemoteFrame>) -> Self {
        Self { rx }
    }

    /// Receive the next frame. Returns `None` when the peer closes.
    pub async fn recv(&mut self) -> Option<RemoteFrame> {
        self.rx.recv().await
    }
}

/// Channel buffer size for transport channels.
const CHANNEL_BUF: usize = 256;

/// Create a pair of in-memory transports connected back-to-back.
///
/// Returns `(host_side, client_side)` where:
/// - `host_side.0` sends frames that `client_side.1` receives
/// - `client_side.0` sends frames that `host_side.1` receives
///
/// This is the `LoopbackTransport` — used for integration testing the
/// protocol without any network.
pub fn loopback() -> (
    (TransportSender, TransportReceiver),
    (TransportSender, TransportReceiver),
) {
    let (host_tx, client_rx) = mpsc::channel(CHANNEL_BUF);
    let (client_tx, host_rx) = mpsc::channel(CHANNEL_BUF);
    (
        (
            TransportSender::new(host_tx),
            TransportReceiver::new(host_rx),
        ),
        (
            TransportSender::new(client_tx),
            TransportReceiver::new(client_rx),
        ),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::{PROTOCOL_VERSION, RemoteEnvelope, RemoteFrame};

    fn test_frame(seq: u64) -> RemoteFrame {
        RemoteFrame {
            version: PROTOCOL_VERSION,
            seq,
            ts_ms: 0,
            payload: RemoteEnvelope::Heartbeat,
            hmac: String::new(),
        }
    }

    #[tokio::test]
    async fn loopback_roundtrip_host_to_client() {
        let ((host_tx, _host_rx), (_client_tx, mut client_rx)) = loopback();
        let frame = test_frame(1);
        host_tx.send(frame.clone()).await.unwrap();
        let received = client_rx.recv().await.unwrap();
        assert_eq!(received.seq, 1);
    }

    #[tokio::test]
    async fn loopback_roundtrip_client_to_host() {
        let ((_host_tx, mut host_rx), (client_tx, _client_rx)) = loopback();
        let frame = test_frame(42);
        client_tx.send(frame).await.unwrap();
        let received = host_rx.recv().await.unwrap();
        assert_eq!(received.seq, 42);
    }

    #[tokio::test]
    async fn loopback_bidirectional() {
        let ((host_tx, mut host_rx), (client_tx, mut client_rx)) = loopback();

        host_tx.send(test_frame(1)).await.unwrap();
        client_tx.send(test_frame(2)).await.unwrap();

        let from_host = client_rx.recv().await.unwrap();
        let from_client = host_rx.recv().await.unwrap();

        assert_eq!(from_host.seq, 1);
        assert_eq!(from_client.seq, 2);
    }

    #[tokio::test]
    async fn drop_sender_closes_receiver() {
        let ((host_tx, _host_rx), (_client_tx, mut client_rx)) = loopback();
        drop(host_tx);
        assert!(client_rx.recv().await.is_none());
    }
}

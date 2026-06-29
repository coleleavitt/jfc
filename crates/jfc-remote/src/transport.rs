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
        linkscope::record_items("remote.transport.sender.new", 1);
        Self { tx }
    }

    /// Send a frame. Returns `Err(TransportError::Closed)` if the peer
    /// has disconnected.
    pub async fn send(&self, frame: RemoteFrame) -> Result<(), TransportError> {
        let _linkscope_send = linkscope::phase("remote.transport.send");
        trace_frame("remote.transport.send.start", &frame);
        let result = self
            .tx
            .send(frame)
            .await
            .map_err(|_| TransportError::Closed);
        linkscope::record_items(
            if result.is_ok() {
                "remote.transport.send.ok"
            } else {
                "remote.transport.send.closed"
            },
            1,
        );
        result
    }

    /// Non-blocking send. Returns `Err` if the channel is full or closed.
    pub fn try_send(&self, frame: RemoteFrame) -> Result<(), TransportError> {
        let _linkscope_try_send = linkscope::phase("remote.transport.try_send");
        trace_frame("remote.transport.try_send.start", &frame);
        self.tx.try_send(frame).map_err(|e| match e {
            mpsc::error::TrySendError::Full(_) => {
                linkscope::record_items("remote.transport.try_send.full", 1);
                TransportError::Send("channel full".into())
            }
            mpsc::error::TrySendError::Closed(_) => {
                linkscope::record_items("remote.transport.try_send.closed", 1);
                TransportError::Closed
            }
        })?;
        linkscope::record_items("remote.transport.try_send.ok", 1);
        Ok(())
    }
}

/// Receive half of a transport.
pub struct TransportReceiver {
    rx: mpsc::Receiver<RemoteFrame>,
}

impl TransportReceiver {
    pub fn new(rx: mpsc::Receiver<RemoteFrame>) -> Self {
        linkscope::record_items("remote.transport.receiver.new", 1);
        Self { rx }
    }

    /// Receive the next frame. Returns `None` when the peer closes.
    pub async fn recv(&mut self) -> Option<RemoteFrame> {
        let _linkscope_recv = linkscope::phase("remote.transport.recv");
        let frame = self.rx.recv().await;
        match &frame {
            Some(frame) => {
                linkscope::record_items("remote.transport.recv.frame", 1);
                trace_frame("remote.transport.recv.detail", frame);
            }
            None => {
                linkscope::record_items("remote.transport.recv.closed", 1);
            }
        }
        frame
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
    let _linkscope_loopback = linkscope::phase("remote.transport.loopback");
    linkscope::record_items("remote.transport.loopback.created", 1);
    linkscope::record_items(
        "remote.transport.loopback.capacity",
        usize_to_u64_saturating(CHANNEL_BUF),
    );
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

fn trace_frame(label: &'static str, frame: &RemoteFrame) {
    if !linkscope::trace_detail_enabled() {
        return;
    }
    linkscope::detail_event_fields(
        label,
        [
            linkscope::TraceField::count("version", u64::from(frame.version)),
            linkscope::TraceField::count("seq", frame.seq),
            linkscope::TraceField::text("payload_kind", frame.payload.kind()),
            linkscope::TraceField::count("outbound", u64::from(frame.payload.is_outbound())),
        ],
    );
}

fn usize_to_u64_saturating(value: usize) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
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

    #[tokio::test]
    async fn transport_trace_records_frame_shape_without_payload_normal() {
        linkscope::trace_detail_enable();
        let ((host_tx, _host_rx), (_client_tx, mut client_rx)) = loopback();
        host_tx
            .send(RemoteFrame {
                version: PROTOCOL_VERSION,
                seq: 99,
                ts_ms: 0,
                payload: RemoteEnvelope::UserPrompt {
                    text: "private prompt".into(),
                },
                hmac: String::new(),
            })
            .await
            .unwrap();
        assert_eq!(client_rx.recv().await.unwrap().seq, 99);

        let snapshot = linkscope::snapshot();
        let rendered = format!("{snapshot:?}");
        assert!(rendered.contains("remote.transport.send.start"));
        assert!(rendered.contains("remote.transport.recv.detail"));
        assert!(rendered.contains("user_prompt"));
        assert!(!rendered.contains("private prompt"));
    }
}

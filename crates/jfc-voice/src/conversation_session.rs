use anyhow::{Context, Result};
use futures_util::{SinkExt, StreamExt};
use std::time::Duration;
use tokio::net::TcpStream;
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

use crate::conversation_ws::{self, ClientEvent, ServerEvent, VoiceConversationOptions};

const EVENT_BUFFER: usize = 128;
const COMMAND_BUFFER: usize = 64;
const KEEPALIVE_INTERVAL: Duration = Duration::from_millis(4000);

#[derive(Debug, Clone, PartialEq)]
pub enum VoiceConversationEvent {
    Server(ServerEvent),
    Audio(Vec<u8>),
    Closed,
    Error(String),
}

enum SessionCommand {
    Audio(Vec<u8>),
    Client(ClientEvent),
    Close,
}

pub struct VoiceConversationSession {
    command_tx: mpsc::Sender<SessionCommand>,
}

impl VoiceConversationSession {
    pub async fn send_audio(&self, pcm_or_opus: &[u8]) -> bool {
        self.command_tx
            .send(SessionCommand::Audio(pcm_or_opus.to_vec()))
            .await
            .is_ok()
    }

    pub async fn send_client_event(&self, event: ClientEvent) -> bool {
        self.command_tx
            .send(SessionCommand::Client(event))
            .await
            .is_ok()
    }

    pub fn close(&self) -> bool {
        self.command_tx.try_send(SessionCommand::Close).is_ok()
    }
}

pub async fn connect(
    base_wss: &str,
    token: &str,
    user_agent: &str,
    opts: &VoiceConversationOptions,
) -> Result<(
    VoiceConversationSession,
    mpsc::Receiver<VoiceConversationEvent>,
)> {
    let request = conversation_ws::build_request(base_wss, token, user_agent, opts)?;
    let (ws, _response) = tokio_tungstenite::connect_async(request)
        .await
        .context("voice conversation WebSocket connect/upgrade failed")?;
    Ok(spawn_session(ws))
}

pub fn spawn_session(
    ws: WebSocketStream<MaybeTlsStream<TcpStream>>,
) -> (
    VoiceConversationSession,
    mpsc::Receiver<VoiceConversationEvent>,
) {
    let (command_tx, command_rx) = mpsc::channel(COMMAND_BUFFER);
    let (event_tx, event_rx) = mpsc::channel(EVENT_BUFFER);
    tokio::spawn(session_loop(ws, command_rx, event_tx));
    (VoiceConversationSession { command_tx }, event_rx)
}

async fn session_loop<S>(
    ws: WebSocketStream<S>,
    mut command_rx: mpsc::Receiver<SessionCommand>,
    event_tx: mpsc::Sender<VoiceConversationEvent>,
) where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    let (mut sink, mut stream) = ws.split();
    let mut keepalive = tokio::time::interval(KEEPALIVE_INTERVAL);
    keepalive.tick().await;

    loop {
        tokio::select! {
            _ = keepalive.tick() => {
                if let Err(err) = send_client_event(&mut sink, ClientEvent::KeepAlive).await {
                    send_event(&event_tx, VoiceConversationEvent::Error(err.to_string())).await;
                    break;
                }
            }
            command = command_rx.recv() => match command {
                Some(SessionCommand::Audio(bytes)) => {
                    if let Err(err) = sink.send(Message::Binary(bytes)).await {
                        send_event(&event_tx, VoiceConversationEvent::Error(err.to_string())).await;
                        break;
                    }
                }
                Some(SessionCommand::Client(event)) => {
                    if let Err(err) = send_client_event(&mut sink, event).await {
                        send_event(&event_tx, VoiceConversationEvent::Error(err.to_string())).await;
                        break;
                    }
                }
                Some(SessionCommand::Close) | None => {
                    let _ = sink.send(Message::Close(None)).await;
                    break;
                }
            },
            message = stream.next() => match message {
                None | Some(Ok(Message::Close(_))) => break,
                Some(Ok(message)) => match ws_message_to_event(message) {
                    Some(event) => send_event(&event_tx, event).await,
                    None => {}
                },
                Some(Err(err)) => {
                    send_event(&event_tx, VoiceConversationEvent::Error(err.to_string())).await;
                    break;
                }
            },
        }
    }
    send_event(&event_tx, VoiceConversationEvent::Closed).await;
}

async fn send_client_event<S>(
    sink: &mut futures_util::stream::SplitSink<WebSocketStream<S>, Message>,
    event: ClientEvent,
) -> Result<()>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let raw = serde_json::to_string(&event)?;
    sink.send(Message::Text(raw)).await?;
    Ok(())
}

async fn send_event(tx: &mpsc::Sender<VoiceConversationEvent>, event: VoiceConversationEvent) {
    let _ = tx.send(event).await;
}

fn ws_message_to_event(message: Message) -> Option<VoiceConversationEvent> {
    match message {
        Message::Binary(bytes) => Some(VoiceConversationEvent::Audio(bytes)),
        Message::Text(raw) => Some(match conversation_ws::parse_server_event(&raw) {
            Ok(event) => VoiceConversationEvent::Server(event),
            Err(err) => VoiceConversationEvent::Error(err.to_string()),
        }),
        Message::Close(_) => Some(VoiceConversationEvent::Closed),
        Message::Ping(_) | Message::Pong(_) | Message::Frame(_) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ws_message_to_event_maps_binary_audio_normal() {
        let event = ws_message_to_event(Message::Binary(vec![1, 2, 3]));

        assert_eq!(event, Some(VoiceConversationEvent::Audio(vec![1, 2, 3])));
    }

    #[test]
    fn ws_message_to_event_parses_server_event_normal() {
        let event = ws_message_to_event(Message::Text(r#"{"type":"playback_start"}"#.to_owned()));

        assert_eq!(
            event,
            Some(VoiceConversationEvent::Server(ServerEvent::PlaybackStart))
        );
    }

    #[test]
    fn ws_message_to_event_reports_bad_json_robust() {
        let event = ws_message_to_event(Message::Text("not-json".to_owned()));

        assert!(matches!(event, Some(VoiceConversationEvent::Error(_))));
    }
}

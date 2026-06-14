//! Anthropic WebSocket speech-to-text client (`/api/ws/speech_to_text/voice_stream`).
//!
//! Protocol reverse-engineered from Claude Code 2.1.167's `connectVoiceStream`
//! (deobfuscated `cli.deobfuscated.js`). It is **fully specified** there, so —
//! unlike the (non-existent) batch REST endpoint — this is a faithful port, not
//! a guess. The handshake, query params, control sentinels, and result message
//! tags below all come verbatim from the CLI.
//!
//! ## Wire protocol
//! - **Connect:** `wss://api.anthropic.com/api/ws/speech_to_text/voice_stream`
//!   with query params:
//!   `encoding=linear16 sample_rate=16000 channels=1 endpointing_ms=300
//!    utterance_end_ms=1000 language=<lang> use_conversation_engine=true
//!    stt_provider=deepgram-nova3`.
//! - **Headers:** `Authorization: Bearer <oauth>`, `x-app: cli`,
//!   `anthropic-client-platform: claude_code_cli`, plus a User-Agent.
//! - **Audio:** raw little-endian 16-bit PCM, sent as **binary** WS frames.
//! - **Control (text frames):** `{"type":"KeepAlive"}` on open + periodically;
//!   `{"type":"CloseStream"}` to flush and end the utterance.
//! - **Results (text frames, JSON):**
//!   - `{"type":"TranscriptInterim"|"TranscriptText","data":"…"}` — partial.
//!   - `{"type":"TranscriptEndpoint"}` — promote the last interim to final.
//!   - `{"type":"TranscriptError","description"|"error_code":"…"}` — failure.
//!
//! Because jfc transcribes a *complete buffered utterance* (batch), we stream
//! all the PCM, send `CloseStream`, then collect transcript text until the
//! socket reports the endpoint / closes. The last non-empty transcript is the
//! result.

use anyhow::{Context, Result};
use futures_util::{SinkExt, StreamExt};
use std::time::Duration;
use tokio_tungstenite::tungstenite::Message;
use tracing::debug;

const KEEPALIVE: &str = r#"{"type":"KeepAlive"}"#;
const CLOSE_STREAM: &str = r#"{"type":"CloseStream"}"#;
/// Bytes of PCM per binary frame (~64ms @ 16kHz/16-bit mono). Small enough to
/// stream smoothly, large enough to avoid frame overhead.
const FRAME_BYTES: usize = 2048;
/// Overall safety timeout for a single batch transcription.
const OVERALL_TIMEOUT: Duration = Duration::from_secs(30);
/// Quiet period after CloseStream before we give up waiting for the endpoint.
const FINALIZE_TIMEOUT: Duration = Duration::from_secs(5);

/// Transcribe a complete PCM buffer (16-bit LE, 16kHz, mono) via the Anthropic
/// voice-stream WebSocket. Returns `Ok(None)` if the server heard nothing.
///
/// `base_wss` is the WS origin (default `wss://api.anthropic.com`, overridable
/// for gateways/tests). `token` is the OAuth access token. `language` is the
/// BCP-47 code (e.g. `en`).
pub async fn transcribe_pcm(
    pcm: &[u8],
    token: &str,
    base_wss: &str,
    language: &str,
    user_agent: &str,
) -> Result<Option<String>> {
    let url = format!(
        "{base}/api/ws/speech_to_text/voice_stream\
         ?encoding=linear16&sample_rate=16000&channels=1\
         &endpointing_ms=300&utterance_end_ms=1000&language={lang}\
         &use_conversation_engine=true&stt_provider=deepgram-nova3",
        base = base_wss.trim_end_matches('/'),
        lang = language,
    );
    debug!(target: "jfc::voice::stt", %url, "voice_stream: connecting");

    // Build the upgrade request with the auth + client headers CC sends.
    use tokio_tungstenite::tungstenite::client::IntoClientRequest;
    let mut req = url
        .as_str()
        .into_client_request()
        .context("invalid voice_stream URL")?;
    {
        let h = req.headers_mut();
        h.insert("Authorization", format!("Bearer {token}").parse()?);
        h.insert("x-app", "cli".parse()?);
        h.insert("anthropic-client-platform", "claude_code_cli".parse()?);
        h.insert("User-Agent", user_agent.parse()?);
    }

    let result = tokio::time::timeout(OVERALL_TIMEOUT, run_session(req, pcm)).await;
    match result {
        Ok(inner) => inner,
        Err(_) => anyhow::bail!("voice_stream timed out after {OVERALL_TIMEOUT:?}"),
    }
}

async fn run_session(
    req: tokio_tungstenite::tungstenite::handshake::client::Request,
    pcm: &[u8],
) -> Result<Option<String>> {
    let (ws, _resp) = tokio_tungstenite::connect_async(req)
        .await
        .context("voice_stream WebSocket connect/upgrade failed")?;
    let (mut sink, mut stream) = ws.split();

    // KeepAlive on open, then stream the PCM as binary frames.
    sink.send(Message::Text(KEEPALIVE.into()))
        .await
        .context("send initial KeepAlive")?;
    for chunk in pcm.chunks(FRAME_BYTES) {
        sink.send(Message::Binary(chunk.to_vec()))
            .await
            .context("send audio frame")?;
    }
    // Flush + end the utterance.
    sink.send(Message::Text(CLOSE_STREAM.into()))
        .await
        .context("send CloseStream")?;

    // Collect transcripts until the endpoint / close, bounded by a quiet timeout.
    let last_transcript = collect_transcript(&mut stream).await?;

    if let Err(e) = sink.send(Message::Close(None)).await {
        debug!(target: "jfc::voice::stt", error = %e, "voice_stream: error sending close (ignored)");
    }
    Ok(nonempty(last_transcript.trim().to_string()))
}

/// Read server frames until the transcript endpoint, a socket close, or a quiet
/// period elapses. Returns the latest (final) transcript text seen.
async fn collect_transcript<S>(stream: &mut S) -> Result<String>
where
    S: futures_util::Stream<Item = tokio_tungstenite::tungstenite::Result<Message>> + Unpin,
{
    let mut last_transcript = String::new();
    loop {
        let msg = match tokio::time::timeout(FINALIZE_TIMEOUT, stream.next()).await {
            Err(_) | Ok(None) => break, // quiet period elapsed / stream ended
            Ok(Some(Err(e))) => return Err(anyhow::anyhow!("voice_stream read error: {e}")),
            Ok(Some(Ok(m))) => m,
        };
        match msg {
            Message::Text(t) => {
                if handle_text(&t, &mut last_transcript)? == Some(true) {
                    break; // TranscriptEndpoint reached
                }
            }
            Message::Close(_) => break,
            // Binary / Ping / Pong frames carry no transcript text; log at trace
            // so a surprising frame type is observable without noise.
            other => {
                tracing::trace!(
                    target: "jfc::voice::stt",
                    kind = ?std::mem::discriminant(&other),
                    "voice_stream: ignoring non-text frame"
                );
            }
        }
    }
    Ok(last_transcript)
}

/// Apply one server text message to the running transcript. Returns
/// `Some(true)` when the endpoint/terminal was reached (stop reading),
/// `Some(false)`/`None` to keep going. Errors on `TranscriptError`.
fn handle_text(raw: &str, last: &mut String) -> Result<Option<bool>> {
    let v: serde_json::Value = match serde_json::from_str(raw) {
        Ok(v) => v,
        Err(_) => return Ok(None), // ignore non-JSON / control echoes
    };
    let kind = v.get("type").and_then(|t| t.as_str()).unwrap_or("");
    match kind {
        "TranscriptInterim" | "TranscriptText" => {
            if let Some(data) = v.get("data").and_then(|d| d.as_str()) {
                if !data.is_empty() {
                    *last = data.to_string();
                }
            }
            Ok(Some(false))
        }
        "TranscriptEndpoint" => Ok(Some(true)),
        "TranscriptError" => {
            let desc = v
                .get("description")
                .or_else(|| v.get("error_code"))
                .and_then(|d| d.as_str())
                .unwrap_or("unknown transcription error");
            Err(anyhow::anyhow!("voice_stream TranscriptError: {desc}"))
        }
        "error" => {
            let desc = v
                .get("message")
                .and_then(|d| d.as_str())
                .unwrap_or("server error");
            Err(anyhow::anyhow!("voice_stream error: {desc}"))
        }
        _ => Ok(None),
    }
}

fn nonempty(s: String) -> Option<String> {
    if s.is_empty() { None } else { Some(s) }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Normal: a TranscriptText updates the running transcript, an Endpoint ends.
    #[test]
    fn handle_text_collects_then_endpoints_normal() {
        let mut last = String::new();
        assert_eq!(
            handle_text(
                r#"{"type":"TranscriptText","data":"hello world"}"#,
                &mut last
            )
            .unwrap(),
            Some(false)
        );
        assert_eq!(last, "hello world");
        // a later interim overwrites with the newest text
        handle_text(
            r#"{"type":"TranscriptInterim","data":"hello world today"}"#,
            &mut last,
        )
        .unwrap();
        assert_eq!(last, "hello world today");
        assert_eq!(
            handle_text(r#"{"type":"TranscriptEndpoint"}"#, &mut last).unwrap(),
            Some(true)
        );
    }

    // Robust: TranscriptError / error become Err; non-JSON and unknown types
    // are ignored (None) without disturbing the transcript.
    #[test]
    fn handle_text_errors_and_ignores_robust() {
        let mut last = String::from("keep");
        assert!(
            handle_text(
                r#"{"type":"TranscriptError","description":"bad audio"}"#,
                &mut last
            )
            .is_err()
        );
        assert!(handle_text(r#"{"type":"error","message":"boom"}"#, &mut last).is_err());
        assert_eq!(handle_text("not json", &mut last).unwrap(), None);
        assert_eq!(
            handle_text(r#"{"type":"SomethingElse"}"#, &mut last).unwrap(),
            None
        );
        assert_eq!(
            last, "keep",
            "non-transcript messages must not change the result"
        );
    }

    // Robust: empty data does not clobber an existing transcript.
    #[test]
    fn empty_data_does_not_clobber_robust() {
        let mut last = String::from("real text");
        handle_text(r#"{"type":"TranscriptText","data":""}"#, &mut last).unwrap();
        assert_eq!(last, "real text");
    }

    #[test]
    fn nonempty_helper_normal() {
        assert_eq!(nonempty(String::new()), None);
        assert_eq!(nonempty("x".into()), Some("x".into()));
    }
}

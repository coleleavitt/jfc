//! Anthropic WebSocket speech-to-text client (`/api/ws/speech_to_text/voice_stream`).
//!
//! Faithful port of Claude Code 2.1.177's `connectVoiceStream` (the `sg8`
//! function in `cli.deobfuscated.js`). Unlike the (non-existent) batch REST
//! endpoint, this protocol is fully specified in the CLI, so the handshake,
//! query params, control sentinels, keepalive cadence, finalize timeouts, and
//! result tags below are verbatim from the 2.1.177 build.
//!
//! ## Live streaming model (2.1.177)
//!
//! Recording starts *before* the socket is ready; the orchestrator buffers PCM
//! and, on connect, flushes it coalesced into ~32 KB frames, then streams
//! subsequent chunks live. The server returns interim transcripts that the UI
//! types in place, promoting the last interim to final on a transcript
//! endpoint or socket close.
//!
//! ## Wire protocol
//! - **Connect:** `wss://api.anthropic.com/api/ws/speech_to_text/voice_stream`
//!   with query params: `encoding=linear16 sample_rate=16000 channels=1
//!   endpointing_ms=300 utterance_end_ms=1000 language=<lang>
//!   use_conversation_engine=true [forward_interims=typed] stt_provider=deepgram-nova3`.
//! - **Headers:** `Authorization: Bearer <oauth>`, `User-Agent`, `x-app: cli`,
//!   `anthropic-client-platform: <platform>`, and `x-config-keyterms` when
//!   sanitized key-terms are supplied.
//! - **Audio:** raw little-endian 16-bit PCM, sent as **binary** WS frames.
//! - **Control (text frames):** `{"type":"KeepAlive"}` on open + every 8 s;
//!   `{"type":"CloseStream"}` to flush and end the utterance.
//! - **Results (text frames, JSON):** `TranscriptInterim` / `TranscriptText`
//!   (partial), `TranscriptEndpoint` (promote last interim → final),
//!   `TranscriptError` / `error` (failure).

use anyhow::{Context, Result};
use futures_util::{SinkExt, StreamExt};
use std::time::Duration;
use tokio::sync::{mpsc, oneshot};
use tokio::time::Instant;
use tokio_tungstenite::tungstenite::Message;
use tracing::debug;

/// `{"type":"KeepAlive"}` control sentinel (`vZ9`).
const KEEPALIVE: &str = r#"{"type":"KeepAlive"}"#;
/// `{"type":"CloseStream"}` control sentinel (`X7A`).
const CLOSE_STREAM: &str = r#"{"type":"CloseStream"}"#;
/// Periodic KeepAlive cadence (`W7A`, 8 s in 2.1.177).
const KEEPALIVE_INTERVAL: Duration = Duration::from_millis(8000);
/// `finalize()` safety timeout (`nXq.safety`).
const FINALIZE_SAFETY: Duration = Duration::from_millis(5000);
/// `finalize()` no-data timeout (`nXq.noData`).
const FINALIZE_NO_DATA: Duration = Duration::from_millis(1500);
/// Max length of the sanitized `x-config-keyterms` header (`V7A`).
const KEYTERMS_MAX_LEN: usize = 1024;
/// Coalesce buffered chunks into frames of up to this size on connect (`32e3`).
pub const COALESCE_BYTES: usize = 32_000;
/// Overall safety timeout for the batch (`transcribe_pcm`) convenience path.
const BATCH_OVERALL_TIMEOUT: Duration = Duration::from_secs(30);

/// Options for a voice-stream connection.
#[derive(Debug, Clone, Default)]
pub struct StreamOpts {
    /// BCP-47 language code (e.g. `en`).
    pub language: String,
    /// Optional project/repo-derived key terms to bias recognition. Sanitized
    /// and length-capped before being sent as `x-config-keyterms`.
    pub keyterms: Vec<String>,
    /// Request typed interim transcripts (`forward_interims=typed`). Gated by
    /// the caller (env / feature flag) — mirrors `isTypedInterimsEnabled`.
    pub forward_interims: bool,
}

/// A message emitted by the voice stream to the orchestrator (the Rust analogue
/// of the `onTranscript` / `onError` / `onReady` / `onClose` callbacks).
#[derive(Debug, Clone)]
pub enum StreamMsg {
    /// The socket is open and ready to receive audio (`onReady`).
    Ready,
    /// A transcript fragment. `is_final` distinguishes a promoted/endpointed
    /// transcript from a live interim.
    Transcript { text: String, is_final: bool },
    /// A transcription or connection error (`onError`).
    Error {
        msg: String,
        fatal: bool,
        connect_failure_code: Option<String>,
    },
    /// The socket closed (`onClose`).
    Closed,
}

/// Why a [`VoiceStream::finalize`] resolved — mirrors the resolution tags the
/// CLI passes through `v19`. The orchestrator uses [`FinalizeReason::NoDataTimeout`]
/// to decide whether to attempt the silent-drop replay.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FinalizeReason {
    /// A `TranscriptEndpoint` arrived after `CloseStream` (`post_closestream_endpoint`).
    Endpoint,
    /// No further data arrived within the no-data window (`no_data_timeout`).
    NoDataTimeout,
    /// The overall safety window elapsed (`safety_timeout`).
    SafetyTimeout,
    /// The socket was already closing/closed when finalize ran (`ws_already_closed`).
    AlreadyClosed,
    /// The socket closed while finalize was pending (`ws_close`).
    WsClose,
}

/// Commands sent to the live session task.
enum Cmd {
    /// Stream an audio chunk (dropped after `CloseStream`).
    Audio(Vec<u8>),
    /// Send `CloseStream` and resolve once the endpoint/close/timeout fires.
    Finalize(oneshot::Sender<FinalizeReason>),
    /// Close the socket immediately.
    Close,
}

#[derive(Default)]
struct BatchTranscript {
    last_final: String,
    last_interim: String,
}

impl BatchTranscript {
    fn push(&mut self, text: String, is_final: bool) {
        let text = text.trim();
        if text.is_empty() {
            return;
        }
        if is_final {
            self.last_final = text.to_owned();
            self.last_interim.clear();
        } else {
            self.last_interim = text.to_owned();
        }
    }

    fn finish(self) -> Option<String> {
        if !self.last_final.is_empty() {
            Some(self.last_final)
        } else if !self.last_interim.is_empty() {
            Some(self.last_interim)
        } else {
            None
        }
    }
}

/// Handle to a live voice-stream session. Cloning is intentionally not derived:
/// a session has a single owner that drives `send`/`finalize`/`close`.
pub struct VoiceStream {
    cmd_tx: mpsc::UnboundedSender<Cmd>,
}

impl VoiceStream {
    /// Stream a chunk of raw PCM. Dropped silently if the session has begun
    /// finalizing or the task has exited (mirrors the `send` guard).
    pub fn send(&self, pcm: &[u8]) {
        let _ = self.cmd_tx.send(Cmd::Audio(pcm.to_vec()));
    }

    /// Send `CloseStream` and await the finalize resolution (endpoint, no-data
    /// timeout, safety timeout, or socket close). The last unreported interim
    /// is promoted to a final transcript before this resolves.
    pub async fn finalize(&self) -> FinalizeReason {
        let (tx, rx) = oneshot::channel();
        if self.cmd_tx.send(Cmd::Finalize(tx)).is_err() {
            return FinalizeReason::AlreadyClosed;
        }
        rx.await.unwrap_or(FinalizeReason::AlreadyClosed)
    }

    /// Close the socket immediately without finalizing.
    pub fn close(&self) {
        let _ = self.cmd_tx.send(Cmd::Close);
    }
}

/// Build the `voice_stream` upgrade request with the 2.1.177 query params and
/// headers. Public for tests.
pub fn build_request(
    base_wss: &str,
    token: &str,
    user_agent: &str,
    platform: &str,
    opts: &StreamOpts,
) -> Result<tokio_tungstenite::tungstenite::handshake::client::Request> {
    let lang = if opts.language.is_empty() {
        "en"
    } else {
        opts.language.as_str()
    };
    // URL-encoding the language is unnecessary for BCP-47 codes, but keep the
    // param order identical to the CLI's URLSearchParams output.
    let mut url = format!(
        "{base}/api/ws/speech_to_text/voice_stream\
         ?encoding=linear16&sample_rate=16000&channels=1\
         &endpointing_ms=300&utterance_end_ms=1000&language={lang}\
         &use_conversation_engine=true",
        base = base_wss.trim_end_matches('/'),
    );
    if opts.forward_interims {
        url.push_str("&forward_interims=typed");
    }
    url.push_str("&stt_provider=deepgram-nova3");

    use tokio_tungstenite::tungstenite::client::IntoClientRequest;
    let mut req = url
        .as_str()
        .into_client_request()
        .context("invalid voice_stream URL")?;
    {
        let h = req.headers_mut();
        h.insert("Authorization", format!("Bearer {token}").parse()?);
        h.insert("User-Agent", user_agent.parse()?);
        h.insert("x-app", "cli".parse()?);
        h.insert("anthropic-client-platform", platform.parse()?);
        if let Some(keyterms) = sanitize_keyterms(&opts.keyterms) {
            h.insert("x-config-keyterms", keyterms.parse()?);
        }
    }
    Ok(req)
}

/// Connect a live voice stream. Returns the [`VoiceStream`] handle once the
/// upgrade completes; transcript/error/close events arrive on `events`.
///
/// `platform` is the `anthropic-client-platform` value (e.g. `claude_code_cli`).
pub async fn connect_voice_stream(
    base_wss: &str,
    token: &str,
    user_agent: &str,
    platform: &str,
    opts: &StreamOpts,
    events: mpsc::UnboundedSender<StreamMsg>,
) -> Result<VoiceStream> {
    let req = build_request(base_wss, token, user_agent, platform, opts)?;
    debug!(target: "jfc::voice::stt", "voice_stream: connecting (live)");

    let (ws, _resp) = tokio_tungstenite::connect_async(req)
        .await
        .context("voice_stream WebSocket connect/upgrade failed")?;

    let (cmd_tx, cmd_rx) = mpsc::unbounded_channel::<Cmd>();
    tokio::spawn(session_loop(ws, cmd_rx, events));
    Ok(VoiceStream { cmd_tx })
}

/// The session task: pumps audio out, keepalives, and parses incoming frames,
/// emitting [`StreamMsg`]s. Owns the mutable session state that lived in the
/// CLI's `sg8` closure (`v10` last interim, `v12` closing, `v22` finalizing).
async fn session_loop<S>(
    ws: tokio_tungstenite::WebSocketStream<S>,
    mut cmd_rx: mpsc::UnboundedReceiver<Cmd>,
    events: mpsc::UnboundedSender<StreamMsg>,
) where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    let (mut sink, mut stream) = ws.split();

    // open: initial KeepAlive, then signal ready.
    let _ = sink.send(Message::Text(KEEPALIVE.into())).await;
    let _ = events.send(StreamMsg::Ready);

    let mut keepalive = tokio::time::interval(KEEPALIVE_INTERVAL);
    keepalive.tick().await; // consume the immediate first tick

    let mut last_interim = String::new(); // v10
    let mut closing = false; // v12 — CloseStream sent, drop further audio
    let mut waiter: Option<oneshot::Sender<FinalizeReason>> = None; // v19
    let mut safety_at: Option<Instant> = None; // nXq.safety deadline
    let mut no_data_at: Option<Instant> = None; // nXq.noData deadline

    loop {
        // Conditional timeout futures: pending when no finalize is in flight.
        let safety = async {
            match safety_at {
                Some(d) => tokio::time::sleep_until(d).await,
                None => std::future::pending::<()>().await,
            }
        };
        let no_data = async {
            match no_data_at {
                Some(d) => tokio::time::sleep_until(d).await,
                None => std::future::pending::<()>().await,
            }
        };

        tokio::select! {
            biased;

            _ = keepalive.tick() => {
                let _ = sink.send(Message::Text(KEEPALIVE.into())).await;
            }

            cmd = cmd_rx.recv() => match cmd {
                Some(Cmd::Audio(bytes)) => {
                    if !closing {
                        let _ = sink.send(Message::Binary(bytes)).await;
                    }
                }
                Some(Cmd::Finalize(w)) => {
                    if closing || waiter.is_some() {
                        let _ = w.send(FinalizeReason::AlreadyClosed);
                    } else {
                        closing = true;
                        waiter = Some(w);
                        safety_at = Some(Instant::now() + FINALIZE_SAFETY);
                        no_data_at = Some(Instant::now() + FINALIZE_NO_DATA);
                        let _ = sink.send(Message::Text(CLOSE_STREAM.into())).await;
                    }
                }
                Some(Cmd::Close) | None => {
                    let _ = sink.send(Message::Close(None)).await;
                    break;
                }
            },

            msg = stream.next() => match msg {
                None | Some(Ok(Message::Close(_))) => break,
                Some(Err(e)) => {
                    if waiter.is_none() {
                        let _ = events.send(StreamMsg::Error {
                            msg: format!("Voice stream connection error: {e}"),
                            fatal: false,
                            connect_failure_code: None,
                        });
                    }
                    break;
                }
                Some(Ok(Message::Text(t))) => {
                    match parse_server_frame(&t) {
                        ServerFrame::Interim(data) => {
                            // Data arrived after CloseStream — cancel the no-data timer.
                            if closing {
                                no_data_at = None;
                            }
                            if !data.is_empty() {
                                last_interim = data.clone();
                                let _ = events.send(StreamMsg::Transcript { text: data, is_final: false });
                            }
                        }
                        ServerFrame::Endpoint => {
                            let promoted = std::mem::take(&mut last_interim);
                            if !promoted.is_empty() {
                                let _ = events.send(StreamMsg::Transcript { text: promoted, is_final: true });
                            }
                            if closing {
                                resolve_finalize(&mut waiter, &mut safety_at, &mut no_data_at, FinalizeReason::Endpoint);
                            }
                        }
                        ServerFrame::Error(desc) => {
                            if waiter.is_none() {
                                let _ = events.send(StreamMsg::Error {
                                    msg: desc,
                                    fatal: false,
                                    connect_failure_code: None,
                                });
                            }
                        }
                        ServerFrame::Ignore => {}
                    }
                }
                Some(Ok(_)) => {} // Ping / Pong / Binary carry no transcript text
            },

            _ = safety => {
                resolve_finalize(&mut waiter, &mut safety_at, &mut no_data_at, FinalizeReason::SafetyTimeout);
            }
            _ = no_data => {
                resolve_finalize(&mut waiter, &mut safety_at, &mut no_data_at, FinalizeReason::NoDataTimeout);
            }
        }
    }

    // on close: promote any pending interim to final, resolve a pending finalize.
    if !last_interim.is_empty() {
        let _ = events.send(StreamMsg::Transcript {
            text: std::mem::take(&mut last_interim),
            is_final: true,
        });
    }
    resolve_finalize(
        &mut waiter,
        &mut safety_at,
        &mut no_data_at,
        FinalizeReason::WsClose,
    );
    let _ = events.send(StreamMsg::Closed);
}

/// Resolve a pending finalize waiter (if any), clearing the timeout deadlines.
/// The last-interim promotion happens at the call sites that have the interim
/// in scope; this only signals the resolution.
fn resolve_finalize(
    waiter: &mut Option<oneshot::Sender<FinalizeReason>>,
    safety_at: &mut Option<Instant>,
    no_data_at: &mut Option<Instant>,
    reason: FinalizeReason,
) {
    *safety_at = None;
    *no_data_at = None;
    if let Some(w) = waiter.take() {
        let _ = w.send(reason);
    }
}

/// A parsed server frame.
#[derive(Debug, PartialEq, Eq)]
enum ServerFrame {
    /// `TranscriptInterim` / `TranscriptText` with its `data` payload.
    Interim(String),
    /// `TranscriptEndpoint`.
    Endpoint,
    /// `TranscriptError` / `error` with a description.
    Error(String),
    /// Non-JSON, control echo, or unknown frame — ignored.
    Ignore,
}

/// Parse one server text frame. Pure + unit-testable.
fn parse_server_frame(raw: &str) -> ServerFrame {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(raw) else {
        return ServerFrame::Ignore;
    };
    match v.get("type").and_then(|t| t.as_str()).unwrap_or("") {
        "TranscriptInterim" | "TranscriptText" => ServerFrame::Interim(
            v.get("data")
                .and_then(|d| d.as_str())
                .unwrap_or("")
                .to_owned(),
        ),
        "TranscriptEndpoint" => ServerFrame::Endpoint,
        "TranscriptError" => ServerFrame::Error(
            v.get("description")
                .or_else(|| v.get("error_code"))
                .and_then(|d| d.as_str())
                .unwrap_or("unknown transcription error")
                .to_owned(),
        ),
        "error" => ServerFrame::Error(
            v.get("message")
                .and_then(|d| d.as_str())
                .unwrap_or("server error")
                .to_owned(),
        ),
        _ => ServerFrame::Ignore,
    }
}

/// Sanitize key terms for the `x-config-keyterms` header (`sanitizeKeytermsForHeader`):
/// replace commas with spaces, strip non-printable-ASCII, collapse whitespace,
/// dedupe, and cap the joined length at [`KEYTERMS_MAX_LEN`]. Returns `None`
/// when nothing usable remains.
pub fn sanitize_keyterms(terms: &[String]) -> Option<String> {
    let mut seen = std::collections::HashSet::new();
    let mut out: Vec<String> = Vec::new();
    let mut total = 0usize;
    for term in terms {
        let cleaned: String = term
            .replace(',', " ")
            .chars()
            .filter(|c| (' '..='~').contains(c))
            .collect();
        let cleaned = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
        if cleaned.is_empty() || seen.contains(&cleaned) {
            continue;
        }
        let added = cleaned.len() + usize::from(!out.is_empty()); // + comma separator
        if total + added > KEYTERMS_MAX_LEN {
            break;
        }
        total += added;
        seen.insert(cleaned.clone());
        out.push(cleaned);
    }
    if out.is_empty() {
        None
    } else {
        Some(out.join(","))
    }
}

/// Transcribe a complete PCM buffer (16-bit LE, 16 kHz, mono) over the
/// voice-stream WebSocket — the batch convenience path used by the non-live
/// backends (VAD utterances, fallbacks). Streams the whole buffer coalesced
/// into ~32 KB frames, finalizes, and returns the last final transcript.
///
/// `Ok(None)` means the server heard nothing.
pub async fn transcribe_pcm(
    pcm: &[u8],
    token: &str,
    base_wss: &str,
    language: &str,
    user_agent: &str,
) -> Result<Option<String>> {
    let opts = StreamOpts {
        language: language.to_owned(),
        keyterms: Vec::new(),
        forward_interims: false,
    };
    let result = tokio::time::timeout(
        BATCH_OVERALL_TIMEOUT,
        batch_session(pcm, token, base_wss, user_agent, &opts),
    )
    .await;
    match result {
        Ok(inner) => inner,
        Err(_) => anyhow::bail!("voice_stream timed out after {BATCH_OVERALL_TIMEOUT:?}"),
    }
}

async fn batch_session(
    pcm: &[u8],
    token: &str,
    base_wss: &str,
    user_agent: &str,
    opts: &StreamOpts,
) -> Result<Option<String>> {
    let (tx, mut rx) = mpsc::unbounded_channel::<StreamMsg>();
    let stream =
        connect_voice_stream(base_wss, token, user_agent, "claude_code_cli", opts, tx).await?;

    // Wait for Ready, then flush the whole buffer coalesced to ~32 KB frames.
    // (connect_async already completed the upgrade, so Ready is imminent.)
    for frame in pcm.chunks(COALESCE_BYTES) {
        stream.send(frame);
    }

    // Finalize and collect transcripts until the stream resolves/closes.
    let finalize = stream.finalize();
    tokio::pin!(finalize);

    let mut transcript = BatchTranscript::default();
    loop {
        tokio::select! {
            _ = &mut finalize => {
                // Drain any transcript still queued before returning. VAD batch
                // sessions can resolve after receiving text but before an
                // endpoint frame promotes it to final.
                while let Ok(msg) = rx.try_recv() {
                    if let StreamMsg::Transcript { text, is_final } = msg {
                        transcript.push(text, is_final);
                    }
                }
                break;
            }
            msg = rx.recv() => match msg {
                None | Some(StreamMsg::Closed) => break,
                Some(StreamMsg::Transcript { text, is_final }) => {
                    transcript.push(text, is_final);
                }
                Some(StreamMsg::Error { msg, .. }) => {
                    stream.close();
                    return Err(anyhow::anyhow!("voice_stream error: {msg}"));
                }
                Some(_) => {}
            }
        }
    }
    stream.close();
    Ok(transcript.finish())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_server_frame_normal() {
        assert_eq!(
            parse_server_frame(r#"{"type":"TranscriptText","data":"hello world"}"#),
            ServerFrame::Interim("hello world".into())
        );
        assert_eq!(
            parse_server_frame(r#"{"type":"TranscriptInterim","data":"hi"}"#),
            ServerFrame::Interim("hi".into())
        );
        assert_eq!(
            parse_server_frame(r#"{"type":"TranscriptEndpoint"}"#),
            ServerFrame::Endpoint
        );
    }

    #[test]
    fn parse_server_frame_errors_and_ignores_robust() {
        assert_eq!(
            parse_server_frame(r#"{"type":"TranscriptError","description":"bad audio"}"#),
            ServerFrame::Error("bad audio".into())
        );
        assert_eq!(
            parse_server_frame(r#"{"type":"error","message":"boom"}"#),
            ServerFrame::Error("boom".into())
        );
        assert_eq!(parse_server_frame("not json"), ServerFrame::Ignore);
        assert_eq!(
            parse_server_frame(r#"{"type":"SomethingElse"}"#),
            ServerFrame::Ignore
        );
        // Empty interim data is preserved as an empty string (the session loop
        // declines to clobber the running interim with it).
        assert_eq!(
            parse_server_frame(r#"{"type":"TranscriptText","data":""}"#),
            ServerFrame::Interim(String::new())
        );
    }

    #[test]
    fn batch_transcript_uses_last_interim_when_endpoint_final_missing_regression() {
        let mut transcript = BatchTranscript::default();

        transcript.push("hello from vad".to_owned(), false);

        assert_eq!(transcript.finish(), Some("hello from vad".to_owned()));
    }

    #[test]
    fn build_request_has_2177_params_and_headers_normal() {
        let opts = StreamOpts {
            language: "es".into(),
            keyterms: vec!["JFC".into(), "ratatui".into()],
            forward_interims: true,
        };
        let req = build_request(
            "wss://api.anthropic.com",
            "tok123",
            "jfc-voice/0.1.0",
            "claude_code_cli",
            &opts,
        )
        .unwrap();
        let url = req.uri().to_string();
        assert!(url.contains("encoding=linear16"));
        assert!(url.contains("sample_rate=16000"));
        assert!(url.contains("endpointing_ms=300"));
        assert!(url.contains("utterance_end_ms=1000"));
        assert!(url.contains("use_conversation_engine=true"));
        assert!(url.contains("stt_provider=deepgram-nova3"));
        assert!(url.contains("language=es"));
        assert!(url.contains("forward_interims=typed"));
        let h = req.headers();
        assert_eq!(h["Authorization"], "Bearer tok123");
        assert_eq!(h["x-app"], "cli");
        assert_eq!(h["anthropic-client-platform"], "claude_code_cli");
        assert_eq!(h["x-config-keyterms"], "JFC,ratatui");
    }

    #[test]
    fn build_request_omits_forward_interims_when_disabled_normal() {
        let opts = StreamOpts {
            language: String::new(), // → default "en"
            keyterms: Vec::new(),
            forward_interims: false,
        };
        let req = build_request("wss://x", "t", "ua", "claude_code_cli", &opts).unwrap();
        let url = req.uri().to_string();
        assert!(url.contains("language=en"));
        assert!(!url.contains("forward_interims"));
        assert!(!req.headers().contains_key("x-config-keyterms"));
    }

    #[test]
    fn sanitize_keyterms_dedupes_strips_and_caps_robust() {
        // Commas → spaces, non-ASCII stripped, whitespace collapsed, deduped.
        assert_eq!(
            sanitize_keyterms(&["foo,bar".into(), "  baz  qux ".into(), "foo bar".into()]),
            Some("foo bar,baz qux".into())
        );
        // Emoji / control chars stripped entirely.
        assert_eq!(sanitize_keyterms(&["héllo✨".into()]), Some("hllo".into()));
        // Empty / whitespace-only yield None.
        assert_eq!(sanitize_keyterms(&["".into(), "   ".into()]), None);
        assert_eq!(sanitize_keyterms(&[]), None);

        // Length cap: many long unique terms stop before exceeding the cap.
        let many: Vec<String> = (0..200).map(|i| format!("term{i:030}")).collect();
        let joined = sanitize_keyterms(&many).unwrap();
        assert!(joined.len() <= KEYTERMS_MAX_LEN, "len = {}", joined.len());
    }

    #[test]
    fn finalize_resolution_clears_deadlines_normal() {
        let (tx, rx) = oneshot::channel();
        let mut waiter = Some(tx);
        let mut safety = Some(Instant::now());
        let mut no_data = Some(Instant::now());
        resolve_finalize(
            &mut waiter,
            &mut safety,
            &mut no_data,
            FinalizeReason::Endpoint,
        );
        assert!(waiter.is_none());
        assert!(safety.is_none());
        assert!(no_data.is_none());
        assert_eq!(rx.blocking_recv().ok(), Some(FinalizeReason::Endpoint));
    }
}

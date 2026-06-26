//! Live streaming record pipeline for hold/tap dictation.
//!
//! Port of Claude Code 2.1.177's recording hook (`useVoiceRecording`, the
//! `startRecordingSession` / `finishRecording` flow in `cli.deobfuscated.js`).
//! The shape mirrors the CLI:
//!
//! ```text
//! start capture (buffer while WS connects)
//!   → connect voice_stream (one early retry on failure)
//!   → on ready: flush buffered audio coalesced into ~32 KB frames
//!   → stream subsequent chunks live; type interim transcripts in place
//!   → on stop: finalize (CloseStream + endpoint/timeout), promote last interim
//!   → silent-drop replay if a connected stream returned nothing for real audio
//! ```
//!
//! When no OAuth token is available, or the configured backend isn't Anthropic,
//! the caller routes to the batch path here ([`run`] → `run_batch`) which
//! records the whole utterance and transcribes it via the backend chain
//! (OpenAI Whisper / local whisper). The live path also falls back to that
//! chain if the socket can't be established, so a captured utterance is never
//! silently lost.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use tokio::sync::{Mutex, mpsc, oneshot};
use tracing::{debug, info, warn};

use crate::anthropic_ws::{self, COALESCE_BYTES, FinalizeReason, StreamMsg, StreamOpts};
use crate::audio::{AudioCapture, CaptureBackend};
use crate::backends;
use crate::config::{SttBackendKind, VoiceConfig};
use crate::conversation_session::{self, VoiceConversationEvent};
use crate::conversation_ws::{self, ClientEvent, ClientMetrics, ServerEvent, ToolsRegisterData};
use crate::playback::PcmPlayback;
use crate::recorder::{VoiceState, VoiceTranscriptEvent, normalize_level, send_or_debug};
use crate::vad::rms_energy;

/// Capture chunk size: 100 ms at 16 kHz / 16-bit mono.
const CHUNK_BYTES: usize = 3200;
/// Delay before the single early-connect retry (`l8(250)` in the CLI).
const EARLY_RETRY_DELAY: Duration = Duration::from_millis(250);
/// Circuit-breaker window (`tp9`).
const CIRCUIT_WINDOW: Duration = Duration::from_millis(10_000);
/// Circuit-breaker threshold (`ep9`): this many early failures in the window
/// suppresses new sessions until one succeeds.
const CIRCUIT_THRESHOLD: usize = 3;
const CONVERSATION_READY_TIMEOUT: Duration = Duration::from_secs(10);
const CONVERSATION_MAX_SESSION: Duration = Duration::from_secs(300);
const CONVERSATION_MAX_INCOMING_AUDIO_BYTES: usize = 16_000 * 2 * 300;
const CONVERSATION_MAX_ASSISTANT_TEXT_CHARS: usize = 200_000;
const CONVERSATION_MAX_TRANSCRIPT_CHARS: usize = 20_000;
const RECORDING_MAX_CAPTURE_BYTES: usize = 16_000 * 2 * 300;

/// Entry point: run a single hold/tap recording session to completion. Picks
/// the live Anthropic stream when a token is present and Anthropic is the
/// effective backend, else the batch backend chain.
pub async fn run(
    backend: CaptureBackend,
    cfg: VoiceConfig,
    token: Option<String>,
    events: mpsc::UnboundedSender<VoiceTranscriptEvent>,
    state: Arc<Mutex<VoiceState>>,
    stop_rx: oneshot::Receiver<()>,
    cancel_flag: Arc<AtomicBool>,
) {
    let effective = cfg.effective_backend();
    let live = should_stream_live(token.as_deref(), effective);
    // Log the decision explicitly: non-Anthropic backends route to the batch
    // chain and therefore do not produce interim transcripts.
    info!(
        target: "jfc::voice",
        live,
        backend = ?effective,
        has_token = token.is_some(),
        "voice session backend decision"
    );

    if live {
        if circuit_tripped() {
            warn!(target: "jfc::voice", "voice circuit breaker open — suppressing session");
            send_or_debug(
                &events,
                VoiceTranscriptEvent::Error(
                    "Voice input is failing repeatedly and has been paused. Check your \
                     microphone and try again in a moment."
                        .to_owned(),
                ),
            );
        } else {
            let Some(token) = token else {
                send_or_debug(
                    &events,
                    VoiceTranscriptEvent::Error(
                        "Anthropic voice streaming selected without an OAuth token".to_owned(),
                    ),
                );
                return;
            };
            if cfg.conversation_enabled && cfg.voice_conversation_options().is_some() {
                run_conversation(
                    backend,
                    &cfg,
                    &token,
                    &events,
                    &state,
                    stop_rx,
                    &cancel_flag,
                )
                .await;
            } else {
                run_live(
                    backend,
                    &cfg,
                    &token,
                    &events,
                    &state,
                    stop_rx,
                    &cancel_flag,
                )
                .await;
            }
        }
    } else {
        run_batch(
            backend,
            &cfg,
            &events,
            &state,
            stop_rx,
            &cancel_flag,
            token.as_deref(),
        )
        .await;
    }

    set_idle(&state, &events).await;
}

async fn run_conversation(
    backend: CaptureBackend,
    cfg: &VoiceConfig,
    token: &str,
    events: &mpsc::UnboundedSender<VoiceTranscriptEvent>,
    state: &Arc<Mutex<VoiceState>>,
    stop_rx: oneshot::Receiver<()>,
    cancel_flag: &AtomicBool,
) {
    let Some(opts) = cfg.voice_conversation_options() else {
        run_live(backend, cfg, token, events, state, stop_rx, cancel_flag).await;
        return;
    };
    let base_wss = conversation_ws::resolve_base(
        cfg.conversation_base_url
            .as_deref()
            .or(cfg.anthropic_voice_url.as_deref()),
    );
    let user_agent = format!("jfc-voice/{}", env!("CARGO_PKG_VERSION"));

    let capture = match AudioCapture::start(backend).await {
        Ok(capture) => capture,
        Err(err) => {
            warn!(target: "jfc::voice", error = %err, "failed to start audio capture");
            send_or_debug(events, VoiceTranscriptEvent::Error(err.to_string()));
            return;
        }
    };
    let mut capture = Some(capture);
    info!(target: "jfc::voice", backend = %backend.label(), "voice conversation recording started");

    let mut chunk = vec![0u8; CHUNK_BYTES];
    let mut prebuffer = Vec::new();
    let mut had_audio = false;
    let mut stop_requested = false;
    tokio::pin!(stop_rx);

    let connect = conversation_session::connect(&base_wss, token, &user_agent, &opts);
    let connect_timeout = tokio::time::sleep(CONVERSATION_READY_TIMEOUT);
    tokio::pin!(connect);
    tokio::pin!(connect_timeout);
    let connected = loop {
        tokio::select! {
            result = &mut connect => break result,
            _ = &mut connect_timeout => {
                break Err(anyhow::anyhow!("voice conversation connect timed out"));
            }
            _ = &mut stop_rx, if !stop_requested => {
                stop_requested = true;
                if let Some(capture) = capture.take() {
                    let _ = append_capture_audio(&mut prebuffer, &capture.stop().await);
                }
                if cancelled(cancel_flag) {
                    return;
                }
                set_processing(state, events).await;
            }
            n = async {
                match capture.as_mut() {
                    Some(capture) => capture.read_chunk(&mut chunk).await,
                    None => Ok(0),
                }
            }, if !stop_requested => match n {
                Ok(0) | Err(_) => {}
                Ok(n) => {
                    let frame = &chunk[..n];
                    emit_level(events, frame, &mut had_audio);
                    if !append_capture_audio(&mut prebuffer, frame) {
                        stop_requested = true;
                        if let Some(capture) = capture.take() {
                            let _ = capture.stop().await;
                        }
                        set_processing(state, events).await;
                    }
                }
            },
        }
    };
    let (session, mut voice_rx) = match connected {
        Ok(session) => session,
        Err(err) => {
            warn!(target: "jfc::voice", error = %err, "voice conversation connect failed; falling back to batch STT");
            record_early_failure();
            if let Some(capture) = capture.take() {
                let _ = append_capture_audio(&mut prebuffer, &capture.stop().await);
            }
            if cancelled(cancel_flag) {
                return;
            }
            set_processing(state, events).await;
            transcribe_buffer_and_emit(prebuffer, cfg, events, Some(token)).await;
            return;
        }
    };

    if !await_conversation_ready(
        &mut voice_rx,
        capture.as_mut(),
        &mut chunk,
        &mut prebuffer,
        &mut had_audio,
        events,
        cancel_flag,
    )
    .await
    {
        session.close();
        if let Some(capture) = capture.take() {
            let _ = capture.stop().await;
        }
        return;
    }
    if !session
        .send_client_event(ClientEvent::ToolsRegister {
            data: ToolsRegisterData::default(),
        })
        .await
    {
        session.close();
        if let Some(capture) = capture.take() {
            let _ = capture.stop().await;
        }
        return;
    }
    let conversation_started = Instant::now();
    let _ = session.send_client_event(clock_sync_ping(0)).await;
    if !flush_conversation_prebuffer(&session, &prebuffer).await {
        session.close();
        if let Some(capture) = capture.take() {
            let _ = capture.stop().await;
        }
        return;
    }
    prebuffer.clear();
    if stop_requested && !session.send_client_event(ClientEvent::ManualInputEnd).await {
        session.close();
        if let Some(capture) = capture.take() {
            let _ = capture.stop().await;
        }
        return;
    }

    let mut player = match PcmPlayback::start(cfg) {
        Ok(player) => Some(player),
        Err(err) => {
            send_or_debug(
                events,
                VoiceTranscriptEvent::ReadAloudError(err.to_string()),
            );
            None
        }
    };
    let mut runtime = ConversationRuntimeState {
        local_playback_enabled: player.is_some(),
        ..Default::default()
    };
    let mut normal_complete = false;
    let mut ended_with_error = None::<String>;
    let session_deadline = tokio::time::sleep(CONVERSATION_MAX_SESSION);
    tokio::pin!(session_deadline);

    loop {
        tokio::select! {
            _ = &mut session_deadline => {
                let msg = "Voice conversation timed out".to_owned();
                send_or_debug(events, VoiceTranscriptEvent::Error(msg.clone()));
                ended_with_error = Some(msg);
                break;
            }
            _ = &mut stop_rx, if !stop_requested => {
                stop_requested = true;
                if let Some(capture) = capture.take() {
                    let _ = capture.stop().await;
                }
                if cancelled(cancel_flag) {
                    session.close();
                    break;
                }
                set_processing(state, events).await;
                if !session.send_client_event(ClientEvent::ManualInputEnd).await {
                    ended_with_error = Some("Voice conversation command channel closed".to_owned());
                    break;
                }
            }
            n = async {
                match capture.as_mut() {
                    Some(capture) => capture.read_chunk(&mut chunk).await,
                    None => Ok(0),
                }
            }, if !stop_requested => match n {
                Ok(0) | Err(_) => {
                    stop_requested = true;
                    if let Some(capture) = capture.take() {
                        let _ = capture.stop().await;
                    }
                    if cancelled(cancel_flag) {
                        session.close();
                        break;
                    }
                    set_processing(state, events).await;
                    if !session.send_client_event(ClientEvent::ManualInputEnd).await {
                        ended_with_error = Some("Voice conversation command channel closed".to_owned());
                        break;
                    }
                }
                Ok(n) => {
                    let frame = &chunk[..n];
                    emit_level(events, frame, &mut had_audio);
                    if !session.send_audio(frame).await {
                        ended_with_error = Some("Voice conversation audio channel closed".to_owned());
                        break;
                    }
                }
            },
            event = voice_rx.recv() => match event {
                None | Some(VoiceConversationEvent::Closed) => break,
                Some(VoiceConversationEvent::Error(msg)) => {
                    warn!(target: "jfc::voice", error = %msg, "voice conversation error");
                    send_or_debug(events, VoiceTranscriptEvent::Error(msg.clone()));
                    ended_with_error = Some(msg);
                    break;
                }
                Some(VoiceConversationEvent::Audio(bytes)) => {
                    if runtime.audio_bytes.saturating_add(bytes.len()) > CONVERSATION_MAX_INCOMING_AUDIO_BYTES {
                        let msg = "Voice conversation produced too much audio".to_owned();
                        send_or_debug(events, VoiceTranscriptEvent::Error(msg.clone()));
                        ended_with_error = Some(msg);
                        break;
                    }
                    runtime.audio_bytes += bytes.len();
                    runtime.audio_chunks += 1;
                    if let Some(active_player) = player.as_mut()
                        && let Err(err) = active_player.write_audio(&bytes).await
                    {
                        send_or_debug(events, VoiceTranscriptEvent::ReadAloudError(err.to_string()));
                        player = None;
                        runtime.local_playback_enabled = false;
                        runtime.read_aloud_active = false;
                    }
                }
                Some(VoiceConversationEvent::Server(server)) => {
                    handle_conversation_server_event(server, events, &mut runtime);
                    if runtime.response_complete && !runtime.playback_active {
                        normal_complete = true;
                        break;
                    }
                }
            },
        }
    }

    if !had_audio && !cancelled(cancel_flag) {
        debug!(target: "jfc::voice", "voice conversation ended with no local audio level above threshold");
    }
    if let Some(capture) = capture.take() {
        let _ = capture.stop().await;
    }
    if let Some(player) = player {
        if let Err(err) = player.finish().await {
            send_or_debug(
                events,
                VoiceTranscriptEvent::ReadAloudError(err.to_string()),
            );
        }
    }
    if normal_complete {
        let _ = session
            .send_client_event(ClientEvent::ClientMetrics {
                data: ClientMetrics {
                    client_perceived_latency_ms: Some(
                        conversation_started
                            .elapsed()
                            .as_millis()
                            .min(u128::from(u64::MAX)) as u64,
                    ),
                    buffer_underrun_count: Some(0),
                },
            })
            .await;
        let _ = session
            .send_client_event(ClientEvent::PlaybackComplete)
            .await;
    }
    complete_conversation_runtime(events, &mut runtime, ended_with_error.as_deref());
    session.close();
}

fn clock_sync_ping(seq: u64) -> ClientEvent {
    let t1 = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis().min(u128::from(u64::MAX)) as u64)
        .unwrap_or(0);
    ClientEvent::ClockSyncPing { seq, t1 }
}

#[derive(Default)]
struct ConversationRuntimeState {
    response_complete: bool,
    playback_active: bool,
    assistant_started: bool,
    read_aloud_active: bool,
    local_playback_enabled: bool,
    audio_bytes: usize,
    audio_chunks: usize,
    assistant_text_chars: usize,
}

fn handle_conversation_server_event(
    event: ServerEvent,
    events: &mpsc::UnboundedSender<VoiceTranscriptEvent>,
    runtime: &mut ConversationRuntimeState,
) {
    match event {
        ServerEvent::TranscriptInterim(value) => {
            if let Some(text) = extract_text_field(&value) {
                send_or_debug(
                    events,
                    VoiceTranscriptEvent::Interim(truncate_chars(
                        text,
                        CONVERSATION_MAX_TRANSCRIPT_CHARS,
                    )),
                );
            }
        }
        ServerEvent::UserInputEnd => {}
        ServerEvent::PlaybackStart => {
            runtime.playback_active = true;
            if runtime.local_playback_enabled {
                runtime.read_aloud_active = true;
                send_or_debug(events, VoiceTranscriptEvent::ReadAloudStarted { chars: 0 });
            }
        }
        ServerEvent::PlaybackEnd => {
            runtime.playback_active = false;
        }
        ServerEvent::MessageStart(_) => {
            runtime.assistant_started = true;
            send_or_debug(events, VoiceTranscriptEvent::AssistantMessageStarted);
        }
        ServerEvent::MessageSse(value) => {
            if let Some(error) = extract_message_sse_error(&value) {
                send_or_debug(events, VoiceTranscriptEvent::Error(error));
                return;
            }
            if let Some(text) = extract_message_sse_text_delta(&value) {
                if let Some(text) = cap_assistant_delta(text, runtime) {
                    send_or_debug(events, VoiceTranscriptEvent::AssistantTextDelta(text));
                }
            }
        }
        ServerEvent::MessageComplete(value) => {
            if matches!(message_complete_role(&value), MessageCompleteRole::User) {
                if let Some(text) = extract_text_field(&value) {
                    send_or_debug(
                        events,
                        VoiceTranscriptEvent::Interim(truncate_chars(
                            text,
                            CONVERSATION_MAX_TRANSCRIPT_CHARS,
                        )),
                    );
                }
            } else {
                runtime.response_complete = true;
                send_or_debug(events, VoiceTranscriptEvent::AssistantMessageCompleted);
            }
        }
        ServerEvent::TtsWord(timing) => {
            send_or_debug(
                events,
                VoiceTranscriptEvent::TtsWord {
                    text: timing.text,
                    pts_ms: timing.pts_ms,
                },
            );
        }
        ServerEvent::Error(value) => {
            send_or_debug(events, VoiceTranscriptEvent::Error(value.to_string()));
        }
        ServerEvent::SessionServerInitialized
        | ServerEvent::TranscriptionStart
        | ServerEvent::Other(_, _) => {}
    }
}

async fn await_conversation_ready(
    voice_rx: &mut mpsc::Receiver<VoiceConversationEvent>,
    mut capture: Option<&mut AudioCapture>,
    chunk: &mut [u8],
    prebuffer: &mut Vec<u8>,
    had_audio: &mut bool,
    events: &mpsc::UnboundedSender<VoiceTranscriptEvent>,
    cancel_flag: &AtomicBool,
) -> bool {
    let timeout = tokio::time::sleep(CONVERSATION_READY_TIMEOUT);
    tokio::pin!(timeout);
    loop {
        tokio::select! {
            _ = &mut timeout => {
                send_or_debug(events, VoiceTranscriptEvent::Error("Voice conversation did not become ready".to_owned()));
                return false;
            }
            n = async {
                match capture.as_mut() {
                    Some(capture) => capture.read_chunk(chunk).await,
                    None => Ok(0),
                }
            }, if capture.is_some() => match n {
                Ok(0) | Err(_) => {}
                Ok(n) => {
                    let frame = &chunk[..n];
                    emit_level(events, frame, had_audio);
                    if !append_capture_audio(prebuffer, frame) {
                        send_or_debug(
                            events,
                            VoiceTranscriptEvent::Error(
                                "Voice conversation recording exceeded safety cap".to_owned(),
                            ),
                        );
                        return false;
                    }
                }
            },
            event = voice_rx.recv() => match event {
                Some(VoiceConversationEvent::Server(ServerEvent::SessionServerInitialized)) => {
                    return !cancelled(cancel_flag);
                }
                Some(VoiceConversationEvent::Server(ServerEvent::Error(value))) => {
                    send_or_debug(events, VoiceTranscriptEvent::Error(value.to_string()));
                    return false;
                }
                Some(VoiceConversationEvent::Error(msg)) => {
                    send_or_debug(events, VoiceTranscriptEvent::Error(msg));
                    return false;
                }
                None | Some(VoiceConversationEvent::Closed) => return false,
                Some(VoiceConversationEvent::Audio(_))
                | Some(VoiceConversationEvent::Server(_)) => {}
            },
        }
    }
}

async fn flush_conversation_prebuffer(
    session: &conversation_session::VoiceConversationSession,
    prebuffer: &[u8],
) -> bool {
    for frame in prebuffer.chunks(COALESCE_BYTES) {
        if !session.send_audio(frame).await {
            return false;
        }
    }
    true
}

fn complete_conversation_runtime(
    events: &mpsc::UnboundedSender<VoiceTranscriptEvent>,
    runtime: &mut ConversationRuntimeState,
    error: Option<&str>,
) {
    if runtime.read_aloud_active {
        runtime.read_aloud_active = false;
        runtime.playback_active = false;
        match error {
            Some(msg) => {
                send_or_debug(events, VoiceTranscriptEvent::ReadAloudError(msg.to_owned()))
            }
            None => send_or_debug(
                events,
                VoiceTranscriptEvent::ReadAloudCompleted {
                    audio_bytes: runtime.audio_bytes,
                    chunks_sent: runtime.audio_chunks,
                },
            ),
        }
    }
    if runtime.assistant_started && !runtime.response_complete {
        runtime.response_complete = true;
        send_or_debug(events, VoiceTranscriptEvent::AssistantMessageCompleted);
    }
}

fn extract_message_sse_text_delta(value: &serde_json::Value) -> Option<String> {
    for event in message_sse_payloads(value) {
        if let Some(text) = extract_message_sse_text_delta_from_payload(event) {
            return Some(text);
        }
    }
    None
}

fn extract_message_sse_text_delta_from_payload(event: &serde_json::Value) -> Option<String> {
    if let Some(raw) = event.as_str()
        && let Ok(parsed) = serde_json::from_str::<serde_json::Value>(raw)
    {
        return extract_message_sse_text_delta(&parsed);
    }
    let delta = event.get("delta")?;
    let kind = delta.get("type").and_then(|kind| kind.as_str());
    if kind != Some("text_delta") {
        return None;
    }
    delta
        .get("text")
        .and_then(|text| text.as_str())
        .filter(|text| !text.is_empty())
        .map(str::to_owned)
}

fn extract_message_sse_error(value: &serde_json::Value) -> Option<String> {
    for event in message_sse_payloads(value) {
        if let Some(raw) = event.as_str()
            && let Ok(parsed) = serde_json::from_str::<serde_json::Value>(raw)
            && let Some(error) = extract_message_sse_error(&parsed)
        {
            return Some(error);
        }
        let kind = event
            .get("type")
            .or_else(|| event.get("event"))
            .and_then(|kind| kind.as_str());
        if matches!(
            kind,
            Some("error" | "message_limit" | "conversation_limit" | "rate_limit")
        ) {
            return extract_text_field(event)
                .or_else(|| {
                    event
                        .get("message")
                        .and_then(|message| message.as_str())
                        .map(str::to_owned)
                })
                .or_else(|| event.get("error").map(serde_json::Value::to_string));
        }
    }
    None
}

fn message_sse_payloads(value: &serde_json::Value) -> Vec<&serde_json::Value> {
    let mut payloads = Vec::with_capacity(4);
    if let Some(data) = value.get("data") {
        payloads.push(data);
    }
    if let Some(event) = value.get("event")
        && !event
            .as_str()
            .is_some_and(|event| !event.trim_start().starts_with('{'))
    {
        payloads.push(event);
    }
    if let Some(message) = value.get("message") {
        payloads.push(message);
    }
    payloads.push(value);
    payloads
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MessageCompleteRole {
    Assistant,
    User,
    Unknown,
}

fn message_complete_role(value: &serde_json::Value) -> MessageCompleteRole {
    match find_role_field(value).as_deref() {
        Some("human" | "user") => MessageCompleteRole::User,
        Some("assistant" | "claude") => MessageCompleteRole::Assistant,
        _ => MessageCompleteRole::Unknown,
    }
}

fn find_role_field(value: &serde_json::Value) -> Option<String> {
    for key in ["role", "sender"] {
        if let Some(role) = value
            .get(key)
            .and_then(|role| role.as_str())
            .filter(|role| !role.trim().is_empty())
        {
            return Some(role.trim().to_ascii_lowercase());
        }
    }
    for key in ["message", "data", "payload"] {
        if let Some(value) = value.get(key)
            && let Some(role) = find_role_field(value)
        {
            return Some(role);
        }
    }
    None
}

fn cap_assistant_delta(text: String, runtime: &mut ConversationRuntimeState) -> Option<String> {
    let remaining =
        CONVERSATION_MAX_ASSISTANT_TEXT_CHARS.saturating_sub(runtime.assistant_text_chars);
    if remaining == 0 {
        return None;
    }
    let text = truncate_chars(text, remaining);
    runtime.assistant_text_chars += text.chars().count();
    Some(text)
}

fn truncate_chars(mut text: String, cap: usize) -> String {
    if text.chars().count() <= cap {
        return text;
    }
    let end = text
        .char_indices()
        .nth(cap)
        .map(|(idx, _)| idx)
        .unwrap_or(text.len());
    text.truncate(end);
    text
}

fn extract_text_field(value: &serde_json::Value) -> Option<String> {
    for key in ["text", "transcript", "partial", "input", "content"] {
        if let Some(text) = value
            .get(key)
            .and_then(|field| field.as_str())
            .filter(|text| !text.trim().is_empty())
        {
            return Some(text.to_owned());
        }
        if let Some(field) = value.get(key) {
            match field {
                serde_json::Value::Object(object) => {
                    if let Some(text) =
                        extract_text_field(&serde_json::Value::Object(object.clone()))
                    {
                        return Some(text);
                    }
                }
                serde_json::Value::Array(items) => {
                    for item in items {
                        if let Some(text) = extract_text_field(item) {
                            return Some(text);
                        }
                    }
                }
                serde_json::Value::Null
                | serde_json::Value::Bool(_)
                | serde_json::Value::Number(_)
                | serde_json::Value::String(_) => {}
            }
        }
    }
    for key in ["message", "data", "payload"] {
        if let Some(field) = value.get(key)
            && let Some(text) = extract_text_field(field)
        {
            return Some(text);
        }
    }
    None
}

/// Whether the active recording was cancelled (discard, no `Final`).
fn cancelled(flag: &AtomicBool) -> bool {
    flag.load(Ordering::SeqCst)
}

/// Decide whether to use the live Anthropic streaming path (interim transcripts
/// that type into the box live) vs. the batch backend chain (record → upload →
/// one final, no interims). `backend` is the *effective* backend
/// ([`VoiceConfig::effective_backend`], which maps `Auto` → `Anthropic`).
///
/// Live requires both an OAuth token and Anthropic as the effective backend.
/// This is the exact switch that determines whether you get live typing.
fn should_stream_live(token: Option<&str>, backend: SttBackendKind) -> bool {
    token.is_some() && matches!(backend, SttBackendKind::Anthropic)
}

/// The live streaming path.
async fn run_live(
    backend: CaptureBackend,
    cfg: &VoiceConfig,
    token: &str,
    events: &mpsc::UnboundedSender<VoiceTranscriptEvent>,
    state: &Arc<Mutex<VoiceState>>,
    stop_rx: oneshot::Receiver<()>,
    cancel_flag: &AtomicBool,
) {
    let base_wss = resolve_ws_base(cfg);
    let user_agent = format!("jfc-voice/{}", env!("CARGO_PKG_VERSION"));
    let language = cfg.language.clone();
    let opts = StreamOpts {
        language: language.clone(),
        keyterms: Vec::new(),
        forward_interims: cfg.forward_interims,
        allow_custom_auth_endpoint: cfg.allow_custom_auth_endpoint,
        allow_insecure_auth_endpoint: cfg.allow_insecure_auth_endpoint,
    };

    let mut capture = match AudioCapture::start(backend).await {
        Ok(c) => c,
        Err(err) => {
            warn!(target: "jfc::voice", error = %err, "failed to start audio capture");
            send_or_debug(events, VoiceTranscriptEvent::Error(err.to_string()));
            return;
        }
    };
    info!(target: "jfc::voice", backend = %backend.label(), "live recording started");

    let mut all_audio: Vec<u8> = Vec::new(); // full utterance, for replay/fallback
    let mut prebuffer: Vec<u8> = Vec::new(); // audio captured before WS ready
    let mut had_audio = false;
    let mut chunk = vec![0u8; CHUNK_BYTES];
    tokio::pin!(stop_rx);
    let mut stop_requested = false;
    let mut capture_capped = false;

    // ── Phase A: connect while buffering capture (one early retry) ──────────
    let mut attempt = 0u32;
    let connected = loop {
        let (tx, rx) = mpsc::unbounded_channel::<StreamMsg>();
        let connect = anthropic_ws::connect_voice_stream(
            &base_wss,
            token,
            &user_agent,
            "claude_code_cli",
            &opts,
            tx,
        );
        tokio::pin!(connect);
        let result = loop {
            tokio::select! {
                r = &mut connect => break r,
                _ = &mut stop_rx, if !stop_requested => {
                    debug!(target: "jfc::voice", "stop requested while connecting");
                    stop_requested = true;
                }
                n = capture.read_chunk(&mut chunk) => match n {
                    Ok(0) | Err(_) => {}
                    Ok(n) => {
                        let frame = &chunk[..n];
                        emit_level(events, frame, &mut had_audio);
                        if !append_capture_audio_pair(&mut prebuffer, &mut all_audio, frame) {
                            capture_capped = true;
                            stop_requested = true;
                        }
                    }
                },
            }
        };
        match result {
            Ok(stream) => break Some((stream, rx)),
            Err(err) if attempt == 0 && !stop_requested => {
                attempt += 1;
                warn!(target: "jfc::voice", error = %err, "early voice_stream connect failed, retrying once");
                tokio::time::sleep(EARLY_RETRY_DELAY).await;
                continue;
            }
            Err(err) => {
                warn!(target: "jfc::voice", error = %err, "voice_stream connect failed; falling back to batch");
                record_early_failure();
                break None;
            }
        }
    };

    let Some((stream, mut ev_rx)) = connected else {
        // Connect failed after the retry — don't lose the utterance: drain and
        // transcribe what we captured via the batch chain (OpenAI / local),
        // unless the session was cancelled (discard, no Final).
        if !capture_capped {
            append_capture_audio(&mut all_audio, &capture.stop().await);
        } else {
            let _ = capture.stop().await;
        }
        if cancelled(cancel_flag) {
            return;
        }
        set_processing(state, events).await;
        transcribe_buffer_and_emit(all_audio, cfg, events, Some(token)).await;
        return;
    };

    // ── Phase B: flush buffered audio, then stream live ─────────────────────
    if !flush_coalesced(&stream, &prebuffer).await {
        stream.close();
        if !capture_capped {
            append_capture_audio(&mut all_audio, &capture.stop().await);
        } else {
            let _ = capture.stop().await;
        }
        set_processing(state, events).await;
        transcribe_buffer_and_emit(all_audio, cfg, events, Some(token)).await;
        return;
    }
    prebuffer.clear();

    let mut final_text = String::new();
    let mut interim = String::new();
    let mut got_transcript = false;
    let mut live_error = false;

    if !stop_requested {
        loop {
            tokio::select! {
                _ = &mut stop_rx => break,
                n = capture.read_chunk(&mut chunk) => match n {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        let frame = &chunk[..n];
                        emit_level(events, frame, &mut had_audio);
                        if !append_capture_audio(&mut all_audio, frame) {
                            capture_capped = true;
                            break;
                        }
                        if !stream.send(frame).await {
                            live_error = true;
                            break;
                        }
                    }
                },
                msg = ev_rx.recv() => match msg {
                    None | Some(StreamMsg::Closed) => break,
                    Some(StreamMsg::Ready) => {}
                    Some(StreamMsg::Transcript { text, is_final }) => {
                        apply_transcript(events, &mut final_text, &mut interim, &mut got_transcript, text, is_final);
                    }
                    Some(StreamMsg::Error { msg, .. }) => {
                        warn!(target: "jfc::voice", error = %msg, "live voice_stream error");
                        if !got_transcript { live_error = true; }
                        break;
                    }
                },
            }
        }
    }

    // Stop the mic before finalizing — no more audio is sent after CloseStream.
    if !capture_capped {
        append_capture_audio(&mut all_audio, &capture.stop().await);
    } else {
        let _ = capture.stop().await;
    }

    // Cancelled (discard): drop the utterance with no finalize / no Final, so a
    // manual Enter-submit doesn't get a duplicate auto-submit from voice.
    if cancelled(cancel_flag) {
        stream.close();
        return;
    }

    set_processing(state, events).await;

    // A pre-transcript error means the live session is unusable; salvage the
    // captured audio through the batch chain rather than emitting nothing.
    if live_error && !got_transcript {
        stream.close();
        transcribe_buffer_and_emit(all_audio, cfg, events, Some(token)).await;
        return;
    }

    // ── Phase C: finalize (CloseStream + endpoint/close/timeout) ────────────
    let reason = finalize_collecting(
        &stream,
        &mut ev_rx,
        events,
        &mut final_text,
        &mut interim,
        &mut got_transcript,
    )
    .await;
    stream.close();

    // ── Silent-drop replay ──────────────────────────────────────────────────
    // A connected stream that returned nothing for real audio (no_data_timeout)
    // is the silent-drop failure mode: replay the buffered audio once on a fresh
    // connection. Mirrors `tengu_voice_silent_drop_replay`.
    if reason == FinalizeReason::NoDataTimeout
        && had_audio
        && !got_transcript
        && final_text.trim().is_empty()
        && !all_audio.is_empty()
    {
        info!(
            target: "jfc::voice",
            bytes = all_audio.len(),
            "silent-drop detected (no_data_timeout); replaying on fresh connection"
        );
        tokio::time::sleep(EARLY_RETRY_DELAY).await;
        match anthropic_ws::transcribe_pcm_with_opts(
            &all_audio,
            token,
            &base_wss,
            &user_agent,
            &opts,
        )
        .await
        {
            Ok(Some(text)) if !text.trim().is_empty() => {
                append_final(&mut final_text, &text);
            }
            Ok(_) => {}
            Err(err) => warn!(target: "jfc::voice", error = %err, "silent-drop replay failed"),
        }
    }

    emit_final(events, final_text.trim(), had_audio);
}

/// The batch path: record the whole utterance, then transcribe via the chain.
async fn run_batch(
    backend: CaptureBackend,
    cfg: &VoiceConfig,
    events: &mpsc::UnboundedSender<VoiceTranscriptEvent>,
    state: &Arc<Mutex<VoiceState>>,
    stop_rx: oneshot::Receiver<()>,
    cancel_flag: &AtomicBool,
    oauth_token: Option<&str>,
) {
    let mut capture = match AudioCapture::start(backend).await {
        Ok(c) => c,
        Err(err) => {
            warn!(target: "jfc::voice", error = %err, "failed to start audio capture");
            send_or_debug(events, VoiceTranscriptEvent::Error(err.to_string()));
            return;
        }
    };
    debug!(target: "jfc::voice", backend = %backend.label(), "batch recording started");

    let mut all_audio: Vec<u8> = Vec::new();
    let mut had_audio = false;
    let mut chunk = vec![0u8; CHUNK_BYTES];
    tokio::pin!(stop_rx);
    let mut capture_capped = false;
    loop {
        tokio::select! {
            _ = &mut stop_rx => break,
            n = capture.read_chunk(&mut chunk) => match n {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    let frame = &chunk[..n];
                    emit_level(events, frame, &mut had_audio);
                    if !append_capture_audio(&mut all_audio, frame) {
                        capture_capped = true;
                        break;
                    }
                }
            },
        }
    }
    if !capture_capped {
        append_capture_audio(&mut all_audio, &capture.stop().await);
    } else {
        let _ = capture.stop().await;
    }
    if cancelled(cancel_flag) {
        return; // discard — no transcription, no Final
    }
    set_processing(state, events).await;
    transcribe_buffer_and_emit(all_audio, cfg, events, oauth_token).await;
}

/// Run the batch backend chain over a buffer and emit Final/Error.
async fn transcribe_buffer_and_emit(
    pcm: Vec<u8>,
    cfg: &VoiceConfig,
    events: &mpsc::UnboundedSender<VoiceTranscriptEvent>,
    oauth_token: Option<&str>,
) {
    match backends::transcribe_with_token(&pcm, cfg, oauth_token).await {
        Ok(Some(text)) => {
            info!(target: "jfc::voice", chars = text.len(), "STT transcript received (batch)");
            send_or_debug(events, VoiceTranscriptEvent::Final(text));
        }
        Ok(None) => debug!(target: "jfc::voice", "batch STT returned empty (silence)"),
        Err(err) => {
            warn!(target: "jfc::voice", error = %err, "batch STT failed");
            send_or_debug(events, VoiceTranscriptEvent::Error(err.to_string()));
        }
    }
}

/// Send `finalize()` and keep draining the event channel until it resolves,
/// accumulating any promoted final transcript that arrives meanwhile.
pub(crate) async fn finalize_collecting(
    stream: &anthropic_ws::VoiceStream,
    ev_rx: &mut mpsc::UnboundedReceiver<StreamMsg>,
    events: &mpsc::UnboundedSender<VoiceTranscriptEvent>,
    final_text: &mut String,
    interim: &mut String,
    got_transcript: &mut bool,
) -> FinalizeReason {
    let finalize = stream.finalize();
    tokio::pin!(finalize);
    loop {
        tokio::select! {
            r = &mut finalize => break r,
            msg = ev_rx.recv() => match msg {
                None => {} // channel drained; keep waiting for finalize to resolve
                Some(StreamMsg::Transcript { text, is_final }) => {
                    apply_transcript(events, final_text, interim, got_transcript, text, is_final);
                }
                Some(_) => {}
            }
        }
    }
}

/// Apply one transcript fragment: interims type live, finals accumulate.
pub(crate) fn apply_transcript(
    events: &mpsc::UnboundedSender<VoiceTranscriptEvent>,
    final_text: &mut String,
    interim: &mut String,
    got_transcript: &mut bool,
    text: String,
    is_final: bool,
) {
    if is_final {
        if !text.trim().is_empty() {
            append_final(final_text, &text);
            *got_transcript = true;
        }
        interim.clear();
    } else {
        *interim = text;
    }
    // The TUI types this whole string in place (interim preview), replacing the
    // previous interim — so it sees accumulated finals plus the live partial.
    send_or_debug(
        events,
        VoiceTranscriptEvent::Interim(join_display(final_text, interim)),
    );
}

/// Append a final fragment to the accumulated transcript with a separating space.
fn append_final(final_text: &mut String, fragment: &str) {
    let fragment = fragment.trim();
    if fragment.is_empty() {
        return;
    }
    if !final_text.is_empty() {
        final_text.push(' ');
    }
    final_text.push_str(fragment);
}

/// Combine accumulated finals + current interim for the in-place preview.
fn join_display(final_text: &str, interim: &str) -> String {
    match (final_text.trim().is_empty(), interim.trim().is_empty()) {
        (true, _) => interim.trim().to_owned(),
        (false, true) => final_text.trim().to_owned(),
        (false, false) => format!("{} {}", final_text.trim(), interim.trim()),
    }
}

/// Flush a buffer to the stream as frames of up to [`COALESCE_BYTES`] each.
async fn flush_coalesced(stream: &anthropic_ws::VoiceStream, buf: &[u8]) -> bool {
    if buf.is_empty() {
        return true;
    }
    debug!(target: "jfc::voice", bytes = buf.len(), "flushing buffered audio (coalesced)");
    for frame in buf.chunks(COALESCE_BYTES) {
        if !stream.send(frame).await {
            return false;
        }
    }
    true
}

fn append_capture_audio(buf: &mut Vec<u8>, frame: &[u8]) -> bool {
    let remaining = RECORDING_MAX_CAPTURE_BYTES.saturating_sub(buf.len());
    let accepted = remaining.min(frame.len());
    buf.extend_from_slice(&frame[..accepted]);
    accepted == frame.len()
}

fn append_capture_audio_pair(
    prebuffer: &mut Vec<u8>,
    all_audio: &mut Vec<u8>,
    frame: &[u8],
) -> bool {
    let remaining = RECORDING_MAX_CAPTURE_BYTES
        .saturating_sub(prebuffer.len())
        .min(RECORDING_MAX_CAPTURE_BYTES.saturating_sub(all_audio.len()));
    let accepted = remaining.min(frame.len());
    prebuffer.extend_from_slice(&frame[..accepted]);
    all_audio.extend_from_slice(&frame[..accepted]);
    accepted == frame.len()
}

/// Compute and emit the normalized RMS level for a chunk; mark `had_audio` once
/// a meaningfully non-silent level is seen (CLI threshold `> 0.01`).
fn emit_level(
    events: &mpsc::UnboundedSender<VoiceTranscriptEvent>,
    frame: &[u8],
    had_audio: &mut bool,
) {
    let level = normalize_level(rms_energy(frame));
    if level > 0.01 {
        *had_audio = true;
    }
    send_or_debug(events, VoiceTranscriptEvent::Level(level));
}

/// Emit the final transcript, or an explanatory error when nothing was heard.
fn emit_final(events: &mpsc::UnboundedSender<VoiceTranscriptEvent>, result: &str, had_audio: bool) {
    if !result.is_empty() {
        info!(target: "jfc::voice", chars = result.len(), "final transcript assembled");
        send_or_debug(events, VoiceTranscriptEvent::Final(result.to_owned()));
    } else if !had_audio {
        send_or_debug(
            events,
            VoiceTranscriptEvent::Error(
                "No audio detected from microphone. Check the input device and mic access."
                    .to_owned(),
            ),
        );
    } else {
        debug!(target: "jfc::voice", "no speech detected in utterance");
    }
}

async fn set_processing(
    state: &Arc<Mutex<VoiceState>>,
    events: &mpsc::UnboundedSender<VoiceTranscriptEvent>,
) {
    *state.lock().await = VoiceState::Processing;
    send_or_debug(
        events,
        VoiceTranscriptEvent::StateChanged(VoiceState::Processing),
    );
}

async fn set_idle(
    state: &Arc<Mutex<VoiceState>>,
    events: &mpsc::UnboundedSender<VoiceTranscriptEvent>,
) {
    *state.lock().await = VoiceState::Idle;
    send_or_debug(events, VoiceTranscriptEvent::StateChanged(VoiceState::Idle));
}

/// Resolve the WS origin: explicit override → wss form, else the default host.
/// Mirrors `try_anthropic_ws`'s base resolution.
pub(crate) fn resolve_ws_base(cfg: &VoiceConfig) -> String {
    let http = cfg
        .anthropic_voice_url
        .as_deref()
        .filter(|u| !u.is_empty())
        .unwrap_or("https://api.anthropic.com");
    http.replacen("https://", "wss://", 1)
        .replacen("http://", "ws://", 1)
}

// ── Circuit breaker (process-global) ─────────────────────────────────────────

fn circuit_state() -> &'static std::sync::Mutex<Vec<Instant>> {
    static CB: std::sync::OnceLock<std::sync::Mutex<Vec<Instant>>> = std::sync::OnceLock::new();
    CB.get_or_init(|| std::sync::Mutex::new(Vec::new()))
}

/// Record an early connect/stream failure for the circuit breaker.
fn record_early_failure() {
    let now = Instant::now();
    if let Ok(mut v) = circuit_state().lock() {
        v.retain(|t| now.duration_since(*t) <= CIRCUIT_WINDOW);
        v.push(now);
    }
}

/// True when [`CIRCUIT_THRESHOLD`] early failures occurred within the window.
fn circuit_tripped() -> bool {
    let now = Instant::now();
    if let Ok(mut v) = circuit_state().lock() {
        v.retain(|t| now.duration_since(*t) <= CIRCUIT_WINDOW);
        v.len() >= CIRCUIT_THRESHOLD
    } else {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn join_display_combines_finals_and_interim_normal() {
        assert_eq!(join_display("", ""), "");
        assert_eq!(join_display("", "hello"), "hello");
        assert_eq!(join_display("hello", ""), "hello");
        assert_eq!(join_display("hello", "world"), "hello world");
        assert_eq!(join_display("  hi ", " there "), "hi there");
    }

    #[test]
    fn append_final_spaces_and_skips_empty_normal() {
        let mut s = String::new();
        append_final(&mut s, "  ");
        assert_eq!(s, "");
        append_final(&mut s, "one");
        append_final(&mut s, " two ");
        assert_eq!(s, "one two");
    }

    #[test]
    fn forward_interims_comes_from_voice_config_normal() {
        let cfg = VoiceConfig::from_settings(Some(&serde_json::json!({
            "forwardInterims": false
        })));

        assert!(!cfg.forward_interims);
    }

    #[test]
    fn apply_transcript_interim_then_final_normal() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let mut final_text = String::new();
        let mut interim = String::new();
        let mut got = false;
        apply_transcript(
            &tx,
            &mut final_text,
            &mut interim,
            &mut got,
            "hel".into(),
            false,
        );
        assert_eq!(interim, "hel");
        assert!(!got);
        apply_transcript(
            &tx,
            &mut final_text,
            &mut interim,
            &mut got,
            "hello there".into(),
            true,
        );
        assert_eq!(final_text, "hello there");
        assert!(interim.is_empty());
        assert!(got);
        // Two interim previews were emitted.
        let mut previews = Vec::new();
        while let Ok(VoiceTranscriptEvent::Interim(t)) = rx.try_recv() {
            previews.push(t);
        }
        assert_eq!(previews, vec!["hel".to_owned(), "hello there".to_owned()]);
    }

    #[test]
    fn extract_message_sse_text_delta_normal() {
        let value = serde_json::json!({
            "type": "message_sse",
            "event": {
                "type": "content_block_delta",
                "delta": { "type": "text_delta", "text": "hello" }
            }
        });

        assert_eq!(
            extract_message_sse_text_delta(&value).as_deref(),
            Some("hello")
        );
    }

    #[test]
    fn extract_message_sse_text_delta_accepts_string_data_robust() {
        let value = serde_json::json!({
            "type": "message_sse",
            "data": "{\"type\":\"content_block_delta\",\"delta\":{\"type\":\"text_delta\",\"text\":\"hello\"}}"
        });

        assert_eq!(
            extract_message_sse_text_delta(&value).as_deref(),
            Some("hello")
        );
    }

    #[test]
    fn extract_message_sse_text_delta_prefers_data_when_event_is_name_regression() {
        let value = serde_json::json!({
            "type": "message_sse",
            "event": "content_block_delta",
            "data": "{\"type\":\"content_block_delta\",\"delta\":{\"type\":\"text_delta\",\"text\":\"from data\"}}"
        });

        assert_eq!(
            extract_message_sse_text_delta(&value).as_deref(),
            Some("from data")
        );
    }

    #[test]
    fn message_complete_user_role_keeps_assistant_open_regression() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let mut runtime = ConversationRuntimeState::default();
        let value = serde_json::json!({
            "type": "message_complete",
            "message": {
                "role": "user",
                "content": [{ "text": "hello claude" }]
            }
        });

        handle_conversation_server_event(ServerEvent::MessageComplete(value), &tx, &mut runtime);

        assert!(!runtime.response_complete);
        assert!(matches!(
            rx.try_recv().unwrap(),
            VoiceTranscriptEvent::Interim(text) if text == "hello claude"
        ));
    }

    #[test]
    fn message_complete_assistant_role_finishes_stream_normal() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let mut runtime = ConversationRuntimeState::default();
        let value = serde_json::json!({
            "type": "message_complete",
            "message": { "role": "assistant" }
        });

        handle_conversation_server_event(ServerEvent::MessageComplete(value), &tx, &mut runtime);

        assert!(runtime.response_complete);
        assert!(matches!(
            rx.try_recv().unwrap(),
            VoiceTranscriptEvent::AssistantMessageCompleted
        ));
    }

    #[test]
    fn playback_end_waits_for_local_player_finish_regression() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let mut runtime = ConversationRuntimeState {
            read_aloud_active: true,
            local_playback_enabled: true,
            audio_bytes: 3200,
            audio_chunks: 1,
            ..Default::default()
        };

        handle_conversation_server_event(ServerEvent::PlaybackEnd, &tx, &mut runtime);

        assert!(!runtime.playback_active);
        assert!(runtime.read_aloud_active);
        assert!(matches!(
            rx.try_recv(),
            Err(mpsc::error::TryRecvError::Empty)
        ));
        complete_conversation_runtime(&tx, &mut runtime, None);
        assert!(matches!(
            rx.try_recv().unwrap(),
            VoiceTranscriptEvent::ReadAloudCompleted {
                audio_bytes: 3200,
                chunks_sent: 1
            }
        ));
    }

    #[test]
    fn extract_transcript_text_accepts_nested_payload_robust() {
        let value = serde_json::json!({
            "type": "transcript_interim",
            "transcript": { "text": "what is this" }
        });

        assert_eq!(extract_text_field(&value).as_deref(), Some("what is this"));
    }

    #[test]
    fn circuit_breaker_trips_after_threshold_robust() {
        // Drain any residual state from other tests in the same process.
        if let Ok(mut v) = circuit_state().lock() {
            v.clear();
        }
        assert!(!circuit_tripped());
        for _ in 0..CIRCUIT_THRESHOLD {
            record_early_failure();
        }
        assert!(circuit_tripped());
        if let Ok(mut v) = circuit_state().lock() {
            v.clear();
        }
    }

    // REGRESSION (no live typing): the live Anthropic path — the only one that
    // streams interim transcripts into the box — must be chosen iff a token is
    // present AND the effective backend is Anthropic.
    #[test]
    fn should_stream_live_decision_normal() {
        use crate::config::SttBackendKind::*;
        assert!(should_stream_live(Some("tok"), Anthropic));
        assert!(!should_stream_live(None, Anthropic)); // no token → batch
        assert!(!should_stream_live(Some("tok"), OpenAiWhisper)); // forced openai → batch
        assert!(!should_stream_live(Some("tok"), LocalWhisper));
    }

    // The default `Auto` backend must resolve to Anthropic so a signed-in user
    // gets live typing out of the box; an explicit `openai` backend must NOT.
    // Batch-only backends produce no interim transcript.
    #[test]
    fn auto_backend_gives_live_openai_gives_batch_normal() {
        use crate::config::SttBackendKind;
        let auto = VoiceConfig::default(); // backend = Auto
        assert_eq!(auto.effective_backend(), SttBackendKind::Anthropic);
        assert!(should_stream_live(Some("tok"), auto.effective_backend()));

        let openai = VoiceConfig {
            backend: SttBackendKind::OpenAiWhisper,
            ..Default::default()
        };
        assert_eq!(openai.effective_backend(), SttBackendKind::OpenAiWhisper);
        assert!(!should_stream_live(Some("tok"), openai.effective_backend()));
    }
}

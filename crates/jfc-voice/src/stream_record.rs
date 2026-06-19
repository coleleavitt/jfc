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
    // Log the decision explicitly — the #1 "no live typing" cause is the
    // effective backend not being Anthropic (e.g. JFC_VOICE_BACKEND=openai),
    // which silently routes to the batch chain with no interim transcripts.
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
            run_live(
                backend,
                &cfg,
                &token.unwrap(),
                &events,
                &state,
                stop_rx,
                &cancel_flag,
            )
            .await;
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

/// Whether the active recording was cancelled (discard, no `Final`).
fn cancelled(flag: &AtomicBool) -> bool {
    flag.load(Ordering::SeqCst)
}

/// Decide whether to use the live Anthropic streaming path (interim transcripts
/// that type into the box live) vs. the batch backend chain (record → upload →
/// one final, no interims). `backend` is the *effective* backend
/// ([`VoiceConfig::effective_backend`], which maps `Auto` → `Anthropic`).
///
/// Live requires both an OAuth token and Anthropic as the effective backend, so
/// an explicit `JFC_VOICE_BACKEND=openai`/`local` — or no token — routes to
/// batch. This is the exact switch that determines whether you get live typing.
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
        forward_interims: forward_interims_enabled(),
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
                        prebuffer.extend_from_slice(frame);
                        all_audio.extend_from_slice(frame);
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
        all_audio.extend_from_slice(&capture.stop().await);
        if cancelled(cancel_flag) {
            return;
        }
        set_processing(state, events).await;
        transcribe_buffer_and_emit(all_audio, cfg, events, Some(token)).await;
        return;
    };

    // ── Phase B: flush buffered audio, then stream live ─────────────────────
    flush_coalesced(&stream, &prebuffer);
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
                        all_audio.extend_from_slice(frame);
                        stream.send(frame);
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
    all_audio.extend_from_slice(&capture.stop().await);

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
        match anthropic_ws::transcribe_pcm(&all_audio, token, &base_wss, &language, &user_agent)
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
    loop {
        tokio::select! {
            _ = &mut stop_rx => break,
            n = capture.read_chunk(&mut chunk) => match n {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    let frame = &chunk[..n];
                    emit_level(events, frame, &mut had_audio);
                    all_audio.extend_from_slice(frame);
                }
            },
        }
    }
    all_audio.extend_from_slice(&capture.stop().await);
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
async fn finalize_collecting(
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
fn apply_transcript(
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
fn flush_coalesced(stream: &anthropic_ws::VoiceStream, buf: &[u8]) {
    if buf.is_empty() {
        return;
    }
    debug!(target: "jfc::voice", bytes = buf.len(), "flushing buffered audio (coalesced)");
    for frame in buf.chunks(COALESCE_BYTES) {
        stream.send(frame);
    }
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
fn resolve_ws_base(cfg: &VoiceConfig) -> String {
    std::env::var("VOICE_STREAM_BASE_URL")
        .ok()
        .unwrap_or_else(|| {
            let http = cfg
                .anthropic_voice_url
                .as_deref()
                .filter(|u| !u.is_empty())
                .unwrap_or("https://api.anthropic.com");
            http.replacen("https://", "wss://", 1)
                .replacen("http://", "ws://", 1)
        })
}

/// Whether to request typed interims (`forward_interims=typed`). Default on for
/// live dictation so the interim text types in place as you speak; opt out via
/// `JFC_VOICE_FORWARD_INTERIMS=0` (or `CLAUDE_CODE_VOICE_FORWARD_INTERIMS_TYPED`).
fn forward_interims_enabled() -> bool {
    let raw = std::env::var("JFC_VOICE_FORWARD_INTERIMS")
        .or_else(|_| std::env::var("CLAUDE_CODE_VOICE_FORWARD_INTERIMS_TYPED"))
        .unwrap_or_default()
        .to_lowercase();
    !matches!(raw.as_str(), "0" | "false" | "off" | "no")
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
    fn forward_interims_default_on_opt_out_robust() {
        // Save/restore env so this test is hermetic.
        const A: &str = "JFC_VOICE_FORWARD_INTERIMS";
        let prev = std::env::var(A).ok();
        unsafe { std::env::set_var(A, "0") };
        assert!(!forward_interims_enabled());
        unsafe { std::env::set_var(A, "1") };
        assert!(forward_interims_enabled());
        unsafe {
            match prev {
                Some(v) => std::env::set_var(A, v),
                None => std::env::remove_var(A),
            }
        }
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
    // (This is what `JFC_VOICE_BACKEND=openai` was silently doing — forcing the
    // batch path with no interims.)
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

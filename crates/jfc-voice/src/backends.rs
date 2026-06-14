//! STT backend implementations.
//!
//! All backends accept raw 16-bit PCM at 16 kHz mono and return the
//! transcript as a `String`. The caller wraps it in WAV when needed
//! via [`crate::audio::wrap_wav`].

use anyhow::Context;
use tracing::{debug, warn};

use crate::config::{SttBackendKind, VoiceConfig};

/// Transcribe a buffer of raw PCM audio using the configured backend chain.
///
/// Priority:
/// 1. Anthropic WebSocket (if OAuth token available)
/// 2. OpenAI Whisper API (if OPENAI_API_KEY set)
/// 3. Local whisper binary (if binary found on PATH)
///
/// Returns `None` if the audio is silent or empty.
pub async fn transcribe(pcm: &[u8], cfg: &VoiceConfig) -> anyhow::Result<Option<String>> {
    if pcm.is_empty() {
        debug!(target: "jfc::voice::stt", "transcribe called with empty PCM");
        return Ok(None);
    }

    // Silence/energy gate: if the buffer's overall RMS is near-silent, don't
    // send it to Whisper at all. Whisper hallucinates caption boilerplate on
    // silence, so the cheapest, most reliable fix is to never transcribe a
    // near-empty buffer. ~120 RMS is comfortably below speech (~1000+) but
    // above a quiet room's noise floor.
    let overall_rms = crate::vad::rms_energy(pcm);
    const MIN_TRANSCRIBE_RMS: u32 = 120;
    if overall_rms < MIN_TRANSCRIBE_RMS {
        debug!(
            target: "jfc::voice::stt",
            overall_rms,
            "skipping transcription — buffer is near-silent (avoids Whisper hallucination)"
        );
        return Ok(None);
    }
    // Also require a minimum duration: a <300ms buffer can't be a real
    // utterance and is a prime hallucination source. 16kHz * 2 bytes * 0.3s.
    const MIN_TRANSCRIBE_BYTES: usize = 16_000 * 2 * 3 / 10;
    if pcm.len() < MIN_TRANSCRIBE_BYTES {
        debug!(
            target: "jfc::voice::stt",
            bytes = pcm.len(),
            "skipping transcription — buffer too short to be speech"
        );
        return Ok(None);
    }

    let wav = crate::audio::wrap_wav(pcm);
    debug!(
        target: "jfc::voice::stt",
        pcm_bytes = pcm.len(),
        wav_bytes = wav.len(),
        backend = ?cfg.effective_backend(),
        "transcribe: starting backend chain"
    );

    match cfg.effective_backend() {
        SttBackendKind::Anthropic | SttBackendKind::Auto => {
            // Anthropic STT, in order of fidelity to Claude Code:
            //   1. The real WebSocket voice_stream protocol (needs raw PCM).
            //   2. An explicitly-configured Whisper-compatible gateway (WAV).
            //   3. Fall through to OpenAI Whisper, then local.
            // Any error (e.g. no OAuth token) falls through; a successful-but-
            // empty result (Ok(None)) means the provider heard silence.
            match try_anthropic_ws(pcm, cfg).await {
                Ok(Some(text)) => {
                    debug!(target: "jfc::voice::stt", "Anthropic WS STT succeeded");
                    return Ok(Some(text));
                }
                Ok(None) => {
                    debug!(target: "jfc::voice::stt", "Anthropic WS STT returned empty");
                    return Ok(None);
                }
                Err(err) => {
                    warn!(
                        target: "jfc::voice::stt",
                        error = %err,
                        "Anthropic WS STT unavailable, trying configured gateway"
                    );
                }
            }
            match try_anthropic_batch(&wav, cfg).await {
                Ok(Some(text)) => {
                    debug!(target: "jfc::voice::stt", "Anthropic gateway STT succeeded");
                    return Ok(Some(text));
                }
                Ok(None) => {
                    debug!(target: "jfc::voice::stt", "Anthropic gateway STT returned empty");
                    return Ok(None);
                }
                Err(err) => {
                    warn!(
                        target: "jfc::voice::stt",
                        error = %err,
                        "Anthropic gateway STT unavailable, trying OpenAI Whisper"
                    );
                }
            }
            match try_openai_whisper(&wav, cfg).await {
                Ok(Some(text)) => {
                    debug!(target: "jfc::voice::stt", "OpenAI Whisper succeeded");
                    return Ok(Some(text));
                }
                Ok(None) => {
                    debug!(target: "jfc::voice::stt", "OpenAI Whisper returned empty");
                    return Ok(None);
                }
                Err(err) => {
                    warn!(
                        target: "jfc::voice::stt",
                        error = %err,
                        "OpenAI Whisper unavailable, trying local whisper"
                    );
                }
            }
            try_local_whisper(&wav, cfg).await
        }
        SttBackendKind::OpenAiWhisper => try_openai_whisper(&wav, cfg).await,
        SttBackendKind::LocalWhisper => try_local_whisper(&wav, cfg).await,
    }
}

// ── Anthropic batch transcription (opt-in gateway only) ──────────────────────
//
// Claude Code transcribes over an undocumented WebSocket protocol
// (`/api/ws/speech_to_text/voice_stream`) backed by a native module — not a
// batch REST endpoint. jfc deliberately does NOT reverse-engineer that unstable
// protocol. This function therefore only runs when the user explicitly
// configures `voice.anthropic_voice_url` to a Whisper-compatible gateway that
// implements `/v1/audio/transcriptions`; otherwise it fails fast (no doomed
// upload to api.anthropic.com) and the chain falls through to OpenAI Whisper.

/// Anthropic STT over the real WebSocket `voice_stream` protocol (the path
/// Claude Code actually uses). Faithfully ported from the CLI — see
/// [`crate::anthropic_ws`]. Needs an OAuth access token; errors (no token,
/// connect failure) fall through to the gateway/OpenAI paths.
async fn try_anthropic_ws(pcm: &[u8], cfg: &VoiceConfig) -> anyhow::Result<Option<String>> {
    let token = std::env::var("CLAUDE_ACCESS_TOKEN")
        .or_else(|_| std::env::var("ANTHROPIC_ACCESS_TOKEN"))
        .or_else(|_| std::env::var("JFC_ANTHROPIC_ACCESS_TOKEN"))
        .context("no Anthropic OAuth token available for voice STT")?;

    // Base WS origin: explicit override (VOICE_STREAM_BASE_URL or the configured
    // voice URL) → wss form; else the default api host. Mirrors the CLI, which
    // converts the REST base to wss://.
    let base_wss = std::env::var("VOICE_STREAM_BASE_URL")
        .ok()
        .unwrap_or_else(|| {
            let http = cfg
                .anthropic_voice_url
                .as_deref()
                .filter(|u| !u.is_empty())
                .unwrap_or("https://api.anthropic.com");
            http.replacen("https://", "wss://", 1)
                .replacen("http://", "ws://", 1)
        });

    let user_agent = format!("jfc-voice/{}", env!("CARGO_PKG_VERSION"));
    crate::anthropic_ws::transcribe_pcm(pcm, &token, &base_wss, &cfg.language, &user_agent).await
}

async fn try_anthropic_batch(wav: &[u8], cfg: &VoiceConfig) -> anyhow::Result<Option<String>> {
    // IMPORTANT — Anthropic has no public batch REST transcription endpoint.
    //
    // Claude Code does speech-to-text over a *WebSocket* stream
    // (`/api/ws/speech_to_text/voice_stream`), pushing PCM frames and receiving
    // partial/final transcripts. The wire protocol (frame framing, message
    // tags) is not publicly documented and is delivered through a compiled
    // native `audio-capture.node` module in the CC binary — it can't be
    // reliably reproduced here without reverse-engineering an undocumented,
    // unstable protocol. So jfc does NOT speculatively implement it.
    //
    // Consequently, the default `api.anthropic.com` has no compatible batch
    // endpoint: a POST there 404s after uploading the whole WAV (wasted
    // bandwidth + latency on every utterance) and then falls through to OpenAI.
    // To avoid that doomed round-trip we only attempt an Anthropic-compatible
    // batch transcription when the user has explicitly pointed
    // `anthropic_voice_url` at an OpenAI-/Whisper-compatible gateway that they
    // know implements `/v1/audio/transcriptions`. Otherwise we fail fast and
    // let the chain fall through to OpenAI Whisper (the working path).
    let Some(base_url) = cfg.anthropic_voice_url.as_deref().filter(|u| !u.is_empty()) else {
        return Err(anyhow::anyhow!(
            "Anthropic batch STT is not available (CC uses an undocumented WebSocket \
             protocol jfc does not implement). Set voice.anthropic_voice_url to a \
             Whisper-compatible /v1/audio/transcriptions gateway to use it, or rely on \
             the OpenAI Whisper fallback."
        ));
    };

    // Read OAuth token from environment (set by the auth subsystem).
    let token = std::env::var("CLAUDE_ACCESS_TOKEN")
        .or_else(|_| std::env::var("ANTHROPIC_ACCESS_TOKEN"))
        .or_else(|_| std::env::var("JFC_ANTHROPIC_ACCESS_TOKEN"))
        .context("no Anthropic OAuth token available for voice STT")?;

    // The configured gateway must expose an OpenAI-compatible transcription
    // route. (We do not append to api.anthropic.com — that endpoint 404s.)
    let url = format!("{base_url}/v1/audio/transcriptions");

    let client = reqwest::Client::new();
    let part = reqwest::multipart::Part::bytes(wav.to_vec())
        .file_name("audio.wav")
        .mime_str("audio/wav")?;
    let form = reqwest::multipart::Form::new()
        .part("file", part)
        .text("language", cfg.language.clone());

    debug!(
        target: "jfc::voice::stt",
        url = %url,
        wav_bytes = wav.len(),
        "Anthropic STT request"
    );

    let resp = client
        .post(&url)
        .bearer_auth(&token)
        .header("x-app", "cli")
        .multipart(form)
        .timeout(std::time::Duration::from_secs(30))
        .send()
        .await
        .context("Anthropic STT HTTP request failed")?;

    if resp.status() == 404 {
        return Err(anyhow::anyhow!(
            "Anthropic REST transcription endpoint not found"
        ));
    }

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(anyhow::anyhow!("Anthropic STT {status}: {body}"));
    }

    let json: serde_json::Value = resp.json().await?;
    let text = json
        .get("text")
        .and_then(|t| t.as_str())
        .map(str::to_owned)
        .unwrap_or_default();
    Ok(nonempty(text))
}

// ── OpenAI Whisper API ────────────────────────────────────────────────────────

async fn try_openai_whisper(wav: &[u8], cfg: &VoiceConfig) -> anyhow::Result<Option<String>> {
    let api_key = cfg
        .openai_api_key
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("no OPENAI_API_KEY for Whisper API"))?;

    let client = reqwest::Client::new();
    let part = reqwest::multipart::Part::bytes(wav.to_vec())
        .file_name("audio.wav")
        .mime_str("audio/wav")?;
    let form = reqwest::multipart::Form::new()
        .part("file", part)
        .text("model", "whisper-1")
        .text("language", cfg.language.clone())
        // temperature=0 makes decoding deterministic and greatly reduces the
        // hallucinated-caption problem ("Thank you for watching", "Please
        // subscribe", "Go to <site>.com") Whisper emits on near-silent audio.
        .text("temperature", "0")
        .text("response_format", "json");

    debug!(
        target: "jfc::voice::stt",
        wav_bytes = wav.len(),
        "OpenAI Whisper API request"
    );

    let resp = client
        .post("https://api.openai.com/v1/audio/transcriptions")
        .bearer_auth(api_key)
        .multipart(form)
        .timeout(std::time::Duration::from_secs(30))
        .send()
        .await
        .context("OpenAI Whisper HTTP request failed")?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(anyhow::anyhow!("OpenAI Whisper {status}: {body}"));
    }

    let json: serde_json::Value = resp.json().await?;
    let text = json
        .get("text")
        .and_then(|t| t.as_str())
        .map(str::to_owned)
        .unwrap_or_default();
    Ok(nonempty(text))
}

// ── Local whisper.cpp binary ──────────────────────────────────────────────────

async fn try_local_whisper(wav: &[u8], cfg: &VoiceConfig) -> anyhow::Result<Option<String>> {
    // Find the binary
    let bin = cfg
        .local_whisper_bin
        .as_deref()
        .and_then(|b| (!b.is_empty()).then_some(b))
        .or_else(|| {
            // Auto-detect common names
            for name in &["whisper-cpp", "whisper", "main"] {
                if which(name) {
                    return Some(*name);
                }
            }
            None
        })
        .ok_or_else(|| anyhow::anyhow!("no local whisper binary found (install whisper.cpp)"))?
        .to_owned();

    // Write WAV to a temp file (whisper.cpp reads from a file)
    let tmp = write_temp_wav(wav).await?;
    let tmp_path = tmp.to_string_lossy().to_string();

    let mut args = vec![
        "--language".to_owned(),
        cfg.language.clone(),
        "--output-txt".to_owned(),
        "--no-prints".to_owned(),
    ];
    if let Some(ref model) = cfg.local_whisper_model {
        args.extend_from_slice(&["--model".to_owned(), model.clone()]);
    }
    args.push(tmp_path.clone());

    debug!(
        target: "jfc::voice::stt",
        bin = %bin,
        wav_bytes = wav.len(),
        "local whisper request"
    );

    let out = tokio::process::Command::new(&bin)
        .args(&args)
        .output()
        .await
        .with_context(|| format!("failed to run {bin}"))?;

    // Remove temp file (best effort)
    // Best-effort cleanup — not a problem if the file is already gone.
    if let Err(err) = tokio::fs::remove_file(&tmp_path).await {
        debug!(target: "jfc::voice::stt", error = %err, "temp WAV cleanup failed");
    }

    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr).to_string();
        return Err(anyhow::anyhow!("{bin} exited non-zero: {stderr}"));
    }

    // whisper.cpp with --output-txt writes <file>.txt; also check stdout
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    if !stdout.trim().is_empty() {
        return Ok(nonempty(stdout.trim().to_owned()));
    }

    // Try reading the .txt sidecar
    let txt_path = format!("{tmp_path}.txt");
    if let Ok(content) = tokio::fs::read_to_string(&txt_path).await {
        if let Err(err) = tokio::fs::remove_file(&txt_path).await {
            debug!(target: "jfc::voice::stt", error = %err, "temp txt cleanup failed");
        }
        return Ok(nonempty(content.trim().to_owned()));
    }

    Ok(None)
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn nonempty(s: String) -> Option<String> {
    let trimmed = collapse_repeats(s.trim());
    if trimmed.is_empty() {
        return None;
    }
    // Drop known Whisper hallucinations on near-silent / non-speech audio.
    // Whisper is trained on YouTube captions, so when fed silence or noise it
    // emits caption boilerplate ("Thank you for watching", "Subscribe", ad
    // reads like "Go to Beadaholique.com"). These are never real user speech.
    if is_whisper_hallucination(&trimmed) {
        tracing::debug!(
            target: "jfc::voice::stt",
            text = %trimmed,
            "dropped Whisper hallucination (non-speech boilerplate)"
        );
        return None;
    }
    Some(trimmed)
}

/// Whether a transcript is a known Whisper hallucination — caption/ad
/// boilerplate Whisper emits on silence or noise rather than real speech.
///
/// Matches the *whole* transcript (case-insensitive, punctuation-stripped)
/// against known phrases, so genuine speech that merely contains one of these
/// words isn't dropped — only a transcript that is *entirely* boilerplate.
fn is_whisper_hallucination(text: &str) -> bool {
    let normalized: String = text
        .to_lowercase()
        .chars()
        .filter(|c| c.is_alphanumeric() || c.is_whitespace())
        .collect();
    let normalized = normalized.split_whitespace().collect::<Vec<_>>().join(" ");
    if normalized.is_empty() {
        return true;
    }

    // Exact full-transcript matches (the most common silence hallucinations).
    const EXACT: &[&str] = &[
        "thank you",
        "thank you for watching",
        "thanks for watching",
        "thank you very much",
        "thank you so much",
        "thank you for watching this video",
        "please subscribe",
        "please subscribe to my channel",
        "subscribe to my channel",
        "dont forget to subscribe",
        "like and subscribe",
        "see you next time",
        "see you in the next video",
        "bye",
        "bye bye",
        "you",
        "the end",
        "music",
        "applause",
        "silence",
        "i dont know",
        "okay",
        "ok",
    ];
    if EXACT.contains(&normalized.as_str()) {
        return true;
    }

    // Substring markers for ad-read / caption hallucinations that vary in
    // wording but always contain a tell-tale fragment.
    const CONTAINS: &[&str] = &[
        "beadaholique",
        "for all of your beading",
        "subtitles by",
        "amara.org",
        "transcription by",
        "captions by",
        "subscribe to",
        "for watching this video",
        "thanks for watching the video",
    ];
    CONTAINS.iter().any(|m| normalized.contains(m))
}

/// Collapse Whisper's repetition hallucination.
///
/// On noisy or near-silent audio, Whisper often loops the same phrase
/// ("Hello? Hello? Hello? …" or "How are you doing today? ×5"). This
/// detects an immediately-repeated phrase and keeps a single copy.
///
/// Strategy: split into sentence-ish chunks on `.?!`, then drop a chunk if
/// it equals the previous kept chunk (case-insensitive, trimmed).
fn collapse_repeats(text: &str) -> String {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return String::new();
    }

    // Split keeping the terminator, so "Hi? Hi?" → ["Hi?", " Hi?"].
    let mut segments: Vec<&str> = Vec::new();
    let mut start = 0;
    let bytes = trimmed.as_bytes();
    for (i, &b) in bytes.iter().enumerate() {
        if b == b'.' || b == b'?' || b == b'!' {
            segments.push(trimmed[start..=i].trim());
            start = i + 1;
        }
    }
    if start < trimmed.len() {
        segments.push(trimmed[start..].trim());
    }
    segments.retain(|s| !s.is_empty());
    if segments.len() < 2 {
        return trimmed.to_owned();
    }

    // Keep a segment only if it differs from the last kept one.
    let mut kept: Vec<&str> = Vec::new();
    for seg in segments {
        let dup = kept
            .last()
            .map(|prev: &&str| prev.eq_ignore_ascii_case(seg))
            .unwrap_or(false);
        if !dup {
            kept.push(seg);
        }
    }

    // If EVERY segment was the same phrase repeated, we now have exactly one.
    kept.join(" ")
}

use crate::platform::which;

async fn write_temp_wav(wav: &[u8]) -> anyhow::Result<std::path::PathBuf> {
    use std::time::{SystemTime, UNIX_EPOCH};
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let path = std::env::temp_dir().join(format!("jfc-voice-{ts}.wav"));
    tokio::fs::write(&path, wav)
        .await
        .context("failed to write temp WAV")?;
    Ok(path)
}

// ── Anthropic real-time WebSocket STT ─────────────────────────────────────────
//
// This provides the full CC-compatible streaming STT experience:
// audio chunks are sent as they're recorded, and interim transcripts
// arrive in real time for display in the TUI.

pub mod anthropic_streaming {
    use anyhow::Context;
    use futures_util::{SinkExt, StreamExt};
    use tokio_tungstenite::{connect_async_tls_with_config, tungstenite::Message};
    use tracing::{debug, info, warn};

    use crate::config::VoiceConfig;

    /// Events from the streaming STT session.
    #[derive(Debug, Clone)]
    pub enum StreamEvent {
        /// Interim partial transcript (update the display but don't submit).
        Interim(String),
        /// Final transcript for the utterance.
        Final(String),
        /// Session ended (cleanly or with error).
        Closed,
        /// An error from the server.
        Error(String),
    }

    /// Open a real-time STT WebSocket session to Anthropic.
    ///
    /// Returns a sender for audio chunks and a receiver for transcript events.
    /// Audio should be sent as raw PCM (16-bit LE, 16 kHz, mono) chunks.
    /// Drop the sender when recording is done to finalize the WebSocket session.
    pub async fn connect(
        cfg: &VoiceConfig,
        oauth_token: &str,
    ) -> anyhow::Result<(
        tokio::sync::mpsc::Sender<Vec<u8>>,
        tokio::sync::mpsc::Receiver<StreamEvent>,
    )> {
        let url = build_ws_url(cfg);
        debug!(target: "jfc::voice::ws", url = %url, "connecting to Anthropic STT WebSocket");

        let ws_stream = open_ws(&url, oauth_token).await?;
        let (audio_tx, event_rx) = spawn_ws_tasks(ws_stream);

        info!(target: "jfc::voice::ws", "Anthropic STT WebSocket connected");
        Ok((audio_tx, event_rx))
    }

    fn build_ws_url(cfg: &VoiceConfig) -> String {
        let base = cfg
            .anthropic_voice_url
            .as_deref()
            .unwrap_or("https://api.anthropic.com");
        let ws_base = base
            .replace("https://", "wss://")
            .replace("http://", "ws://");
        let params = format!(
            "encoding=linear16&sample_rate=16000&channels=1\
             &endpointing_ms=300&utterance_end_ms=1000\
             &language={lang}&use_conversation_engine=true\
             &forward_interims=typed&stt_provider=deepgram-nova3",
            lang = cfg.language,
        );
        format!("{ws_base}/api/ws/speech_to_text/voice_stream?{params}")
    }

    async fn open_ws(
        url: &str,
        oauth_token: &str,
    ) -> anyhow::Result<
        tokio_tungstenite::WebSocketStream<
            tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
        >,
    > {
        let mut req =
            tokio_tungstenite::tungstenite::client::IntoClientRequest::into_client_request(url)
                .context("invalid WebSocket URL")?;
        req.headers_mut().insert(
            "Authorization",
            format!("Bearer {oauth_token}").parse().unwrap(),
        );
        req.headers_mut().insert("x-app", "cli".parse().unwrap());
        let (ws, _) = connect_async_tls_with_config(req, None, false, None)
            .await
            .context("WebSocket connection failed")?;
        Ok(ws)
    }

    fn spawn_ws_tasks(
        ws: tokio_tungstenite::WebSocketStream<
            tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
        >,
    ) -> (
        tokio::sync::mpsc::Sender<Vec<u8>>,
        tokio::sync::mpsc::Receiver<StreamEvent>,
    ) {
        let (mut write, mut read) = ws.split();
        let (audio_tx, mut audio_rx) = tokio::sync::mpsc::channel::<Vec<u8>>(64);
        let (event_tx, event_rx) = tokio::sync::mpsc::channel::<StreamEvent>(64);

        // Forward audio chunks → WebSocket, then send CloseStream
        let ev_tx_clone = event_tx.clone();
        tokio::spawn(async move {
            while let Some(chunk) = audio_rx.recv().await {
                if write.send(Message::Binary(chunk)).await.is_err() {
                    break;
                }
            }
            let close = serde_json::json!({"type": "CloseStream"});
            if let Err(err) = write.send(Message::Text(close.to_string())).await {
                debug!(target: "jfc::voice::ws", error = %err, "failed to send CloseStream");
            }
            if let Err(err) = write.close().await {
                debug!(target: "jfc::voice::ws", error = %err, "WebSocket close failed");
            }
            drop(ev_tx_clone);
        });

        // Read transcript events ← WebSocket
        tokio::spawn(async move {
            run_reader_loop(&mut read, event_tx).await;
        });

        (audio_tx, event_rx)
    }

    async fn run_reader_loop<S>(
        read: &mut futures_util::stream::SplitStream<tokio_tungstenite::WebSocketStream<S>>,
        event_tx: tokio::sync::mpsc::Sender<StreamEvent>,
    ) where
        S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
    {
        while let Some(msg) = read.next().await {
            match msg {
                Ok(Message::Text(text)) => {
                    let ev = parse_stt_event(&text);
                    let closed = matches!(ev, StreamEvent::Closed);
                    if event_tx.send(ev).await.is_err() {
                        break;
                    }
                    if closed {
                        break;
                    }
                }
                Ok(Message::Close(_)) => {
                    if event_tx.send(StreamEvent::Closed).await.is_err() {
                        debug!(target: "jfc::voice::ws", "event receiver dropped on Close");
                    }
                    break;
                }
                Err(err) => {
                    warn!(target: "jfc::voice::ws", error = %err, "WebSocket read error");
                    if event_tx
                        .send(StreamEvent::Error(err.to_string()))
                        .await
                        .is_err()
                    {
                        debug!(target: "jfc::voice::ws", "event receiver dropped on error");
                    }
                    break;
                }
                Ok(other) => {
                    debug!(
                        target: "jfc::voice::ws",
                        msg_type = ?std::mem::discriminant(&other),
                        "ignoring non-text WS message"
                    );
                }
            }
        }
    }

    fn parse_stt_event(text: &str) -> StreamEvent {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(text) else {
            return StreamEvent::Error(format!("unparseable: {text}"));
        };
        let type_ = v.get("type").and_then(|t| t.as_str()).unwrap_or("");
        match type_ {
            "TranscriptEndpoint" | "transcript_final" => {
                let t = v
                    .get("transcript")
                    .or_else(|| v.get("text"))
                    .and_then(|t| t.as_str())
                    .unwrap_or("")
                    .to_owned();
                StreamEvent::Final(t)
            }
            "TranscriptInterim" | "transcript_interim" => {
                let t = v
                    .get("transcript")
                    .or_else(|| v.get("text"))
                    .and_then(|t| t.as_str())
                    .unwrap_or("")
                    .to_owned();
                StreamEvent::Interim(t)
            }
            "TranscriptError" => {
                let msg = v
                    .get("message")
                    .and_then(|m| m.as_str())
                    .unwrap_or("unknown error")
                    .to_owned();
                StreamEvent::Error(msg)
            }
            "StreamClose" | "stream_close" => StreamEvent::Closed,
            _ => {
                debug!(target: "jfc::voice::ws", type_ = %type_, "unknown STT event");
                StreamEvent::Closed // treat unknown terminal events as close
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::collapse_repeats;

    #[test]
    fn collapse_repeated_hello_normal() {
        let input = "Hello? Hello? Hello? Hello? Hello?";
        assert_eq!(collapse_repeats(input), "Hello?");
    }

    #[test]
    fn collapse_repeated_sentence_normal() {
        let input = "How are you doing today? How are you doing today? How are you doing today?";
        assert_eq!(collapse_repeats(input), "How are you doing today?");
    }

    #[test]
    fn keeps_distinct_sentences_normal() {
        let input = "How are you? I am fine. How are you?";
        // Only collapses *adjacent* duplicates, so the second "How are you?"
        // (after "I am fine.") is kept.
        assert_eq!(
            collapse_repeats(input),
            "How are you? I am fine. How are you?"
        );
    }

    #[test]
    fn collapse_adjacent_only_robust() {
        let input = "Yes. Yes. No. No. Yes.";
        assert_eq!(collapse_repeats(input), "Yes. No. Yes.");
    }

    #[test]
    fn single_sentence_unchanged_robust() {
        assert_eq!(collapse_repeats("Just one sentence."), "Just one sentence.");
    }

    #[test]
    fn empty_returns_empty_robust() {
        assert_eq!(collapse_repeats(""), "");
        assert_eq!(collapse_repeats("   "), "");
    }

    #[test]
    fn case_insensitive_dedup_robust() {
        assert_eq!(collapse_repeats("Hello? hello? HELLO?"), "Hello?");
    }
}

#[cfg(test)]
mod anthropic_backend_tests {
    use super::*;
    use crate::config::VoiceConfig;

    // Anthropic batch STT must fail fast (not upload + 404) when no compatible
    // gateway is configured, so the chain falls through to OpenAI Whisper. CC's
    // real STT is an undocumented WebSocket protocol jfc does not implement.
    #[tokio::test]
    async fn anthropic_batch_fails_fast_without_gateway_robust() {
        let cfg = VoiceConfig {
            anthropic_voice_url: None,
            ..Default::default()
        };
        let wav = vec![0u8; 64];
        let err = try_anthropic_batch(&wav, &cfg)
            .await
            .expect_err("must error without a configured gateway");
        let msg = err.to_string();
        assert!(
            msg.contains("not available") && msg.contains("WebSocket"),
            "error should explain the WS-only reality, got: {msg}"
        );
    }

    // An empty gateway URL is treated the same as None (no doomed upload).
    #[tokio::test]
    async fn anthropic_batch_empty_url_is_unavailable_robust() {
        let cfg = VoiceConfig {
            anthropic_voice_url: Some(String::new()),
            ..Default::default()
        };
        let err = try_anthropic_batch(&[0u8; 64], &cfg).await.unwrap_err();
        assert!(err.to_string().contains("not available"));
    }
}

#[cfg(test)]
mod hallucination_tests {
    use super::{collapse_repeats, is_whisper_hallucination};

    #[test]
    fn detects_thank_you_for_watching_normal() {
        assert!(is_whisper_hallucination("Thank you for watching!"));
        assert!(is_whisper_hallucination("Thanks for watching."));
        assert!(is_whisper_hallucination("THANK YOU FOR WATCHING"));
    }

    #[test]
    fn detects_subscribe_boilerplate_normal() {
        assert!(is_whisper_hallucination("Please subscribe to my channel."));
        assert!(is_whisper_hallucination("Like and subscribe!"));
        assert!(is_whisper_hallucination("Don't forget to subscribe."));
    }

    #[test]
    fn detects_ad_read_hallucinations_normal() {
        assert!(is_whisper_hallucination(
            "Go to Beadaholique.com for all of your beading supply needs!"
        ));
        assert!(is_whisper_hallucination(
            "Subtitles by the Amara.org community"
        ));
    }

    #[test]
    fn keeps_real_speech_robust() {
        // Real speech that merely contains a flagged word is NOT dropped.
        assert!(!is_whisper_hallucination(
            "Can you subscribe me to the newsletter in the code?"
        ));
        assert!(!is_whisper_hallucination(
            "Thank you for the help, now let's fix the parser bug"
        ));
        assert!(!is_whisper_hallucination("How are you doing today?"));
        assert!(!is_whisper_hallucination(
            "Add a function that reads the config file"
        ));
    }

    #[test]
    fn bare_thank_you_dropped_robust() {
        // A bare "Thank you." with nothing else is the classic silence output.
        assert!(is_whisper_hallucination("Thank you."));
        assert!(is_whisper_hallucination("You"));
    }

    #[test]
    fn collapse_still_works_normal() {
        assert_eq!(collapse_repeats("Hello? Hello? Hello?"), "Hello?");
    }
}

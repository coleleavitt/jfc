//! Incremental streaming text-to-speech.
//!
//! Unlike [`crate::playback::speak_anthropic_tts`] (which synthesizes a whole
//! finished reply in one shot), this keeps a single `text_to_speech/text_stream`
//! WebSocket and a persistent [`PcmPlayback`] open for a turn, and lets the
//! caller `feed` text incrementally — sentence by sentence as the model
//! generates — while audio frames stream back and play in real time. The result
//! is speech that starts after roughly the first sentence instead of after the
//! whole reply, i.e. near-simultaneous with generation.
//!
//! ```text
//! connect() ── opens WS + PcmPlayback, spawns the session loop
//!   feed("First sentence.")   → text_chunk → server → binary PCM → player
//!   feed("Second sentence.")  → (plays while the first is still going)
//!   finish()                  → close_stream → drain playback → stats
//!   cancel()                  → barge-in: kill playback + close WS now
//! ```

use anyhow::{Context, Result};
use futures_util::{SinkExt, StreamExt};
use std::sync::atomic::{AtomicI64, Ordering};
use std::time::Duration;
use tokio::sync::{mpsc, oneshot};
use tokio_tungstenite::tungstenite::Message;

use crate::config::VoiceConfig;
use crate::playback::PcmPlayback;
use crate::tts::{self, ServerFrame, TtsOptions};

const KEEPALIVE_INTERVAL: Duration = Duration::from_millis(4000);
const COMMAND_BUFFER: usize = 128;

/// Number of streaming-TTS sessions currently holding an open player. Read by
/// the VAD loop ([`tts_playback_active`]) to gate the mic during read-aloud so
/// the assistant's own voice can't trigger an utterance (half-duplex echo
/// guard — there is no acoustic echo cancellation).
static PLAYBACK_SESSIONS: AtomicI64 = AtomicI64::new(0);

/// True while at least one streaming-TTS session is open and playing.
pub fn tts_playback_active() -> bool {
    PLAYBACK_SESSIONS.load(Ordering::Acquire) > 0
}

/// Audio actually streamed and played during a turn.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TtsPlaybackStats {
    pub audio_bytes: usize,
    pub chunks: usize,
}

enum Cmd {
    Text(String),
    Finish(oneshot::Sender<TtsPlaybackStats>),
    Cancel,
}

/// Opportunistic self-voice enrollment: tees a bounded amount of the played TTS
/// PCM during a read-aloud turn to build a reject-profile for the assistant's
/// own voice (we control the TTS audio → a clean reference). One-shot — only
/// armed when the speaker gate is on, the output is raw PCM, and no reject
/// profile exists yet for this voice. Saved on normal completion.
struct SelfEnroll {
    cfg: VoiceConfig,
    key: String,
    buf: Vec<u8>,
}

impl SelfEnroll {
    /// ~4 s of 16 kHz mono S16LE — plenty of voiced audio to voiceprint.
    const BUDGET_BYTES: usize = 4 * 16_000 * 2;

    /// Decide whether to arm self-enrollment for this turn.
    fn arm(cfg: &VoiceConfig) -> Option<Self> {
        if !cfg.speaker_gate
            || !cfg.tts_output_format.starts_with("pcm")
            || crate::recorder::reject_profile_path(cfg, &cfg.tts_voice).exists()
        {
            return None;
        }
        Some(Self {
            cfg: cfg.clone(),
            key: cfg.tts_voice.clone(),
            buf: Vec::with_capacity(Self::BUDGET_BYTES),
        })
    }

    fn push(&mut self, bytes: &[u8]) {
        let remaining = Self::BUDGET_BYTES.saturating_sub(self.buf.len());
        if remaining > 0 {
            let take = remaining.min(bytes.len());
            self.buf.extend_from_slice(&bytes[..take]);
        }
    }

    /// Save the reject-profile once enough audio was collected.
    fn save(self) {
        if self.buf.len() < 16_000 {
            return; // <0.5 s — not enough to voiceprint
        }
        match crate::recorder::save_reject_profile_from_pcm(&self.cfg, &self.key, &self.buf) {
            Ok(path) => tracing::info!(
                target: "jfc::voice::speaker",
                path = %path.display(),
                voice = %self.key,
                "auto-enrolled self-voice reject profile from read-aloud"
            ),
            Err(err) => tracing::debug!(
                target: "jfc::voice::speaker",
                error = %err,
                "self-voice auto-enroll skipped (not enough voiced audio)"
            ),
        }
    }
}

/// Handle to a live streaming-TTS session. Drop or [`cancel`](Self::cancel) to
/// tear it down.
pub struct StreamingTts {
    cmd_tx: mpsc::Sender<Cmd>,
}

impl StreamingTts {
    /// Open the TTS WebSocket + local player and start the session loop.
    pub async fn connect(cfg: &VoiceConfig, token: &str, user_agent: &str) -> Result<Self> {
        let player = PcmPlayback::start(cfg)?;
        let opts = TtsOptions::from_config(cfg);
        let req = tts::build_request(&tts::resolve_tts_base(cfg), token, user_agent, &opts)?;
        let (ws, _resp) = tokio_tungstenite::connect_async(req)
            .await
            .context("text_to_speech WebSocket connect/upgrade failed")?;
        let (cmd_tx, cmd_rx) = mpsc::channel(COMMAND_BUFFER);
        // Mark playback active for the VAD echo guard; the session loop clears it
        // when it exits (paired add/sub).
        PLAYBACK_SESSIONS.fetch_add(1, Ordering::AcqRel);
        let self_enroll = SelfEnroll::arm(cfg);
        tokio::spawn(session_loop(ws, player, cmd_rx, self_enroll));
        Ok(Self { cmd_tx })
    }

    /// Feed a chunk of text to synthesize (ideally a complete sentence).
    pub async fn feed(&self, text: &str) -> bool {
        self.cmd_tx.send(Cmd::Text(text.to_owned())).await.is_ok()
    }

    /// Signal end of input and await playback draining; returns what was played.
    pub async fn finish(&self) -> TtsPlaybackStats {
        let (tx, rx) = oneshot::channel();
        if self.cmd_tx.send(Cmd::Finish(tx)).await.is_err() {
            return TtsPlaybackStats::default();
        }
        rx.await.unwrap_or_default()
    }

    /// Barge-in: stop playback and tear down immediately, dropping buffered audio.
    pub fn cancel(&self) {
        let _ = self.cmd_tx.try_send(Cmd::Cancel);
    }
}

async fn session_loop<S>(
    ws: tokio_tungstenite::WebSocketStream<S>,
    mut player: PcmPlayback,
    mut cmd_rx: mpsc::Receiver<Cmd>,
    mut self_enroll: Option<SelfEnroll>,
) where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    let (mut sink, mut stream) = ws.split();
    let mut keepalive = tokio::time::interval(KEEPALIVE_INTERVAL);
    keepalive.tick().await;

    let mut stats = TtsPlaybackStats::default();
    let mut finish_waiter: Option<oneshot::Sender<TtsPlaybackStats>> = None;
    let mut closing = false;
    let mut killed = false;

    loop {
        tokio::select! {
            _ = keepalive.tick() => {
                let _ = sink
                    .send(Message::Text(r#"{"type":"keep_alive"}"#.to_owned()))
                    .await;
            }
            cmd = cmd_rx.recv() => match cmd {
                Some(Cmd::Text(text)) => {
                    let cleaned = tts::sanitize_tts_text(&text);
                    if !cleaned.is_empty() {
                        let msg = serde_json::json!({"type": "text_chunk", "text": cleaned})
                            .to_string();
                        if sink.send(Message::Text(msg)).await.is_err() {
                            break;
                        }
                    }
                }
                Some(Cmd::Finish(waiter)) => {
                    closing = true;
                    finish_waiter = Some(waiter);
                    let _ = sink
                        .send(Message::Text(r#"{"type":"close_stream"}"#.to_owned()))
                        .await;
                }
                Some(Cmd::Cancel) | None => {
                    killed = true;
                    player.kill();
                    let _ = sink.send(Message::Close(None)).await;
                    break;
                }
            },
            msg = stream.next() => match msg {
                Some(Ok(Message::Binary(bytes))) => {
                    stats.audio_bytes = stats.audio_bytes.saturating_add(bytes.len());
                    stats.chunks = stats.chunks.saturating_add(1);
                    // Tee the played PCM for one-shot self-voice enrollment.
                    if let Some(se) = self_enroll.as_mut() {
                        se.push(&bytes);
                    }
                    if player.write_audio(&bytes).await.is_err() {
                        break;
                    }
                }
                Some(Ok(Message::Text(raw))) => match tts::parse_server_frame(&raw) {
                    // The server flushes `SpeechComplete` after each `close_stream`.
                    // We only close once the caller has signalled `finish`.
                    ServerFrame::Complete => {
                        if closing {
                            break;
                        }
                    }
                    ServerFrame::Error(_) => break,
                    ServerFrame::Ignore => {}
                },
                None | Some(Ok(Message::Close(_))) => break,
                Some(Err(_)) => break,
                _ => {}
            },
        }
    }

    if !killed {
        // Let the player drain the audio already handed to it.
        let _ = player.finish().await;
        // A complete turn played — bank the self-voice reject profile if armed.
        if let Some(se) = self_enroll.take() {
            se.save();
        }
    }
    // Playback is no longer active — lift the VAD echo guard.
    PLAYBACK_SESSIONS.fetch_sub(1, Ordering::AcqRel);
    if let Some(waiter) = finish_waiter {
        let _ = waiter.send(stats);
    }
}

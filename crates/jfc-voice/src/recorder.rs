//! Voice recorder state machine.
//!
//! Drives the full push-to-talk lifecycle:
//!
//! ```text
//! Idle
//!   → start_recording() → Recording (audio capture + streaming STT)
//!   → stop_recording()  → Processing (finalize transcript)
//!   → Idle              (transcript emitted via callback)
//! ```
//!
//! For hold mode: `start` on key-down, `stop` on key-up.
//! For tap mode:  first `tap` starts recording, second `tap` stops it.

use std::sync::Arc;
use tokio::sync::{mpsc, Mutex};
use tracing::{debug, info, warn};

use crate::audio::{AudioCapture, CaptureBackend};
use crate::backends;
use crate::config::{VoiceConfig, VoiceMode};
use crate::vad::{Vad, VadEvent};

/// Current voice state (exposed to the TUI for rendering).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum VoiceState {
    /// Idle — not recording.
    #[default]
    Idle,
    /// Currently recording audio.
    Recording,
    /// Recording stopped; waiting for STT to return the transcript.
    Processing,
}

impl VoiceState {
    pub fn label(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Recording => "●rec",
            Self::Processing => "…stt",
        }
    }
}

/// Events emitted by the recorder to the TUI.
#[derive(Debug, Clone)]
pub enum VoiceTranscriptEvent {
    /// Interim partial transcript — update the status indicator but don't inject yet.
    Interim(String),
    /// Final transcript — inject into the textarea.
    Final(String),
    /// An error occurred.
    Error(String),
    /// State changed.
    StateChanged(VoiceState),
}

/// The voice recorder — manages the capture+STT pipeline.
pub struct VoiceRecorder {
    cfg: VoiceConfig,
    state: Arc<Mutex<VoiceState>>,
    audio_buf: Arc<Mutex<Vec<u8>>>,
    /// Stop signal for the recording task.
    stop_tx: Option<tokio::sync::oneshot::Sender<()>>,
    /// Stop signal for the VAD listen loop (VAD mode only).
    vad_stop_tx: Option<tokio::sync::oneshot::Sender<()>>,
    /// Output channel for transcript events.
    pub events: mpsc::UnboundedSender<VoiceTranscriptEvent>,
}

impl VoiceRecorder {
    pub fn new(cfg: VoiceConfig, events: mpsc::UnboundedSender<VoiceTranscriptEvent>) -> Self {
        Self {
            cfg,
            state: Arc::new(Mutex::new(VoiceState::Idle)),
            audio_buf: Arc::new(Mutex::new(Vec::new())),
            stop_tx: None,
            vad_stop_tx: None,
            events,
        }
    }

    /// Start the VAD listen loop (VAD mode only).
    /// The loop runs continuously until `cancel()` is called.
    pub async fn start_vad_loop(&mut self) {
        if self.vad_stop_tx.is_some() {
            return; // already running
        }
        let backend = match AudioCapture::detect_backend().await {
            Some(b) => b,
            None => {
                send_or_debug(
                    &self.events,
                    VoiceTranscriptEvent::Error(
                        "No audio backend for VAD (install arecord/sox/ffmpeg)".to_owned(),
                    ),
                );
                return;
            }
        };
        info!(target: "jfc::voice", backend = %backend.label(), "starting VAD listen loop");
        let (vad_stop_tx, vad_stop_rx) = tokio::sync::oneshot::channel::<()>();
        self.vad_stop_tx = Some(vad_stop_tx);

        let cfg = self.cfg.clone();
        let events = self.events.clone();
        let state = Arc::clone(&self.state);
        tokio::spawn(async move {
            vad_listen_loop(backend, cfg, events, state, vad_stop_rx).await;
        });
    }

    /// Current state (cheap clone).
    pub async fn state(&self) -> VoiceState {
        *self.state.lock().await
    }

    /// Handle a push-to-talk activation.
    ///
    /// - Hold mode: call `activate(true)` on key-down, `activate(false)` on key-up.
    /// - Tap mode: `activate(true)` on each tap (toggles recording).
    pub async fn activate(&mut self, pressed: bool) {
        let state = *self.state.lock().await;
        match (self.cfg.mode, pressed, state) {
            (VoiceMode::Hold, true, VoiceState::Idle) => self.start_recording().await,
            (VoiceMode::Hold, false, VoiceState::Recording) => self.stop_recording().await,
            (VoiceMode::Tap, true, VoiceState::Idle) => self.start_recording().await,
            (VoiceMode::Tap, true, VoiceState::Recording) => self.stop_recording().await,
            // VAD mode: Space key force-stops an active utterance early.
            (VoiceMode::Vad, true, VoiceState::Recording) => self.stop_recording().await,
            // Remaining cases are intentionally inactive:
            // - hold key-up when not recording (spurious release)
            // - tap while STT is processing (debounce)
            // - hold key-down when already recording (key repeat)
            // - VAD mode key events that don't apply
            (_, _, _) => {
                tracing::trace!(
                    target: "jfc::voice",
                    mode = ?self.cfg.mode,
                    pressed,
                    state = ?state,
                    "activate: no action for this (mode, pressed, state) combination"
                );
            }
        }
    }

    async fn start_recording(&mut self) {
        info!(target: "jfc::voice", "start_recording");
        self.set_state(VoiceState::Recording).await;

        let backend = match AudioCapture::detect_backend().await {
            Some(b) => b,
            None => {
                self.emit_error("No audio recording backend found (install arecord, sox, or ffmpeg)");
                self.set_state(VoiceState::Idle).await;
                return;
            }
        };

        let buf = Arc::clone(&self.audio_buf);
        let state = Arc::clone(&self.state);
        let events = self.events.clone();
        let (stop_tx, stop_rx) = tokio::sync::oneshot::channel::<()>();
        self.stop_tx = Some(stop_tx);

        tokio::spawn(async move {
            record_loop(backend, buf, state, events, stop_rx).await;
        });
    }

    async fn stop_recording(&mut self) {
        info!(target: "jfc::voice", "stop_recording");
        // Signal the recording task to stop (receiver may already be gone if it exited early).
        if let Some(tx) = self.stop_tx.take() {
            if tx.send(()).is_err() {
                debug!(target: "jfc::voice", "stop signal had no receiver (task already finished)");
            }
        }
        self.set_state(VoiceState::Processing).await;

        // Transcribe the buffered audio
        let pcm = {
            let mut guard = self.audio_buf.lock().await;
            std::mem::take(&mut *guard)
        };

        let cfg = self.cfg.clone();
        let events = self.events.clone();
        let state = Arc::clone(&self.state);
        tokio::spawn(async move {
            transcribe_and_emit(pcm, &cfg, events, state).await;
        });
    }

    fn emit_error(&self, msg: &str) {
        warn!(target: "jfc::voice", error = %msg, "voice error");
        // Send errors mean the TUI event receiver has closed — expected on shutdown.
        self.events
            .send(VoiceTranscriptEvent::Error(msg.to_owned()))
            .unwrap_or_else(|_| debug!(target: "jfc::voice", "event channel closed"));
    }

    async fn set_state(&self, s: VoiceState) {
        *self.state.lock().await = s;
        self.events
            .send(VoiceTranscriptEvent::StateChanged(s))
            .unwrap_or_else(|_| debug!(target: "jfc::voice", "event channel closed on state change"));
    }

    /// Cancel any in-progress recording and reset to Idle.
    pub async fn cancel(&mut self) {
        if let Some(tx) = self.stop_tx.take() {
            if tx.send(()).is_err() {
                debug!(target: "jfc::voice", "cancel: stop signal had no receiver");
            }
        }
        self.audio_buf.lock().await.clear();
        self.set_state(VoiceState::Idle).await;
    }
}

/// Record audio into `buf` until `stop_rx` fires.
async fn record_loop(
    backend: CaptureBackend,
    buf: Arc<Mutex<Vec<u8>>>,
    state: Arc<Mutex<VoiceState>>,
    events: mpsc::UnboundedSender<VoiceTranscriptEvent>,
    stop_rx: tokio::sync::oneshot::Receiver<()>,
) {
    let capture = match AudioCapture::start(backend).await {
        Ok(c) => c,
        Err(err) => {
            warn!(target: "jfc::voice", error = %err, "failed to start audio capture");
            set_idle_and_notify(&state, &events, err.to_string()).await;
            return;
        }
    };

    debug!(target: "jfc::voice", backend = %backend.label(), "recording started");
    let tail = run_capture_loop(capture, &buf, stop_rx).await;
    buf.lock().await.extend_from_slice(&tail);
    debug!(target: "jfc::voice", "recording stopped");
}

/// Inner capture loop; returns any buffered tail audio from `capture.stop()`.
async fn run_capture_loop(
    mut capture: AudioCapture,
    buf: &Arc<Mutex<Vec<u8>>>,
    stop_rx: tokio::sync::oneshot::Receiver<()>,
) -> Vec<u8> {
    let mut chunk = vec![0u8; 3200]; // 100ms at 16kHz 16-bit mono
    tokio::pin!(stop_rx);
    loop {
        tokio::select! {
            _ = &mut stop_rx => break,
            n = capture.read_chunk(&mut chunk) => match n {
                Ok(0) => break,
                Ok(n) => buf.lock().await.extend_from_slice(&chunk[..n]),
                Err(err) => {
                    debug!(target: "jfc::voice", error = %err, "read_chunk error; stopping");
                    break;
                }
            },
        }
    }
    capture.stop().await
}

async fn set_idle_and_notify(
    state: &Arc<Mutex<VoiceState>>,
    events: &mpsc::UnboundedSender<VoiceTranscriptEvent>,
    error_msg: String,
) {
    *state.lock().await = VoiceState::Idle;
    events
        .send(VoiceTranscriptEvent::Error(error_msg))
        .unwrap_or_else(|_| debug!(target: "jfc::voice", "event channel closed"));
    events
        .send(VoiceTranscriptEvent::StateChanged(VoiceState::Idle))
        .unwrap_or_else(|_| debug!(target: "jfc::voice", "event channel closed"));
}

/// Run STT and emit the transcript.
async fn transcribe_and_emit(
    pcm: Vec<u8>,
    cfg: &VoiceConfig,
    events: mpsc::UnboundedSender<VoiceTranscriptEvent>,
    state: Arc<Mutex<VoiceState>>,
) {
    let result = backends::transcribe(&pcm, cfg).await;
    *state.lock().await = VoiceState::Idle;
    send_or_debug(&events, VoiceTranscriptEvent::StateChanged(VoiceState::Idle));

    match result {
        Ok(Some(text)) => {
            info!(target: "jfc::voice", chars = text.len(), "STT transcript received");
            send_or_debug(&events, VoiceTranscriptEvent::Final(text));
        }
        Ok(None) => {
            debug!(target: "jfc::voice", "STT returned empty transcript (silence)");
        }
        Err(err) => {
            warn!(target: "jfc::voice", error = %err, "STT failed");
            send_or_debug(&events, VoiceTranscriptEvent::Error(err.to_string()));
        }
    }
}

#[inline]
fn send_or_debug(tx: &mpsc::UnboundedSender<VoiceTranscriptEvent>, ev: VoiceTranscriptEvent) {
    tx.send(ev)
        .unwrap_or_else(|_| debug!(target: "jfc::voice", "event channel closed"));
}

/// VAD continuous-listen loop.
///
/// Streams audio indefinitely, running it through the VAD energy detector.
/// When speech is detected, buffers PCM until silence, then transcribes and
/// emits the result. Loops back to listening after each utterance.
async fn vad_listen_loop(
    backend: CaptureBackend,
    cfg: VoiceConfig,
    events: mpsc::UnboundedSender<VoiceTranscriptEvent>,
    state: Arc<Mutex<VoiceState>>,
    stop_rx: tokio::sync::oneshot::Receiver<()>,
) {
    debug!(target: "jfc::voice::vad", "VAD loop starting");
    tokio::pin!(stop_rx);

    loop {
        // ── Listening phase (Idle) ─────────────────────────────────────────
        let mut capture = match AudioCapture::start(backend).await {
            Ok(c) => c,
            Err(err) => {
                send_or_debug(&events, VoiceTranscriptEvent::Error(err.to_string()));
                break;
            }
        };

        let mut detector = Vad::new();
        let mut utterance_buf: Vec<u8> = Vec::new();
        let mut chunk = vec![0u8; 640]; // 20ms at 16kHz 16-bit mono

        // Pre-roll ring buffer (Silero's `speech_pad_ms`): the onset debounce
        // only fires SpeechStart after a few voiced frames, so without this the
        // first ~60ms of audio — often the leading consonant — is discarded,
        // which both hurts the transcript and feeds Whisper a clipped utterance
        // it's more likely to hallucinate on. Keep the last PREROLL_FRAMES of
        // pre-onset audio and prepend it when speech starts.
        const PREROLL_FRAMES: usize = 10; // ~200ms at 20ms/frame
        let mut preroll: std::collections::VecDeque<Vec<u8>> =
            std::collections::VecDeque::with_capacity(PREROLL_FRAMES);

        // Wait for speech start (or stop signal)
        let speech_started = loop {
            tokio::select! {
                _ = &mut stop_rx => break false,
                n = capture.read_chunk(&mut chunk) => match n {
                    Ok(0) | Err(_) => break false,
                    Ok(n) => {
                        let frame = &chunk[..n];
                        let vad_events = detector.push(frame);
                        if vad_events.contains(&VadEvent::SpeechStart) {
                            // Prepend the buffered pre-onset audio so the first
                            // phoneme isn't clipped, then the onset frame itself.
                            for pf in preroll.drain(..) {
                                utterance_buf.extend_from_slice(&pf);
                            }
                            utterance_buf.extend_from_slice(frame);
                            break true;
                        }
                        // Not speech yet — keep it in the rolling pre-roll.
                        if preroll.len() == PREROLL_FRAMES {
                            preroll.pop_front();
                        }
                        preroll.push_back(frame.to_vec());
                    }
                }
            }
        };

        if !speech_started {
            // Stop signal or audio error
            capture.stop().await;
            break;
        }

        // ── Recording phase ────────────────────────────────────────────────
        *state.lock().await = VoiceState::Recording;
        send_or_debug(&events, VoiceTranscriptEvent::StateChanged(VoiceState::Recording));
        info!(target: "jfc::voice::vad", "speech detected, recording utterance");

        // Safety cap: if the noise floor stays above the VAD threshold (loud
        // room / high mic gain), SpeechEnd may never fire. Force-end after
        // MAX_UTTERANCE so the loop can't hang forever. Override via
        // JFC_VAD_MAX_UTTERANCE_MS.
        //
        // This is a stuck-recording safety net, NOT a speech-length limit —
        // it must be long enough that normal continuous speech never hits it
        // (that was the "it cuts me off mid-sentence" bug, where a 20s cap
        // fired while the user was still talking). 90s is well beyond any
        // single spoken utterance while still bounding a truly wedged loop.
        let max_utterance_ms: u64 = std::env::var("JFC_VAD_MAX_UTTERANCE_MS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(90_000);
        let max_bytes = (max_utterance_ms as usize * 16_000 * 2 / 1000).max(640);
        let mut frames_seen: u64 = 0;
        let mut max_rms: u32 = 0;

        let speech_ended = loop {
            tokio::select! {
                _ = &mut stop_rx => break false,
                n = capture.read_chunk(&mut chunk) => match n {
                    Ok(0) | Err(_) => break true,
                    Ok(n) => {
                        let frame = &chunk[..n];
                        utterance_buf.extend_from_slice(frame);
                        frames_seen += 1;
                        let rms = crate::vad::rms_energy(frame);
                        max_rms = max_rms.max(rms);
                        // Periodic heartbeat so we can see the loop is alive
                        // and what RMS it's reading (helps diagnose a high
                        // noise floor that prevents SpeechEnd).
                        if frames_seen.is_multiple_of(50) {
                            debug!(
                                target: "jfc::voice::vad",
                                frames = frames_seen,
                                frame_rms = rms,
                                max_rms,
                                buf_bytes = utterance_buf.len(),
                                "recording heartbeat"
                            );
                        }
                        let vad_events = detector.push(frame);
                        if vad_events.contains(&VadEvent::SpeechEnd) {
                            debug!(target: "jfc::voice::vad", frames = frames_seen, "SpeechEnd fired");
                            break true;
                        }
                        if utterance_buf.len() >= max_bytes {
                            warn!(
                                target: "jfc::voice::vad",
                                frames = frames_seen,
                                max_rms,
                                cap_ms = max_utterance_ms,
                                "utterance hit max-duration cap before silence — \
                                 noise floor may be above the VAD threshold. \
                                 Transcribing what we have; consider raising JFC_VAD_THRESHOLD."
                            );
                            break true;
                        }
                    }
                }
            }
        };
        info!(
            target: "jfc::voice::vad",
            frames = frames_seen,
            max_rms,
            bytes = utterance_buf.len(),
            speech_ended,
            "recording phase ended"
        );

        // Drain the capture subprocess
        let tail = capture.stop().await;
        utterance_buf.extend_from_slice(&tail);

        if !speech_ended {
            // Stop signal arrived mid-utterance — transcribe what we have
        }

        // ── Transcription phase ────────────────────────────────────────────
        *state.lock().await = VoiceState::Processing;
        send_or_debug(&events, VoiceTranscriptEvent::StateChanged(VoiceState::Processing));

        let pcm = std::mem::take(&mut utterance_buf);
        info!(
            target: "jfc::voice::vad",
            bytes = pcm.len(),
            backend = ?cfg.effective_backend(),
            "transcribing utterance"
        );
        let result = backends::transcribe(&pcm, &cfg).await;

        *state.lock().await = VoiceState::Idle;
        send_or_debug(&events, VoiceTranscriptEvent::StateChanged(VoiceState::Idle));

        match result {
            Ok(Some(text)) => {
                info!(target: "jfc::voice::vad", chars = text.len(), "VAD utterance transcribed");
                send_or_debug(&events, VoiceTranscriptEvent::Final(text));
            }
            Ok(None) => {
                debug!(target: "jfc::voice::vad", "VAD utterance was empty after transcription");
            }
            Err(err) => {
                warn!(target: "jfc::voice::vad", error = %err, "VAD transcription failed");
                send_or_debug(&events, VoiceTranscriptEvent::Error(err.to_string()));
            }
        }

        // If stop was signalled, exit the loop
        if !speech_ended {
            break;
        }

        // Otherwise loop back and listen for the next utterance
        detector.reset();
    }

    *state.lock().await = VoiceState::Idle;
    send_or_debug(&events, VoiceTranscriptEvent::StateChanged(VoiceState::Idle));
    debug!(target: "jfc::voice::vad", "VAD loop exited");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn voice_state_labels_normal() {
        assert_eq!(VoiceState::Idle.label(), "idle");
        assert_eq!(VoiceState::Recording.label(), "●rec");
        assert_eq!(VoiceState::Processing.label(), "…stt");
    }

    #[tokio::test]
    async fn recorder_starts_idle_normal() {
        let (tx, _rx) = mpsc::unbounded_channel();
        let rec = VoiceRecorder::new(VoiceConfig::default(), tx);
        assert_eq!(rec.state().await, VoiceState::Idle);
    }

    #[tokio::test]
    async fn cancel_from_idle_is_noop_robust() {
        let (tx, _rx) = mpsc::unbounded_channel();
        let mut rec = VoiceRecorder::new(VoiceConfig::default(), tx);
        rec.cancel().await; // should not panic
        assert_eq!(rec.state().await, VoiceState::Idle);
    }
}

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
use tokio::sync::{Mutex, mpsc};
use tracing::{debug, info, warn};

use crate::audio::{AudioCapture, CaptureBackend};
use crate::backends;
use crate::config::{VadEngine, VoiceConfig, VoiceMode};
use crate::vad::{Vad, VadEvent};

/// Runtime dispatch over the available VAD engines, so the listen loop is
/// engine-agnostic. The neural variant only exists when the `vad-neural`
/// feature is compiled in; selection happens once per loop based on
/// `VoiceConfig::vad_engine`, falling back to energy if the neural model
/// can't be constructed.
enum VadDetector {
    Energy(Vad),
    #[cfg(feature = "vad-neural")]
    Neural(crate::neural_vad::NeuralVad),
}

impl VadDetector {
    /// Choose the engine from config, falling back to the energy detector if
    /// neural is requested but unavailable (feature off, or model load failed).
    fn select(cfg: &VoiceConfig) -> Self {
        match cfg.vad_engine {
            VadEngine::Neural => {
                #[cfg(feature = "vad-neural")]
                {
                    match crate::neural_vad::NeuralVad::new() {
                        Ok(nv) => {
                            info!(target: "jfc::voice::vad", "using neural (Silero) VAD engine");
                            return Self::Neural(nv);
                        }
                        Err(err) => {
                            warn!(
                                target: "jfc::voice::vad",
                                error = %err,
                                "neural VAD unavailable, falling back to energy VAD"
                            );
                        }
                    }
                }
                #[cfg(not(feature = "vad-neural"))]
                {
                    warn!(
                        target: "jfc::voice::vad",
                        "neural VAD requested but jfc-voice was built without the \
                         `vad-neural` feature; using energy VAD"
                    );
                }
                Self::Energy(Vad::new())
            }
            VadEngine::Energy => Self::Energy(Vad::new()),
        }
    }

    fn push(&mut self, pcm: &[u8]) -> Vec<VadEvent> {
        match self {
            Self::Energy(v) => v.push(pcm),
            #[cfg(feature = "vad-neural")]
            Self::Neural(v) => v.push(pcm),
        }
    }

    fn reset(&mut self) {
        match self {
            Self::Energy(v) => v.reset(),
            #[cfg(feature = "vad-neural")]
            Self::Neural(v) => v.reset(),
        }
    }
}

/// Optional target-speaker gate (the BVC decision layer). When enabled in
/// config and a profile is enrolled, a captured utterance is scored against the
/// enrolled primary speaker; non-matching segments (a background movie/TV voice
/// or another person) are dropped instead of transcribed. No-ops cleanly when
/// disabled or unenrolled — default behavior is unchanged.
struct SpeakerGate {
    profile: Option<crate::speaker::SpeakerProfile>,
    /// Embedding backend used for scoring. The neural ONNX backend (feature
    /// `speaker-neural` + `JFC_VOICE_SPEAKER_MODEL`) when available, else the
    /// null embedder (classical Mahalanobis+pitch path).
    embedder: Box<dyn crate::speaker::SpeakerEmbedder>,
}

impl SpeakerGate {
    /// Load the gate from config: returns an inert gate (no profile) unless the
    /// gate is enabled AND a profile JSON loads successfully.
    fn from_config(cfg: &VoiceConfig) -> Self {
        if !cfg.speaker_gate {
            return Self {
                profile: None,
                embedder: Box::new(crate::speaker::NullEmbedder),
            };
        }
        let embedder = crate::speaker::default_embedder();
        let path = Self::profile_path(cfg);
        match crate::speaker::SpeakerProfile::load(&path) {
            Ok(mut profile) => {
                if let Some(t) = cfg.speaker_threshold {
                    profile = profile.with_threshold(t);
                }
                info!(
                    target: "jfc::voice::speaker",
                    path = %path.display(),
                    threshold = profile.threshold,
                    backend = embedder.name(),
                    neural = profile.neural.is_some(),
                    "target-speaker gate enabled"
                );
                Self {
                    profile: Some(profile),
                    embedder,
                }
            }
            Err(err) => {
                warn!(
                    target: "jfc::voice::speaker",
                    path = %path.display(),
                    error = %err,
                    "speaker gate enabled but no usable profile; gate disabled \
                     (enroll one to filter background voices)"
                );
                Self {
                    profile: None,
                    embedder,
                }
            }
        }
    }

    /// Resolve the profile path: explicit config/env, else `<config>/voice/
    /// speaker_profile.json` under the user config dir.
    fn profile_path(cfg: &VoiceConfig) -> std::path::PathBuf {
        if let Some(p) = &cfg.speaker_profile_path {
            return std::path::PathBuf::from(p);
        }
        let base = dirs_config_dir().unwrap_or_else(|| std::path::PathBuf::from("."));
        base.join("jfc").join("voice").join("speaker_profile.json")
    }

    /// Decide whether a captured utterance should be transcribed. Returns `true`
    /// (transcribe) when the gate is inert, or when the segment matches the
    /// enrolled speaker; `false` to drop a non-matching (background) voice.
    fn admits(&self, pcm: &[u8]) -> bool {
        let Some(profile) = &self.profile else {
            return true;
        };
        let score = profile.score_with(pcm, self.embedder.as_ref());
        if score.voiced_frames == 0 {
            // Couldn't measure — fail open so we never silently swallow speech.
            return true;
        }
        if !score.accepted {
            info!(
                target: "jfc::voice::speaker",
                mahalanobis = score.mahalanobis,
                threshold = profile.threshold,
                cosine = score.cosine,
                pitch_ok = score.pitch_ok,
                "dropped a non-primary-speaker utterance (background voice)"
            );
        }
        score.accepted
    }
}

/// Best-effort user config dir without pulling in the `dirs` crate.
fn dirs_config_dir() -> Option<std::path::PathBuf> {
    if let Ok(x) = std::env::var("XDG_CONFIG_HOME") {
        if !x.is_empty() {
            return Some(std::path::PathBuf::from(x));
        }
    }
    std::env::var("HOME")
        .ok()
        .map(|h| std::path::PathBuf::from(h).join(".config"))
}

/// The default on-disk path for the enrolled speaker profile, honoring an
/// explicit `cfg.speaker_profile_path` and falling back to the user config dir.
pub fn default_speaker_profile_path(cfg: &VoiceConfig) -> std::path::PathBuf {
    SpeakerGate::profile_path(cfg)
}

/// Enroll the primary speaker by capturing ~`secs` seconds of microphone audio
/// and writing a [`crate::speaker::SpeakerProfile`] to `cfg`'s profile path.
///
/// This is the one-off setup step that makes the target-speaker gate useful:
/// the user speaks naturally for a few seconds; we build their voiceprint and
/// persist it. Returns the path written. Speak only yourself during enrollment.
pub async fn enroll_primary_speaker(
    cfg: &VoiceConfig,
    secs: f64,
) -> Result<std::path::PathBuf, String> {
    let backend = AudioCapture::detect_backend()
        .await
        .ok_or_else(|| "no audio capture backend available".to_owned())?;
    let mut capture = AudioCapture::start(backend)
        .await
        .map_err(|e| e.to_string())?;

    let target_bytes = (secs * 16_000.0 * 2.0) as usize;
    let mut pcm: Vec<u8> = Vec::with_capacity(target_bytes);
    let mut chunk = vec![0u8; 640];
    while pcm.len() < target_bytes {
        match capture.read_chunk(&mut chunk).await {
            Ok(0) => break,
            Ok(n) => pcm.extend_from_slice(&chunk[..n]),
            Err(e) => return Err(e.to_string()),
        }
    }
    pcm.extend_from_slice(&capture.stop().await);

    let mut profile = crate::speaker::SpeakerProfile::enroll_from_pcm(&pcm).ok_or_else(|| {
        "not enough voiced speech to enroll — speak continuously for a few seconds".to_owned()
    })?;
    // Attach a learned neural embedding when a model is configured (no-op
    // otherwise), so the gate scores with the SOTA backend.
    let embedder = crate::speaker::default_embedder();
    profile = profile.with_neural_embedding(embedder.as_ref(), &pcm);
    let path = SpeakerGate::profile_path(cfg);
    profile.save(&path).map_err(|e| e.to_string())?;
    info!(
        target: "jfc::voice::speaker",
        path = %path.display(),
        frames = profile.enrolled_frames,
        threshold = profile.threshold,
        backend = embedder.name(),
        neural = profile.neural.is_some(),
        "enrolled primary speaker profile"
    );
    Ok(path)
}

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
                self.emit_error(
                    "No audio recording backend found (install arecord, sox, or ffmpeg)",
                );
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
            .unwrap_or_else(
                |_| debug!(target: "jfc::voice", "event channel closed on state change"),
            );
    }

    /// Cancel any in-progress recording AND the VAD listen loop, resetting to
    /// Idle. Both stop signals must fire: in VAD mode the long-running
    /// `vad_stop_tx` owns the capture loop, while `stop_tx` only exists for a
    /// hold/tap recording. A previous version signalled `stop_tx` only, so
    /// `/voice off` left the VAD loop running in the background (mic stayed hot,
    /// utterances kept being transcribed after the user turned voice off).
    pub async fn cancel(&mut self) {
        if let Some(tx) = self.stop_tx.take() {
            if tx.send(()).is_err() {
                debug!(target: "jfc::voice", "cancel: recording stop signal had no receiver");
            }
        }
        if let Some(tx) = self.vad_stop_tx.take() {
            if tx.send(()).is_err() {
                debug!(target: "jfc::voice", "cancel: VAD stop signal had no receiver");
            }
        }
        self.audio_buf.lock().await.clear();
        self.set_state(VoiceState::Idle).await;
    }

    /// Whether the VAD listen loop is currently running. Used by tests and the
    /// UI to reflect mic-hot state accurately.
    pub fn vad_loop_running(&self) -> bool {
        self.vad_stop_tx.is_some()
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
    send_or_debug(
        &events,
        VoiceTranscriptEvent::StateChanged(VoiceState::Idle),
    );

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

/// Default max-utterance safety cap (ms). High enough that no real single
/// spoken utterance reaches it; it only bounds a wedged loop (e.g. a noise
/// floor stuck above the VAD threshold so SpeechEnd never fires).
const DEFAULT_MAX_UTTERANCE_MS: u64 = 90_000;
/// Hard floor for the cap. A long sentence routinely runs 20-30s; clamping
/// below this would truncate normal speech mid-sentence — the reported
/// "long sentence gets cut off" bug, which happened when the env was set to a
/// too-aggressive value (e.g. 15000). Anything below the floor is ignored.
const MIN_MAX_UTTERANCE_MS: u64 = 45_000;

/// Resolve the max-utterance cap from `JFC_VAD_MAX_UTTERANCE_MS`, clamped to a
/// safe floor so a misconfigured/too-small value can't truncate normal speech.
fn max_utterance_cap_ms() -> u64 {
    let configured = std::env::var("JFC_VAD_MAX_UTTERANCE_MS")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(DEFAULT_MAX_UTTERANCE_MS);
    if configured < MIN_MAX_UTTERANCE_MS {
        warn!(
            target: "jfc::voice::vad",
            configured,
            floor = MIN_MAX_UTTERANCE_MS,
            "JFC_VAD_MAX_UTTERANCE_MS is below the safe floor — clamping so long \
             sentences aren't cut off mid-utterance"
        );
        MIN_MAX_UTTERANCE_MS
    } else {
        configured
    }
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

    // Target-speaker gate (BVC decision layer). Built once; inert unless enabled
    // in config AND a profile is enrolled. When active it drops utterances that
    // don't match the enrolled primary speaker (background TV/movie/other voice).
    let speaker_gate = SpeakerGate::from_config(&cfg);

    loop {
        // ── Listening phase (Idle) ─────────────────────────────────────────
        let mut capture = match AudioCapture::start(backend).await {
            Ok(c) => c,
            Err(err) => {
                send_or_debug(&events, VoiceTranscriptEvent::Error(err.to_string()));
                break;
            }
        };

        let mut detector = VadDetector::select(&cfg);
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
        send_or_debug(
            &events,
            VoiceTranscriptEvent::StateChanged(VoiceState::Recording),
        );
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
        let max_utterance_ms: u64 = max_utterance_cap_ms();
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
        send_or_debug(
            &events,
            VoiceTranscriptEvent::StateChanged(VoiceState::Processing),
        );

        let pcm = std::mem::take(&mut utterance_buf);

        // Target-speaker gate: drop a non-primary-speaker utterance (e.g. a
        // background movie/TV voice) before spending an STT call on it. Inert
        // when the gate is disabled or unenrolled.
        if !speaker_gate.admits(&pcm) {
            *state.lock().await = VoiceState::Idle;
            send_or_debug(
                &events,
                VoiceTranscriptEvent::StateChanged(VoiceState::Idle),
            );
            if speech_ended {
                detector.reset();
                continue;
            } else {
                break;
            }
        }

        info!(
            target: "jfc::voice::vad",
            bytes = pcm.len(),
            backend = ?cfg.effective_backend(),
            "transcribing utterance"
        );
        let result = backends::transcribe(&pcm, &cfg).await;

        *state.lock().await = VoiceState::Idle;
        send_or_debug(
            &events,
            VoiceTranscriptEvent::StateChanged(VoiceState::Idle),
        );

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
    send_or_debug(
        &events,
        VoiceTranscriptEvent::StateChanged(VoiceState::Idle),
    );
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

    // REGRESSION (`/voice off` left the mic hot): cancel() must signal the VAD
    // stop channel, not just the push-to-talk one. We install a vad_stop_tx
    // directly (start_vad_loop needs a real audio backend) and assert cancel
    // consumes it and the loop's receiver observes the stop.
    #[tokio::test]
    async fn cancel_stops_vad_listen_loop_robust() {
        let (tx, _rx) = mpsc::unbounded_channel();
        let mut rec = VoiceRecorder::new(
            VoiceConfig {
                mode: VoiceMode::Vad,
                ..Default::default()
            },
            tx,
        );
        let (vad_tx, mut vad_rx) = tokio::sync::oneshot::channel::<()>();
        rec.vad_stop_tx = Some(vad_tx);
        assert!(rec.vad_loop_running(), "loop should be marked running");

        rec.cancel().await;

        assert!(
            !rec.vad_loop_running(),
            "cancel must clear the VAD stop handle"
        );
        assert_eq!(rec.state().await, VoiceState::Idle);
        // The listen loop's receiver must see the stop signal (Ok), not a
        // dropped-sender error — proving cancel actually told it to stop.
        assert_eq!(
            vad_rx.try_recv(),
            Ok(()),
            "VAD loop must receive the stop signal"
        );
    }

    // REGRESSION (long sentence cut off mid-utterance): the max-utterance cap
    // must never fall below the safe floor, even if JFC_VAD_MAX_UTTERANCE_MS is
    // set to a too-aggressive value. The constants are the contract; assert them
    // directly so the test is hermetic (independent of the caller's env).
    #[test]
    fn max_utterance_floor_protects_long_sentences_normal() {
        // Default is well beyond any real single utterance.
        assert!(DEFAULT_MAX_UTTERANCE_MS >= 60_000);
        // Floor is at least ~45s — a long sentence (20-30s) is comfortably under.
        assert!(MIN_MAX_UTTERANCE_MS >= 30_000);
        assert!(DEFAULT_MAX_UTTERANCE_MS >= MIN_MAX_UTTERANCE_MS);

        // A 30s continuous sentence in bytes must be under the floor's byte cap,
        // so even the smallest allowed cap can't truncate it.
        let thirty_second_sentence = 30 * 16_000 * 2;
        let floor_bytes = (MIN_MAX_UTTERANCE_MS as usize * 16_000 * 2 / 1000).max(640);
        assert!(
            thirty_second_sentence < floor_bytes,
            "a 30s sentence ({thirty_second_sentence}) must fit under the floor cap ({floor_bytes})"
        );
    }

    // Robust: a too-small env value is clamped up to the floor (this is the
    // exact failure mode behind the user's report — env was set to 15000).
    #[test]
    fn too_small_env_cap_is_clamped_to_floor_robust() {
        const KEY: &str = "JFC_VAD_MAX_UTTERANCE_MS";
        // Save + restore the prior value so this test can't corrupt another
        // test that reads the same env (no serial_test dependency needed).
        let prev = std::env::var(KEY).ok();
        // SAFETY: env mutation; bounded to this test and restored immediately.
        unsafe { std::env::set_var(KEY, "15000") };
        let resolved = max_utterance_cap_ms();
        unsafe {
            match &prev {
                Some(v) => std::env::set_var(KEY, v),
                None => std::env::remove_var(KEY),
            }
        }
        assert_eq!(
            resolved, MIN_MAX_UTTERANCE_MS,
            "a 15s cap must be clamped up to the floor so long speech isn't cut off"
        );
    }

    // And a generous value passes through unclamped.
    #[test]
    fn large_env_cap_passes_through_normal() {
        const KEY: &str = "JFC_VAD_MAX_UTTERANCE_MS";
        let prev = std::env::var(KEY).ok();
        unsafe { std::env::set_var(KEY, "120000") };
        let resolved = max_utterance_cap_ms();
        unsafe {
            match &prev {
                Some(v) => std::env::set_var(KEY, v),
                None => std::env::remove_var(KEY),
            }
        }
        assert_eq!(
            resolved, 120_000,
            "a generous cap must pass through unchanged"
        );
    }

    // REGRESSION: cancel() must stop the VAD listen loop, not just a hold/tap
    // recording. Before the fix, cancel() only signalled stop_tx, so `/voice
    // off` left the VAD loop (and the mic) running. We simulate a running VAD
    // loop by installing a vad_stop_tx and asserting cancel consumes it and the
    // receiver observes the stop signal.
    #[tokio::test]
    async fn cancel_stops_vad_loop_robust() {
        let (tx, _rx) = mpsc::unbounded_channel();
        let mut rec = VoiceRecorder::new(VoiceConfig::default(), tx);
        let (vad_stop_tx, vad_stop_rx) = tokio::sync::oneshot::channel::<()>();
        rec.vad_stop_tx = Some(vad_stop_tx);
        assert!(
            rec.vad_loop_running(),
            "precondition: VAD loop marked running"
        );

        rec.cancel().await;

        assert!(
            !rec.vad_loop_running(),
            "cancel must clear the VAD stop sender"
        );
        assert_eq!(rec.state().await, VoiceState::Idle);
        // The loop's receiver must have been signalled (Ok) — i.e. it would break.
        assert_eq!(
            vad_stop_rx.await,
            Ok(()),
            "VAD loop must receive the stop signal"
        );
    }

    // Normal: vad_loop_running reflects whether a stop sender is installed.
    #[tokio::test]
    async fn vad_loop_running_reflects_state_normal() {
        let (tx, _rx) = mpsc::unbounded_channel();
        let mut rec = VoiceRecorder::new(VoiceConfig::default(), tx);
        assert!(!rec.vad_loop_running());
        let (vad_stop_tx, _rx2) = tokio::sync::oneshot::channel::<()>();
        rec.vad_stop_tx = Some(vad_stop_tx);
        assert!(rec.vad_loop_running());
    }

    #[test]
    fn vad_detector_energy_engine_selects_energy_normal() {
        let cfg = VoiceConfig {
            vad_engine: VadEngine::Energy,
            ..Default::default()
        };
        assert!(matches!(VadDetector::select(&cfg), VadDetector::Energy(_)));
    }

    #[test]
    fn vad_detector_processes_audio_through_dispatch_normal() {
        // The dispatch enum must forward push/reset to the underlying engine.
        let cfg = VoiceConfig::default();
        let mut det = VadDetector::select(&cfg);
        let loud: Vec<u8> = (0..320)
            .flat_map(|i| (if i % 2 == 0 { 5000i16 } else { -5000 }).to_le_bytes())
            .collect();
        let _ = det.push(&loud);
        det.reset(); // must not panic
    }

    #[cfg(not(feature = "vad-neural"))]
    #[test]
    fn vad_detector_neural_falls_back_to_energy_without_feature_robust() {
        // When the neural feature isn't compiled in, requesting it must fall
        // back to the energy engine rather than failing.
        let cfg = VoiceConfig {
            vad_engine: VadEngine::Neural,
            ..Default::default()
        };
        assert!(matches!(VadDetector::select(&cfg), VadDetector::Energy(_)));
    }

    #[cfg(feature = "vad-neural")]
    #[test]
    fn vad_detector_neural_engine_selects_neural_normal() {
        // With the feature on, the neural engine loads the bundled Silero model.
        let cfg = VoiceConfig {
            vad_engine: VadEngine::Neural,
            ..Default::default()
        };
        assert!(matches!(VadDetector::select(&cfg), VadDetector::Neural(_)));
    }

    // ── Target-speaker gate ────────────────────────────────────────────────

    /// A disabled gate (the default) must admit every utterance — proving the
    /// feature is fully backward-compatible / no-op when off.
    #[test]
    fn speaker_gate_disabled_admits_everything_normal() {
        let cfg = VoiceConfig::default(); // speaker_gate = false
        let gate = SpeakerGate::from_config(&cfg);
        assert!(gate.profile.is_none());
        // Even random bytes are admitted when the gate is inert.
        assert!(gate.admits(&vec![0u8; 6400]));
        assert!(gate.admits(b"\x01\x02\x03\x04"));
    }

    /// Enabled-but-unenrolled (no profile file) must also admit everything: the
    /// gate fails open rather than swallowing speech when misconfigured.
    #[test]
    fn speaker_gate_enabled_without_profile_admits_everything_robust() {
        let cfg = VoiceConfig {
            speaker_gate: true,
            speaker_profile_path: Some("/nonexistent/speaker_profile.json".to_owned()),
            ..Default::default()
        };
        let gate = SpeakerGate::from_config(&cfg);
        assert!(gate.profile.is_none(), "missing profile ⇒ inert gate");
        assert!(gate.admits(&vec![0u8; 6400]));
    }

    /// With an enrolled profile, the gate admits the enrolled speaker and drops
    /// an acoustically very different source. Uses synthetic signals (validates
    /// the wiring + decision, not real two-human-voice accuracy).
    #[test]
    fn speaker_gate_admits_self_drops_other_robust() {
        use crate::speaker::SpeakerProfile;
        // Synthesize an enrolled voice and persist it to a temp profile.
        let me = synth_pcm(130.0, 2.0, 1.0, 7);
        let profile = SpeakerProfile::enroll_from_pcm(&me).expect("enroll");
        let gate = SpeakerGate {
            profile: Some(profile),
            embedder: Box::new(crate::speaker::NullEmbedder),
        };

        // Same synthetic speaker → admitted.
        let me_again = synth_pcm(130.0, 1.0, 1.0, 21);
        assert!(gate.admits(&me_again), "enrolled speaker must be admitted");

        // A very different source (high pitch, different spectrum) → dropped.
        let other = synth_pcm(330.0, 1.0, 1.6, 9);
        assert!(
            !gate.admits(&other),
            "acoustically different voice must be dropped"
        );

        // Unmeasurable audio (silence) fails open → admitted.
        assert!(gate.admits(&vec![0u8; 16_000 * 2]));
    }

    /// Mirror of speaker.rs's synth helper for the recorder-level gate test.
    fn synth_pcm(f0: f64, secs: f64, tilt: f64, seed: u64) -> Vec<u8> {
        use std::f64::consts::TAU;
        let sr = 16_000.0;
        let n = (sr * secs) as usize;
        let mut state = seed.wrapping_add(0x9E3779B97F4A7C15);
        let mut rng = move || {
            state = state.wrapping_add(0x9E3779B97F4A7C15);
            let mut z = state;
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
            ((z ^ (z >> 31)) as f64 / u64::MAX as f64) - 0.5
        };
        let mut out = Vec::with_capacity(n * 2);
        for i in 0..n {
            let t = i as f64 / sr;
            let mut s = 0.0;
            for k in 1..=8 {
                s += (1.0 / (k as f64).powf(tilt)) * (TAU * f0 * k as f64 * t).sin();
            }
            s += 0.02 * rng();
            let v = (s / 3.0 * 12000.0).clamp(-32000.0, 32000.0) as i16;
            out.extend_from_slice(&v.to_le_bytes());
        }
        out
    }
}

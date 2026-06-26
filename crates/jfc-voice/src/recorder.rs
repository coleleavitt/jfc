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
                if !VadEngine::neural_runtime_enabled() {
                    warn!(
                        target: "jfc::voice::vad",
                        "neural VAD requested but JFC_VAD_ENABLE_NEURAL=1 is not set; using energy VAD"
                    );
                    return Self::Energy(Vad::new());
                }
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

    fn force_end(&mut self) -> bool {
        match self {
            Self::Energy(v) => v.force_end(),
            #[cfg(feature = "vad-neural")]
            Self::Neural(v) => v.force_end(),
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
    /// A normalized [0,1] RMS audio level for the current capture chunk. Emitted
    /// continuously while recording so the UI can animate the recording cursor.
    Level(f32),
    /// An error occurred.
    Error(String),
    /// State changed.
    StateChanged(VoiceState),
}

/// Resolves the current Claude.ai OAuth access token on demand. Supplied by the
/// embedding app (the TUI) so `jfc-voice` stays provider-neutral: it never
/// reaches into the auth/provider crates itself. `None` means no token is
/// available (not signed in), in which case the live Anthropic voice stream is
/// skipped and the batch backend chain (OpenAI / local) is used instead.
pub type TokenProvider = std::sync::Arc<
    dyn Fn() -> std::pin::Pin<Box<dyn std::future::Future<Output = Option<String>> + Send>>
        + Send
        + Sync,
>;

/// dBFS window mapped to the level meter's [0,1]. Real mics vary ~10× in gain,
/// so a linear `rms/32768` map collapses normal speech to ~0.01 (this machine's
/// speech RMS peaks near ~350 — a few hundredths of full scale) and the meter
/// never lights up. A log (dBFS) scale matches how loudness is perceived and is
/// tolerant of mic gain: `LEVEL_DB_FLOOR` reads as silence, `LEVEL_DB_CEIL` as
/// full. The window (-55..-18 dBFS) brackets quiet-to-loud speech on this class
/// of microphone.
const LEVEL_DB_FLOOR: f32 = -55.0;
const LEVEL_DB_CEIL: f32 = -18.0;

/// Normalize a raw RMS energy (i16 sample units, ~0..32767) to a perceptual
/// [0,1] level for the recording-cursor animation, via a dBFS window. Silence
/// (and `rms == 0`) maps to 0; loud speech approaches 1.
pub fn normalize_level(rms: u32) -> f32 {
    if rms == 0 {
        return 0.0;
    }
    let dbfs = 20.0 * (rms as f32 / 32768.0).log10();
    ((dbfs - LEVEL_DB_FLOOR) / (LEVEL_DB_CEIL - LEVEL_DB_FLOOR)).clamp(0.0, 1.0)
}

/// The voice recorder — manages the capture+STT pipeline.
pub struct VoiceRecorder {
    cfg: VoiceConfig,
    state: Arc<Mutex<VoiceState>>,
    /// Stop signal for the recording task.
    stop_tx: Option<tokio::sync::oneshot::Sender<()>>,
    /// Stop signal for the VAD listen loop (VAD mode only).
    vad_stop_tx: Option<tokio::sync::oneshot::Sender<()>>,
    /// Per-utterance force-end command for the VAD listen loop. Unlike
    /// `vad_stop_tx`, this keeps the loop alive and only tells the active
    /// detector to finish the current utterance.
    vad_force_end_tx: Option<mpsc::UnboundedSender<()>>,
    /// Discard flag shared with the active recording task. `stop()` (finish)
    /// leaves it false → the task finalizes and emits a `Final`; `cancel()`
    /// (discard) sets it true → the task drops the utterance with NO `Final`.
    /// Without this distinction, cancelling an in-flight recording (e.g. the
    /// user presses Enter to submit, then stops voice) still emitted a `Final`
    /// that auto-submitted a duplicate.
    cancel_flag: Arc<std::sync::atomic::AtomicBool>,
    /// Resolves the OAuth token for the live Anthropic voice stream. `None`
    /// disables the live path (batch backends still work).
    token_provider: Option<TokenProvider>,
    /// Output channel for transcript events.
    pub events: mpsc::UnboundedSender<VoiceTranscriptEvent>,
}

impl VoiceRecorder {
    pub fn new(cfg: VoiceConfig, events: mpsc::UnboundedSender<VoiceTranscriptEvent>) -> Self {
        Self {
            cfg,
            state: Arc::new(Mutex::new(VoiceState::Idle)),
            stop_tx: None,
            vad_stop_tx: None,
            vad_force_end_tx: None,
            cancel_flag: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            token_provider: None,
            events,
        }
    }

    /// Attach an OAuth token resolver, enabling the live Anthropic voice stream.
    pub fn with_token_provider(mut self, provider: TokenProvider) -> Self {
        self.token_provider = Some(provider);
        self
    }

    pub fn reconfigure(&mut self, cfg: VoiceConfig) {
        if !cfg.enabled || cfg.mode != VoiceMode::Vad {
            if let Some(tx) = self.vad_stop_tx.take() {
                let _ = tx.send(());
            }
            self.vad_force_end_tx = None;
        }
        self.cfg = cfg;
        self.cancel_flag
            .store(false, std::sync::atomic::Ordering::SeqCst);
    }

    /// Resolve the current OAuth token via the provider, falling back to the
    /// legacy env vars so manual setups keep working.
    async fn resolve_token(&self) -> Option<String> {
        if let Some(p) = &self.token_provider {
            if let Some(tok) = p().await {
                return Some(tok);
            }
        }
        std::env::var("CLAUDE_ACCESS_TOKEN")
            .or_else(|_| std::env::var("ANTHROPIC_ACCESS_TOKEN"))
            .or_else(|_| std::env::var("JFC_ANTHROPIC_ACCESS_TOKEN"))
            .ok()
    }

    /// Start the VAD listen loop (VAD mode only).
    /// The loop runs continuously until `cancel()` is called.
    pub async fn start_vad_loop(&mut self) {
        self.clear_stale_vad_loop_handles();
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
        let (vad_force_end_tx, vad_force_end_rx) = mpsc::unbounded_channel::<()>();
        self.vad_stop_tx = Some(vad_stop_tx);
        self.vad_force_end_tx = Some(vad_force_end_tx);
        self.cancel_flag
            .store(false, std::sync::atomic::Ordering::SeqCst);

        let cfg = self.cfg.clone();
        let events = self.events.clone();
        let state = Arc::clone(&self.state);
        let cancel_flag = Arc::clone(&self.cancel_flag);
        let token_provider = self.token_provider.clone();
        tokio::spawn(async move {
            vad_listen_loop(
                backend,
                cfg,
                events,
                state,
                vad_stop_rx,
                vad_force_end_rx,
                cancel_flag,
                token_provider,
            )
            .await;
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
            (VoiceMode::Vad, true, VoiceState::Recording) => self.force_end_vad_utterance(),
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

        // Resolve the OAuth token up front so the streaming session can use the
        // live Anthropic voice stream when signed in.
        let token = self.resolve_token().await;
        let cfg = self.cfg.clone();
        let events = self.events.clone();
        let state = Arc::clone(&self.state);
        let (stop_tx, stop_rx) = tokio::sync::oneshot::channel::<()>();
        self.stop_tx = Some(stop_tx);
        // Fresh session — clear any stale discard flag from a prior recording.
        self.cancel_flag
            .store(false, std::sync::atomic::Ordering::SeqCst);
        let cancel_flag = Arc::clone(&self.cancel_flag);

        // The streaming pipeline owns the whole lifecycle from here: capture →
        // live STT (or batch fallback) → finalize → Final emission → Idle.
        tokio::spawn(async move {
            crate::stream_record::run(backend, cfg, token, events, state, stop_rx, cancel_flag)
                .await;
        });
    }

    async fn stop_recording(&mut self) {
        info!(target: "jfc::voice", "stop_recording");
        // Signal the streaming task to finish; it transitions Recording →
        // Processing → Idle and emits the Final transcript itself.
        if let Some(tx) = self.stop_tx.take() {
            if tx.send(()).is_err() {
                debug!(target: "jfc::voice", "stop signal had no receiver (task already finished)");
            }
        }
    }

    fn force_end_vad_utterance(&mut self) {
        self.clear_stale_vad_loop_handles();
        let Some(tx) = self.vad_force_end_tx.as_ref() else {
            debug!(
                target: "jfc::voice::vad",
                "force-end requested but no VAD control channel is active"
            );
            return;
        };
        if tx.send(()).is_err() {
            debug!(
                target: "jfc::voice::vad",
                "force-end command had no receiver (VAD loop already exited)"
            );
            self.vad_force_end_tx = None;
            if self
                .vad_stop_tx
                .as_ref()
                .is_some_and(tokio::sync::oneshot::Sender::is_closed)
            {
                self.vad_stop_tx = None;
            }
        }
    }

    /// Discard an in-flight hold/tap recording WITHOUT finalizing — the task
    /// drops the utterance and emits no `Final`. No-op when nothing is recording
    /// or in VAD mode (which has no per-key recording task, so the continuous
    /// listen loop is left running). Used on a manual submit (Enter) so voice
    /// doesn't auto-submit a duplicate of what the user just sent.
    pub async fn discard_recording(&mut self) {
        if let Some(tx) = self.stop_tx.take() {
            info!(target: "jfc::voice", "discard_recording (manual submit)");
            self.cancel_flag
                .store(true, std::sync::atomic::Ordering::SeqCst);
            let _ = tx.send(());
            self.set_state(VoiceState::Idle).await;
        }
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
        // Mark discard BEFORE signalling stop so the recording task sees it and
        // drops the utterance instead of finalizing + emitting a `Final` (which
        // would auto-submit a duplicate after a manual Enter submit).
        self.cancel_flag
            .store(true, std::sync::atomic::Ordering::SeqCst);
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
        self.vad_force_end_tx = None;
        self.set_state(VoiceState::Idle).await;
    }

    /// Whether the VAD listen loop is currently running. Used by tests and the
    /// UI to reflect mic-hot state accurately.
    pub fn vad_loop_running(&self) -> bool {
        self.vad_stop_tx.as_ref().is_some_and(|tx| !tx.is_closed())
    }

    fn clear_stale_vad_loop_handles(&mut self) {
        if self
            .vad_stop_tx
            .as_ref()
            .is_some_and(tokio::sync::oneshot::Sender::is_closed)
        {
            self.vad_stop_tx = None;
            self.vad_force_end_tx = None;
            return;
        }
        if self
            .vad_force_end_tx
            .as_ref()
            .is_some_and(mpsc::UnboundedSender::is_closed)
        {
            self.vad_force_end_tx = None;
        }
    }
}

#[inline]
pub(crate) fn send_or_debug(
    tx: &mpsc::UnboundedSender<VoiceTranscriptEvent>,
    ev: VoiceTranscriptEvent,
) {
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
    mut force_end_rx: mpsc::UnboundedReceiver<()>,
    cancel_flag: Arc<std::sync::atomic::AtomicBool>,
    token_provider: Option<TokenProvider>,
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
                Some(()) = force_end_rx.recv() => {
                    debug!(
                        target: "jfc::voice::vad",
                        "force-end ignored while VAD is waiting for speech"
                    );
                }
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
                Some(()) = force_end_rx.recv() => {
                    let detector_was_speaking = detector.force_end();
                    debug!(
                        target: "jfc::voice::vad",
                        detector_was_speaking,
                        frames = frames_seen,
                        "force-ending active VAD utterance"
                    );
                    break true;
                }
                n = capture.read_chunk(&mut chunk) => match n {
                    Ok(0) | Err(_) => break true,
                    Ok(n) => {
                        let frame = &chunk[..n];
                        utterance_buf.extend_from_slice(frame);
                        frames_seen += 1;
                        let rms = crate::vad::rms_energy(frame);
                        max_rms = max_rms.max(rms);
                        // Feed the recording-cursor animation with the live level.
                        send_or_debug(&events, VoiceTranscriptEvent::Level(normalize_level(rms)));
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

        // Discard on cancel (`/voice off` / Esc): if the loop was stopped via
        // cancel rather than ending naturally, drop the utterance without
        // transcribing or emitting a Final.
        if cancel_flag.load(std::sync::atomic::Ordering::SeqCst) {
            *state.lock().await = VoiceState::Idle;
            send_or_debug(
                &events,
                VoiceTranscriptEvent::StateChanged(VoiceState::Idle),
            );
            break;
        }

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
        let token = match token_provider.as_ref() {
            Some(provider) => provider().await,
            None => None,
        };
        let result = backends::transcribe_with_token(&pcm, &cfg, token.as_deref()).await;

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

    // REGRESSION (recording cursor was always gray + min-bar): a linear
    // rms/32768 map collapses this mic's speech (RMS ~350) to ~0.01, far below
    // the meter's gray threshold and bar range. The dBFS window must map silence
    // → ~0, real speech → a clearly visible mid level, and loud input → ~1.
    #[test]
    fn normalize_level_dbfs_window_normal() {
        assert_eq!(normalize_level(0), 0.0);
        // Near-silent room noise stays near zero (below the gray threshold).
        assert!(
            normalize_level(30) < 0.05,
            "silence = {}",
            normalize_level(30)
        );
        // This machine's measured speech RMS (~350) lands in a visible,
        // colorable mid range (well above the 0.10 gray threshold).
        let speech = normalize_level(350);
        assert!(
            (0.2..0.8).contains(&speech),
            "speech level should be a visible mid value, got {speech}"
        );
        // Loud input saturates near full.
        assert!(normalize_level(5000) > 0.95);
        // Monotonic in RMS.
        assert!(normalize_level(350) < normalize_level(1500));
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

    #[tokio::test]
    async fn reconfigure_updates_mode_and_stops_stale_vad_loop_regression() {
        let (tx, _rx) = mpsc::unbounded_channel();
        let mut rec = VoiceRecorder::new(
            VoiceConfig {
                enabled: true,
                mode: VoiceMode::Vad,
                ..Default::default()
            },
            tx,
        );
        let (vad_stop_tx, vad_stop_rx) = tokio::sync::oneshot::channel();
        let (force_tx, _force_rx) = mpsc::unbounded_channel();
        rec.vad_stop_tx = Some(vad_stop_tx);
        rec.vad_force_end_tx = Some(force_tx);

        rec.reconfigure(VoiceConfig {
            enabled: true,
            mode: VoiceMode::Tap,
            ..Default::default()
        });

        assert_eq!(rec.cfg.mode, VoiceMode::Tap);
        assert!(rec.vad_stop_tx.is_none());
        assert!(rec.vad_force_end_tx.is_none());
        assert_eq!(vad_stop_rx.await, Ok(()));
    }

    #[tokio::test]
    async fn vad_space_force_ends_active_utterance_not_loop_regression() {
        let (tx, _rx) = mpsc::unbounded_channel();
        let mut rec = VoiceRecorder::new(
            VoiceConfig {
                mode: VoiceMode::Vad,
                ..Default::default()
            },
            tx,
        );
        let (hold_stop_tx, mut hold_stop_rx) = tokio::sync::oneshot::channel::<()>();
        let (vad_stop_tx, mut vad_stop_rx) = tokio::sync::oneshot::channel::<()>();
        let (force_tx, mut force_rx) = mpsc::unbounded_channel::<()>();
        rec.stop_tx = Some(hold_stop_tx);
        rec.vad_stop_tx = Some(vad_stop_tx);
        rec.vad_force_end_tx = Some(force_tx);
        rec.set_state(VoiceState::Recording).await;

        rec.activate(true).await;

        assert_eq!(
            force_rx.try_recv(),
            Ok(()),
            "Space in VAD recording state should force-end the utterance"
        );
        assert!(
            matches!(
                hold_stop_rx.try_recv(),
                Err(tokio::sync::oneshot::error::TryRecvError::Empty)
            ),
            "VAD force-end must not signal the hold/tap recording channel"
        );
        assert!(
            matches!(
                vad_stop_rx.try_recv(),
                Err(tokio::sync::oneshot::error::TryRecvError::Empty)
            ),
            "VAD force-end must not stop the continuous listen loop"
        );
        assert!(rec.stop_tx.is_some());
        assert!(rec.vad_loop_running());
    }

    #[tokio::test]
    async fn stale_vad_loop_handle_is_cleared_before_restart_regression() {
        let (tx, _rx) = mpsc::unbounded_channel();
        let mut rec = VoiceRecorder::new(
            VoiceConfig {
                mode: VoiceMode::Vad,
                ..Default::default()
            },
            tx,
        );
        let (vad_stop_tx, vad_stop_rx) = tokio::sync::oneshot::channel::<()>();
        let (force_tx, force_rx) = mpsc::unbounded_channel::<()>();
        drop(vad_stop_rx);
        drop(force_rx);
        rec.vad_stop_tx = Some(vad_stop_tx);
        rec.vad_force_end_tx = Some(force_tx);

        assert!(
            !rec.vad_loop_running(),
            "closed stop receiver must not count as a running VAD loop"
        );
        rec.clear_stale_vad_loop_handles();

        assert!(rec.vad_stop_tx.is_none());
        assert!(rec.vad_force_end_tx.is_none());
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
        // SAFETY: this test mutates one process env var and restores it before
        // returning; no code under test spawns threads that read it concurrently.
        unsafe { std::env::set_var(KEY, "120000") };
        let resolved = max_utterance_cap_ms();
        // SAFETY: restore the same test-owned env var to its prior value.
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
        // With the feature on, the neural engine loads the bundled Silero model —
        // but only when the `JFC_VAD_ENABLE_NEURAL=1` runtime opt-in is set, since
        // ONNX native init is gated off by default. Honor that gate (rather than
        // mutating shared process env, which would race the fallback test) so the
        // assertion is correct in both CI (flag off ⇒ energy) and opt-in builds.
        let cfg = VoiceConfig {
            vad_engine: VadEngine::Neural,
            ..Default::default()
        };
        let detector = VadDetector::select(&cfg);
        if VadEngine::neural_runtime_enabled() {
            assert!(matches!(detector, VadDetector::Neural(_)));
        } else {
            assert!(matches!(detector, VadDetector::Energy(_)));
        }
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

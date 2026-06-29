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
use crate::config::{SttBackendKind, VadEngine, VoiceConfig, VoiceMode};
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
        let _linkscope_select = linkscope::phase("voice.vad_detector.select");
        linkscope::event_fields(
            "voice.vad_detector.select",
            [linkscope::TraceField::text(
                "engine",
                format!("{:?}", cfg.vad_engine),
            )],
        );
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
        let _linkscope_push = linkscope::phase("voice.vad_detector.push");
        match self {
            Self::Energy(v) => v.push(pcm),
            #[cfg(feature = "vad-neural")]
            Self::Neural(v) => v.push(pcm),
        }
    }

    fn reset(&mut self) {
        let _linkscope_reset = linkscope::phase("voice.vad_detector.reset");
        match self {
            Self::Energy(v) => v.reset(),
            #[cfg(feature = "vad-neural")]
            Self::Neural(v) => v.reset(),
        }
    }

    fn force_end(&mut self) -> bool {
        let _linkscope_force = linkscope::phase("voice.vad_detector.force_end");
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
    /// Our speakers — admit an utterance matching ANY of these. Empty ⇒ admit
    /// anything not in `reject` (self-rejection-only mode).
    accept: Vec<crate::speaker::SpeakerProfile>,
    /// Voices to drop even if a user is enrolled — the assistant's own TTS
    /// voice(s). The reject-list always wins over the accept-list, so self-echo
    /// leaking past the time-based echo guard is still suppressed acoustically.
    reject: Vec<crate::speaker::SpeakerProfile>,
    /// Embedding backend used for scoring. The neural ONNX backend (feature
    /// `speaker-neural` + `JFC_VOICE_SPEAKER_MODEL`) when available, else the
    /// null embedder (classical Mahalanobis+pitch path).
    embedder: Box<dyn crate::speaker::SpeakerEmbedder>,
}

impl SpeakerGate {
    /// Load the gate from config: returns an inert gate unless the gate is
    /// enabled AND at least one accept/reject profile loads. Accept profiles
    /// come from the legacy single `speaker_profile.json` PLUS every profile in
    /// the `speakers/` dir; reject profiles from the `reject/` dir.
    fn from_config(cfg: &VoiceConfig) -> Self {
        let _linkscope_gate = linkscope::phase("voice.speaker_gate.from_config");
        let inert = || Self {
            accept: Vec::new(),
            reject: Vec::new(),
            embedder: Box::new(crate::speaker::NullEmbedder),
        };
        if !cfg.speaker_gate {
            linkscope::event_fields(
                "voice.speaker_gate.from_config",
                [linkscope::TraceField::count("enabled", 0)],
            );
            return inert();
        }
        let speaker_model_path = cfg.speaker_model_path.as_ref().map(std::path::Path::new);
        let embedder = crate::speaker::default_embedder(speaker_model_path);
        let dir = Self::profile_dir(cfg);

        // Accept-list: legacy single file (back-compat) + the speakers/ dir.
        let mut accept = Vec::new();
        if let Ok(profile) = crate::speaker::SpeakerProfile::load(&Self::profile_path(cfg)) {
            accept.push(profile);
        }
        accept.extend(crate::speaker::load_profiles_dir(&dir.join("speakers")));
        // Reject-list: the reject/ dir (the assistant's own TTS voiceprints).
        let reject = crate::speaker::load_profiles_dir(&dir.join("reject"));

        // Apply the optional threshold override to every accept profile.
        if let Some(t) = cfg.speaker_threshold {
            for p in &mut accept {
                *p = p.clone().with_threshold(t);
            }
        }

        if accept.is_empty() && reject.is_empty() {
            warn!(
                target: "jfc::voice::speaker",
                dir = %dir.display(),
                "speaker gate enabled but no accept/reject profiles found; gate inert \
                 (enroll a speaker to filter background voices)"
            );
            return inert();
        }
        linkscope::event_fields(
            "voice.speaker_gate.from_config",
            [
                linkscope::TraceField::count("enabled", 1),
                linkscope::TraceField::count(
                    "accept",
                    u64::try_from(accept.len()).unwrap_or(u64::MAX),
                ),
                linkscope::TraceField::count(
                    "reject",
                    u64::try_from(reject.len()).unwrap_or(u64::MAX),
                ),
                linkscope::TraceField::text("backend", embedder.name().to_owned()),
            ],
        );
        info!(
            target: "jfc::voice::speaker",
            accept = accept.len(),
            reject = reject.len(),
            backend = embedder.name(),
            "speaker gate enabled"
        );
        Self {
            accept,
            reject,
            embedder,
        }
    }

    /// Resolve the legacy single-profile path: explicit config/env, else
    /// `<config>/jfc/voice/speaker_profile.json` under the user config dir.
    fn profile_path(cfg: &VoiceConfig) -> std::path::PathBuf {
        if let Some(p) = &cfg.speaker_profile_path {
            return std::path::PathBuf::from(p);
        }
        Self::profile_dir(cfg).join("speaker_profile.json")
    }

    /// The base directory holding `speakers/` (accept) and `reject/` profile
    /// dirs: the parent of the configured single-profile path, else the default
    /// `<config>/jfc/voice` dir.
    fn profile_dir(cfg: &VoiceConfig) -> std::path::PathBuf {
        if let Some(p) = &cfg.speaker_profile_path
            && let Some(parent) = std::path::Path::new(p).parent()
            && !parent.as_os_str().is_empty()
        {
            return parent.to_path_buf();
        }
        let base = dirs_config_dir().unwrap_or_else(|| std::path::PathBuf::from("."));
        base.join("jfc").join("voice")
    }

    /// Whether the gate will actually filter anything (has any profile loaded).
    fn is_active(&self) -> bool {
        !self.accept.is_empty() || !self.reject.is_empty()
    }

    /// Classify a captured utterance against the accept/reject sets.
    fn decide(&self, pcm: &[u8]) -> crate::speaker::AdmitDecision {
        crate::speaker::verify_admit(&self.accept, &self.reject, pcm, self.embedder.as_ref())
    }

    /// Decide whether a captured utterance should be transcribed. Returns `true`
    /// (transcribe) when the gate is inert or the utterance matches one of our
    /// speakers; `false` to drop the assistant's own voice or a background voice.
    fn admits(&self, pcm: &[u8]) -> bool {
        let _linkscope_admits = linkscope::phase("voice.speaker_gate.admits");
        let decision = self.decide(pcm);
        if !decision.admitted() {
            info!(
                target: "jfc::voice::speaker",
                ?decision,
                "dropped an utterance (not one of our speakers / own TTS voice)"
            );
        }
        linkscope::event_fields(
            "voice.speaker_gate.admits.result",
            [
                linkscope::TraceField::count("admitted", u64::from(decision.admitted())),
                linkscope::TraceField::bytes(
                    "pcm_bytes",
                    u64::try_from(pcm.len()).unwrap_or(u64::MAX),
                ),
            ],
        );
        decision.admitted()
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

/// Filesystem-safe profile stem: `"Alice 2"` → `"alice_2"`. Non-alphanumerics
/// collapse to `_`; an empty result falls back to `"speaker"`.
fn sanitize_profile_name(name: &str) -> String {
    let mapped: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect();
    let trimmed = mapped.trim_matches('_').to_owned();
    if trimmed.is_empty() {
        "speaker".to_owned()
    } else {
        trimmed
    }
}

/// Capture ~`secs` seconds of microphone PCM (16 kHz mono S16LE).
async fn capture_pcm(secs: f64) -> Result<Vec<u8>, String> {
    let _linkscope_capture = linkscope::phase("voice.capture_pcm");
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
    linkscope::record_bytes(
        "voice.capture_pcm.bytes",
        u64::try_from(pcm.len()).unwrap_or(u64::MAX),
    );
    Ok(pcm)
}

/// Build a [`crate::speaker::SpeakerProfile`] from `pcm` (attaching a neural
/// embedding when a model is configured) and save it to `path`.
fn build_and_save_profile(
    cfg: &VoiceConfig,
    pcm: &[u8],
    path: &std::path::Path,
) -> Result<crate::speaker::SpeakerProfile, String> {
    let mut profile = crate::speaker::SpeakerProfile::enroll_from_pcm(pcm).ok_or_else(|| {
        "not enough voiced speech to enroll — speak continuously for a few seconds".to_owned()
    })?;
    let speaker_model_path = cfg.speaker_model_path.as_ref().map(std::path::Path::new);
    let embedder = crate::speaker::default_embedder(speaker_model_path);
    profile = profile.with_neural_embedding(embedder.as_ref(), pcm);
    profile.save(path).map_err(|e| e.to_string())?;
    Ok(profile)
}

/// Enroll the primary speaker by capturing ~`secs` seconds of microphone audio
/// and writing a [`crate::speaker::SpeakerProfile`] to `cfg`'s legacy
/// single-profile path. Returns the path written. Speak only yourself during
/// enrollment.
pub async fn enroll_primary_speaker(
    cfg: &VoiceConfig,
    secs: f64,
) -> Result<std::path::PathBuf, String> {
    let _linkscope_enroll = linkscope::phase("voice.enroll_primary_speaker");
    let pcm = capture_pcm(secs).await?;
    let path = SpeakerGate::profile_path(cfg);
    let profile = build_and_save_profile(cfg, &pcm, &path)?;
    info!(
        target: "jfc::voice::speaker",
        path = %path.display(),
        frames = profile.enrolled_frames,
        threshold = profile.threshold,
        neural = profile.neural.is_some(),
        "enrolled primary speaker profile"
    );
    Ok(path)
}

/// Enroll an additional **named** speaker into the accept-list ("our speakers"),
/// writing `<profile_dir>/speakers/<name>.json`. The gate admits an utterance
/// matching ANY enrolled speaker, so this is how you add a teammate.
pub async fn enroll_speaker(
    cfg: &VoiceConfig,
    name: &str,
    secs: f64,
) -> Result<std::path::PathBuf, String> {
    let _linkscope_enroll = linkscope::phase("voice.enroll_speaker");
    linkscope::event_fields(
        "voice.enroll_speaker",
        [linkscope::TraceField::text("name", name.to_owned())],
    );
    let pcm = capture_pcm(secs).await?;
    let path = SpeakerGate::profile_dir(cfg)
        .join("speakers")
        .join(format!("{}.json", sanitize_profile_name(name)));
    let profile = build_and_save_profile(cfg, &pcm, &path)?;
    info!(
        target: "jfc::voice::speaker",
        path = %path.display(),
        name,
        frames = profile.enrolled_frames,
        "enrolled speaker into accept-list",
    );
    Ok(path)
}

/// The on-disk path of the reject-profile for `key` (a TTS voice name). Used to
/// check whether self-voice enrollment has already happened before re-doing it.
pub fn reject_profile_path(cfg: &VoiceConfig, key: &str) -> std::path::PathBuf {
    SpeakerGate::profile_dir(cfg)
        .join("reject")
        .join(format!("{}.json", sanitize_profile_name(key)))
}

/// Save a **reject**-profile — the assistant's own TTS voice — from PCM we
/// captured of read-aloud playback (we control the TTS audio). Stored under
/// `<profile_dir>/reject/<key>.json`, keyed by the TTS voice name so each
/// Anthropic voice gets its own reject voiceprint. The gate then drops any
/// utterance matching it, acoustically — covering self-echo that leaks past the
/// time-based echo guard or with `echo_suppression=false`.
pub fn save_reject_profile_from_pcm(
    cfg: &VoiceConfig,
    key: &str,
    pcm: &[u8],
) -> Result<std::path::PathBuf, String> {
    let _linkscope_save = linkscope::phase("voice.save_reject_profile");
    linkscope::event_fields(
        "voice.save_reject_profile",
        [
            linkscope::TraceField::text("key", key.to_owned()),
            linkscope::TraceField::bytes("pcm_bytes", u64::try_from(pcm.len()).unwrap_or(u64::MAX)),
        ],
    );
    let path = reject_profile_path(cfg, key);
    build_and_save_profile(cfg, pcm, &path)?;
    info!(
        target: "jfc::voice::speaker",
        path = %path.display(),
        key,
        "saved self-voice reject profile",
    );
    Ok(path)
}

/// Enroll the assistant's OWN voice as a reject-profile by synthesizing a known
/// phrase through the TTS backend and voiceprinting the resulting PCM. We
/// control the TTS audio, so this is a clean reference signal. Forces 16 kHz PCM
/// output (what the voiceprint expects) regardless of the configured playback
/// format, and saves under `reject/<tts_voice>.json`. Requires an OAuth token.
pub async fn enroll_self_voice(
    cfg: &VoiceConfig,
    token: &str,
    user_agent: &str,
) -> Result<std::path::PathBuf, String> {
    let _linkscope_enroll = linkscope::phase("voice.enroll_self_voice");
    // Two sentences → a few seconds of voiced audio (enroll needs ~0.5 s voiced).
    const PHRASE: &str = "The quick brown fox jumps over the lazy dog. \
        I am the assistant; this is the sound of my own voice, so I can recognize \
        and ignore it when it echoes back into the microphone.";
    // The voiceprint expects 16 kHz mono S16LE PCM; force it for enrollment even
    // if read-aloud is configured to play a compressed format.
    let mut pcm_cfg = cfg.clone();
    pcm_cfg.tts_output_format = "pcm_16000".to_owned();
    let mut pcm: Vec<u8> = Vec::new();
    crate::tts::synthesize_to_writer(&pcm_cfg, token, user_agent, PHRASE, &mut pcm)
        .await
        .map_err(|e| e.to_string())?;
    if pcm.is_empty() {
        return Err("TTS returned no audio to enroll the self-voice profile".to_owned());
    }
    save_reject_profile_from_pcm(cfg, &cfg.tts_voice, &pcm)
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
    AssistantMessageStarted,
    AssistantTextDelta(String),
    AssistantMessageCompleted,
    ReadAloudStarted {
        chars: usize,
    },
    ReadAloudCompleted {
        audio_bytes: usize,
        chunks_sent: usize,
    },
    ReadAloudError(String),
    TtsWord {
        text: String,
        pts_ms: u64,
    },
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
    /// Discard the in-flight VAD utterance WITHOUT emitting a `Final`, while
    /// leaving the listen loop running. Unlike `cancel_flag` (which ends the
    /// whole loop), this drops just the current utterance — used when the user
    /// presses Enter mid-utterance so a late server endpoint doesn't re-hydrate
    /// the box or auto-submit a duplicate of what was just sent.
    vad_discard: Arc<std::sync::atomic::AtomicBool>,
    /// Resolves the OAuth token for the live Anthropic voice stream. `None`
    /// disables the live path (batch backends still work).
    token_provider: Option<TokenProvider>,
    /// Output channel for transcript events.
    pub events: mpsc::UnboundedSender<VoiceTranscriptEvent>,
}

impl VoiceRecorder {
    pub fn new(cfg: VoiceConfig, events: mpsc::UnboundedSender<VoiceTranscriptEvent>) -> Self {
        let _linkscope_recorder = linkscope::phase("voice.recorder.new");
        linkscope::event_fields(
            "voice.recorder.new",
            [
                linkscope::TraceField::count("enabled", u64::from(cfg.enabled)),
                linkscope::TraceField::text("mode", format!("{:?}", cfg.mode)),
                linkscope::TraceField::text("backend", format!("{:?}", cfg.effective_backend())),
            ],
        );
        Self {
            cfg,
            state: Arc::new(Mutex::new(VoiceState::Idle)),
            stop_tx: None,
            vad_stop_tx: None,
            vad_force_end_tx: None,
            cancel_flag: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            vad_discard: Arc::new(std::sync::atomic::AtomicBool::new(false)),
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
        let _linkscope_reconfigure = linkscope::phase("voice.recorder.reconfigure");
        linkscope::event_fields(
            "voice.recorder.reconfigure",
            [
                linkscope::TraceField::count("enabled", u64::from(cfg.enabled)),
                linkscope::TraceField::text("mode", format!("{:?}", cfg.mode)),
            ],
        );
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

    async fn resolve_token(&self) -> Option<String> {
        let _linkscope_token = linkscope::phase("voice.recorder.resolve_token");
        let provider = self.token_provider.as_ref()?;
        provider().await
    }

    /// Start the VAD listen loop (VAD mode only).
    /// The loop runs continuously until `cancel()` is called.
    pub async fn start_vad_loop(&mut self) {
        let _linkscope_start = linkscope::phase("voice.recorder.start_vad_loop");
        self.clear_stale_vad_loop_handles();
        if self.vad_stop_tx.is_some() {
            linkscope::event_fields(
                "voice.recorder.start_vad_loop.result",
                [linkscope::TraceField::text("status", "already_running")],
            );
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
        linkscope::event_fields(
            "voice.recorder.start_vad_loop.result",
            [
                linkscope::TraceField::text("status", "starting"),
                linkscope::TraceField::text("backend", backend.label().to_owned()),
            ],
        );
        info!(target: "jfc::voice", backend = %backend.label(), "starting VAD listen loop");
        let (vad_stop_tx, vad_stop_rx) = tokio::sync::oneshot::channel::<()>();
        let (vad_force_end_tx, vad_force_end_rx) = mpsc::unbounded_channel::<()>();
        self.vad_stop_tx = Some(vad_stop_tx);
        self.vad_force_end_tx = Some(vad_force_end_tx);
        self.cancel_flag
            .store(false, std::sync::atomic::Ordering::SeqCst);
        self.vad_discard
            .store(false, std::sync::atomic::Ordering::SeqCst);

        let cfg = self.cfg.clone();
        let events = self.events.clone();
        let state = Arc::clone(&self.state);
        let cancel_flag = Arc::clone(&self.cancel_flag);
        let vad_discard = Arc::clone(&self.vad_discard);
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
                vad_discard,
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
        let _linkscope_activate = linkscope::phase("voice.recorder.activate");
        let state = *self.state.lock().await;
        linkscope::event_fields(
            "voice.recorder.activate",
            [
                linkscope::TraceField::text("mode", format!("{:?}", self.cfg.mode)),
                linkscope::TraceField::count("pressed", u64::from(pressed)),
                linkscope::TraceField::text("state", format!("{state:?}")),
            ],
        );
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
        let _linkscope_start = linkscope::phase("voice.recorder.start_recording");
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
        linkscope::event_fields(
            "voice.recorder.start_recording.backend",
            [linkscope::TraceField::text(
                "backend",
                backend.label().to_owned(),
            )],
        );

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
        let _linkscope_stop = linkscope::phase("voice.recorder.stop_recording");
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
        let _linkscope_force = linkscope::phase("voice.recorder.force_end_vad_utterance");
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
        let _linkscope_discard = linkscope::phase("voice.recorder.discard_recording");
        if let Some(tx) = self.stop_tx.take() {
            info!(target: "jfc::voice", "discard_recording (manual submit)");
            self.cancel_flag
                .store(true, std::sync::atomic::Ordering::SeqCst);
            let _ = tx.send(());
            self.set_state(VoiceState::Idle).await;
            return;
        }
        // VAD mode has no per-key task, but an utterance may be mid-capture OR
        // already finalizing (Processing). Drop just that utterance (no `Final`)
        // and keep the loop listening, so a manual Enter doesn't later re-hydrate
        // the box / auto-submit a duplicate. Covering Processing matters: when
        // Enter lands AFTER speech end (server endpoint / SpeechEnd) but during
        // transcription, the state is already Processing — a `Recording`-only
        // check missed it and the in-flight Final still auto-submitted.
        if self.cfg.mode == VoiceMode::Vad && *self.state.lock().await != VoiceState::Idle {
            info!(target: "jfc::voice::vad", "discard_recording (manual submit, VAD utterance)");
            self.vad_discard
                .store(true, std::sync::atomic::Ordering::SeqCst);
            self.force_end_vad_utterance();
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
        let _linkscope_state = linkscope::phase("voice.recorder.set_state");
        linkscope::event_fields(
            "voice.recorder.state",
            [linkscope::TraceField::text("state", format!("{s:?}"))],
        );
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
        let _linkscope_cancel = linkscope::phase("voice.recorder.cancel");
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
        let _linkscope_clear = linkscope::phase("voice.recorder.clear_stale_vad_loop_handles");
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

/// Onset window (bytes of 16 kHz mono i16 PCM) captured un-streamed for the
/// speaker pre-gate: long enough to hold a few voiced frames for a confident
/// accept/reject decision, short enough that the added latency before live
/// interims is barely perceptible. Default 600 ms; override via
/// `JFC_VOICE_PREGATE_MS` (clamped to a sane 200 ms..=2000 ms range).
fn pregate_window_bytes() -> usize {
    const DEFAULT_PREGATE_MS: u64 = 600;
    let ms = std::env::var("JFC_VOICE_PREGATE_MS")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(DEFAULT_PREGATE_MS)
        .clamp(200, 2_000);
    pregate_window_bytes_for_ms(ms)
}

/// Bytes of 16 kHz mono i16 PCM spanning `ms` milliseconds.
fn pregate_window_bytes_for_ms(ms: u64) -> usize {
    (ms as usize) * 16_000 * 2 / 1000
}

/// Half-duplex echo-guard decision for one captured frame while VAD is waiting
/// for speech. Returns `true` to suppress the frame (don't let it start an
/// utterance) while read-aloud is `playing`, and for `tail` afterwards so the
/// speaker's acoustic decay doesn't trip the detector. `tail_until` is advanced
/// while playing; pure given its inputs (no global reads) so it's unit-testable.
fn echo_guard(
    enabled: bool,
    playing: bool,
    now: std::time::Instant,
    tail_until: &mut Option<std::time::Instant>,
    tail: std::time::Duration,
) -> bool {
    if !enabled {
        return false;
    }
    if playing {
        *tail_until = Some(now + tail);
        return true;
    }
    tail_until.is_some_and(|until| now < until)
}

/// VAD continuous-listen loop.
///
/// Streams audio indefinitely, running it through the VAD energy detector.
/// When speech is detected, buffers PCM until silence, then transcribes and
/// emits the result. Loops back to listening after each utterance.
/// The server's authoritative end-of-turn signal: a promoted/endpointed
/// transcript (`TranscriptEndpoint` → an `is_final` fragment). This — Deepgram
/// server-side endpointing, exactly what Claude Code relies on — is what ends a
/// VAD turn, NOT the client energy/neural VAD (which only detects onset/level
/// reliably and is kept solely as a batch-path fallback).
fn server_endpointed_msg(msg: &crate::anthropic_ws::StreamMsg) -> bool {
    matches!(
        msg,
        crate::anthropic_ws::StreamMsg::Transcript { is_final: true, .. }
    )
}

#[allow(clippy::too_many_arguments)]
async fn vad_listen_loop(
    backend: CaptureBackend,
    cfg: VoiceConfig,
    events: mpsc::UnboundedSender<VoiceTranscriptEvent>,
    state: Arc<Mutex<VoiceState>>,
    stop_rx: tokio::sync::oneshot::Receiver<()>,
    mut force_end_rx: mpsc::UnboundedReceiver<()>,
    cancel_flag: Arc<std::sync::atomic::AtomicBool>,
    vad_discard: Arc<std::sync::atomic::AtomicBool>,
    token_provider: Option<TokenProvider>,
) {
    let _linkscope_loop = linkscope::phase("voice.vad.listen_loop");
    linkscope::event_fields(
        "voice.vad.listen_loop.start",
        [
            linkscope::TraceField::text("backend", backend.label().to_owned()),
            linkscope::TraceField::text("mode", format!("{:?}", cfg.mode)),
            linkscope::TraceField::text("backend_kind", format!("{:?}", cfg.effective_backend())),
        ],
    );
    debug!(target: "jfc::voice::vad", "VAD loop starting");
    tokio::pin!(stop_rx);

    // Target-speaker gate (BVC decision layer). Built once; inert unless enabled
    // in config AND a profile is enrolled. When active it drops utterances that
    // don't match the enrolled primary speaker (background TV/movie/other voice).
    let speaker_gate = SpeakerGate::from_config(&cfg);

    loop {
        let _linkscope_iteration = linkscope::phase("voice.vad.listen_iteration");
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

        // Half-duplex echo guard: while read-aloud is playing (and for a short
        // decay tail after), ignore the mic so the assistant's own spoken reply
        // doesn't start an utterance. There's no acoustic echo cancellation, so
        // this is the safe default; `voice.echo_suppression = false` (e.g. with
        // headphones) restores full-duplex voice barge-in.
        const ECHO_TAIL: std::time::Duration = std::time::Duration::from_millis(400);
        let mut playback_tail_until: Option<std::time::Instant> = None;
        let mut echo_muted = false;

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

                        // Echo guard: suppress while read-aloud is playing (+tail).
                        let suppress = echo_guard(
                            cfg.echo_suppression,
                            crate::streaming_tts::tts_playback_active(),
                            std::time::Instant::now(),
                            &mut playback_tail_until,
                            ECHO_TAIL,
                        );
                        if suppress {
                            if !echo_muted {
                                debug!(target: "jfc::voice::vad", "read-aloud playing — mic suppressed (echo guard)");
                                echo_muted = true;
                            }
                            // Don't let speaker bleed start an utterance; reset so
                            // we begin clean once playback ends, but keep the
                            // pre-roll rolling for a clean onset.
                            detector.reset();
                            if preroll.len() == PREROLL_FRAMES {
                                preroll.pop_front();
                            }
                            preroll.push_back(frame.to_vec());
                        } else {
                            if echo_muted {
                                debug!(target: "jfc::voice::vad", "echo guard lifted — listening");
                                echo_muted = false;
                            }
                            let vad_events = detector.push(frame);
                            if vad_events.contains(&VadEvent::SpeechStart) {
                                linkscope::event_fields(
                                    "voice.vad.listen_loop.speech_start",
                                    [linkscope::TraceField::bytes(
                                        "preroll_bytes",
                                        u64::try_from(
                                            preroll.iter().map(Vec::len).sum::<usize>(),
                                        )
                                        .unwrap_or(u64::MAX),
                                    )],
                                );
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

        // Resolve the OAuth token up front — live streaming needs it at the
        // START of the utterance, not after recording.
        let token = match token_provider.as_ref() {
            Some(provider) => provider().await,
            None => None,
        };
        linkscope::event_fields(
            "voice.vad.listen_loop.token",
            [linkscope::TraceField::count(
                "available",
                u64::from(token.is_some()),
            )],
        );

        // Decide whether to STREAM this utterance to the live voice_stream WS as
        // it is captured (transcript ready ~immediately at SpeechEnd, no replay
        // latency), or fall back to capture-then-batch. Streaming needs the
        // Anthropic backend + a token.
        let mut want_stream = token.is_some()
            && matches!(
                cfg.effective_backend(),
                SttBackendKind::Anthropic | SttBackendKind::Auto
            );

        // ── Speaker pre-gate ────────────────────────────────────────────────
        // Before opening the WS / hydrating the input box, capture a short onset
        // window WITHOUT streaming and decide whether this speaker is wanted. A
        // rejected utterance (own TTS echo, another person, background TV/movie)
        // then NEVER reaches the voice_stream WS and NEVER types interims into
        // the box — it is dropped here. Only runs when the gate is active AND we
        // were going to stream, so the default (gate-off) path adds no latency.
        // The end-gate on the full `utterance_buf` (below) stays authoritative:
        // if this onset is a false-reject but the whole utterance admits, the
        // batch fallback still transcribes it (self-correcting), so a wrong
        // onset decision never loses real speech — it only costs live interims.
        if want_stream && speaker_gate.is_active() {
            let _linkscope_pregate = linkscope::phase("voice.vad.pregate");
            let pregate_target = utterance_buf.len() + pregate_window_bytes();
            let pregated = loop {
                if utterance_buf.len() >= pregate_target {
                    break true;
                }
                tokio::select! {
                    _ = &mut stop_rx => break false,
                    Some(()) = force_end_rx.recv() => {
                        detector.force_end();
                        break true;
                    }
                    n = capture.read_chunk(&mut chunk) => match n {
                        Ok(0) | Err(_) => break true,
                        Ok(n) => {
                            let frame = &chunk[..n];
                            utterance_buf.extend_from_slice(frame);
                            // Keep the recording-cursor animation alive while we
                            // buffer (no interims are emitted during the pre-gate).
                            send_or_debug(
                                &events,
                                VoiceTranscriptEvent::Level(normalize_level(
                                    crate::vad::rms_energy(frame),
                                )),
                            );
                            // A very short utterance can end inside the window;
                            // stop accumulating and gate on what we have.
                            if detector.push(frame).contains(&VadEvent::SpeechEnd) {
                                break true;
                            }
                        }
                    }
                }
            };
            if !pregated {
                // Stop signalled during the pre-gate window — exit cleanly.
                capture.stop().await;
                break;
            }
            if !speaker_gate.admits(&utterance_buf) {
                // Rejected at the onset: do NOT open the WS. Streaming stays off,
                // so no audio is sent and no interims hydrate the box. The
                // recording loop below still captures the rest (silently) and the
                // end-gate confirms the drop — emitting no Final.
                want_stream = false;
                linkscope::event_fields(
                    "voice.vad.pregate.result",
                    [linkscope::TraceField::count("admitted", 0)],
                );
                info!(
                    target: "jfc::voice::vad",
                    onset_bytes = utterance_buf.len(),
                    "speaker pre-gate rejected onset — not opening voice_stream WS (utterance will use batch if end-gate admits)"
                );
            }
        }
        let mut live: Option<(
            crate::anthropic_ws::VoiceStream,
            mpsc::UnboundedReceiver<crate::anthropic_ws::StreamMsg>,
        )> = None;
        if want_stream {
            let _linkscope_connect = linkscope::phase("voice.vad.live_connect");
            let base = crate::stream_record::resolve_ws_base(&cfg);
            let user_agent = format!("jfc-voice/{}", env!("CARGO_PKG_VERSION"));
            let opts = crate::anthropic_ws::StreamOpts {
                language: cfg.language.clone(),
                keyterms: Vec::new(),
                forward_interims: cfg.forward_interims,
                allow_custom_auth_endpoint: cfg.allow_custom_auth_endpoint,
                allow_insecure_auth_endpoint: cfg.allow_insecure_auth_endpoint,
            };
            let (ev_tx, ev_rx) = mpsc::unbounded_channel::<crate::anthropic_ws::StreamMsg>();
            match crate::anthropic_ws::connect_voice_stream(
                &base,
                token.as_deref().unwrap_or_default(),
                &user_agent,
                "claude_code_cli",
                &opts,
                ev_tx,
            )
            .await
            {
                Ok(stream) => {
                    // Flush the preroll + onset already captured into utterance_buf.
                    let _ = stream.send(&utterance_buf).await;
                    debug!(
                        target: "jfc::voice::vad",
                        preroll_bytes = utterance_buf.len(),
                        "streaming utterance live to voice_stream during capture"
                    );
                    live = Some((stream, ev_rx));
                    linkscope::event_fields(
                        "voice.vad.live_connect.result",
                        [linkscope::TraceField::count("connected", 1)],
                    );
                }
                Err(err) => {
                    warn!(
                        target: "jfc::voice::vad",
                        error = %err,
                        "live voice_stream connect failed — using batch transcribe"
                    );
                    linkscope::event_fields(
                        "voice.vad.live_connect.result",
                        [linkscope::TraceField::count("connected", 0)],
                    );
                }
            }
        }

        // Live-transcript accumulators (used on the streaming path; harmlessly
        // empty on the batch path).
        let mut final_text = String::new();
        let mut interim = String::new();
        let mut got_transcript = false;

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
                        // Stream this frame live as captured — the mic's own
                        // cadence IS the real-time pacing the server needs — then
                        // drain any interim transcripts without blocking capture.
                        let mut drop_live = false;
                        // Server-side endpointing fired this frame (Deepgram
                        // `TranscriptEndpoint` → an `is_final` transcript).
                        let mut server_endpointed = false;
                        if let Some((stream, ev_rx)) = live.as_mut() {
                            if !stream.send(frame).await {
                                drop_live = true;
                            } else {
                                while let Ok(msg) = ev_rx.try_recv() {
                                    if server_endpointed_msg(&msg) {
                                        server_endpointed = true;
                                    }
                                    match msg {
                                        crate::anthropic_ws::StreamMsg::Transcript { text, is_final } => {
                                            crate::stream_record::apply_transcript(
                                                &events,
                                                &mut final_text,
                                                &mut interim,
                                                &mut got_transcript,
                                                text,
                                                is_final,
                                            );
                                        }
                                        crate::anthropic_ws::StreamMsg::Error { msg, .. } => {
                                            warn!(target: "jfc::voice::vad", error = %msg, "live voice_stream error; batch-fallback");
                                            drop_live = true;
                                            break;
                                        }
                                        crate::anthropic_ws::StreamMsg::Closed => {
                                            drop_live = true;
                                            break;
                                        }
                                        _ => {}
                                    }
                                }
                            }
                        }
                        if drop_live {
                            live = None;
                        }
                        // PRIMARY turn-end: the server says you stopped talking.
                        // This is Claude Code's model (server endpointing) and is
                        // reliable where the client energy VAD's SpeechEnd (below)
                        // is not — that's why VAD "recorded forever" without it.
                        if server_endpointed {
                            debug!(
                                target: "jfc::voice::vad",
                                frames = frames_seen,
                                "server endpoint (is_final) — ending utterance"
                            );
                            break true;
                        }
                        frames_seen += 1;
                        let rms = crate::vad::rms_energy(frame);
                        max_rms = max_rms.max(rms);
                        // Feed the recording-cursor animation with the live level.
                        send_or_debug(&events, VoiceTranscriptEvent::Level(normalize_level(rms)));
                        // Periodic heartbeat so we can see the loop is alive
                        // and what RMS it's reading (helps diagnose a high
                        // noise floor that prevents SpeechEnd).
                        if frames_seen.is_multiple_of(50) {
                            linkscope::event_fields(
                                "voice.vad.recording_heartbeat",
                                [
                                    linkscope::TraceField::count("frames", frames_seen),
                                    linkscope::TraceField::count("frame_rms", u64::from(rms)),
                                    linkscope::TraceField::count("max_rms", u64::from(max_rms)),
                                    linkscope::TraceField::bytes(
                                        "buf_bytes",
                                        u64::try_from(utterance_buf.len()).unwrap_or(u64::MAX),
                                    ),
                                ],
                            );
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
        linkscope::event_fields(
            "voice.vad.recording_ended",
            [
                linkscope::TraceField::count("frames", frames_seen),
                linkscope::TraceField::count("max_rms", u64::from(max_rms)),
                linkscope::TraceField::bytes(
                    "bytes",
                    u64::try_from(utterance_buf.len()).unwrap_or(u64::MAX),
                ),
                linkscope::TraceField::count("speech_ended", u64::from(speech_ended)),
            ],
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

        // Discard on cancel (`/voice off` / Esc): drop the utterance without
        // transcribing or emitting a Final.
        if cancel_flag.load(std::sync::atomic::Ordering::SeqCst) {
            if let Some((stream, _)) = live.take() {
                stream.close();
            }
            *state.lock().await = VoiceState::Idle;
            send_or_debug(
                &events,
                VoiceTranscriptEvent::StateChanged(VoiceState::Idle),
            );
            break;
        }

        // Speaker gate FIRST — uniform for the streaming and batch paths. Score
        // the full captured `utterance_buf` (accumulated on both paths) against
        // our speakers (accept) and the assistant's own TTS voice(s) (reject). A
        // rejected utterance — own voice leaking past the echo guard, another
        // person, or background TV/YouTube — is dropped: clear the live interim
        // preview the streaming path may have typed, drop the stream, emit no
        // Final (so it never auto-submits), and keep listening.
        let admitted = !speaker_gate.is_active() || speaker_gate.admits(&utterance_buf);

        // Resolve the transcript. Streaming path: finalize the live stream — the
        // server already transcribed the audio as it arrived, so this resolves
        // almost immediately (no replay latency). Batch path (no stream, connect
        // failed, or the live stream returned nothing): transcribe the captured
        // buffer through the backend chain.
        let mut transcript: Option<String> = None;
        if !admitted {
            if let Some((stream, _)) = live.take() {
                stream.close();
            }
            // Clear any interim text the streaming path already typed into the
            // input box (an empty interim deletes it in place).
            send_or_debug(&events, VoiceTranscriptEvent::Interim(String::new()));
            debug!(
                target: "jfc::voice::vad",
                bytes = utterance_buf.len(),
                speech_ended,
                "utterance rejected by speaker gate (own TTS / another speaker / background) — \
                 dropping without emitting a Final"
            );
        } else if let Some((stream, mut ev_rx)) = live.take() {
            let reason = crate::stream_record::finalize_collecting(
                &stream,
                &mut ev_rx,
                &events,
                &mut final_text,
                &mut interim,
                &mut got_transcript,
            )
            .await;
            stream.close();
            let text = final_text.trim();
            if text.is_empty() {
                info!(target: "jfc::voice::vad", ?reason, "live stream returned empty — batch-fallback");
            } else {
                info!(
                    target: "jfc::voice::vad",
                    chars = text.len(),
                    ?reason,
                    "VAD utterance transcribed (live)"
                );
                transcript = Some(text.to_owned());
            }
        }

        if admitted && transcript.is_none() {
            let _linkscope_batch = linkscope::phase("voice.vad.batch_transcribe");
            // Batch fallback. The speaker gate already ran above, so go straight
            // to transcription here.
            let pcm = std::mem::take(&mut utterance_buf);
            info!(
                target: "jfc::voice::vad",
                bytes = pcm.len(),
                backend = ?cfg.effective_backend(),
                "transcribing utterance (batch)"
            );
            match backends::transcribe_with_token(&pcm, &cfg, token.as_deref()).await {
                Ok(Some(text)) => {
                    info!(target: "jfc::voice::vad", chars = text.len(), "VAD utterance transcribed (batch)");
                    transcript = Some(text);
                }
                Ok(None) => {
                    debug!(target: "jfc::voice::vad", "VAD utterance was empty after transcription");
                }
                Err(err) => {
                    warn!(target: "jfc::voice::vad", error = %err, "VAD transcription failed");
                    send_or_debug(&events, VoiceTranscriptEvent::Error(err.to_string()));
                }
            }
        }

        *state.lock().await = VoiceState::Idle;
        send_or_debug(
            &events,
            VoiceTranscriptEvent::StateChanged(VoiceState::Idle),
        );

        // Suppress the Final for an utterance the user discarded (pressed Enter
        // mid-utterance): emit nothing and keep listening, so the late server
        // endpoint doesn't re-hydrate the box or auto-submit a duplicate. The
        // swap clears the flag so only THIS utterance is dropped.
        let discarded = vad_discard.swap(false, std::sync::atomic::Ordering::SeqCst);
        if let Some(text) = transcript {
            if discarded {
                debug!(target: "jfc::voice::vad", "discarded utterance Final (manual submit)");
            } else {
                info!(
                    target: "jfc::voice::vad",
                    chars = text.chars().count(),
                    speech_ended,
                    "emitting Final for VAD utterance → TUI auto-submit path (acts as Enter)"
                );
                send_or_debug(&events, VoiceTranscriptEvent::Final(text));
            }
        }

        // If stop was signalled, exit the loop.
        if !speech_ended {
            break;
        }

        // Otherwise loop back and listen for the next utterance.
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

    #[test]
    fn server_endpoint_is_final_ends_turn_regression() {
        use crate::anthropic_ws::StreamMsg;
        // The server's TranscriptEndpoint surfaces as an `is_final` transcript —
        // THAT ends the VAD turn (server-side endpointing, like Claude Code),
        // not the flaky client energy VAD. This is the fix for "VAD records
        // forever and never auto-submits".
        assert!(server_endpointed_msg(&StreamMsg::Transcript {
            text: "hello".into(),
            is_final: true,
        }));
        // Interims and control frames must NOT end the turn.
        assert!(!server_endpointed_msg(&StreamMsg::Transcript {
            text: "hel".into(),
            is_final: false,
        }));
        assert!(!server_endpointed_msg(&StreamMsg::Ready));
        assert!(!server_endpointed_msg(&StreamMsg::Closed));
    }

    #[test]
    fn echo_guard_suppresses_during_playback_and_tail_regression() {
        use std::time::{Duration, Instant};
        let tail = Duration::from_millis(400);
        let t0 = Instant::now();
        let mut tail_until = None;

        // Disabled → never suppress, even while playing.
        assert!(!echo_guard(false, true, t0, &mut tail_until, tail));
        assert_eq!(tail_until, None);

        // Enabled + playing → suppress and arm the decay tail.
        assert!(echo_guard(true, true, t0, &mut tail_until, tail));
        assert_eq!(tail_until, Some(t0 + tail));

        // Playback stopped but still within the tail → keep suppressing (covers
        // speaker/room decay so the tail-end doesn't trip the detector).
        assert!(echo_guard(
            true,
            false,
            t0 + Duration::from_millis(200),
            &mut tail_until,
            tail
        ));

        // Past the tail → listen again (full barge-in once the echo is gone).
        assert!(!echo_guard(
            true,
            false,
            t0 + Duration::from_millis(500),
            &mut tail_until,
            tail
        ));
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

    #[test]
    fn pregate_window_bytes_match_sample_rate_normal() {
        // 16 kHz * 2 bytes/sample * (ms / 1000). The pre-gate buffers this many
        // un-streamed bytes at the onset before deciding whether to open the WS.
        assert_eq!(pregate_window_bytes_for_ms(600), 19_200);
        assert_eq!(pregate_window_bytes_for_ms(1_000), 32_000);
        assert_eq!(pregate_window_bytes_for_ms(200), 6_400);
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
        assert!(!gate.is_active());
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
        assert!(!gate.is_active(), "missing profile ⇒ inert gate");
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
            accept: vec![profile],
            reject: Vec::new(),
            embedder: Box::new(crate::speaker::NullEmbedder),
        };
        assert!(gate.is_active());

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

    /// Saving a self-voice reject-profile writes to `reject/<voice>.json`, the
    /// path matches `reject_profile_path`, and a gate built afterwards loads it.
    #[test]
    fn save_reject_profile_roundtrips_and_gate_loads_it_normal() {
        let dir = std::env::temp_dir().join(format!("jfc_reject_test_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let cfg = VoiceConfig {
            // profile_dir() derives from the parent of speaker_profile_path.
            speaker_profile_path: Some(
                dir.join("speaker_profile.json")
                    .to_string_lossy()
                    .into_owned(),
            ),
            ..Default::default()
        };

        let tts = synth_pcm(150.0, 2.0, 1.3, 5);
        let path =
            save_reject_profile_from_pcm(&cfg, "buttery", &tts).expect("save reject profile");
        assert_eq!(path, reject_profile_path(&cfg, "buttery"));
        assert!(path.exists(), "reject profile file must be written");

        // A gate built with the gate enabled now carries the reject profile and
        // is active (will filter), even with no accept-list enrolled.
        let gate = SpeakerGate::from_config(&VoiceConfig {
            speaker_gate: true,
            ..cfg
        });
        assert!(gate.is_active(), "reject-only gate must be active");
        assert_eq!(gate.reject.len(), 1);
        assert!(gate.accept.is_empty());

        // An utterance from the same TTS source is rejected (own voice).
        let tts_again = synth_pcm(150.0, 1.5, 1.3, 77);
        assert!(
            !gate.admits(&tts_again),
            "self-voice utterance must be dropped"
        );

        let _ = std::fs::remove_dir_all(&dir);
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

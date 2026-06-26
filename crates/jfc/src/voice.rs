//! Voice mode integration — bridges jfc-voice into the TUI.
//!
//! Lifecycle:
//! 1. `init()` reads voice config and starts the event forwarder.
//! 2. Key handlers call `activate(pressed)` for push-to-talk.
//! 3. The event loop consumes `VoiceEvent` from the engine bus.
//! 4. `EngineEvent::Voice(Final(text))` injects text into the textarea.

use std::sync::OnceLock;
use tokio::sync::Mutex;

use jfc_engine::runtime::{EngineEvent, VoiceEvent};
use jfc_voice::{VoiceConfig, VoiceRecorder, VoiceTranscriptEvent};

/// Process-global recorder handle.
static RECORDER: OnceLock<Mutex<VoiceRecorder>> = OnceLock::new();

/// Initialize voice mode.
///
/// Reads the voice config from `~/.claude/settings.json` (via the loaded
/// `ClaudeCompatibilityConfig`), creates the recorder, and starts routing
/// transcript events → the engine event bus.
///
/// Returns immediately without initializing when voice is disabled or config is absent.
pub fn init(voice_value: Option<&serde_json::Value>, engine_tx: jfc_engine::runtime::EventSender) {
    let cfg = VoiceConfig::from_settings(voice_value);
    init_with_config(cfg, engine_tx);
}

fn init_with_config(cfg: VoiceConfig, engine_tx: jfc_engine::runtime::EventSender) {
    if !cfg.enabled {
        tracing::debug!(target: "jfc::voice", "voice mode disabled (voice.enabled=false)");
        return;
    }

    tracing::info!(
        target: "jfc::voice",
        mode = %cfg.mode.label(),
        "voice mode initializing"
    );

    let (event_tx, mut event_rx) = tokio::sync::mpsc::unbounded_channel::<VoiceTranscriptEvent>();

    // Token provider: resolve the real Claude.ai OAuth access token from the
    // shared accounts store on demand, so the live Anthropic voice stream is
    // wired to the same auth the rest of the app uses (not a dead env var).
    let token_provider: jfc_voice::TokenProvider =
        std::sync::Arc::new(|| Box::pin(jfc_providers::current_access_token()));
    let recorder = VoiceRecorder::new(cfg, event_tx).with_token_provider(token_provider);

    if RECORDER.set(Mutex::new(recorder)).is_err() {
        tracing::warn!(target: "jfc::voice", "init called twice — ignoring");
        return;
    }

    // Forward transcript events → engine bus as VoiceEvent
    tokio::spawn(async move {
        while let Some(ev) = event_rx.recv().await {
            let engine_ev = match ev {
                VoiceTranscriptEvent::Interim(t) => EngineEvent::Voice(VoiceEvent::Interim(t)),
                VoiceTranscriptEvent::Final(t) => EngineEvent::Voice(VoiceEvent::Final(t)),
                VoiceTranscriptEvent::Level(l) => EngineEvent::Voice(VoiceEvent::Level(l)),
                VoiceTranscriptEvent::Error(e) => EngineEvent::Voice(VoiceEvent::Error(e)),
                VoiceTranscriptEvent::StateChanged(s) => {
                    EngineEvent::Voice(VoiceEvent::StateChanged(s as u8))
                }
            };
            if engine_tx.send(engine_ev).await.is_err() {
                break; // engine bus closed
            }
        }
    });
}

pub async fn configure(
    voice_value: Option<&serde_json::Value>,
    engine_tx: jfc_engine::runtime::EventSender,
) {
    let cfg = VoiceConfig::from_settings(voice_value);
    if let Some(rec) = RECORDER.get() {
        rec.lock().await.reconfigure(cfg);
    } else {
        init_with_config(cfg, engine_tx);
    }
}

/// Activate/deactivate push-to-talk.
///
/// - `pressed = true` → key down (start recording in hold mode, or toggle in tap mode)
/// - `pressed = false` → key up (stop recording in hold mode only)
///
/// Returns immediately if voice has not been initialized (voice is disabled or not configured).
pub async fn activate(pressed: bool) {
    if let Some(rec) = RECORDER.get() {
        rec.lock().await.activate(pressed).await;
    }
}

/// Cancel any active recording (e.g. on Esc) AND the VAD listen loop.
pub async fn cancel() {
    if let Some(rec) = RECORDER.get() {
        rec.lock().await.cancel().await;
    }
}

/// Discard an in-flight hold/tap recording without emitting a transcript — used
/// when the user submits manually (Enter) so voice doesn't auto-submit a
/// duplicate. Leaves the VAD listen loop running.
pub async fn discard_recording() {
    if let Some(rec) = RECORDER.get() {
        rec.lock().await.discard_recording().await;
    }
}

/// Current voice state for rendering.
pub async fn state() -> jfc_voice::VoiceState {
    if let Some(rec) = RECORDER.get() {
        rec.lock().await.state().await
    } else {
        jfc_voice::VoiceState::Idle
    }
}

/// Start the VAD continuous-listen loop. Called by `/voice vad`.
/// The recorder must already be initialized. The loop runs until `cancel()`.
pub async fn start_vad() {
    if let Some(rec) = RECORDER.get() {
        rec.lock().await.start_vad_loop().await;
    }
}

/// True if voice has been initialized (enabled + init() called).
pub fn is_initialized() -> bool {
    RECORDER.get().is_some()
}

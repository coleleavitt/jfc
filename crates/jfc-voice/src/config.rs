//! Voice mode configuration — parsed from ClaudeCompatibilityConfig.voice.

use serde::{Deserialize, Serialize};

/// How push-to-talk or voice activity detection is triggered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum VoiceMode {
    /// Hold the push-to-talk key to record; release to submit (default).
    #[default]
    Hold,
    /// Tap the key once to start recording, tap again to stop and submit.
    Tap,
    /// Hands-free: always listening, auto-detects speech via energy VAD.
    /// Starts recording when you speak, stops after silence, auto-submits.
    /// No key press needed — just talk.
    Vad,
}

impl VoiceMode {
    pub fn from_str(s: &str) -> Option<Self> {
        match s.trim().to_lowercase().as_str() {
            "hold" => Some(Self::Hold),
            "tap" => Some(Self::Tap),
            "vad" | "auto" | "handsfree" | "hands-free" | "continuous" => Some(Self::Vad),
            _ => None,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Hold => "hold",
            Self::Tap => "tap",
            Self::Vad => "vad",
        }
    }
}

/// Which VAD engine drives hands-free speech detection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum VadEngine {
    /// Dependency-free energy + periodicity + modulation detector (default).
    #[default]
    Energy,
    /// Neural Silero VAD (requires the `vad-neural` build feature). Far more
    /// robust to tonal noise / babble / low SNR; falls back to Energy when the
    /// feature isn't compiled in or the model fails to load.
    Neural,
}

impl VadEngine {
    /// Resolve the configured engine from `JFC_VAD_ENGINE`.
    ///
    /// - An explicit value always wins: `neural`/`silero`/`onnx`/`ml` → Neural,
    ///   `energy`/`classic`/`default` → Energy.
    /// - When unset, the default depends on the build: if compiled with the
    ///   `vad-neural` feature the neural Silero engine is the default (it's the
    ///   more robust detector); otherwise Energy. This means a `vad-neural`
    ///   build is hands-free-neural out of the box, and `JFC_VAD_ENGINE=energy`
    ///   is the opt-out.
    pub fn from_env() -> Self {
        match std::env::var("JFC_VAD_ENGINE")
            .unwrap_or_default()
            .to_lowercase()
            .as_str()
        {
            "neural" | "silero" | "onnx" | "ml" => Self::Neural,
            "energy" | "classic" | "default" => Self::Energy,
            // Unset / unrecognized → build-dependent default.
            _ => Self::build_default(),
        }
    }

    /// The default engine for this build: Neural when the `vad-neural` feature
    /// is compiled in, Energy otherwise.
    pub const fn build_default() -> Self {
        #[cfg(feature = "vad-neural")]
        {
            Self::Neural
        }
        #[cfg(not(feature = "vad-neural"))]
        {
            Self::Energy
        }
    }

    /// Parse from a config-file string. `None` for unrecognized values.
    pub fn from_str(s: &str) -> Option<Self> {
        match s.trim().to_lowercase().as_str() {
            "energy" | "classic" | "default" => Some(Self::Energy),
            "neural" | "silero" | "onnx" | "ml" => Some(Self::Neural),
            _ => None,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Energy => "energy",
            Self::Neural => "neural",
        }
    }
}

/// Resolved voice configuration.
#[derive(Debug, Clone, Default)]
pub struct VoiceConfig {
    /// Voice mode is enabled.
    pub enabled: bool,
    /// Hold or tap mode.
    pub mode: VoiceMode,
    /// Which VAD engine drives hands-free mode (energy vs neural Silero).
    pub vad_engine: VadEngine,
    /// Auto-submit after hold-to-talk release (hold mode only).
    pub auto_submit: bool,
    /// BCP-47 language code for STT (default "en").
    pub language: String,
    /// Which STT backend to prefer.
    pub backend: SttBackendKind,
    /// Override the Anthropic voice stream WebSocket URL.
    pub anthropic_voice_url: Option<String>,
    /// OpenAI API key for Whisper API backend.
    pub openai_api_key: Option<String>,
    /// Path to local whisper binary (e.g. "whisper-cpp", "whisper").
    pub local_whisper_bin: Option<String>,
    /// Path to whisper model file for local backend.
    pub local_whisper_model: Option<String>,
    /// Target-speaker gate: when enabled and a profile is enrolled, captured
    /// utterances that don't match the enrolled primary speaker (e.g. a movie /
    /// TV / another person in the room) are dropped instead of transcribed.
    /// OFF by default; opt-in via config `voice.speakerGate` or
    /// `JFC_VOICE_SPEAKER_GATE`.
    pub speaker_gate: bool,
    /// Path to the enrolled [`crate::speaker::SpeakerProfile`] JSON. When unset,
    /// defaults to `<config dir>/speaker_profile.json`. The gate no-ops when the
    /// file is missing/unreadable.
    pub speaker_profile_path: Option<String>,
    /// Optional override for the profile's calibrated acceptance threshold
    /// (`JFC_VOICE_SPEAKER_THRESHOLD`). Larger = more permissive.
    pub speaker_threshold: Option<f64>,
    /// Path to an ECAPA-TDNN/x-vector ONNX speaker-embedding model
    /// (`JFC_VOICE_SPEAKER_MODEL`). Only used when built with the
    /// `speaker-neural` feature; enables the SOTA-accuracy neural gate. When
    /// unset/unavailable the gate uses the classical MFCC-template score. The
    /// embedder reads this env directly; the field is here for discoverability
    /// and so the config can surface it.
    pub speaker_model_path: Option<String>,
}

/// Which STT backend to attempt first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SttBackendKind {
    /// Try Anthropic WebSocket first, then OpenAI, then local.
    #[default]
    Auto,
    /// Anthropic real-time WebSocket (requires Claude.ai OAuth).
    Anthropic,
    /// OpenAI Whisper API (requires OPENAI_API_KEY).
    OpenAiWhisper,
    /// Local whisper.cpp binary (works offline).
    LocalWhisper,
}

impl VoiceConfig {
    /// Build from the `voice` serde_json::Value from ClaudeCompatibilityConfig.
    pub fn from_settings(voice_value: Option<&serde_json::Value>) -> Self {
        let mut cfg = Self {
            language: std::env::var("JFC_VOICE_LANGUAGE").unwrap_or_else(|_| "en".to_owned()),
            anthropic_voice_url: std::env::var("JFC_VOICE_ANTHROPIC_URL")
                .ok()
                .or_else(|| std::env::var("VOICE_STREAM_BASE_URL").ok()),
            openai_api_key: std::env::var("OPENAI_API_KEY").ok(),
            local_whisper_bin: std::env::var("JFC_WHISPER_BIN").ok(),
            local_whisper_model: std::env::var("JFC_WHISPER_MODEL").ok(),
            backend: parse_backend_env(),
            vad_engine: VadEngine::from_env(),
            speaker_gate: env_flag("JFC_VOICE_SPEAKER_GATE"),
            speaker_profile_path: std::env::var("JFC_VOICE_SPEAKER_PROFILE").ok(),
            speaker_threshold: std::env::var("JFC_VOICE_SPEAKER_THRESHOLD")
                .ok()
                .and_then(|s| s.parse().ok()),
            speaker_model_path: std::env::var("JFC_VOICE_SPEAKER_MODEL").ok(),
            ..Default::default()
        };

        let Some(v) = voice_value else { return cfg };

        // voice.enabled / voiceEnabled (both shapes CC supports)
        if let Some(enabled) = v.get("enabled").and_then(|e| e.as_bool()) {
            cfg.enabled = enabled;
        }

        // voice.mode: "hold" | "tap"
        if let Some(mode_str) = v.get("mode").and_then(|m| m.as_str()) {
            if let Some(mode) = VoiceMode::from_str(mode_str) {
                cfg.mode = mode;
            }
        }

        // voice.vadEngine: "energy" | "neural" (env JFC_VAD_ENGINE wins).
        if let Some(engine_str) = v.get("vadEngine").and_then(|m| m.as_str()) {
            if std::env::var("JFC_VAD_ENGINE").is_err() {
                if let Some(engine) = VadEngine::from_str(engine_str) {
                    cfg.vad_engine = engine;
                }
            }
        }

        // voice.autoSubmit
        if let Some(auto) = v.get("autoSubmit").and_then(|a| a.as_bool()) {
            cfg.auto_submit = auto;
        }

        // voice.speakerGate (env JFC_VOICE_SPEAKER_GATE wins).
        if std::env::var("JFC_VOICE_SPEAKER_GATE").is_err() {
            if let Some(g) = v.get("speakerGate").and_then(|g| g.as_bool()) {
                cfg.speaker_gate = g;
            }
        }
        // voice.speakerProfile (path) / voice.speakerThreshold.
        if cfg.speaker_profile_path.is_none() {
            cfg.speaker_profile_path = v
                .get("speakerProfile")
                .and_then(|p| p.as_str())
                .map(|s| s.to_owned());
        }
        if cfg.speaker_threshold.is_none() {
            cfg.speaker_threshold = v.get("speakerThreshold").and_then(|t| t.as_f64());
        }
        // voice.speakerModel: ONNX embedding model path (speaker-neural). The
        // embedder reads JFC_VOICE_SPEAKER_MODEL, so mirror a config value into
        // the env when the env isn't already set, keeping env-wins precedence.
        if cfg.speaker_model_path.is_none() {
            if let Some(p) = v.get("speakerModel").and_then(|p| p.as_str()) {
                cfg.speaker_model_path = Some(p.to_owned());
                // SAFETY: single-threaded config load at startup; bridges the
                // config value to the env the embedder reads.
                unsafe { std::env::set_var("JFC_VOICE_SPEAKER_MODEL", p) };
            }
        }

        cfg
    }

    /// Determine which backend to actually use, given available credentials.
    pub fn effective_backend(&self) -> SttBackendKind {
        match self.backend {
            SttBackendKind::Auto => {
                // Anthropic first if we have any auth (checked at call time)
                SttBackendKind::Anthropic
            }
            other => other,
        }
    }

    /// Human-readable description of the active mode for the /voice output.
    pub fn mode_hint(&self) -> String {
        let key = "Space"; // default push-to-talk key
        match self.mode {
            VoiceMode::Hold => format!("Hold {key} to record, release to send."),
            VoiceMode::Tap => format!("Tap {key} (empty input) to start, tap again to send."),
            VoiceMode::Vad => format!(
                "Hands-free — just speak. {} VAD detects speech automatically.",
                self.vad_engine.label()
            ),
        }
    }
}

fn parse_backend_env() -> SttBackendKind {
    match std::env::var("JFC_VOICE_BACKEND")
        .unwrap_or_default()
        .to_lowercase()
        .as_str()
    {
        "anthropic" => SttBackendKind::Anthropic,
        "openai" | "whisper-api" | "openai-whisper" => SttBackendKind::OpenAiWhisper,
        "local" | "whisper" | "local-whisper" | "whisper-cpp" => SttBackendKind::LocalWhisper,
        _ => SttBackendKind::Auto,
    }
}

/// Interpret an env var as a boolean flag (`1`/`true`/`yes`/`on` → true).
fn env_flag(key: &str) -> bool {
    matches!(
        std::env::var(key)
            .unwrap_or_default()
            .to_lowercase()
            .as_str(),
        "1" | "true" | "yes" | "on"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn voice_mode_from_str_normal() {
        assert_eq!(VoiceMode::from_str("hold"), Some(VoiceMode::Hold));
        assert_eq!(VoiceMode::from_str("tap"), Some(VoiceMode::Tap));
        assert_eq!(VoiceMode::from_str("vad"), Some(VoiceMode::Vad));
        assert_eq!(VoiceMode::from_str("auto"), Some(VoiceMode::Vad));
        assert_eq!(VoiceMode::from_str("HOLD"), Some(VoiceMode::Hold));
        assert_eq!(VoiceMode::from_str("off"), None);
    }

    #[test]
    fn voice_config_from_settings_normal() {
        let val = json!({"enabled": true, "mode": "tap", "autoSubmit": true});
        let cfg = VoiceConfig::from_settings(Some(&val));
        assert!(cfg.enabled);
        assert_eq!(cfg.mode, VoiceMode::Tap);
        assert!(cfg.auto_submit);
    }

    #[test]
    fn voice_config_defaults_on_none_robust() {
        let cfg = VoiceConfig::from_settings(None);
        assert!(!cfg.enabled);
        assert_eq!(cfg.mode, VoiceMode::Hold);
        // The engine comes from the env resolver, which uses the build default
        // when JFC_VAD_ENGINE is unset (Neural for a vad-neural build).
        if std::env::var("JFC_VAD_ENGINE").is_err() {
            assert_eq!(cfg.vad_engine, VadEngine::build_default());
        }
    }

    #[test]
    fn vad_engine_from_str_normal() {
        assert_eq!(VadEngine::from_str("energy"), Some(VadEngine::Energy));
        assert_eq!(VadEngine::from_str("neural"), Some(VadEngine::Neural));
        assert_eq!(VadEngine::from_str("silero"), Some(VadEngine::Neural));
        assert_eq!(VadEngine::from_str("ONNX"), Some(VadEngine::Neural));
        assert_eq!(VadEngine::from_str("bogus"), None);
    }

    #[test]
    fn vad_engine_derive_default_is_energy_normal() {
        // The `#[derive(Default)]` value is always Energy (used by struct
        // literals); the *build* default may differ when vad-neural is on.
        assert_eq!(VadEngine::default(), VadEngine::Energy);
        assert_eq!(VadEngine::Energy.label(), "energy");
        assert_eq!(VadEngine::Neural.label(), "neural");
    }

    #[cfg(feature = "vad-neural")]
    #[test]
    fn build_default_is_neural_with_feature_normal() {
        assert_eq!(VadEngine::build_default(), VadEngine::Neural);
    }

    #[cfg(not(feature = "vad-neural"))]
    #[test]
    fn build_default_is_energy_without_feature_normal() {
        assert_eq!(VadEngine::build_default(), VadEngine::Energy);
    }

    #[test]
    fn voice_config_reads_vad_engine_from_settings_normal() {
        // Only when the env override isn't set (env wins over file).
        if std::env::var("JFC_VAD_ENGINE").is_err() {
            let val = json!({"enabled": true, "mode": "vad", "vadEngine": "neural"});
            let cfg = VoiceConfig::from_settings(Some(&val));
            assert_eq!(cfg.vad_engine, VadEngine::Neural);
        }
    }
}

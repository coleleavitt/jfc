use serde::{Deserialize, Serialize};

use crate::conversation_ws::VoiceConversationOptions;

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
    /// The safe default engine for every build.
    pub const fn build_default() -> Self {
        Self::Energy
    }

    /// Whether this process is allowed to construct the native neural VAD.
    pub fn neural_runtime_enabled() -> bool {
        matches!(
            std::env::var("JFC_VAD_ENABLE_NEURAL")
                .unwrap_or_default()
                .to_lowercase()
                .as_str(),
            "1" | "true" | "yes" | "on"
        )
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
#[derive(Debug, Clone)]
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
    pub speaker_gate: bool,
    /// Path to the enrolled [`crate::speaker::SpeakerProfile`] JSON. When unset,
    /// defaults to `<config dir>/speaker_profile.json`. The gate no-ops when the
    /// file is missing/unreadable.
    pub speaker_profile_path: Option<String>,
    /// Optional override for the profile's calibrated acceptance threshold.
    pub speaker_threshold: Option<f64>,
    /// Path to an ECAPA-TDNN/x-vector ONNX speaker-embedding model.
    pub speaker_model_path: Option<String>,
    pub read_aloud: bool,
    /// Half-duplex echo guard: while read-aloud is playing (+ a short decay
    /// tail), suppress VAD mic-start so the assistant's own spoken reply can't
    /// trigger an utterance. There is no acoustic echo cancellation, so this
    /// defaults on. Turn it off (`echoSuppression: false`) for full-duplex
    /// voice barge-in if you use headphones.
    pub echo_suppression: bool,
    /// TTS voice style passed to `text_to_speech/text_stream?voice=…`. The five
    /// Anthropic voices (claude.ai picker): `buttery` (default), `airy`,
    /// `mellow`, `glassy`, `rounded`. Any server-accepted value works.
    pub tts_voice: String,
    pub tts_speed: f32,
    pub tts_output_format: String,
    pub tts_base_url: Option<String>,
    pub tts_playback_command: Option<String>,
    pub selected_speaker_device_id: Option<String>,
    pub conversation_enabled: bool,
    pub conversation_base_url: Option<String>,
    pub conversation_organization_uuid: Option<String>,
    pub conversation_uuid: Option<String>,
    pub conversation_input_encoding: String,
    pub conversation_output_format: String,
    pub conversation_timezone: String,
    pub conversation_model: Option<String>,
    pub conversation_effort: Option<String>,
    pub conversation_thinking_mode: Option<String>,
    pub forward_interims: bool,
    pub allow_custom_auth_endpoint: bool,
    pub allow_insecure_auth_endpoint: bool,
}

impl Default for VoiceConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            mode: VoiceMode::Hold,
            vad_engine: VadEngine::build_default(),
            auto_submit: false,
            language: "en".to_owned(),
            backend: SttBackendKind::Auto,
            anthropic_voice_url: None,
            openai_api_key: None,
            local_whisper_bin: None,
            local_whisper_model: None,
            speaker_gate: false,
            speaker_profile_path: None,
            speaker_threshold: None,
            speaker_model_path: None,
            read_aloud: false,
            echo_suppression: true,
            tts_voice: "buttery".to_owned(),
            tts_speed: 1.0,
            tts_output_format: "pcm_16000".to_owned(),
            tts_base_url: None,
            tts_playback_command: None,
            selected_speaker_device_id: None,
            conversation_enabled: false,
            conversation_base_url: None,
            conversation_organization_uuid: None,
            conversation_uuid: None,
            conversation_input_encoding: "linear16".to_owned(),
            conversation_output_format: "pcm_16000".to_owned(),
            conversation_timezone: "UTC".to_owned(),
            conversation_model: None,
            conversation_effort: None,
            conversation_thinking_mode: None,
            forward_interims: true,
            allow_custom_auth_endpoint: false,
            allow_insecure_auth_endpoint: false,
        }
    }
}

/// Which STT backend to attempt first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SttBackendKind {
    /// Try Anthropic WebSocket first, then OpenAI, then local.
    #[default]
    Auto,
    /// Anthropic real-time WebSocket (requires Claude.ai OAuth).
    Anthropic,
    /// OpenAI Whisper API.
    OpenAiWhisper,
    /// Local whisper.cpp binary (works offline).
    LocalWhisper,
}

impl VoiceConfig {
    /// Build from the `voice` serde_json::Value from ClaudeCompatibilityConfig.
    pub fn from_settings(voice_value: Option<&serde_json::Value>) -> Self {
        let mut cfg = Self::default();

        let Some(v) = voice_value else { return cfg };

        // voice.enabled / voiceEnabled (both shapes CC supports)
        if let Some(enabled) = v.get("enabled").and_then(|e| e.as_bool()) {
            cfg.enabled = enabled;
        }

        if let Some(mode_str) = v.get("mode").and_then(|m| m.as_str()) {
            if let Some(mode) = VoiceMode::from_str(mode_str) {
                cfg.mode = mode;
            }
        }

        if let Some(language) = string_field(v, &["language"]) {
            cfg.language = language.to_owned();
        }
        if let Some(backend) = string_field(v, &["backend"])
            && let Some(kind) = SttBackendKind::from_str(backend)
        {
            cfg.backend = kind;
        }
        if let Some(url) = string_field(v, &["anthropicVoiceUrl", "anthropic_voice_url"]) {
            cfg.anthropic_voice_url = Some(url.to_owned());
        }
        if let Some(key) = string_field(v, &["openaiApiKey", "openai_api_key"]) {
            cfg.openai_api_key = Some(key.to_owned());
        }
        if let Some(bin) = string_field(v, &["localWhisperBin", "local_whisper_bin"]) {
            cfg.local_whisper_bin = Some(bin.to_owned());
        }
        if let Some(model) = string_field(v, &["localWhisperModel", "local_whisper_model"]) {
            cfg.local_whisper_model = Some(model.to_owned());
        }

        if let Some(engine_str) = v.get("vadEngine").and_then(|m| m.as_str()) {
            if let Some(engine) = VadEngine::from_str(engine_str) {
                cfg.vad_engine =
                    if engine == VadEngine::Neural && !VadEngine::neural_runtime_enabled() {
                        VadEngine::Energy
                    } else {
                        engine
                    };
            }
        }

        // voice.autoSubmit
        if let Some(auto) = v.get("autoSubmit").and_then(|a| a.as_bool()) {
            cfg.auto_submit = auto;
        }

        if let Some(read) = v
            .get("readAloud")
            .or_else(|| v.get("readAssistant"))
            .and_then(|a| a.as_bool())
        {
            cfg.read_aloud = read;
        }

        if let Some(echo) = v
            .get("echoSuppression")
            .or_else(|| v.get("echo_suppression"))
            .and_then(|a| a.as_bool())
        {
            cfg.echo_suppression = echo;
        }

        if let Some(voice) = v.get("ttsVoice").and_then(|p| p.as_str()) {
            cfg.tts_voice = voice.to_owned();
        }
        if let Some(speed) = v.get("ttsSpeed").and_then(|p| p.as_f64()) {
            cfg.tts_speed = speed as f32;
        }
        if let Some(format) = v.get("ttsOutputFormat").and_then(|p| p.as_str()) {
            cfg.tts_output_format = format.to_owned();
        }
        if cfg.tts_base_url.is_none() {
            cfg.tts_base_url = v
                .get("ttsBaseUrl")
                .or_else(|| v.get("tts_base_url"))
                .and_then(|p| p.as_str())
                .map(|s| s.to_owned());
        }
        if cfg.selected_speaker_device_id.is_none() {
            cfg.selected_speaker_device_id = v
                .get("selectedSpeakerDeviceId")
                .or_else(|| v.get("speakerDeviceId"))
                .and_then(|p| p.as_str())
                .map(|s| s.to_owned());
        }
        if let Some(enabled) = v
            .get("conversationEnabled")
            .or_else(|| v.get("fullDuplex"))
            .and_then(|p| p.as_bool())
        {
            cfg.conversation_enabled = enabled;
        }
        if cfg.conversation_base_url.is_none() {
            cfg.conversation_base_url = v
                .get("conversationBaseUrl")
                .and_then(|p| p.as_str())
                .map(str::to_owned);
        }
        if cfg.conversation_organization_uuid.is_none() {
            cfg.conversation_organization_uuid = v
                .get("organizationUuid")
                .or_else(|| v.get("organizationUUID"))
                .or_else(|| v.get("orgUuid"))
                .and_then(|p| p.as_str())
                .map(str::to_owned);
        }
        if cfg.conversation_uuid.is_none() {
            cfg.conversation_uuid = v
                .get("conversationUuid")
                .or_else(|| v.get("conversationUUID"))
                .and_then(|p| p.as_str())
                .map(str::to_owned);
        }
        if let Some(encoding) = v.get("conversationInputEncoding").and_then(|p| p.as_str()) {
            cfg.conversation_input_encoding = encoding.to_owned();
        }
        if let Some(format) = v.get("conversationOutputFormat").and_then(|p| p.as_str()) {
            cfg.conversation_output_format = format.to_owned();
        }
        if let Some(timezone) = v.get("timezone").and_then(|p| p.as_str()) {
            cfg.conversation_timezone = timezone.to_owned();
        }
        if cfg.conversation_model.is_none() {
            cfg.conversation_model = v
                .get("conversationModel")
                .or_else(|| v.get("model"))
                .and_then(|p| p.as_str())
                .map(str::to_owned);
        }
        if cfg.conversation_effort.is_none() {
            cfg.conversation_effort = v
                .get("conversationEffort")
                .or_else(|| v.get("effort"))
                .and_then(|p| p.as_str())
                .map(str::to_owned);
        }
        if cfg.conversation_thinking_mode.is_none() {
            cfg.conversation_thinking_mode = v
                .get("conversationThinkingMode")
                .or_else(|| v.get("thinkingMode"))
                .and_then(|p| p.as_str())
                .map(str::to_owned);
        }

        if let Some(g) = v.get("speakerGate").and_then(|g| g.as_bool()) {
            cfg.speaker_gate = g;
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
        if cfg.speaker_model_path.is_none() {
            if let Some(p) = v.get("speakerModel").and_then(|p| p.as_str()) {
                cfg.speaker_model_path = Some(p.to_owned());
            }
        }
        if let Some(value) = v.get("forwardInterims").and_then(|value| value.as_bool()) {
            cfg.forward_interims = value;
        }
        if let Some(value) = v
            .get("allowCustomAuthEndpoint")
            .and_then(|value| value.as_bool())
        {
            cfg.allow_custom_auth_endpoint = value;
        }
        if let Some(value) = v
            .get("allowInsecureAuthEndpoint")
            .and_then(|value| value.as_bool())
        {
            cfg.allow_insecure_auth_endpoint = value;
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

    pub fn clamped_tts_speed(&self) -> f32 {
        self.tts_speed.clamp(0.7, 1.2)
    }

    pub fn voice_conversation_options(&self) -> Option<VoiceConversationOptions> {
        if !self.conversation_enabled {
            return None;
        }
        let org = self
            .conversation_organization_uuid
            .as_deref()
            .filter(|value| !value.trim().is_empty())?;
        let conversation = self
            .conversation_uuid
            .as_deref()
            .filter(|value| !value.trim().is_empty())?;
        let mut opts = VoiceConversationOptions::new(org, conversation);
        opts.input_encoding = self.conversation_input_encoding.clone();
        opts.output_format = self.conversation_output_format.clone();
        opts.language = self.language.clone();
        opts.timezone = self.conversation_timezone.clone();
        opts.voice = self.tts_voice.clone();
        opts.tts_speed = self.tts_speed;
        opts.model = self.conversation_model.clone();
        opts.effort = self.conversation_effort.clone();
        opts.thinking_mode = self.conversation_thinking_mode.clone();
        opts.allow_custom_auth_endpoint = self.allow_custom_auth_endpoint;
        opts.allow_insecure_auth_endpoint = self.allow_insecure_auth_endpoint;
        Some(opts)
    }

    pub(crate) fn endpoint_policy(&self) -> crate::auth_endpoint::AuthEndpointPolicy {
        crate::auth_endpoint::AuthEndpointPolicy {
            allow_custom: self.allow_custom_auth_endpoint,
            allow_insecure: self.allow_insecure_auth_endpoint,
        }
    }
}

impl SttBackendKind {
    pub fn from_str(value: &str) -> Option<Self> {
        match value.trim().to_lowercase().as_str() {
            "auto" => Some(Self::Auto),
            "anthropic" => Some(Self::Anthropic),
            "openai" | "whisper-api" | "openai-whisper" => Some(Self::OpenAiWhisper),
            "local" | "whisper" | "local-whisper" | "whisper-cpp" => Some(Self::LocalWhisper),
            _ => None,
        }
    }
}

fn string_field<'a>(value: &'a serde_json::Value, keys: &[&str]) -> Option<&'a str> {
    keys.iter()
        .find_map(|key| value.get(*key).and_then(|value| value.as_str()))
        .filter(|value| !value.trim().is_empty())
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
    fn voice_config_reads_tts_settings_normal() {
        let val = json!({
            "readAloud": true,
            "ttsVoice": "mellow",
            "ttsSpeed": 2.0,
            "ttsBaseUrl": "https://voice.example",
            "ttsPlaybackCommand": "aplay -q -f S16_LE -r 16000 -c 1",
            "selectedSpeakerDeviceId": "alsa:hw:0,0"
        });
        let cfg = VoiceConfig::from_settings(Some(&val));

        assert!(cfg.read_aloud);
        assert_eq!(cfg.tts_voice, "mellow");
        assert_eq!(cfg.clamped_tts_speed(), 1.2);
        assert_eq!(cfg.tts_base_url.as_deref(), Some("https://voice.example"));
        assert!(cfg.tts_playback_command.is_none());
        assert_eq!(
            cfg.selected_speaker_device_id.as_deref(),
            Some("alsa:hw:0,0")
        );
    }

    #[test]
    fn voice_config_reads_conversation_settings_normal() {
        let val = json!({
            "conversationEnabled": true,
            "organizationUuid": "org-123",
            "conversationUuid": "conv-456",
            "conversationInputEncoding": "pcm_s16le",
            "conversationOutputFormat": "pcm_16000",
            "timezone": "America/Detroit",
            "conversationModel": "claude-test",
            "conversationEffort": "medium"
        });
        let cfg = VoiceConfig::from_settings(Some(&val));
        let opts = cfg.voice_conversation_options().unwrap();

        assert!(cfg.conversation_enabled);
        assert_eq!(opts.organization_uuid, "org-123");
        assert_eq!(opts.conversation_uuid, "conv-456");
        assert_eq!(opts.input_encoding, "pcm_s16le");
        assert_eq!(opts.output_format, "pcm_16000");
        assert_eq!(opts.timezone, "America/Detroit");
        assert_eq!(opts.model.as_deref(), Some("claude-test"));
        assert_eq!(opts.effort.as_deref(), Some("medium"));
    }

    #[test]
    fn voice_config_defaults_on_none_robust() {
        let cfg = VoiceConfig::from_settings(None);
        assert!(!cfg.enabled);
        assert_eq!(cfg.mode, VoiceMode::Hold);
        assert_eq!(cfg.vad_engine, VadEngine::build_default());
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

    #[test]
    fn build_default_is_energy_even_with_neural_feature_regression() {
        assert_eq!(VadEngine::build_default(), VadEngine::Energy);
    }

    #[test]
    fn voice_config_ignores_neural_settings_without_native_opt_in_regression() {
        if !VadEngine::neural_runtime_enabled() {
            let val = json!({"enabled": true, "mode": "vad", "vadEngine": "neural"});
            let cfg = VoiceConfig::from_settings(Some(&val));
            assert_eq!(cfg.vad_engine, VadEngine::Energy);
        }
    }

    #[test]
    fn voice_config_reads_backend_and_endpoint_policy_normal() {
        let val = json!({
            "backend": "anthropic",
            "forwardInterims": false,
            "allowCustomAuthEndpoint": true,
            "allowInsecureAuthEndpoint": true
        });

        let cfg = VoiceConfig::from_settings(Some(&val));

        assert_eq!(cfg.backend, SttBackendKind::Anthropic);
        assert!(!cfg.forward_interims);
        assert!(cfg.allow_custom_auth_endpoint);
        assert!(cfg.allow_insecure_auth_endpoint);
    }
}

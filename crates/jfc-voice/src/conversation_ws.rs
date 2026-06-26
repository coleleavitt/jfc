use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::tts::percent_encode_component;

const DEFAULT_BASE_WSS: &str = "wss://api.anthropic.com";
const DEFAULT_CLIENT_PLATFORM: &str = "web_claude_ai";

#[derive(Debug, Clone, PartialEq)]
pub struct VoiceConversationOptions {
    pub organization_uuid: String,
    pub conversation_uuid: String,
    pub input_encoding: String,
    pub input_sample_rate: u32,
    pub input_channels: u8,
    pub output_format: String,
    pub language: String,
    pub timezone: String,
    pub voice: String,
    pub tts_speed: f32,
    pub server_interrupt_enabled: bool,
    pub client_aec: bool,
    pub client_platform: String,
    pub model: Option<String>,
    pub effort: Option<String>,
    pub thinking_mode: Option<String>,
    pub dev_overrides: Option<serde_json::Value>,
    pub allow_custom_auth_endpoint: bool,
    pub allow_insecure_auth_endpoint: bool,
}

impl VoiceConversationOptions {
    pub fn new(organization_uuid: impl Into<String>, conversation_uuid: impl Into<String>) -> Self {
        Self {
            organization_uuid: organization_uuid.into(),
            conversation_uuid: conversation_uuid.into(),
            input_encoding: "opus".to_owned(),
            input_sample_rate: 16_000,
            input_channels: 1,
            output_format: "pcm_16000".to_owned(),
            language: "en".to_owned(),
            timezone: "UTC".to_owned(),
            voice: "buttery".to_owned(),
            tts_speed: 1.0,
            server_interrupt_enabled: true,
            client_aec: true,
            client_platform: DEFAULT_CLIENT_PLATFORM.to_owned(),
            model: None,
            effort: None,
            thinking_mode: None,
            dev_overrides: None,
            allow_custom_auth_endpoint: false,
            allow_insecure_auth_endpoint: false,
        }
    }

    pub fn clamped_tts_speed(&self) -> f32 {
        self.tts_speed.clamp(0.7, 1.2)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientEvent {
    ToolsRegister { data: ToolsRegisterData },
    ClientMetrics { data: ClientMetrics },
    ClientAbortReason { reason: String },
    Interrupt,
    ManualInputEnd,
    PlaybackComplete,
    ClockSyncPing { seq: u64, t1: u64 },
    KeepAlive,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ToolsRegisterData {
    pub tools: Vec<serde_json::Value>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ClientMetrics {
    pub client_perceived_latency_ms: Option<u64>,
    pub buffer_underrun_count: Option<u64>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ServerEvent {
    SessionServerInitialized,
    TranscriptionStart,
    TranscriptInterim(serde_json::Value),
    UserInputEnd,
    PlaybackStart,
    PlaybackEnd,
    MessageStart(serde_json::Value),
    MessageSse(serde_json::Value),
    MessageComplete(serde_json::Value),
    TtsWord(TtsWordTiming),
    Error(serde_json::Value),
    Other(String, serde_json::Value),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TtsWordTiming {
    pub text: String,
    pub pts_ms: u64,
}

pub fn resolve_base(base: Option<&str>) -> String {
    base.filter(|base| !base.trim().is_empty())
        .unwrap_or(DEFAULT_BASE_WSS)
        .trim_end_matches('/')
        .replacen("https://", "wss://", 1)
        .replacen("http://", "ws://", 1)
}

pub fn build_request(
    base_wss: &str,
    token: &str,
    user_agent: &str,
    opts: &VoiceConversationOptions,
) -> Result<tokio_tungstenite::tungstenite::handshake::client::Request> {
    crate::auth_endpoint::validate_auth_base_url_with_policy(
        base_wss,
        crate::auth_endpoint::AuthEndpointPolicy {
            allow_custom: opts.allow_custom_auth_endpoint,
            allow_insecure: opts.allow_insecure_auth_endpoint,
        },
    )?;
    let mut query = vec![
        ("input_encoding", opts.input_encoding.clone()),
        ("input_sample_rate", opts.input_sample_rate.to_string()),
        ("input_channels", opts.input_channels.to_string()),
        ("output_format", opts.output_format.clone()),
        ("language", opts.language.clone()),
        ("timezone", opts.timezone.clone()),
        ("voice", opts.voice.clone()),
        ("tts_speed", format_speed(opts.clamped_tts_speed())),
        (
            "server_interrupt_enabled",
            opts.server_interrupt_enabled.to_string(),
        ),
        ("client_aec", opts.client_aec.to_string()),
        ("client_platform", opts.client_platform.clone()),
    ];
    push_optional(&mut query, "model", opts.model.as_deref());
    push_optional(&mut query, "effort", opts.effort.as_deref());
    push_optional(&mut query, "thinking_mode", opts.thinking_mode.as_deref());
    let dev_overrides = opts
        .dev_overrides
        .as_ref()
        .map(serde_json::Value::to_string);
    push_optional(&mut query, "dev_overrides", dev_overrides.as_deref());

    let query = query
        .into_iter()
        .map(|(key, value)| format!("{key}={}", percent_encode_component(&value)))
        .collect::<Vec<_>>()
        .join("&");
    let url = format!(
        "{base}/api/ws/voice/organizations/{org}/chat_conversations/{conversation}?{query}",
        base = base_wss.trim_end_matches('/'),
        org = percent_encode_component(&opts.organization_uuid),
        conversation = percent_encode_component(&opts.conversation_uuid),
    );

    use tokio_tungstenite::tungstenite::client::IntoClientRequest;
    let mut req = url
        .as_str()
        .into_client_request()
        .context("invalid voice conversation URL")?;
    {
        let h = req.headers_mut();
        h.insert("Authorization", format!("Bearer {token}").parse()?);
        h.insert("User-Agent", user_agent.parse()?);
        h.insert("x-app", "cli".parse()?);
        h.insert("anthropic-client-platform", "claude_code_cli".parse()?);
    }
    Ok(req)
}

pub fn parse_server_event(raw: &str) -> Result<ServerEvent> {
    let value: serde_json::Value = serde_json::from_str(raw).context("invalid voice event JSON")?;
    let event_type = value
        .get("type")
        .and_then(|event_type| event_type.as_str())
        .unwrap_or("")
        .to_owned();
    let event = match event_type.as_str() {
        "session_server_initialized" => ServerEvent::SessionServerInitialized,
        "transcription_start" => ServerEvent::TranscriptionStart,
        "transcript_interim" => ServerEvent::TranscriptInterim(value),
        "user_input_end" => ServerEvent::UserInputEnd,
        "playback_start" => ServerEvent::PlaybackStart,
        "playback_end" => ServerEvent::PlaybackEnd,
        "message_start" => ServerEvent::MessageStart(value),
        "message_sse" => ServerEvent::MessageSse(value),
        "message_complete" => ServerEvent::MessageComplete(value),
        "tts_word" => ServerEvent::TtsWord(parse_tts_word(&value)),
        "error" => ServerEvent::Error(value),
        _ => ServerEvent::Other(event_type, value),
    };
    Ok(event)
}

fn parse_tts_word(value: &serde_json::Value) -> TtsWordTiming {
    let text = value
        .get("text")
        .and_then(|text| text.as_str())
        .unwrap_or("")
        .to_owned();
    let pts_ms = value
        .get("ptsMs")
        .or_else(|| value.get("pts_ms"))
        .and_then(|pts| pts.as_u64())
        .unwrap_or(0);
    TtsWordTiming { text, pts_ms }
}

fn push_optional(query: &mut Vec<(&'static str, String)>, key: &'static str, value: Option<&str>) {
    if let Some(value) = value.filter(|value| !value.trim().is_empty()) {
        query.push((key, value.to_owned()));
    }
}

fn format_speed(speed: f32) -> String {
    let formatted = format!("{speed:.2}");
    formatted
        .trim_end_matches('0')
        .trim_end_matches('.')
        .to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_request_matches_claude_voice_conversation_endpoint_normal() {
        let mut opts = VoiceConversationOptions::new("org-123", "conv-456");
        opts.tts_speed = 2.0;
        opts.client_aec = true;
        opts.model = Some("claude-sonnet-test".to_owned());
        opts.effort = Some("medium".to_owned());
        let req = build_request("wss://api.anthropic.com", "tok", "jfc", &opts).unwrap();
        let url = req.uri().to_string();

        assert!(url.contains("/api/ws/voice/organizations/org-123/chat_conversations/conv-456?"));
        assert!(url.contains("input_encoding=opus"));
        assert!(url.contains("input_sample_rate=16000"));
        assert!(url.contains("output_format=pcm_16000"));
        assert!(url.contains("client_platform=web_claude_ai"));
        assert!(url.contains("server_interrupt_enabled=true"));
        assert!(url.contains("client_aec=true"));
        assert!(url.contains("tts_speed=1.2"));
        assert!(url.contains("model=claude-sonnet-test"));
        assert!(url.contains("effort=medium"));
        assert_eq!(req.headers()["Authorization"], "Bearer tok");
    }

    #[test]
    fn build_request_rejects_custom_auth_endpoint_without_opt_in_robust() {
        let opts = VoiceConversationOptions::new("org-123", "conv-456");
        let err = build_request("wss://example.invalid", "tok", "jfc", &opts).unwrap_err();

        assert!(err.to_string().contains("non-Anthropic"));
    }

    #[test]
    fn parse_server_event_routes_known_types_normal() {
        assert_eq!(
            parse_server_event(r#"{"type":"session_server_initialized"}"#).unwrap(),
            ServerEvent::SessionServerInitialized
        );
        assert!(matches!(
            parse_server_event(r#"{"type":"tts_word","text":"hello","ptsMs":12}"#).unwrap(),
            ServerEvent::TtsWord(TtsWordTiming { text, pts_ms }) if text == "hello" && pts_ms == 12
        ));
        assert!(matches!(
            parse_server_event(r#"{"type":"message_start","message":{"role":"assistant"}}"#)
                .unwrap(),
            ServerEvent::MessageStart(_)
        ));
        assert!(matches!(
            parse_server_event(r#"{"type":"new_future_event"}"#).unwrap(),
            ServerEvent::Other(kind, _) if kind == "new_future_event"
        ));
    }

    #[test]
    fn client_event_shapes_match_wire_contract_normal() {
        let event = ClientEvent::ToolsRegister {
            data: ToolsRegisterData::default(),
        };
        let json = serde_json::to_value(event).unwrap();

        assert_eq!(json["type"], "tools_register");
        assert_eq!(json["data"]["tools"], serde_json::json!([]));

        let event = ClientEvent::ClockSyncPing { seq: 7, t1: 1234 };
        let json = serde_json::to_value(event).unwrap();
        assert_eq!(json["type"], "clock_sync_ping");
        assert_eq!(json["seq"], 7);
        assert_eq!(json["t1"], 1234);

        let event = ClientEvent::ClientMetrics {
            data: ClientMetrics {
                client_perceived_latency_ms: Some(42),
                buffer_underrun_count: Some(0),
            },
        };
        let json = serde_json::to_value(event).unwrap();
        assert_eq!(json["type"], "client_metrics");
        assert_eq!(json["data"]["client_perceived_latency_ms"], 42);
        assert_eq!(json["data"]["buffer_underrun_count"], 0);
    }
}

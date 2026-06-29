use anyhow::{Context, Result};
use futures_util::{SinkExt, StreamExt};
use std::time::Duration;
use tokio::io::{AsyncWrite, AsyncWriteExt};
use tokio_tungstenite::tungstenite::Message;

use crate::config::VoiceConfig;

const KEEPALIVE_INTERVAL: Duration = Duration::from_millis(4000);
const DEFAULT_BASE_WSS: &str = "wss://api.anthropic.com";
const DEFAULT_CLIENT_PLATFORM: &str = "web_claude_ai";
const CHUNK_MAX_CHARS: usize = 800;

#[derive(Debug, Clone, PartialEq)]
pub struct TtsOptions {
    pub voice: String,
    pub speed: f32,
    pub output_format: String,
    pub client_platform: String,
    pub allow_custom_auth_endpoint: bool,
    pub allow_insecure_auth_endpoint: bool,
}

impl TtsOptions {
    pub fn from_config(cfg: &VoiceConfig) -> Self {
        Self {
            voice: cfg.tts_voice.clone(),
            speed: cfg.clamped_tts_speed(),
            output_format: cfg.tts_output_format.clone(),
            client_platform: DEFAULT_CLIENT_PLATFORM.to_owned(),
            allow_custom_auth_endpoint: cfg.allow_custom_auth_endpoint,
            allow_insecure_auth_endpoint: cfg.allow_insecure_auth_endpoint,
        }
    }
}

impl Default for TtsOptions {
    fn default() -> Self {
        Self {
            voice: "buttery".to_owned(),
            speed: 1.0,
            output_format: "pcm_16000".to_owned(),
            client_platform: DEFAULT_CLIENT_PLATFORM.to_owned(),
            allow_custom_auth_endpoint: false,
            allow_insecure_auth_endpoint: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TtsStats {
    pub audio_bytes: usize,
    pub chunks_sent: usize,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum ServerFrame {
    Complete,
    Error(String),
    Ignore,
}

pub fn resolve_tts_base(cfg: &VoiceConfig) -> String {
    cfg.tts_base_url
        .as_deref()
        .filter(|base| !base.trim().is_empty())
        .unwrap_or(DEFAULT_BASE_WSS)
        .trim_end_matches('/')
        .replacen("https://", "wss://", 1)
        .replacen("http://", "ws://", 1)
}

pub fn build_request(
    base_wss: &str,
    token: &str,
    user_agent: &str,
    opts: &TtsOptions,
) -> Result<tokio_tungstenite::tungstenite::handshake::client::Request> {
    crate::auth_endpoint::validate_auth_base_url_with_policy(
        base_wss,
        crate::auth_endpoint::AuthEndpointPolicy {
            allow_custom: opts.allow_custom_auth_endpoint,
            allow_insecure: opts.allow_insecure_auth_endpoint,
        },
    )?;
    let url = format!(
        "{base}/api/ws/text_to_speech/text_stream?output_format={format}&voice={voice}&tts_speed={speed}&client_platform={platform}",
        base = base_wss.trim_end_matches('/'),
        format = percent_encode_component(&opts.output_format),
        voice = percent_encode_component(&opts.voice),
        speed = format_speed(opts.speed),
        platform = percent_encode_component(&opts.client_platform),
    );

    use tokio_tungstenite::tungstenite::client::IntoClientRequest;
    let mut req = url
        .as_str()
        .into_client_request()
        .context("invalid text_to_speech URL")?;
    {
        let h = req.headers_mut();
        h.insert("Authorization", format!("Bearer {token}").parse()?);
        h.insert("User-Agent", user_agent.parse()?);
        h.insert("x-app", "cli".parse()?);
        h.insert("anthropic-client-platform", "claude_code_cli".parse()?);
    }
    Ok(req)
}

pub async fn synthesize_to_writer<W>(
    cfg: &VoiceConfig,
    token: &str,
    user_agent: &str,
    text: &str,
    writer: &mut W,
) -> Result<TtsStats>
where
    W: AsyncWrite + Unpin,
{
    let cleaned = sanitize_tts_text(text);
    if cleaned.is_empty() {
        return Ok(TtsStats::default());
    }
    let opts = TtsOptions::from_config(cfg);
    let req = build_request(&resolve_tts_base(cfg), token, user_agent, &opts)?;
    let (mut ws, _resp) = tokio_tungstenite::connect_async(req)
        .await
        .context("text_to_speech WebSocket connect/upgrade failed")?;

    let chunks = split_text_chunks(&cleaned);
    let mut stats = TtsStats {
        audio_bytes: 0,
        chunks_sent: chunks.len(),
    };
    for chunk in chunks {
        let msg = serde_json::json!({
            "type": "text_chunk",
            "text": chunk,
        });
        ws.send(Message::Text(msg.to_string())).await?;
    }
    ws.send(Message::Text(
        serde_json::json!({"type": "close_stream"}).to_string(),
    ))
    .await?;

    let mut keepalive = tokio::time::interval(KEEPALIVE_INTERVAL);
    keepalive.tick().await;

    loop {
        tokio::select! {
            _ = keepalive.tick() => {
                let _ = ws.send(Message::Text(
                    serde_json::json!({"type": "keep_alive"}).to_string()
                )).await;
            }
            msg = ws.next() => match msg {
                None | Some(Ok(Message::Close(_))) => break,
                Some(Err(err)) => return Err(err).context("text_to_speech WebSocket read failed"),
                Some(Ok(Message::Binary(bytes))) => {
                    writer.write_all(&bytes).await?;
                    stats.audio_bytes = stats.audio_bytes.saturating_add(bytes.len());
                }
                Some(Ok(Message::Text(raw))) => match parse_server_frame(&raw) {
                    ServerFrame::Complete => break,
                    ServerFrame::Error(desc) => anyhow::bail!("text_to_speech error: {desc}"),
                    ServerFrame::Ignore => {}
                },
                Some(Ok(_)) => {}
            }
        }
    }
    writer.flush().await?;
    let _ = ws.close(None).await;
    Ok(stats)
}

pub fn sanitize_tts_text(text: &str) -> String {
    text.chars()
        .filter_map(|ch| match ch {
            '\n' | '\t' => Some(' '),
            ch if ch.is_control() => None,
            ch => Some(ch),
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

pub fn split_text_chunks(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = text.trim();
    while !rest.is_empty() {
        let split_at = split_index(rest, CHUNK_MAX_CHARS);
        let (chunk, tail) = rest.split_at(split_at);
        let chunk = chunk.trim();
        if !chunk.is_empty() {
            out.push(chunk.to_owned());
        }
        rest = tail.trim_start();
    }
    out
}

fn split_index(text: &str, max_chars: usize) -> usize {
    if text.chars().count() <= max_chars {
        return text.len();
    }

    let mut hard = text.len();
    for (count, (idx, _)) in text.char_indices().enumerate() {
        if count == max_chars {
            hard = idx;
            break;
        }
    }

    let candidate = &text[..hard];
    for boundary in ['\n', '.', '!', '?', ';', ':', ','] {
        if let Some(idx) = candidate.rfind(boundary) {
            let end = idx + boundary.len_utf8();
            if end >= max_chars / 3 {
                return end;
            }
        }
    }
    if let Some(idx) = candidate.rfind(' ')
        && idx >= max_chars / 3
    {
        return idx;
    }
    hard
}

pub(crate) fn parse_server_frame(raw: &str) -> ServerFrame {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(raw) else {
        return ServerFrame::Ignore;
    };
    match v.get("type").and_then(|t| t.as_str()).unwrap_or("") {
        "SpeechComplete" => ServerFrame::Complete,
        "SpeechError" => ServerFrame::Error(
            v.get("description")
                .or_else(|| v.get("message"))
                .or_else(|| v.get("error"))
                .and_then(|d| d.as_str())
                .unwrap_or("unknown speech error")
                .to_owned(),
        ),
        _ => ServerFrame::Ignore,
    }
}

fn format_speed(speed: f32) -> String {
    let speed = speed.clamp(0.7, 1.2);
    let formatted = format!("{speed:.2}");
    formatted
        .trim_end_matches('0')
        .trim_end_matches('.')
        .to_owned()
}

pub(crate) fn percent_encode_component(input: &str) -> String {
    let mut out = String::new();
    for byte in input.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_request_has_claude_web_tts_endpoint_normal() {
        let opts = TtsOptions {
            voice: "buttery".to_owned(),
            speed: 1.333,
            output_format: "pcm_16000".to_owned(),
            client_platform: "web_claude_ai".to_owned(),
            allow_custom_auth_endpoint: false,
            allow_insecure_auth_endpoint: false,
        };
        let req = build_request("wss://api.anthropic.com", "tok", "jfc", &opts).unwrap();
        let url = req.uri().to_string();

        assert!(url.contains("/api/ws/text_to_speech/text_stream"));
        assert!(url.contains("output_format=pcm_16000"));
        assert!(url.contains("voice=buttery"));
        assert!(url.contains("tts_speed=1.2"));
        assert!(url.contains("client_platform=web_claude_ai"));
        assert_eq!(req.headers()["Authorization"], "Bearer tok");
    }

    #[test]
    fn build_request_rejects_custom_auth_endpoint_without_opt_in_robust() {
        let err = build_request(
            "wss://example.invalid",
            "tok",
            "jfc",
            &TtsOptions::default(),
        )
        .unwrap_err();

        assert!(err.to_string().contains("non-Anthropic"));
    }

    #[test]
    fn split_text_chunks_prefers_sentence_boundary_normal() {
        let text = format!("{}.", "a".repeat(500)) + &" b".repeat(500);
        let chunks = split_text_chunks(&text);

        assert!(chunks.len() > 1);
        assert!(
            chunks
                .iter()
                .all(|chunk| chunk.chars().count() <= CHUNK_MAX_CHARS)
        );
        assert!(chunks[0].ends_with('.'));
    }

    #[test]
    fn sanitize_tts_text_strips_controls_regression() {
        assert_eq!(
            sanitize_tts_text("hi\u{0}\nthere\tfriend"),
            "hi there friend"
        );
    }

    #[test]
    fn parse_server_frame_handles_complete_and_error_normal() {
        assert_eq!(
            parse_server_frame(r#"{"type":"SpeechComplete"}"#),
            ServerFrame::Complete
        );
        assert_eq!(
            parse_server_frame(r#"{"type":"SpeechError","description":"bad"}"#),
            ServerFrame::Error("bad".to_owned())
        );
        assert_eq!(parse_server_frame("nope"), ServerFrame::Ignore);
    }
}

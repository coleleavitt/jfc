//! Headless / pipe-mode driver.
//!
//! When `--json` is supplied on the command line, the binary skips
//! TUI bring-up and instead runs as a stdin→stdout JSON-lines bridge:
//!
//! ```text
//! stdin :  {"prompt": "...", "model": "claude-3-7-sonnet"}
//! stdout:  {"role": "assistant", "content": "...", "tool_calls": []}
//! ```
//!
//! One JSON object per line, both directions. Exits when stdin EOF
//! is reached *or* when `--max-turns` is hit. No color, no spinners,
//! no interactive prompts — this is the surface CI scripts wrap.
//!
//! The driver is intentionally minimal: tool dispatch is *not*
//! performed here (tools require permission which is meaningless
//! without an interactive terminal). Callers can still observe
//! `tool_calls` blocks the model emits and dispatch them themselves
//! via subsequent JSON-line turns. Full agentic loops in headless
//! mode live in `cli::headless::run_print_mode`; this module is the
//! generic JSON-lines transport.

use std::sync::Arc;

use anyhow::Context;
use serde::{Deserialize, Serialize};

/// Caller-tunable knobs for [`run_headless`].
#[allow(dead_code)]
#[derive(Debug, Clone, Default)]
pub struct HeadlessConfig {
    /// Stop after this many turns even if stdin still has input.
    /// `None` = run until stdin EOF.
    pub max_turns: Option<u32>,
    /// Override model used for every turn. When `None`, the provider's
    /// first listed model is used.
    pub model: Option<String>,
    /// Max tokens per turn.
    pub max_tokens: Option<u32>,
}

/// One stdin line.
#[derive(Debug, Deserialize)]
struct HeadlessRequest {
    prompt: String,
    #[serde(default)]
    model: Option<String>,
}

/// One stdout line.
#[derive(Debug, Serialize)]
struct HeadlessResponse {
    role: &'static str,
    content: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    tool_calls: Vec<ToolCallRecord>,
    #[serde(skip_serializing_if = "Option::is_none")]
    stop_reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

#[derive(Debug, Serialize)]
struct ToolCallRecord {
    id: String,
    name: String,
    input: serde_json::Value,
}

/// Drive the headless loop until stdin EOF or `max_turns` is reached.
#[allow(dead_code)]
pub async fn run_headless(
    config: HeadlessConfig,
    provider: Arc<dyn jfc_provider::Provider>,
) -> anyhow::Result<()> {
    use std::io::BufRead;

    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout().lock();
    let mut turns: u32 = 0;
    let max = config.max_turns;

    for line in stdin.lock().lines() {
        if let Some(cap) = max
            && turns >= cap
        {
            break;
        }
        let line = match line {
            Ok(l) => l,
            Err(e) => {
                emit_error(&mut stdout, format!("stdin read error: {e}"))?;
                break;
            }
        };
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let req: HeadlessRequest = match serde_json::from_str(trimmed) {
            Ok(r) => r,
            Err(e) => {
                emit_error(&mut stdout, format!("invalid request JSON: {e}"))?;
                continue;
            }
        };
        let model_id = pick_model(&provider, req.model.as_deref().or(config.model.as_deref()))?;
        let response = run_turn(&provider, model_id, req.prompt, config.max_tokens).await;
        emit(&mut stdout, &response)?;
        turns += 1;
    }
    Ok(())
}

fn pick_model(
    provider: &Arc<dyn jfc_provider::Provider>,
    override_name: Option<&str>,
) -> anyhow::Result<jfc_provider::ModelId> {
    let models = provider.available_models();
    if let Some(name) = override_name {
        for m in &models {
            if m.id.as_str() == name {
                return Ok(m.id.clone());
            }
        }
        anyhow::bail!("requested model {name:?} not available on provider {}", provider.name());
    }
    models
        .into_iter()
        .next()
        .map(|m| m.id)
        .context("provider exposes no models")
}

async fn run_turn(
    provider: &Arc<dyn jfc_provider::Provider>,
    model: jfc_provider::ModelId,
    prompt: String,
    max_tokens: Option<u32>,
) -> HeadlessResponse {
    use futures::StreamExt;
    use jfc_provider::{
        ProviderContent, ProviderMessage, ProviderRole, StreamEvent, StreamOptions,
    };

    let messages = vec![ProviderMessage {
        role: ProviderRole::User,
        content: vec![ProviderContent::Text(prompt)],
    }];
    let mut opts = StreamOptions::new(model);
    if let Some(mt) = max_tokens {
        opts = opts.max_tokens(mt);
    } else {
        opts = opts.max_tokens(8192);
    }

    let mut stream = match provider.stream(messages, &opts).await {
        Ok(s) => s,
        Err(e) => return error_response(format!("stream open failed: {e}")),
    };

    let mut text = String::new();
    let mut tool_calls: Vec<ToolCallRecord> = Vec::new();
    let mut stop_reason: Option<String> = None;

    while let Some(event) = stream.next().await {
        match event {
            Ok(StreamEvent::TextDelta { delta, .. }) => text.push_str(&delta),
            Ok(StreamEvent::ToolDone { tool_use_id, tool_name, input_json, .. }) => {
                let input = serde_json::from_str(&input_json)
                    .unwrap_or(serde_json::Value::String(input_json));
                tool_calls.push(ToolCallRecord {
                    id: tool_use_id,
                    name: tool_name,
                    input,
                });
            }
            Ok(StreamEvent::Done { stop_reason: reason, .. }) => {
                stop_reason = Some(format!("{reason:?}"));
                break;
            }
            Ok(_) => {}
            Err(e) => return error_response(format!("stream error: {e}")),
        }
    }

    HeadlessResponse {
        role: "assistant",
        content: text,
        tool_calls,
        stop_reason,
        error: None,
    }
}

fn error_response(msg: String) -> HeadlessResponse {
    HeadlessResponse {
        role: "assistant",
        content: String::new(),
        tool_calls: Vec::new(),
        stop_reason: None,
        error: Some(msg),
    }
}

fn emit(stdout: &mut impl std::io::Write, resp: &HeadlessResponse) -> anyhow::Result<()> {
    let line = serde_json::to_string(resp)?;
    stdout.write_all(line.as_bytes())?;
    stdout.write_all(b"\n")?;
    stdout.flush()?;
    Ok(())
}

fn emit_error(stdout: &mut impl std::io::Write, msg: String) -> anyhow::Result<()> {
    emit(stdout, &error_response(msg))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn response_serialization_includes_role_and_content() {
        let resp = HeadlessResponse {
            role: "assistant",
            content: "hello".into(),
            tool_calls: vec![],
            stop_reason: Some("EndTurn".into()),
            error: None,
        };
        let json = serde_json::to_string(&resp).unwrap();
        assert!(json.contains("\"role\":\"assistant\""));
        assert!(json.contains("\"content\":\"hello\""));
        assert!(json.contains("\"stop_reason\":\"EndTurn\""));
        // tool_calls is skipped when empty.
        assert!(!json.contains("tool_calls"));
        assert!(!json.contains("error"));
    }

    #[test]
    fn error_response_includes_error_field() {
        let resp = error_response("boom".into());
        let json = serde_json::to_string(&resp).unwrap();
        assert!(json.contains("\"error\":\"boom\""));
    }

    #[test]
    fn request_deserialization_accepts_optional_model() {
        let r1: HeadlessRequest = serde_json::from_str(r#"{"prompt":"hi"}"#).unwrap();
        assert_eq!(r1.prompt, "hi");
        assert!(r1.model.is_none());
        let r2: HeadlessRequest =
            serde_json::from_str(r#"{"prompt":"hi","model":"claude-3"}"#).unwrap();
        assert_eq!(r2.model.as_deref(), Some("claude-3"));
    }

    #[test]
    fn config_defaults_to_no_limits() {
        let cfg = HeadlessConfig::default();
        assert!(cfg.max_turns.is_none());
        assert!(cfg.model.is_none());
        assert!(cfg.max_tokens.is_none());
    }
}

use std::{sync::Arc, time::Duration};

use crate::provider::{EventStream, Provider, ProviderMessage, StreamOptions};

const BEDROCK_TRANSIENT_400_RETRIES: u32 = 3;

fn is_bedrock_transient_400(msg: &str) -> bool {
    let m = msg.to_lowercase();
    let bedrock_marker = m.contains("bedrockexception")
        || m.contains("badrequesterror")
        || m.contains("litellm.badrequesterror");
    let transient_shape = m.contains("touse.input is empty")
        || m.contains("tooluse.input is empty")
        || (m.contains("tool_use") && m.contains("empty"))
        || m.contains("tool use concurrency");
    bedrock_marker && transient_shape
}

/// Detect the Anthropic-direct API variant of the tool_use.input
/// validation error: `"messages.N.content.M.tool_use.input: Input should
/// be an object"`. This fires when stale conversation history in a subagent
/// session includes a stringified `input`. The fix is sanitization at the
/// source (ensure_input_object), but existing sessions may carry the bad
/// shape for the remainder of their lifetime -- retrying after sanitization
/// at the edges is the safety net.
fn is_anthropic_tool_input_400(msg: &str) -> bool {
    let m = msg.to_lowercase();
    m.contains("invalid_request_error")
        && m.contains("tool_use.input")
        && m.contains("input should be an object")
}

pub(crate) async fn open_stream_with_bedrock_retries(
    provider: &dyn Provider,
    messages: Arc<Vec<ProviderMessage>>,
    opts: &StreamOptions,
) -> anyhow::Result<EventStream> {
    let mut last_err: Option<anyhow::Error> = None;
    for attempt in 0..=BEDROCK_TRANSIENT_400_RETRIES {
        // Arc::clone is O(1) -- no heap allocation for the messages vec itself.
        // The provider impl receives Vec<ProviderMessage> by value as before.
        match provider.stream((*messages).clone(), opts).await {
            Ok(s) => {
                if attempt > 0 {
                    tracing::info!(
                        target: "jfc::stream::bedrock_retry",
                        attempt,
                        "bedrock transient 400 cleared on retry"
                    );
                }
                return Ok(s);
            }
            Err(e) => {
                let err_str = e.to_string();
                if !is_bedrock_transient_400(&err_str) && !is_anthropic_tool_input_400(&err_str) {
                    return Err(e);
                }
                if attempt == BEDROCK_TRANSIENT_400_RETRIES {
                    last_err = Some(e);
                    break;
                }
                let base_ms = 250u64.saturating_mul(1u64 << attempt);
                let jitter = rand::random::<f64>() * 0.25 + 0.875;
                let delay =
                    Duration::from_millis((base_ms as f64 * jitter).round().max(50.0) as u64);
                tracing::warn!(
                    target: "jfc::stream::bedrock_retry",
                    attempt = attempt + 1,
                    max = BEDROCK_TRANSIENT_400_RETRIES,
                    delay_ms = delay.as_millis() as u64,
                    error = %e,
                    "bedrock transient 400 - silent retry"
                );
                tokio::time::sleep(delay).await;
            }
        }
    }
    Err(last_err.unwrap_or_else(|| anyhow::anyhow!("bedrock transient 400 retries exhausted")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bedrock_transient_400_recognized_normal() {
        let real_msg = r#"OpenWebUI API error 400 Bad Request: Bad request: {"error":{"message":"litellm.BadRequestError: BedrockException - {\"message\":\"The value at messages.15.content.1.toolUse.input is empty.\"}. Received Model Group=bedrock-claude-4-6-opus","type":null,"param":null,"code":"400"}}"#;
        assert!(is_bedrock_transient_400(real_msg));
    }

    #[test]
    fn bedrock_tool_use_concurrency_recognized_robust() {
        let msg = "BadRequestError: tool use concurrency error";
        assert!(is_bedrock_transient_400(msg));
    }

    #[test]
    fn unrelated_400_not_transient_robust() {
        assert!(!is_bedrock_transient_400(
            "OpenWebUI API error 401: Authentication failed"
        ));
        assert!(!is_bedrock_transient_400(
            "BadRequestError: prompt is too long: 210169 tokens > 200000"
        ));
        assert!(!is_bedrock_transient_400("Some random connection error"));
    }

    #[test]
    fn anthropic_tool_input_400_recognized_normal() {
        let real_msg = r#"Anthropic API error 400 Bad Request: Bad request: {"type":"error","error":{"type":"invalid_request_error","message":"messages.303.content.1.tool_use.input: Input should be an object"},"request_id":"req_011Catt7pd1idGGinGrMtMNs"}"#;
        assert!(is_anthropic_tool_input_400(real_msg));
    }

    #[test]
    fn classifiers_dont_cross_match_edge() {
        let bedrock_msg = "BedrockException: toolUse.input is empty";
        let anthropic_msg = r#"invalid_request_error: messages.5.content.0.tool_use.input: Input should be an object"#;
        assert!(!is_anthropic_tool_input_400(bedrock_msg));
        assert!(!is_bedrock_transient_400(anthropic_msg));
    }
}

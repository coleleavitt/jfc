use std::{sync::Arc, time::Duration};

use jfc_provider::{
    EventStream, Provider, ProviderContent, ProviderMessage, ProviderRole, ServerToolResultKind,
    StreamOptions,
};

const BEDROCK_TRANSIENT_400_RETRIES: u32 = 3;
const ADVISOR_HISTORY_400: &str = "Advisor tool result content could not be processed";

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

fn is_advisor_history_400(msg: &str) -> bool {
    msg.contains(ADVISOR_HISTORY_400)
}

fn is_advisor_content(content: &ProviderContent) -> bool {
    match content {
        ProviderContent::ServerToolUse { name, .. } | ProviderContent::ToolUse { name, .. } => {
            name == "advisor"
        }
        ProviderContent::ServerToolResult { tool_kind, .. } => match tool_kind {
            ServerToolResultKind::Advisor => true,
            ServerToolResultKind::Other(wire) => wire == "advisor_tool_result",
            _ => false,
        },
        _ => false,
    }
}

fn advisor_placeholder_needed(content: &[ProviderContent]) -> bool {
    content.is_empty()
        || content.iter().all(|block| match block {
            ProviderContent::Text(text) => text.trim().is_empty(),
            ProviderContent::Thinking { .. } => true,
            ProviderContent::RedactedThinking { .. } => true,
            _ => false,
        })
}

pub fn strip_advisor_blocks(messages: &[ProviderMessage]) -> Vec<ProviderMessage> {
    messages
        .iter()
        .map(|message| {
            if message.role != ProviderRole::Assistant {
                return message.clone();
            }
            let original_len = message.content.len();
            let mut content: Vec<ProviderContent> = message
                .content
                .iter()
                .filter(|block| !is_advisor_content(block))
                .cloned()
                .collect();
            if content.len() != original_len && advisor_placeholder_needed(&content) {
                content = vec![ProviderContent::Text("[Advisor response]".to_owned())];
            }
            ProviderMessage {
                role: message.role,
                content,
            }
        })
        .collect()
}

pub async fn open_stream_with_bedrock_retries(
    provider: &dyn Provider,
    messages: Arc<Vec<ProviderMessage>>,
    opts: &StreamOptions,
) -> anyhow::Result<EventStream> {
    let mut current_messages = messages;
    let mut advisor_strip_used = false;
    let mut attempt = 0;
    loop {
        // Arc::clone is O(1) -- no heap allocation for the messages vec itself.
        // The provider impl receives Vec<ProviderMessage> by value as before.
        match provider.stream((*current_messages).clone(), opts).await {
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
                if opts.advisor_model.is_some()
                    && !advisor_strip_used
                    && is_advisor_history_400(&err_str)
                {
                    let stripped = strip_advisor_blocks(current_messages.as_ref());
                    tracing::warn!(
                        target: "jfc::stream::advisor_retry",
                        original_messages = current_messages.len(),
                        stripped_messages = stripped.len(),
                        error = %err_str,
                        "advisor history rejected - stripping advisor blocks and retrying once"
                    );
                    current_messages = Arc::new(stripped);
                    advisor_strip_used = true;
                    attempt = 0;
                    continue;
                }
                if !is_bedrock_transient_400(&err_str) && !is_anthropic_tool_input_400(&err_str) {
                    return Err(e);
                }
                if attempt == BEDROCK_TRANSIENT_400_RETRIES {
                    return Err(e);
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
                attempt += 1;
            }
        }
    }
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

    #[test]
    fn advisor_history_400_recognized_normal() {
        assert!(is_advisor_history_400(
            "400 invalid_request_error: Advisor tool result content could not be processed"
        ));
    }

    #[test]
    fn strip_advisor_blocks_removes_assistant_server_tool_blocks_normal() {
        let messages = vec![ProviderMessage {
            role: ProviderRole::Assistant,
            content: vec![
                ProviderContent::ServerToolUse {
                    id: "srvtool_1".into(),
                    name: "advisor".into(),
                    input: serde_json::json!({}),
                },
                ProviderContent::ServerToolResult {
                    tool_use_id: "srvtool_1".into(),
                    tool_kind: ServerToolResultKind::Advisor,
                    content: serde_json::json!({"type":"advisor_result","text":"x"}),
                },
            ],
        }];
        let stripped = strip_advisor_blocks(&messages);
        assert_eq!(stripped.len(), 1);
        assert!(matches!(
            stripped[0].content.as_slice(),
            [ProviderContent::Text(text)] if text == "[Advisor response]"
        ));
    }
}

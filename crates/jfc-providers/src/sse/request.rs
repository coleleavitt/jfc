use serde_json::{Value, json};

use jfc_provider::{ProviderContent, ProviderMessage, ProviderRole};

/// Anthropic's Messages API requires `tool_use.input` to be a JSON object.
/// Streamed deltas, Generic ToolInput fallbacks, and round-trip edge cases can
/// produce a `Value::String` (stringified JSON) or `Value::Null`. This helper
/// coerces non-object values into valid objects before the request leaves jfc.
///
/// Mirrors the v137 CLI logic at line 434836:
///   if typeof input === "string" → JSON.parse(input) ?? {}
///   if typeof input !== "object" → throw (we default to {} instead)
pub(crate) fn ensure_input_object(v: &serde_json::Value) -> serde_json::Value {
    match v {
        serde_json::Value::Object(_) => v.clone(),
        serde_json::Value::String(s) => {
            let trimmed = s.trim();
            if trimmed.is_empty() || trimmed == "null" {
                return serde_json::json!({});
            }
            match serde_json::from_str::<serde_json::Value>(trimmed) {
                Ok(serde_json::Value::Object(map)) => serde_json::Value::Object(map),
                Ok(other) => {
                    // Parsed but not an object (e.g., array, number). Wrap it
                    // so the API gets a valid object.
                    serde_json::json!({ "value": other })
                }
                Err(_) => serde_json::json!({}),
            }
        }
        serde_json::Value::Null => serde_json::json!({}),
        // Array/Number/Bool — shouldn't happen but handle defensively.
        other => serde_json::json!({ "value": other }),
    }
}

pub fn build_messages(messages: &[ProviderMessage]) -> Value {
    let tool_use_count = messages
        .iter()
        .flat_map(|m| m.content.iter())
        .filter(|c| matches!(c, ProviderContent::ToolUse { .. }))
        .count();
    let tool_result_count = messages
        .iter()
        .flat_map(|m| m.content.iter())
        .filter(|c| matches!(c, ProviderContent::ToolResult { .. }))
        .count();
    tracing::debug!(
        target: "jfc::provider::sse",
        message_count = messages.len(),
        tool_use_count,
        tool_result_count,
        "build_messages"
    );
    let mut out: Vec<Value> = messages
        .iter()
        .map(|m| {
            let role = match m.role {
                ProviderRole::User => "user",
                ProviderRole::Assistant => "assistant",
            };
            let content: Vec<Value> = m
                .content
                .iter()
                .map(|c| match c {
                    ProviderContent::Text(t) => json!({ "type": "text", "text": t }),
                    ProviderContent::ToolResult {
                        tool_use_id,
                        content,
                        is_error,
                    } => json!({
                        "type": "tool_result",
                        "tool_use_id": tool_use_id,
                        "content": content,
                        "is_error": is_error,
                    }),
                    ProviderContent::ToolUse {
                        id, name, input, ..
                    } => json!({
                        "type": "tool_use",
                        "id": id,
                        "name": name,
                        "input": ensure_input_object(input),
                    }),
                    // Server-side tools round-trip with their original
                    // wire type. Re-emitting them as plain `tool_use`
                    // breaks Anthropic's server-side sampling loop
                    // resumption (cli.js v142:7057, :441090). Anthropic
                    // also accepts `server_tool_use.input` as either a
                    // string OR an object on resend (cli.js v142:441090
                    // tolerates both), so we run the same coercion as
                    // for regular `tool_use` to land on the safe shape.
                    ProviderContent::ServerToolUse { id, name, input } => json!({
                        "type": "server_tool_use",
                        "id": id,
                        "name": name,
                        "input": ensure_input_object(input),
                    }),
                    // Server-side tool results re-emit verbatim with
                    // their original `type` string and content. Per
                    // cli.js v142:441375 these survive the
                    // normalize-for-resend pass unchanged.
                    ProviderContent::ServerToolResult {
                        tool_use_id,
                        tool_kind,
                        content,
                    } => json!({
                        "type": tool_kind.wire_type(),
                        "tool_use_id": tool_use_id,
                        "content": content,
                    }),
                    // Image (PNG/JPEG/GIF/WebP) → `image` block;
                    // PDF → `document` block. Both share the base64
                    // source struct — `to_anthropic_content_block`
                    // owns the type-routing rule.
                    ProviderContent::Attachment(att) => {
                        jfc_provider::content::to_anthropic_content_block(att)
                    }
                    ProviderContent::RedactedThinking { data } => json!({
                        "type": "redacted_thinking",
                        "data": data,
                    }),
                })
                .collect();
            json!({ "role": role, "content": content })
        })
        .collect();

    // Prompt-caching: place cache_control breakpoints on the last content
    // block of the last 2 user messages. This matches cli.js v142's YB5()
    // strategy — everything before the second-to-last user turn is served
    // from cache on subsequent requests.
    let user_indices: Vec<usize> = out
        .iter()
        .enumerate()
        .filter(|(_, m)| m.get("role").and_then(|r| r.as_str()) == Some("user"))
        .map(|(i, _)| i)
        .collect();
    let mut user_breakpoints_set = 0usize;
    for &idx in user_indices.iter().rev().take(2) {
        if let Some(content) = out[idx].get_mut("content").and_then(|c| c.as_array_mut())
            && let Some(last_block) = content.last_mut()
        {
            last_block["cache_control"] = json!({ "type": "ephemeral" });
            user_breakpoints_set += 1;
        }
    }

    // v143 also places a breakpoint on the last assistant message's last
    // non-thinking block. This ensures the prefix up through the last
    // assistant response is cached for the next turn.
    let mut assistant_breakpoint_set = false;
    if let Some(asst_idx) = out
        .iter()
        .enumerate()
        .rev()
        .find(|(_, m)| m.get("role").and_then(|r| r.as_str()) == Some("assistant"))
        .map(|(i, _)| i)
        && let Some(content) = out[asst_idx]
            .get_mut("content")
            .and_then(|c| c.as_array_mut())
    {
        // Find last block that isn't thinking/redacted_thinking
        if let Some(block) = content.iter_mut().rev().find(|b| {
            let ty = b.get("type").and_then(|t| t.as_str()).unwrap_or("");
            ty != "thinking" && ty != "redacted_thinking"
        }) {
            block["cache_control"] = json!({ "type": "ephemeral" });
            assistant_breakpoint_set = true;
        }
    }

    // Diagnostic: if NO breakpoints landed, the request will bypass cache
    // entirely (`cache_read_input_tokens=0`, `cache_creation_input_tokens=0`).
    // For a session at 60k+ tokens that means paying full-prompt input
    // pricing every turn. The signature we observed: post-ESC×2 interrupt,
    // turns [41]/[43]/[45] of ses_20260516_063649 showed in≈200k / read=0
    // / write=0, i.e. cache-control attachment failed for the whole turn.
    // Log loudly enough that a single `rg cache_control` over the log
    // catches it.
    if user_breakpoints_set == 0 && !assistant_breakpoint_set {
        tracing::warn!(
            target: "jfc::provider::cache",
            message_count = out.len(),
            user_message_count = user_indices.len(),
            "no cache_control breakpoints landed — entire prompt will be uncached on this request"
        );
    } else {
        tracing::debug!(
            target: "jfc::provider::cache",
            message_count = out.len(),
            user_breakpoints_set,
            assistant_breakpoint_set,
            "cache_control breakpoints attached"
        );
    }

    out.into()
}

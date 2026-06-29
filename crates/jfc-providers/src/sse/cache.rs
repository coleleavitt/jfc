use serde_json::Value;

pub(crate) fn cap_cache_control_breakpoints(body: &mut Value, max: usize) {
    let mut total = count_cache_control_breakpoints(body);
    if total <= max {
        return;
    }

    let mut removed = 0usize;
    if let Some(messages) = body.get_mut("messages").and_then(|m| m.as_array_mut()) {
        for msg in messages {
            if total <= max {
                break;
            }
            let Some(content) = msg.get_mut("content").and_then(|c| c.as_array_mut()) else {
                continue;
            };
            for block in content {
                if total <= max {
                    break;
                }
                if let Some(obj) = block.as_object_mut()
                    && obj.remove("cache_control").is_some()
                {
                    total -= 1;
                    removed += 1;
                }
            }
        }
    }

    if removed > 0 {
        tracing::debug!(
            target: "jfc::provider::cache",
            removed,
            remaining = total,
            max,
            "trimmed message cache_control breakpoints to provider limit"
        );
    }

    if total > max {
        tracing::warn!(
            target: "jfc::provider::cache",
            remaining = total,
            max,
            "cache_control breakpoint count still exceeds provider limit"
        );
    }
}

pub(crate) fn count_cache_control_breakpoints(value: &Value) -> usize {
    match value {
        Value::Object(map) => {
            usize::from(map.contains_key("cache_control"))
                + map
                    .values()
                    .map(count_cache_control_breakpoints)
                    .sum::<usize>()
        }
        Value::Array(items) => items.iter().map(count_cache_control_breakpoints).sum(),
        _ => 0,
    }
}

pub(crate) fn has_cache_control_ttl(value: &Value) -> bool {
    match value {
        Value::Object(map) => {
            map.get("cache_control")
                .and_then(Value::as_object)
                .and_then(|cache| cache.get("ttl"))
                .is_some()
                || map.values().any(has_cache_control_ttl)
        }
        Value::Array(items) => items.iter().any(has_cache_control_ttl),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn has_cache_control_ttl_detects_nested_ttl_normal() {
        let body = json!({
            "system": [{
                "type": "text",
                "text": "stable",
                "cache_control": { "type": "ephemeral", "ttl": "1h" }
            }]
        });

        assert!(has_cache_control_ttl(&body));
    }

    #[test]
    fn has_cache_control_ttl_ignores_plain_breakpoints_robust() {
        let body = json!({
            "messages": [{
                "role": "user",
                "content": [{
                    "type": "text",
                    "text": "hi",
                    "cache_control": { "type": "ephemeral" }
                }]
            }]
        });

        assert!(!has_cache_control_ttl(&body));
    }
}

const MIN_PLAUSIBLE_LIMIT: usize = 1_024;
const MAX_PLAUSIBLE_LIMIT: usize = 10_000_000;

mod storage;

pub(crate) use storage::{
    load_session_detected_context_limit, persist_session_detected_context_limit,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct DetectedContextLimit {
    pub(crate) actual_tokens: Option<usize>,
    pub(crate) limit_tokens: usize,
}

pub(crate) fn parse_detected_context_limit(message: &str) -> Option<DetectedContextLimit> {
    if message.trim().is_empty() {
        return None;
    }
    let lower = message.to_ascii_lowercase();
    if !looks_like_context_overflow(&lower) {
        return None;
    }
    parse_structured_limit(message, &lower)
        .or_else(|| parse_greater_than_limit(message, &lower))
        .or_else(|| parse_phrase_limit(message, &lower))
}

fn looks_like_context_overflow(lower: &str) -> bool {
    lower.contains("prompt is too long")
        || lower.contains("prompt_too_long")
        || lower.contains("input is too long for requested model")
        || lower.contains("exceeds the context window")
        || (lower.contains("input token count") && lower.contains("exceeds the maximum"))
        || lower.contains("maximum prompt length")
        || lower.contains("reduce the length of the messages")
        || lower.contains("maximum context length")
        || lower.contains("exceeds the limit of")
        || lower.contains("exceeds the available context size")
        || lower.contains("greater than the context length")
        || lower.contains("context window exceeds limit")
        || lower.contains("exceeded model token limit")
        || lower.contains("context_length_exceeded")
        || lower.contains("context length exceeded")
        || lower.contains("request entity too large")
        || lower.contains("request_too_large")
        || (lower.contains("payload too large") && lower.contains("request"))
        || lower.contains("context length is only")
        || (lower.contains("input length")
            && lower.contains("exceeds")
            && lower.contains("context length"))
        || (lower.contains("prompt too long") && lower.contains("context length"))
        || (lower.contains("too large for model with") && lower.contains("maximum context length"))
        || lower.contains("model_context_window_exceeded")
        || lower.contains("context size has been exceeded")
        || lower.contains("context limit")
        || lower.contains("request exceeds the maximum size")
}

fn parse_structured_limit(message: &str, lower: &str) -> Option<DetectedContextLimit> {
    let limit = first_number_after_any(
        message,
        lower,
        &[
            "limittokens",
            "limit_tokens",
            "limit tokens",
            "\"limit\"",
            "context_window",
            "contextwindow",
        ],
    )?;
    let limit_tokens = plausible_limit(limit)?;
    let actual_tokens = first_number_after_any(
        message,
        lower,
        &[
            "actualtokens",
            "actual_tokens",
            "actual tokens",
            "inputtokens",
            "input_tokens",
            "input tokens",
        ],
    );
    Some(DetectedContextLimit {
        actual_tokens,
        limit_tokens,
    })
}

fn parse_greater_than_limit(message: &str, lower: &str) -> Option<DetectedContextLimit> {
    let marker = lower.find('>')?;
    let limit_tokens = plausible_limit(parse_number_at_or_after(message, marker + 1)?)?;
    let actual_tokens = sum_numbers_before_marker(message, marker);
    Some(DetectedContextLimit {
        actual_tokens,
        limit_tokens,
    })
}

fn parse_phrase_limit(message: &str, lower: &str) -> Option<DetectedContextLimit> {
    let limit_tokens = first_number_after_any(
        message,
        lower,
        &[
            "maximum prompt length is",
            "maximum context length is",
            "context length is only",
            "exceeds the limit of",
            "too large for model with",
            "context length of",
            "context size",
            "context limit",
        ],
    )
    .and_then(plausible_limit)?;
    Some(DetectedContextLimit {
        actual_tokens: None,
        limit_tokens,
    })
}

fn first_number_after_any(message: &str, lower: &str, needles: &[&str]) -> Option<usize> {
    needles
        .iter()
        .filter_map(|needle| lower.find(needle).map(|idx| idx + needle.len()))
        .filter_map(|idx| parse_number_at_or_after(message, idx))
        .next()
}

fn parse_number_at_or_after(message: &str, start: usize) -> Option<usize> {
    let bytes = message.as_bytes();
    let mut idx = start.min(bytes.len());
    while idx < bytes.len() && !bytes[idx].is_ascii_digit() {
        idx += 1;
    }
    parse_number_starting_at(message, idx)
}

fn parse_number_starting_at(message: &str, start: usize) -> Option<usize> {
    let mut value = String::new();
    for byte in message.as_bytes().iter().skip(start).copied() {
        if byte.is_ascii_digit() {
            value.push(char::from(byte));
        } else if !value.is_empty() && matches!(byte, b',' | b'_') {
            continue;
        } else if value.is_empty() {
            return None;
        } else {
            break;
        }
    }
    value.parse::<usize>().ok()
}

fn sum_numbers_before_marker(message: &str, marker: usize) -> Option<usize> {
    let prefix = &message[..marker.min(message.len())];
    let mut total = 0usize;
    let mut found = false;
    let mut idx = 0usize;
    while idx < prefix.len() {
        if prefix.as_bytes()[idx].is_ascii_digit() {
            if let Some(value) = parse_number_starting_at(prefix, idx) {
                total = total.saturating_add(value);
                found = true;
            }
            while idx < prefix.len()
                && (prefix.as_bytes()[idx].is_ascii_digit()
                    || matches!(prefix.as_bytes()[idx], b',' | b'_'))
            {
                idx += 1;
            }
        } else {
            idx += 1;
        }
    }
    found.then_some(total)
}

fn plausible_limit(tokens: usize) -> Option<usize> {
    (MIN_PLAUSIBLE_LIMIT..=MAX_PLAUSIBLE_LIMIT)
        .contains(&tokens)
        .then_some(tokens)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_anthropic_structured_limit_regression() {
        let message = r#"auto-compact: Anthropic API error 413 Payload Too Large: raw: {"error":{"type":"request_too_large","message":"Request exceeds the maximum size"},"actualTokens":900000,"limitTokens":200000}"#;

        assert_eq!(
            parse_detected_context_limit(message),
            Some(DetectedContextLimit {
                actual_tokens: Some(900_000),
                limit_tokens: 200_000,
            })
        );
    }

    #[test]
    fn parses_snake_case_structured_limit_normal() {
        let message = "prompt is too long: actual_tokens=350,000 limit_tokens=200,000";

        assert_eq!(
            parse_detected_context_limit(message),
            Some(DetectedContextLimit {
                actual_tokens: Some(350_000),
                limit_tokens: 200_000,
            })
        );
    }

    #[test]
    fn parses_prompt_too_long_greater_than_shape_normal() {
        let message = "prompt is too long: 210000 > 200000";

        assert_eq!(
            parse_detected_context_limit(message),
            Some(DetectedContextLimit {
                actual_tokens: Some(210_000),
                limit_tokens: 200_000,
            })
        );
    }

    #[test]
    fn parses_context_limit_expression_robust() {
        let message = "input length and `max_tokens` exceed context limit: 350000 + 4096 > 200000";

        assert_eq!(
            parse_detected_context_limit(message),
            Some(DetectedContextLimit {
                actual_tokens: Some(354_096),
                limit_tokens: 200_000,
            })
        );
    }

    #[test]
    fn ignores_output_token_cap_error_regression() {
        let message = "invalid request: max_tokens: 128000 > 64000, which is the maximum allowed number of output tokens";

        assert_eq!(parse_detected_context_limit(message), None);
    }
}

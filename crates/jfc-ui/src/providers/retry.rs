//! HTTP retry utilities with exponential backoff and jitter.
//!
//! Implements the same retry strategy as Claude Code's Anthropic SDK:
//! - Retries on: 408 (timeout), 409 (conflict), 429 (rate limit), 500+ (server error)
//! - Backoff: min(0.5 * 2^attempt, 8) * (1 - random*0.25) * 1000ms
//! - Checks `x-should-retry` header override
//! - Configurable max retries (default: 2)

use std::time::Duration;
use tracing::{debug, warn};

/// Configuration for retry behavior.
#[derive(Debug, Clone)]
pub struct RetryConfig {
    /// Maximum number of retry attempts (default: 2).
    pub max_retries: u32,
    /// Base delay multiplier in seconds (default: 0.5).
    pub base_delay_secs: f64,
    /// Maximum delay cap in seconds (default: 8.0).
    pub max_delay_secs: f64,
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            max_retries: 2,
            base_delay_secs: 0.5,
            max_delay_secs: 8.0,
        }
    }
}

impl RetryConfig {
    /// Create a config with more retries (for critical operations).
    pub fn aggressive() -> Self {
        Self {
            max_retries: 5,
            base_delay_secs: 0.5,
            max_delay_secs: 16.0,
        }
    }

    /// Calculate backoff delay for a given attempt number.
    /// Uses exponential backoff with jitter: min(base * 2^attempt, max) * (1 - rand*0.25)
    pub fn delay_for_attempt(&self, attempt: u32) -> Duration {
        let exp = self.base_delay_secs * f64::powi(2.0, attempt as i32);
        let capped = exp.min(self.max_delay_secs);
        let jitter = 1.0 - rand::random::<f64>() * 0.25;
        Duration::from_secs_f64(capped * jitter)
    }
}

/// Whether a response status code should be retried.
pub fn should_retry_status(status: u16, headers: Option<&reqwest::header::HeaderMap>) -> bool {
    // Check x-should-retry header override
    if let Some(hdrs) = headers {
        if let Some(val) = hdrs.get("x-should-retry") {
            if val == "true" {
                return true;
            }
            if val == "false" {
                return false;
            }
        }
    }

    // Match v132's retry policy: 408 (timeout), 409 (conflict), 425
    // (too-early), 429 (rate-limit), and any 5xx including the
    // Cloudflare-specific 520-526 + 529 (overloaded). 413 is *not*
    // retried because it indicates the body itself is the problem;
    // the caller should compact instead.
    matches!(status, 408 | 409 | 425 | 429 | 500..=599)
}

/// Whether an error is a connection/network error worth retrying.
pub fn is_retriable_error(err: &reqwest::Error) -> bool {
    err.is_connect() || err.is_timeout() || err.is_request()
}

/// A provider-level stream error that should restart the same request rather
/// than fail the surrounding turn/task.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetryableStreamError<'a> {
    pub provider: &'static str,
    pub message: &'a str,
}

/// Classify retry sentinels and common transient provider strings emitted
/// after the HTTP request has already opened. The parent stream loop, one-shot
/// subagents, and teammate runners all use this so 429/529/5xx recovery stays
/// consistent instead of each path deciding independently.
pub fn retryable_stream_error(message: &str) -> Option<RetryableStreamError<'_>> {
    if let Some(stripped) = message.strip_prefix(super::anthropic::AUTO_RETRY_SENTINEL) {
        return Some(RetryableStreamError {
            provider: "anthropic",
            message: stripped,
        });
    }
    if let Some(stripped) = message.strip_prefix(super::anthropic_oauth::AUTO_RETRY_SENTINEL) {
        return Some(RetryableStreamError {
            provider: "anthropic-oauth",
            message: stripped,
        });
    }
    if let Some(stripped) = message.strip_prefix(super::openwebui::AUTO_RETRY_SENTINEL) {
        return Some(RetryableStreamError {
            provider: "openwebui",
            message: stripped,
        });
    }

    if is_transient_stream_message(message) {
        Some(RetryableStreamError {
            provider: "provider",
            message,
        })
    } else {
        None
    }
}

pub fn stream_retry_delay(attempt: u32) -> Duration {
    #[cfg(test)]
    {
        let _ = attempt;
        return Duration::from_millis(1);
    }

    #[cfg(not(test))]
    {
        RetryConfig::aggressive().delay_for_attempt(attempt.min(8))
    }
}

fn is_transient_stream_message(message: &str) -> bool {
    let lower = message.to_ascii_lowercase();
    let has_status_context =
        lower.contains("http") || lower.contains("api error") || lower.contains("status");
    lower.contains("overloaded_error")
        || lower.contains("rate_limit_error")
        || lower.contains("rate limited")
        || lower.contains("rate-limited")
        || lower.contains("too many requests")
        || lower.contains("temporarily overloaded")
        || lower.contains("server overloaded")
        || has_status_context
            && retryable_status_code_in_text(&lower)
                .is_some_and(|code| should_retry_status(code, None))
}

fn retryable_status_code_in_text(message: &str) -> Option<u16> {
    message
        .split(|ch: char| !ch.is_ascii_digit())
        .find_map(|part| {
            if part.len() == 3 {
                part.parse::<u16>()
                    .ok()
                    .filter(|code| (100..=599).contains(code))
            } else {
                None
            }
        })
}

/// User-friendly error message for common HTTP errors.
///
/// Coverage matches v2.1.132's `cli.js` error-handling matrix
/// (extracted from `~/VulnerabilityResearch/anthropic/extracted_2.1.132/
/// src/entrypoints/cli.js`): 400 (with prompt-too-long detection),
/// 401, 403, 408, 409, 413, 425, 429, 500, 502, 503, 504, 520-526,
/// 529. Anything outside this set falls through to the generic
/// `HTTP <status>:` form so the user still gets the raw status.
pub fn friendly_error_message(status: u16, body: &str) -> String {
    match status {
        // ── 4xx — client/auth ────────────────────────────────────
        400 if body.contains("prompt is too long")
            || body.contains("ContextWindowExceeded")
            || body.contains("context_length_exceeded") =>
        {
            if let Some(cap) = extract_token_count(body) {
                format!("Context window exceeded ({cap} tokens). Auto-compaction should trigger.")
            } else {
                "Context window exceeded. Auto-compaction should trigger.".to_string()
            }
        }
        400 if body.contains("tool use concurrency") => {
            "API Error: 400 due to tool use concurrency issues — retrying.".to_string()
        }
        400 if body.contains("toolUse.input is empty")
            || (body.contains("BedrockException")
                && body.contains("tool_use")
                && body.contains("empty")) =>
        {
            "Bedrock validator hiccup (empty toolUse.input) — retrying silently.".to_string()
        }
        400 => format!("Bad request: {}", truncate(body, 200)),
        401 => "Authentication failed — check your API key or token (run /login if using OAuth)."
            .to_string(),
        403 => "Access forbidden — your account may not have access to this model.".to_string(),
        408 => "Request timed out (408) — the upstream gave up before the body finished. Retrying."
            .to_string(),
        409 => "Conflict (409) — concurrent state change. Retrying.".to_string(),
        413 => {
            // v132 treats 413 like request_too_large: hint at compaction
            // rather than just dumping "payload too large".
            "Request body too large (413). Auto-compaction should kick in for context-window cases."
                .to_string()
        }
        425 => "Server rejected the request as 'too early' (425). Retrying after a short delay."
            .to_string(),
        429 => {
            if body.contains("rate_limit") {
                "Rate limited — too many requests. Retrying with backoff.".to_string()
            } else {
                "Rate limited — waiting before retry.".to_string()
            }
        }
        // ── 5xx — server/proxy ───────────────────────────────────
        500 => "Internal server error (500) — the provider is having issues.".to_string(),
        502 => "Bad gateway (502) — the provider proxy is unreachable.".to_string(),
        503 => "Service unavailable (503) — the model may be overloaded. Retrying.".to_string(),
        504 => "Gateway timeout (504) — upstream took too long to respond. Retrying.".to_string(),
        // Cloudflare proxy errors. v132 surfaces these as transient
        // and auto-retries; the user almost never needs to react.
        520 => "Cloudflare 520 — origin returned an unknown error. Retrying.".to_string(),
        521 => "Cloudflare 521 — origin web server is down. Retrying.".to_string(),
        522 => "Cloudflare 522 — connection timed out at the edge. Retrying.".to_string(),
        523 => "Cloudflare 523 — origin is unreachable. Retrying.".to_string(),
        524 => "Cloudflare 524 — origin took too long to send the response. Retrying.".to_string(),
        525 => "Cloudflare 525 — TLS handshake failed at the edge. Retrying.".to_string(),
        526 => "Cloudflare 526 — invalid SSL cert at the origin.".to_string(),
        529 => "Provider is overloaded (529). Retrying.".to_string(),
        _ => format!("HTTP {status}: {}", truncate(body, 150)),
    }
}

/// Execute a request with retry logic.
pub async fn with_retry<F, Fut, T>(
    config: &RetryConfig,
    operation_name: &str,
    mut make_request: F,
) -> Result<T, String>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<T, RetryableError>>,
{
    let mut last_error = String::new();

    for attempt in 0..=config.max_retries {
        match make_request().await {
            Ok(val) => return Ok(val),
            Err(RetryableError::Retriable { status, message }) => {
                last_error = message.clone();
                if attempt < config.max_retries {
                    let delay = config.delay_for_attempt(attempt);
                    warn!(
                        target: "jfc::retry",
                        operation = operation_name,
                        attempt = attempt + 1,
                        max = config.max_retries,
                        status,
                        delay_ms = delay.as_millis() as u64,
                        "retriable error — backing off"
                    );
                    tokio::time::sleep(delay).await;
                } else {
                    warn!(
                        target: "jfc::retry",
                        operation = operation_name,
                        status,
                        "max retries exhausted"
                    );
                }
            }
            Err(RetryableError::Fatal(msg)) => {
                debug!(target: "jfc::retry", operation = operation_name, "fatal error — not retrying");
                return Err(msg);
            }
        }
    }

    Err(format!("{operation_name}: {last_error}"))
}

/// Error type for retry logic.
pub enum RetryableError {
    /// Error that can be retried (transient).
    Retriable { status: u16, message: String },
    /// Error that should not be retried (permanent).
    Fatal(String),
}

fn extract_token_count(body: &str) -> Option<String> {
    // "prompt is too long: 210169 tokens > 200000 maximum"
    let start = body.find("prompt is too long: ")?;
    let after = &body[start + 20..];
    let end = after.find(' ')?;
    Some(after[..end].to_string())
}

fn truncate(s: &str, max: usize) -> &str {
    if s.len() <= max { s } else { &s[..max] }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn should_retry_status_codes() {
        assert!(should_retry_status(429, None));
        assert!(should_retry_status(500, None));
        assert!(should_retry_status(503, None));
        assert!(should_retry_status(408, None));
        assert!(!should_retry_status(400, None));
        assert!(!should_retry_status(401, None));
        assert!(!should_retry_status(200, None));
    }

    #[test]
    fn backoff_increases_with_attempts() {
        let config = RetryConfig::default();
        let d0 = config.delay_for_attempt(0);
        let d1 = config.delay_for_attempt(1);
        let d2 = config.delay_for_attempt(2);
        // Each attempt should roughly double (with jitter)
        assert!(d1 > d0);
        assert!(d2 > d1);
        // Cap at 8 seconds
        let d10 = config.delay_for_attempt(10);
        assert!(d10.as_secs_f64() <= 8.5); // 8 + jitter tolerance
    }

    #[test]
    fn friendly_messages() {
        assert!(friendly_error_message(429, "rate_limit").contains("Rate limited"));
        assert!(friendly_error_message(401, "").contains("Authentication"));
        assert!(
            friendly_error_message(400, "prompt is too long: 210169 tokens > 200000")
                .contains("210169")
        );
    }

    /// Coverage check: every status code v132's cli.js explicitly
    /// branches on must produce a status-specific friendly message,
    /// not the generic `HTTP <status>:` fallback. Prevents a quiet
    /// regression where someone deletes a branch and the user starts
    /// seeing raw upstream JSON for an error type we used to handle.
    #[test]
    fn covers_all_v132_status_codes_normal() {
        let v132_codes: &[u16] = &[
            400, 401, 403, 408, 409, 413, 425, 429, 500, 502, 503, 504, 520, 521, 522, 523, 524,
            525, 526, 529,
        ];
        for code in v132_codes {
            let msg = friendly_error_message(*code, "");
            assert!(
                !msg.starts_with(&format!("HTTP {code}")),
                "status {code} should have a tailored message, got: {msg}"
            );
        }
    }

    /// 413 is special: v132 rejects retrying it (it's the body that's
    /// the problem, not the network). Confirm `should_retry_status`
    /// returns false so we don't loop on a forever-too-large request.
    #[test]
    fn should_retry_excludes_413_robust() {
        assert!(!should_retry_status(413, None));
        assert!(!should_retry_status(400, None));
        assert!(!should_retry_status(401, None));
    }

    /// 408 / 425 / Cloudflare 5xx (520-526) / 529 must all retry —
    /// these are exactly the transient cases v132 retries.
    #[test]
    fn retries_v132_transient_codes_normal() {
        for code in [
            408, 425, 429, 500, 502, 503, 504, 520, 521, 522, 523, 524, 525, 526, 529,
        ] {
            assert!(
                should_retry_status(code, None),
                "status {code} should be retried"
            );
        }
    }

    #[test]
    fn retryable_stream_error_strips_provider_sentinels_normal() {
        let message = format!(
            "{}Anthropic transient API error 529: overloaded",
            super::super::anthropic::AUTO_RETRY_SENTINEL
        );
        let signal = retryable_stream_error(&message).expect("sentinel should classify");
        assert_eq!(signal.provider, "anthropic");
        assert_eq!(
            signal.message,
            "Anthropic transient API error 529: overloaded"
        );

        let message = format!(
            "{}OpenWebUI API error 503: unavailable",
            super::super::openwebui::AUTO_RETRY_SENTINEL
        );
        let signal = retryable_stream_error(&message).expect("openwebui sentinel should classify");
        assert_eq!(signal.provider, "openwebui");
    }

    #[test]
    fn retryable_stream_error_recognizes_raw_transients_robust() {
        assert!(retryable_stream_error("HTTP 529 from upstream").is_some());
        assert!(retryable_stream_error("rate_limit_error: slow down").is_some());
        assert!(retryable_stream_error("too many requests").is_some());
        assert!(retryable_stream_error("HTTP 401 unauthorized").is_none());
        assert!(retryable_stream_error("invalid_request_error: bad tool").is_none());
        assert!(retryable_stream_error("prompt is too long: 500 tokens").is_none());
    }
}

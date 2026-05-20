//! Anthropic unified rate-limit + retry header parsing.
//!
//! Claude.ai OAuth sessions return a richer header set than raw API-key
//! requests. Port of `opencode-anthropic-auth/src/plugin/routing.ts` and the
//! header-handling slices of the beautified Claude Code v138 bundle
//! (`/tmp/claude-code-beautified.js` lines 168022–168200 and 388264–388485).
//!
//! ## Headers parsed
//!
//! Standard (always present on 429):
//! - `retry-after`        — int seconds OR HTTP-date
//! - `retry-after-ms`     — float milliseconds (takes precedence)
//!
//! Unified (claude.ai OAuth only):
//! - `anthropic-ratelimit-unified-status`        — `allowed | allowed_warning | rejected`
//! - `anthropic-ratelimit-unified-reset`         — unix seconds (representative claim)
//! - `anthropic-ratelimit-unified-fallback`      — `"available"` ⇒ sonnet fallback is offered
//! - `anthropic-ratelimit-unified-representative-claim`
//!     — `five_hour | seven_day | seven_day_opus | seven_day_sonnet | overage`
//! - `anthropic-ratelimit-unified-overage-status`
//! - `anthropic-ratelimit-unified-overage-reset`
//! - `anthropic-ratelimit-unified-overage-disabled-reason`
//! - `anthropic-ratelimit-unified-5h-utilization` / `-5h-reset`
//! - `anthropic-ratelimit-unified-7d-utilization` / `-7d-reset`
//!
//! Bodies inspected: any response whose body contains `"type":"overloaded_error"`
//! is treated as a 529 even if the status code differs (CC `ZwH()` line 388392).

use std::time::Duration;

use reqwest::header::HeaderMap;

/// Lowest sensible cooldown when a server says "retry after 0".
const MIN_RETRY_AFTER: Duration = Duration::from_secs(1);
/// Hard upper bound on any retry-after we'll honor — clamps malicious or
/// pathological values from the server.
const MAX_RETRY_AFTER: Duration = Duration::from_secs(24 * 60 * 60);

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UnifiedStatus {
    Allowed,
    AllowedWarning,
    Rejected,
}

impl UnifiedStatus {
    fn parse(value: Option<&str>) -> Option<Self> {
        match value? {
            "allowed" => Some(Self::Allowed),
            "allowed_warning" => Some(Self::AllowedWarning),
            "rejected" => Some(Self::Rejected),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClaimType {
    FiveHour,
    SevenDay,
    SevenDayOpus,
    SevenDaySonnet,
    Overage,
    /// Server returned a `representative-claim` we don't yet recognise — kept
    /// verbatim for diagnostics / forward-compat (the docs warn that new claim
    /// types ship occasionally).
    Other(String),
}

impl ClaimType {
    fn parse(value: Option<&str>) -> Option<Self> {
        match value? {
            "five_hour" => Some(Self::FiveHour),
            "seven_day" => Some(Self::SevenDay),
            "seven_day_opus" => Some(Self::SevenDayOpus),
            "seven_day_sonnet" => Some(Self::SevenDaySonnet),
            "overage" => Some(Self::Overage),
            other => Some(Self::Other(other.to_owned())),
        }
    }
}

/// Fully-parsed rate-limit information from a single Anthropic response.
///
/// Every field is optional: a raw API-key request returns none of the unified
/// headers, and even 429s under OAuth occasionally omit some. Callers should
/// fall back to a sensible default cooldown when `retry_after()` is `None`.
#[derive(Debug, Clone, Default)]
pub struct RateLimitInfo {
    /// Combined `retry-after-ms` / `retry-after` parsed value. `None` means
    /// neither header was present (or both were unparseable).
    pub retry_after: Option<Duration>,
    /// `anthropic-ratelimit-unified-status`.
    pub unified_status: Option<UnifiedStatus>,
    /// `anthropic-ratelimit-unified-reset` — unix-ms.
    pub unified_reset_ms: Option<u64>,
    /// `anthropic-ratelimit-unified-fallback == "available"`.
    #[allow(dead_code)]
    pub fallback_available: bool,
    /// `anthropic-ratelimit-unified-representative-claim`.
    pub claim: Option<ClaimType>,
    /// `anthropic-ratelimit-unified-overage-status`.
    pub overage_status: Option<UnifiedStatus>,
    /// `anthropic-ratelimit-unified-overage-reset` — unix-ms.
    pub overage_reset_ms: Option<u64>,
    /// `anthropic-ratelimit-unified-overage-disabled-reason` (verbatim).
    pub overage_disabled_reason: Option<String>,
    /// `true` when the primary claim is rejected but overage is still
    /// servicing the request. Computed identically to opencode's
    /// `runtimeState.isUsingOverage`.
    pub is_using_overage: bool,
    /// `anthropic-ratelimit-unified-5h-utilization` in `[0, 1]`.
    pub utilization_5h: Option<f64>,
    pub utilization_5h_reset_ms: Option<u64>,
    pub utilization_7d: Option<f64>,
    pub utilization_7d_reset_ms: Option<u64>,
}

impl RateLimitInfo {
    /// Whether the response signals that this account should be cooled-down.
    /// True when status is `Rejected` or `retry-after` is set.
    #[allow(dead_code)]
    pub fn is_rate_limited(&self) -> bool {
        matches!(self.unified_status, Some(UnifiedStatus::Rejected)) || self.retry_after.is_some()
    }

    /// Whether the unified payload says a Sonnet fallback is available for
    /// this rejection. Only meaningful when `claim == SevenDayOpus` — at the
    /// call site, gate on both.
    #[allow(dead_code)]
    pub fn opus_fallback_offered(&self) -> bool {
        self.fallback_available && matches!(self.claim, Some(ClaimType::SevenDayOpus))
    }

    /// The recommended cooldown duration for marking the account rate-limited.
    /// Picks (in order): `retry-after*` header, soonest unified reset, soonest
    /// per-claim reset, else `None` (caller applies a default).
    pub fn cooldown_hint(&self, now_ms: u64) -> Option<Duration> {
        if let Some(d) = self.retry_after {
            return Some(clamp_retry(d));
        }
        let candidates = [
            self.unified_reset_ms,
            self.overage_reset_ms,
            self.utilization_5h_reset_ms,
            self.utilization_7d_reset_ms,
        ];
        let soonest = candidates
            .into_iter()
            .flatten()
            .filter(|ms| *ms > now_ms)
            .min()?;
        Some(clamp_retry(Duration::from_millis(soonest - now_ms)))
    }
}

fn clamp_retry(d: Duration) -> Duration {
    d.max(MIN_RETRY_AFTER).min(MAX_RETRY_AFTER)
}

fn header_str<'h>(headers: &'h HeaderMap, name: &str) -> Option<&'h str> {
    headers.get(name)?.to_str().ok()
}

fn parse_f64(value: Option<&str>) -> Option<f64> {
    let raw = value?.trim();
    let parsed: f64 = raw.parse().ok()?;
    parsed.is_finite().then_some(parsed)
}

/// Parse a header that carries a unix-seconds timestamp (possibly fractional).
/// Returns unix-**milliseconds**.
fn parse_unix_seconds_ms(value: Option<&str>) -> Option<u64> {
    let secs = parse_f64(value)?;
    if secs <= 0.0 {
        return None;
    }
    Some((secs * 1000.0) as u64)
}

/// Parse `retry-after` (int seconds OR HTTP-date) and `retry-after-ms`
/// (float milliseconds). `retry-after-ms` takes precedence per the SDK
/// (cli.js:7325). Returns `None` if neither header is present or parseable.
pub fn parse_retry_after(headers: &HeaderMap, now_ms: u64) -> Option<Duration> {
    if let Some(raw) = header_str(headers, "retry-after-ms") {
        if let Ok(ms) = raw.trim().parse::<u64>() {
            return Some(clamp_retry(Duration::from_millis(ms)));
        }
        if let Some(ms_f) = parse_f64(Some(raw)) {
            if ms_f > 0.0 {
                return Some(clamp_retry(Duration::from_millis(ms_f as u64)));
            }
        }
    }
    let raw = header_str(headers, "retry-after")?.trim();
    if let Ok(secs) = raw.parse::<u64>() {
        return Some(clamp_retry(Duration::from_secs(secs)));
    }
    // HTTP-date fallback (RFC 7231 §7.1.3) is intentionally not implemented:
    // Anthropic's API consistently returns integer seconds, and pulling in a
    // date parser for a path we never hit isn't worth the dependency. Caller
    // applies its own fallback when this returns None.
    let _ = now_ms; // kept in signature so callers don't need to plumb time twice
    None
}

/// Parse every relevant rate-limit header off the response into a single
/// struct. Safe to call on 2xx responses too — the unified utilization
/// headers are sometimes set on success to give a warning hint.
pub fn parse_rate_limit_headers(headers: &HeaderMap, now_ms: u64) -> RateLimitInfo {
    let unified_status =
        UnifiedStatus::parse(header_str(headers, "anthropic-ratelimit-unified-status"));
    let unified_reset_ms =
        parse_unix_seconds_ms(header_str(headers, "anthropic-ratelimit-unified-reset"));
    let fallback_available =
        header_str(headers, "anthropic-ratelimit-unified-fallback") == Some("available");
    let claim = ClaimType::parse(header_str(
        headers,
        "anthropic-ratelimit-unified-representative-claim",
    ));
    let overage_status = UnifiedStatus::parse(header_str(
        headers,
        "anthropic-ratelimit-unified-overage-status",
    ));
    let overage_reset_ms = parse_unix_seconds_ms(header_str(
        headers,
        "anthropic-ratelimit-unified-overage-reset",
    ));
    let overage_disabled_reason = header_str(
        headers,
        "anthropic-ratelimit-unified-overage-disabled-reason",
    )
    .filter(|s| !s.is_empty())
    .map(str::to_owned);

    let utilization_5h = parse_f64(header_str(
        headers,
        "anthropic-ratelimit-unified-5h-utilization",
    ));
    let utilization_5h_reset_ms =
        parse_unix_seconds_ms(header_str(headers, "anthropic-ratelimit-unified-5h-reset"));
    let utilization_7d = parse_f64(header_str(
        headers,
        "anthropic-ratelimit-unified-7d-utilization",
    ));
    let utilization_7d_reset_ms =
        parse_unix_seconds_ms(header_str(headers, "anthropic-ratelimit-unified-7d-reset"));

    let is_using_overage = matches!(unified_status, Some(UnifiedStatus::Rejected))
        && matches!(
            overage_status,
            Some(UnifiedStatus::Allowed) | Some(UnifiedStatus::AllowedWarning)
        );

    RateLimitInfo {
        retry_after: parse_retry_after(headers, now_ms),
        unified_status,
        unified_reset_ms,
        fallback_available,
        claim,
        overage_status,
        overage_reset_ms,
        overage_disabled_reason,
        is_using_overage,
        utilization_5h,
        utilization_5h_reset_ms,
        utilization_7d,
        utilization_7d_reset_ms,
    }
}

/// CC v138 `ZwH()` (line 388392): a 529 status OR a body that contains
/// `"type":"overloaded_error"` (covers streaming responses that surface the
/// error after the initial 200 status).
pub fn is_overloaded_error(status: u16, body_preview: &str) -> bool {
    status == 529 || body_preview.contains("\"type\":\"overloaded_error\"")
}

#[cfg(test)]
mod tests {
    use super::*;
    use reqwest::header::{HeaderMap, HeaderValue};

    fn hm(pairs: &[(&str, &str)]) -> HeaderMap {
        let mut h = HeaderMap::new();
        for (k, v) in pairs {
            h.insert(
                reqwest::header::HeaderName::from_bytes(k.as_bytes()).unwrap(),
                HeaderValue::from_str(v).unwrap(),
            );
        }
        h
    }

    // Normal: retry-after-ms takes precedence over retry-after.
    #[test]
    fn retry_after_ms_wins() {
        let h = hm(&[("retry-after", "60"), ("retry-after-ms", "1500")]);
        let d = parse_retry_after(&h, 0).unwrap();
        assert_eq!(d, Duration::from_millis(1500));
    }

    // Normal: plain integer seconds in retry-after parses.
    #[test]
    fn retry_after_seconds() {
        let h = hm(&[("retry-after", "45")]);
        assert_eq!(parse_retry_after(&h, 0).unwrap(), Duration::from_secs(45));
    }

    // Edge: retry-after of "0" gets clamped up to the min (1s).
    #[test]
    fn retry_after_zero_clamped() {
        let h = hm(&[("retry-after", "0")]);
        assert_eq!(parse_retry_after(&h, 0).unwrap(), MIN_RETRY_AFTER);
    }

    // Robust: garbage retry-after returns None (caller falls back).
    #[test]
    fn retry_after_garbage() {
        let h = hm(&[("retry-after", "not-a-number")]);
        assert!(parse_retry_after(&h, 0).is_none());
    }

    // Edge: pathological retry-after capped at MAX_RETRY_AFTER.
    #[test]
    fn retry_after_huge_clamped() {
        let h = hm(&[("retry-after", "9999999999")]);
        assert_eq!(parse_retry_after(&h, 0).unwrap(), MAX_RETRY_AFTER);
    }

    // Normal: full unified header set parses end-to-end.
    #[test]
    fn unified_headers_parse() {
        let h = hm(&[
            ("anthropic-ratelimit-unified-status", "rejected"),
            ("anthropic-ratelimit-unified-reset", "1700000000"),
            ("anthropic-ratelimit-unified-fallback", "available"),
            (
                "anthropic-ratelimit-unified-representative-claim",
                "seven_day_opus",
            ),
            ("anthropic-ratelimit-unified-5h-utilization", "0.95"),
            ("anthropic-ratelimit-unified-7d-utilization", "0.42"),
        ]);
        let info = parse_rate_limit_headers(&h, 0);
        assert_eq!(info.unified_status, Some(UnifiedStatus::Rejected));
        assert_eq!(info.unified_reset_ms, Some(1_700_000_000_000));
        assert!(info.fallback_available);
        assert!(matches!(info.claim, Some(ClaimType::SevenDayOpus)));
        assert!(info.opus_fallback_offered());
        assert_eq!(info.utilization_5h, Some(0.95));
        assert_eq!(info.utilization_7d, Some(0.42));
        assert!(info.is_rate_limited());
    }

    // Edge: rejected primary + allowed_warning overage => is_using_overage.
    #[test]
    fn using_overage_detection() {
        let h = hm(&[
            ("anthropic-ratelimit-unified-status", "rejected"),
            (
                "anthropic-ratelimit-unified-overage-status",
                "allowed_warning",
            ),
        ]);
        let info = parse_rate_limit_headers(&h, 0);
        assert!(info.is_using_overage);
    }

    // Robust: missing headers ⇒ all-None info, not_rate_limited.
    #[test]
    fn empty_headers_safe() {
        let info = parse_rate_limit_headers(&HeaderMap::new(), 0);
        assert!(info.unified_status.is_none());
        assert!(!info.is_rate_limited());
        assert!(!info.opus_fallback_offered());
        assert!(info.cooldown_hint(0).is_none());
    }

    // Normal: cooldown_hint prefers retry-after over reset timestamps.
    #[test]
    fn cooldown_hint_prefers_retry_after() {
        let h = hm(&[
            ("retry-after", "30"),
            ("anthropic-ratelimit-unified-reset", "9999999999"),
        ]);
        let info = parse_rate_limit_headers(&h, 0);
        assert_eq!(info.cooldown_hint(0), Some(Duration::from_secs(30)));
    }

    // Edge: when only reset is set, cooldown_hint computes remaining time.
    #[test]
    fn cooldown_hint_from_reset() {
        let now = 1_000_000_000_000u64;
        let h = hm(&[(
            "anthropic-ratelimit-unified-reset",
            // 60s in the future
            "1000000060",
        )]);
        let info = parse_rate_limit_headers(&h, now);
        let hint = info.cooldown_hint(now).unwrap();
        assert!(hint >= Duration::from_secs(59) && hint <= Duration::from_secs(61));
    }

    // Robust: overloaded_error detection works via body inspection.
    #[test]
    fn overloaded_via_body() {
        assert!(is_overloaded_error(
            200,
            r#"{"type":"overloaded_error","message":"..."}"#
        ));
        assert!(is_overloaded_error(529, ""));
        assert!(!is_overloaded_error(200, "fine"));
        assert!(!is_overloaded_error(429, "rate_limit_error"));
    }

    // Robust: unknown claim type is preserved as Other(_) so we don't crash
    // when Anthropic ships a new tier (defensive: this happened before with
    // seven_day_opus).
    #[test]
    fn unknown_claim_preserved() {
        let h = hm(&[(
            "anthropic-ratelimit-unified-representative-claim",
            "thirty_day_pro",
        )]);
        let info = parse_rate_limit_headers(&h, 0);
        assert!(matches!(info.claim, Some(ClaimType::Other(ref s)) if s == "thirty_day_pro"));
    }
}

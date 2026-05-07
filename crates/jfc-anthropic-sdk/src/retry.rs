//! Retry policy mirroring the Go SDK exactly.
//!
//! Retryable status codes: 408, 409, 425, 429, 500+. Backoff:
//! `min(0.5s * 2^attempt, 8s)` plus `±25%` jitter. Cap at 5 attempts.
//! `Retry-After` and `Retry-After-Ms` headers override the computed delay.

use std::time::Duration;

pub const MAX_ATTEMPTS: u32 = 5;
const BASE_DELAY_MS: u64 = 500;
const MAX_DELAY_MS: u64 = 8_000;

/// Compute the next backoff delay for a given attempt count (zero-based).
/// Adds ±25% jitter to avoid thundering-herd on shared upstreams.
pub fn delay_for(attempt: u32) -> Duration {
    let base = BASE_DELAY_MS.saturating_mul(2u64.saturating_pow(attempt));
    let capped = base.min(MAX_DELAY_MS);
    let jitter = capped / 4;
    let offset: i64 = if jitter == 0 {
        0
    } else {
        (rand_u64() % (jitter * 2)) as i64 - jitter as i64
    };
    let total = (capped as i64).saturating_add(offset).max(0) as u64;
    Duration::from_millis(total)
}

/// Tiny PRNG so this module stays dependency-free. Wraps a thread-local
/// LCG keyed off the current nanosecond. Not cryptographic — fine for
/// jitter.
fn rand_u64() -> u64 {
    use std::cell::Cell;
    use std::time::{SystemTime, UNIX_EPOCH};
    thread_local! {
        static STATE: Cell<u64> = Cell::new(0);
    }
    STATE.with(|s| {
        let mut x = s.get();
        if x == 0 {
            x = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_nanos() as u64)
                .unwrap_or(0xDEAD_BEEF_CAFE_BABE);
            if x == 0 {
                x = 0xC0FFEE;
            }
        }
        // xorshift64
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        s.set(x);
        x
    })
}

/// Parse the `Retry-After-Ms` (preferred) or `Retry-After` (seconds)
/// response header. Returns `None` if neither is present or parseable.
pub fn parse_retry_after(headers: &reqwest::header::HeaderMap) -> Option<Duration> {
    if let Some(v) = headers.get("retry-after-ms") {
        if let Ok(s) = v.to_str() {
            if let Ok(ms) = s.trim().parse::<u64>() {
                return Some(Duration::from_millis(ms));
            }
        }
    }
    if let Some(v) = headers.get("retry-after") {
        if let Ok(s) = v.to_str() {
            if let Ok(secs) = s.trim().parse::<u64>() {
                return Some(Duration::from_secs(secs));
            }
        }
    }
    None
}

/// Should the given status code be retried?
pub fn should_retry_status(code: u16) -> bool {
    matches!(code, 408 | 409 | 425 | 429) || code >= 500
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delay_grows_then_caps_normal() {
        let d0 = delay_for(0);
        let d4 = delay_for(4);
        let d10 = delay_for(10);
        // d0 ≈ 500ms ± 125ms
        assert!(d0.as_millis() >= 375 && d0.as_millis() <= 625, "{:?}", d0);
        // d4 = 8000ms ± 25%, but BASE * 2^4 = 8000 = MAX → capped
        assert!(d4.as_millis() >= 6_000 && d4.as_millis() <= 10_000);
        // d10 should still be capped
        assert!(d10.as_millis() <= MAX_DELAY_MS as u128 * 2);
    }

    #[test]
    fn should_retry_status_normal() {
        assert!(should_retry_status(408));
        assert!(should_retry_status(409));
        assert!(should_retry_status(425));
        assert!(should_retry_status(429));
        assert!(should_retry_status(500));
        assert!(should_retry_status(503));
        assert!(!should_retry_status(200));
        assert!(!should_retry_status(400));
        assert!(!should_retry_status(401));
        assert!(!should_retry_status(404));
    }

    #[test]
    fn parse_retry_after_prefers_ms_normal() {
        let mut h = reqwest::header::HeaderMap::new();
        h.insert("retry-after-ms", "1500".parse().unwrap());
        h.insert("retry-after", "60".parse().unwrap());
        assert_eq!(parse_retry_after(&h), Some(Duration::from_millis(1500)));
    }

    #[test]
    fn parse_retry_after_falls_back_to_seconds_normal() {
        let mut h = reqwest::header::HeaderMap::new();
        h.insert("retry-after", "5".parse().unwrap());
        assert_eq!(parse_retry_after(&h), Some(Duration::from_secs(5)));
    }

    #[test]
    fn parse_retry_after_missing_is_none_robust() {
        let h = reqwest::header::HeaderMap::new();
        assert!(parse_retry_after(&h).is_none());
    }
}

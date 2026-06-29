use std::time::{Duration, Instant};

const HTTP_CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
/// Default inter-chunk read timeout for streaming responses. A 60s value was
/// too aggressive for Bedrock-via-LiteLLM and other proxies that can go silent
/// for 60-90s during long thinking turns or while a large tool call is being
/// assembled. 600s (matching `x-litellm-stream-timeout: 600`) went too far the
/// other way: a genuinely dead stream could sit byte-silent for ~10 minutes
/// before the HTTP layer noticed, freezing the spinner with no recourse.
/// 300s keeps a 3-4x margin over the worst observed proxy quiet period while
/// bounding a hung stream to ~5 minutes. Proxy-heavy users who need the old
/// behavior can restore it via `JFC_STREAM_IDLE_TIMEOUT_MS=600000` (or the
/// Claude Code-compatible `CLAUDE_STREAM_IDLE_TIMEOUT_MS`).
const DEFAULT_STREAM_IDLE_TIMEOUT: Duration = Duration::from_secs(300);
const MIN_STREAM_IDLE_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_STREAM_IDLE_TIMEOUT: Duration = Duration::from_secs(30 * 60);
const HTTP_POOL_IDLE_TIMEOUT: Duration = Duration::from_secs(90);
const HTTP_TCP_KEEPALIVE: Duration = Duration::from_secs(30);
const HTTP_TCP_KEEPALIVE_INTERVAL: Duration = Duration::from_secs(15);
const HTTP_TCP_KEEPALIVE_RETRIES: u32 = 3;
const HTTP2_KEEPALIVE_INTERVAL: Duration = Duration::from_secs(30);
const HTTP2_KEEPALIVE_TIMEOUT: Duration = Duration::from_secs(10);

pub fn streaming_client() -> reqwest::Client {
    linkscope::record_items("http.client.build", 1);
    reqwest::Client::builder()
        .connect_timeout(HTTP_CONNECT_TIMEOUT)
        // Streaming responses have no known total duration. `read_timeout`
        // catches stalled reads without imposing a hard deadline on the body.
        .read_timeout(byte_stream_idle_timeout())
        .pool_idle_timeout(HTTP_POOL_IDLE_TIMEOUT)
        // CC 2.1.167 uses maxSockets:50 per host. reqwest defaults to no
        // hard per-host cap (unlimited) but having an explicit value makes
        // the connection budget visible and matches CC's tuning for the
        // Anthropic endpoint. Parallel tool calls and streaming can open
        // several connections simultaneously.
        .pool_max_idle_per_host(50)
        // TCP-level keepalive helps the kernel notice half-open
        // sockets through NAT/LB rewrites. Keep the interval/retry
        // values explicit so upgrades don't silently change long-stream
        // behavior.
        .tcp_keepalive(HTTP_TCP_KEEPALIVE)
        .tcp_keepalive_interval(HTTP_TCP_KEEPALIVE_INTERVAL)
        .tcp_keepalive_retries(HTTP_TCP_KEEPALIVE_RETRIES)
        // reqwest's default feature set enables HTTP/2, but this
        // workspace disables default features. Re-enable it explicitly
        // and use h2's adaptive flow-control + pings for long SSE
        // streams through proxies that support ALPN.
        .http2_adaptive_window(true)
        .http2_keep_alive_interval(HTTP2_KEEPALIVE_INTERVAL)
        .http2_keep_alive_timeout(HTTP2_KEEPALIVE_TIMEOUT)
        .http2_keep_alive_while_idle(true)
        .build()
        .expect("provider HTTP client configuration is valid")
}

/// Main stream idle timeout. `JFC_STREAM_IDLE_TIMEOUT_MS` is the native name;
/// `CLAUDE_STREAM_IDLE_TIMEOUT_MS` is accepted so users can reuse existing
/// Claude Code tuning.
pub fn stream_idle_timeout() -> Duration {
    timeout_from_env(
        &[
            "JFC_STREAM_IDLE_TIMEOUT_MS",
            "CLAUDE_STREAM_IDLE_TIMEOUT_MS",
        ],
        DEFAULT_STREAM_IDLE_TIMEOUT,
    )
}

/// Lower-level byte-stream idle timeout. Mirrors Claude Code 2.1.157's split:
/// when a byte-specific override is present it wins, otherwise the general
/// stream timeout applies.
pub fn byte_stream_idle_timeout() -> Duration {
    timeout_from_env(
        &[
            "JFC_BYTE_STREAM_IDLE_TIMEOUT_MS",
            "CLAUDE_BYTE_STREAM_IDLE_TIMEOUT_MS",
        ],
        stream_idle_timeout(),
    )
}

fn timeout_from_env(keys: &[&str], default: Duration) -> Duration {
    let configured = keys
        .iter()
        .filter_map(|key| std::env::var(key).ok())
        .find_map(|value| parse_timeout_ms(Some(value.as_str())));
    clamp_stream_timeout(configured.unwrap_or(default))
}

fn parse_timeout_ms(value: Option<&str>) -> Option<Duration> {
    let millis = value?.trim().parse::<u64>().ok()?;
    (millis > 0).then(|| Duration::from_millis(millis))
}

fn clamp_stream_timeout(timeout: Duration) -> Duration {
    timeout.clamp(MIN_STREAM_IDLE_TIMEOUT, MAX_STREAM_IDLE_TIMEOUT)
}

/// Send an HTTP request with automatic retry on transient failures.
/// Each attempt invokes `build` to construct a fresh `RequestBuilder`
/// (so the body and headers are re-serialized) and awaits its `.send()`
/// future. Retries on connection-level failures as well as transient
/// HTTP statuses like 408/409/425/429/5xx.
///
/// This addresses the `error sending request for url (…)` failures
/// users hit on flaky networks or load-balanced proxies (e.g.
/// genai.arizona.edu): a single transient TCP RST or DNS hiccup no
/// longer aborts the whole turn.
pub async fn send_with_retry<F, Fut>(
    operation: &str,
    mut build: F,
) -> reqwest::Result<reqwest::Response>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = reqwest::Result<reqwest::Response>>,
{
    let _linkscope_http = linkscope::phase("http.send_with_retry");
    if linkscope::is_enabled() {
        linkscope::event_fields(
            "http.send.start",
            [linkscope::TraceField::text("operation", operation)],
        );
    }
    let config = crate::retry::RetryConfig::default();
    let mut last_err: Option<reqwest::Error> = None;
    for attempt in 0..=config.max_retries {
        linkscope::record_items("http.send.attempt", 1);
        match build().await {
            Ok(resp) => {
                let status = resp.status();
                if linkscope::is_enabled() {
                    linkscope::detail_event_fields(
                        "http.send.response",
                        [
                            linkscope::TraceField::text("operation", operation),
                            linkscope::TraceField::count("attempt", u64::from(attempt + 1)),
                            linkscope::TraceField::count("status", u64::from(status.as_u16())),
                        ],
                    );
                }
                if crate::retry::should_retry_status(status.as_u16(), Some(resp.headers()))
                    && attempt < config.max_retries
                {
                    linkscope::record_items("http.send.retry_status", 1);
                    let delay = config.delay_for_attempt(attempt);
                    tracing::warn!(
                        target: "jfc::http::retry",
                        operation = operation,
                        attempt = attempt + 1,
                        max = config.max_retries + 1,
                        status = status.as_u16(),
                        delay_ms = delay.as_millis() as u64,
                        "retrying after transient HTTP status"
                    );
                    tokio::time::sleep(delay).await;
                    continue;
                }
                if attempt > 0 {
                    linkscope::record_items("http.send.succeeded_after_retry", 1);
                    tracing::info!(
                        target: "jfc::http::retry",
                        operation = operation,
                        attempt = attempt + 1,
                        "succeeded after retry"
                    );
                }
                return Ok(resp);
            }
            Err(e) => {
                let retriable = crate::retry::is_retriable_error(&e);
                if retriable && attempt < config.max_retries {
                    linkscope::record_items("http.send.retry_error", 1);
                    if linkscope::is_enabled() {
                        linkscope::detail_event_fields(
                            "http.send.error",
                            [
                                linkscope::TraceField::text("operation", operation),
                                linkscope::TraceField::count("attempt", u64::from(attempt + 1)),
                                linkscope::TraceField::text("cause", classify_send_error(&e)),
                            ],
                        );
                    }
                    let delay = config.delay_for_attempt(attempt);
                    tracing::warn!(
                        target: "jfc::http::retry",
                        operation = operation,
                        attempt = attempt + 1,
                        max = config.max_retries + 1,
                        cause = classify_send_error(&e),
                        delay_ms = delay.as_millis() as u64,
                        error = %e,
                        "retrying after connection error"
                    );
                    last_err = Some(e);
                    tokio::time::sleep(delay).await;
                    continue;
                }
                if retriable {
                    linkscope::record_items("http.send.exhausted", 1);
                    tracing::warn!(
                        target: "jfc::http::retry",
                        operation = operation,
                        cause = classify_send_error(&e),
                        error = %e,
                        "exhausted retries"
                    );
                }
                return Err(e);
            }
        }
    }
    Err(last_err.expect("loop must produce an error to reach this point"))
}

/// Threshold past which a streaming send's first byte is considered
/// "slow" — fires a tracing warning so the user can see why their
/// turn feels stuck. Mirrors v132's `tengu_api_slow_first_byte`
/// telemetry. 5s is conservative — most streams begin within 2s
/// even from cold-start Bedrock; 5s+ usually means a proxy
/// queueing problem.
pub const SLOW_FIRST_BYTE_MS: u128 = 5_000;

/// Walk-clock time the request stayed open before bytes arrived.
/// Callers compute this around `client.send().await` and call
/// `report_first_byte_latency` to surface the warning.
pub fn report_first_byte_latency(operation: &str, elapsed: std::time::Duration) {
    let ms = elapsed.as_millis();
    if linkscope::is_enabled() {
        linkscope::event_fields(
            "http.first_byte",
            [
                linkscope::TraceField::text("operation", operation),
                linkscope::TraceField::count("elapsed_ms", u128_to_u64_saturating(ms)),
            ],
        );
    }
    if ms >= SLOW_FIRST_BYTE_MS {
        linkscope::record_items("http.first_byte.slow", 1);
        tracing::warn!(
            target: "jfc::http::slow_first_byte",
            operation = operation,
            elapsed_ms = ms as u64,
            threshold_ms = SLOW_FIRST_BYTE_MS as u64,
            "first byte was slow — upstream proxy queueing or model cold-start"
        );
    } else {
        linkscope::record_items("http.first_byte.ok", 1);
        tracing::debug!(
            target: "jfc::http::first_byte",
            operation = operation,
            elapsed_ms = ms as u64,
            "first byte received"
        );
    }
}

/// Issue a cheap HEAD request to `url` using `client` purely to prime
/// DNS + TCP + TLS into the idle connection pool.
///
/// The request intentionally carries no auth headers and expects to get
/// back any HTTP response (even 4xx/5xx), or time out, or be refused —
/// all of those outcomes still establish the TLS session and warm the
/// DNS/TCP state in the OS and in reqwest's pool. Errors are silently
/// swallowed: warmup is best-effort and must never affect startup.
///
/// Call [`spawn_connect_warmup`] instead of this directly — that helper
/// applies the env-var opt-out and fire-and-forgets the task.
pub async fn connect_warmup(url: String, client: reqwest::Client) {
    let _linkscope_warmup = linkscope::phase("http.connect_warmup");
    let t0 = Instant::now();
    let outcome = client
        .head(&url)
        .timeout(Duration::from_secs(5))
        .send()
        .await;
    let elapsed = t0.elapsed();
    match outcome {
        Ok(resp) => {
            linkscope::record_items("http.connect_warmup.ok", 1);
            tracing::debug!(
                target: "jfc::http::warmup",
                url = %url,
                status = resp.status().as_u16(),
                elapsed_ms = elapsed.as_millis() as u64,
                "TLS/DNS preconnect warmup completed — connection pool primed"
            );
        }
        Err(e) => {
            linkscope::record_items("http.connect_warmup.error", 1);
            tracing::debug!(
                target: "jfc::http::warmup",
                url = %url,
                elapsed_ms = elapsed.as_millis() as u64,
                error = %e,
                "TLS/DNS preconnect warmup failed (best-effort, ignored)"
            );
        }
    }
}

/// Fire-and-forget TLS/DNS preconnect warmup for the active provider.
///
/// Spawns a background task that issues a lightweight HEAD request to
/// the provider's base origin so that DNS resolution, TCP handshake, and
/// TLS negotiation are complete before the user's first turn arrives.
/// The warmed connection lands in the provider's idle pool and is reused
/// by the first real streaming request, eliminating that latency from the
/// perceived TTFT.
///
/// Guards:
/// - `JFC_DISABLE_CONNECT_WARMUP=1` (or `true`/`yes`/`on`) opts out entirely.
/// - Providers that return `None` from `Provider::warmup_url()` or
///   `Provider::http_client()` are silently skipped.
/// - The spawned task is detached — completion/failure never surfaces to the
///   caller.
pub fn spawn_connect_warmup(provider: &dyn crate::Provider) {
    // Env-var opt-out.
    if std::env::var("JFC_DISABLE_CONNECT_WARMUP")
        .map(|v| matches!(v.as_str(), "1" | "true" | "yes" | "on"))
        .unwrap_or(false)
    {
        tracing::debug!(
            target: "jfc::http::warmup",
            "JFC_DISABLE_CONNECT_WARMUP set — skipping TLS preconnect warmup"
        );
        return;
    }

    let Some(url) = provider.warmup_url() else {
        tracing::debug!(
            target: "jfc::http::warmup",
            provider = provider.name(),
            "provider has no warmup_url — skipping TLS preconnect warmup"
        );
        return;
    };

    let Some(client) = provider.http_client() else {
        tracing::debug!(
            target: "jfc::http::warmup",
            provider = provider.name(),
            "provider has no http_client — skipping TLS preconnect warmup"
        );
        return;
    };

    tracing::debug!(
        target: "jfc::http::warmup",
        provider = provider.name(),
        url = %url,
        "spawning TLS/DNS preconnect warmup task"
    );
    tokio::spawn(connect_warmup(url, client));
}

fn u128_to_u64_saturating(value: u128) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
}

/// Translate a `reqwest::Error` from a `.send()` call into a
/// user-visible string. The default `Display` impl produces
/// "error sending request for url (…)" which tells the user
/// nothing about *why* the send failed — was it DNS? TLS? a
/// stalled body? This helper drills into the error kind and
/// returns a human-readable cause, then preserves the chain via
/// anyhow when wrapped at the call site.
pub fn classify_send_error(err: &reqwest::Error) -> &'static str {
    if err.is_timeout() {
        "request timed out — check your internet connection or proxy settings"
    } else if err.is_connect() {
        "connection failed — DNS resolution, TLS handshake, or refused connection"
    } else if err.is_request() {
        "request failed before the server responded — possible mid-stream disconnect or upstream timeout"
    } else if err.is_decode() {
        "response could not be decoded — upstream returned malformed bytes"
    } else if err.is_body() {
        "response body stream ended unexpectedly — upstream closed the connection"
    } else if err.is_status() {
        "HTTP error status — see logs for the response body"
    } else {
        "unspecified HTTP error — see logs for details"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn synthetic_response(status: u16) -> reqwest::Response {
        use reqwest::ResponseBuilderExt;

        http::Response::builder()
            .status(status)
            .url(reqwest::Url::parse("http://example.test/").unwrap())
            .body("")
            .unwrap()
            .into()
    }

    // Normal: streaming_client construction never panics. The
    // `.expect()` inside is intentional — if we ever introduce an
    // invalid combination of timeouts we want the test suite to
    // catch it, not the user.
    #[test]
    fn streaming_client_builds_without_panic_normal() {
        let _ = streaming_client();
    }

    #[test]
    fn stream_timeout_parser_ignores_missing_or_zero_robust() {
        assert_eq!(parse_timeout_ms(None), None);
        assert_eq!(parse_timeout_ms(Some("")), None);
        assert_eq!(parse_timeout_ms(Some("0")), None);
        assert_eq!(
            parse_timeout_ms(Some("2500")),
            Some(Duration::from_millis(2500))
        );
    }

    #[test]
    fn stream_timeout_clamps_to_supported_range_normal() {
        assert_eq!(
            clamp_stream_timeout(Duration::from_millis(1)),
            MIN_STREAM_IDLE_TIMEOUT
        );
        assert_eq!(
            clamp_stream_timeout(Duration::from_secs(60 * 60)),
            MAX_STREAM_IDLE_TIMEOUT
        );
        assert_eq!(
            clamp_stream_timeout(Duration::from_secs(600)),
            Duration::from_secs(600)
        );
    }

    // Robust: classify_send_error returns *something* non-empty for
    // every reqwest::Error variant we can synthesize. We can't easily
    // construct internal reqwest::Error instances directly, so use
    // a real failed connection to a non-routable address.
    #[tokio::test(flavor = "current_thread")]
    async fn classify_send_error_returns_message_robust() {
        let client = reqwest::Client::builder()
            .connect_timeout(Duration::from_millis(50))
            .build()
            .unwrap();
        // 192.0.2.0/24 is TEST-NET-1 — guaranteed unreachable.
        let res = client.get("http://192.0.2.1:9999/").send().await;
        let err = res.expect_err("should fail to connect");
        let msg = classify_send_error(&err);
        assert!(!msg.is_empty(), "classification must be non-empty");
    }

    // Normal: send_with_retry calls the closure multiple times when
    // the request keeps failing with a retriable error. Default
    // RetryConfig sets max_retries=2, so we expect 3 total attempts
    // (initial + 2 retries) before giving up.
    #[tokio::test(flavor = "current_thread")]
    async fn send_with_retry_attempts_count_robust() {
        use std::sync::atomic::{AtomicU32, Ordering};
        let client = reqwest::Client::builder()
            .connect_timeout(Duration::from_millis(30))
            .build()
            .unwrap();
        let attempts = std::sync::Arc::new(AtomicU32::new(0));
        let attempts_c = attempts.clone();
        let res = send_with_retry("test.unreachable", || {
            attempts_c.fetch_add(1, Ordering::SeqCst);
            client.get("http://192.0.2.1:9999/").send()
        })
        .await;
        assert!(res.is_err(), "TEST-NET-1 must remain unreachable");
        let n = attempts.load(Ordering::SeqCst);
        // Default config: 2 retries → 3 attempts total. Allow exactly
        // 3 (no off-by-ones from the for loop boundary).
        assert_eq!(n, 3, "expected 3 attempts, got {n}");
    }

    // Normal: send_with_retry returns success without retrying when
    // the first attempt succeeds. Pin attempt count to exactly 1.
    #[tokio::test(flavor = "current_thread")]
    async fn send_with_retry_success_first_try_normal() {
        use std::sync::atomic::{AtomicU32, Ordering};

        let attempts = std::sync::Arc::new(AtomicU32::new(0));
        let attempts_c = attempts.clone();
        let res = send_with_retry("test.success", || {
            attempts_c.fetch_add(1, Ordering::SeqCst);
            async { Ok(synthetic_response(200)) }
        })
        .await;
        assert!(res.is_ok(), "happy path should succeed");
        assert_eq!(attempts.load(Ordering::SeqCst), 1, "no retry on success");
    }

    // Normal: connect_warmup completes without panicking even when given an
    // unreachable address (TEST-NET-1, guaranteed to fail). The function must
    // swallow the error and return normally — warmup failures must never
    // propagate to the caller.
    #[tokio::test(flavor = "current_thread")]
    async fn connect_warmup_swallows_error_on_unreachable_normal() {
        let client = reqwest::Client::builder()
            .connect_timeout(Duration::from_millis(50))
            .build()
            .unwrap();
        // Should complete without panicking, regardless of outcome.
        connect_warmup("http://192.0.2.1:9999/".to_owned(), client).await;
    }

    // Normal: transient HTTP statuses should be retried before the
    // final response is returned to the provider.
    #[tokio::test(flavor = "current_thread")]
    async fn send_with_retry_retries_504_before_success_normal() {
        use std::sync::atomic::{AtomicU32, Ordering};

        let attempts = std::sync::Arc::new(AtomicU32::new(0));
        let attempts_c = attempts.clone();

        let res = send_with_retry("test.status_retry", || {
            let attempts = attempts_c.clone();
            async move {
                let n = attempts.fetch_add(1, Ordering::SeqCst);
                Ok(synthetic_response(if n == 0 { 504 } else { 200 }))
            }
        })
        .await
        .expect("request should succeed after retry");

        assert_eq!(res.status(), reqwest::StatusCode::OK);
        assert_eq!(
            attempts.load(Ordering::SeqCst),
            2,
            "expected one retry after 504"
        );
    }
}

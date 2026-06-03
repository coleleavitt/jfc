//! Retry rate limiting, ported from Kubernetes `client-go`
//! `util/workqueue/default_rate_limiters.go`.
//!
//! The controller pattern there is `MaxOf(per-item exponential backoff, overall
//! token bucket)`: each failing item backs off on its *own* exponential curve
//! (`base * 2^failures`, capped), while a *shared* token bucket caps the
//! aggregate requeue rate so a storm of distinct failing items can't hammer the
//! system. The effective delay is the larger of the two — whichever discipline
//! is currently the tighter constraint wins.
//!
//! Items carry a retry count (`num_requeues`) and are cleared with `forget` on
//! success or permanent failure — the same `AddRateLimited` / `Forget` /
//! `NumRequeues` contract k8s controllers rely on. jfc uses this for the
//! subagent + bounty retry layer, which previously had only a single global
//! budget (`TokenLedger`) and no per-item retry discipline.
//!
//! ## Clock
//!
//! The token bucket is driven by an explicit logical clock (`now_secs`,
//! monotonic seconds) so its behaviour is fully deterministic and testable
//! without sleeping or reading the wall clock. Production callers use
//! [`RetryRateLimiter::when`], which reads an internal monotonic origin;
//! tests use [`RetryRateLimiter::when_at`] with synthetic timestamps.

use std::collections::HashMap;
use std::hash::Hash;
use std::time::{Duration, Instant};

/// Per-item exponential backoff: the `i`-th requeue of an item waits
/// `base * 2^i`, saturating at `max`. Mirrors k8s
/// `ItemExponentialFailureRateLimiter`.
#[derive(Debug, Clone)]
pub struct ItemExponentialBackoff<K> {
    base: Duration,
    max: Duration,
    failures: HashMap<K, u32>,
}

impl<K: Eq + Hash + Clone> ItemExponentialBackoff<K> {
    pub fn new(base: Duration, max: Duration) -> Self {
        Self {
            base,
            max,
            failures: HashMap::new(),
        }
    }

    /// Record a requeue for `item` and return its backoff delay. The first
    /// requeue waits `base`, the second `2*base`, the third `4*base`, … capped
    /// at `max`. Matches k8s: the failure count is read, *then* incremented, so
    /// the first call returns `base * 2^0 = base`.
    pub fn when(&mut self, item: &K) -> Duration {
        let exp = self.failures.entry(item.clone()).or_insert(0);
        let cur = *exp;
        *exp = exp.saturating_add(1);
        backoff_for(self.base, self.max, cur)
    }

    /// Number of times `item` has been requeued so far.
    pub fn num_requeues(&self, item: &K) -> u32 {
        self.failures.get(item).copied().unwrap_or(0)
    }

    /// Drop `item`'s backoff state — call on success or permanent failure so a
    /// later reuse of the same key starts fresh.
    pub fn forget(&mut self, item: &K) {
        self.failures.remove(item);
    }
}

/// `base * 2^exp`, saturating to `max` on overflow or when it exceeds `max`.
/// Pulled out so it can be unit-tested in isolation and shared by callers that
/// only have an attempt count (e.g. a task's `attempt_count`).
pub fn backoff_for(base: Duration, max: Duration, exp: u32) -> Duration {
    let base_nanos = base.as_nanos() as f64;
    // 2^exp grows fast; f64 keeps it finite and lets the cap catch it.
    let scaled = base_nanos * 2f64.powi(exp as i32);
    if !scaled.is_finite() || scaled >= max.as_nanos() as f64 {
        return max;
    }
    Duration::from_nanos(scaled as u64).min(max)
}

/// Continuous token bucket (the `golang.org/x/time/rate` model k8s composes
/// in): tokens refill at `qps` up to a `burst` ceiling. A reservation that
/// can't be satisfied immediately drives the token count negative and returns
/// the wait until the bucket recovers — so callers are spaced at `1/qps`
/// rather than rejected.
#[derive(Debug, Clone)]
pub struct TokenBucket {
    qps: f64,
    burst: f64,
    tokens: f64,
    last_secs: f64,
}

impl TokenBucket {
    /// `qps` tokens per second, up to `burst` accumulated. Starts full.
    pub fn new(qps: f64, burst: u32) -> Self {
        Self {
            qps,
            burst: burst as f64,
            tokens: burst as f64,
            last_secs: 0.0,
        }
    }

    /// Reserve one token as of logical time `now_secs` (monotonic seconds) and
    /// return how long the caller must wait before acting. Refills lazily based
    /// on elapsed time, clamped to `burst`.
    pub fn reserve(&mut self, now_secs: f64) -> Duration {
        // Refill for elapsed time (guard against a non-monotonic clock).
        if now_secs > self.last_secs {
            self.tokens = (self.tokens + (now_secs - self.last_secs) * self.qps).min(self.burst);
            self.last_secs = now_secs;
        }
        let wait = if self.tokens >= 1.0 {
            0.0
        } else if self.qps > 0.0 {
            (1.0 - self.tokens) / self.qps
        } else {
            // qps == 0 means "never refill" — treat as effectively unbounded.
            f64::MAX
        };
        self.tokens -= 1.0;
        Duration::from_secs_f64(wait.clamp(0.0, Duration::MAX.as_secs_f64()))
    }
}

/// `MaxOf(per-item exponential backoff, shared token bucket)` — the k8s
/// `DefaultControllerRateLimiter`. The retry delay for an item is the larger
/// of its own exponential backoff and the global-rate reservation.
#[derive(Debug, Clone)]
pub struct RetryRateLimiter<K> {
    item: ItemExponentialBackoff<K>,
    bucket: TokenBucket,
    origin: Instant,
}

impl<K: Eq + Hash + Clone> RetryRateLimiter<K> {
    /// Build from explicit parameters.
    pub fn new(base: Duration, max: Duration, qps: f64, burst: u32) -> Self {
        Self {
            item: ItemExponentialBackoff::new(base, max),
            bucket: TokenBucket::new(qps, burst),
            origin: Instant::now(),
        }
    }

    /// The k8s `DefaultControllerRateLimiter` defaults: per-item backoff from
    /// 5ms to 1000s, overall bucket 10 qps / 100 burst. Sensible for jfc's
    /// subagent + bounty retries.
    pub fn default_controller() -> Self {
        Self::new(
            Duration::from_millis(5),
            Duration::from_secs(1000),
            10.0,
            100,
        )
    }

    /// Record a requeue for `item` and return how long to wait before retrying
    /// it — `max(item backoff, bucket reservation)`. Uses the internal
    /// monotonic clock; tests should prefer [`Self::when_at`].
    pub fn when(&mut self, item: &K) -> Duration {
        let now = self.origin.elapsed().as_secs_f64();
        self.when_at(item, now)
    }

    /// As [`Self::when`] but with an explicit logical timestamp (monotonic
    /// seconds) for the token bucket, making the result deterministic.
    pub fn when_at(&mut self, item: &K, now_secs: f64) -> Duration {
        let item_delay = self.item.when(item);
        let bucket_delay = self.bucket.reserve(now_secs);
        item_delay.max(bucket_delay)
    }

    /// Retry count for `item`.
    pub fn num_requeues(&self, item: &K) -> u32 {
        self.item.num_requeues(item)
    }

    /// Clear `item`'s per-item backoff (call on success / permanent failure).
    /// The shared bucket is intentionally untouched — it's a global rate, not
    /// per-item state.
    pub fn forget(&mut self, item: &K) {
        self.item.forget(item);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Normal: exponential curve doubles each requeue and starts at base.
    #[test]
    fn item_backoff_doubles_then_caps_normal() {
        let mut rl = ItemExponentialBackoff::new(Duration::from_millis(5), Duration::from_secs(10));
        let key = "task-a";
        assert_eq!(rl.when(&key), Duration::from_millis(5)); // 5 * 2^0
        assert_eq!(rl.when(&key), Duration::from_millis(10)); // 5 * 2^1
        assert_eq!(rl.when(&key), Duration::from_millis(20)); // 5 * 2^2
        assert_eq!(rl.when(&key), Duration::from_millis(40)); // 5 * 2^3
        assert_eq!(rl.num_requeues(&key), 4);
    }

    // Robust: a far-out exponent saturates at max instead of overflowing.
    #[test]
    fn item_backoff_saturates_at_max_robust() {
        let mut rl = ItemExponentialBackoff::new(Duration::from_millis(5), Duration::from_secs(1000));
        let key = "hot";
        for _ in 0..200 {
            // 5ms * 2^200 is astronomically large — must clamp, never panic.
            let d = rl.when(&key);
            assert!(d <= Duration::from_secs(1000));
        }
        assert_eq!(rl.when(&key), Duration::from_secs(1000));
    }

    // Normal: forget resets a key's curve to the base delay.
    #[test]
    fn forget_resets_item_curve_normal() {
        let mut rl = ItemExponentialBackoff::new(Duration::from_millis(5), Duration::from_secs(10));
        let key = "k";
        rl.when(&key);
        rl.when(&key);
        assert_eq!(rl.num_requeues(&key), 2);
        rl.forget(&key);
        assert_eq!(rl.num_requeues(&key), 0);
        assert_eq!(rl.when(&key), Duration::from_millis(5)); // back to base
    }

    // Normal: distinct items have independent backoff curves.
    #[test]
    fn item_curves_are_independent_normal() {
        let mut rl = ItemExponentialBackoff::new(Duration::from_millis(5), Duration::from_secs(10));
        rl.when(&"a");
        rl.when(&"a");
        assert_eq!(rl.when(&"b"), Duration::from_millis(5)); // b unaffected by a
    }

    // Normal: a full bucket serves burst tokens instantly, then spaces the
    // rest at 1/qps once empty.
    #[test]
    fn token_bucket_burst_then_spaces_normal() {
        let mut b = TokenBucket::new(10.0, 2); // 10 qps, burst 2
        assert_eq!(b.reserve(0.0), Duration::ZERO); // token 1 (free)
        assert_eq!(b.reserve(0.0), Duration::ZERO); // token 2 (free)
        // Bucket now empty; next reservation waits 1/qps = 0.1s.
        let d = b.reserve(0.0);
        assert!((d.as_secs_f64() - 0.1).abs() < 1e-9, "got {d:?}");
    }

    // Robust: tokens refill over elapsed logical time up to burst.
    #[test]
    fn token_bucket_refills_over_time_robust() {
        let mut b = TokenBucket::new(10.0, 1); // 1 token cap
        assert_eq!(b.reserve(0.0), Duration::ZERO); // spend the token
        // 0.05s later only half a token has refilled -> wait ~0.05s.
        let d = b.reserve(0.05);
        assert!((d.as_secs_f64() - 0.05).abs() < 1e-9, "got {d:?}");
    }

    // Normal: MaxOf picks the larger of item backoff vs bucket reservation.
    // With a full bucket the bucket cost is 0, so item backoff dominates.
    #[test]
    fn maxof_item_dominates_with_full_bucket_normal() {
        let mut rl = RetryRateLimiter::new(
            Duration::from_millis(50),
            Duration::from_secs(10),
            1000.0, // huge qps + burst so the bucket is never the constraint
            1000,
        );
        let d = rl.when_at(&"task", 0.0);
        assert_eq!(d, Duration::from_millis(50)); // item backoff wins
    }

    // Robust: MaxOf picks the bucket when the global rate is the tighter
    // constraint (tiny item backoff, exhausted bucket).
    #[test]
    fn maxof_bucket_dominates_when_rate_limited_robust() {
        let mut rl = RetryRateLimiter::new(
            Duration::from_nanos(1), // negligible item backoff
            Duration::from_secs(10),
            1.0, // 1 qps
            1,   // burst 1
        );
        assert_eq!(rl.when_at(&"a", 0.0), Duration::from_nanos(1)); // bucket free -> item wins
        // Bucket now empty; the 1 qps reservation (1s) dwarfs the 1ns backoff.
        let d = rl.when_at(&"b", 0.0);
        assert!((d.as_secs_f64() - 1.0).abs() < 1e-9, "bucket should dominate, got {d:?}");
    }

    // Normal: the documented default controller params are wired correctly.
    #[test]
    fn default_controller_params_normal() {
        let mut rl = RetryRateLimiter::default_controller();
        // First requeue of an item = base delay (5ms), bucket full so it wins.
        assert_eq!(rl.when_at(&"x", 0.0), Duration::from_millis(5));
        rl.forget(&"x");
        assert_eq!(rl.num_requeues(&"x"), 0);
    }
}

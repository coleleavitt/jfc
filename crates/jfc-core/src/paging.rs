//! MemGPT-style self-paging memory policy.
//!
//! From *MemGPT: Towards LLMs as Operating Systems* (arXiv:2310.08560): treat
//! the context window like physical RAM with a paging discipline. As the window
//! fills, the system raises a **memory-pressure warning** so the model can flush
//! salient facts to durable store; at the hard limit it **evicts** the oldest
//! half of the FIFO queue and folds it into a *recursive summary* kept pinned at
//! the head, so old context degrades gracefully instead of being lost.
//!
//! jfc's memory store is otherwise plain CRUD with no eviction/pressure loop;
//! this module is that loop, as a pure state machine. The actual summarisation
//! (an LLM call in production) is injected as a closure to [`PageStore::flush`],
//! so the policy — token accounting, the Ok/Warn/Flush thresholds, evicting
//! ~50% of the FIFO, prepending the fold to the recursive summary — is fully
//! deterministic and tested.

use std::collections::VecDeque;

/// Memory-pressure level relative to the window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pressure {
    /// Below the warning threshold — nothing to do.
    Ok,
    /// At/above `warn_frac` — the model should proactively persist salient
    /// facts before the hard limit forces an eviction.
    Warn,
    /// At/above `flush_frac` — an eviction is due.
    Flush,
}

/// A paged working set with a recursive summary head and a FIFO body.
///
/// Token usage is estimated as `chars / 4` unless an explicit per-item token
/// count is supplied via [`PageStore::push_with_tokens`].
#[derive(Debug, Clone)]
pub struct PageStore {
    /// Pinned recursive summary kept at the head of the context.
    recursive_summary: String,
    /// FIFO of live items (oldest at the front), each with its token estimate.
    fifo: VecDeque<(String, u64)>,
    /// Window size in tokens.
    window_tokens: u64,
    /// Fraction of the window at which [`Pressure::Warn`] begins.
    warn_frac: f64,
    /// Fraction of the window at which [`Pressure::Flush`] begins.
    flush_frac: f64,
}

/// Estimate tokens for a string as `ceil(chars / 4)` — the rough rule of thumb
/// for English text, intentionally cheap and dependency-free.
pub fn estimate_tokens(s: &str) -> u64 {
    linkscope::detail_event_fields(
        "paging.estimate_tokens",
        [linkscope::TraceField::bytes(
            "input_bytes",
            u64::try_from(s.len()).unwrap_or(u64::MAX),
        )],
    );
    u64::try_from(s.chars().count())
        .unwrap_or(u64::MAX)
        .div_ceil(4)
}

impl PageStore {
    /// New store over a `window_tokens` budget. `warn_frac` and `flush_frac` are
    /// clamped to `[0, 1]` and ordered so `warn <= flush`.
    pub fn new(window_tokens: u64, warn_frac: f64, flush_frac: f64) -> Self {
        let _linkscope_new = linkscope::phase("paging.store.new");
        let warn = warn_frac.clamp(0.0, 1.0);
        let flush = flush_frac.clamp(0.0, 1.0);
        linkscope::event_fields(
            "paging.store.new",
            [
                linkscope::TraceField::count("window_tokens", window_tokens),
                linkscope::TraceField::text("warn_frac", format!("{warn:.3}")),
                linkscope::TraceField::text("flush_frac", format!("{flush:.3}")),
            ],
        );
        Self {
            recursive_summary: String::new(),
            fifo: VecDeque::new(),
            window_tokens,
            warn_frac: warn.min(flush),
            flush_frac: flush.max(warn),
        }
    }

    /// Push an item, estimating its tokens with [`estimate_tokens`].
    pub fn push(&mut self, item: impl Into<String>) {
        let _linkscope_push = linkscope::phase("paging.store.push");
        let s = item.into();
        let t = estimate_tokens(&s);
        self.fifo.push_back((s, t));
        linkscope::event_fields(
            "paging.store.push.result",
            [
                linkscope::TraceField::count("tokens", t),
                linkscope::TraceField::count(
                    "items",
                    u64::try_from(self.fifo.len()).unwrap_or(u64::MAX),
                ),
                linkscope::TraceField::count("used_tokens", self.used_tokens()),
            ],
        );
    }

    /// Push an item with an explicit token count (e.g. from a real tokenizer).
    pub fn push_with_tokens(&mut self, item: impl Into<String>, tokens: u64) {
        let _linkscope_push = linkscope::phase("paging.store.push_with_tokens");
        self.fifo.push_back((item.into(), tokens));
        linkscope::event_fields(
            "paging.store.push_with_tokens.result",
            [
                linkscope::TraceField::count("tokens", tokens),
                linkscope::TraceField::count(
                    "items",
                    u64::try_from(self.fifo.len()).unwrap_or(u64::MAX),
                ),
                linkscope::TraceField::count("used_tokens", self.used_tokens()),
            ],
        );
    }

    /// Current token usage: the recursive summary plus every FIFO item.
    pub fn used_tokens(&self) -> u64 {
        estimate_tokens(&self.recursive_summary) + self.fifo.iter().map(|(_, t)| t).sum::<u64>()
    }

    /// Number of live FIFO items (excludes the recursive summary).
    pub fn len(&self) -> usize {
        self.fifo.len()
    }

    pub fn is_empty(&self) -> bool {
        self.fifo.is_empty()
    }

    /// The pinned recursive summary.
    pub fn recursive_summary(&self) -> &str {
        &self.recursive_summary
    }

    /// Current memory pressure relative to the configured thresholds.
    pub fn pressure(&self) -> Pressure {
        let _linkscope_pressure = linkscope::phase("paging.store.pressure");
        if self.window_tokens == 0 {
            linkscope::event_fields(
                "paging.store.pressure.result",
                [linkscope::TraceField::text("pressure", "Flush")],
            );
            return Pressure::Flush;
        }
        let frac = self.used_tokens() as f64 / self.window_tokens as f64;
        let pressure = if frac >= self.flush_frac {
            Pressure::Flush
        } else if frac >= self.warn_frac {
            Pressure::Warn
        } else {
            Pressure::Ok
        };
        linkscope::event_fields(
            "paging.store.pressure.result",
            [
                linkscope::TraceField::text("pressure", format!("{pressure:?}")),
                linkscope::TraceField::text("frac", format!("{frac:.3}")),
                linkscope::TraceField::count("used_tokens", self.used_tokens()),
                linkscope::TraceField::count("window_tokens", self.window_tokens),
            ],
        );
        pressure
    }

    /// Evict the oldest ~50% of FIFO items (rounding up so a non-empty queue
    /// always frees at least one), summarise them via `summarize`, and prepend
    /// that fold to the recursive summary. Returns the number of items evicted.
    ///
    /// `summarize` receives the evicted items oldest-first and returns their
    /// condensed form; passing the previous recursive summary is the caller's
    /// job if they want a true rolling fold — here we prepend so the newest
    /// fold sits closest to the live FIFO, matching MemGPT's head placement.
    pub fn flush(&mut self, summarize: impl Fn(&[String]) -> String) -> usize {
        let _linkscope_flush = linkscope::phase("paging.store.flush");
        if self.fifo.is_empty() {
            linkscope::event_fields(
                "paging.store.flush.result",
                [linkscope::TraceField::count("evicted", 0)],
            );
            return 0;
        }
        let evict = self.fifo.len().div_ceil(2);
        let drained: Vec<String> = self.fifo.drain(..evict).map(|(s, _)| s).collect();
        let fold = summarize(&drained);
        if self.recursive_summary.is_empty() {
            self.recursive_summary = fold;
        } else if !fold.is_empty() {
            self.recursive_summary = format!("{fold}\n{}", self.recursive_summary);
        }
        linkscope::event_fields(
            "paging.store.flush.result",
            [
                linkscope::TraceField::count(
                    "evicted",
                    u64::try_from(drained.len()).unwrap_or(u64::MAX),
                ),
                linkscope::TraceField::count(
                    "remaining",
                    u64::try_from(self.fifo.len()).unwrap_or(u64::MAX),
                ),
                linkscope::TraceField::bytes(
                    "summary_bytes",
                    u64::try_from(self.recursive_summary.len()).unwrap_or(u64::MAX),
                ),
            ],
        );
        drained.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Normal: token estimate is ceil(chars / 4).
    #[test]
    fn token_estimate_is_ceil_quarter_normal() {
        assert_eq!(estimate_tokens(""), 0);
        assert_eq!(estimate_tokens("abcd"), 1);
        assert_eq!(estimate_tokens("abcde"), 2); // ceil(5/4)
    }

    // Normal: pressure crosses Ok -> Warn -> Flush as the window fills.
    #[test]
    fn pressure_thresholds_normal() {
        // window 100 tokens, warn at 50%, flush at 80%.
        let mut s = PageStore::new(100, 0.5, 0.8);
        assert_eq!(s.pressure(), Pressure::Ok);
        // push ~40 tokens (160 chars) -> 40% -> still Ok.
        s.push_with_tokens("x", 40);
        assert_eq!(s.pressure(), Pressure::Ok);
        // +15 -> 55% -> Warn.
        s.push_with_tokens("y", 15);
        assert_eq!(s.pressure(), Pressure::Warn);
        // +30 -> 85% -> Flush.
        s.push_with_tokens("z", 30);
        assert_eq!(s.pressure(), Pressure::Flush);
    }

    // Normal: flush evicts ~50% (rounding up) of the FIFO.
    #[test]
    fn flush_evicts_half_rounding_up_normal() {
        let mut s = PageStore::new(1000, 0.5, 0.8);
        for i in 0..5 {
            s.push_with_tokens(format!("item-{i}"), 10);
        }
        // 5 items -> evict ceil(5/2) = 3.
        let evicted = s.flush(|items| format!("summary-of-{}", items.len()));
        assert_eq!(evicted, 3);
        assert_eq!(s.len(), 2);
    }

    // Robust: the eviction summary is folded into the recursive summary head,
    // and a second flush prepends the newer fold above the older one.
    #[test]
    fn recursive_summary_folds_at_head_robust() {
        let mut s = PageStore::new(1000, 0.5, 0.8);
        s.push_with_tokens("a", 10);
        s.push_with_tokens("b", 10);
        // First flush: 2 items -> evict 1 ("a").
        s.flush(|items| format!("[{}]", items.join(",")));
        assert_eq!(s.recursive_summary(), "[a]");
        // Second flush: remaining ["b"] -> evict 1, newer fold goes on top.
        s.flush(|items| format!("[{}]", items.join(",")));
        assert_eq!(s.recursive_summary(), "[b]\n[a]");
    }

    // Robust: flushing an empty store changes nothing and never panics.
    #[test]
    fn flush_empty_does_nothing_robust() {
        let mut s = PageStore::new(1000, 0.5, 0.8);
        let evicted = s.flush(|_| "x".to_string());
        assert_eq!(evicted, 0);
        assert_eq!(s.recursive_summary(), "");
    }

    // Robust: a zero-size window is always under flush pressure.
    #[test]
    fn zero_window_is_flush_pressure_robust() {
        let s = PageStore::new(0, 0.5, 0.8);
        assert_eq!(s.pressure(), Pressure::Flush);
    }

    // Robust: out-of-order/over-range fracs are clamped and ordered.
    #[test]
    fn fracs_clamped_and_ordered_robust() {
        // warn given larger than flush -> swapped so warn <= flush.
        let s = PageStore::new(100, 0.9, 0.2);
        assert!(s.warn_frac <= s.flush_frac);
        // out of range -> clamped into [0,1].
        let s2 = PageStore::new(100, -1.0, 2.0);
        assert_eq!(s2.warn_frac, 0.0);
        assert_eq!(s2.flush_frac, 1.0);
    }
}

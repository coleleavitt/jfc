//! Per-request token/cost timeline capture (binary-side).
//!
//! Each completed provider request appends one [`jfc_dashboard::TimelineSample`]
//! to a bounded ring on `App`. Deltas are derived from the *cumulative*
//! `usage_by_model` via a stored baseline (the engine never records per-request
//! usage), so each request's incremental tokens are attributed exactly once.
//! The capture fires from the drained `EngineEffect::StreamingFinalized` arm —
//! edge-triggered, exactly once per round-trip.

use std::time::{SystemTime, UNIX_EPOCH};

use crate::app::App;

/// Max samples retained (~a long agentic session); bounds memory.
pub const TIMELINE_CAP: usize = 256;

/// Cumulative usage snapshot used to compute per-request deltas. Not serialized.
#[derive(Debug, Clone, Default)]
pub struct TimelineBaseline {
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_write: u64,
    pub thinking: u64,
    pub cost: f64,
}

// Anomaly heuristics — flagged "for review", not "error" (these are candidates
// for human inspection, not measured truths). Tunable.
//
// Baselines are a ROLLING MEDIAN over the last `ROLLING_WINDOW` requests, not the
// single previous request: in cache-heavy sessions, per-request cost/cache
// oscillates wildly (a turn writes a big context to cache → spike, the next just
// reads it cheaply → drop), so comparing to one neighbor produces false
// positives. The median smooths that out.
const ROLLING_WINDOW: usize = 8;
const INPUT_SPIKE_RATIO: f64 = 1.5;
const INPUT_SPIKE_ABS: u64 = 4_000;
const COST_SPIKE_RATIO: f64 = 2.0;
/// A request must cost at least this much to be a "spike" — avoids flagging
/// cheap requests that merely doubled a near-zero neighbor.
const COST_SPIKE_FLOOR_USD: f64 = 0.10;
/// Cache-write tokens above this, when writes exceed reads, mark a `cache_rewrite`
/// (a fresh / re-cached context). It explains a cost spike *without* the request
/// being "wrong", so it suppresses the `cost_spike` alarm for that sample.
const CACHE_REWRITE_ABS: u64 = 8_000;
const CACHE_DROP_PTS: f64 = 20.0;
const CACHE_DROP_FLOOR: f64 = 40.0;
const CONTEXT_HIGH_WATER: f64 = 0.85;

impl App {
    /// Record one timeline sample for the request that just finalized. No-op for
    /// an empty round-trip (no new tokens), so the chart isn't padded with zeros.
    pub fn record_timeline_sample(&mut self) {
        // Snapshot everything needed from the engine first (immutable borrows),
        // then mutate the ring + baseline.
        let mut now = TimelineBaseline::default();
        for usage in self.engine.usage_by_model.values() {
            now.input = now.input.saturating_add(usage.input_tokens);
            now.output = now.output.saturating_add(usage.output_tokens);
            now.cache_read = now.cache_read.saturating_add(usage.cache_read_tokens);
            now.cache_write = now.cache_write.saturating_add(usage.cache_write_tokens);
            now.thinking = now.thinking.saturating_add(usage.thinking_tokens);
        }
        now.cost = jfc_engine::cost::total_cost(&self.engine.usage_by_model);

        let base = self.timeline_baseline.clone();
        let input_delta = now.input.saturating_sub(base.input);
        let output_delta = now.output.saturating_sub(base.output);
        let cost_delta_usd = (now.cost - base.cost).max(0.0);

        // Skip a finalize that produced no measurable work (aborted/empty
        // round-trip): advance the baseline but don't emit a zero bar.
        if input_delta == 0 && output_delta == 0 && cost_delta_usd <= 0.0 {
            self.timeline_baseline = now;
            return;
        }

        let cache_read_delta = now.cache_read.saturating_sub(base.cache_read);
        let cache_write_delta = now.cache_write.saturating_sub(base.cache_write);
        let thinking_delta = now.thinking.saturating_sub(base.thinking);
        let cache_hit_pct = if input_delta > 0 {
            (cache_read_delta as f64 / input_delta as f64 * 100.0).min(100.0)
        } else {
            0.0
        };

        let mut sample = jfc_dashboard::TimelineSample {
            ts_unix: now_unix(),
            model: self.engine.model.as_str().to_owned(),
            prompt: last_user_prompt(&self.engine.messages),
            input_delta,
            output_delta,
            cache_read_delta,
            cache_write_delta,
            thinking_delta,
            cost_delta_usd,
            context_used_tokens: self.engine.tool_ctx.approx_tokens as u64,
            context_window_tokens: self.engine.selected_context_window_tokens() as u64,
            cache_hit_pct,
            flags: Vec::new(),
            rsi_prompt_sections: self
                .engine
                .current_stream_request
                .as_ref()
                .map(|metadata| metadata.rsi_prompt_sections as u64)
                .unwrap_or(0),
            rsi_tool_visibility_rules: self
                .engine
                .current_stream_request
                .as_ref()
                .map(|metadata| metadata.rsi_tool_visibility_rules as u64)
                .unwrap_or(0),
        };
        sample.flags = anomaly_flags(&sample, &self.timeline);

        if self.timeline.len() >= TIMELINE_CAP {
            self.timeline.pop_front();
        }
        self.timeline.push_back(sample);
        self.timeline_baseline = now;
    }

    /// Clear the timeline + baseline (session switch / `/clear`) so the next
    /// request doesn't emit one spurious giant delta.
    pub fn reset_timeline(&mut self) {
        self.timeline.clear();
        self.timeline_baseline = TimelineBaseline::default();
    }
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or(0)
}

/// The most recent user-role message's text, truncated — the prompt the current
/// turn is answering.
fn last_user_prompt(messages: &[jfc_core::ChatMessage]) -> Option<String> {
    let message = messages
        .iter()
        .rev()
        .find(|message| message.role_is_user())?;
    let text: String = message
        .parts
        .iter()
        .filter_map(|part| match part {
            jfc_core::MessagePart::Text(text) => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join(" ");
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if text.is_empty() {
        return None;
    }
    Some(truncate(&text, 140))
}

fn truncate(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_owned();
    }
    let head: String = text.chars().take(max_chars).collect();
    format!("{head}…")
}

/// Median of a set of values (0.0 when empty).
fn median(mut values: Vec<f64>) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    values.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let mid = values.len() / 2;
    if values.len().is_multiple_of(2) {
        (values[mid - 1] + values[mid]) / 2.0
    } else {
        values[mid]
    }
}

/// Tag review-worthy anomalies on a fresh sample, judged against a rolling median
/// of the recent requests (`recent`, newest at the back) rather than a single
/// neighbor — see the constants above for why.
fn anomaly_flags(
    sample: &jfc_dashboard::TimelineSample,
    recent: &std::collections::VecDeque<jfc_dashboard::TimelineSample>,
) -> Vec<String> {
    let mut flags = Vec::new();

    // Context pressure — absolute, needs no baseline.
    if sample.context_window_tokens > 0 {
        let occupancy = sample.context_used_tokens as f64 / sample.context_window_tokens as f64;
        if occupancy >= CONTEXT_HIGH_WATER {
            flags.push("context_near_window".to_owned());
        }
    }

    // Cache rewrite: this request wrote far more cache than it read — a fresh or
    // re-cached context. It's the normal cost of caching (not a problem), and it
    // explains a cost spike, so we surface it as informational AND suppress the
    // `cost_spike` alarm below.
    let cache_rewrite = sample.cache_write_delta >= CACHE_REWRITE_ABS
        && sample.cache_write_delta > sample.cache_read_delta;
    if cache_rewrite {
        flags.push("cache_rewrite".to_owned());
    }

    let window: Vec<&jfc_dashboard::TimelineSample> =
        recent.iter().rev().take(ROLLING_WINDOW).collect();
    if window.is_empty() {
        return flags; // first request — no baseline to compare against.
    }
    let median_cost = median(window.iter().map(|s| s.cost_delta_usd).collect());
    let median_input = median(window.iter().map(|s| s.input_delta as f64).collect());

    // Input ballooned vs the rolling median (context bloat / fat tool result).
    if median_input > 0.0
        && sample.input_delta as f64 > median_input * INPUT_SPIKE_RATIO
        && sample.input_delta as f64 > median_input + INPUT_SPIKE_ABS as f64
    {
        flags.push("input_spike".to_owned());
    }

    // Genuine cost spike: above the rolling median by the ratio AND above an
    // absolute floor AND not already explained by a cache rewrite.
    if !cache_rewrite
        && sample.cost_delta_usd >= COST_SPIKE_FLOOR_USD
        && median_cost > 0.0
        && sample.cost_delta_usd > median_cost * COST_SPIKE_RATIO
    {
        flags.push("cost_spike".to_owned());
    }

    // Prompt-cache reuse fell off vs the most recent request (lost cache → cost up).
    if let Some(prev) = window.first()
        && sample.input_delta > 0
        && prev.cache_hit_pct >= CACHE_DROP_FLOOR
        && sample.cache_hit_pct < prev.cache_hit_pct - CACHE_DROP_PTS
    {
        flags.push("cache_hit_drop".to_owned());
    }

    flags
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;

    fn sample(
        cost: f64,
        input: u64,
        cache_read: u64,
        cache_write: u64,
    ) -> jfc_dashboard::TimelineSample {
        jfc_dashboard::TimelineSample {
            cost_delta_usd: cost,
            input_delta: input,
            cache_read_delta: cache_read,
            cache_write_delta: cache_write,
            context_window_tokens: 1_000_000,
            ..Default::default()
        }
    }

    fn steady(cost: f64, input: u64, cache_read: u64) -> VecDeque<jfc_dashboard::TimelineSample> {
        let mut recent = VecDeque::new();
        for _ in 0..6 {
            recent.push_back(sample(cost, input, cache_read, 0));
        }
        recent
    }

    #[test]
    fn cache_write_spike_is_not_flagged_as_cost_spike_robust() {
        // Steady ~$0.40 requests, then a $2 request that is all cache-write — the
        // exact false positive from a cache-heavy session.
        let recent = steady(0.40, 2, 50_000);
        let write_heavy = sample(2.00, 2, 0, 120_000);
        let flags = anomaly_flags(&write_heavy, &recent);
        assert!(flags.contains(&"cache_rewrite".to_owned()));
        assert!(
            !flags.contains(&"cost_spike".to_owned()),
            "cache-write cost must not trip cost_spike"
        );
    }

    #[test]
    fn genuine_cost_and_input_spike_fires_normal() {
        let recent = steady(0.20, 100, 50_000);
        // Big uncached input, no cache write → real spike.
        let expensive = sample(1.50, 8_000, 50_000, 0);
        let flags = anomaly_flags(&expensive, &recent);
        assert!(flags.contains(&"cost_spike".to_owned()));
        assert!(flags.contains(&"input_spike".to_owned()));
        assert!(!flags.contains(&"cache_rewrite".to_owned()));
    }

    #[test]
    fn first_request_has_no_baseline_flags_normal() {
        let flags = anomaly_flags(&sample(5.0, 9_999, 0, 0), &VecDeque::new());
        assert!(!flags.contains(&"cost_spike".to_owned()));
        assert!(!flags.contains(&"input_spike".to_owned()));
    }

    #[test]
    fn cheap_request_doubling_a_near_zero_neighbor_is_not_a_spike_robust() {
        // Median cost ~$0.01; a $0.05 request is 5× but below the absolute floor.
        let recent = steady(0.01, 2, 1_000);
        let flags = anomaly_flags(&sample(0.05, 2, 1_000, 0), &recent);
        assert!(!flags.contains(&"cost_spike".to_owned()));
    }
}

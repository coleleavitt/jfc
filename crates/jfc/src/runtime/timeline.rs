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
const INPUT_SPIKE_RATIO: f64 = 1.5;
const INPUT_SPIKE_ABS: u64 = 4_000;
const COST_SPIKE_RATIO: f64 = 2.0;
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
        sample.flags = anomaly_flags(&sample, self.timeline.back());

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
    let message = messages.iter().rev().find(|message| message.role_is_user())?;
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

/// Compare a fresh sample to the previous one and tag review-worthy anomalies.
fn anomaly_flags(
    sample: &jfc_dashboard::TimelineSample,
    prev: Option<&jfc_dashboard::TimelineSample>,
) -> Vec<String> {
    let mut flags = Vec::new();

    if sample.context_window_tokens > 0 {
        let occupancy = sample.context_used_tokens as f64 / sample.context_window_tokens as f64;
        if occupancy >= CONTEXT_HIGH_WATER {
            flags.push("context_near_window".to_owned());
        }
    }

    if let Some(prev) = prev {
        // Input ballooned vs the prior request (context bloat / fat tool result).
        if prev.input_delta > 0
            && sample.input_delta > prev.input_delta.saturating_add(INPUT_SPIKE_ABS)
            && sample.input_delta as f64 > prev.input_delta as f64 * INPUT_SPIKE_RATIO
        {
            flags.push("input_spike".to_owned());
        }
        // Cost jumped — the dollar-side confirmation.
        if prev.cost_delta_usd > 0.0 && sample.cost_delta_usd > prev.cost_delta_usd * COST_SPIKE_RATIO
        {
            flags.push("cost_spike".to_owned());
        }
        // Prompt-cache reuse fell off (drives input cost up).
        if sample.input_delta > 0
            && prev.cache_hit_pct >= CACHE_DROP_FLOOR
            && sample.cache_hit_pct < prev.cache_hit_pct - CACHE_DROP_PTS
        {
            flags.push("cache_hit_drop".to_owned());
        }
    }

    flags
}

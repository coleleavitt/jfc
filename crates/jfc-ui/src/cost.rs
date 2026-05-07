//! Per-model pricing and session cost calculation.
//!
//! Mirrors v126 cli.js `meter`/`costCounter` (line ~2091): we accumulate
//! token counts per model in `app.usage_by_model` (see `types::ModelUsage`)
//! and surface a running dollar total so the user sees their session spend
//! while chatting.
//!
//! Pricing rates are public Anthropic list prices in USD per million
//! tokens. Substring + case-insensitive matching means
//! `"anthropic/claude-opus-4-7"`, `"claude-opus-4-7"`, and
//! `"claude-opus-4-7[1m]"` all hit the Opus rate without a model-id
//! registry. Unknown models (Bedrock-routed ids, OpenAI, etc.) return
//! `None` and contribute $0 — accurate for "we don't know the rate".
//!
//! Cache writes are billed at a premium (1.25x input) and cache reads at
//! a discount (0.1x input), per Anthropic's prompt-caching pricing.

use std::collections::HashMap;

use crate::types::ModelUsage;

/// Dollar prices per million tokens for one model family.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ModelPricing {
    pub input_per_mtok: f64,
    pub output_per_mtok: f64,
    pub cache_read_per_mtok: f64,
    pub cache_write_per_mtok: f64,
}

const OPUS: ModelPricing = ModelPricing {
    input_per_mtok: 15.0,
    output_per_mtok: 75.0,
    cache_read_per_mtok: 1.50,
    cache_write_per_mtok: 18.75,
};

const SONNET: ModelPricing = ModelPricing {
    input_per_mtok: 3.0,
    output_per_mtok: 15.0,
    cache_read_per_mtok: 0.30,
    cache_write_per_mtok: 3.75,
};

const HAIKU: ModelPricing = ModelPricing {
    input_per_mtok: 1.0,
    output_per_mtok: 5.0,
    cache_read_per_mtok: 0.10,
    cache_write_per_mtok: 1.25,
};

/// Look up rates for a model id by case-insensitive substring match.
///
/// Returns `None` for unrecognized ids so cost calculation can default to
/// $0 rather than guess.
pub fn pricing_for(model_id: &str) -> Option<ModelPricing> {
    let id = model_id.to_ascii_lowercase();
    let result = if id.contains("opus") {
        Some(OPUS)
    } else if id.contains("sonnet") {
        Some(SONNET)
    } else if id.contains("haiku") {
        Some(HAIKU)
    } else {
        None
    };
    tracing::trace!(
        target: "jfc::cost",
        model_id,
        found = result.is_some(),
        "pricing_for"
    );
    result
}

/// Dollar cost for a single model's accumulated usage.
///
/// Returns `0.0` for unknown models — see module docs.
pub fn cost_for(model_id: &str, usage: &ModelUsage) -> f64 {
    let Some(p) = pricing_for(model_id) else {
        tracing::trace!(
            target: "jfc::cost",
            model_id,
            "cost_for: unknown model, returning $0"
        );
        return 0.0;
    };
    let m = 1_000_000.0;
    let cost = (usage.input_tokens as f64 / m) * p.input_per_mtok
        + (usage.output_tokens as f64 / m) * p.output_per_mtok
        + (usage.cache_read_tokens as f64 / m) * p.cache_read_per_mtok
        + (usage.cache_write_tokens as f64 / m) * p.cache_write_per_mtok;
    tracing::trace!(
        target: "jfc::cost",
        model_id,
        input_tokens = usage.input_tokens,
        output_tokens = usage.output_tokens,
        cache_read_tokens = usage.cache_read_tokens,
        cache_write_tokens = usage.cache_write_tokens,
        cost,
        "cost_for"
    );
    cost
}

/// Sum of `cost_for` across every model in the session usage map.
pub fn total_cost(usage_by_model: &HashMap<String, ModelUsage>) -> f64 {
    let total: f64 = usage_by_model
        .iter()
        .map(|(model, usage)| cost_for(model, usage))
        .sum();
    tracing::trace!(
        target: "jfc::cost",
        model_count = usage_by_model.len(),
        total,
        "total_cost"
    );
    total
}

/// Estimate per-agent cost from a `BackgroundTask`'s captured model and
/// running token counts. Returns `0.0` when no model is recorded or no
/// pricing entry matches.
pub fn cost_for_background_task(bt: &crate::app::BackgroundTask) -> f64 {
    let Some(model) = bt.model_used.as_deref() else {
        return 0.0;
    };
    let mut usage = crate::types::ModelUsage::default();
    usage.input_tokens = bt.latest_input_tokens;
    usage.output_tokens = bt.cumulative_output_tokens;
    cost_for(model, &usage)
}

/// Format a dollar amount for the sidebar.
///
/// Rules: zero renders as `"$0.00"`, amounts under $1 use 4 decimal
/// places (so sub-cent figures stay visible), $1+ uses 2 decimal places.
pub fn fmt_cost(dollars: f64) -> String {
    if dollars == 0.0 {
        "$0.00".to_string()
    } else if dollars < 1.0 {
        format!("${:.4}", dollars)
    } else {
        format!("${:.2}", dollars)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::ModelUsage;

    #[test]
    fn pricing_for_opus_substring_normal() {
        let bare = pricing_for("claude-opus-4-7").expect("bare opus id should match");
        assert_eq!(bare, OPUS, "bare opus id should resolve to Opus rates");

        let prefixed =
            pricing_for("anthropic/claude-opus-4-7[1m]").expect("prefixed opus id should match");
        assert_eq!(
            prefixed, OPUS,
            "substring match must work across provider prefixes and [1m] suffix"
        );
    }

    #[test]
    fn pricing_for_sonnet_normal() {
        let p = pricing_for("claude-sonnet-4-7").expect("sonnet id should match");
        assert_eq!(p, SONNET, "sonnet id should resolve to Sonnet rates");
    }

    #[test]
    fn pricing_for_haiku_normal() {
        let p = pricing_for("claude-haiku-4-7").expect("haiku id should match");
        assert_eq!(p, HAIKU, "haiku id should resolve to Haiku rates");
    }

    #[test]
    fn pricing_for_unknown_returns_none_robust() {
        assert!(
            pricing_for("gpt-4o").is_none(),
            "OpenAI model should be unknown"
        );
        assert!(
            pricing_for("random-model").is_none(),
            "arbitrary string should be unknown"
        );
        assert!(pricing_for("").is_none(), "empty id should be unknown");
    }

    #[test]
    fn pricing_for_case_insensitive_robust() {
        let upper = pricing_for("CLAUDE-OPUS-4-7").expect("uppercased opus id should match");
        assert_eq!(
            upper, OPUS,
            "match must be case-insensitive so providers can pass any casing"
        );
    }

    #[test]
    fn cost_for_opus_known_usage_normal() {
        // 1M input tokens * $15/Mtok = $15.00
        // 100K output tokens * $75/Mtok = $7.50
        // total = $22.50
        let usage = ModelUsage {
            input_tokens: 1_000_000,
            output_tokens: 100_000,
            cache_read_tokens: 0,
            cache_write_tokens: 0,
            cost_usd: None,
        };
        let dollars = cost_for("claude-opus-4-7", &usage);
        assert!(
            (dollars - 22.50).abs() < 1e-9,
            "expected $22.50 for 1M in + 100K out at Opus rates, got ${dollars}"
        );
    }

    #[test]
    fn cost_for_unknown_model_is_zero_robust() {
        let usage = ModelUsage {
            input_tokens: 1_000_000,
            output_tokens: 1_000_000,
            cache_read_tokens: 1_000_000,
            cache_write_tokens: 1_000_000,
            cost_usd: None,
        };
        let dollars = cost_for("gpt-4o", &usage);
        assert_eq!(
            dollars, 0.0,
            "unknown model must report $0 (we don't know its rate)"
        );
    }

    #[test]
    fn fmt_cost_small_normal() {
        assert_eq!(
            fmt_cost(0.0123),
            "$0.0123",
            "sub-dollar amounts must keep 4 decimals so cents stay visible"
        );
    }

    #[test]
    fn fmt_cost_dollar_plus_normal() {
        assert_eq!(
            fmt_cost(1.23),
            "$1.23",
            "amounts >= $1 should use 2 decimal places"
        );
    }

    #[test]
    fn fmt_cost_zero_normal() {
        assert_eq!(
            fmt_cost(0.0),
            "$0.00",
            "zero should render as the canonical dollars-and-cents form"
        );
    }

    #[test]
    fn total_cost_sums_across_models_normal() {
        // Opus: 1M in ($15) + 100K out ($7.50) = $22.50
        // Sonnet: 1M in ($3) + 100K out ($1.50) = $4.50
        // Total: $27.00
        let mut by_model: HashMap<String, ModelUsage> = HashMap::new();
        by_model.insert(
            "claude-opus-4-7".into(),
            ModelUsage {
                input_tokens: 1_000_000,
                output_tokens: 100_000,
                cache_read_tokens: 0,
                cache_write_tokens: 0,
                cost_usd: None,
            },
        );
        by_model.insert(
            "claude-sonnet-4-5".into(),
            ModelUsage {
                input_tokens: 1_000_000,
                output_tokens: 100_000,
                cache_read_tokens: 0,
                cache_write_tokens: 0,
                cost_usd: None,
            },
        );
        let total = total_cost(&by_model);
        assert!(
            (total - 27.00).abs() < 1e-9,
            "expected $27.00 across opus + sonnet entries, got ${total}"
        );
    }
}

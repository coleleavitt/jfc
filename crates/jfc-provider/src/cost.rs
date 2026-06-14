//! Per-model pricing and session cost calculation.

use jfc_core::ModelUsage;

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
pub fn pricing_for(model_id: &str) -> Option<ModelPricing> {
    let id = model_id.to_ascii_lowercase();
    // fable-5 / mythos-5 (Claude Code 2.1.170) bill at Opus rates — they share
    // Opus 4.8's pricing group in the cli.
    if id.contains("opus") || id.contains("fable") || id.contains("mythos") {
        Some(OPUS)
    } else if id.contains("sonnet") {
        Some(SONNET)
    } else if id.contains("haiku") {
        Some(HAIKU)
    } else {
        None
    }
}

/// Dollar cost for a single model's accumulated usage.
///
/// Returns `0.0` for unknown models.
pub fn cost_for(model_id: &str, usage: &ModelUsage) -> f64 {
    let Some(p) = pricing_for(model_id) else {
        return 0.0;
    };
    let m = 1_000_000.0;
    (usage.input_tokens as f64 / m) * p.input_per_mtok
        + (usage.output_tokens as f64 / m) * p.output_per_mtok
        + (usage.cache_read_tokens as f64 / m) * p.cache_read_per_mtok
        + (usage.cache_write_tokens as f64 / m) * p.cache_write_per_mtok
}

/// Sum of `cost_for` across every model in the session usage map.
pub fn total_cost(usage_by_model: &std::collections::HashMap<String, ModelUsage>) -> f64 {
    usage_by_model
        .iter()
        .map(|(model, usage)| cost_for(model, usage))
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cost_for_opus_known_usage_normal() {
        let usage = ModelUsage {
            input_tokens: 1_000_000,
            output_tokens: 100_000,
            cache_read_tokens: 500_000,
            cache_write_tokens: 0,
            cost_usd: None,
        };
        let dollars = cost_for("claude-opus-4-7", &usage);
        assert!((dollars - 23.25).abs() < 0.01);
    }

    // CC 2.1.170: fable-5 / mythos-5 bill at Opus rates (same pricing group).
    #[test]
    fn fable_and_mythos_price_at_opus_rates_normal() {
        assert_eq!(pricing_for("claude-fable-5"), Some(OPUS));
        assert_eq!(pricing_for("claude-mythos-5"), Some(OPUS));
        assert_eq!(pricing_for("anthropic/claude-fable-5[1m]"), Some(OPUS));
    }

    #[test]
    fn cost_for_unknown_model_is_zero_robust() {
        let usage = ModelUsage {
            input_tokens: 1_000_000,
            output_tokens: 100_000,
            cache_read_tokens: 0,
            cache_write_tokens: 0,
            cost_usd: None,
        };
        let dollars = cost_for("gpt-4o", &usage);
        assert_eq!(dollars, 0.0);
    }
}

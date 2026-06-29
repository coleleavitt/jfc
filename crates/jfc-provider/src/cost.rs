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
    let _linkscope_pricing = linkscope::phase("provider.cost.pricing_for");
    let id = model_id.to_ascii_lowercase();
    let result = if id.contains("opus") || id.contains("fable") || id.contains("mythos") {
        Some(("opus", OPUS))
    } else if id.contains("sonnet") {
        Some(("sonnet", SONNET))
    } else if id.contains("haiku") {
        Some(("haiku", HAIKU))
    } else {
        None
    };
    trace_pricing_lookup(model_id, result.map(|(label, _)| label));
    result.map(|(_, pricing)| pricing)
}

/// Dollar cost for a single model's accumulated usage.
///
/// Returns `0.0` for unknown models.
pub fn cost_for(model_id: &str, usage: &ModelUsage) -> f64 {
    let _linkscope_cost = linkscope::phase("provider.cost.cost_for");
    let Some(p) = pricing_for(model_id) else {
        linkscope::record_items("provider.cost.unknown_model", 1);
        trace_cost(model_id, usage, 0.0, "unknown");
        return 0.0;
    };
    let m = 1_000_000.0;
    let cost = (usage.input_tokens as f64 / m) * p.input_per_mtok
        + (usage.output_tokens as f64 / m) * p.output_per_mtok
        + (usage.cache_read_tokens as f64 / m) * p.cache_read_per_mtok
        + (usage.cache_write_tokens as f64 / m) * p.cache_write_per_mtok;
    linkscope::record_items("provider.cost.known_model", 1);
    trace_cost(model_id, usage, cost, pricing_label(p));
    cost
}

/// Sum of `cost_for` across every model in the session usage map.
pub fn total_cost(usage_by_model: &std::collections::HashMap<String, ModelUsage>) -> f64 {
    let _linkscope_total = linkscope::phase("provider.cost.total_cost");
    linkscope::record_items(
        "provider.cost.total.models",
        usize_to_u64_saturating(usage_by_model.len()),
    );
    let total = usage_by_model
        .iter()
        .map(|(model, usage)| cost_for(model, usage))
        .sum();
    trace_total_cost(usage_by_model.len(), total);
    total
}

fn pricing_label(pricing: ModelPricing) -> &'static str {
    if pricing == OPUS {
        "opus"
    } else if pricing == SONNET {
        "sonnet"
    } else if pricing == HAIKU {
        "haiku"
    } else {
        "custom"
    }
}

fn trace_pricing_lookup(model_id: &str, pricing: Option<&'static str>) {
    linkscope::record_items(
        if pricing.is_some() {
            "provider.cost.pricing.hit"
        } else {
            "provider.cost.pricing.miss"
        },
        1,
    );
    if !linkscope::trace_detail_enabled() {
        return;
    }
    linkscope::detail_event_fields(
        "provider.cost.pricing.detail",
        [
            linkscope::TraceField::bytes("model_id_bytes", usize_to_u64_saturating(model_id.len())),
            linkscope::TraceField::text("pricing", pricing.unwrap_or("unknown")),
        ],
    );
}

fn trace_cost(model_id: &str, usage: &ModelUsage, cost: f64, pricing: &'static str) {
    if !linkscope::trace_detail_enabled() {
        return;
    }
    linkscope::detail_event_fields(
        "provider.cost.cost.detail",
        [
            linkscope::TraceField::bytes("model_id_bytes", usize_to_u64_saturating(model_id.len())),
            linkscope::TraceField::text("pricing", pricing),
            linkscope::TraceField::count("input_tokens", usage.input_tokens),
            linkscope::TraceField::count("output_tokens", usage.output_tokens),
            linkscope::TraceField::count("thinking_tokens", usage.thinking_tokens),
            linkscope::TraceField::count("cache_read_tokens", usage.cache_read_tokens),
            linkscope::TraceField::count("cache_write_tokens", usage.cache_write_tokens),
            linkscope::TraceField::count("estimated_micro_usd", dollars_to_micro_usd(cost)),
        ],
    );
}

fn trace_total_cost(models: usize, total: f64) {
    if !linkscope::trace_detail_enabled() {
        return;
    }
    linkscope::detail_event_fields(
        "provider.cost.total.detail",
        [
            linkscope::TraceField::count("models", usize_to_u64_saturating(models)),
            linkscope::TraceField::count("estimated_micro_usd", dollars_to_micro_usd(total)),
        ],
    );
}

fn dollars_to_micro_usd(value: f64) -> u64 {
    if !value.is_finite() || value <= 0.0 {
        return 0;
    }
    let scaled = value * 1_000_000.0;
    if scaled >= 18_446_744_073_709_551_615.0 {
        u64::MAX
    } else {
        format!("{:.0}", scaled.round()).parse().unwrap_or(u64::MAX)
    }
}

fn usize_to_u64_saturating(value: usize) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cost_for_opus_known_usage_normal() {
        let usage = ModelUsage {
            input_tokens: 1_000_000,
            output_tokens: 100_000,
            thinking_tokens: 25_000,
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
            thinking_tokens: 0,
            cache_read_tokens: 0,
            cache_write_tokens: 0,
            cost_usd: None,
        };
        let dollars = cost_for("gpt-4o", &usage);
        assert_eq!(dollars, 0.0);
    }

    #[test]
    fn cost_trace_records_shape_without_model_payload_normal() {
        linkscope::trace_detail_enable();
        let usage = ModelUsage {
            input_tokens: 1_000,
            output_tokens: 200,
            thinking_tokens: 50,
            cache_read_tokens: 25,
            cache_write_tokens: 5,
            cost_usd: None,
        };
        let dollars = cost_for("private-model-opus-name", &usage);
        assert!(dollars > 0.0);

        let snapshot = linkscope::snapshot();
        let rendered = format!("{snapshot:?}");
        assert!(rendered.contains("provider.cost.pricing.detail"));
        assert!(rendered.contains("provider.cost.cost.detail"));
        assert!(rendered.contains("model_id_bytes"));
        assert!(!rendered.contains("private-model-opus-name"));
    }
}

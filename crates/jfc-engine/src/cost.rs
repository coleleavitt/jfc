//! Re-export shim: pricing tables and cost math moved to `jfc_economy::cost`
//! during the engine extraction. Only the `BackgroundTask`-aware wrapper
//! stays here until `BackgroundTask` itself moves into the engine state.

pub use jfc_economy::cost::*;

/// Estimate per-agent cost from a `BackgroundTask`'s captured model and
/// running token counts. Returns `0.0` when no model is recorded or no
/// pricing entry matches.
pub fn cost_for_background_task(bt: &crate::app::BackgroundTask) -> f64 {
    let Some(model) = bt.model_used.as_deref() else {
        return 0.0;
    };
    let usage = crate::types::ModelUsage {
        input_tokens: bt.latest_input_tokens,
        cache_read_tokens: bt.latest_cache_read_tokens,
        cache_write_tokens: bt.latest_cache_write_tokens,
        output_tokens: bt.cumulative_output_tokens,
        ..Default::default()
    };
    cost_for(model, &usage)
}

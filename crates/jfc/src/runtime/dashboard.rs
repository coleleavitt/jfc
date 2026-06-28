//! Builds the token-audit dashboard snapshot from live engine state.
//!
//! `jfc-dashboard` is a leaf crate that never sees `EngineState`; this
//! binary-side builder is the only place that reads `app.engine` and projects
//! it into the serializable [`jfc_dashboard::DashboardSnapshot`]. The event loop
//! calls [`publish`] once per drained burst, so the dashboard reflects one
//! coherent view rather than mid-mutation state.

use std::time::{SystemTime, UNIX_EPOCH};

use jfc_dashboard::{CompartmentSummary, DashboardSnapshot, ModelUsageRow};

use crate::app::{App, EngineState};

/// Publish a fresh snapshot if the dashboard server is running. No-op otherwise.
pub(crate) fn publish(app: &App) {
    if app.dashboard.is_some() {
        let snapshot = build_snapshot(app);
        if let Some(handle) = app.dashboard.as_ref() {
            jfc_dashboard::publish(handle, snapshot);
        }
    }
}

/// Project the current app state into a dashboard snapshot. Pure read — it
/// reuses the engine's owned context-account and compartment producers so the
/// dashboard and the TUI sidebar can never disagree, and copies the per-request
/// timeline ring captured on `App`.
fn build_snapshot(app: &App) -> DashboardSnapshot {
    let engine: &EngineState = &app.engine;
    let budget = engine
        .current_stream_request
        .as_ref()
        .and_then(|metadata| metadata.context_budget)
        .or(engine.last_context_budget);
    let system_fallback = engine.last_system_prompt_len.unwrap_or(0) as u64;
    let account = jfc_engine::context_accounting::build_context_account(
        budget,
        &engine.messages,
        system_fallback,
    );

    let compartments = jfc_engine::context_accounting::build_compartment_sequence(&engine.messages)
        .map(|sequence| {
            CompartmentSummary::from_sequence(
                &sequence,
                jfc_engine::context_accounting::compartment_total_tokens(&engine.messages),
            )
        })
        .unwrap_or_default();

    let mut usage_by_model: Vec<ModelUsageRow> = engine
        .usage_by_model
        .iter()
        .map(|(model, usage)| ModelUsageRow {
            model: model.clone(),
            input_tokens: usage.input_tokens,
            output_tokens: usage.output_tokens,
            cache_read_tokens: usage.cache_read_tokens,
            cache_write_tokens: usage.cache_write_tokens,
            thinking_tokens: usage.thinking_tokens,
            cache_hit_pct: usage.cache_hit_pct(),
            cost_usd: jfc_engine::cost::cost_for(model, usage),
        })
        .collect();
    usage_by_model.sort_by(|a, b| {
        b.cost_usd
            .partial_cmp(&a.cost_usd)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    DashboardSnapshot {
        generated_at_unix: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|elapsed| elapsed.as_secs())
            .unwrap_or(0),
        session_id: engine
            .current_session_id
            .as_ref()
            .map(|id| id.as_str().to_owned()),
        model: Some(engine.model.as_str().to_owned()),
        context_window_tokens: engine.selected_context_window_tokens() as u64,
        context_used_tokens: engine.tool_ctx.approx_tokens as u64,
        account,
        compartments,
        usage_by_model,
        total_cost_usd: jfc_engine::cost::total_cost(&engine.usage_by_model),
        rsi_prompt_sections: engine
            .current_stream_request
            .as_ref()
            .map(|metadata| metadata.rsi_prompt_sections as u64)
            .unwrap_or(0),
        rsi_tool_visibility_rules: engine
            .current_stream_request
            .as_ref()
            .map(|metadata| metadata.rsi_tool_visibility_rules as u64)
            .unwrap_or(0),
        timeline: app.timeline.iter().cloned().collect(),
        profile: profile_phases(),
    }
}

/// Map linkscope's phase timings into the dashboard DTO. Empty when profiling is
/// off (a default launch), so the panel simply doesn't render.
fn profile_phases() -> Vec<jfc_dashboard::ProfilePhase> {
    if !linkscope::is_enabled() {
        return Vec::new();
    }
    let mut phases: Vec<jfc_dashboard::ProfilePhase> = linkscope::snapshot()
        .phases
        .into_iter()
        .map(|phase| jfc_dashboard::ProfilePhase {
            name: phase.name,
            ms: phase.nanos as f64 / 1_000_000.0,
            spans: phase.spans,
            bytes: phase.bytes,
            items: phase.items,
        })
        .collect();
    phases.sort_by(|a, b| b.ms.partial_cmp(&a.ms).unwrap_or(std::cmp::Ordering::Equal));
    phases
}

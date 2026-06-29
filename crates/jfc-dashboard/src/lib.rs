//! Local token-audit dashboard.
//!
//! A dependency-light localhost server (hand-rolled on `std::net`, one thread
//! per connection, no async runtime — the same shape as the design preview
//! server) that serves a single-page **token audit** view plus a JSON snapshot
//! endpoint the page polls.
//!
//! The engine owns a [`DashboardHandle`] (`Arc<Mutex<DashboardSnapshot>>`) and
//! publishes a fresh [`DashboardSnapshot`] whenever context/usage changes; the
//! server thread only ever *reads* that shared cell, so it never touches
//! `EngineState` and cannot race the single-threaded event loop.
//!
//! Everything served is real measured/estimated data — there are no synthetic
//! metrics. When a field is unknown it is `0`/`null`, not faked.

use std::sync::{Arc, Mutex};

use serde::Serialize;

mod server;
mod trace;

pub use server::{DashboardServer, spawn};

/// Shared, lock-guarded snapshot the engine writes and the server reads.
pub type DashboardHandle = Arc<Mutex<DashboardSnapshot>>;

/// Create an empty handle. The engine stores one end; [`spawn`] holds the other.
#[must_use]
pub fn new_handle() -> DashboardHandle {
    let _linkscope_handle = linkscope::phase("dashboard.new_handle");
    Arc::new(Mutex::new(DashboardSnapshot::default()))
}

/// Publish a fresh snapshot. Never panics: a poisoned lock is recovered so a
/// prior panic on the server side can't wedge the engine.
pub fn publish(handle: &DashboardHandle, snapshot: DashboardSnapshot) {
    let _linkscope_publish = linkscope::phase("dashboard.publish");
    trace::record_snapshot("dashboard.publish.shape", &snapshot);
    let mut guard = match handle.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };
    *guard = snapshot;
}

/// One immutable view of where the context window and spend are going.
#[derive(Debug, Clone, Default, Serialize)]
pub struct DashboardSnapshot {
    /// Wall-clock seconds since the Unix epoch when this snapshot was built.
    pub generated_at_unix: u64,
    pub session_id: Option<String>,
    pub model: Option<String>,
    /// The model's context window, in tokens (0 if unknown).
    pub context_window_tokens: u64,
    /// Tokens currently occupying the window (the gauge numerator).
    pub context_used_tokens: u64,
    /// Owned composition breakdown (System / Docs / Compartments / Memories /
    /// Conversation / Tool Calls / Tool Defs) with per-contributor tokens.
    pub account: jfc_context::ContextAccount,
    /// Real compaction-compartment rollup (counts by tier + tokens).
    pub compartments: CompartmentSummary,
    /// Per-model token usage and cost for the session.
    pub usage_by_model: Vec<ModelUsageRow>,
    /// Total session spend across all models, in USD.
    pub total_cost_usd: f64,
    /// Active RSI (recursive self-improvement) runtime guidance for the latest
    /// request: prompt sections + tool-visibility rules injected by the curator's
    /// promoted definitions. Grows as RSI verifies and promotes more.
    pub rsi_prompt_sections: u64,
    pub rsi_tool_visibility_rules: u64,
    /// DB-sourced RSI candidate→verified→active funnel. Unlike the per-request
    /// `rsi_*` counts above (which read the live request metadata and are 0
    /// between turns), this reads the knowledge store directly so the panel
    /// shows the *standing* self-improvement state and whether it's progressing.
    pub rsi_funnel: RsiFunnel,
    /// Per-request token/cost timeline (oldest → newest), for debugging where
    /// input/output tokens go over the session. Bounded ring; see
    /// [`TimelineSample`].
    pub timeline: Vec<TimelineSample>,
    /// In-process pipeline phase timings (from `linkscope`), e.g. `turn.submit`,
    /// `turn.compact`, `stream_context_budget`. Empty unless profiling is on.
    pub profile: Vec<ProfilePhase>,
}

/// One profiled pipeline phase: cumulative wall-time + invocation count, plus
/// any bytes/items the phase recorded. Mirrors a `linkscope` phase row without
/// the dashboard crate depending on linkscope.
#[derive(Debug, Clone, Default, Serialize)]
pub struct ProfilePhase {
    pub name: String,
    /// Cumulative wall-time across all spans, in milliseconds.
    pub ms: f64,
    /// Number of times the phase ran.
    pub spans: u64,
    pub bytes: u64,
    pub items: u64,
}

/// One completed provider request (LLM round-trip) — the per-request *delta*,
/// not a cumulative total. The audit panel charts these over time and groups
/// consecutive samples by `prompt` to show per-prompt cost.
#[derive(Debug, Clone, Default, Serialize)]
pub struct TimelineSample {
    /// Wall-clock seconds since the Unix epoch when this request finalized.
    pub ts_unix: u64,
    pub model: String,
    /// The user prompt this request belongs to (truncated). Consecutive samples
    /// sharing a prompt are one turn; the UI groups them for per-prompt totals.
    pub prompt: Option<String>,
    pub input_delta: u64,
    pub output_delta: u64,
    pub cache_read_delta: u64,
    pub cache_write_delta: u64,
    pub thinking_delta: u64,
    pub cost_delta_usd: f64,
    /// Context window occupancy *after* this request (gauge numerator).
    pub context_used_tokens: u64,
    pub context_window_tokens: u64,
    /// Per-request cache-hit fraction (cache_read / input). NOT the cumulative
    /// session figure.
    pub cache_hit_pct: f64,
    /// Anomaly tags flagged for review (e.g. `input_spike`, `cache_hit_drop`,
    /// `cost_spike`, `context_near_window`). Empty when nothing stood out.
    pub flags: Vec<String>,
    /// Active RSI prompt sections / tool-visibility rules at this request — lets
    /// the timeline show RSI guidance growing over the session.
    pub rsi_prompt_sections: u64,
    pub rsi_tool_visibility_rules: u64,
}

/// The RSI candidate→active funnel, sourced from the knowledge store. Tells the
/// honest story the per-request counts can't: how many self-improvement
/// definitions are staged, how many carry promotion-grade verification evidence,
/// and how many are live in the prompt. A growing `active` count over sessions
/// is "RSI improving over time"; a large `candidates` with `verified == 0` means
/// the loop is *generating* but not *proving* (the common stuck state).
#[derive(Debug, Clone, Default, Serialize)]
pub struct RsiFunnel {
    /// Staged candidate definitions awaiting promotion evidence (status=candidate).
    pub candidates: u64,
    /// Candidates whose metadata satisfies the promotion gate (trust=verified,
    /// fixtures pass, research verified) — eligible to promote right now.
    pub verified: u64,
    /// Promoted, runtime-active definitions (status=active) — injected into the
    /// system prompt. This is what `rsi_prompt_sections` reflects per-request.
    pub active: u64,
    /// Per-kind staged-candidate breakdown (e.g. `system_prompt`,
    /// `reasoning_policy`), newest-heavy first. Lets the panel show *what* the
    /// system keeps proposing for itself.
    pub by_kind: Vec<RsiKindCount>,
}

/// One RSI definition-kind row in the funnel breakdown.
#[derive(Debug, Clone, Default, Serialize)]
pub struct RsiKindCount {
    pub kind: String,
    pub candidates: u64,
    pub active: u64,
}

/// Rollup of the owned [`jfc_context::CompartmentSequence`] by tier.
#[derive(Debug, Clone, Default, Serialize)]
pub struct CompartmentSummary {
    pub count: usize,
    pub recent: usize,
    pub warm: usize,
    pub cold: usize,
    pub archived: usize,
    /// Tokens folded into compartments (the compaction-summary footprint).
    pub total_tokens: u64,
}

impl CompartmentSummary {
    /// Build a tier rollup from an owned compartment sequence.
    #[must_use]
    pub fn from_sequence(sequence: &jfc_context::CompartmentSequence, total_tokens: u64) -> Self {
        let _linkscope_summary = linkscope::phase("dashboard.compartment_summary.from_sequence");
        use jfc_context::CompartmentTier;
        let mut summary = Self {
            total_tokens,
            ..Self::default()
        };
        for compartment in sequence.compartments() {
            summary.count += 1;
            match compartment.tier() {
                CompartmentTier::Recent => summary.recent += 1,
                CompartmentTier::Warm => summary.warm += 1,
                CompartmentTier::Cold => summary.cold += 1,
                CompartmentTier::Archived => summary.archived += 1,
            }
        }
        linkscope::event_fields(
            "dashboard.compartment_summary.result",
            [
                linkscope::TraceField::count(
                    "count",
                    u64::try_from(summary.count).unwrap_or(u64::MAX),
                ),
                linkscope::TraceField::count("total_tokens", summary.total_tokens),
            ],
        );
        summary
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn publish_trace_records_snapshot_shape_without_payload_normal() {
        linkscope::trace_detail_enable();
        let handle = new_handle();
        let snapshot = DashboardSnapshot {
            session_id: Some("private-session-id".to_owned()),
            model: Some("private-model-name".to_owned()),
            context_window_tokens: 100,
            context_used_tokens: 42,
            usage_by_model: vec![ModelUsageRow {
                model: "private-row-model".to_owned(),
                input_tokens: 1,
                ..Default::default()
            }],
            timeline: vec![TimelineSample {
                prompt: Some("private prompt text".to_owned()),
                flags: vec!["private_flag".to_owned()],
                ..Default::default()
            }],
            profile: vec![ProfilePhase {
                name: "private-phase-name".to_owned(),
                spans: 1,
                ..Default::default()
            }],
            rsi_funnel: RsiFunnel {
                candidates: 3,
                verified: 2,
                active: 1,
                by_kind: vec![RsiKindCount {
                    kind: "private-kind".to_owned(),
                    candidates: 3,
                    active: 1,
                }],
            },
            ..Default::default()
        };

        publish(&handle, snapshot);

        let rendered = format!("{:?}", linkscope::snapshot());
        assert!(rendered.contains("dashboard.publish.shape"));
        assert!(rendered.contains("timeline_flags"));
        assert!(rendered.contains("rsi_candidates"));
        assert!(!rendered.contains("private-session-id"));
        assert!(!rendered.contains("private-model-name"));
        assert!(!rendered.contains("private-row-model"));
        assert!(!rendered.contains("private prompt text"));
        assert!(!rendered.contains("private_flag"));
        assert!(!rendered.contains("private-phase-name"));
        assert!(!rendered.contains("private-kind"));
    }
}

/// One model's session usage + computed cost.
#[derive(Debug, Clone, Default, Serialize)]
pub struct ModelUsageRow {
    pub model: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_write_tokens: u64,
    pub thinking_tokens: u64,
    pub cache_hit_pct: f64,
    pub cost_usd: f64,
}

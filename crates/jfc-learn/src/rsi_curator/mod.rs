//! RSI curator — host integration layer.
//!
//! The pure curator algorithm (trace → candidate → verify → promote) now lives
//! in the standalone `rsi-rs` crate and is re-exported here so existing
//! `crate::rsi_curator::*` paths (and the glue modules' `super::<module>::*`
//! paths) keep resolving unchanged. This module keeps the JFC-coupled glue:
//! `jfc-knowledge` persistence (`store`, `activation`, `loop_state`, `metadata`),
//! trace sourcing from session transcripts (`extract`), and the sandboxed
//! worker (`worker`) — plus the `RsiCuratorJob` that binds the pure config to
//! the host worker config.

mod activation;
mod extract;
mod loop_state;
mod metadata;
mod store;
mod worker;

// Pure curator core (the algorithm) — re-exported from `rsi-rs`.
pub use rsi_rs::*;

// --- host glue (stays in jfc; depends on jfc-knowledge / the sandbox) ---
pub use activation::{
    RsiActivationAction, RsiActivationReport, RsiDefinitionRef, is_promotable_candidate,
    promote_rsi_definition, rollback_rsi_definition,
};
pub use extract::{
    build_recent_rsi_job, load_recent_traces_from_store, load_trace_from_store, trace_from_messages,
};
pub use loop_state::{
    RSI_LOOP_STATE_KIND, RSI_LOOP_STATE_NAME, RsiExperimentLoopState, RsiLoopDueDecision,
    build_next_loop_state, current_time_ms, experiment_loop_due_decision,
    load_experiment_loop_state,
};
pub use store::{ApplyToStore, StoreApplyReport};
pub use worker::{
    RsiCuratorWorkerConfig, RsiWorkerInput, RsiWorkerOutput, run_rsi_worker_file,
    run_rsi_worker_job,
};

/// A curator job that binds the pure curator inputs to the host's sandboxed
/// worker config. Stays in jfc because `worker` is host glue (bubblewrap +
/// jfc-knowledge); the pure `RsiCurator`/`RsiCuratorConfig`/`RsiCuratorReport`
/// come from `rsi-rs`.
#[derive(Debug, Clone)]
pub struct RsiCuratorJob {
    pub traces: Vec<RsiTrace>,
    pub config: RsiCuratorConfig,
    pub promotion_policy: RsiPromotionPolicy,
    pub project_key: Option<String>,
    pub sandbox_enforcement: Option<RsiSandboxEnforcement>,
    pub worker: Option<RsiCuratorWorkerConfig>,
}

#[cfg(test)]
mod tests;

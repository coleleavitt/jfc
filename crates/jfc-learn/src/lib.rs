//! jfc-learn — self-learning agent with auto-memory, user profile, and key-file pinning.
//!
//! This crate provides:
//! - `normalize_hash` — content-addressable deduplication via normalized SHA256
//! - `historian` — fact extraction from coding session transcripts
//! - `user_memory` — user observation pipeline and profile promotion
//! - `dreamer` — background maintenance (consolidation, verification, archival)
//! - `key_files` — pinning frequently-accessed files into system prompt
//! - `auto_hints` — auto-search hint formatting
//! - `verifier` — ASG-SI contract verification for memory promotion

pub mod arch_graph;
pub mod auto_hints;
pub mod curator;
pub mod digest;
pub mod dreamer;
pub mod error;
pub mod historian;
pub mod key_files;
pub mod lifecycle;
pub mod next_task;
pub mod normalize_hash;
pub mod project_files;
pub mod prompt_miner;
pub mod provision;
pub mod rsi_curator;
pub mod scaffold_detector;
pub mod self_critique;
pub mod skill_induction;
pub mod skill_usage;
pub mod trajectory;
pub mod user_memory;
pub mod variant_selector;
pub mod verifier;
pub mod workflow_opt;

pub use auto_hints::{HintSource, RecallHint};
pub use curator::{CuratorConfig, CuratorPlan, SkillTransition, plan_transitions};
pub use digest::{
    Cadence, Digest, DigestItem, DigestSettings, DreamSettings, Wiki, WikiPage, build_digest,
    build_wiki,
};
pub use dreamer::{Dreamer, DreamerReport, DreamerTask};
pub use error::LearnError;
pub use historian::{
    CandidateFact, Historian, HistorianConfig, HistorianProvider, HistorianReport, MemoryLookup,
    ProcessedFact,
};
pub use key_files::{KeyFileStore, PinnedFile, ReadEvent};
pub use normalize_hash::normalize_and_hash;
pub use project_files::{ProjectContext, ProjectFile, ProjectFileSet};
pub use rsi_curator::{
    BudgetRecommendation, CandidateChange, CandidateEval, CandidateKind, CandidateStatus,
    CandidateTarget, ControlAssessment, ControlCapability, ControlTrust, ExperienceEdge,
    ExperienceEdgeKind, ExperienceGraph, ExperienceNode, ExperienceNodeKind, PromotionMode,
    RSI_LOOP_STATE_KIND, RSI_LOOP_STATE_NAME, RsiActivationAction, RsiActivationReport,
    RsiAgentFanout, RsiCurator, RsiCuratorConfig, RsiCuratorJob, RsiCuratorReport,
    RsiCuratorWorkerConfig, RsiDefinitionRef, RsiEvalProfile, RsiExperimentAction,
    RsiExperimentDashboard, RsiExperimentJobSpec, RsiExperimentLoopPlan, RsiExperimentLoopState,
    RsiExperimentPhase, RsiExperimentSchedule, RsiHiddenValidationHarness, RsiJobPreflight,
    RsiJobPreflightStatus, RsiLoopDueDecision, RsiLoopSandboxPlan, RsiOutcome, RsiPromotionPolicy,
    RsiRegressionFixture, RsiResearchCheck, RsiResearchRef, RsiRetrievalStep,
    RsiSandboxEnforcement, RsiSandboxEnforcementStatus, RsiSandboxExecutionMode, RsiSelectionEvent,
    RsiToolStep, RsiTrace, RsiTraceScore, RsiVerification, RsiWorkerInput, RsiWorkerOutput,
    StoreApplyReport, ThinkingAnalysis, ThinkingPatternKind, ThinkingProvenance, ThinkingSource,
    ToolVisibilityAction, ToolVisibilityRecommendation, analyze_thinking, assess_control,
    build_experience_graph, build_experiment_dashboard, build_experiment_job_spec,
    build_experiment_loop_plan, build_next_loop_state, build_recent_rsi_job, current_time_ms,
    evaluate_candidate, experiment_loop_due_decision, fixtures_for_candidate,
    is_promotable_candidate, load_experiment_loop_state, load_recent_traces_from_store,
    load_trace_from_store, promote_rsi_definition, rollback_rsi_definition, run_rsi_worker_file,
    run_rsi_worker_job, trace_from_messages,
};
pub use skill_usage::{CreatedBy, SkillState, SkillUsage, SkillUsageStore, record_skill_use};
pub use trajectory::{Turn, compress, total_tokens};
pub use user_memory::{UserMemoryPipeline, UserObservation, UserProfile, UserProfileEntry};
pub use variant_selector::{
    CaseOutcome, CompileReport, EvalCase, PromptVariant, Teleprompter, VariantEvaluator,
    VariantScore,
};
pub use verifier::{LlmVerifier, PromotionVerifier, VerifierContract, VerifierVerdict};
pub use workflow_opt::{
    WorkflowEvaluator, WorkflowOp, WorkflowOptimizer, WorkflowOutcome, WorkflowTask,
    WorkflowVariant,
};

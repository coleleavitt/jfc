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

pub mod auto_hints;
pub mod curator;
pub mod digest;
pub mod dreamer;
pub mod error;
pub mod historian;
pub mod key_files;
pub mod lifecycle;
pub mod normalize_hash;
pub mod project_files;
pub mod scaffold_detector;
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

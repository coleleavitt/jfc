//! Provider-neutral JFC domain types.
//!
//! This crate intentionally stays free of terminal UI, HTTP providers, and
//! runtime orchestration. Types move here only after their ownership is stable
//! enough to be shared without dragging `jfc-ui` dependencies with them.

mod agent_def;
mod assertions;
mod attachment;
mod compaction;
mod decision_quality;
pub mod diff;
mod execution;
mod execution_result;
mod fanout;
mod ids;
mod paging;
mod plan_cache;
mod routing;
mod task;
mod task_store;
mod tool_input;
mod tool_kind;
mod tool_retrieval;
mod usage;
mod workflow_search;

pub use agent_def::{AgentCost, AgentDef, Effort, MemoryScope, PermissionMode};
pub use assertions::{Assertion, AssertionOutcome, AssertionRun, run_with_assertions};
pub use attachment::{Attachment, AttachmentKind, PastedContent};
pub use compaction::{Retention, TurnCost, select_retained, select_retained_hybrid};
pub use decision_quality::{ChainOutput, DecisionQuality, DqWeights, SpecialistChain, Stage};
pub use diff::{
    DiffHunk, DiffLine, DiffLineKind, DiffView, parse_hunk_header, parse_hunk_start,
    parse_unified_diff, truncate_lines,
};
pub use execution::{ExecutionStatus, TaskLifecycle, ToolStatus};
pub use execution_result::{
    DiagnosticLevel, ExecutionResult, ToolDiagnostic, ToolOutcome, ToolProvenance, ToolSource,
};
pub use fanout::{FanoutDecision, FanoutPlan, FanoutPredictor, PlannedAgent};
pub use ids::{AgentId, SessionId, TaskId, ToolId};
pub use paging::{PageStore, Pressure, estimate_tokens};
pub use plan_cache::{CachedPlan, PlanCache, normalize_signature};
pub use routing::{cascade_pick, knapsack_select};
pub use task::{TaskInput, TaskStatusPart};
pub use task_store::{
    FactoryMetrics, Task, TaskCounts, TaskError, TaskKind, TaskPatch, TaskRisk, TaskStatus,
    TaskValidation, TodoTaskId,
};
pub use tool_input::{ReplacementMode, ToolInput, ToolInputError};
pub use tool_kind::ToolKind;
pub use tool_retrieval::{IdentityQueryGen, QueryGen, ToolIndex, retrieve_multi, should_defer};
pub use usage::ModelUsage;
pub use workflow_search::{
    Evaluator, Experience, Mutator, SearchResult, argmax, search, soft_mixed_probability,
};

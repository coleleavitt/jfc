//! Provider-neutral JFC domain types.
//!
//! This crate intentionally stays free of terminal UI, HTTP providers, and
//! runtime orchestration. Types move here only after their ownership is stable
//! enough to be shared without dragging `jfc` dependencies with them.

mod agent_def;
mod assertions;
mod attachment;
pub mod attention;
mod compaction;
pub mod context;
pub mod context_budget;
pub mod context_management;
mod decision_quality;
pub mod diff;
pub mod diff_compression;
mod execution;
mod execution_result;
mod fanout;
pub mod hierarchical_compression;
mod ids;
pub mod information_bottleneck;
pub mod kv_cache;
pub mod mcp_elicitation;
mod message;
mod paging;
mod plan_cache;
pub mod position_encoding;
mod prompt_queue;
mod routing;
pub mod semantic_hash;
mod server_tool;
mod status;
mod task;
mod task_store;
mod tool;
pub mod tool_call;
pub mod tool_dispatch_model;
pub mod tool_display;
mod tool_input;
mod tool_kind;
pub mod tool_output;
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
    DiagnosticLevel, ExecutionResult, ToolDiagnostic, ToolErrorCategory, ToolOutcome,
    ToolProvenance, ToolSource,
};
pub use fanout::{FanoutDecision, FanoutPlan, FanoutPredictor, PlannedAgent};
pub use ids::{AgentId, SessionId, TaskId, ToolId};
pub use message::*;
pub use paging::{PageStore, Pressure, estimate_tokens};
pub use plan_cache::{CachedPlan, PlanCache, normalize_signature};
pub use prompt_queue::{
    DEFERRED_TOOL_USES_CAP, DeferredToolUse, MessageQueue, QueuePriority, QueuedPrompt,
    TOOL_USE_SUMMARIES_CAP, ToolUseSummary, push_bounded_drop_oldest, queued_prompt_placeholder,
    should_preserve_prompt,
};
pub use routing::{cascade_pick, knapsack_select};
pub use server_tool::ServerToolResultKind;
pub use status::*;
pub use task::{TaskInput, TaskStatusPart};
pub use task_store::{
    FactoryMetrics, Task, TaskCounts, TaskError, TaskKind, TaskPatch, TaskRisk, TaskStatus,
    TaskValidation, TodoTaskId,
};
pub use tool::*;
pub use tool_call::{InvalidToolTransition, ToolCall, ToolUndoEntry};
pub use tool_display::ToolDisplayState;
pub use tool_input::{CoercionOutcome, ReplacementMode, ToolInput, ToolInputError};
pub use tool_kind::ToolKind;
pub use tool_output::{LargeText, ToolOutput, format_server_tool_result_text_public};
pub use tool_retrieval::{IdentityQueryGen, QueryGen, ToolIndex, retrieve_multi, should_defer};
pub use usage::ModelUsage;
pub use workflow_search::{
    Evaluator, Experience, Mutator, SearchResult, argmax, search, soft_mixed_probability,
};

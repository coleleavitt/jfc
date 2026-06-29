//! Workflow orchestration — deterministic JS script runner (CC 146 parity)
//! plus legacy TOML step-based templates.
//!
//! The JS workflow system runs user-written scripts in a sandboxed boa_engine
//! context. Scripts spawn subagents via `agent()`, with concurrency managed
//! by a tokio Semaphore (min(16, cpus-2) parallel agents, max 1000 total).
//!
//! Legacy TOML workflows (`.jfc/workflows/*.toml`) are still supported via
//! the `/workflow run <name>` slash command.

pub mod engine;
pub mod journal;
pub mod legacy;
pub mod meta;
pub mod permissions;
mod plugin_discovery;
pub mod registry;
pub mod runner;
pub mod task;

// Public module API. Some entries are consumed only by sibling modules or
// reserved for the slash-command/registry surface; export them uniformly.
pub use legacy::{Workflow, WorkflowStep, list, load, render_summary, workflows_dir};
pub use meta::{WorkflowMeta, parse_meta, validate_script};
pub use permissions::{SaveScope, WorkflowPermission, decide, save_workflow};
pub use registry::{
    RegisteredWorkflow, WorkflowSource, discover, list_meta, parse_meta_of, resolve,
};
pub use runner::{WorkflowOutcome, WorkflowRunConfig, run_workflow};
pub use task::{
    AgentProgress, AgentStatus, WorkflowRunStatus, WorkflowTaskProgress, WorkflowTaskState,
    generate_run_id,
};

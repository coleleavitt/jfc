//! Provider-neutral JFC domain types.
//!
//! This crate intentionally stays free of terminal UI, HTTP providers, and
//! runtime orchestration. Types move here only after their ownership is stable
//! enough to be shared without dragging `jfc-ui` dependencies with them.

mod attachment;
mod execution;
mod ids;
mod task;
mod task_store;
mod tool_input;
mod tool_kind;
mod usage;

pub use attachment::{Attachment, AttachmentKind, PastedContent};
pub use execution::{ExecutionStatus, TaskLifecycle, ToolStatus};
pub use ids::{AgentId, SessionId, TaskId, ToolId};
pub use task::{TaskInput, TaskStatusPart};
pub use task_store::{
    Task, TaskCounts, TaskError, TaskKind, TaskPatch, TaskRisk, TaskStatus, TaskValidation,
    TodoTaskId,
};
pub use tool_input::{ReplacementMode, ToolInput, ToolInputError};
pub use tool_kind::ToolKind;
pub use usage::ModelUsage;

//! Tool concurrency scheduler.
//!
//! Groups tool calls from a single model response into batches that respect
//! concurrency safety. Concurrency-safe tools (Read, Glob, Grep) run in
//! parallel up to `MAX_CONCURRENCY`. Non-safe tools (Edit, Write, Bash) run
//! sequentially. Batches are processed in model order:
//!
//!   parallel batch → sequential single → parallel batch → …

use std::path::PathBuf;
use std::sync::Arc;

use tokio::sync::{Mutex, mpsc};

use crate::app::AppEvent;
use crate::context::ReadDedupCache;
use crate::tasks::TaskStore;
use crate::tools::{self, ExecutionResult};
use crate::types::{ToolCall, ToolKind};

/// Maximum number of concurrency-safe tools that run in a single parallel batch.
pub const MAX_CONCURRENCY: usize = 10;

/// A scheduled batch of tool calls.
#[derive(Debug)]
pub enum ToolBatch {
    /// Tools that can execute simultaneously (Read, Glob, Grep, Search).
    Parallel(Vec<ToolCall>),
    /// A single tool that must run alone (Edit, Write, Bash, ApplyPatch).
    Sequential(ToolCall),
}

/// Whether a tool kind is safe to run concurrently with other tools.
///
/// Read-only tools that don't mutate the filesystem or have side effects are
/// concurrency-safe. Write tools, shell commands, and patches are not.
pub fn is_concurrency_safe(kind: &ToolKind) -> bool {
    matches!(
        kind,
        ToolKind::Read
            | ToolKind::Glob
            | ToolKind::Grep
            | ToolKind::Search
            | ToolKind::TaskCreate
            | ToolKind::TaskUpdate
            | ToolKind::TaskList
            | ToolKind::TaskDone
    )
}

/// Group tool calls into ordered batches, preserving model order.
///
/// Adjacent concurrency-safe tools are collapsed into `Parallel` batches
/// (capped at [`MAX_CONCURRENCY`]). Non-safe tools become individual
/// `Sequential` entries. The result maintains the original ordering:
///
/// ```text
/// [Read, Glob, Edit, Read, Read] → [Parallel([Read, Glob]), Sequential(Edit), Parallel([Read, Read])]
/// ```
pub fn schedule_tools(calls: Vec<ToolCall>) -> Vec<ToolBatch> {
    let mut batches: Vec<ToolBatch> = Vec::new();
    let mut safe_buf: Vec<ToolCall> = Vec::new();

    let flush_safe = |buf: &mut Vec<ToolCall>, out: &mut Vec<ToolBatch>| {
        if buf.is_empty() {
            return;
        }
        for chunk in buf.drain(..).collect::<Vec<_>>().chunks(MAX_CONCURRENCY) {
            out.push(ToolBatch::Parallel(chunk.to_vec()));
        }
    };

    for call in calls {
        if is_concurrency_safe(&call.kind) {
            safe_buf.push(call);
        } else {
            flush_safe(&mut safe_buf, &mut batches);
            batches.push(ToolBatch::Sequential(call));
        }
    }
    flush_safe(&mut safe_buf, &mut batches);

    batches
}

/// Result of executing a single tool, carrying its id and output.
pub struct ToolExecution {
    pub tool_id: String,
    pub result: ExecutionResult,
}

/// Execute all batches in order, sending `ToolResult` events for each completion.
///
/// Parallel batches spawn up to `MAX_CONCURRENCY` tasks and join them.
/// Sequential batches run one at a time. The `tx` channel is used to send
/// per-tool `AppEvent::ToolResult` events as each tool finishes.
pub async fn execute_batches(
    batches: Vec<ToolBatch>,
    tx: &mpsc::UnboundedSender<AppEvent>,
    cwd: PathBuf,
    dedup: Arc<Mutex<ReadDedupCache>>,
    task_store: Option<Arc<TaskStore>>,
) -> Vec<ToolExecution> {
    let mut all_results = Vec::new();

    for batch in batches {
        match batch {
            ToolBatch::Parallel(calls) => {
                let mut handles = Vec::with_capacity(calls.len());
                for call in calls {
                    let id = call.id.clone();
                    let kind = call.kind.clone();
                    let input = call.input.clone();
                    let cwd = cwd.clone();
                    let dedup = Arc::clone(&dedup);
                    let ts = task_store.clone();
                    handles.push(tokio::spawn(async move {
                        let result = tools::execute_tool(kind, input, cwd, Some(dedup), ts).await;
                        ToolExecution {
                            tool_id: id,
                            result,
                        }
                    }));
                }
                for handle in handles {
                    if let Ok(exec) = handle.await {
                        let _ = tx.send(AppEvent::ToolResult {
                            tool_id: exec.tool_id.clone(),
                            result: exec.result.clone(),
                        });
                        all_results.push(exec);
                    }
                }
            }
            ToolBatch::Sequential(call) => {
                let id = call.id.clone();
                let kind = call.kind.clone();
                let input = call.input.clone();
                let result = tools::execute_tool(
                    kind,
                    input,
                    cwd.clone(),
                    Some(Arc::clone(&dedup)),
                    task_store.clone(),
                )
                .await;
                let _ = tx.send(AppEvent::ToolResult {
                    tool_id: id.clone(),
                    result: result.clone(),
                });
                all_results.push(ToolExecution {
                    tool_id: id,
                    result,
                });
            }
        }
    }

    all_results
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{ToolInput, ToolOutput, ToolStatus};

    fn make_call(kind: ToolKind, id: &str) -> ToolCall {
        ToolCall {
            id: id.to_owned(),
            kind,
            status: ToolStatus::Pending,
            input: ToolInput::Generic {
                summary: String::new(),
            },
            output: ToolOutput::Empty,
            is_collapsed: false,
        }
    }

    #[test]
    fn schedule_groups_adjacent_safe_tools() {
        let calls = vec![
            make_call(ToolKind::Read, "r1"),
            make_call(ToolKind::Glob, "g1"),
            make_call(ToolKind::Edit, "e1"),
            make_call(ToolKind::Read, "r2"),
            make_call(ToolKind::Read, "r3"),
        ];
        let batches = schedule_tools(calls);
        assert_eq!(batches.len(), 3);
        assert!(matches!(&batches[0], ToolBatch::Parallel(v) if v.len() == 2));
        assert!(matches!(&batches[1], ToolBatch::Sequential(_)));
        assert!(matches!(&batches[2], ToolBatch::Parallel(v) if v.len() == 2));
    }

    #[test]
    fn all_safe_tools_single_batch() {
        let calls = vec![
            make_call(ToolKind::Read, "r1"),
            make_call(ToolKind::Grep, "g1"),
            make_call(ToolKind::Read, "r2"),
        ];
        let batches = schedule_tools(calls);
        assert_eq!(batches.len(), 1);
        assert!(matches!(&batches[0], ToolBatch::Parallel(v) if v.len() == 3));
    }

    #[test]
    fn all_unsafe_tools_individual_batches() {
        let calls = vec![
            make_call(ToolKind::Edit, "e1"),
            make_call(ToolKind::Write, "w1"),
            make_call(ToolKind::Bash, "b1"),
        ];
        let batches = schedule_tools(calls);
        assert_eq!(batches.len(), 3);
        assert!(
            batches
                .iter()
                .all(|b| matches!(b, ToolBatch::Sequential(_)))
        );
    }

    #[test]
    fn empty_input_empty_output() {
        let batches = schedule_tools(vec![]);
        assert!(batches.is_empty());
    }
}

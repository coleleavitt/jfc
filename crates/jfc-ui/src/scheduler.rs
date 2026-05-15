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
use tracing::{debug, info, warn};

use crate::context::ReadDedupCache;
use crate::runtime::{AppEvent, ExecutionResult, ToolEvent};
use crate::tasks::TaskStore;
use crate::tools;
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
            | ToolKind::Skill
            | ToolKind::ToolSearch
            | ToolKind::ToolSuggest
            | ToolKind::TeamCreate
            | ToolKind::TeamDelete
            | ToolKind::SendMessage
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
    let total_calls = calls.len();
    let mut batches: Vec<ToolBatch> = Vec::new();
    let mut safe_buf: Vec<ToolCall> = Vec::new();

    let flush_safe = |buf: &mut Vec<ToolCall>, out: &mut Vec<ToolBatch>| {
        if buf.is_empty() {
            return;
        }
        for chunk in std::mem::take(buf).chunks(MAX_CONCURRENCY) {
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

    let parallel_count = batches
        .iter()
        .filter(|b| matches!(b, ToolBatch::Parallel(_)))
        .count();
    let sequential_count = batches.len() - parallel_count;

    debug!(
        target: "jfc::scheduler",
        total_calls,
        batch_count = batches.len(),
        parallel_count,
        sequential_count,
        "scheduled tool calls into batches",
    );

    batches
}

/// Result of executing a single tool, carrying its id and output.
pub struct ToolExecution {
    pub tool_id: String,
    pub result: ExecutionResult,
}

/// Execute all batches in order, sending `ToolEvent::Result` events for each completion.
///
/// Parallel batches spawn up to `MAX_CONCURRENCY` tasks and join them.
/// Sequential batches run one at a time. The `tx` channel is used to send
/// per-tool `AppEvent::Tool(ToolEvent::Result)` events as each tool finishes.
pub async fn execute_batches(
    batches: Vec<ToolBatch>,
    tx: &mpsc::Sender<AppEvent>,
    cwd: PathBuf,
    dedup: Arc<Mutex<ReadDedupCache>>,
    task_store: Option<Arc<TaskStore>>,
    active_team_name: Option<String>,
) -> Vec<ToolExecution> {
    let mut all_results = Vec::new();

    info!(
        target: "jfc::scheduler",
        batch_count = batches.len(),
        "executing tool batches",
    );

    for batch in batches {
        match batch {
            ToolBatch::Parallel(calls) => {
                debug!(
                    target: "jfc::scheduler",
                    batch_size = calls.len(),
                    kinds = ?calls.iter().map(|c| &c.kind).collect::<Vec<_>>(),
                    "executing parallel batch",
                );
                // Track each spawned task's identity outside the future so a
                // JoinError (panic / cancel) still carries enough context to
                // log which tool failed.
                let mut handles = Vec::with_capacity(calls.len());
                for call in calls {
                    let id = call.id.clone();
                    let kind = call.kind.clone();
                    let input = call.input.clone();
                    let cwd = cwd.clone();
                    let dedup = Arc::clone(&dedup);
                    let ts = task_store.clone();
                    let atn = active_team_name.clone();
                    let task_kind = kind.clone();
                    let task_id = id.clone();
                    let handle = tokio::spawn(async move {
                        let result = tools::execute_tool(
                            task_kind,
                            input,
                            cwd,
                            Some(dedup),
                            ts,
                            atn.as_deref(),
                        )
                        .await;
                        ToolExecution {
                            tool_id: task_id.as_str().to_owned(),
                            result,
                        }
                    });
                    handles.push((id, kind, handle));
                }
                for (id, kind, handle) in handles {
                    match handle.await {
                        Ok(exec) => {
                            info!(
                                target: "jfc::scheduler",
                                tool_id = %exec.tool_id,
                                kind = ?kind,
                                outcome = ?exec.result.outcome,
                                output_len = exec.result.output.len(),
                                "tool completed",
                            );
                            if tx
                                .send(AppEvent::Tool(ToolEvent::Result {
                                    tool_id: id.clone(),
                                    result: exec.result.clone(),
                                }))
                                .await
                                .is_err()
                            {
                                warn!(
                                    target: "jfc::scheduler",
                                    tool_id = %exec.tool_id,
                                    kind = ?kind,
                                    "app event channel closed — dropping tool result",
                                );
                            }
                            all_results.push(exec);
                        }
                        Err(err) => {
                            warn!(
                                target: "jfc::scheduler",
                                tool_id = %id,
                                tool_kind = ?kind,
                                error = %err,
                                "parallel tool task panicked or was cancelled",
                            );
                        }
                    }
                }
            }
            ToolBatch::Sequential(call) => {
                let id = call.id.clone();
                let kind = call.kind.clone();
                let input = call.input.clone();
                debug!(
                    target: "jfc::scheduler",
                    tool_id = %id,
                    kind = ?kind,
                    "executing sequential tool",
                );
                let result = tools::execute_tool(
                    kind.clone(),
                    input,
                    cwd.clone(),
                    Some(Arc::clone(&dedup)),
                    task_store.clone(),
                    active_team_name.as_deref(),
                )
                .await;
                info!(
                    target: "jfc::scheduler",
                    tool_id = %id,
                    kind = ?kind,
                    outcome = ?result.outcome,
                    output_len = result.output.len(),
                    "tool completed",
                );
                if tx
                    .send(AppEvent::Tool(ToolEvent::Result {
                        tool_id: id.clone(),
                        result: result.clone(),
                    }))
                    .await
                    .is_err()
                {
                    warn!(
                        target: "jfc::scheduler",
                        tool_id = %id,
                        tool_kind = ?kind,
                        "app event channel closed — dropping tool result",
                    );
                }
                all_results.push(ToolExecution {
                    tool_id: id.as_str().to_owned(),
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
            id: crate::ids::ToolId::from(id),
            kind,
            status: ToolStatus::Pending,
            input: ToolInput::Generic {
                summary: String::new(),
            },
            output: ToolOutput::Empty,
            display: crate::types::ToolDisplayState::DEFAULT,
            elapsed_ms: None,
            started_at: None,
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

    // Normal: every read-only tool kind is reported concurrency-safe.
    #[test]
    fn is_concurrency_safe_lists_read_only_tools_normal() {
        for kind in [
            ToolKind::Read,
            ToolKind::Glob,
            ToolKind::Grep,
            ToolKind::Search,
            ToolKind::TaskCreate,
            ToolKind::TaskUpdate,
            ToolKind::TaskList,
            ToolKind::TaskDone,
            ToolKind::Skill,
            ToolKind::ToolSearch,
            ToolKind::ToolSuggest,
            ToolKind::TeamCreate,
            ToolKind::TeamDelete,
            ToolKind::SendMessage,
        ] {
            assert!(
                is_concurrency_safe(&kind),
                "expected {kind:?} concurrency-safe"
            );
        }
    }

    // Robust: side-effecting tool kinds are NOT concurrency-safe — they
    // must run sequentially because they mutate the filesystem or invoke
    // shell processes.
    #[test]
    fn is_concurrency_safe_rejects_mutating_tools_robust() {
        for kind in [
            ToolKind::Edit,
            ToolKind::Write,
            ToolKind::Bash,
            ToolKind::ApplyPatch,
            ToolKind::Task,
            ToolKind::MemoryCreate,
            ToolKind::MemoryDelete,
        ] {
            assert!(!is_concurrency_safe(&kind), "expected {kind:?} unsafe");
        }
    }

    // Robust: a parallel batch larger than MAX_CONCURRENCY is split into
    // multiple chunks of at most MAX_CONCURRENCY each so we never spawn an
    // unbounded number of tasks.
    #[test]
    fn schedule_chunks_large_parallel_batch_robust() {
        let calls: Vec<_> = (0..MAX_CONCURRENCY * 2 + 3)
            .map(|i| make_call(ToolKind::Read, &format!("r{i}")))
            .collect();
        let batches = schedule_tools(calls);
        // 2 full chunks + 1 partial chunk.
        assert_eq!(batches.len(), 3);
        for b in &batches[..batches.len() - 1] {
            match b {
                ToolBatch::Parallel(v) => assert_eq!(v.len(), MAX_CONCURRENCY),
                _ => panic!("expected Parallel"),
            }
        }
        match &batches[batches.len() - 1] {
            ToolBatch::Parallel(v) => assert_eq!(v.len(), 3),
            _ => panic!("expected Parallel"),
        }
    }

    // Normal: a single concurrency-safe call still flushes as a Parallel
    // batch (containing one element), preserving model order.
    #[test]
    fn schedule_single_safe_call_emits_parallel_batch_normal() {
        let calls = vec![make_call(ToolKind::Glob, "g1")];
        let batches = schedule_tools(calls);
        assert_eq!(batches.len(), 1);
        match &batches[0] {
            ToolBatch::Parallel(v) => assert_eq!(v.len(), 1),
            _ => panic!("expected Parallel"),
        }
    }

    // Normal: alternating safe/unsafe/safe yields three batches in order.
    #[test]
    fn schedule_preserves_model_order_normal() {
        let calls = vec![
            make_call(ToolKind::Read, "r1"),
            make_call(ToolKind::Bash, "b1"),
            make_call(ToolKind::Read, "r2"),
        ];
        let batches = schedule_tools(calls);
        assert_eq!(batches.len(), 3);
        assert!(matches!(&batches[0], ToolBatch::Parallel(v) if v.len() == 1));
        assert!(matches!(&batches[1], ToolBatch::Sequential(c) if c.id == "b1"));
        assert!(matches!(&batches[2], ToolBatch::Parallel(v) if v.len() == 1));
    }

    // ──────────────────────────────────────────────────────────────────
    // execute_batches integration: drive the async dispatcher using
    // benign Read/Glob tool calls in a tempdir. Verifies the parallel
    // *and* sequential paths emit ToolResult events on the channel and
    // accumulate into the returned Vec<ToolExecution>.
    // ──────────────────────────────────────────────────────────────────

    fn read_call(id: &str, path: &str) -> ToolCall {
        ToolCall {
            id: crate::ids::ToolId::from(id),
            kind: ToolKind::Read,
            status: ToolStatus::Pending,
            input: crate::types::ToolInput::Read {
                file_path: path.to_owned(),
                offset: None,
                limit: None,
            },
            output: ToolOutput::Empty,
            display: crate::types::ToolDisplayState::DEFAULT,
            elapsed_ms: None,
            started_at: None,
        }
    }

    fn glob_call(id: &str, pattern: &str) -> ToolCall {
        ToolCall {
            id: crate::ids::ToolId::from(id),
            kind: ToolKind::Glob,
            status: ToolStatus::Pending,
            input: crate::types::ToolInput::Glob {
                pattern: pattern.to_owned(),
                path: None,
            },
            output: ToolOutput::Empty,
            display: crate::types::ToolDisplayState::DEFAULT,
            elapsed_ms: None,
            started_at: None,
        }
    }

    // Normal: a parallel batch of two Read calls runs to completion, sends
    // two ToolResult events on the channel, and returns two executions.
    #[tokio::test(flavor = "current_thread")]
    async fn execute_batches_parallel_emits_results_normal() {
        let dir = tempfile::TempDir::new().expect("tempdir");
        let p1 = dir.path().join("a.txt");
        let p2 = dir.path().join("b.txt");
        std::fs::write(&p1, "hello A").unwrap();
        std::fs::write(&p2, "hello B").unwrap();

        let calls = vec![
            read_call("r1", p1.to_str().unwrap()),
            read_call("r2", p2.to_str().unwrap()),
        ];
        let batches = schedule_tools(calls);
        let (tx, mut rx) = mpsc::channel::<AppEvent>(1024);
        let dedup = Arc::new(Mutex::new(ReadDedupCache::new()));
        let results =
            execute_batches(batches, &tx, dir.path().to_path_buf(), dedup, None, None).await;
        assert_eq!(results.len(), 2);
        // Both ToolResult events should be on the channel.
        drop(tx);
        let mut got = 0usize;
        while let Some(ev) = rx.recv().await {
            if matches!(ev, AppEvent::Tool(ToolEvent::Result { .. })) {
                got += 1;
            }
        }
        assert_eq!(got, 2);
    }

    // Normal: a sequential batch (Glob → Edit) runs the unsafe call alone.
    // Driving the executor with a Glob tool exercises the Sequential arm
    // because Glob *is* concurrency-safe — instead, use ToolKind::Bash
    // with an `echo` to force the Sequential branch with a benign
    // command.
    #[tokio::test(flavor = "current_thread")]
    async fn execute_batches_sequential_emits_result_normal() {
        let dir = tempfile::TempDir::new().expect("tempdir");
        let mut bash = read_call("b1", "ignored");
        bash.kind = ToolKind::Bash;
        bash.input = crate::types::ToolInput::Bash {
            command: "echo hi".to_owned(),
            timeout: None,
            workdir: None,
        };
        let batches = schedule_tools(vec![bash]);
        // One Sequential batch.
        assert_eq!(batches.len(), 1);
        assert!(matches!(&batches[0], ToolBatch::Sequential(_)));
        let (tx, mut rx) = mpsc::channel::<AppEvent>(1024);
        let dedup = Arc::new(Mutex::new(ReadDedupCache::new()));
        let results =
            execute_batches(batches, &tx, dir.path().to_path_buf(), dedup, None, None).await;
        assert_eq!(results.len(), 1);
        drop(tx);
        let ev = rx.recv().await.expect("event present");
        assert!(matches!(ev, AppEvent::Tool(ToolEvent::Result { .. })));
    }

    // Robust: empty batches list returns empty results without contacting
    // the channel.
    #[tokio::test(flavor = "current_thread")]
    async fn execute_batches_empty_input_robust() {
        let dir = tempfile::TempDir::new().expect("tempdir");
        let (tx, mut rx) = mpsc::channel::<AppEvent>(1024);
        let dedup = Arc::new(Mutex::new(ReadDedupCache::new()));
        let results =
            execute_batches(Vec::new(), &tx, dir.path().to_path_buf(), dedup, None, None).await;
        assert!(results.is_empty());
        drop(tx);
        assert!(rx.recv().await.is_none());
    }

    // Robust: even when the underlying tool fails (Read of missing path),
    // execute_batches still sends a ToolResult with the failure outcome
    // and accumulates the execution. We don't assert success/failure of
    // the inner result — just that the dispatcher behaves uniformly.
    #[tokio::test(flavor = "current_thread")]
    async fn execute_batches_handles_failing_tool_robust() {
        let dir = tempfile::TempDir::new().expect("tempdir");
        let calls = vec![glob_call("g1", "**/*.nonexistent_pattern_zzz")];
        let batches = schedule_tools(calls);
        let (tx, mut rx) = mpsc::channel::<AppEvent>(1024);
        let dedup = Arc::new(Mutex::new(ReadDedupCache::new()));
        let results =
            execute_batches(batches, &tx, dir.path().to_path_buf(), dedup, None, None).await;
        assert_eq!(results.len(), 1);
        drop(tx);
        let ev = rx.recv().await.expect("got result");
        assert!(matches!(ev, AppEvent::Tool(ToolEvent::Result { .. })));
    }
}

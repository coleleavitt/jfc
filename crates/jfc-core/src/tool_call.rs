use super::tool_display::ToolDisplayState;
use super::tool_output::ToolOutput;
use super::{ExecutionStatus, ToolInput, ToolKind};

/// One step in the per-session undo stack. Captured by the tool
/// dispatcher *before* an Edit / Write / MultiEdit / ApplyPatch fires
/// so `/undo` can restore the pre-mutation state. `previous_content =
/// None` means the file didn't exist before — undo deletes it.
#[derive(Debug, Clone)]
pub struct ToolUndoEntry {
    pub file_path: String,
    pub previous_content: Option<String>,
    pub op_label: String,
}

/// Returned by [`ToolCall::mark_running`] and friends when the caller
/// asked for a state transition that the lifecycle enum forbids
/// (e.g. Failed → Running, or any movement out of a terminal state).
/// The Display impl produces a one-line message suitable for logging.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
#[error("invalid ToolCall status transition: {from:?} → {to:?}")]
pub struct InvalidToolTransition {
    pub from: ExecutionStatus,
    pub to: ExecutionStatus,
}

#[derive(Clone, Debug)]
pub struct ToolCall {
    pub id: crate::ids::ToolId,
    pub kind: ToolKind,
    /// Lifecycle status for this tool. Direct assignment is still
    /// permitted because in-flight migration of the codebase requires
    /// it, but new code SHOULD use the [`Self::mark_running`] /
    /// [`Self::mark_completed`] / [`Self::mark_failed`] /
    /// [`Self::mark_cancelled`] transition methods, which validate the
    /// before-state and refuse invalid jumps (e.g. Failed → Running).
    /// The methods centralize the "what state did we come from?"
    /// invariant so no future caller can silently resurrect a
    /// terminal tool.
    pub status: ExecutionStatus,
    pub input: ToolInput,
    pub output: ToolOutput,
    /// Tri-state display mode (collapsed teaser / default body /
    /// expanded body), with an orthogonal pin flag baked into the
    /// states where it's meaningful. Replaces three separate bools
    /// (`is_collapsed`, `expanded`, `pinned`) so the renderer can't
    /// be handed a contradictory pair like "collapsed AND expanded".
    /// See [`ToolDisplayState`] for the variants and their helpers.
    pub display: ToolDisplayState,
    /// Wall-clock millis between the tool's dispatch and its result
    /// landing. `None` while the tool is in flight. Set by the
    /// `ToolResult` handler in `main.rs`. Surfaced in the title as
    /// a muted `[2.3s]` badge so the user can spot slow operations
    /// at a glance.
    pub elapsed_ms: Option<u64>,
    /// Wall-clock instant when the tool transitioned into flight —
    /// captured at construction and used to compute `elapsed_ms` on
    /// completion. Not persisted (recomputing the duration after a
    /// session reload is meaningless), so this isn't serialized.
    pub started_at: Option<std::time::Instant>,
    /// Gemini 3.x thought signature captured from the streaming response
    /// when this tool call was emitted. Echoed back verbatim on replay so
    /// the server's "thinking" context survives across turns.
    /// `None` for non-Gemini providers and for pre-3.x Gemini turns.
    /// See https://ai.google.dev/gemini-api/docs/thought-signatures
    pub thought_signature: Option<String>,
}

impl ToolCall {
    /// Construct a fresh ToolCall in the `Pending` state. Use this
    /// from the stream layer where a tool is just leaving the model
    /// and hasn't been dispatched yet — guarantees the start state is
    /// always a sane `Pending`, never accidentally `Running` or
    /// `Completed`.
    pub fn new_pending(id: crate::ids::ToolId, kind: ToolKind, input: ToolInput) -> Self {
        Self {
            id,
            kind,
            status: ExecutionStatus::Pending,
            input,
            output: ToolOutput::Empty,
            display: ToolDisplayState::DEFAULT,
            elapsed_ms: None,
            started_at: Some(std::time::Instant::now()),
            thought_signature: None,
        }
    }

    /// Attach a Gemini thought signature captured from the SSE stream.
    /// Builder-style so the stream layer can write
    /// `ToolCall::new_pending(...).with_thought_signature(sig)`.
    pub fn with_thought_signature(mut self, signature: Option<String>) -> Self {
        self.thought_signature = signature;
        self
    }

    /// Construct a ToolCall that's already in the `Failed` terminal
    /// state — used by the stream layer when malformed provider input
    /// (bad JSON, schema mismatch) means we never even get to dispatch
    /// the tool. The output carries the diagnostic that will be
    /// shipped back to the model as the tool_result.
    pub fn new_failed(
        id: crate::ids::ToolId,
        kind: ToolKind,
        input: ToolInput,
        output: ToolOutput,
    ) -> Self {
        Self {
            id,
            kind,
            status: ExecutionStatus::Failed,
            input,
            output,
            display: ToolDisplayState::DEFAULT,
            elapsed_ms: None,
            started_at: None,
            thought_signature: None,
        }
    }

    /// Pending → Running. Returns Err if the tool is already in a
    /// terminal state (Completed/Failed/Cancelled). Idempotent on
    /// Running.
    pub fn mark_running(&mut self) -> Result<(), InvalidToolTransition> {
        self.try_transition_to(ExecutionStatus::Running)
    }

    /// {Pending|Running} → Completed. Returns Err on terminal state.
    /// Idempotent on Completed.
    pub fn mark_completed(&mut self) -> Result<(), InvalidToolTransition> {
        self.try_transition_to(ExecutionStatus::Completed)
    }

    /// {Pending|Running} → Failed. Returns Err if the tool is already
    /// in a different terminal state (Completed/Cancelled).
    pub fn mark_failed(&mut self) -> Result<(), InvalidToolTransition> {
        self.try_transition_to(ExecutionStatus::Failed)
    }

    /// {Pending|Running} → Cancelled. Returns Err on a different
    /// terminal state. Used when the user denies a tool or moves on
    /// before it dispatches.
    pub fn mark_cancelled(&mut self) -> Result<(), InvalidToolTransition> {
        self.try_transition_to(ExecutionStatus::Cancelled)
    }

    fn try_transition_to(&mut self, target: ExecutionStatus) -> Result<(), InvalidToolTransition> {
        if !self.status.allows_transition_to(target) {
            return Err(InvalidToolTransition {
                from: self.status,
                to: target,
            });
        }
        self.status = target;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{McpStatus, TaskLifecycle, ToolInput, ToolKind, ToolOutput};

    // ─── TaskLifecycle ────────────────────────────────────────────────────

    #[test]
    fn task_lifecycle_label_normal() {
        assert_eq!(TaskLifecycle::Pending.label(), "pending");
        assert_eq!(TaskLifecycle::Running.label(), "running");
        assert_eq!(TaskLifecycle::Idle.label(), "idle");
        assert_eq!(TaskLifecycle::Completed.label(), "completed");
        assert_eq!(TaskLifecycle::Failed.label(), "failed");
        assert_eq!(TaskLifecycle::Cancelled.label(), "cancelled");
    }

    #[test]
    fn task_lifecycle_is_alive_normal() {
        assert!(TaskLifecycle::Pending.is_alive());
        assert!(TaskLifecycle::Running.is_alive());
        assert!(TaskLifecycle::Idle.is_alive());
        assert!(!TaskLifecycle::Completed.is_alive());
        assert!(!TaskLifecycle::Failed.is_alive());
        assert!(!TaskLifecycle::Cancelled.is_alive());
    }

    #[test]
    fn task_lifecycle_terminal_and_alive_partition_robust() {
        // Every variant must be exactly one of: alive XOR terminal.
        // If a refactor adds a Limbo variant that's neither, this test
        // catches it before we ship a state the agent fan can't display.
        for state in [
            TaskLifecycle::Pending,
            TaskLifecycle::Running,
            TaskLifecycle::Idle,
            TaskLifecycle::Completed,
            TaskLifecycle::Failed,
            TaskLifecycle::Cancelled,
        ] {
            assert_ne!(
                state.is_alive(),
                state.is_terminal(),
                "{state:?} must be exactly one of alive/terminal",
            );
        }
    }

    // ─── McpStatus / LspStatus ────────────────────────────────────────────

    #[test]
    fn mcp_status_labels_normal() {
        assert_eq!(McpStatus::Connected.label(), "Connected");
        assert_eq!(McpStatus::Disabled.label(), "Disabled");
        assert_eq!(McpStatus::Error.label(), "Error");
    }

    // ─── ToolStatus ───────────────────────────────────────────────────────

    #[test]
    fn tool_status_labels_normal() {
        use crate::ToolStatus;
        assert_eq!(ToolStatus::Pending.label(), "pending");
        assert_eq!(ToolStatus::Running.label(), "running");
        assert_eq!(ToolStatus::Completed.label(), "completed");
        assert_eq!(ToolStatus::Failed.label(), "failed");
    }

    #[test]
    fn tool_status_alias_equals_task_lifecycle_normal() {
        // Both names alias the same underlying ExecutionStatus enum.
        use crate::ToolStatus;
        let a: ToolStatus = ToolStatus::Completed;
        let b: TaskLifecycle = TaskLifecycle::Completed;
        assert_eq!(a, b);
    }

    // ─── ExecutionStatus transitions ──────────────────────────────────────

    #[test]
    fn execution_status_is_terminal_complete_normal() {
        assert!(ExecutionStatus::Completed.is_terminal());
        assert!(ExecutionStatus::Failed.is_terminal());
        assert!(ExecutionStatus::Cancelled.is_terminal());
        assert!(!ExecutionStatus::Pending.is_terminal());
        assert!(!ExecutionStatus::Running.is_terminal());
        assert!(!ExecutionStatus::Idle.is_terminal());
    }

    #[test]
    fn execution_status_allows_transition_normal() {
        // Forward edges from non-terminal states: any move is OK,
        // including the Idle exit (Tasks legitimately go Idle → Running
        // when a teammate picks up new mail).
        assert!(ExecutionStatus::Pending.allows_transition_to(ExecutionStatus::Running));
        assert!(ExecutionStatus::Running.allows_transition_to(ExecutionStatus::Completed));
        assert!(ExecutionStatus::Idle.allows_transition_to(ExecutionStatus::Running));
        // Terminal lock-in: nothing leaves Failed/Completed/Cancelled.
        assert!(!ExecutionStatus::Failed.allows_transition_to(ExecutionStatus::Running));
        assert!(!ExecutionStatus::Completed.allows_transition_to(ExecutionStatus::Failed));
        assert!(!ExecutionStatus::Cancelled.allows_transition_to(ExecutionStatus::Pending));
        // Idempotent same-state transitions are allowed (the stream
        // layer occasionally re-asserts the same status on retry).
        assert!(ExecutionStatus::Completed.allows_transition_to(ExecutionStatus::Completed));
        assert!(ExecutionStatus::Failed.allows_transition_to(ExecutionStatus::Failed));
    }

    fn fixture_pending_tool() -> ToolCall {
        ToolCall::new_pending(
            crate::ids::ToolId::from("test-tool-1".to_owned()),
            ToolKind::Bash,
            ToolInput::Bash {
                command: "ls".into(),
                timeout: None,
                workdir: None,
                run_in_background: None,
            },
        )
    }

    #[test]
    fn tool_call_pending_to_running_normal() {
        let mut tc = fixture_pending_tool();
        assert_eq!(tc.status, ExecutionStatus::Pending);
        assert!(tc.mark_running().is_ok());
        assert_eq!(tc.status, ExecutionStatus::Running);
    }

    #[test]
    fn tool_call_pending_to_running_to_completed_normal() {
        let mut tc = fixture_pending_tool();
        tc.mark_running().expect("Pending → Running should succeed");
        tc.mark_completed()
            .expect("Running → Completed should succeed");
        assert_eq!(tc.status, ExecutionStatus::Completed);
    }

    #[test]
    fn tool_call_pending_directly_to_completed_normal() {
        // Some provider streams collapse Pending and skip directly to
        // Completed when a tool was approved + executed faster than
        // the UI can poll. The transition rules allow this.
        let mut tc = fixture_pending_tool();
        tc.mark_completed()
            .expect("Pending → Completed should succeed");
        assert_eq!(tc.status, ExecutionStatus::Completed);
    }

    #[test]
    fn tool_call_failed_to_running_returns_err_robust() {
        let mut tc = fixture_pending_tool();
        tc.mark_failed().unwrap();
        let err = tc
            .mark_running()
            .expect_err("Failed → Running must be refused");
        assert_eq!(err.from, ExecutionStatus::Failed);
        assert_eq!(err.to, ExecutionStatus::Running);
        // Status stays at Failed — refused transitions don't mutate.
        assert_eq!(tc.status, ExecutionStatus::Failed);
    }

    #[test]
    fn tool_call_completed_to_failed_returns_err_robust() {
        let mut tc = fixture_pending_tool();
        tc.mark_completed().unwrap();
        let err = tc
            .mark_failed()
            .expect_err("Completed → Failed must be refused");
        assert_eq!(err.from, ExecutionStatus::Completed);
        assert_eq!(err.to, ExecutionStatus::Failed);
        assert_eq!(tc.status, ExecutionStatus::Completed);
    }

    #[test]
    fn tool_call_cancel_from_pending_normal() {
        let mut tc = fixture_pending_tool();
        tc.mark_cancelled()
            .expect("Pending → Cancelled should succeed");
        assert_eq!(tc.status, ExecutionStatus::Cancelled);
        // Now terminal — further transitions refused.
        assert!(tc.mark_completed().is_err());
    }

    #[test]
    fn tool_call_idempotent_same_state_normal() {
        // Re-asserting the same status doesn't error — protects the
        // stream layer from spurious "you already said Running" panics
        // when the provider replays an event mid-stream.
        let mut tc = fixture_pending_tool();
        tc.mark_running().unwrap();
        tc.mark_running().expect("Running → Running is idempotent");
        assert_eq!(tc.status, ExecutionStatus::Running);
    }

    #[test]
    fn tool_call_new_failed_constructor_normal() {
        // new_failed lands directly in the terminal Failed state for
        // the malformed-input path (stream.rs ToolDone handler).
        let tc = ToolCall::new_failed(
            crate::ids::ToolId::from("toolu_x".to_owned()),
            ToolKind::Bash,
            ToolInput::Generic {
                summary: "(empty input for Bash)".into(),
            },
            ToolOutput::Text("bad JSON".into()),
        );
        assert_eq!(tc.status, ExecutionStatus::Failed);
        assert!(matches!(tc.output, ToolOutput::Text(_)));
    }
}

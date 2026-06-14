/// Canonical lifecycle for both Tool and Task execution.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ExecutionStatus {
    Pending,
    Running,
    /// Started but quiescent. Distinct from `Running` so UI layers can stop
    /// activity indicators without marking work terminal.
    Idle,
    Completed,
    Failed,
    Cancelled,
}

/// Documentation alias for task lifecycle state.
pub type TaskLifecycle = ExecutionStatus;

/// Documentation alias for tool lifecycle state.
pub type ToolStatus = ExecutionStatus;

impl ExecutionStatus {
    pub fn label(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Running => "running",
            Self::Idle => "idle",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }

    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Failed | Self::Cancelled)
    }

    /// Counts as "alive" for fan-out / agent-count purposes.
    pub fn is_alive(self) -> bool {
        matches!(self, Self::Pending | Self::Running | Self::Idle)
    }

    /// Returns true if a transition from `self` to `target` is well-formed.
    pub fn allows_transition_to(self, target: Self) -> bool {
        if self == target {
            return true;
        }
        if self.is_terminal() {
            return false;
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn execution_status_terminal_and_alive_partition_robust() {
        for status in [
            ExecutionStatus::Pending,
            ExecutionStatus::Running,
            ExecutionStatus::Idle,
        ] {
            assert!(status.is_alive());
            assert!(!status.is_terminal());
        }
        for status in [
            ExecutionStatus::Completed,
            ExecutionStatus::Failed,
            ExecutionStatus::Cancelled,
        ] {
            assert!(status.is_terminal());
            assert!(!status.is_alive());
        }
    }

    #[test]
    fn execution_status_rejects_leaving_terminal_robust() {
        assert!(!ExecutionStatus::Failed.allows_transition_to(ExecutionStatus::Running));
        assert!(ExecutionStatus::Failed.allows_transition_to(ExecutionStatus::Failed));
        assert!(ExecutionStatus::Pending.allows_transition_to(ExecutionStatus::Completed));
    }
}

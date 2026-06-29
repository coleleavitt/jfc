//! Pure tool-dispatch state machine helpers.
//!
//! These mirror `rcoq-tests/theorems/ToolDispatch.v` without depending on the
//! async runtime: approval queue semantics, batch transitions, result ordering,
//! advisor decisions, progressive selection, metrics, registry dispatch, and
//! terminal-state guards.

use std::collections::HashSet;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DispatchToolStatus {
    Pending,
    Executing,
    Completed,
    Failed,
    Cancelled,
}

pub fn is_terminal_tool(status: DispatchToolStatus) -> bool {
    matches!(
        status,
        DispatchToolStatus::Completed | DispatchToolStatus::Failed | DispatchToolStatus::Cancelled
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DispatchToolUse {
    pub tool_id: u64,
    pub tool_name: u64,
    pub tool_input: u64,
    pub tool_status: DispatchToolStatus,
    pub tool_requires_approval: bool,
    pub tool_approved: Option<bool>,
}

pub fn batch_ready(batch: &[DispatchToolUse]) -> bool {
    batch
        .iter()
        .all(|tool| !tool.tool_requires_approval || tool.tool_approved == Some(true))
}

pub fn batch_complete(batch: &[DispatchToolUse]) -> bool {
    batch.iter().all(|tool| is_terminal_tool(tool.tool_status))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ApprovalEntry {
    pub tool_id: u64,
    pub decision: Option<bool>,
}

pub fn resolve_head(queue: &[ApprovalEntry], decision: bool) -> Vec<ApprovalEntry> {
    let Some((head, rest)) = queue.split_first() else {
        return Vec::new();
    };
    let mut next = Vec::with_capacity(queue.len());
    next.push(ApprovalEntry {
        tool_id: head.tool_id,
        decision: Some(decision),
    });
    next.extend_from_slice(rest);
    next
}

pub fn advance_queue(queue: &[ApprovalEntry]) -> Vec<ApprovalEntry> {
    match queue.split_first() {
        None => Vec::new(),
        Some((head, rest)) if head.decision.is_some() => rest.to_vec(),
        Some(_) => queue.to_vec(),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BatchState {
    Pending,
    Approving,
    Executing,
    Complete,
    Cancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BatchEvent {
    Start,
    AllApproved,
    Cancelled,
    AllDone,
}

pub fn batch_transition(state: BatchState, event: BatchEvent) -> Option<BatchState> {
    match (state, event) {
        (BatchState::Pending, BatchEvent::Start) => Some(BatchState::Approving),
        (BatchState::Approving, BatchEvent::AllApproved) => Some(BatchState::Executing),
        (BatchState::Approving | BatchState::Executing, BatchEvent::Cancelled) => {
            Some(BatchState::Cancelled)
        }
        (BatchState::Executing, BatchEvent::AllDone) => Some(BatchState::Complete),
        _ => None,
    }
}

pub fn execution_order(batch: &[DispatchToolUse]) -> Vec<u64> {
    batch.iter().map(|tool| tool.tool_id).collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BatchResult {
    pub tool_id: u64,
    pub success: bool,
    pub output: u64,
}

pub fn collect_results(batch: &[DispatchToolUse], results: &[BatchResult]) -> bool {
    execution_order(batch)
        == results
            .iter()
            .map(|result| result.tool_id)
            .collect::<Vec<_>>()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdvisorDecision {
    Proceed,
    Modify(u64),
    Skip,
    AbortBatch,
}

pub fn apply_advisor(
    batch: &[DispatchToolUse],
    decisions: &[AdvisorDecision],
) -> Vec<DispatchToolUse> {
    batch
        .iter()
        .zip(decisions.iter())
        .map(|(tool, decision)| match decision {
            AdvisorDecision::Proceed => tool.clone(),
            AdvisorDecision::Modify(new_input) => DispatchToolUse {
                tool_input: *new_input,
                ..tool.clone()
            },
            AdvisorDecision::Skip | AdvisorDecision::AbortBatch => DispatchToolUse {
                tool_status: DispatchToolStatus::Cancelled,
                ..tool.clone()
            },
        })
        .collect()
}

pub fn has_duplicate(ids: &[u64]) -> bool {
    let mut seen = HashSet::with_capacity(ids.len());
    ids.iter().any(|id| !seen.insert(*id))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProgressiveToolDef {
    pub name: u64,
    pub description: u64,
    pub schema: u64,
}

pub fn matches_intent(intent: Option<u64>, tool: ProgressiveToolDef) -> bool {
    intent == Some(tool.name)
}

pub fn progressive_select(
    catalog: &[ProgressiveToolDef],
    _history: &[u64],
    intent: Option<u64>,
    max_tools: usize,
) -> Vec<ProgressiveToolDef> {
    let mut prioritized: Vec<_> = catalog
        .iter()
        .copied()
        .filter(|tool| matches_intent(intent, *tool))
        .collect();
    prioritized.extend(
        catalog
            .iter()
            .copied()
            .filter(|tool| !matches_intent(intent, *tool)),
    );
    prioritized.into_iter().take(max_tools).collect()
}

pub fn intent_match_count(catalog: &[ProgressiveToolDef], intent: Option<u64>) -> usize {
    catalog
        .iter()
        .filter(|tool| matches_intent(intent, **tool))
        .count()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BatchMetrics {
    pub total_tools: usize,
    pub approved: usize,
    pub rejected: usize,
    pub completed: usize,
    pub failed: usize,
    pub cancelled: usize,
}

pub fn compute_metrics(batch: &[DispatchToolUse]) -> BatchMetrics {
    BatchMetrics {
        total_tools: batch.len(),
        approved: batch
            .iter()
            .filter(|tool| tool.tool_approved == Some(true))
            .count(),
        rejected: batch
            .iter()
            .filter(|tool| tool.tool_approved == Some(false))
            .count(),
        completed: batch
            .iter()
            .filter(|tool| tool.tool_status == DispatchToolStatus::Completed)
            .count(),
        failed: batch
            .iter()
            .filter(|tool| tool.tool_status == DispatchToolStatus::Failed)
            .count(),
        cancelled: batch
            .iter()
            .filter(|tool| tool.tool_status == DispatchToolStatus::Cancelled)
            .count(),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DispatchKind {
    Read,
    Write,
    Bash,
    Glob,
    Task,
}

pub type Handler = u64;
pub type Registry = Vec<(DispatchKind, Handler)>;

pub fn dispatch(registry: &Registry, kind: DispatchKind) -> Option<Handler> {
    registry
        .iter()
        .find(|(registered, _)| *registered == kind)
        .map(|(_, handler)| *handler)
}

pub fn register(registry: &Registry, kind: DispatchKind, handler: Handler) -> Registry {
    let mut next = Vec::with_capacity(registry.len() + 1);
    next.push((kind, handler));
    next.extend_from_slice(registry);
    next
}

pub fn dispatchable(tool: &DispatchToolUse) -> bool {
    !is_terminal_tool(tool.tool_status)
}

pub fn guarded_dispatch(
    registry: &Registry,
    kind: DispatchKind,
    tool: &DispatchToolUse,
) -> Option<Handler> {
    if dispatchable(tool) {
        dispatch(registry, kind)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tool(id: u64, status: DispatchToolStatus) -> DispatchToolUse {
        DispatchToolUse {
            tool_id: id,
            tool_name: id,
            tool_input: 0,
            tool_status: status,
            tool_requires_approval: false,
            tool_approved: None,
        }
    }

    #[test]
    fn approval_queue_resolve_and_advance_match_fifo_rules() {
        let queue = vec![
            ApprovalEntry {
                tool_id: 1,
                decision: None,
            },
            ApprovalEntry {
                tool_id: 2,
                decision: None,
            },
        ];
        assert_eq!(advance_queue(&queue), queue);
        let resolved = resolve_head(&queue, true);
        assert_eq!(resolved[0].decision, Some(true));
        assert_eq!(advance_queue(&resolved), vec![queue[1]]);
    }

    #[test]
    fn terminal_batch_states_are_absorbing() {
        assert_eq!(
            batch_transition(BatchState::Complete, BatchEvent::Start),
            None
        );
        assert_eq!(
            batch_transition(BatchState::Cancelled, BatchEvent::AllApproved),
            None
        );
    }

    #[test]
    fn results_must_match_batch_order() {
        let batch = vec![
            tool(1, DispatchToolStatus::Pending),
            tool(2, DispatchToolStatus::Pending),
        ];
        assert!(collect_results(
            &batch,
            &[
                BatchResult {
                    tool_id: 1,
                    success: true,
                    output: 0,
                },
                BatchResult {
                    tool_id: 2,
                    success: true,
                    output: 0,
                }
            ]
        ));
        assert!(!collect_results(
            &batch,
            &[
                BatchResult {
                    tool_id: 2,
                    success: true,
                    output: 0,
                },
                BatchResult {
                    tool_id: 1,
                    success: true,
                    output: 0,
                }
            ]
        ));
    }

    #[test]
    fn abort_cancels_all_tools() {
        let batch = vec![
            tool(1, DispatchToolStatus::Pending),
            tool(2, DispatchToolStatus::Executing),
        ];
        let result = apply_advisor(
            &batch,
            &[AdvisorDecision::AbortBatch, AdvisorDecision::AbortBatch],
        );
        assert!(
            result
                .iter()
                .all(|tool| tool.tool_status == DispatchToolStatus::Cancelled)
        );
    }

    #[test]
    fn duplicate_detector_false_means_unique_ids() {
        assert!(!has_duplicate(&[1, 2, 3]));
        assert!(has_duplicate(&[1, 2, 1]));
    }

    #[test]
    fn progressive_selection_prioritizes_intent_matches() {
        let catalog = vec![
            ProgressiveToolDef {
                name: 1,
                description: 0,
                schema: 0,
            },
            ProgressiveToolDef {
                name: 2,
                description: 0,
                schema: 0,
            },
        ];
        let selected = progressive_select(
            &catalog,
            &[],
            Some(2),
            intent_match_count(&catalog, Some(2)),
        );
        assert!(selected.contains(&catalog[1]));
    }

    #[test]
    fn complete_metrics_partition_terminal_statuses() {
        let batch = vec![
            tool(1, DispatchToolStatus::Completed),
            tool(2, DispatchToolStatus::Failed),
            tool(3, DispatchToolStatus::Cancelled),
        ];
        assert!(batch_complete(&batch));
        let metrics = compute_metrics(&batch);
        assert_eq!(
            metrics.completed + metrics.failed + metrics.cancelled,
            metrics.total_tools
        );
    }

    #[test]
    fn registry_dispatch_is_deterministic_and_register_round_trips() {
        let registry = register(&Vec::new(), DispatchKind::Read, 10);
        assert_eq!(dispatch(&registry, DispatchKind::Read), Some(10));
        assert_eq!(dispatch(&registry, DispatchKind::Write), None);
    }

    #[test]
    fn guarded_dispatch_refuses_terminal_tools() {
        let registry = register(&Vec::new(), DispatchKind::Read, 10);
        let completed = tool(1, DispatchToolStatus::Completed);
        let pending = tool(1, DispatchToolStatus::Pending);
        assert_eq!(
            guarded_dispatch(&registry, DispatchKind::Read, &completed),
            None
        );
        assert_eq!(
            guarded_dispatch(&registry, DispatchKind::Read, &pending),
            Some(10)
        );
    }

    #[test]
    fn approval_requirements_gate_batch_ready() {
        let mut batch = vec![tool(1, DispatchToolStatus::Pending)];
        batch[0].tool_requires_approval = true;
        assert!(!batch_ready(&batch));
        batch[0].tool_approved = Some(true);
        assert!(batch_ready(&batch));
    }
}

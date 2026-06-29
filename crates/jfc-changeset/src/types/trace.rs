use super::{AgentChangeSet, Approval, ChangeState, TestRun};

pub(super) struct OpenTrace<'a> {
    pub changeset: &'a AgentChangeSet,
    pub base_head_bytes: usize,
    pub branch_bytes: usize,
    pub worktree_path_bytes: usize,
}

pub(super) struct ComputeIdTrace {
    pub base_head_bytes: usize,
    pub branch_bytes: usize,
    pub worktree_path_bytes: usize,
    pub now_ms: u64,
}

pub(super) struct TransitionTrace<'a> {
    pub changeset: &'a AgentChangeSet,
    pub previous: ChangeState,
    pub next: ChangeState,
    pub status: &'static str,
}

pub(super) fn open_inputs(input: OpenTrace<'_>) {
    linkscope::record_items("changeset.opened", 1);
    if !linkscope::trace_detail_enabled() {
        return;
    }
    linkscope::detail_event_fields(
        "changeset.types.open.detail",
        [
            linkscope::TraceField::text("state", input.changeset.state.label()),
            linkscope::TraceField::bytes(
                "base_head_bytes",
                usize_to_u64_saturating(input.base_head_bytes),
            ),
            linkscope::TraceField::bytes(
                "branch_bytes",
                usize_to_u64_saturating(input.branch_bytes),
            ),
            linkscope::TraceField::bytes(
                "worktree_path_bytes",
                usize_to_u64_saturating(input.worktree_path_bytes),
            ),
            linkscope::TraceField::bytes(
                "id_bytes",
                usize_to_u64_saturating(input.changeset.id.len()),
            ),
        ],
    );
}

pub(super) fn compute_id(input: ComputeIdTrace) {
    if !linkscope::trace_detail_enabled() {
        return;
    }
    linkscope::detail_event_fields(
        "changeset.types.compute_id.detail",
        [
            linkscope::TraceField::bytes(
                "base_head_bytes",
                usize_to_u64_saturating(input.base_head_bytes),
            ),
            linkscope::TraceField::bytes(
                "branch_bytes",
                usize_to_u64_saturating(input.branch_bytes),
            ),
            linkscope::TraceField::bytes(
                "worktree_path_bytes",
                usize_to_u64_saturating(input.worktree_path_bytes),
            ),
            linkscope::TraceField::count("now_ms", input.now_ms),
        ],
    );
}

pub(super) fn transition(input: TransitionTrace<'_>) {
    linkscope::record_items("changeset.transition", 1);
    if !linkscope::trace_detail_enabled() {
        return;
    }
    linkscope::detail_event_fields(
        "changeset.types.transition.detail",
        [
            linkscope::TraceField::text("id", input.changeset.id.clone()),
            linkscope::TraceField::text("from", input.previous.label()),
            linkscope::TraceField::text("to", input.next.label()),
            linkscope::TraceField::text("status", input.status),
            linkscope::TraceField::count(
                "changed_files",
                usize_to_u64_saturating(input.changeset.changed_files.len()),
            ),
            linkscope::TraceField::count(
                "test_runs",
                usize_to_u64_saturating(input.changeset.test_runs.len()),
            ),
        ],
    );
}

pub(super) fn change_content(changeset: &AgentChangeSet) {
    let insertions: u64 = changeset
        .changed_files
        .iter()
        .map(|file| u64::from(file.insertions))
        .sum();
    let deletions: u64 = changeset
        .changed_files
        .iter()
        .map(|file| u64::from(file.deletions))
        .sum();
    linkscope::record_items(
        "changeset.changed_files",
        usize_to_u64_saturating(changeset.changed_files.len()),
    );
    if !linkscope::trace_detail_enabled() {
        return;
    }
    linkscope::detail_event_fields(
        "changeset.types.content.detail",
        [
            linkscope::TraceField::text("id", changeset.id.clone()),
            linkscope::TraceField::count("insertions", insertions),
            linkscope::TraceField::count("deletions", deletions),
            linkscope::TraceField::bytes(
                "diff_summary_bytes",
                usize_to_u64_saturating(changeset.diff_summary.len()),
            ),
            linkscope::TraceField::count("has_patch", u64::from(changeset.patch_path.is_some())),
        ],
    );
}

pub(super) fn test_run(run: &TestRun, passed: bool) {
    if !linkscope::trace_detail_enabled() {
        return;
    }
    linkscope::detail_event_fields(
        "changeset.types.test_run.detail",
        [
            linkscope::TraceField::bytes(
                "command_bytes",
                usize_to_u64_saturating(run.command.len()),
            ),
            linkscope::TraceField::signed("exit_code", i64::from(run.exit_code)),
            linkscope::TraceField::count("duration_ms", run.duration_ms),
            linkscope::TraceField::count("passed", u64::from(passed)),
        ],
    );
}

pub(super) fn test_summary(changeset: &AgentChangeSet, passed: bool) {
    if !linkscope::trace_detail_enabled() {
        return;
    }
    linkscope::detail_event_fields(
        "changeset.types.tests.summary",
        [
            linkscope::TraceField::text("id", changeset.id.clone()),
            linkscope::TraceField::count(
                "test_runs",
                usize_to_u64_saturating(changeset.test_runs.len()),
            ),
            linkscope::TraceField::count("passed", u64::from(passed)),
        ],
    );
}

pub(super) fn approval(approval: &Approval) {
    linkscope::record_items(approval_metric_label(approval), 1);
    if !linkscope::trace_detail_enabled() {
        return;
    }
    let (confirmations, total) = match approval {
        Approval::Human { .. } => (0, 0),
        Approval::ValidatorQuorum {
            confirmations,
            total,
            ..
        } => (*confirmations, *total),
    };
    linkscope::detail_event_fields(
        "changeset.types.approval.detail",
        [
            linkscope::TraceField::text("kind", approval.label()),
            linkscope::TraceField::count("confirmations", u64::from(confirmations)),
            linkscope::TraceField::count("total", u64::from(total)),
        ],
    );
}

fn approval_metric_label(approval: &Approval) -> &'static str {
    match approval {
        Approval::Human { .. } => "changeset.approval.human",
        Approval::ValidatorQuorum { .. } => "changeset.approval.validator_quorum",
    }
}

fn usize_to_u64_saturating(value: usize) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
}

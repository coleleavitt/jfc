use super::{AgentRole, AgentState, AgentStatus};

pub(super) fn status_classification(label: &'static str, status: AgentStatus, value: bool) {
    linkscope::record_items(
        if value {
            "agent.status.classification.true"
        } else {
            "agent.status.classification.false"
        },
        1,
    );
    if !linkscope::trace_detail_enabled() {
        return;
    }
    linkscope::detail_event_fields(
        label,
        [
            linkscope::TraceField::text("status", status.label()),
            linkscope::TraceField::count("value", u64::from(value)),
        ],
    );
}

pub(super) fn role_accessor(label: &'static str, role: &AgentRole, value_bytes: Option<usize>) {
    linkscope::record_items(
        if value_bytes.is_some() {
            "agent.role.accessor.hit"
        } else {
            "agent.role.accessor.miss"
        },
        1,
    );
    if !linkscope::trace_detail_enabled() {
        return;
    }
    linkscope::detail_event_fields(
        label,
        [
            linkscope::TraceField::text("role", role.label()),
            linkscope::TraceField::count("has_value", u64::from(value_bytes.is_some())),
            linkscope::TraceField::bytes(
                "value_bytes",
                usize_to_u64_saturating(value_bytes.unwrap_or_default()),
            ),
        ],
    );
}

pub(super) fn state(label: &'static str, state: &AgentState) {
    linkscope::record_items(state_metric_label(state.status), 1);
    if !linkscope::trace_detail_enabled() {
        return;
    }
    linkscope::detail_event_fields(
        label,
        [
            linkscope::TraceField::text("id", state.id.uuid().to_string()),
            linkscope::TraceField::text("status", state.status.label()),
            linkscope::TraceField::text("role", state.role.label()),
            linkscope::TraceField::bytes(
                "description_bytes",
                usize_to_u64_saturating(state.description.len()),
            ),
            linkscope::TraceField::count("tokens", state.token_count),
            linkscope::TraceField::count("tools", u64::from(state.tool_use_count)),
            linkscope::TraceField::count("has_summary", u64::from(state.summary.is_some())),
            linkscope::TraceField::count("has_error", u64::from(state.error.is_some())),
        ],
    );
}

pub(super) fn role_metric_label(role: &str) -> &'static str {
    match role {
        "solo" => "agent.role.solo",
        "teammate" => "agent.role.teammate",
        "solver" => "agent.role.solver",
        "validator" => "agent.role.validator",
        "council" => "agent.role.council",
        _ => "agent.role.unknown",
    }
}

fn state_metric_label(status: AgentStatus) -> &'static str {
    match status {
        AgentStatus::Pending => "agent.state.pending",
        AgentStatus::Running => "agent.state.running",
        AgentStatus::Idle => "agent.state.idle",
        AgentStatus::Completed => "agent.state.completed",
        AgentStatus::Failed => "agent.state.failed",
        AgentStatus::Cancelled => "agent.state.cancelled",
    }
}

fn usize_to_u64_saturating(value: usize) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
}

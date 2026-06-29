use crate::DashboardSnapshot;

pub(crate) struct RequestShape<'a> {
    pub method: &'a str,
    pub route: &'static str,
    pub target_bytes: usize,
    pub head: bool,
}

pub(crate) fn record_snapshot(label: &'static str, snapshot: &DashboardSnapshot) {
    linkscope::record_items(label, usize_to_u64_saturating(snapshot.timeline.len()));
    if !linkscope::trace_detail_enabled() {
        return;
    }
    linkscope::detail_event_fields(
        label,
        [
            linkscope::TraceField::count("has_session", bool_to_u64(snapshot.session_id.is_some())),
            linkscope::TraceField::count("has_model", bool_to_u64(snapshot.model.is_some())),
            linkscope::TraceField::count("context_used_tokens", snapshot.context_used_tokens),
            linkscope::TraceField::count("context_window_tokens", snapshot.context_window_tokens),
            linkscope::TraceField::count(
                "usage_rows",
                usize_to_u64_saturating(snapshot.usage_by_model.len()),
            ),
            linkscope::TraceField::count(
                "timeline",
                usize_to_u64_saturating(snapshot.timeline.len()),
            ),
            linkscope::TraceField::count(
                "profile",
                usize_to_u64_saturating(snapshot.profile.len()),
            ),
            linkscope::TraceField::count(
                "timeline_flags",
                usize_to_u64_saturating(
                    snapshot
                        .timeline
                        .iter()
                        .map(|sample| sample.flags.len())
                        .sum(),
                ),
            ),
            linkscope::TraceField::count("rsi_candidates", snapshot.rsi_funnel.candidates),
            linkscope::TraceField::count("rsi_verified", snapshot.rsi_funnel.verified),
            linkscope::TraceField::count("rsi_active", snapshot.rsi_funnel.active),
        ],
    );
}

pub(crate) fn record_request(input: RequestShape<'_>) {
    linkscope::record_items("dashboard.server.request", 1);
    if !linkscope::trace_detail_enabled() {
        return;
    }
    linkscope::detail_event_fields(
        "dashboard.server.request",
        [
            linkscope::TraceField::text("method", input.method),
            linkscope::TraceField::text("route", input.route),
            linkscope::TraceField::bytes(
                "target_bytes",
                usize_to_u64_saturating(input.target_bytes),
            ),
            linkscope::TraceField::count("head", bool_to_u64(input.head)),
        ],
    );
}

pub(crate) fn route_label(target: &str) -> &'static str {
    match target.split('?').next().unwrap_or(target) {
        "/" | "/index.html" => "index",
        "/health" => "health",
        "/api/snapshot" => "snapshot",
        _ => "not_found",
    }
}

pub(crate) fn record_server_bind(label: &'static str, addr_bytes: usize) {
    linkscope::record_items(label, 1);
    if !linkscope::trace_detail_enabled() {
        return;
    }
    linkscope::detail_event_fields(
        label,
        [linkscope::TraceField::bytes(
            "addr_bytes",
            usize_to_u64_saturating(addr_bytes),
        )],
    );
}

pub(crate) fn record_response(status: u16, content_type: &str, body_bytes: usize, head_only: bool) {
    linkscope::record_items("dashboard.server.respond", 1);
    if !linkscope::trace_detail_enabled() {
        return;
    }
    linkscope::detail_event_fields(
        "dashboard.server.respond",
        [
            linkscope::TraceField::count("status", u64::from(status)),
            linkscope::TraceField::text("content_type", content_type),
            linkscope::TraceField::bytes("body_bytes", usize_to_u64_saturating(body_bytes)),
            linkscope::TraceField::count("head_only", bool_to_u64(head_only)),
        ],
    );
}

fn bool_to_u64(value: bool) -> u64 {
    u64::from(value)
}

fn usize_to_u64_saturating(value: usize) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn route_label_classifies_without_query_payload_normal() {
        assert_eq!(route_label("/"), "index");
        assert_eq!(route_label("/index.html?private=query"), "index");
        assert_eq!(route_label("/api/snapshot?private=query"), "snapshot");
        assert_eq!(route_label("/health"), "health");
        assert_eq!(route_label("/private-path?secret=value"), "not_found");
    }
}

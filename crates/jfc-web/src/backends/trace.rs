use std::time::Duration;

pub(super) struct BackendStart<'a> {
    pub(super) backend: &'static str,
    pub(super) query: &'a str,
    pub(super) max_results: usize,
}

pub(super) struct BackendResultTrace {
    pub(super) backend: &'static str,
    pub(super) status: &'static str,
    pub(super) results: usize,
}

pub(super) struct PageRequest<'a> {
    pub(super) backend: &'static str,
    pub(super) query: &'a str,
    pub(super) offset: usize,
    pub(super) limit: usize,
}

pub(super) fn backend_start(input: BackendStart<'_>) {
    if !linkscope::trace_detail_enabled() {
        return;
    }
    linkscope::detail_event_fields(
        "web.backend.start",
        [
            linkscope::TraceField::text("backend", input.backend),
            linkscope::TraceField::bytes("query_bytes", len_to_u64(input.query.len())),
            linkscope::TraceField::count("max_results", len_to_u64(input.max_results)),
        ],
    );
}

pub(super) fn backend_result(input: BackendResultTrace) {
    if !linkscope::trace_detail_enabled() {
        return;
    }
    linkscope::detail_event_fields(
        "web.backend.result",
        [
            linkscope::TraceField::text("backend", input.backend),
            linkscope::TraceField::text("status", input.status),
            linkscope::TraceField::count("results", len_to_u64(input.results)),
        ],
    );
}

pub(super) fn page_request(input: PageRequest<'_>) {
    if !linkscope::trace_detail_enabled() {
        return;
    }
    linkscope::detail_event_fields(
        "web.backend.page",
        [
            linkscope::TraceField::text("backend", input.backend),
            linkscope::TraceField::bytes("query_bytes", len_to_u64(input.query.len())),
            linkscope::TraceField::count("offset", len_to_u64(input.offset)),
            linkscope::TraceField::count("limit", len_to_u64(input.limit)),
        ],
    );
}

pub(super) fn timeout(backend: &'static str, timeout: Duration) {
    if !linkscope::trace_detail_enabled() {
        return;
    }
    linkscope::detail_event_fields(
        "web.backend.timeout_config",
        [
            linkscope::TraceField::text("backend", backend),
            linkscope::TraceField::count("timeout_ms", duration_ms(timeout)),
        ],
    );
}

pub(super) fn count(label: &'static str, value: usize) {
    linkscope::record_items(label, len_to_u64(value));
}

pub(super) fn bytes(label: &'static str, value: usize) {
    linkscope::record_bytes(label, len_to_u64(value));
}

fn duration_ms(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

fn len_to_u64(value: usize) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backend_start_records_shape_without_query_payload_normal() {
        linkscope::trace_detail_enable();

        backend_start(BackendStart {
            backend: "example",
            query: "private search terms",
            max_results: 7,
        });

        let snapshot = linkscope::snapshot();
        let trace = snapshot
            .traces
            .iter()
            .find(|trace| trace.label == "web.backend.start")
            .expect("backend start trace should exist");
        assert!(trace.fields.iter().any(|field| field.name == "query_bytes"));
        assert!(
            !trace
                .fields
                .iter()
                .any(|field| field.value == "private search terms")
        );
    }
}

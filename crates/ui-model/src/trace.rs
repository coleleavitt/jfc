pub(crate) struct CollectionChange {
    pub label: &'static str,
    pub item_bytes_label: &'static str,
    pub item_bytes: usize,
    pub before: usize,
    pub after: usize,
}

pub(crate) struct NamedShape {
    pub label: &'static str,
    pub kind_label: &'static str,
    pub kind: &'static str,
    pub text_label: &'static str,
    pub text_bytes: usize,
}

pub(crate) struct StatusSegmentTrace {
    pub label: &'static str,
    pub tone: &'static str,
    pub priority: u8,
    pub text_bytes: usize,
}

pub(crate) fn record_count(label: &'static str, count: usize) {
    linkscope::record_items(label, usize_to_u64_saturating(count));
}

pub(crate) fn record_text_shape(label: &'static str, field: &'static str, bytes: usize) {
    record_count(label, 1);
    if !linkscope::trace_detail_enabled() {
        return;
    }
    linkscope::detail_event_fields(
        label,
        [linkscope::TraceField::bytes(
            field,
            usize_to_u64_saturating(bytes),
        )],
    );
}

pub(crate) fn record_named_shape(input: NamedShape) {
    record_count(input.label, 1);
    if !linkscope::trace_detail_enabled() {
        return;
    }
    linkscope::detail_event_fields(
        input.label,
        [
            linkscope::TraceField::text(input.kind_label, input.kind),
            linkscope::TraceField::bytes(
                input.text_label,
                usize_to_u64_saturating(input.text_bytes),
            ),
        ],
    );
}

pub(crate) fn record_status_segment(input: StatusSegmentTrace) {
    record_count(input.label, 1);
    if !linkscope::trace_detail_enabled() {
        return;
    }
    linkscope::detail_event_fields(
        input.label,
        [
            linkscope::TraceField::text("tone", input.tone),
            linkscope::TraceField::count("priority", u64::from(input.priority)),
            linkscope::TraceField::bytes("text_bytes", usize_to_u64_saturating(input.text_bytes)),
        ],
    );
}

pub(crate) fn record_collection_change(input: CollectionChange) {
    record_count(input.label, input.after);
    if !linkscope::trace_detail_enabled() {
        return;
    }
    linkscope::detail_event_fields(
        input.label,
        [
            linkscope::TraceField::bytes(
                input.item_bytes_label,
                usize_to_u64_saturating(input.item_bytes),
            ),
            linkscope::TraceField::count("rows_before", usize_to_u64_saturating(input.before)),
            linkscope::TraceField::count("rows_after", usize_to_u64_saturating(input.after)),
        ],
    );
}

fn usize_to_u64_saturating(value: usize) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
}

use crate::{CompartmentRange, CompartmentTier, ContextAccount, ContextContributor};

pub(crate) struct TextShape {
    pub label: &'static str,
    pub field: &'static str,
    pub bytes: usize,
}

pub(crate) struct RangeShape {
    pub label: &'static str,
    pub range: CompartmentRange,
    pub items: usize,
}

pub(crate) fn record_text_shape(input: TextShape) {
    linkscope::record_items(input.label, 1);
    if !linkscope::trace_detail_enabled() {
        return;
    }
    linkscope::detail_event_fields(
        input.label,
        [linkscope::TraceField::bytes(
            input.field,
            usize_to_u64_saturating(input.bytes),
        )],
    );
}

pub(crate) fn record_contributor(label: &'static str, contributor: &ContextContributor) {
    linkscope::record_items(label, 1);
    if !linkscope::trace_detail_enabled() {
        return;
    }
    linkscope::detail_event_fields(
        label,
        [
            linkscope::TraceField::bytes(
                "id_bytes",
                usize_to_u64_saturating(contributor.id().as_str().len()),
            ),
            linkscope::TraceField::bytes(
                "label_bytes",
                usize_to_u64_saturating(contributor.label().len()),
            ),
            linkscope::TraceField::count("tokens", contributor.tokens()),
        ],
    );
}

pub(crate) fn record_account(label: &'static str, account: &ContextAccount) {
    linkscope::record_items(label, usize_to_u64_saturating(account.contributors().len()));
    if !linkscope::trace_detail_enabled() {
        return;
    }
    linkscope::detail_event_fields(
        label,
        [
            linkscope::TraceField::count(
                "contributors",
                usize_to_u64_saturating(account.contributors().len()),
            ),
            linkscope::TraceField::count("total_tokens", account.total_tokens()),
            linkscope::TraceField::count("empty", bool_to_u64(account.is_empty())),
        ],
    );
}

pub(crate) fn record_range_shape(input: RangeShape) {
    linkscope::record_items(input.label, usize_to_u64_saturating(input.items));
    if !linkscope::trace_detail_enabled() {
        return;
    }
    linkscope::detail_event_fields(
        input.label,
        [
            linkscope::TraceField::count("start", input.range.start().get()),
            linkscope::TraceField::count("end", input.range.end().get()),
            linkscope::TraceField::count("items", usize_to_u64_saturating(input.items)),
        ],
    );
}

pub(crate) fn record_compartment(
    label: &'static str,
    tier: CompartmentTier,
    range: CompartmentRange,
    events: usize,
) {
    linkscope::record_items(label, usize_to_u64_saturating(events));
    if !linkscope::trace_detail_enabled() {
        return;
    }
    linkscope::detail_event_fields(
        label,
        [
            linkscope::TraceField::text("tier", tier_label(tier)),
            linkscope::TraceField::count("start", range.start().get()),
            linkscope::TraceField::count("end", range.end().get()),
            linkscope::TraceField::count("events", usize_to_u64_saturating(events)),
        ],
    );
}

pub(crate) fn record_sequence(
    label: &'static str,
    compartments: usize,
    first_start: u64,
    last_end: u64,
) {
    linkscope::record_items(label, usize_to_u64_saturating(compartments));
    if !linkscope::trace_detail_enabled() {
        return;
    }
    linkscope::detail_event_fields(
        label,
        [
            linkscope::TraceField::count("compartments", usize_to_u64_saturating(compartments)),
            linkscope::TraceField::count("first_start", first_start),
            linkscope::TraceField::count("last_end", last_end),
        ],
    );
}

pub(crate) fn record_status(label: &'static str, status: &'static str) {
    linkscope::record_items(label, 1);
    if !linkscope::trace_detail_enabled() {
        return;
    }
    linkscope::detail_event_fields(label, [linkscope::TraceField::text("status", status)]);
}

fn tier_label(tier: CompartmentTier) -> &'static str {
    match tier {
        CompartmentTier::Recent => "recent",
        CompartmentTier::Warm => "warm",
        CompartmentTier::Cold => "cold",
        CompartmentTier::Archived => "archived",
    }
}

fn bool_to_u64(value: bool) -> u64 {
    u64::from(value)
}

fn usize_to_u64_saturating(value: usize) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
}

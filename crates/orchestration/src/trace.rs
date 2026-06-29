pub(crate) struct NamedTrace<'a> {
    pub label: &'static str,
    pub id: &'a str,
    pub kind: &'static str,
    pub value_label: &'static str,
    pub value_bytes: usize,
}

pub(crate) struct CollectionTrace<'a> {
    pub label: &'static str,
    pub id: &'a str,
    pub item_label: &'static str,
    pub items: usize,
}

pub(crate) struct EventTrace<'a> {
    pub sequence: u64,
    pub module: &'a str,
    pub kind: &'a str,
    pub actor_bytes: usize,
    pub summary_bytes: usize,
}

pub(crate) fn record_named(input: NamedTrace<'_>) {
    linkscope::record_items(input.label, 1);
    if !linkscope::trace_detail_enabled() {
        return;
    }
    linkscope::detail_event_fields(
        input.label,
        [
            linkscope::TraceField::bytes("id_bytes", usize_to_u64_saturating(input.id.len())),
            linkscope::TraceField::text("kind", input.kind),
            linkscope::TraceField::text("value", input.value_label),
            linkscope::TraceField::bytes("value_bytes", usize_to_u64_saturating(input.value_bytes)),
        ],
    );
}

pub(crate) fn record_collection(input: CollectionTrace<'_>) {
    linkscope::record_items(input.label, usize_to_u64_saturating(input.items));
    if !linkscope::trace_detail_enabled() {
        return;
    }
    linkscope::detail_event_fields(
        input.label,
        [
            linkscope::TraceField::bytes("id_bytes", usize_to_u64_saturating(input.id.len())),
            linkscope::TraceField::text("items_label", input.item_label),
            linkscope::TraceField::count("items", usize_to_u64_saturating(input.items)),
        ],
    );
}

pub(crate) fn record_event_shape(input: EventTrace<'_>) {
    linkscope::record_items("orchestration.event.created", 1);
    if !linkscope::trace_detail_enabled() {
        return;
    }
    linkscope::detail_event_fields(
        "orchestration.event.new",
        [
            linkscope::TraceField::count("sequence", input.sequence),
            linkscope::TraceField::text("module", input.module),
            linkscope::TraceField::text("kind", input.kind),
            linkscope::TraceField::bytes("actor_bytes", usize_to_u64_saturating(input.actor_bytes)),
            linkscope::TraceField::bytes(
                "summary_bytes",
                usize_to_u64_saturating(input.summary_bytes),
            ),
        ],
    );
}

pub(crate) fn record_layout(label: &'static str, modules: usize) {
    linkscope::record_items(label, usize_to_u64_saturating(modules));
    if !linkscope::trace_detail_enabled() {
        return;
    }
    linkscope::detail_event_fields(
        label,
        [linkscope::TraceField::count(
            "modules",
            usize_to_u64_saturating(modules),
        )],
    );
}

pub(crate) fn record_layout_complete(modules: usize, complete: bool) {
    linkscope::record_items(
        if complete {
            "orchestration.layout.complete"
        } else {
            "orchestration.layout.incomplete"
        },
        1,
    );
    if !linkscope::trace_detail_enabled() {
        return;
    }
    linkscope::detail_event_fields(
        "orchestration.layout.complete.detail",
        [
            linkscope::TraceField::count("modules", usize_to_u64_saturating(modules)),
            linkscope::TraceField::count("complete", u64::from(complete)),
        ],
    );
}

pub(crate) fn record_error(label: &'static str, status: &'static str) {
    linkscope::record_items(label, 1);
    if !linkscope::trace_detail_enabled() {
        return;
    }
    linkscope::detail_event_fields(
        label,
        [
            linkscope::TraceField::text("status", status),
            linkscope::TraceField::count("ok", 0),
        ],
    );
}

fn usize_to_u64_saturating(value: usize) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
}

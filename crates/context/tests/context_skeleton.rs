use jfc_context::{
    Compartment, CompartmentFingerprint, CompartmentRange, CompartmentSequence, CompartmentTier,
    ContextContributor, ContextHealth, ContextHealthStatus, ContextLayout, ContextModule,
    ContributorId, HistoryAnchor, HistoryEvent, HistoryEventIndex, MemoryAnchor, ReducePlan,
    SearchQuery,
};

#[test]
fn context_layout_exports_all_destination_modules_normal() {
    let layout = ContextLayout::destination_skeleton();

    assert_eq!(
        layout.modules(),
        &[
            ContextModule::Contributors,
            ContextModule::Health,
            ContextModule::Memory,
            ContextModule::History,
            ContextModule::Reduce,
            ContextModule::Search,
        ]
    );
    assert!(layout.is_complete_destination_skeleton());

    let _: ContextContributor = ContextContributor::new(
        ContributorId::new("builtin.session-history").expect("valid contributor id"),
        "session history",
    );
    let _: MemoryAnchor = MemoryAnchor::new("memory://project").expect("valid memory anchor");
    let _: HistoryAnchor = HistoryAnchor::new("session://latest").expect("valid history anchor");
    let _: ReducePlan = ReducePlan::new("protected-tail").expect("valid reduce plan");
    let _: SearchQuery = SearchQuery::new("recent decisions").expect("valid search query");
}

#[test]
fn context_health_record_constructs_real_dto_normal() {
    let layout = ContextLayout::destination_skeleton();
    let contributor = ContextContributor::new(
        ContributorId::new("builtin.memory").expect("valid contributor id"),
        "memory skeleton",
    );
    let health = ContextHealth::new(layout, ContextHealthStatus::Healthy, vec![contributor])
        .expect("complete layout with contributor is healthy");

    assert_eq!(health.status(), ContextHealthStatus::Healthy);
    assert_eq!(health.contributors().len(), 1);
    assert!(health.layout().is_complete_destination_skeleton());
}

#[test]
fn context_health_rejects_incomplete_layout_malformed() {
    let layout = ContextLayout::new([ContextModule::Health]).expect("non-empty layout");
    let contributor = ContextContributor::new(
        ContributorId::new("builtin.health").expect("valid contributor id"),
        "health skeleton",
    );

    let error = ContextHealth::new(layout, ContextHealthStatus::Healthy, vec![contributor])
        .expect_err("health records require the destination skeleton layout");

    assert_eq!(
        error.to_string(),
        "context layout is missing destination modules"
    );
}

#[test]
fn history_compartment_sequence_accepts_contiguous_ranges_normal() {
    let first = Compartment::new(
        CompartmentTier::Recent,
        CompartmentRange::new(HistoryEventIndex::new(0), HistoryEventIndex::new(2))
            .expect("non-empty first range"),
        CompartmentFingerprint::new("fp-0001").expect("valid first fingerprint"),
        vec![
            HistoryEvent::new(
                HistoryEventIndex::new(0),
                CompartmentFingerprint::new("event-0000").expect("valid event fingerprint"),
            ),
            HistoryEvent::new(
                HistoryEventIndex::new(1),
                CompartmentFingerprint::new("event-0001").expect("valid event fingerprint"),
            ),
        ],
    )
    .expect("events cover the first range");
    let second = Compartment::new(
        CompartmentTier::Warm,
        CompartmentRange::new(HistoryEventIndex::new(2), HistoryEventIndex::new(4))
            .expect("non-empty second range"),
        CompartmentFingerprint::new("fp-0002").expect("valid second fingerprint"),
        vec![
            HistoryEvent::new(
                HistoryEventIndex::new(2),
                CompartmentFingerprint::new("event-0002").expect("valid event fingerprint"),
            ),
            HistoryEvent::new(
                HistoryEventIndex::new(3),
                CompartmentFingerprint::new("event-0003").expect("valid event fingerprint"),
            ),
        ],
    )
    .expect("events cover the second range");

    let sequence = CompartmentSequence::new(vec![first, second])
        .expect("adjacent compartments form a valid sequence");

    assert_eq!(sequence.compartments().len(), 2);
    assert_eq!(sequence.compartments()[0].tier(), CompartmentTier::Recent);
    assert_eq!(
        sequence.compartments()[1].range().start(),
        HistoryEventIndex::new(2)
    );
}

#[test]
fn history_compartment_sequence_rejects_overlapping_ranges_malformed() {
    let compartments = vec![
        compartment(0, 3, "fp-overlap-left"),
        compartment(2, 4, "fp-overlap-right"),
    ];

    let error = CompartmentSequence::new(compartments)
        .expect_err("overlapping compartment ranges must be rejected");

    assert_eq!(error.to_string(), "compartment ranges overlap");
}

#[test]
fn history_compartment_sequence_rejects_gapped_ranges_malformed() {
    let compartments = vec![
        compartment(0, 2, "fp-gap-left"),
        compartment(3, 5, "fp-gap-right"),
    ];

    let error = CompartmentSequence::new(compartments)
        .expect_err("gapped compartment ranges must be rejected");

    assert_eq!(error.to_string(), "compartment ranges must be contiguous");
}

#[test]
fn history_fingerprint_rejects_blank_or_spaced_input_malformed() {
    assert_eq!(
        CompartmentFingerprint::new("   ")
            .expect_err("blank fingerprints are invalid")
            .to_string(),
        "compartment fingerprint cannot be empty"
    );
    assert_eq!(
        CompartmentFingerprint::new("fp with spaces")
            .expect_err("fingerprints cannot contain whitespace")
            .to_string(),
        "compartment fingerprint cannot contain whitespace"
    );
}

#[test]
fn history_compartment_rejects_events_outside_range_malformed() {
    let error = Compartment::new(
        CompartmentTier::Archived,
        CompartmentRange::new(HistoryEventIndex::new(10), HistoryEventIndex::new(12))
            .expect("non-empty range"),
        CompartmentFingerprint::new("fp-events").expect("valid fingerprint"),
        vec![HistoryEvent::new(
            HistoryEventIndex::new(11),
            CompartmentFingerprint::new("event-0011").expect("valid event fingerprint"),
        )],
    )
    .expect_err("events must exactly cover the range");

    assert_eq!(
        error.to_string(),
        "compartment events must exactly cover the range"
    );
}

fn compartment(start: u64, end: u64, fingerprint: &str) -> Compartment {
    let range = CompartmentRange::new(HistoryEventIndex::new(start), HistoryEventIndex::new(end))
        .expect("test range must be non-empty");
    let events = (start..end)
        .map(|index| {
            HistoryEvent::new(
                HistoryEventIndex::new(index),
                CompartmentFingerprint::new(format!("event-{index:04}"))
                    .expect("event fingerprint is valid"),
            )
        })
        .collect();

    Compartment::new(
        CompartmentTier::Warm,
        range,
        CompartmentFingerprint::new(fingerprint).expect("compartment fingerprint is valid"),
        events,
    )
    .expect("test compartment events must cover the range")
}

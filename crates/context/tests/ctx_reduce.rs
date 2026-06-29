use jfc_context::{
    ContextDropReplayMode, ContextDropSpec, ContextReduceOptions, ContextSkeletonError, ContextTag,
    ContextTagId, ContextTagKind, ContextTagStatus, PlannedContextDrops, dropped_tag_marker,
};

#[test]
fn drop_spec_parses_magic_style_ranges_normal() {
    let spec = ContextDropSpec::parse("3-5, 8").expect("valid drop spec");

    assert_eq!(spec.ranges().len(), 2);
    assert_eq!(spec.ranges()[0].start(), 3);
    assert_eq!(spec.ranges()[0].end(), 5);
    assert_eq!(spec.ranges()[1].start(), 8);
    assert_eq!(spec.ranges()[1].end(), 8);
}

#[test]
fn drop_spec_rejects_empty_or_malformed_values_malformed() {
    assert_eq!(
        ContextDropSpec::parse(" ").expect_err("empty specs are invalid"),
        ContextSkeletonError::EmptyContextDropSpec
    );
    assert_eq!(
        ContextDropSpec::parse("3--5").expect_err("double dash is invalid"),
        ContextSkeletonError::InvalidContextDropSpec
    );
    assert_eq!(
        ContextDropSpec::parse("5-3").expect_err("inverted ranges are invalid"),
        ContextSkeletonError::InvalidContextDropRange
    );
}

#[test]
fn planned_drops_queue_prefix_and_protect_recent_tail_normal() {
    let tags = active_message_tags(8);
    let spec = ContextDropSpec::parse("2-8").expect("valid drop spec");
    let options = ContextReduceOptions::new(3)
        .with_replay_mode(ContextDropReplayMode::Skeleton)
        .expect("skeleton replay is valid");

    let plan = PlannedContextDrops::plan(&tags, &spec, options).expect("plan drops");

    assert_eq!(plan.queued().len(), 1);
    assert_eq!(plan.queued()[0].range().start(), 2);
    assert_eq!(plan.queued()[0].range().end(), 5);
    assert_eq!(
        plan.queued()[0].replay_mode(),
        ContextDropReplayMode::Skeleton
    );
    assert_eq!(plan.protected_tail_skips().len(), 1);
    assert_eq!(plan.protected_tail_skips()[0].range().start(), 6);
    assert_eq!(plan.protected_tail_skips()[0].range().end(), 8);
    assert_eq!(
        plan.protected_tail_skips()[0].protected_tail_start(),
        Some(6)
    );
}

#[test]
fn planned_drops_skip_already_pending_and_dropped_tags_robust() {
    let tags = vec![
        tag(1, ContextTagStatus::Active),
        tag(2, ContextTagStatus::PendingDrop),
        tag(3, ContextTagStatus::Dropped),
        tag(4, ContextTagStatus::Active),
    ];
    let spec = ContextDropSpec::parse("1-4").expect("valid drop spec");

    let plan = PlannedContextDrops::plan(&tags, &spec, ContextReduceOptions::default())
        .expect("plan skips handled tags");

    assert_eq!(plan.queued().len(), 2);
    assert_eq!(plan.queued()[0].range().start(), 1);
    assert_eq!(plan.queued()[0].range().end(), 1);
    assert_eq!(plan.queued()[1].range().start(), 4);
    assert_eq!(plan.queued()[1].range().end(), 4);
    assert_eq!(
        plan.already_pending(),
        &[ContextTagId::new(2).expect("valid id")]
    );
    assert_eq!(
        plan.already_dropped(),
        &[ContextTagId::new(3).expect("valid id")]
    );
}

#[test]
fn planned_drops_reject_unknown_or_compacted_tags_malformed() {
    let tags = vec![
        tag(1, ContextTagStatus::Active),
        tag(2, ContextTagStatus::Compacted),
    ];

    let compacted = PlannedContextDrops::plan(
        &tags,
        &ContextDropSpec::parse("2").expect("valid drop spec"),
        ContextReduceOptions::default(),
    )
    .expect_err("compacted tags cannot be reduced again");
    assert_eq!(compacted, ContextSkeletonError::CompactedContextTag);

    let unknown = PlannedContextDrops::plan(
        &tags,
        &ContextDropSpec::parse("3").expect("valid drop spec"),
        ContextReduceOptions::default(),
    )
    .expect_err("unknown tags cannot be reduced");
    assert_eq!(unknown, ContextSkeletonError::UnknownContextTag);
}

#[test]
fn dropped_marker_replays_stable_tag_label_normal() {
    assert_eq!(
        dropped_tag_marker(ContextTagId::new(7).expect("valid id")),
        "[dropped §7§]"
    );
}

fn active_message_tags(count: u32) -> Vec<ContextTag> {
    (1..=count)
        .map(|id| ContextTag::active(id, ContextTagKind::Message).expect("valid tag"))
        .collect()
}

fn tag(id: u32, status: ContextTagStatus) -> ContextTag {
    ContextTag::new(id, ContextTagKind::Message, status).expect("valid test tag")
}

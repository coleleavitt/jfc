use super::*;

fn ready_set() -> AgentChangeSet {
    let mut cs = AgentChangeSet::open("abc123", "jfc/feature", "/tmp/wt", 1000);
    cs.mark_ready(
        vec![ChangedFile {
            path: "src/lib.rs".into(),
            insertions: 10,
            deletions: 2,
        }],
        "1 file changed, 10 insertions(+), 2 deletions(-)",
        1001,
    )
    .unwrap();
    cs
}

#[test]
fn open_starts_in_draft_normal() {
    let cs = AgentChangeSet::open("head", "jfc/x", "/tmp/x", 42);
    assert_eq!(cs.state, ChangeState::Draft);
    assert_eq!(cs.id.len(), 16);
    assert_eq!(cs.created_at_ms, cs.updated_at_ms);
}

#[test]
fn id_is_deterministic_and_time_sensitive_robust() {
    let a = AgentChangeSet::compute_id("h", "b", "w", 1);
    let b = AgentChangeSet::compute_id("h", "b", "w", 1);
    let c = AgentChangeSet::compute_id("h", "b", "w", 2);
    assert_eq!(a, b, "same inputs -> same id");
    assert_ne!(a, c, "different timestamp -> different id");
}

#[test]
fn review_pipeline_advances_state_normal() {
    let mut cs = ready_set();
    assert_eq!(cs.state, ChangeState::Ready);

    cs.record_test_run(
        TestRun {
            command: "cargo test".into(),
            exit_code: 0,
            duration_ms: 1234,
            finished_at_ms: 1002,
        },
        1002,
    )
    .unwrap();
    assert_eq!(cs.state, ChangeState::Tested);
    assert!(cs.all_tests_passed());

    cs.approve(
        Approval::Human {
            user: "cole".into(),
            at_ms: 1003,
        },
        1003,
    )
    .unwrap();
    assert_eq!(cs.state, ChangeState::Approved);
    assert!(cs.approval.is_some());

    cs.transition_to(ChangeState::Applied, 1004).unwrap();
    assert_eq!(cs.state, ChangeState::Applied);
    assert_eq!(cs.updated_at_ms, 1004);
}

#[test]
fn ready_set_refuses_direct_apply_robust() {
    let mut cs = ready_set();
    let err = cs.transition_to(ChangeState::Applied, 1002).unwrap_err();
    assert!(matches!(
        err,
        crate::error::ChangeSetError::IllegalTransition { .. }
    ));
    assert_eq!(cs.state, ChangeState::Ready);
}

#[test]
fn failing_test_run_does_not_pass_robust() {
    let mut cs = ready_set();
    cs.record_test_run(
        TestRun {
            command: "cargo test".into(),
            exit_code: 101,
            duration_ms: 50,
            finished_at_ms: 1002,
        },
        1002,
    )
    .unwrap();
    assert_eq!(cs.state, ChangeState::Tested);
    assert!(!cs.all_tests_passed(), "exit 101 must not pass");
}

#[test]
fn changeset_type_trace_records_shape_without_payload_normal() {
    linkscope::trace_detail_enable();
    let mut cs = AgentChangeSet::open(
        "private-base-head",
        "private-branch-name",
        "/private/worktree/path",
        2000,
    );
    cs.mark_ready(
        vec![ChangedFile {
            path: "private/source/path.rs".into(),
            insertions: 3,
            deletions: 1,
        }],
        "private diff summary",
        2001,
    )
    .unwrap();
    cs.record_test_run(
        TestRun {
            command: "private test command".into(),
            exit_code: 0,
            duration_ms: 10,
            finished_at_ms: 2002,
        },
        2002,
    )
    .unwrap();
    assert!(cs.all_tests_passed());
    cs.approve(
        Approval::Human {
            user: "private-user".into(),
            at_ms: 2003,
        },
        2003,
    )
    .unwrap();

    let snapshot = linkscope::snapshot();
    let rendered = format!("{snapshot:?}");
    assert!(rendered.contains("changeset.types.open.detail"));
    assert!(rendered.contains("changeset.types.content.detail"));
    assert!(rendered.contains("changeset.types.test_run.detail"));
    assert!(rendered.contains("changeset.types.approval.detail"));
    assert!(!rendered.contains("private-base-head"));
    assert!(!rendered.contains("private-branch-name"));
    assert!(!rendered.contains("/private/worktree/path"));
    assert!(!rendered.contains("private/source/path.rs"));
    assert!(!rendered.contains("private diff summary"));
    assert!(!rendered.contains("private test command"));
    assert!(!rendered.contains("private-user"));
}

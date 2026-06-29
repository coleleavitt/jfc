use super::*;
use crate::id::AgentId;

#[test]
fn status_terminal_classification_normal() {
    assert!(AgentStatus::Completed.is_terminal());
    assert!(AgentStatus::Failed.is_terminal());
    assert!(AgentStatus::Cancelled.is_terminal());
    assert!(!AgentStatus::Running.is_terminal());
    assert!(!AgentStatus::Idle.is_terminal());
    assert!(!AgentStatus::Pending.is_terminal());
}

#[test]
fn status_active_classification_normal() {
    assert!(AgentStatus::Pending.is_active());
    assert!(AgentStatus::Running.is_active());
    assert!(AgentStatus::Idle.is_active());
    assert!(!AgentStatus::Completed.is_active());
}

#[test]
fn role_accessors_normal() {
    let team = AgentRole::Teammate {
        team_name: "alpha".into(),
    };
    assert_eq!(team.team_name(), Some("alpha"));
    assert_eq!(team.bounty_id(), None);
    assert_eq!(team.label(), "teammate");

    let solver = AgentRole::Solver {
        bounty_id: "b1".into(),
        worktree: None,
    };
    assert_eq!(solver.bounty_id(), Some("b1"));
    assert_eq!(solver.team_name(), None);
}

#[test]
fn state_lifecycle_transitions_normal() {
    let mut s = AgentState::new(AgentId::named("x"), AgentRole::Solo, "do work");
    assert_eq!(s.status, AgentStatus::Pending);
    s.status = AgentStatus::Running;
    s.complete(Some("done".into()));
    assert_eq!(s.status, AgentStatus::Completed);
    assert!(s.completed_at.is_some());
    assert_eq!(s.summary.as_deref(), Some("done"));
}

#[test]
fn state_fail_records_error_robust() {
    let mut s = AgentState::new(AgentId::named("x"), AgentRole::Solo, "do work");
    s.fail("boom");
    assert_eq!(s.status, AgentStatus::Failed);
    assert_eq!(s.error.as_deref(), Some("boom"));
    assert!(s.completed_at.is_some());
}

#[test]
fn state_serde_roundtrip_with_role_payload_robust() {
    let mut s = AgentState::new(
        AgentId::stable("solver", 2),
        AgentRole::Solver {
            bounty_id: "b9".into(),
            worktree: Some(PathBuf::from("/tmp/wt")),
        },
        "solve it",
    );
    s.trust_score = Some(7);
    let json = serde_json::to_string(&s).unwrap();
    let back: AgentState = serde_json::from_str(&json).unwrap();
    assert_eq!(back.id, s.id);
    assert_eq!(back.role.bounty_id(), Some("b9"));
    assert_eq!(back.trust_score, Some(7));
}

#[test]
fn state_trace_records_shape_without_text_payload_normal() {
    linkscope::trace_detail_enable();
    let mut state = AgentState::new(
        AgentId::named("private-agent-name"),
        AgentRole::Teammate {
            team_name: "private-team-name".into(),
        },
        "private description body",
    );
    state.complete(Some("private summary body".into()));

    let snapshot = linkscope::snapshot();
    let rendered = format!("{snapshot:?}");
    assert!(rendered.contains("agent.state.new.detail"));
    assert!(rendered.contains("agent.state.complete.detail"));
    assert!(rendered.contains("description_bytes"));
    assert!(!rendered.contains("private description body"));
    assert!(!rendered.contains("private summary body"));
    assert!(!rendered.contains("private-team-name"));
}

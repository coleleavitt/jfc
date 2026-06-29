use super::*;

#[test]
fn orchestration_trace_records_shape_without_payload_normal() {
    linkscope::trace_detail_enable();
    let layout = OrchestrationLayout::destination_skeleton();
    let mut service = InMemoryOrchestrationEventService::new(layout).unwrap();
    let agent =
        AgentOrchestration::new("private-agent-id", AgentOrchestrationRole::Worker).unwrap();
    let swarm = SwarmOrchestration::new("private-swarm-id", vec!["private-member".into()]).unwrap();
    let council =
        CouncilOrchestration::new("private-council-id", vec!["private-seat".into()]).unwrap();
    let workflow = WorkflowOrchestration::new("private-workflow-id", "private-phase").unwrap();
    let goal = GoalOrchestration::new("private-goal-id", "private-condition").unwrap();
    service
        .record_event(
            OrchestrationEvent::new(
                1,
                OrchestrationModule::Agents,
                OrchestrationEventKind::AgentLaunched,
                "private-actor",
                "private-summary",
            )
            .unwrap(),
        )
        .unwrap();

    assert_eq!(agent.id(), "private-agent-id");
    assert_eq!(swarm.members().len(), 1);
    assert_eq!(council.seats().len(), 1);
    assert_eq!(workflow.phase(), "private-phase");
    assert_eq!(goal.condition(), "private-condition");

    let snapshot = linkscope::snapshot();
    let rendered = format!("{snapshot:?}");
    assert!(rendered.contains("orchestration.agent.new"));
    assert!(rendered.contains("orchestration.event.new"));
    assert!(rendered.contains("id_bytes"));
    assert!(rendered.contains("summary_bytes"));
    assert!(!rendered.contains("private-agent-id"));
    assert!(!rendered.contains("private-swarm-id"));
    assert!(!rendered.contains("private-member"));
    assert!(!rendered.contains("private-seat"));
    assert!(!rendered.contains("private-phase"));
    assert!(!rendered.contains("private-condition"));
    assert!(!rendered.contains("private-actor"));
    assert!(!rendered.contains("private-summary"));
}

use jfc_orchestration::{
    AgentOrchestration, AgentOrchestrationRole, CouncilOrchestration, GoalOrchestration,
    InMemoryOrchestrationEventService, OrchestrationEvent, OrchestrationEventKind,
    OrchestrationEventService, OrchestrationLayout, OrchestrationModule, SwarmOrchestration,
    WorkflowOrchestration,
};
use serde_json::json;

#[test]
fn destination_layout_covers_agents_swarm_council_workflows_and_goals_normal() {
    let layout = OrchestrationLayout::destination_skeleton();

    assert_eq!(
        layout.modules(),
        &[
            OrchestrationModule::Agents,
            OrchestrationModule::Swarm,
            OrchestrationModule::Council,
            OrchestrationModule::Workflows,
            OrchestrationModule::Goals,
        ]
    );
    assert!(layout.is_complete_destination_skeleton());
}

#[test]
fn orchestration_dtos_roundtrip_with_snake_case_domains_normal() {
    let agent = AgentOrchestration::new("agent:explore", AgentOrchestrationRole::Explore)
        .expect("valid agent dto");
    let swarm = SwarmOrchestration::new("swarm:alpha", vec![agent.id().to_owned()])
        .expect("valid swarm dto");
    let council = CouncilOrchestration::new("council:arch", vec![agent.id().to_owned()])
        .expect("valid council dto");
    let workflow =
        WorkflowOrchestration::new("workflow:review", "phase:inspect").expect("valid workflow dto");
    let goal = GoalOrchestration::new("goal:ship", "ship task 25").expect("valid goal dto");

    let encoded =
        serde_json::to_value((&agent, &swarm, &council, &workflow, &goal)).expect("dtos serialize");

    assert_eq!(encoded[0]["role"], json!("explore"));
    assert_eq!(encoded[1]["members"], json!(["agent:explore"]));
    assert_eq!(encoded[2]["seats"], json!(["agent:explore"]));
    assert_eq!(encoded[3]["phase"], json!("phase:inspect"));
    assert_eq!(encoded[4]["condition"], json!("ship task 25"));

    let decoded: (
        AgentOrchestration,
        SwarmOrchestration,
        CouncilOrchestration,
        WorkflowOrchestration,
        GoalOrchestration,
    ) = serde_json::from_value(encoded).expect("dtos deserialize");

    assert_eq!(decoded.0, agent);
    assert_eq!(decoded.1, swarm);
    assert_eq!(decoded.2, council);
    assert_eq!(decoded.3, workflow);
    assert_eq!(decoded.4, goal);
}

#[test]
fn fake_orchestration_event_records_through_service_normal() {
    let mut service =
        InMemoryOrchestrationEventService::new(OrchestrationLayout::destination_skeleton())
            .expect("complete layout constructs event service");
    let event = OrchestrationEvent::new(
        7,
        OrchestrationModule::Workflows,
        OrchestrationEventKind::WorkflowAdvanced,
        "workflow:review",
        "fake orchestration event advanced review phase",
    )
    .expect("valid fake event");

    let recorded = service.record_event(event).expect("event records");

    assert_eq!(recorded.sequence(), 7);
    assert_eq!(recorded.module(), OrchestrationModule::Workflows);
    assert_eq!(recorded.kind(), OrchestrationEventKind::WorkflowAdvanced);
    assert_eq!(recorded.actor(), "workflow:review");
    assert_eq!(service.events().len(), 1);
}

#[test]
fn malformed_orchestration_event_is_rejected_robust() {
    let error = OrchestrationEvent::new(
        1,
        OrchestrationModule::Goals,
        OrchestrationEventKind::GoalEvaluated,
        " ",
        "goal evaluator ran",
    )
    .expect_err("empty actor is malformed");

    assert_eq!(error.to_string(), "orchestration actor id cannot be empty");
}

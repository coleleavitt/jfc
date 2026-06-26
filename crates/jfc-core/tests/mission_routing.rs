use jfc_core::{MissionRouteKind, MissionRouter, TaskRisk};

#[test]
fn direct_prompt_stays_out_of_task_graph_normal() {
    let route = MissionRouter::route_prompt("what is rust ownership?", 0);

    assert_eq!(route.kind, MissionRouteKind::Direct);
    assert!(!route.create_task_graph);
    assert!(!route.should_inject_turn_reminder());
}

#[test]
fn ordinary_multistep_work_routes_solo_with_task_graph_normal() {
    let route = MissionRouter::route_prompt("fix the resume bug and verify the tests", 0);

    assert_eq!(route.kind, MissionRouteKind::Solo);
    assert!(route.create_task_graph);
    assert_eq!(route.risk, None);
    assert!(route.should_inject_turn_reminder());
}

#[test]
fn architecture_review_routes_assisted_normal() {
    let route =
        MissionRouter::route_prompt("review the task architecture and map the remaining gaps", 0);

    assert_eq!(route.kind, MissionRouteKind::Assisted);
    assert!(route.create_task_graph);
    assert_eq!(route.risk, Some(TaskRisk::Medium));
    assert!(route.tags.iter().any(|tag| tag == "architecture"));
    assert!(route.should_inject_turn_reminder());
}

#[test]
fn rsi_market_prompt_routes_bounty_with_distillation_guard_normal() {
    let route = MissionRouter::route_prompt(
        "audit the RSI bounty architecture and implement all of the prompt, skill, tool definition, and memory improvements",
        0,
    );

    assert_eq!(route.kind, MissionRouteKind::Bounty);
    assert!(route.create_task_graph);
    assert_eq!(route.risk, Some(TaskRisk::High));
    assert!(route.tags.iter().any(|tag| tag == "bounty"));
    assert!(route.tags.iter().any(|tag| tag == "rsi"));
    assert!(
        route
            .turn_reminder()
            .contains("Do not store private chain-of-thought")
    );
}

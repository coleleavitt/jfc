//! Agent subsystem.
//!
//! - `registry_impl` — concrete `AgentRegistry` backed by a tokio `RwLock`.
//! - `lifecycle`, `registry` — v126 skill/agent loaders from `jfc-agents`.

pub mod in_process;
pub mod launch;
pub mod process_bridge;
#[cfg(test)]
mod process_bridge_tests;
pub mod registry_impl;
pub mod team;
pub use in_process::{InProcessBackend, WorkOutcome};
pub use launch::{
    AgentLaunchBackend, AgentLaunchError, AgentLaunchPlan, background_worker_execution_task_input,
    plan_from_agent_launch_descriptor, select_agent_launch_plan_by_name,
    select_background_agent_launch_plan, select_background_task_agent_launch_plan,
    select_builtin_agent_launch_plan, select_default_agent_launch_plan,
    select_default_background_agent_launch_plan, select_project_agent_launch_plan,
    select_task_agent_launch_plan, select_teammate_agent_launch_plan,
};
pub use process_bridge::{ProcessBridgeAgentLaunchInvocation, execute_process_bridge_agent_launch};
pub use registry_impl::AgentRegistryImpl;
pub use team::TeamBackend;

pub mod lifecycle {
    pub use jfc_agents::{
        build_agent_system_prompt, build_agent_system_prompt_with_context, render_dispatch_section,
        render_skills_section,
    };
}

pub mod registry {
    pub use jfc_agents::{find_skill_by_name, load_agents, load_skills};
}

// Public items used via `crate::agents::` by callers outside this module.
pub use jfc_agents::{AgentDef, SkillContext, SkillRenderContext, render_skill_invocation};
pub use lifecycle::{
    build_agent_system_prompt, build_agent_system_prompt_with_context, render_dispatch_section,
    render_skills_section,
};
pub use registry::{find_skill_by_name, load_agents, load_skills};

#[cfg(test)]
mod roster_integration_tests {
    //! Cross-backend roster: the unified registry is the single source of truth
    //! for agents spawned through *any* path (solo subagent, teammate, economy
    //! solver/validator). These tests assert that heterogeneous roles coexist
    //! in one `list()` and resolve back by their spawn-time label.
    use std::sync::Arc;

    use jfc_agent::{AgentId, AgentRegistry, AgentResult, AgentRole, AgentState, AgentStatus};

    use super::AgentRegistryImpl;

    /// Register an agent the way each engine spawn path does: a labelled id +
    /// a role, transitioned to Running.
    async fn spawn_like(
        reg: &AgentRegistryImpl,
        label: &str,
        role: AgentRole,
        description: &str,
    ) -> AgentId {
        let id = AgentId::from_label(label);
        reg.register(AgentState::new(id.clone(), role, description.to_string()))
            .await;
        reg.update_status(&id, AgentStatus::Running).await;
        id
    }

    #[tokio::test]
    async fn heterogeneous_roster_coexists_and_resolves_normal() {
        let reg = Arc::new(AgentRegistryImpl::new());

        // One of each spawn path, mirroring the wiring in dispatch/swarm/economy.
        let solo = spawn_like(&reg, "task-42", AgentRole::Solo, "solo subagent").await;
        let mate = spawn_like(
            &reg,
            "researcher",
            AgentRole::Teammate {
                team_name: "alpha".into(),
            },
            "teammate",
        )
        .await;
        let solver = spawn_like(
            &reg,
            "solver-0",
            AgentRole::Solver {
                bounty_id: "b1".into(),
                worktree: None,
            },
            "economy solver",
        )
        .await;
        let validator = spawn_like(
            &reg,
            "validator-0",
            AgentRole::Validator {
                bounty_id: "b1".into(),
            },
            "economy validator",
        )
        .await;

        // All four live in ONE roster.
        let roster = reg.list().await;
        assert_eq!(roster.len(), 4);

        // Each resolves back by its spawn-time label.
        assert_eq!(reg.resolve_name("task-42").await, Some(solo));
        assert_eq!(reg.resolve_name("researcher").await, Some(mate.clone()));
        assert_eq!(reg.resolve_name("solver-0").await, Some(solver.clone()));
        assert_eq!(reg.resolve_name("validator-0").await, Some(validator));

        // Roster groups cleanly by role label for UI rendering.
        let mut labels: Vec<&str> = roster.iter().map(|s| s.role.label()).collect();
        labels.sort_unstable();
        assert_eq!(labels, ["solo", "solver", "teammate", "validator"]);
    }

    #[tokio::test]
    async fn terminal_transitions_are_visible_across_paths_robust() {
        let reg = Arc::new(AgentRegistryImpl::new());

        let solver = spawn_like(
            &reg,
            "solver-1",
            AgentRole::Solver {
                bounty_id: "b2".into(),
                worktree: None,
            },
            "economy solver",
        )
        .await;
        let mate = spawn_like(
            &reg,
            "verifier",
            AgentRole::Teammate {
                team_name: "beta".into(),
            },
            "teammate",
        )
        .await;

        // Economy completion (via `complete`) and team cancellation (via
        // `update_status`) both land in the same roster as terminal states.
        reg.complete(
            &solver,
            AgentResult {
                id: solver.clone(),
                output: "patch".into(),
                tokens_used: 1234,
                elapsed_ms: 10,
                patch: None,
            },
        )
        .await;
        reg.update_status(&mate, AgentStatus::Cancelled).await;

        assert_eq!(reg.status(&solver).await, Some(AgentStatus::Completed));
        assert_eq!(reg.state(&solver).await.unwrap().token_count, 1234);
        assert_eq!(reg.status(&mate).await, Some(AgentStatus::Cancelled));
    }
}

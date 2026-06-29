//! Deterministic agent-quality evals for built-in prompts and prompt helpers.
//!
//! These are intentionally local and API-free. They catch regressions in the
//! instruction surface before prompt changes reach live model calls.

use std::path::PathBuf;

use crate::{
    Skill, build_agent_system_prompt, built_in_agents, render_dispatch_section,
    render_skills_section,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvalSuiteReport {
    pub total: usize,
    pub passed: usize,
    pub cases: Vec<EvalCaseReport>,
}

impl EvalSuiteReport {
    pub fn is_pass(&self) -> bool {
        self.passed == self.total
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvalCaseReport {
    pub id: &'static str,
    pub passed: bool,
    pub missing: Vec<String>,
}

pub fn run_builtin_agent_evals() -> EvalSuiteReport {
    let agents = built_in_agents();
    let cases = vec![
        required_agents_exist(&agents),
        verification_prompt_is_adversarial(&agents),
        verification_agent_is_read_only_and_persistent(&agents),
        delegation_prompt_preserves_parallel_review_shape(&agents),
        skills_are_progressively_disclosed(),
    ];

    let total = cases.len();
    let passed = cases.iter().filter(|case| case.passed).count();
    EvalSuiteReport {
        total,
        passed,
        cases,
    }
}

fn required_agents_exist(agents: &[jfc_core::AgentDef]) -> EvalCaseReport {
    let required = [
        "general-purpose",
        "Explore",
        "Plan",
        "verification",
        "orchestrator",
    ];
    let missing = required
        .iter()
        .filter(|name| !agents.iter().any(|agent| agent.name == **name))
        .map(|name| (*name).to_owned())
        .collect::<Vec<_>>();
    case("required_agents_exist", missing)
}

fn verification_prompt_is_adversarial(agents: &[jfc_core::AgentDef]) -> EvalCaseReport {
    let Some(agent) = find_agent(agents, "verification") else {
        return case(
            "verification_prompt_is_adversarial",
            vec!["missing verification agent".to_owned()],
        );
    };
    contains_all(
        "verification_prompt_is_adversarial",
        &agent.system_prompt,
        &[
            "try to break it",
            "DO NOT MODIFY THE PROJECT",
            "broken build = automatic FAIL",
            "failing tests = automatic FAIL",
            "VERDICT: PASS or VERDICT: FAIL or VERDICT: PARTIAL",
        ],
    )
}

fn verification_agent_is_read_only_and_persistent(agents: &[jfc_core::AgentDef]) -> EvalCaseReport {
    let Some(agent) = find_agent(agents, "verification") else {
        return case(
            "verification_agent_is_read_only_and_persistent",
            vec!["missing verification agent".to_owned()],
        );
    };

    let mut missing = Vec::new();
    for disallowed in ["Edit", "Write", "ApplyPatch"] {
        if !agent
            .disallowed_tools
            .iter()
            .any(|tool| tool.eq_ignore_ascii_case(disallowed))
        {
            missing.push(format!("disallowed tool {disallowed}"));
        }
    }
    for forbidden in ["Edit", "Write", "ApplyPatch"] {
        if agent
            .allowed_tools
            .iter()
            .any(|tool| tool.eq_ignore_ascii_case(forbidden))
        {
            missing.push(format!("allowed write tool {forbidden}"));
        }
    }
    if !agent
        .skills
        .iter()
        .any(|skill| skill == "verification-findings")
    {
        missing.push("verification-findings skill".to_owned());
    }
    case("verification_agent_is_read_only_and_persistent", missing)
}

fn delegation_prompt_preserves_parallel_review_shape(
    agents: &[jfc_core::AgentDef],
) -> EvalCaseReport {
    let prompt = render_dispatch_section(agents);
    contains_all(
        "delegation_prompt_preserves_parallel_review_shape",
        &prompt,
        &[
            "Default Bias: DELEGATE",
            "multiple Task tool_use blocks",
            "Parallel fan-out",
            "Result synthesis",
            "Fire `verification` in background",
        ],
    )
}

fn skills_are_progressively_disclosed() -> EvalCaseReport {
    let skill = Skill::new(
        "rust-style".to_owned(),
        PathBuf::from(".agents/skills/rust-style/SKILL.md"),
        Some("Prefer small, idiomatic Rust changes.".to_owned()),
        "This body must not be listed until Skill is invoked.".to_owned(),
    );
    let listing = render_skills_section(std::slice::from_ref(&skill));
    let agent = jfc_core::AgentDef {
        name: "tester".to_owned(),
        source: PathBuf::from("eval"),
        model: None,
        isolation: None,
        skills: vec!["rust-style".to_owned()],
        allowed_tools: Vec::new(),
        disallowed_tools: Vec::new(),
        permission_mode: None,
        forks_parent_context: None,
        background: None,
        color: None,
        effort: None,
        max_turns: None,
        max_input_tokens: None,
        memory: None,
        mcp_servers: Vec::new(),
        hooks: std::collections::HashMap::new(),
        key_trigger: None,
        use_when: Vec::new(),
        avoid_when: Vec::new(),
        cost: None,
        system_prompt: "Base prompt.".to_owned(),
    };
    let compiled = build_agent_system_prompt(&agent, &[skill]);

    let mut missing = Vec::new();
    if !listing.contains("rust-style") {
        missing.push("skill name listed".to_owned());
    }
    if !listing.contains("Prefer small, idiomatic Rust changes.") {
        missing.push("skill description listed".to_owned());
    }
    if listing.contains("This body must not be listed") {
        missing.push("skill body hidden from listing".to_owned());
    }
    if !compiled.contains("This body must not be listed until Skill is invoked.") {
        missing.push("skill body injected into resolved agent prompt".to_owned());
    }
    case("skills_are_progressively_disclosed", missing)
}

fn contains_all(id: &'static str, text: &str, required: &[&str]) -> EvalCaseReport {
    case(
        id,
        required
            .iter()
            .filter(|needle| !text.contains(**needle))
            .map(|needle| (*needle).to_owned())
            .collect(),
    )
}

fn find_agent<'a>(agents: &'a [jfc_core::AgentDef], name: &str) -> Option<&'a jfc_core::AgentDef> {
    agents
        .iter()
        .find(|agent| agent.name.eq_ignore_ascii_case(name))
}

fn case(id: &'static str, missing: Vec<String>) -> EvalCaseReport {
    EvalCaseReport {
        id,
        passed: missing.is_empty(),
        missing,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_agent_evals_pass_normal() {
        let report = run_builtin_agent_evals();
        assert!(report.is_pass(), "{report:#?}");
    }
}

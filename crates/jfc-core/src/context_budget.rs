//! Context-budget accounting and LLMLingua-style budget control.
//!
//! This module implements the numeric models from
//! `rcoq-tests/theorems/ContextBudget.v`: initial-query token accounting,
//! progressive tool disclosure, lazy memory loading, post-compaction size, and
//! the LLMLingua component budget controller.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ContextBudget {
    pub system_prompt_tokens: u64,
    pub tool_definition_tokens: u64,
    pub memory_tokens: u64,
    pub project_instructions_tokens: u64,
    pub user_message_tokens: u64,
}

pub const OVERHEAD_NUM: u64 = 3;
pub const OVERHEAD_DEN: u64 = 2;

pub fn raw_tokens(budget: ContextBudget) -> u64 {
    budget
        .system_prompt_tokens
        .saturating_add(budget.tool_definition_tokens)
        .saturating_add(budget.memory_tokens)
        .saturating_add(budget.project_instructions_tokens)
        .saturating_add(budget.user_message_tokens)
}

pub fn with_overhead(tokens: u64) -> u64 {
    tokens.saturating_mul(OVERHEAD_NUM) / OVERHEAD_DEN
}

pub fn effective_tokens(budget: ContextBudget) -> u64 {
    with_overhead(raw_tokens(budget))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ToolDefBudget {
    pub name_tokens: u64,
    pub description_tokens: u64,
    pub schema_tokens: u64,
}

pub fn tool_tokens(tool: ToolDefBudget) -> u64 {
    tool.name_tokens
        .saturating_add(tool.description_tokens)
        .saturating_add(tool.schema_tokens)
}

pub const SMALL_TOOL: ToolDefBudget = ToolDefBudget {
    name_tokens: 2,
    description_tokens: 50,
    schema_tokens: 100,
};
pub const MEDIUM_TOOL: ToolDefBudget = ToolDefBudget {
    name_tokens: 3,
    description_tokens: 100,
    schema_tokens: 200,
};

pub const JFC_CORE_TOOLS: u64 = 15;
pub const JFC_MCP_TOOLS: u64 = 15;
pub const JFC_TASK_TOOLS: u64 = 10;
pub const JFC_SKILL_TOOLS: u64 = 10;

pub fn estimated_tool_tokens() -> u64 {
    JFC_CORE_TOOLS
        .saturating_mul(tool_tokens(MEDIUM_TOOL))
        .saturating_add(JFC_MCP_TOOLS.saturating_mul(tool_tokens(MEDIUM_TOOL)))
        .saturating_add(JFC_TASK_TOOLS.saturating_mul(tool_tokens(SMALL_TOOL)))
        .saturating_add(JFC_SKILL_TOOLS.saturating_mul(tool_tokens(SMALL_TOOL)))
}

pub const SYSTEM_PROMPT_BASE: u64 = 8000;
pub const SYSTEM_PROMPT_RULES: u64 = 2000;
pub const SYSTEM_PROMPT_MEMORY: u64 = 3000;
pub const SYSTEM_PROMPT_AGENTS: u64 = 2000;

pub fn total_system_prompt() -> u64 {
    SYSTEM_PROMPT_BASE
        .saturating_add(SYSTEM_PROMPT_RULES)
        .saturating_add(SYSTEM_PROMPT_MEMORY)
        .saturating_add(SYSTEM_PROMPT_AGENTS)
}

pub fn typical_initial_budget() -> ContextBudget {
    ContextBudget {
        system_prompt_tokens: total_system_prompt(),
        tool_definition_tokens: estimated_tool_tokens(),
        memory_tokens: 5000,
        project_instructions_tokens: 3000,
        user_message_tokens: 500,
    }
}

pub fn progressive_tools(query_type: u64) -> u64 {
    match query_type {
        0 => 5,
        1 => 15,
        2 => 30,
        _ => 50,
    }
}

pub fn lazy_memory_tokens(relevance_threshold: u64, total_memories: u64) -> u64 {
    total_memories.saturating_mul(relevance_threshold) / 100
}

pub fn post_compact_tokens(initial: u64, compression_ratio: u64) -> u64 {
    initial.saturating_mul(compression_ratio) / 100
}

pub fn compressible_system_prompt() -> u64 {
    SYSTEM_PROMPT_RULES.saturating_add(SYSTEM_PROMPT_MEMORY)
}

pub fn static_system_prompt() -> u64 {
    SYSTEM_PROMPT_BASE.saturating_add(SYSTEM_PROMPT_AGENTS)
}

pub fn optimized_budget() -> ContextBudget {
    ContextBudget {
        system_prompt_tokens: total_system_prompt().saturating_mul(7) / 10,
        tool_definition_tokens: estimated_tool_tokens() / 3,
        memory_tokens: 2000,
        project_instructions_tokens: 3000,
        user_message_tokens: 500,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CompBudget {
    pub ins_budget: u64,
    pub dems_budget: u64,
    pub que_budget: u64,
}

pub fn total_budget(budget: CompBudget) -> u64 {
    budget
        .ins_budget
        .saturating_add(budget.dems_budget)
        .saturating_add(budget.que_budget)
}

pub const GRANULAR_K: u64 = 2;

pub fn demo_cap(budget: CompBudget) -> u64 {
    GRANULAR_K.saturating_mul(budget.dems_budget)
}

pub fn demo_select(cap: u64, demos: &[u64]) -> u64 {
    let mut acc = 0u64;
    for demo in demos {
        let next = acc.saturating_add(*demo);
        if next <= cap {
            acc = next;
        } else {
            break;
        }
    }
    acc
}

pub fn demo_slack(cap: u64, demos: &[u64]) -> u64 {
    cap.saturating_sub(demo_select(cap, demos))
}

pub fn redistribute(budget: CompBudget, d1: u64, d2: u64) -> CompBudget {
    CompBudget {
        ins_budget: budget.ins_budget.saturating_add(d1),
        dems_budget: budget.dems_budget,
        que_budget: budget.que_budget.saturating_add(d2),
    }
}

pub const TAU_INS_PCT: u64 = 85;
pub const TAU_QUE_PCT: u64 = 90;
pub const TAU_DEMS_PCT: u64 = 50;

pub fn kept_tokens_at_rate(raw_len: u64, rate_pct: u64) -> u64 {
    rate_pct.saturating_mul(raw_len) / 100
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initial_query_budget_lands_in_observed_50k_band() {
        assert!(estimated_tool_tokens() >= 10_000);
        assert!(total_system_prompt() >= 15_000);
        let effective = effective_tokens(typical_initial_budget());
        assert!((40_000..=60_000).contains(&effective));
    }

    #[test]
    fn progressive_and_lazy_loading_save_tokens() {
        for query_type in 0..3 {
            assert!(
                progressive_tools(query_type).saturating_mul(tool_tokens(MEDIUM_TOOL))
                    < estimated_tool_tokens()
            );
        }
        assert!(lazy_memory_tokens(20, 5000) <= 5000 / 4);
    }

    #[test]
    fn compaction_and_static_cache_control_budget_growth() {
        assert!(post_compact_tokens(60_000, 30) <= 20_000);
        assert!(static_system_prompt() >= total_system_prompt() / 2);
        assert!(
            effective_tokens(optimized_budget())
                < effective_tokens(typical_initial_budget()).saturating_mul(7) / 10
        );
        assert_eq!(compressible_system_prompt(), 5000);
    }

    #[test]
    fn component_budget_is_conserved_and_slack_is_accounted() {
        let budget = CompBudget {
            ins_budget: 100,
            dems_budget: 50,
            que_budget: 80,
        };
        assert_eq!(total_budget(budget), 230);
        let cap = demo_cap(budget);
        let demos = vec![20, 30, 80];
        let selected = demo_select(cap, &demos);
        let slack = demo_slack(cap, &demos);
        assert!(selected <= cap);
        assert_eq!(selected + slack, cap);

        let redistributed = redistribute(budget, 10, slack.saturating_sub(10));
        assert_eq!(total_budget(redistributed), total_budget(budget) + slack);
    }

    #[test]
    fn component_rate_ordering_keeps_more_at_higher_rate() {
        assert!(TAU_DEMS_PCT < TAU_INS_PCT);
        assert!(TAU_DEMS_PCT < TAU_QUE_PCT);
        assert!(TAU_INS_PCT <= TAU_QUE_PCT);
        assert!(kept_tokens_at_rate(1000, TAU_DEMS_PCT) <= kept_tokens_at_rate(1000, TAU_QUE_PCT));
    }
}

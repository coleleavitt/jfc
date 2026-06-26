use serde::{Deserialize, Serialize};

use crate::{TaskExecutionMode, TaskRisk};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MissionRouteKind {
    Direct,
    Solo,
    Assisted,
    Bounty,
}

impl MissionRouteKind {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Direct => "direct",
            Self::Solo => "solo",
            Self::Assisted => "assisted",
            Self::Bounty => "bounty",
        }
    }

    pub const fn task_execution_mode(self) -> Option<TaskExecutionMode> {
        match self {
            Self::Direct => None,
            Self::Solo => Some(TaskExecutionMode::Solo),
            Self::Assisted => Some(TaskExecutionMode::Assisted),
            Self::Bounty => Some(TaskExecutionMode::Bounty),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MissionRoute {
    pub kind: MissionRouteKind,
    pub create_task_graph: bool,
    pub risk: Option<TaskRisk>,
    pub tags: Vec<String>,
    pub reason: String,
}

impl MissionRoute {
    pub fn should_inject_turn_reminder(&self) -> bool {
        self.create_task_graph && self.kind.task_execution_mode().is_some()
    }

    pub fn turn_reminder(&self) -> String {
        let risk = self.risk.map(task_risk_label).unwrap_or("unspecified");
        let tags = if self.tags.is_empty() {
            "[]".to_owned()
        } else {
            format!("[{}]", self.tags.join(", "))
        };
        let task_graph = if self.create_task_graph {
            "required"
        } else {
            "optional"
        };

        format!(
            "Mission router: route={}, task_graph={}, risk={}, tags={}. Reason: {}.\n\
             Treat this as a durable mission. Create TaskCreate records for the meaningful \
             steps before execution, with acceptance criteria and verification commands where \
             known. For solver-worthy steps, set risk=\"high\" or include tags like \
             \"bounty\"/\"market\" so the task factory can dispatch a bounty with \
             parent_task_id. Use bounty results as execution evidence; store only distilled \
             evidence, decisions, prompts, skill changes, and outcomes in memory. Do not store \
             private chain-of-thought.",
            self.kind.label(),
            task_graph,
            risk,
            tags,
            self.reason
        )
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub struct MissionRouter;

impl MissionRouter {
    pub fn route_prompt(prompt: &str, attachment_count: usize) -> MissionRoute {
        let normalized = normalize(prompt);
        let word_count = prompt.split_whitespace().count();
        let has_action = contains_any(&normalized, ACTION_SIGNALS);
        let has_multi_step = contains_any(&normalized, MULTI_STEP_SIGNALS)
            || (has_action && contains_any(&normalized, COORDINATION_SIGNALS));
        let has_bounty_domain = contains_any(&normalized, BOUNTY_DOMAIN_SIGNALS);
        let has_assisted_domain = contains_any(&normalized, ASSISTED_DOMAIN_SIGNALS);
        let has_attachment_work = attachment_count > 0 && has_action;

        if has_bounty_domain && (has_action || has_multi_step || word_count > 18) {
            return MissionRoute {
                kind: MissionRouteKind::Bounty,
                create_task_graph: true,
                risk: Some(TaskRisk::High),
                tags: bounty_tags_for(&normalized),
                reason: "prompt touches market, RSI, safety, memory, prompt, or tool surfaces that need solver/validator pressure"
                    .to_owned(),
            };
        }

        if has_assisted_domain && (has_action || has_multi_step || has_attachment_work) {
            return MissionRoute {
                kind: MissionRouteKind::Assisted,
                create_task_graph: true,
                risk: Some(TaskRisk::Medium),
                tags: assisted_tags_for(&normalized),
                reason: "prompt spans planning, research, review, or architecture work that benefits from a second focused agent"
                    .to_owned(),
            };
        }

        if has_action || has_multi_step || has_attachment_work {
            return MissionRoute {
                kind: MissionRouteKind::Solo,
                create_task_graph: has_multi_step || has_attachment_work,
                risk: None,
                tags: Vec::new(),
                reason: "ordinary scoped work can run as a direct solo task".to_owned(),
            };
        }

        MissionRoute {
            kind: MissionRouteKind::Direct,
            create_task_graph: false,
            risk: None,
            tags: Vec::new(),
            reason: "prompt reads as a direct answer or clarification".to_owned(),
        }
    }
}

fn task_risk_label(risk: TaskRisk) -> &'static str {
    match risk {
        TaskRisk::Low => "low",
        TaskRisk::Medium => "medium",
        TaskRisk::High => "high",
    }
}

fn normalize(input: &str) -> String {
    input.to_ascii_lowercase()
}

fn contains_any(haystack: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| haystack.contains(needle))
}

fn bounty_tags_for(normalized: &str) -> Vec<String> {
    let mut tags = vec!["bounty".to_owned(), "market".to_owned()];
    push_if_signal(&mut tags, normalized, "rsi", RSI_SIGNALS);
    push_if_signal(&mut tags, normalized, "safety", SAFETY_SIGNALS);
    push_if_signal(&mut tags, normalized, "prompt", PROMPT_SIGNALS);
    push_if_signal(&mut tags, normalized, "tools", TOOL_SIGNALS);
    push_if_signal(&mut tags, normalized, "memory", MEMORY_SIGNALS);
    tags
}

fn assisted_tags_for(normalized: &str) -> Vec<String> {
    let mut tags = Vec::new();
    push_if_signal(&mut tags, normalized, "architecture", ARCHITECTURE_SIGNALS);
    push_if_signal(&mut tags, normalized, "research", RESEARCH_SIGNALS);
    push_if_signal(&mut tags, normalized, "review", REVIEW_SIGNALS);
    push_if_signal(&mut tags, normalized, "refactor", REFACTOR_SIGNALS);
    tags
}

fn push_if_signal(tags: &mut Vec<String>, normalized: &str, tag: &str, signals: &[&str]) {
    if contains_any(normalized, signals) && !tags.iter().any(|existing| existing == tag) {
        tags.push(tag.to_owned());
    }
}

const ACTION_SIGNALS: &[&str] = &[
    "add ",
    "audit",
    "build",
    "change",
    "compare",
    "create",
    "debug",
    "dig",
    "do ",
    "find",
    "fix",
    "implement",
    "investigate",
    "patch",
    "read",
    "refactor",
    "run",
    "trace",
    "update",
    "verify",
    "wire",
    "write",
];

const MULTI_STEP_SIGNALS: &[&str] = &[
    " all of ",
    " all the ",
    " and also ",
    " and run",
    " and test",
    " and verify",
    " both ",
    " end to end",
    " everything",
    " gaps",
    " remaining",
    " then ",
    " what else",
];

const COORDINATION_SIGNALS: &[&str] = &["architecture", "bounty", "market", "rsi", "task"];

const BOUNTY_DOMAIN_SIGNALS: &[&str] = &[
    "bounty",
    "chain-of-thought",
    "cot",
    "exploit",
    "market",
    "memory",
    "private thought",
    "prompt",
    "race",
    "rsi",
    "security",
    "self-improve",
    "self improve",
    "skill",
    "system prompt",
    "tool definition",
    "unsafe",
    "vulnerability",
];

const ASSISTED_DOMAIN_SIGNALS: &[&str] = &[
    "architecture",
    "design",
    "plan",
    "refactor",
    "research",
    "review",
];

const RSI_SIGNALS: &[&str] = &["rsi", "self-improve", "self improve"];
const SAFETY_SIGNALS: &[&str] = &["exploit", "race", "security", "unsafe", "vulnerability"];
const PROMPT_SIGNALS: &[&str] = &["chain-of-thought", "cot", "private thought", "prompt"];
const TOOL_SIGNALS: &[&str] = &["skill", "tool definition"];
const MEMORY_SIGNALS: &[&str] = &["memory"];
const ARCHITECTURE_SIGNALS: &[&str] = &["architecture"];
const RESEARCH_SIGNALS: &[&str] = &["research"];
const REVIEW_SIGNALS: &[&str] = &["review"];
const REFACTOR_SIGNALS: &[&str] = &["refactor"];

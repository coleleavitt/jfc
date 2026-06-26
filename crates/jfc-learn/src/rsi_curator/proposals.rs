use super::analysis::ThinkingAnalysis;
use super::candidate::{CandidateChange, CandidateKind, CandidateTarget, candidate};
use super::id::slug;
use super::trace::{RsiToolStep, RsiTrace, RsiTraceScore};

pub(super) fn memory_rule(
    trace: &RsiTrace,
    score: &RsiTraceScore,
    lesson: &str,
) -> CandidateChange {
    candidate(
        trace,
        CandidateKind::MemoryRule,
        CandidateTarget {
            kind: "knowledge".to_owned(),
            name: "rsi-memory-rule".to_owned(),
        },
        "RSI lesson from corrected trace",
        lesson.to_owned(),
        score.overall.max(0.65),
        None,
    )
}

pub(super) fn system_prompt_patch(trace: &RsiTrace, score: &RsiTraceScore) -> CandidateChange {
    candidate(
        trace,
        CandidateKind::SystemPromptPatch,
        CandidateTarget {
            kind: "system_prompt".to_owned(),
            name: "rsi-trace-correction-guard".to_owned(),
        },
        "Prompt patch for correction recovery",
        "When the user corrects a result, classify the correction, update the working assumption, and verify the next action against the corrected state before proceeding."
            .to_owned(),
        score.overall.max(0.7),
        None,
    )
}

pub(super) fn tool_patch(trace: &RsiTrace, score: &RsiTraceScore, tool: &str) -> CandidateChange {
    candidate(
        trace,
        CandidateKind::ToolDefinitionPatch,
        CandidateTarget {
            kind: "tool_definition".to_owned(),
            name: tool.to_owned(),
        },
        format!("Tool definition patch for {tool} recovery"),
        format!(
            "For `{tool}`, failed-then-recovered traces should make the model verify current inputs, paths, and exact ids before retrying the same tool."
        ),
        score.overall.max(0.75),
        None,
    )
}

pub(super) fn harness_patch(
    trace: &RsiTrace,
    score: &RsiTraceScore,
    analysis: &ThinkingAnalysis,
    recovered_tool: Option<&str>,
) -> CandidateChange {
    let name = harness_name(trace, recovered_tool);
    let target = recovered_tool.unwrap_or("general-agent-harness");
    let body = format!(
        "Weakness Mining: {}.\nHarness Proposal: before repeating `{target}` or continuing after a correction, snapshot the current task state, verify paths/ids/inputs against the latest observation, then execute the smallest next tool step.\nProposal Validation: accept only after the original failure fixture and a final observable verification pass; rollback the harness change if success rate or cost regresses.",
        analysis.lesson
    );
    candidate(
        trace,
        CandidateKind::HarnessPatch,
        CandidateTarget {
            kind: "agent_harness".to_owned(),
            name,
        },
        "Harness patch from RSI trace",
        body,
        score.overall.max(0.72),
        None,
    )
}

pub(super) fn context_playbook_patch(
    trace: &RsiTrace,
    score: &RsiTraceScore,
    analysis: &ThinkingAnalysis,
) -> CandidateChange {
    let pattern = analysis.pattern.slug();
    let retrievals = trace.retrieval_steps.len();
    let useful_retrievals = trace
        .retrieval_steps
        .iter()
        .filter(|step| step.result_count > 0)
        .count();
    let fanout_agents: u64 = trace.agent_fanouts.iter().map(|fanout| fanout.count).sum();
    let selections = trace.selections.len();
    let body = format!(
        "Context Playbook: {pattern}\nPattern: {}\nLesson: {}\nObserved Signals: retrievals={retrievals}, useful_retrievals={useful_retrievals}, fanout_agents={fanout_agents}, selections={selections}.\nApplication: for similar tasks, ground the run with current context, use parallel attempts only for independent solution paths, and select from observable evidence instead of private reasoning text.\nValidation: keep the playbook only when the next run includes an observable verification and no provider reasoning transcript is copied into memory, skills, prompts, or tool definitions.",
        pattern_label(analysis.pattern),
        analysis.lesson
    );
    candidate(
        trace,
        CandidateKind::ContextPlaybookPatch,
        CandidateTarget {
            kind: "context_playbook".to_owned(),
            name: pattern.to_owned(),
        },
        format!("Context playbook: {pattern}"),
        body,
        score.overall.max(0.68),
        None,
    )
}

pub(super) fn skill_draft(trace: &RsiTrace, score: &RsiTraceScore) -> CandidateChange {
    let sequence = tool_sequence(&trace.tool_steps);
    let name = format!("rsi-{}", slug(&sequence.join("-")));
    candidate(
        trace,
        CandidateKind::SkillDraft,
        CandidateTarget {
            kind: "skill".to_owned(),
            name: name.clone(),
        },
        format!("Skill draft: {}", sequence.join(" -> ")),
        format!(
            "---\nname: {name}\ndescription: 'RSI-mined successful tool procedure.'\ncreated-by: rsi-curator\n---\nUse this when the task follows this successful tool sequence: {}.\nVerify the final observable outcome before treating the procedure as complete.\n",
            sequence.join(" -> ")
        ),
        score.overall.max(0.8),
        None,
    )
}

pub(super) fn recovered_tool(steps: &[RsiToolStep]) -> Option<String> {
    for (idx, step) in steps.iter().enumerate() {
        if step.success {
            continue;
        }
        if steps[idx + 1..]
            .iter()
            .any(|later| later.name == step.name && later.success)
        {
            return Some(step.name.clone());
        }
    }
    None
}

fn tool_sequence(steps: &[RsiToolStep]) -> Vec<String> {
    steps.iter().map(|step| step.name.clone()).collect()
}

fn harness_name(trace: &RsiTrace, recovered_tool: Option<&str>) -> String {
    let model = trace.model.as_deref().unwrap_or("default-model");
    let target = recovered_tool.unwrap_or("general");
    format!("{}-{}", slug(model), slug(target))
}

fn pattern_label(pattern: super::ThinkingPatternKind) -> &'static str {
    match pattern {
        super::ThinkingPatternKind::VerifiedEfficient => "verified efficient run",
        super::ThinkingPatternKind::GroundedSearch => "grounded search",
        super::ThinkingPatternKind::ParallelSelection => "parallel selection",
        super::ThinkingPatternKind::OverthoughtFailure => "overthought failure",
        super::ThinkingPatternKind::ToolRecovery => "tool recovery",
        super::ThinkingPatternKind::CorrectionRecovery => "correction recovery",
        super::ThinkingPatternKind::UnverifiedAction => "unverified action",
    }
}

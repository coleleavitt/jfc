use super::analysis::{ThinkingAnalysis, ThinkingPatternKind};
use super::candidate::{
    CandidateChange, CandidateKind, CandidateTarget, ThinkingProvenance, candidate,
};
use super::id::slug;
use super::trace::{RsiTrace, RsiTraceScore};

pub(super) fn should_emit_reasoning_policy(trace: &RsiTrace) -> bool {
    trace.thinking_tokens > 0 || !trace.thinking_blocks.is_empty()
}

pub(super) fn reasoning_policy(
    trace: &RsiTrace,
    score: &RsiTraceScore,
    analysis: &ThinkingAnalysis,
) -> CandidateChange {
    let pattern = analysis.pattern.slug();
    let thinking = ThinkingProvenance::from_trace(trace);
    let name = format!(
        "{}-{}",
        trace.model.as_deref().unwrap_or("default-model"),
        pattern
    );
    let body = format!(
        "Reasoning Process Policy: {pattern}\nPattern: {}\nLesson: {}\nReflection Signal: support={}, self_consistency={}, observable_support_count={}.\nSelf-Refinement Loop: use private reasoning as a hypothesis generator, critique it against independent signals, distill only the reusable lesson, and require self-consistency or observable verification before promotion.\nPolicy: make the smallest useful private reasoning pass, name assumptions internally, then run an observable verification before treating the result as learned or complete.\nSafety: never copy private reasoning into prompts, skills, memory, tool definitions, harnesses, or logs; persist only this distilled policy and its verification evidence.\nValidation: keep the policy only while fixtures pass, research checks stay verified, and rollback remains available.",
        pattern_label(analysis.pattern),
        analysis.lesson,
        thinking.support.slug(),
        thinking.self_consistency.slug(),
        thinking.observable_support_count
    );
    candidate(
        trace,
        CandidateKind::ReasoningPolicy,
        CandidateTarget {
            kind: "reasoning_policy".to_owned(),
            name: slug(&name),
        },
        format!("Reasoning process policy: {pattern}"),
        body,
        score.overall.max(0.69),
        None,
    )
}

fn pattern_label(pattern: ThinkingPatternKind) -> &'static str {
    match pattern {
        ThinkingPatternKind::VerifiedEfficient => "verified efficient run",
        ThinkingPatternKind::GroundedSearch => "grounded search",
        ThinkingPatternKind::ParallelSelection => "parallel selection",
        ThinkingPatternKind::OverthoughtFailure => "overthought failure",
        ThinkingPatternKind::ToolRecovery => "tool recovery",
        ThinkingPatternKind::CorrectionRecovery => "correction recovery",
        ThinkingPatternKind::UnverifiedAction => "unverified action",
    }
}

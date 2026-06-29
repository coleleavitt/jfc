use jfc_context::{
    ContextDropSpec, ContextTagId, ContextTagStatus, PlannedContextDrops, QueuedContextDrop,
};

use crate::{
    app::EngineState,
    context_accounting::{ContextPressureNudge, ContextPressureNudgeKind},
    types::ChatMessage,
};

const MAX_PRESSURE_AUTO_DROP_TAGS: usize = 32;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PressureContextReduction {
    pub(crate) queued_tags: usize,
    pub(crate) queued_ranges: usize,
    pub(crate) estimated_reclaim_tokens: u64,
}

pub(crate) fn queue_pressure_reduction(
    state: &mut EngineState,
    nudge: ContextPressureNudge,
) -> Option<PressureContextReduction> {
    if nudge.kind == ContextPressureNudgeKind::ChannelOne {
        return None;
    }

    let tags = super::transcript_tags(&state.messages, &state.context_reduction_queue);
    let selected = pressure_drop_candidates(&state.messages, &tags, nudge.reclaim_floor_tokens);
    if selected.is_empty() {
        return None;
    }

    let spec_text = format_drop_spec(&selected);
    let spec = ContextDropSpec::parse(&spec_text).ok()?;
    let plan = PlannedContextDrops::plan(
        &tags,
        &spec,
        jfc_context::ContextReduceOptions::new(super::PROTECTED_TAIL_MESSAGES),
    )
    .ok()?;
    let queued_tags =
        queued_tag_count(plan.queued()) + queued_tag_count(plan.protected_tail_skips());
    if queued_tags == 0 {
        return None;
    }

    let queued_ranges = plan.queued().len() + plan.protected_tail_skips().len();
    state
        .context_reduction_queue
        .extend(plan.queued().iter().cloned());
    state
        .context_reduction_queue
        .extend(plan.protected_tail_skips().iter().cloned());

    Some(PressureContextReduction {
        queued_tags,
        queued_ranges,
        estimated_reclaim_tokens: selected
            .iter()
            .map(|candidate| candidate.estimated_tokens)
            .sum(),
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PressureDropCandidate {
    tag_id: ContextTagId,
    estimated_tokens: u64,
}

fn pressure_drop_candidates(
    messages: &[ChatMessage],
    tags: &[jfc_context::ContextTag],
    reclaim_floor_tokens: u64,
) -> Vec<PressureDropCandidate> {
    let protected_tail_start = protected_tail_start(tags);
    let mut selected = Vec::new();
    let mut selected_tokens = 0u64;

    for tag in tags {
        if tag.status() != ContextTagStatus::Active {
            continue;
        }
        if protected_tail_start.is_some_and(|tail_start| tag.id() >= tail_start) {
            continue;
        }
        let Some(message) = tag
            .id()
            .get()
            .checked_sub(1)
            .and_then(|index| usize::try_from(index).ok())
            .and_then(|index| messages.get(index))
        else {
            continue;
        };
        let estimated_tokens = estimated_message_tokens(message).max(1);
        selected.push(PressureDropCandidate {
            tag_id: tag.id(),
            estimated_tokens,
        });
        selected_tokens = selected_tokens.saturating_add(estimated_tokens);
        if selected_tokens >= reclaim_floor_tokens || selected.len() >= MAX_PRESSURE_AUTO_DROP_TAGS
        {
            break;
        }
    }

    selected
}

fn protected_tail_start(tags: &[jfc_context::ContextTag]) -> Option<ContextTagId> {
    tags.iter()
        .rev()
        .filter(|tag| tag.status() == ContextTagStatus::Active)
        .nth(super::PROTECTED_TAIL_MESSAGES - 1)
        .map(|tag| tag.id())
}

fn estimated_message_tokens(message: &ChatMessage) -> u64 {
    let chars = message
        .parts
        .iter()
        .map(|part| part.text_only().len())
        .sum::<usize>();
    (chars as u64).saturating_div(crate::compact::CHARS_PER_TOKEN as u64)
}

fn format_drop_spec(candidates: &[PressureDropCandidate]) -> String {
    candidates
        .iter()
        .map(|candidate| candidate.tag_id.get().to_string())
        .collect::<Vec<_>>()
        .join(",")
}

fn queued_tag_count(drops: &[QueuedContextDrop]) -> usize {
    drops
        .iter()
        .map(|drop| {
            let range = drop.range();
            match usize::try_from(range.end() - range.start() + 1) {
                Ok(width) => width,
                Err(_) => usize::MAX,
            }
        })
        .sum()
}

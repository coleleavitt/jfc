use jfc_context::{
    ContextDropReplayMode, ContextReductionQueue, ContextTag, ContextTagId, ContextTagKind,
    ContextTagStatus, QueuedContextDrop, dropped_tag_marker,
};

use crate::{
    app::EngineState,
    types::{ChatMessage, MessagePart},
};

pub(crate) const PROTECTED_TAIL_MESSAGES: usize = 6;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct ContextReductionDrain {
    pub applied: usize,
    pub deferred: usize,
}

pub(crate) fn transcript_tags(
    messages: &[ChatMessage],
    queue: &ContextReductionQueue,
) -> Vec<ContextTag> {
    messages
        .iter()
        .enumerate()
        .filter_map(|(index, message)| {
            let id = tag_id_for_index(index)?;
            let status = context_tag_status(id, message, queue);
            ContextTag::new(id.get(), context_tag_kind(message), status).ok()
        })
        .collect()
}

pub(crate) fn drain_context_reduction_queue(state: &mut EngineState) -> ContextReductionDrain {
    if state.context_reduction_queue.is_empty() {
        return ContextReductionDrain::default();
    }

    let _linkscope_drain = linkscope::phase("context.reduce.drain");
    let tail_start = current_protected_tail_start(&state.messages, PROTECTED_TAIL_MESSAGES);
    let pending = state.context_reduction_queue.take();
    linkscope::record_items(
        "context.reduce.pending",
        usize_to_u64_saturating(pending.len()),
    );
    let mut deferred = Vec::new();
    let mut applied = 0;

    for drop in pending {
        if should_defer_protected_tail_drop(&drop, tail_start) {
            deferred.push(drop);
            continue;
        }

        applied += apply_context_drop(&mut state.messages, &drop);
    }

    let deferred_count = deferred.len();
    state.context_reduction_queue.extend(deferred);
    if applied > 0 {
        state.tool_ctx.approx_tokens = crate::compact::estimate_tokens(&state.messages);
    }
    linkscope::record_items("context.reduce.applied", usize_to_u64_saturating(applied));
    linkscope::record_items(
        "context.reduce.deferred",
        usize_to_u64_saturating(deferred_count),
    );

    ContextReductionDrain {
        applied,
        deferred: deferred_count,
    }
}

pub(crate) fn mark_expected_cache_drop(
    state: &mut EngineState,
    identity: String,
    drain: ContextReductionDrain,
) {
    if drain.applied == 0 {
        return;
    }

    crate::cache_lineage::mark_expected_drop(
        state,
        identity,
        "ctx_reduce drained queued transcript drops",
        drain.applied,
        None,
    );
}

fn context_tag_status(
    id: ContextTagId,
    message: &ChatMessage,
    queue: &ContextReductionQueue,
) -> ContextTagStatus {
    if queue
        .drops()
        .iter()
        .any(|drop| drop.range().contains(id.get()))
    {
        return ContextTagStatus::PendingDrop;
    }
    if message.is_compact_boundary() {
        return ContextTagStatus::Compacted;
    }
    if message_is_drop_marker(message, id) {
        return ContextTagStatus::Dropped;
    }

    ContextTagStatus::Active
}

fn context_tag_kind(message: &ChatMessage) -> ContextTagKind {
    if message
        .parts
        .iter()
        .any(|part| matches!(part, MessagePart::Tool(_)))
    {
        ContextTagKind::Tool
    } else {
        ContextTagKind::Message
    }
}

fn current_protected_tail_start(
    messages: &[ChatMessage],
    protected_tail_len: usize,
) -> Option<ContextTagId> {
    if protected_tail_len == 0 {
        return None;
    }

    messages
        .iter()
        .enumerate()
        .rev()
        .filter_map(|(index, message)| {
            let id = tag_id_for_index(index)?;
            is_active_for_tail(message, id).then_some(id)
        })
        .nth(protected_tail_len - 1)
}

fn is_active_for_tail(message: &ChatMessage, id: ContextTagId) -> bool {
    !message.is_compact_boundary() && !message_is_drop_marker(message, id)
}

fn should_defer_protected_tail_drop(
    drop: &QueuedContextDrop,
    tail_start: Option<ContextTagId>,
) -> bool {
    if drop.replay_mode() != ContextDropReplayMode::ProtectedTailSkip {
        return false;
    }

    tail_start.is_some_and(|tail_start| drop.range().end() >= tail_start.get())
}

fn apply_context_drop(messages: &mut [ChatMessage], drop: &QueuedContextDrop) -> usize {
    let mut applied = 0;
    for tag in drop.range().start()..=drop.range().end() {
        let Some(index) = tag
            .checked_sub(1)
            .and_then(|zero_based| usize::try_from(zero_based).ok())
        else {
            continue;
        };
        let Some(message) = messages.get_mut(index) else {
            continue;
        };
        let Ok(id) = ContextTagId::new(tag) else {
            continue;
        };
        if message.is_compact_boundary() || message_is_drop_marker(message, id) {
            continue;
        }

        message.parts = vec![MessagePart::Text(dropped_tag_marker(id))];
        applied += 1;
    }
    applied
}

fn message_is_drop_marker(message: &ChatMessage, id: ContextTagId) -> bool {
    let marker = dropped_tag_marker(id);
    message.parts.iter().any(|part| part.text_only() == marker)
}

fn tag_id_for_index(index: usize) -> Option<ContextTagId> {
    let one_based = index.checked_add(1)?;
    let raw = u32::try_from(one_based).ok()?;
    ContextTagId::new(raw).ok()
}

fn usize_to_u64_saturating(value: usize) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests;

#[path = "context_reduction_pressure.rs"]
mod pressure;
pub(crate) use pressure::queue_pressure_reduction;

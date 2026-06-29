use crate::{ContextDropRange, ContextDropReplayMode, ContextSkeletonError, QueuedContextDrop};

use super::{ContextDropSpec, ContextTag, ContextTagId, ContextTagStatus};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ContextReduceOptions {
    protected_tail_len: usize,
    replay_mode: ContextDropReplayMode,
}

impl ContextReduceOptions {
    pub const fn new(protected_tail_len: usize) -> Self {
        Self {
            protected_tail_len,
            replay_mode: ContextDropReplayMode::Full,
        }
    }

    pub fn with_replay_mode(
        mut self,
        replay_mode: ContextDropReplayMode,
    ) -> Result<Self, ContextSkeletonError> {
        if matches!(replay_mode, ContextDropReplayMode::ProtectedTailSkip) {
            return Err(ContextSkeletonError::InvalidContextDropReplayMode);
        }

        self.replay_mode = replay_mode;
        Ok(self)
    }

    pub const fn protected_tail_len(self) -> usize {
        self.protected_tail_len
    }

    pub const fn replay_mode(self) -> ContextDropReplayMode {
        self.replay_mode
    }
}

impl Default for ContextReduceOptions {
    fn default() -> Self {
        Self::new(0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedContextDrops {
    queued: Vec<QueuedContextDrop>,
    protected_tail_skips: Vec<QueuedContextDrop>,
    already_pending: Vec<ContextTagId>,
    already_dropped: Vec<ContextTagId>,
}

impl PlannedContextDrops {
    pub fn plan(
        tags: &[ContextTag],
        spec: &ContextDropSpec,
        options: ContextReduceOptions,
    ) -> Result<Self, ContextSkeletonError> {
        let _linkscope_plan = linkscope::phase("context.drop_plan.plan");
        let _linkscope_plan_trace = linkscope::trace_fields(
            "context.drop_plan.plan",
            [
                linkscope::TraceField::count("tags", u64::try_from(tags.len()).unwrap_or(u64::MAX)),
                linkscope::TraceField::count(
                    "protected_tail_len",
                    u64::try_from(options.protected_tail_len()).unwrap_or(u64::MAX),
                ),
                linkscope::TraceField::text("replay_mode", format!("{:?}", options.replay_mode())),
            ],
        );
        let protected_tail_start = protected_tail_start(tags, options.protected_tail_len());
        let mut active = Vec::new();
        let mut protected = Vec::new();
        let mut already_pending = Vec::new();
        let mut already_dropped = Vec::new();

        let requested_ids = spec.tag_ids()?;
        linkscope::record_items(
            "context.drop_plan.requested",
            u64::try_from(requested_ids.len()).unwrap_or(u64::MAX),
        );
        for id in requested_ids {
            let tag = find_tag(tags, id).ok_or(ContextSkeletonError::UnknownContextTag)?;
            match tag.status() {
                ContextTagStatus::Active if is_protected(id, protected_tail_start) => {
                    protected.push(id);
                }
                ContextTagStatus::Active => active.push(id),
                ContextTagStatus::PendingDrop => already_pending.push(id),
                ContextTagStatus::Dropped => already_dropped.push(id),
                ContextTagStatus::Compacted => {
                    return Err(ContextSkeletonError::CompactedContextTag);
                }
            }
        }

        let planned = Self {
            queued: queued_drops(active, options.replay_mode())?,
            protected_tail_skips: protected_tail_skips(protected)?,
            already_pending,
            already_dropped,
        };
        linkscope::event_fields(
            "context.drop_plan.result",
            [
                linkscope::TraceField::count(
                    "queued",
                    u64::try_from(planned.queued.len()).unwrap_or(u64::MAX),
                ),
                linkscope::TraceField::count(
                    "protected_tail_skips",
                    u64::try_from(planned.protected_tail_skips.len()).unwrap_or(u64::MAX),
                ),
                linkscope::TraceField::count(
                    "already_pending",
                    u64::try_from(planned.already_pending.len()).unwrap_or(u64::MAX),
                ),
                linkscope::TraceField::count(
                    "already_dropped",
                    u64::try_from(planned.already_dropped.len()).unwrap_or(u64::MAX),
                ),
            ],
        );
        Ok(planned)
    }

    pub fn queued(&self) -> &[QueuedContextDrop] {
        &self.queued
    }

    pub fn protected_tail_skips(&self) -> &[QueuedContextDrop] {
        &self.protected_tail_skips
    }

    pub fn already_pending(&self) -> &[ContextTagId] {
        &self.already_pending
    }

    pub fn already_dropped(&self) -> &[ContextTagId] {
        &self.already_dropped
    }

    pub fn is_empty(&self) -> bool {
        self.queued.is_empty()
            && self.protected_tail_skips.is_empty()
            && self.already_pending.is_empty()
            && self.already_dropped.is_empty()
    }
}

fn protected_tail_start(tags: &[ContextTag], protected_tail_len: usize) -> Option<ContextTagId> {
    if protected_tail_len == 0 {
        return None;
    }

    tags.iter()
        .rev()
        .filter(|tag| tag.status() == ContextTagStatus::Active)
        .nth(protected_tail_len - 1)
        .map(|tag| tag.id())
}

fn is_protected(id: ContextTagId, protected_tail_start: Option<ContextTagId>) -> bool {
    protected_tail_start.is_some_and(|tail_start| id >= tail_start)
}

fn find_tag(tags: &[ContextTag], id: ContextTagId) -> Option<ContextTag> {
    tags.iter().copied().find(|tag| tag.id() == id)
}

fn queued_drops(
    ids: Vec<ContextTagId>,
    replay_mode: ContextDropReplayMode,
) -> Result<Vec<QueuedContextDrop>, ContextSkeletonError> {
    let _linkscope_queued = linkscope::phase("context.drop_plan.queued_drops");
    linkscope::record_items(
        "context.drop_plan.active_ids",
        u64::try_from(ids.len()).unwrap_or(u64::MAX),
    );
    ranges_from_ids(ids)?
        .into_iter()
        .map(|range| QueuedContextDrop::new(range, replay_mode))
        .collect()
}

fn protected_tail_skips(
    ids: Vec<ContextTagId>,
) -> Result<Vec<QueuedContextDrop>, ContextSkeletonError> {
    let _linkscope_protected = linkscope::phase("context.drop_plan.protected_tail_skips");
    linkscope::record_items(
        "context.drop_plan.protected_ids",
        u64::try_from(ids.len()).unwrap_or(u64::MAX),
    );
    ranges_from_ids(ids)?
        .into_iter()
        .map(|range| QueuedContextDrop::protected_tail_skip(range, range.start()))
        .collect()
}

fn ranges_from_ids(ids: Vec<ContextTagId>) -> Result<Vec<ContextDropRange>, ContextSkeletonError> {
    let _linkscope_ranges = linkscope::phase("context.drop_plan.ranges_from_ids");
    let mut ranges = Vec::new();
    let mut iter = ids.into_iter();
    let Some(first) = iter.next() else {
        return Ok(ranges);
    };

    let mut start = first.get();
    let mut end = start;
    for id in iter {
        let next = id.get();
        if next == end + 1 {
            end = next;
            continue;
        }

        ranges.push(ContextDropRange::new(start, end)?);
        start = next;
        end = next;
    }
    ranges.push(ContextDropRange::new(start, end)?);

    linkscope::record_items(
        "context.drop_plan.ranges",
        u64::try_from(ranges.len()).unwrap_or(u64::MAX),
    );
    Ok(ranges)
}

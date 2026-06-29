use std::collections::BTreeSet;

use crate::{ContextDropRange, ContextSkeletonError};

use super::{ContextTagId, MAX_DROP_TAGS};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextDropSpec {
    ranges: Vec<ContextDropRange>,
}

impl ContextDropSpec {
    pub fn parse(drop: &str) -> Result<Self, ContextSkeletonError> {
        let _linkscope_parse = linkscope::phase("context.drop_spec.parse");
        linkscope::record_bytes(
            "context.drop_spec.input",
            u64::try_from(drop.len()).unwrap_or(u64::MAX),
        );
        if drop.trim().is_empty() {
            return Err(ContextSkeletonError::EmptyContextDropSpec);
        }

        let mut ranges = Vec::new();
        for segment in drop.split(',') {
            ranges.push(parse_range(segment.trim())?);
        }
        linkscope::record_items(
            "context.drop_spec.ranges",
            u64::try_from(ranges.len()).unwrap_or(u64::MAX),
        );

        Ok(Self { ranges })
    }

    pub fn ranges(&self) -> &[ContextDropRange] {
        &self.ranges
    }

    pub(super) fn tag_ids(&self) -> Result<Vec<ContextTagId>, ContextSkeletonError> {
        let _linkscope_ids = linkscope::phase("context.drop_spec.tag_ids");
        let mut ids = BTreeSet::new();
        for range in &self.ranges {
            for id in range.start()..=range.end() {
                ids.insert(ContextTagId::new(id)?);
                if ids.len() > MAX_DROP_TAGS {
                    return Err(ContextSkeletonError::InvalidContextDropSpec);
                }
            }
        }

        let ids = ids.into_iter().collect::<Vec<_>>();
        linkscope::record_items(
            "context.drop_spec.tag_ids",
            u64::try_from(ids.len()).unwrap_or(u64::MAX),
        );
        Ok(ids)
    }
}

fn parse_range(segment: &str) -> Result<ContextDropRange, ContextSkeletonError> {
    if segment.is_empty() {
        return Err(ContextSkeletonError::InvalidContextDropSpec);
    }

    let Some((start, end)) = segment.split_once('-') else {
        let position = parse_position(segment)?;
        return ContextDropRange::new(position, position);
    };

    if end.contains('-') {
        return Err(ContextSkeletonError::InvalidContextDropSpec);
    }

    ContextDropRange::new(parse_position(start.trim())?, parse_position(end.trim())?)
}

fn parse_position(position: &str) -> Result<u32, ContextSkeletonError> {
    if position.is_empty() || !position.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(ContextSkeletonError::InvalidContextDropSpec);
    }

    position
        .parse()
        .map_err(|_| ContextSkeletonError::InvalidContextDropSpec)
}

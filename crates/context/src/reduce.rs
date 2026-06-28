use crate::ContextSkeletonError;
use serde::{Deserialize, Serialize, de};

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ReducePlan(String);

impl ReducePlan {
    pub fn new(plan: impl Into<String>) -> Result<Self, ContextSkeletonError> {
        let plan = plan.into();
        if plan.trim().is_empty() {
            return Err(ContextSkeletonError::EmptyReducePlan);
        }

        Ok(Self(plan))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
pub struct ContextDropRange {
    start: u32,
    end: u32,
}

impl ContextDropRange {
    pub fn new(start: u32, end: u32) -> Result<Self, ContextSkeletonError> {
        if start == 0 || end == 0 || start > end {
            return Err(ContextSkeletonError::InvalidContextDropRange);
        }

        Ok(Self { start, end })
    }

    pub fn start(self) -> u32 {
        self.start
    }

    pub fn end(self) -> u32 {
        self.end
    }

    pub fn contains(self, position: u32) -> bool {
        self.start <= position && position <= self.end
    }
}

impl<'de> Deserialize<'de> for ContextDropRange {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct RawContextDropRange {
            start: u32,
            end: u32,
        }

        let raw = RawContextDropRange::deserialize(deserializer)?;
        Self::new(raw.start, raw.end).map_err(de::Error::custom)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ContextDropReplayMode {
    Full,
    Skeleton,
    EditMarker,
    ProtectedTailSkip,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct QueuedContextDrop {
    range: ContextDropRange,
    replay_mode: ContextDropReplayMode,
    #[serde(skip_serializing_if = "Option::is_none")]
    protected_tail_start: Option<u32>,
}

impl QueuedContextDrop {
    pub fn new(
        range: ContextDropRange,
        replay_mode: ContextDropReplayMode,
    ) -> Result<Self, ContextSkeletonError> {
        Self::from_parts(range, replay_mode, None)
    }

    pub fn protected_tail_skip(
        range: ContextDropRange,
        protected_tail_start: u32,
    ) -> Result<Self, ContextSkeletonError> {
        Self::from_parts(
            range,
            ContextDropReplayMode::ProtectedTailSkip,
            Some(protected_tail_start),
        )
    }

    pub fn range(&self) -> ContextDropRange {
        self.range
    }

    pub fn replay_mode(&self) -> ContextDropReplayMode {
        self.replay_mode
    }

    pub fn protected_tail_start(&self) -> Option<u32> {
        self.protected_tail_start
    }

    fn from_parts(
        range: ContextDropRange,
        replay_mode: ContextDropReplayMode,
        protected_tail_start: Option<u32>,
    ) -> Result<Self, ContextSkeletonError> {
        match replay_mode {
            ContextDropReplayMode::ProtectedTailSkip => {
                let protected_tail_start =
                    protected_tail_start.ok_or(ContextSkeletonError::ProtectedTailStartRequired)?;
                if !range.contains(protected_tail_start) {
                    return Err(ContextSkeletonError::InvalidProtectedTailStart);
                }
                Ok(Self {
                    range,
                    replay_mode,
                    protected_tail_start: Some(protected_tail_start),
                })
            }
            _ if protected_tail_start.is_some() => {
                Err(ContextSkeletonError::UnexpectedProtectedTailStart)
            }
            _ => Ok(Self {
                range,
                replay_mode,
                protected_tail_start: None,
            }),
        }
    }
}

impl<'de> Deserialize<'de> for QueuedContextDrop {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct RawQueuedContextDrop {
            range: ContextDropRange,
            replay_mode: ContextDropReplayMode,
            #[serde(default)]
            protected_tail_start: Option<u32>,
        }

        let raw = RawQueuedContextDrop::deserialize(deserializer)?;
        Self::from_parts(raw.range, raw.replay_mode, raw.protected_tail_start)
            .map_err(de::Error::custom)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContextReductionQueue {
    #[serde(default)]
    drops: Vec<QueuedContextDrop>,
}

impl ContextReductionQueue {
    pub fn new(drops: impl IntoIterator<Item = QueuedContextDrop>) -> Self {
        Self {
            drops: drops.into_iter().collect(),
        }
    }

    pub fn drops(&self) -> &[QueuedContextDrop] {
        &self.drops
    }

    pub fn is_empty(&self) -> bool {
        self.drops.is_empty()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
pub struct ProviderToolPair {
    tool_use_turn: u32,
    tool_result_turn: u32,
}

impl ProviderToolPair {
    pub fn new(tool_use_turn: u32, tool_result_turn: u32) -> Result<Self, ContextSkeletonError> {
        if tool_use_turn == 0 || tool_result_turn == 0 {
            return Err(ContextSkeletonError::InvalidProviderToolPair);
        }

        if tool_use_turn.checked_add(1) != Some(tool_result_turn) {
            return Err(ContextSkeletonError::InvalidProviderToolPair);
        }

        Ok(Self {
            tool_use_turn,
            tool_result_turn,
        })
    }

    pub fn tool_use_turn(self) -> u32 {
        self.tool_use_turn
    }

    pub fn tool_result_turn(self) -> u32 {
        self.tool_result_turn
    }

    pub fn remains_valid_after(self, drop_range: ContextDropRange) -> bool {
        drop_range.contains(self.tool_use_turn) == drop_range.contains(self.tool_result_turn)
    }
}

impl<'de> Deserialize<'de> for ProviderToolPair {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct RawProviderToolPair {
            tool_use_turn: u32,
            tool_result_turn: u32,
        }

        let raw = RawProviderToolPair::deserialize(deserializer)?;
        Self::new(raw.tool_use_turn, raw.tool_result_turn).map_err(de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ContextDropRange, ContextDropReplayMode, ContextReductionQueue, ProviderToolPair,
        QueuedContextDrop,
    };

    #[test]
    fn reduction_queue_serializes_replay_modes_normal() {
        let queue = ContextReductionQueue::new([
            QueuedContextDrop::new(
                ContextDropRange::new(2, 3).expect("valid range"),
                ContextDropReplayMode::Full,
            )
            .expect("full drop"),
            QueuedContextDrop::new(
                ContextDropRange::new(4, 8).expect("valid range"),
                ContextDropReplayMode::Skeleton,
            )
            .expect("skeleton drop"),
            QueuedContextDrop::new(
                ContextDropRange::new(9, 9).expect("valid range"),
                ContextDropReplayMode::EditMarker,
            )
            .expect("edit-marker drop"),
            QueuedContextDrop::protected_tail_skip(
                ContextDropRange::new(10, 12).expect("valid range"),
                11,
            )
            .expect("protected-tail skip"),
        ]);

        let json = serde_json::to_string(&queue).expect("queue serializes");

        assert!(json.contains("\"full\""));
        assert!(json.contains("\"skeleton\""));
        assert!(json.contains("\"edit-marker\""));
        assert!(json.contains("\"protected-tail-skip\""));

        let reparsed: ContextReductionQueue =
            serde_json::from_str(&json).expect("queue deserializes");
        assert_eq!(reparsed.drops(), queue.drops());
    }

    #[test]
    fn reduction_queue_rejects_invalid_range_malformed() {
        let malformed = r#"
            {
                "drops": [{
                    "range": { "start": 8, "end": 4 },
                    "replay_mode": "skeleton"
                }]
            }
        "#;

        let error = serde_json::from_str::<ContextReductionQueue>(malformed)
            .expect_err("inverted ranges are invalid");

        assert!(error.to_string().contains("context drop range is invalid"));
    }

    #[test]
    fn reduction_queue_rejects_unknown_replay_mode_malformed() {
        let malformed = r#"
            {
                "drops": [{
                    "range": { "start": 4, "end": 8 },
                    "replay_mode": "summary-ish"
                }]
            }
        "#;

        serde_json::from_str::<ContextReductionQueue>(malformed)
            .expect_err("unknown replay modes are invalid");
    }

    #[test]
    fn protected_tail_skip_requires_tail_inside_range_malformed() {
        let malformed = r#"
            {
                "drops": [{
                    "range": { "start": 4, "end": 8 },
                    "replay_mode": "protected-tail-skip",
                    "protected_tail_start": 9
                }]
            }
        "#;

        let error = serde_json::from_str::<ContextReductionQueue>(malformed)
            .expect_err("protected tail skip must point inside the skipped range");

        assert!(
            error
                .to_string()
                .contains("protected tail start is invalid")
        );
    }

    #[test]
    fn dropped_provider_tool_pair_remains_valid_normal() {
        let drop_range = ContextDropRange::new(2, 3).expect("valid drop range");
        let pair = ProviderToolPair::new(2, 3).expect("adjacent tool pair");

        assert!(pair.remains_valid_after(drop_range));
    }

    #[test]
    fn half_dropped_provider_tool_pair_is_invalid_robust() {
        let drop_range = ContextDropRange::new(2, 2).expect("valid drop range");
        let pair = ProviderToolPair::new(2, 3).expect("adjacent tool pair");

        assert!(!pair.remains_valid_after(drop_range));
    }
}

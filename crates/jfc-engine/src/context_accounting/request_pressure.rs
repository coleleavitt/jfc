use jfc_core::context_budget::ContextBudget;

use crate::compact::{
    CompactLevel, blocked_threshold_with_output, compact_level_with_output,
    compact_threshold_with_output,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct RequestContextPressure {
    pub(crate) budget: ContextBudget,
    pub(crate) raw_tokens: u64,
    pub(crate) effective_tokens: u64,
    pub(crate) overhead_tokens: usize,
    pub(crate) window_tokens: Option<usize>,
    pub(crate) max_output_tokens: Option<usize>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContextPressureNudgeKind {
    ChannelOne,
    ChannelTwo,
    Emergency,
}

impl ContextPressureNudgeKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::ChannelOne => "channel_one",
            Self::ChannelTwo => "channel_two",
            Self::Emergency => "emergency",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ContextPressureNudge {
    pub kind: ContextPressureNudgeKind,
    pub level: CompactLevel,
    pub raw_tokens: u64,
    pub effective_tokens: u64,
    pub window_tokens: usize,
    pub threshold_tokens: usize,
    pub reclaim_floor_tokens: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct RequestContextOverflow {
    pub(crate) raw_tokens: u64,
    pub(crate) effective_tokens: u64,
    pub(crate) window_tokens: usize,
    pub(crate) level: CompactLevel,
}

fn clamp_u64_to_usize(tokens: u64) -> usize {
    usize::try_from(tokens).unwrap_or(usize::MAX)
}

impl RequestContextPressure {
    pub(crate) fn new(
        budget: ContextBudget,
        window_tokens: Option<usize>,
        max_output_tokens: Option<usize>,
    ) -> Self {
        let raw_tokens = jfc_core::context_budget::raw_tokens(budget);
        let effective_tokens = jfc_core::context_budget::effective_tokens(budget);
        let overhead_tokens = clamp_u64_to_usize(
            budget
                .system_prompt_tokens
                .saturating_add(budget.tool_definition_tokens)
                .saturating_add(budget.memory_tokens)
                .saturating_add(budget.project_instructions_tokens),
        );
        Self {
            budget,
            raw_tokens,
            effective_tokens,
            overhead_tokens,
            window_tokens,
            max_output_tokens,
        }
    }

    pub(crate) fn preflight_overflow(self) -> Option<RequestContextOverflow> {
        let window_tokens = self.window_tokens?;
        let level = self.compact_level()?;
        matches!(level, CompactLevel::Compact | CompactLevel::Blocked).then_some(
            RequestContextOverflow {
                raw_tokens: self.raw_tokens,
                effective_tokens: self.effective_tokens,
                window_tokens,
                level,
            },
        )
    }

    pub(crate) fn compact_level(self) -> Option<CompactLevel> {
        let window_tokens = self.window_tokens?;
        Some(compact_level_with_output(
            clamp_u64_to_usize(self.raw_tokens),
            window_tokens,
            self.max_output_tokens,
        ))
    }

    pub(crate) fn context_pressure_nudge(self) -> Option<ContextPressureNudge> {
        let window_tokens = self.window_tokens?;
        let level = self.compact_level()?;
        let (kind, threshold_tokens) = match level {
            CompactLevel::Ok | CompactLevel::Precompute => return None,
            CompactLevel::Warn => (
                ContextPressureNudgeKind::ChannelOne,
                compact_threshold_with_output(window_tokens, self.max_output_tokens)
                    .saturating_sub(20_000),
            ),
            CompactLevel::Compact => (
                ContextPressureNudgeKind::ChannelTwo,
                compact_threshold_with_output(window_tokens, self.max_output_tokens),
            ),
            CompactLevel::Blocked => (
                ContextPressureNudgeKind::Emergency,
                blocked_threshold_with_output(window_tokens, self.max_output_tokens),
            ),
        };
        let reclaim_floor_tokens = self
            .raw_tokens
            .saturating_sub(threshold_tokens as u64)
            .saturating_add(1);
        Some(ContextPressureNudge {
            kind,
            level,
            raw_tokens: self.raw_tokens,
            effective_tokens: self.effective_tokens,
            window_tokens,
            threshold_tokens,
            reclaim_floor_tokens,
        })
    }
}

#[cfg(test)]
#[path = "request_pressure_tests.rs"]
mod tests;

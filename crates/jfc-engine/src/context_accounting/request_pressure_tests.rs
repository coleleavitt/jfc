use jfc_core::context_budget::ContextBudget;

use super::*;
use crate::compact::{CompactLevel, blocked_threshold_with_output, compact_threshold_with_output};

fn budget_with_replay_tokens(user_message_tokens: u64) -> ContextBudget {
    ContextBudget {
        system_prompt_tokens: 10_000,
        tool_definition_tokens: 12_000,
        memory_tokens: 0,
        project_instructions_tokens: 0,
        user_message_tokens,
    }
}

#[test]
fn preflight_overflow_blocks_huge_prepared_request_regression() {
    let pressure = RequestContextPressure::new(
        budget_with_replay_tokens(8_000_000),
        Some(200_000),
        Some(8_192),
    );

    let overflow = pressure
        .preflight_overflow()
        .expect("huge prepared request should be caught before provider call");

    assert_eq!(overflow.raw_tokens, pressure.raw_tokens);
    assert_eq!(overflow.effective_tokens, pressure.effective_tokens);
    assert_eq!(overflow.window_tokens, 200_000);
    assert_eq!(overflow.level, CompactLevel::Blocked);
}

#[test]
fn preflight_overflow_waits_when_window_unknown_normal() {
    let pressure =
        RequestContextPressure::new(budget_with_replay_tokens(8_000_000), None, Some(8_192));

    assert_eq!(pressure.preflight_overflow(), None);
}

#[test]
fn preflight_overflow_allows_small_prepared_request_normal() {
    let pressure =
        RequestContextPressure::new(budget_with_replay_tokens(2_000), Some(200_000), Some(8_192));

    assert_eq!(pressure.preflight_overflow(), None);
}

#[test]
fn context_pressure_nudge_waits_when_window_unknown_normal() {
    let pressure =
        RequestContextPressure::new(budget_with_replay_tokens(200_000), None, Some(8_192));

    assert_eq!(pressure.context_pressure_nudge(), None);
}

#[test]
fn context_pressure_nudge_emits_channel_one_at_warn_normal() {
    let pressure = RequestContextPressure::new(
        budget_with_replay_tokens(150_000),
        Some(200_000),
        Some(8_192),
    );

    let nudge = pressure
        .context_pressure_nudge()
        .expect("warn pressure should emit a channel-one nudge");

    assert_eq!(nudge.kind, ContextPressureNudgeKind::ChannelOne);
    assert_eq!(nudge.level, CompactLevel::Warn);
    assert_eq!(
        nudge.threshold_tokens,
        compact_threshold_with_output(200_000, Some(8_192)).saturating_sub(20_000)
    );
    assert_eq!(
        nudge.reclaim_floor_tokens,
        nudge
            .raw_tokens
            .saturating_sub(nudge.threshold_tokens as u64)
            .saturating_add(1)
    );
}

#[test]
fn context_pressure_nudge_emits_channel_two_at_compact_normal() {
    let pressure = RequestContextPressure::new(
        budget_with_replay_tokens(165_000),
        Some(200_000),
        Some(8_192),
    );

    let nudge = pressure
        .context_pressure_nudge()
        .expect("compact pressure should emit a channel-two nudge");

    assert_eq!(nudge.kind, ContextPressureNudgeKind::ChannelTwo);
    assert_eq!(nudge.level, CompactLevel::Compact);
    assert_eq!(
        nudge.threshold_tokens,
        compact_threshold_with_output(200_000, Some(8_192))
    );
}

#[test]
fn context_pressure_nudge_emits_emergency_at_blocked_normal() {
    let pressure = RequestContextPressure::new(
        budget_with_replay_tokens(170_000),
        Some(200_000),
        Some(8_192),
    );

    let nudge = pressure
        .context_pressure_nudge()
        .expect("blocked pressure should emit an emergency nudge");

    assert_eq!(nudge.kind, ContextPressureNudgeKind::Emergency);
    assert_eq!(nudge.level, CompactLevel::Blocked);
    assert_eq!(
        nudge.threshold_tokens,
        blocked_threshold_with_output(200_000, Some(8_192))
    );
}

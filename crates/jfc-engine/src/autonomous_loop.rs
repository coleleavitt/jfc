//! Autonomous loop — persistent background agent that works while the user is away.
//!
//! Mirrors Claude Code v2.1.146+'s `/loop` command with two pacing modes:
//! - **Cron-based**: Uses CronCreate for fixed-interval ticks
//! - **Dynamic**: Uses ScheduleWakeup for self-paced ticks
//!
//! The loop reads `.claude/loop.md` or `loop.md` for task directives and
//! injects a trust-boundary preamble into the system prompt.

use std::path::{Path, PathBuf};

/// Sentinel strings that identify autonomous loop wakeup prompts.
pub const LOOP_SENTINEL_CRON: &str = "<<autonomous-loop>>";
pub const LOOP_SENTINEL_DYNAMIC: &str = "<<autonomous-loop-dynamic>>";

/// Fallback delay when a dynamic loop tick finishes without scheduling
/// its own next wakeup. Mirrors Claude 2.1.177's `Pz5` keepalive fallback
/// (1200s).
pub const LOOP_KEEPALIVE_DELAY_SECONDS: u32 = 20 * 60;

/// Minimum dynamic-wakeup delay (seconds). A model-chosen delay below this is
/// clamped up. Mirrors Claude 2.1.177's `ZT$` (60s).
pub const LOOP_WAKEUP_MIN_DELAY_SECONDS: u32 = 60;

/// Maximum dynamic-wakeup delay (seconds). A model-chosen delay above this is
/// clamped down. Mirrors Claude 2.1.177's `yJ8` (3600s).
pub const LOOP_WAKEUP_MAX_DELAY_SECONDS: u32 = 3600;

/// Keepalive budget: how many times the keepalive may fire (because the model
/// declined to reschedule) before the dynamic loop self-terminates. Mirrors
/// Claude 2.1.177's `Wz5` (1) — after the first model-no-reschedule keepalive,
/// a second decline ends the loop rather than firing again.
pub const LOOP_KEEPALIVE_BUDGET: u8 = 1;

/// Maximum size of loop.md content (bytes) before truncation.
const MAX_LOOP_FILE_BYTES: usize = 8192;

/// After this many consecutive no-op ticks, the loop self-terminates.
const MAX_CONSECUTIVE_NOOP_TICKS: u8 = 3;

/// Pacing mode for the autonomous loop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoopPacing {
    /// Fixed-interval via CronCreate (e.g. every 5 minutes).
    Cron,
    /// Self-paced via ScheduleWakeup (agent decides delay each tick).
    Dynamic,
}

/// Runtime state for an active autonomous loop.
#[derive(Debug, Clone)]
pub struct AutonomousLoopState {
    pub pacing: LoopPacing,
    pub consecutive_noop_ticks: u8,
    pub total_ticks: u32,
    pub started_at: std::time::Instant,
    pub loop_file_content: Option<String>,
    /// How many times the keepalive has fired because the model declined to
    /// reschedule the dynamic loop itself. Bounded by `LOOP_KEEPALIVE_BUDGET`;
    /// once exhausted the loop self-terminates instead of firing again.
    pub keepalive_fired_count: u8,
}

impl AutonomousLoopState {
    pub fn new(pacing: LoopPacing) -> Self {
        Self {
            pacing,
            consecutive_noop_ticks: 0,
            total_ticks: 0,
            started_at: std::time::Instant::now(),
            loop_file_content: None,
            keepalive_fired_count: 0,
        }
    }

    pub fn should_stop(&self) -> bool {
        self.consecutive_noop_ticks >= MAX_CONSECUTIVE_NOOP_TICKS
    }

    pub fn record_tick(&mut self, had_work: bool) {
        self.total_ticks += 1;
        if had_work {
            self.consecutive_noop_ticks = 0;
        } else {
            self.consecutive_noop_ticks += 1;
        }
        // A tick that ran resets the keepalive budget — the loop is alive again,
        // so a future model-no-reschedule gets a fresh keepalive allowance.
        // Mirrors Claude 2.1.177 `FY$(0)` on a non-keepalive reschedule.
        if self.pacing == LoopPacing::Dynamic {
            self.keepalive_fired_count = 0;
        }
    }

    /// Whether the keepalive may fire again (model declined to reschedule).
    /// Returns `false` once the budget is exhausted; mirrors Claude 2.1.177's
    /// `hc$() >= Wz5` guard that ends the loop after the model declines twice.
    pub fn keepalive_budget_available(&self) -> bool {
        self.keepalive_fired_count < LOOP_KEEPALIVE_BUDGET
    }

    /// Record that the keepalive fired this tick (model did not reschedule).
    pub fn record_keepalive_fired(&mut self) {
        self.keepalive_fired_count = self.keepalive_fired_count.saturating_add(1);
    }
}

/// Clamp a model-chosen dynamic-wakeup delay to the supported window
/// `[LOOP_WAKEUP_MIN_DELAY_SECONDS, LOOP_WAKEUP_MAX_DELAY_SECONDS]`. Returns the
/// clamped value and whether clamping occurred. Mirrors Claude 2.1.177's `Zz5`
/// clamp of the model's `delaySeconds` to `[ZT$, yJ8]`.
pub fn clamp_wakeup_delay_seconds(requested: u32) -> (u32, bool) {
    let clamped = requested.clamp(LOOP_WAKEUP_MIN_DELAY_SECONDS, LOOP_WAKEUP_MAX_DELAY_SECONDS);
    (clamped, clamped != requested)
}

/// Check if a wakeup prompt is an autonomous loop sentinel.
pub fn is_loop_sentinel(prompt: &str) -> bool {
    prompt == LOOP_SENTINEL_CRON || prompt == LOOP_SENTINEL_DYNAMIC
}

/// Whether dynamic autonomous loops should be kept alive when the model
/// forgets to call ScheduleWakeup. Enabled by default for active loops;
/// set JFC_LOOP_KEEPALIVE=0 to restore strict model-driven pacing.
pub fn loop_keepalive_enabled() -> bool {
    std::env::var("JFC_LOOP_KEEPALIVE")
        .ok()
        .and_then(|value| parse_loop_keepalive_flag(&value))
        .or_else(|| {
            std::env::var("CLAUDE_CODE_LOOP_KEEPALIVE")
                .ok()
                .and_then(|value| parse_loop_keepalive_flag(&value))
        })
        .unwrap_or(true)
}

pub fn parse_loop_keepalive_flag(value: &str) -> Option<bool> {
    match value.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Some(true),
        "0" | "false" | "no" | "off" => Some(false),
        _ => None,
    }
}

/// Read loop.md from the project, truncating if too large.
pub fn read_loop_file(project_root: &Path) -> Option<String> {
    let candidates = [
        project_root.join(".claude").join("loop.md"),
        project_root.join("loop.md"),
    ];
    for path in &candidates {
        if let Ok(content) = std::fs::read_to_string(path) {
            let trimmed = content.trim();
            if trimmed.is_empty() {
                continue;
            }
            if trimmed.len() <= MAX_LOOP_FILE_BYTES {
                return Some(trimmed.to_string());
            }
            let truncated = &trimmed[..MAX_LOOP_FILE_BYTES];
            let last_newline = truncated.rfind('\n').unwrap_or(MAX_LOOP_FILE_BYTES);
            return Some(format!(
                "{}\n\n> WARNING: loop.md was truncated to {} bytes. Keep the task list concise.",
                &trimmed[..last_newline],
                MAX_LOOP_FILE_BYTES
            ));
        }
    }
    None
}

/// Generate the autonomous loop tick preamble for injection into the system prompt.
pub fn loop_tick_preamble(pacing: LoopPacing) -> &'static str {
    match pacing {
        LoopPacing::Cron => CRON_TICK_PREAMBLE,
        LoopPacing::Dynamic => DYNAMIC_TICK_PREAMBLE,
    }
}

/// The path(s) we search for loop.md.
pub fn loop_file_paths(project_root: &Path) -> Vec<PathBuf> {
    vec![
        project_root.join(".claude").join("loop.md"),
        project_root.join("loop.md"),
    ]
}

const CRON_TICK_PREAMBLE: &str = "\
# Autonomous loop tick

Run the autonomous check using the loop instructions established earlier in this \
conversation. If you cannot find them, treat this as a no-op tick. The recurring \
cron will fire the next tick automatically — do not call ScheduleWakeup from this tick.";

const DYNAMIC_TICK_PREAMBLE: &str = "\
# Autonomous loop tick (dynamic pacing)

Run the autonomous check using the loop instructions established earlier in this \
conversation. If you cannot find them, treat this as a no-op tick.

You scheduled this tick via the ScheduleWakeup tool (not a recurring cron). To keep \
the loop alive, call ScheduleWakeup again at the end of this turn with `prompt` set \
to the literal sentinel `<<autonomous-loop-dynamic>>` — otherwise the loop ends after this tick. \
Pick `delaySeconds` between 60 and 3600; values outside that window are clamped. If a Monitor \
is armed (check TaskList), keep `delaySeconds` at 1200–1800 — the Monitor is the wake signal \
and this is only the fallback heartbeat. If you were woken by a task notification, handle the \
event before rescheduling.";

/// Full autonomous loop system preamble (injected once at loop start).
pub const AUTONOMOUS_LOOP_PREAMBLE: &str = r#"# Autonomous loop check

You are being invoked on a timer while the user is away or occupied. Keep already
established work moving: finish tasks the conversation started, maintain PRs the user
is building, and catch blockers before the user returns. Do not invent new work just
because the timer fired.

Trust is the boundary. Continue work when the transcript, branch, or PR clearly shows
the user wanted it done. When you are unsure whether an action continues established
work or starts something new, prefer reversible investigation and wait on irreversible
steps.

## What to act on

Re-read the current conversation first. Prioritize unfinished implementation, explicit
commitments, skipped verification, and unresolved questions you can now answer. If you
find actionable work in that category, do the work instead of describing it.

If the conversation has no active work left, inspect the current branch's PR or merge
request. Check CI status, unresolved review threads, whether the branch is behind the
base branch, and obvious blockers in the latest discussion. Diagnose failing jobs from
their logs before changing code. Re-run only failures that look transient. Fix real
failures with the smallest change that preserves the user's intent.

When a PR has green CI, no unresolved review threads, and no branch maintenance left,
a narrow bug-hunt or simplification pass is acceptable if it stays inside the work the
branch is already doing.

## Reversibility rule

For reversible actions such as reading, editing locally, running tests, or drafting
messages, bias toward acting. For irreversible actions such as pushing, deleting,
sending external messages, or resolving review threads, require clear authorization or
an established pattern in the transcript that makes the user's intent obvious.

## Repeated invocations

If earlier autonomous checks are visible, adjust scope. A previous unanswered question
does not block reversible work, but it does block irreversible work. After three
consecutive checks with nothing actionable, do one quick PR/CI/thread check and then
stop with a single quiet status line.

## When to stop

Stop when the original work is complete, the user asked you to stop, or the repeated
invocation rule says the loop is quiet. Do not fill idle ticks with speculative work."#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_loop_keepalive_flag_normal() {
        assert_eq!(parse_loop_keepalive_flag("1"), Some(true));
        assert_eq!(parse_loop_keepalive_flag("on"), Some(true));
        assert_eq!(parse_loop_keepalive_flag("false"), Some(false));
        assert_eq!(parse_loop_keepalive_flag("0"), Some(false));
    }

    #[test]
    fn parse_loop_keepalive_flag_ignores_unknown_robust() {
        assert_eq!(parse_loop_keepalive_flag(""), None);
        assert_eq!(parse_loop_keepalive_flag("maybe"), None);
    }

    // Normal: a delay inside the window passes through unchanged.
    #[test]
    fn clamp_wakeup_delay_passes_in_window_normal() {
        assert_eq!(clamp_wakeup_delay_seconds(1200), (1200, false));
        assert_eq!(
            clamp_wakeup_delay_seconds(LOOP_WAKEUP_MIN_DELAY_SECONDS),
            (LOOP_WAKEUP_MIN_DELAY_SECONDS, false)
        );
        assert_eq!(
            clamp_wakeup_delay_seconds(LOOP_WAKEUP_MAX_DELAY_SECONDS),
            (LOOP_WAKEUP_MAX_DELAY_SECONDS, false)
        );
    }

    // Robust: an over-eager or oversized delay is clamped to the window and the
    // clamp flag flips. Mirrors Claude 2.1.177's Zz5 clamp.
    #[test]
    fn clamp_wakeup_delay_clamps_out_of_window_robust() {
        assert_eq!(
            clamp_wakeup_delay_seconds(5),
            (LOOP_WAKEUP_MIN_DELAY_SECONDS, true)
        );
        assert_eq!(
            clamp_wakeup_delay_seconds(100_000),
            (LOOP_WAKEUP_MAX_DELAY_SECONDS, true)
        );
        assert_eq!(
            clamp_wakeup_delay_seconds(0),
            (LOOP_WAKEUP_MIN_DELAY_SECONDS, true)
        );
    }

    // Normal: keepalive budget allows exactly LOOP_KEEPALIVE_BUDGET fires, then
    // closes. Mirrors Claude 2.1.177's Wz5=1 decline budget.
    #[test]
    fn keepalive_budget_exhausts_after_limit_normal() {
        let mut state = AutonomousLoopState::new(LoopPacing::Dynamic);
        assert!(state.keepalive_budget_available());
        state.record_keepalive_fired();
        assert_eq!(state.keepalive_fired_count, LOOP_KEEPALIVE_BUDGET);
        assert!(!state.keepalive_budget_available());
    }

    // Robust: a real tick (loop alive again) resets the keepalive budget so a
    // later model-no-reschedule gets a fresh allowance.
    #[test]
    fn keepalive_budget_resets_on_tick_robust() {
        let mut state = AutonomousLoopState::new(LoopPacing::Dynamic);
        state.record_keepalive_fired();
        assert!(!state.keepalive_budget_available());
        state.record_tick(true);
        assert!(state.keepalive_budget_available());
        assert_eq!(state.keepalive_fired_count, 0);
    }

    // Robust: the dynamic preamble surfaces the clamp window and monitor-aware
    // delay guidance ported from Claude 2.1.177.
    #[test]
    fn dynamic_preamble_mentions_clamp_window_and_monitor_robust() {
        let preamble = loop_tick_preamble(LoopPacing::Dynamic);
        assert!(preamble.contains("60 and 3600"));
        assert!(preamble.contains("Monitor"));
        assert!(preamble.contains("1200"));
    }
}

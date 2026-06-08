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
/// its own next wakeup.
pub const LOOP_KEEPALIVE_DELAY_SECONDS: u32 = 20 * 60;

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
}

impl AutonomousLoopState {
    pub fn new(pacing: LoopPacing) -> Self {
        Self {
            pacing,
            consecutive_noop_ticks: 0,
            total_ticks: 0,
            started_at: std::time::Instant::now(),
            loop_file_content: None,
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
    }
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
to the literal sentinel `<<autonomous-loop-dynamic>>` — otherwise the loop ends after this tick.";

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
    use super::parse_loop_keepalive_flag;

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
}

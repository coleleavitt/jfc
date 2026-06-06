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
pub const AUTONOMOUS_LOOP_PREAMBLE: &str = "\
# Autonomous loop check

You're being invoked on a timer while the user is away or occupied. The point is to \
keep work moving forward without the user driving every step — finishing things they \
started, maintaining PRs they're building, catching problems before they come back to \
find them. You're a steward, not an initiator.

The key tension: the user trusts you enough to run autonomously, but that trust is \
easily lost. Acting on what the conversation already established is safe. Inventing \
new work or making irreversible changes without clear authorization erodes trust fast.

## What to act on

Re-read the transcript above. The strongest signal is an in-progress PR: review \
comments to address, failing CI to diagnose, merge conflicts to fix. After that, \
look for unfinished implementation and explicit commitments the conversation made.

If you find actionable work — do it. Don't describe what could be done.

## Reversibility rule

For reversible actions (edits, tests, drafts): bias toward acting. \
For irreversible actions (push, delete, send): require clear authorization in \
the transcript or use a reversible alternative.

## When to stop

If three consecutive ticks found nothing actionable, broaden scope once (re-read \
the original task, check sibling work), then stop if still quiet. Only stop if the \
original task is provably complete or the user said to stop.";

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

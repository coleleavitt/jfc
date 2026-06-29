//! View-layer reveal cap for live streaming assistant text.
//!
//! JFC's engine appends every received `StreamEvent::Chunk` immediately into
//! `EngineState` (the single source of truth — unchanged). The default TUI path
//! now calls [`StreamPacer::reveal_all`] on every tick so the transcript renders
//! received text immediately, matching Claude/Codex behavior. The adaptive
//! [`advance`] path remains available for narrow experiments/tests, but visible
//! smoothing belongs in the spinner token counter, not in transcript text.
//!
//! ## Model
//!
//! The pacer is purely a function of `(total_lines, revealed, now)`:
//!
//! - `total_lines` = how many display lines the live streaming message currently
//!   has (computed by the renderer from `EngineState`).
//! - the unrevealed tail `total_lines - revealed` is treated as the "queue".
//! - on each [`StreamPacer::advance`] tick the [`AdaptiveChunkingPolicy`] decides
//!   `Single` (reveal one line) or `Batch(n)` (drain the backlog), and `revealed`
//!   moves toward `total_lines`.
//!
//! The owning view ([`crate::app::App`]) holds one pacer, [`reset`]s it when a
//! new stream starts, and normally calls [`reveal_all`] while streaming and when
//! the stream finishes (so nothing is held back).
//!
//! [`advance`]: StreamPacer::advance
//! [`reset`]: StreamPacer::reset
//! [`reveal_all`]: StreamPacer::reveal_all
//! [`chunking`]: super::chunking
//! [`AdaptiveChunkingPolicy`]: super::chunking::AdaptiveChunkingPolicy
//! [`crate::app::App`]: crate::app::App

use std::time::Instant;

use super::chunking::AdaptiveChunkingPolicy;
use super::chunking::DrainPlan;
use super::chunking::QueueSnapshot;

/// Reveal-cap state for one live stream, layered over the engine's
/// (already-immediate) streaming text.
#[derive(Debug, Default)]
pub(crate) struct StreamPacer {
    policy: AdaptiveChunkingPolicy,
    /// How many display lines of the current streaming message are shown.
    revealed: usize,
    /// When the current backlog (revealed < total) first appeared — used to
    /// derive the queue's `oldest_age` so the policy can escalate to catch-up
    /// on age, not just depth.
    backlog_since: Option<Instant>,
}

impl StreamPacer {
    /// Start fresh for a new streaming message.
    pub(crate) fn reset(&mut self) {
        self.policy.reset();
        self.revealed = 0;
        self.backlog_since = None;
    }

    /// Lines currently revealed for display.
    pub(crate) fn revealed(&self) -> usize {
        self.revealed
    }

    /// Advance the reveal toward `total_lines` using the adaptive policy, and
    /// return the number of lines that should now be shown (clamped to
    /// `total_lines`). Call once per animation tick while streaming.
    ///
    /// Returns the new revealed count; the caller renders exactly that many
    /// lines of the live message. When `revealed < total_lines` after this
    /// call, the caller should keep requesting animation frames.
    pub(crate) fn advance(&mut self, total_lines: usize, now: Instant) -> usize {
        // A shrink (e.g. the renderer recomputed fewer lines) or full catch-up
        // collapses the backlog clock.
        if self.revealed >= total_lines {
            self.revealed = total_lines;
            self.backlog_since = None;
            // Keep the policy informed that the queue is empty so it relaxes
            // back to smooth and the re-entry hold is seeded correctly.
            let _ = self.policy.decide(QueueSnapshot::default(), now);
            return self.revealed;
        }

        let queued = total_lines - self.revealed;
        if self.backlog_since.is_none() {
            self.backlog_since = Some(now);
        }
        let oldest_age = self
            .backlog_since
            .map(|since| now.saturating_duration_since(since));

        let decision = self.policy.decide(
            QueueSnapshot {
                queued_lines: queued,
                oldest_age,
            },
            now,
        );

        let to_reveal = match decision.drain_plan {
            DrainPlan::Single => 1,
            DrainPlan::Batch(n) => n,
        };
        self.revealed = (self.revealed + to_reveal).min(total_lines);
        if self.revealed >= total_lines {
            self.backlog_since = None;
        } else {
            // Still behind: restart the age clock from now so the next tick's
            // age reflects the *current* head-of-line wait, not the whole burst.
            self.backlog_since = Some(now);
        }
        self.revealed
    }

    /// Reveal everything immediately (stream finished / finalized). Nothing is
    /// ever held back past end-of-turn.
    pub(crate) fn reveal_all(&mut self, total_lines: usize) {
        self.revealed = total_lines;
        self.backlog_since = None;
    }

    /// Whether there is still un-revealed text (the caller should keep
    /// scheduling animation frames while true).
    pub(crate) fn is_catching_up(&self, total_lines: usize) -> bool {
        self.revealed < total_lines
    }
}

/// Number of display segments in `s` (lines split on `'\n'`, the trailing
/// partial line included). This is the pacer's `total_lines`: a one-line
/// response with no trailing newline counts as 1 so it can't stay invisible
/// during streaming. Empty input is 0.
pub(crate) fn display_line_count(s: &str) -> usize {
    if s.is_empty() {
        0
    } else {
        s.bytes().filter(|&b| b == b'\n').count() + 1
    }
}

/// Borrow the first `n` display segments of `s` (everything up to, but not
/// including, the `n`-th `'\n'`). When `s` has fewer than `n` newlines the whole
/// string is returned, so once the pacer reveals every segment the full text —
/// including the in-progress partial line — shows. `n == 0` yields `""`.
pub(crate) fn take_first_lines(s: &str, n: usize) -> &str {
    if n == 0 {
        return "";
    }
    let mut newlines = 0usize;
    for (i, b) in s.bytes().enumerate() {
        if b == b'\n' {
            newlines += 1;
            if newlines == n {
                return &s[..i];
            }
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn display_line_count_counts_segments_incl_partial() {
        assert_eq!(display_line_count(""), 0);
        assert_eq!(display_line_count("Hello!"), 1); // no newline -> 1 segment
        assert_eq!(display_line_count("a\nb\nc"), 3);
        assert_eq!(display_line_count("a\nb\n"), 3); // trailing empty segment counts
    }

    #[test]
    fn take_first_lines_slices_on_segment_boundaries() {
        assert_eq!(take_first_lines("a\nb\nc", 0), "");
        assert_eq!(take_first_lines("a\nb\nc", 1), "a");
        assert_eq!(take_first_lines("a\nb\nc", 2), "a\nb");
        assert_eq!(take_first_lines("a\nb\nc", 3), "a\nb\nc"); // fewer newlines than n -> all
        assert_eq!(take_first_lines("a\nb\nc", 9), "a\nb\nc");
        assert_eq!(take_first_lines("Hello!", 1), "Hello!"); // single-line response reveals fully
    }

    #[test]
    fn reveals_one_line_per_tick_when_smooth() {
        let mut pacer = StreamPacer::default();
        let t0 = Instant::now();
        // 3 lines available, smooth pacing reveals one at a time.
        assert_eq!(pacer.advance(3, t0), 1);
        assert_eq!(pacer.advance(3, t0 + Duration::from_millis(16)), 2);
        assert_eq!(pacer.advance(3, t0 + Duration::from_millis(32)), 3);
        assert!(!pacer.is_catching_up(3));
    }

    #[test]
    fn batches_under_depth_pressure() {
        let mut pacer = StreamPacer::default();
        let t0 = Instant::now();
        // A burst of 20 lines lands at once: depth >= 8 triggers catch-up,
        // which drains the whole backlog in one tick.
        let revealed = pacer.advance(20, t0);
        assert_eq!(revealed, 20, "catch-up should drain the burst");
        assert!(!pacer.is_catching_up(20));
    }

    #[test]
    fn reset_clears_revealed() {
        let mut pacer = StreamPacer::default();
        let t0 = Instant::now();
        pacer.advance(5, t0);
        pacer.reset();
        assert_eq!(pacer.revealed(), 0);
        // After reset, a fresh smooth stream reveals one line again.
        assert_eq!(pacer.advance(5, t0 + Duration::from_millis(16)), 1);
    }

    #[test]
    fn reveal_all_flushes_on_finish() {
        let mut pacer = StreamPacer::default();
        let t0 = Instant::now();
        pacer.advance(10, t0); // smooth: 1 revealed
        pacer.reveal_all(10);
        assert_eq!(pacer.revealed(), 10);
        assert!(!pacer.is_catching_up(10));
    }

    #[test]
    fn revealed_never_exceeds_total() {
        let mut pacer = StreamPacer::default();
        let t0 = Instant::now();
        // Even a severe batch can't over-reveal.
        let revealed = pacer.advance(2, t0);
        assert!(revealed <= 2);
    }

    #[test]
    fn age_pressure_escalates_to_catch_up() {
        let mut pacer = StreamPacer::default();
        let t0 = Instant::now();
        // Two lines sit unrevealed long enough (>= 120ms) that age — not depth —
        // forces catch-up and drains them.
        assert_eq!(pacer.advance(2, t0), 1); // smooth: reveal 1, 1 remains queued
        let revealed = pacer.advance(2, t0 + Duration::from_millis(200));
        assert_eq!(revealed, 2, "aged backlog should catch up");
    }
}

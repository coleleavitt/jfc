//! `<system-reminder>` injection — a v132 idiom for surfacing
//! environmental warnings, mode transitions, and recall-context snippets
//! to the model without making them look like user instructions.
//!
//! From v2.1.132's `cli.js`:
//!
//! > <system-reminder>` blocks are background context, not user
//! > instructions, and reflect what was true when written — if one
//! > names a file, function, or flag, verify it still exists before
//! > recommending it.
//!
//! Patterns we use:
//!   * `Read` returns a system-reminder when the file exists but is
//!     empty or the offset overshoots the line count.
//!   * Plan-mode transitions emit a one-line system-reminder so the
//!     next turn knows the mode flipped.
//!   * Memory recall (when wired) drops the synthesized facts in a
//!     `<system-reminder>` block so the model treats them as
//!     background context, not user-supplied requirements.
//!
//! The wrapping is *visible to the model* but documented in its base
//! prompt as "tags from the system, not from the user." See
//! `~/VulnerabilityResearch/anthropic/extracted_2.1.132/src/entrypoints/cli.js`
//! for the canonical format and base-prompt rules.

use crate::types::{ChatMessage, MessagePart, Role};

/// Wrap `body` in `<system-reminder>...</system-reminder>` tags.
/// Returns a fresh String so callers can splice the result anywhere
/// (user message, tool_result content, etc.).
pub fn format(body: &str) -> String {
    format!("<system-reminder>\n{}\n</system-reminder>", body.trim())
}

/// Append a `<system-reminder>` to the last user message in
/// `messages`. If no user message exists, append a new one. Used for
/// out-of-band signals like "the user just toggled plan mode" that
/// the next turn needs to see *before* it processes the queued user
/// prompt.
pub fn append_to_last_user(messages: &mut Vec<ChatMessage>, body: &str) {
    let formatted = format(body);
    if let Some(last_user) = messages.iter_mut().rfind(|m| m.role == Role::User) {
        last_user.parts.push(MessagePart::Text(formatted));
        return;
    }
    messages.push(ChatMessage::user(formatted));
}

/// Periodic "persist what you learned" nudge.
///
/// Ported from Hermes Agent (`agent/turn_context.py` `_turns_since_memory` /
/// `_memory_nudge_interval`): every N user turns, inject a background reminder
/// that prompts the agent to save durable facts via its memory tool. This is
/// what makes an agent *reliably* write memories mid-session, complementing the
/// post-hoc historian extraction. Deterministic + cheap (no model call).
///
/// The counter is owned by the caller (engine state) so it survives the turn
/// and can be hydrated across restarts. `interval == 0` disables nudging.
#[derive(Debug, Clone)]
pub struct MemoryNudge {
    /// Fire a nudge every `interval` user turns. `0` disables.
    pub interval: u32,
    /// User turns observed since the last nudge fired.
    pub turns_since: u32,
}

impl Default for MemoryNudge {
    fn default() -> Self {
        // Mirrors Hermes' default `_memory_nudge_interval = 10`.
        Self {
            interval: 10,
            turns_since: 0,
        }
    }
}

impl MemoryNudge {
    pub fn new(interval: u32) -> Self {
        Self {
            interval,
            turns_since: 0,
        }
    }

    /// Record one user turn. Returns the nudge body to inject when the interval
    /// is reached (and resets the counter), or `None` otherwise.
    pub fn on_user_turn(&mut self) -> Option<String> {
        if self.interval == 0 {
            return None;
        }
        self.turns_since += 1;
        if self.turns_since >= self.interval {
            self.turns_since = 0;
            Some(MEMORY_NUDGE_BODY.to_string())
        } else {
            None
        }
    }

    /// Hydrate the counter from the number of prior user turns this session, so
    /// a restart mid-interval resumes at the right phase (Hermes issue #22357).
    pub fn hydrate(&mut self, prior_user_turns: u32) {
        if self.interval > 0 {
            self.turns_since = prior_user_turns % self.interval;
        }
    }
}

/// The nudge text. Phrased as background context (it's wrapped in a
/// system-reminder), pointing the agent at its own memory-write path.
pub const MEMORY_NUDGE_BODY: &str = "Checkpoint: if anything durable emerged since the last few \
turns — a user preference or correction, a project decision, a non-obvious constraint, or a \
confirmed approach — save it now with the memory tool so it survives this session. Skip if \
nothing is worth persisting (don't save ephemeral task state or anything already in the code).";

#[cfg(test)]
mod tests {
    use super::*;

    /// Normal: format wraps body with the canonical v132 tag.
    #[test]
    fn format_wraps_with_tags_normal() {
        let s = format("File is empty");
        assert!(s.starts_with("<system-reminder>\n"));
        assert!(s.ends_with("\n</system-reminder>"));
        assert!(s.contains("File is empty"));
    }

    /// Robust: leading/trailing whitespace in body is trimmed so the
    /// wrapper renders cleanly regardless of caller hygiene.
    #[test]
    fn format_trims_body_robust() {
        let s = format("  trimmed  ");
        // Tag boundaries are clean newlines bracketing the content.
        assert!(s.contains("\ntrimmed\n"));
        assert!(!s.contains("\n  trimmed"));
    }

    /// Normal: append_to_last_user adds a Text part to the existing
    /// user message rather than creating a new one — keeps the
    /// transcript shape stable.
    #[test]
    fn append_to_last_user_extends_existing_user_normal() {
        let mut msgs = vec![
            ChatMessage::assistant("hi".into()),
            ChatMessage::user("query".into()),
        ];
        append_to_last_user(&mut msgs, "you are now in Accept Edits mode");
        assert_eq!(msgs.len(), 2);
        let user = msgs.iter().rfind(|m| m.role == Role::User).unwrap();
        assert!(
            user.parts
                .iter()
                .any(|p| matches!(p, MessagePart::Text(t) if t.contains("Accept Edits"))),
            "reminder should be appended to existing user message",
        );
    }

    /// Robust: when no user message exists, append_to_last_user
    /// creates a synthetic one rather than panic. Without this fall-
    /// back, calling the helper in cold start would silently lose
    /// the reminder.
    #[test]
    fn append_to_last_user_creates_when_missing_robust() {
        let mut msgs: Vec<ChatMessage> = vec![];
        append_to_last_user(&mut msgs, "boot reminder");
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].role, Role::User);
    }

    // ─── MemoryNudge (Hermes parity) ────────────────────────────────────────

    /// Normal: the nudge fires exactly on the Nth user turn and resets.
    #[test]
    fn memory_nudge_fires_on_interval_normal() {
        let mut n = MemoryNudge::new(3);
        assert!(n.on_user_turn().is_none(), "turn 1");
        assert!(n.on_user_turn().is_none(), "turn 2");
        let body = n.on_user_turn().expect("turn 3 fires");
        assert!(body.contains("memory tool"));
        // Counter reset → next two turns are quiet again.
        assert!(n.on_user_turn().is_none(), "turn 4");
        assert!(n.on_user_turn().is_none(), "turn 5");
        assert!(n.on_user_turn().is_some(), "turn 6 fires");
    }

    /// Robust: interval 0 disables nudging entirely (never fires).
    #[test]
    fn memory_nudge_zero_interval_never_fires_robust() {
        let mut n = MemoryNudge::new(0);
        for _ in 0..50 {
            assert!(n.on_user_turn().is_none());
        }
    }

    /// Robust: hydration resumes at the right phase after a restart, so a
    /// session that already had `prior` turns doesn't re-start the count.
    #[test]
    fn memory_nudge_hydrates_phase_robust() {
        let mut n = MemoryNudge::new(10);
        n.hydrate(7); // 7 turns already happened this session
        assert_eq!(n.turns_since, 7);
        // 3 more turns reach the interval and fire.
        assert!(n.on_user_turn().is_none());
        assert!(n.on_user_turn().is_none());
        assert!(n.on_user_turn().is_some());
    }
}

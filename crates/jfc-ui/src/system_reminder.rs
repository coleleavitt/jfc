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
            user.parts.iter().any(|p| matches!(p, MessagePart::Text(t) if t.contains("Accept Edits"))),
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
}

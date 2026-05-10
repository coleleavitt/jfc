//! Auto-generate a short, human-readable title for a session after the
//! first turn finishes.
//!
//! v132 fires a small Haiku call after the first user/assistant exchange
//! to summarize the conversation into a 4–6 word title. The default
//! `display_title` fallback truncates the user's first line, which is
//! usually fine but loses meaning for long prompts ("can you help me
//! with the…" → "can you help me with the…"). A Haiku call replaces
//! that with something like "Refactor auth middleware".
//!
//! This module is intentionally lightweight: a single function that
//! takes the active provider, the first user message, and the first
//! assistant response, asks the model for a title, and persists it via
//! `session::set_session_title`. Failures are swallowed — title is
//! cosmetic and shouldn't disrupt the chat.

use crate::provider::{
    ModelId, Provider, ProviderContent, ProviderMessage, ProviderRole, StreamOptions,
};
use std::sync::Arc;

const MAX_TITLE_CHARS: usize = 60;

/// Build the prompt asking the model for a 4–6 word title summarizing
/// the first turn. Returns a single ProviderMessage list ready to send.
fn build_request(first_user: &str, first_assistant: &str) -> Vec<ProviderMessage> {
    let user_truncated: String = first_user.chars().take(800).collect();
    let assistant_truncated: String = first_assistant.chars().take(800).collect();
    let body = format!(
        "Summarize the following first-turn exchange into a 4–6 word title \
         suitable for a session sidebar. Use Title Case. No quotes, no \
         trailing punctuation, no emojis. Output the title only — no \
         preamble, no explanation.\n\n\
         User: {user_truncated}\n\n\
         Assistant: {assistant_truncated}\n\n\
         Title:"
    );
    vec![ProviderMessage {
        role: ProviderRole::User,
        content: vec![ProviderContent::Text(body)],
    }]
}

/// Generate a title and persist it to the session file. Best-effort —
/// returns the title that was written, or `None` on any failure.
pub async fn generate_and_save(
    session_id: crate::ids::SessionId,
    provider: Arc<dyn Provider>,
    model: ModelId,
    first_user: String,
    first_assistant: String,
) -> Option<String> {
    let messages = build_request(&first_user, &first_assistant);
    let opts = StreamOptions::new(model.as_str())
        .system("You generate concise, descriptive session titles.")
        .max_tokens(40);

    let resp = match provider.complete(messages, &opts).await {
        Ok(r) => r,
        Err(e) => {
            tracing::debug!(
                target: "jfc::session_naming",
                error = %e,
                "title generation failed (provider.complete)"
            );
            return None;
        }
    };

    let title = sanitize(&resp.content);
    if title.is_empty() {
        tracing::debug!(target: "jfc::session_naming", "model returned empty title");
        return None;
    }

    crate::session::set_session_title(&session_id, &title).await;
    tracing::info!(
        target: "jfc::session_naming",
        %session_id,
        title = %title,
        "auto-generated session title"
    );
    Some(title)
}

/// Strip quotes, trailing punctuation, and excess whitespace; clamp
/// length to MAX_TITLE_CHARS.
fn sanitize(raw: &str) -> String {
    let mut s = raw.trim().to_owned();
    // Strip surrounding quotes if present.
    for q in ['"', '\'', '`', '“', '”'] {
        s = s.trim_matches(q).trim().to_owned();
    }
    // Drop leading "Title:" prefix if the model returned the prompt label.
    for prefix in ["Title:", "title:", "TITLE:"] {
        if let Some(rest) = s.strip_prefix(prefix) {
            s = rest.trim().to_owned();
        }
    }
    // Single line only.
    if let Some(line) = s.lines().next() {
        s = line.to_owned();
    }
    // Trailing punctuation.
    s = s
        .trim_end_matches(['.', ',', ';', ':', '!', '?'])
        .to_owned();
    // Length cap.
    if s.chars().count() > MAX_TITLE_CHARS {
        s = s.chars().take(MAX_TITLE_CHARS).collect();
    }
    s.trim().to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_strips_quotes_normal() {
        assert_eq!(sanitize("\"Refactor auth\""), "Refactor auth");
        assert_eq!(sanitize("'Add caching'"), "Add caching");
        assert_eq!(sanitize("`Fix bug`"), "Fix bug");
    }

    #[test]
    fn sanitize_strips_title_prefix_normal() {
        assert_eq!(sanitize("Title: Refactor auth"), "Refactor auth");
        assert_eq!(sanitize("title: add tests"), "add tests");
    }

    #[test]
    fn sanitize_strips_trailing_punctuation_normal() {
        assert_eq!(sanitize("Refactor auth."), "Refactor auth");
        assert_eq!(sanitize("Add tests!"), "Add tests");
    }

    #[test]
    fn sanitize_clamps_long_titles_robust() {
        let long = "A".repeat(100);
        let out = sanitize(&long);
        assert!(out.chars().count() <= MAX_TITLE_CHARS);
    }

    #[test]
    fn sanitize_handles_multiline_robust() {
        assert_eq!(sanitize("First line\nSecond line"), "First line");
    }

    #[test]
    fn sanitize_empty_is_empty_robust() {
        assert_eq!(sanitize(""), "");
        assert_eq!(sanitize("   "), "");
    }

    #[test]
    fn build_request_truncates_long_inputs_robust() {
        let long_user = "u".repeat(2000);
        let long_assistant = "a".repeat(2000);
        let req = build_request(&long_user, &long_assistant);
        assert_eq!(req.len(), 1);
        // The combined prompt should not contain the full 2000-char inputs.
        let body_len: usize = req[0]
            .content
            .iter()
            .map(|c| match c {
                ProviderContent::Text(t) => t.len(),
                _ => 0,
            })
            .sum();
        // System prompt + truncated user (800) + truncated assistant (800) ~< 2500
        assert!(
            body_len < 2500,
            "body length {body_len} exceeds expected cap"
        );
    }
}

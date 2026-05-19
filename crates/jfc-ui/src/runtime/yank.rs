//! Clipboard helpers — local clipboard via `arboard` *and* the OSC 52
//! terminal escape so SSH/tmux sessions copy correctly without a local
//! clipboard daemon.
//!
//! Two entry points:
//!   * [`yank_last_assistant`] — used by Ctrl+Y and the mouse click-to-
//!     copy handler; copies the most recent assistant message text.
//!   * [`copy_to_clipboard`] — generic helper for the `/copy` slash
//!     command; takes any text payload.

use std::io::Write;

use crate::{
    app::App,
    types::{MessagePart, Role},
};

/// Copy the text of the most recent assistant message to the local
/// clipboard *and* echo it through an OSC 52 escape so terminal
/// multiplexers / SSH sessions get a copy too.
pub(crate) fn yank_last_assistant(app: &App) {
    let Some(text) = last_assistant_text(app).filter(|t| !t.is_empty()) else {
        return;
    };
    copy_to_clipboard(&text, "yank_last_assistant");
}

/// Pull the rendered text of the last assistant message. Pure helper —
/// no side effects, exposed at crate-vis for the `/copy last` path.
pub(crate) fn last_assistant_text(app: &App) -> Option<String> {
    app.messages
        .iter()
        .rev()
        .find(|m| m.role == Role::Assistant)
        .map(|m| join_text_parts(&m.parts))
}

/// Flatten an entire transcript to a plaintext blob. Used by `/copy all`.
pub(crate) fn full_transcript_text(app: &App) -> String {
    let mut out = String::new();
    for msg in &app.messages {
        let role_label = match msg.role {
            Role::User => "user",
            Role::Assistant => "assistant",
        };
        let body = join_text_parts(&msg.parts);
        if body.is_empty() {
            continue;
        }
        out.push_str(&format!("[{role_label}]\n{body}\n\n"));
    }
    out.trim_end().to_owned()
}

/// Last N assistant + user messages, plaintext. Used by `/copy <n>`.
pub(crate) fn tail_transcript_text(app: &App, n: usize) -> String {
    let mut taken = 0;
    let mut chunks: Vec<String> = Vec::new();
    for msg in app.messages.iter().rev() {
        if taken >= n {
            break;
        }
        let role_label = match msg.role {
            Role::User => "user",
            Role::Assistant => "assistant",
        };
        let body = join_text_parts(&msg.parts);
        if body.is_empty() {
            continue;
        }
        chunks.push(format!("[{role_label}]\n{body}"));
        taken += 1;
    }
    chunks.reverse();
    chunks.join("\n\n")
}

fn join_text_parts(parts: &[MessagePart]) -> String {
    parts
        .iter()
        .filter_map(|p| match p {
            MessagePart::Text(t) => Some(t.clone()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Write `text` to the local clipboard (via `arboard`) AND emit an OSC
/// 52 escape so terminal-multiplexers, SSH, and copy-over-the-wire
/// flows pick it up. Logs at info on success, warn on failure, but
/// never panics — clipboard backends are notoriously flaky on Linux
/// (no display server, sandboxed wayland, etc.) and we want the copy
/// to be best-effort.
pub(crate) fn copy_to_clipboard(text: &str, source: &str) {
    if text.is_empty() {
        return;
    }
    // 1. Local clipboard via arboard.
    match arboard::Clipboard::new() {
        Ok(mut clipboard) => match clipboard.set_text(text.to_owned()) {
            Ok(()) => tracing::info!(
                target: "jfc::ui::yank",
                source,
                len = text.len(),
                "clipboard set via arboard"
            ),
            Err(e) => tracing::warn!(
                target: "jfc::ui::yank",
                source,
                error = %e,
                "arboard set_text failed; OSC 52 fallback only"
            ),
        },
        Err(e) => tracing::warn!(
            target: "jfc::ui::yank",
            source,
            error = %e,
            "arboard backend unavailable; OSC 52 fallback only"
        ),
    }

    // 2. OSC 52 — terminal escape for SSH/tmux/Kitty/iTerm2 etc. Format:
    //    ESC ] 52 ; c ; <base64-payload> BEL
    // We write directly to stderr so ratatui's frame buffer doesn't
    // swallow the escape. Bounded at 100KB because xterm rejects bigger
    // payloads silently (some terminals quietly truncate at smaller
    // ceilings, but 100KB is the widely-quoted spec ceiling).
    const OSC52_MAX: usize = 100 * 1024;
    let payload: &str = if text.len() > OSC52_MAX {
        tracing::warn!(
            target: "jfc::ui::yank",
            source,
            len = text.len(),
            "OSC 52 payload exceeds 100KB; truncating"
        );
        &text[..text
            .char_indices()
            .take_while(|(i, _)| *i < OSC52_MAX)
            .last()
            .map(|(i, c)| i + c.len_utf8())
            .unwrap_or(0)]
    } else {
        text
    };
    let encoded = base64_encode(payload.as_bytes());
    let escape = format!("\x1b]52;c;{encoded}\x07");
    let mut stderr = std::io::stderr().lock();
    if let Err(e) = stderr.write_all(escape.as_bytes()) {
        tracing::warn!(
            target: "jfc::ui::yank",
            source,
            error = %e,
            "OSC 52 write failed"
        );
    } else {
        let _ = stderr.flush();
        tracing::debug!(
            target: "jfc::ui::yank",
            source,
            len = payload.len(),
            "OSC 52 escape emitted"
        );
    }
}

/// Standard base64 (RFC 4648) encoder. Pulled inline so we don't add a
/// `base64` crate dependency just for this one path — the OSC 52 alphabet
/// is the standard one, no URL-safe variant.
fn base64_encode(input: &[u8]) -> String {
    const ALPHABET: &[u8; 64] =
        b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity((input.len() + 2) / 3 * 4);
    for chunk in input.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = chunk.get(1).copied().unwrap_or(0) as u32;
        let b2 = chunk.get(2).copied().unwrap_or(0) as u32;
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(ALPHABET[((n >> 18) & 0x3f) as usize] as char);
        out.push(ALPHABET[((n >> 12) & 0x3f) as usize] as char);
        if chunk.len() > 1 {
            out.push(ALPHABET[((n >> 6) & 0x3f) as usize] as char);
        } else {
            out.push('=');
        }
        if chunk.len() > 2 {
            out.push(ALPHABET[(n & 0x3f) as usize] as char);
        } else {
            out.push('=');
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::base64_encode;

    #[test]
    fn base64_empty_normal() {
        assert_eq!(base64_encode(b""), "");
    }

    #[test]
    fn base64_known_vectors_normal() {
        // RFC 4648 §10.
        assert_eq!(base64_encode(b"f"), "Zg==");
        assert_eq!(base64_encode(b"fo"), "Zm8=");
        assert_eq!(base64_encode(b"foo"), "Zm9v");
        assert_eq!(base64_encode(b"foob"), "Zm9vYg==");
        assert_eq!(base64_encode(b"fooba"), "Zm9vYmE=");
        assert_eq!(base64_encode(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn base64_round_through_special_bytes_robust() {
        // All 256 byte values to make sure no signed/unsigned bug
        // leaks into the index math.
        let raw: Vec<u8> = (0u8..=255).collect();
        let out = base64_encode(&raw);
        assert!(out.chars().all(|c| {
            c.is_ascii_alphanumeric() || c == '+' || c == '/' || c == '='
        }));
        // 256 bytes → 344 chars (256 / 3 = 85.33, round up to 86 groups × 4 = 344).
        assert_eq!(out.len(), 344);
    }
}

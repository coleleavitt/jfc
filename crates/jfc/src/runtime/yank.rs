//! Clipboard helpers — every text-copy path in the TUI funnels through
//! [`copy_to_clipboard`], which hands the payload to a single
//! process-lifetime [`ClipboardOwner`] thread. That thread owns the
//! `arboard` handle for the whole session (so X11/Wayland clipboard
//! ownership survives — see [`ClipboardOwner`]) and emits an OSC 52
//! escape so SSH/tmux/remote sessions copy correctly too. The actual
//! (potentially blocking) clipboard I/O runs on that thread, never on
//! the render/event loop.

use std::io::Write;
use std::sync::OnceLock;
use std::sync::mpsc::{self, Receiver, Sender};

use crate::app::App;
use jfc_core::{MessagePart, Role};

/// Pull the rendered text of the last assistant message. Pure helper —
/// no side effects, exposed at crate-vis for the `/copy last` path.
pub(crate) fn last_assistant_text(app: &App) -> Option<String> {
    app.engine
        .messages
        .iter()
        .rev()
        .find(|m| m.role == Role::Assistant)
        .map(|m| join_text_parts(&m.parts))
}

/// Flatten an entire transcript to a plaintext blob. Used by `/copy all`.
pub(crate) fn full_transcript_text(app: &App) -> String {
    let mut out = String::new();
    for msg in &app.engine.messages {
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
    for msg in app.engine.messages.iter().rev() {
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

/// Copy `text` to the system clipboard. The payload is handed to the
/// process-lifetime [`ClipboardOwner`] thread, which keeps the native
/// clipboard handle alive (so the copy survives on X11/Wayland) and
/// emits an OSC 52 escape for SSH/tmux/remote. This call itself is a
/// non-blocking channel send — safe to invoke from the render path.
/// `source` is a static label used only for logging.
pub(crate) fn copy_to_clipboard(text: &str, source: &'static str) {
    if text.is_empty() {
        return;
    }
    clipboard_owner().copy(text.to_owned(), source);
}

/// Detect an SSH session. Over SSH the *native* clipboard (arboard) writes
/// to the remote machine the user can't see (and may block), so we skip it
/// and let OSC 52 — which the local terminal emulator intercepts — carry the
/// copy.
///
/// Gate on `SSH_CONNECTION`, NOT `SSH_TTY`: a tmux pane inherits `SSH_TTY`
/// forever even after you detach the SSH session and reattach locally, so
/// keying on `SSH_TTY` would wrongly suppress the native clipboard on a local
/// pane. `SSH_CONNECTION` is in tmux's default `update-environment` set and
/// clears on local attach. (Lesson lifted from Claude Code's `osc.ts`.)
fn is_ssh_session() -> bool {
    std::env::var_os("SSH_CONNECTION").is_some()
}

/// Wrap an escape sequence for tmux / GNU screen DCS passthrough so OSC 52
/// reaches the *outer* terminal. Without this, the multiplexer swallows the
/// raw sequence and the copy never reaches the system clipboard. tmux gates
/// passthrough behind `set -g allow-passthrough on`; when off it silently
/// drops the whole DCS (no worse than an unwrapped sequence it would ignore).
/// Whether the active terminal is kitty (which accepts `ST` to terminate
/// OSC 52, avoiding a BEL beep on some configs).
fn is_kitty() -> bool {
    std::env::var_os("KITTY_WINDOW_ID").is_some()
        || std::env::var("TERM").is_ok_and(|t| t.contains("kitty"))
}

/// OSC 52 terminator: `ST` (`ESC \`) on kitty when NOT inside a multiplexer —
/// inside tmux/screen the `ST`'s `ESC` would collide with the DCS terminator
/// that `wrap_for_multiplexer` appends, so fall back to the universal `BEL`.
fn osc52_terminator(is_kitty: bool, in_mux: bool) -> &'static str {
    if is_kitty && !in_mux {
        "\x1b\\"
    } else {
        "\x07"
    }
}

fn wrap_for_multiplexer(seq: &str) -> String {
    if std::env::var_os("TMUX").is_some() {
        // Inner ESCs must be doubled inside tmux passthrough.
        let escaped = seq.replace('\x1b', "\x1b\x1b");
        format!("\x1bPtmux;{escaped}\x1b\\")
    } else if std::env::var_os("STY").is_some() {
        format!("\x1bP{seq}\x1b\\")
    } else {
        seq.to_owned()
    }
}

/// Lazily-spawned, process-lifetime owner of the system clipboard.
fn clipboard_owner() -> &'static ClipboardOwner {
    static OWNER: OnceLock<ClipboardOwner> = OnceLock::new();
    OWNER.get_or_init(ClipboardOwner::spawn)
}

/// A long-lived owner of the system clipboard handle.
///
/// On X11 and most Wayland compositors the clipboard is *served by the
/// process that set it*: the instant the `arboard::Clipboard` is dropped, a
/// freshly-copied selection vanishes. Historically every copy path in this
/// crate created a `Clipboard`, called `set_text`, and dropped it on the same
/// line — so on Linux the copy was gone before the user could paste.
/// `ClipboardOwner` holds the handle on a dedicated thread for the whole
/// process lifetime, and runs the (possibly blocking) clipboard + OSC 52 I/O
/// there so a copy never stalls a render frame.
struct ClipboardOwner {
    tx: Sender<(String, &'static str)>,
}

impl ClipboardOwner {
    fn spawn() -> Self {
        let (tx, rx) = mpsc::channel::<(String, &'static str)>();
        let _ = std::thread::Builder::new()
            .name("jfc-clipboard".into())
            .spawn(move || clipboard_owner_loop(rx));
        Self { tx }
    }

    fn copy(&self, text: String, source: &'static str) {
        // Best-effort: the owner is process-lifetime, so a closed channel
        // only happens at shutdown, where OSC 52 / the terminal already hold
        // the copy.
        let _ = self.tx.send((text, source));
    }
}

/// Owns the native clipboard handle across copies and serves each request.
fn clipboard_owner_loop(rx: Receiver<(String, &'static str)>) {
    let ssh = is_ssh_session();
    // Kept alive between copies so X11/Wayland clipboard ownership survives.
    // `None` over SSH (native would hit the remote box) or until the first
    // successful `Clipboard::new()`.
    let mut native: Option<arboard::Clipboard> = None;
    while let Ok((text, source)) = rx.recv() {
        if !ssh {
            if native.is_none() {
                match arboard::Clipboard::new() {
                    Ok(cb) => native = Some(cb),
                    Err(e) => tracing::warn!(
                        target: "jfc::ui::yank",
                        source,
                        error = %e,
                        "arboard backend unavailable; OSC 52 only"
                    ),
                }
            }
            if let Some(cb) = native.as_mut() {
                match cb.set_text(text.clone()) {
                    Ok(()) => tracing::info!(
                        target: "jfc::ui::yank",
                        source,
                        len = text.len(),
                        "clipboard set via arboard"
                    ),
                    Err(e) => {
                        tracing::warn!(
                            target: "jfc::ui::yank",
                            source,
                            error = %e,
                            "arboard set_text failed; dropping handle, OSC 52 only"
                        );
                        // Drop the handle so the next copy re-creates it.
                        native = None;
                    }
                }
            }
        }
        emit_osc52(&text, source);
    }
}

/// Emit an OSC 52 clipboard escape on stderr.
///
/// Format: `ESC ] 52 ; c ; <base64-payload> BEL`. Written to stderr so
/// ratatui's stdout frame buffer doesn't swallow it. Bounded at 100KB
/// because xterm rejects bigger payloads silently.
fn emit_osc52(text: &str, source: &'static str) {
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
    let in_mux = std::env::var_os("TMUX").is_some() || std::env::var_os("STY").is_some();
    let term = osc52_terminator(is_kitty(), in_mux);
    let escape = wrap_for_multiplexer(&format!("\x1b]52;c;{encoded}{term}"));
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
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(input.len().div_ceil(3) * 4);
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
    use super::{base64_encode, osc52_terminator};

    #[test]
    fn osc52_terminator_picks_st_only_on_bare_kitty_normal() {
        assert_eq!(osc52_terminator(true, false), "\x1b\\"); // kitty, no mux → ST
        assert_eq!(osc52_terminator(true, true), "\x07"); // kitty in tmux → BEL
        assert_eq!(osc52_terminator(false, false), "\x07"); // non-kitty → BEL
        assert_eq!(osc52_terminator(false, true), "\x07");
    }

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
        assert!(
            out.chars()
                .all(|c| { c.is_ascii_alphanumeric() || c == '+' || c == '/' || c == '=' })
        );
        // 256 bytes → 344 chars (256 / 3 = 85.33, round up to 86 groups × 4 = 344).
        assert_eq!(out.len(), 344);
    }
}

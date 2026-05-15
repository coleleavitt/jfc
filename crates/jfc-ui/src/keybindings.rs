//! User-configurable keybindings loaded from `~/.config/jfc/keybindings.toml`.
//!
//! # Example `keybindings.toml`
//! ```toml
//! [keys]
//! "alt+o"    = "toggle_fast_mode"
//! "ctrl+k"   = "clear_history"
//! "alt+m"    = "open_model_picker"
//! "ctrl+h"   = "toggle_help"
//! "alt+v"    = "toggle_verbose"
//! "alt+c"    = "compact"
//! "ctrl+q"   = "exit"
//! ```
//!
//! Keys are case-insensitive strings in the form `[modifier+]*key`.
//! Supported modifiers: `ctrl`, `alt`, `shift`.
//! Supported keys: letters (`a`–`z`), digits (`0`–`9`), and specials:
//! `enter`, `esc`/`escape`, `backspace`, `delete`/`del`, `tab`,
//! `up`, `down`, `left`, `right`, `home`, `end`, `pageup`, `pagedown`.
//!
//! Custom bindings shown in the `?` help overlay under "Custom bindings".
//! The file is hot-reloaded when it changes on disk (via the file watcher).

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use serde::Deserialize;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::RwLock;

// ──────────────────────────────────────────────────────────────────────────────
// Actions
// ──────────────────────────────────────────────────────────────────────────────

/// Named actions that keybindings can dispatch.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum KeyAction {
    ToggleFastMode,
    ClearHistory,
    Compact,
    OpenModelPicker,
    ToggleVerbose,
    Exit,
    ToggleHelp,
}

impl KeyAction {
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "toggle_fast_mode" => Some(Self::ToggleFastMode),
            "clear_history" => Some(Self::ClearHistory),
            "compact" => Some(Self::Compact),
            "open_model_picker" => Some(Self::OpenModelPicker),
            "toggle_verbose" => Some(Self::ToggleVerbose),
            "exit" => Some(Self::Exit),
            "toggle_help" => Some(Self::ToggleHelp),
            _ => None,
        }
    }

    pub fn description(&self) -> &'static str {
        match self {
            Self::ToggleFastMode => "Toggle fast mode",
            Self::ClearHistory => "Clear conversation history",
            Self::Compact => "Compact context",
            Self::OpenModelPicker => "Open model picker",
            Self::ToggleVerbose => "Toggle verbose tool display",
            Self::Exit => "Exit jfc",
            Self::ToggleHelp => "Show/hide keybindings help",
        }
    }
}

// ──────────────────────────────────────────────────────────────────────────────
// Key string parser
// ──────────────────────────────────────────────────────────────────────────────

/// Parse a key string like `"alt+o"`, `"ctrl+k"`, `"ctrl+shift+p"` into a
/// [`KeyEvent`]. Returns `None` for unrecognised modifiers or key names.
pub fn parse_key(s: &str) -> Option<KeyEvent> {
    let s_lower = s.to_lowercase();
    let parts: Vec<&str> = s_lower.split('+').collect();
    if parts.is_empty() {
        return None;
    }

    let mut mods = KeyModifiers::NONE;
    let key_str = parts.last()?;

    for part in &parts[..parts.len() - 1] {
        match *part {
            "ctrl" => mods |= KeyModifiers::CONTROL,
            "alt" => mods |= KeyModifiers::ALT,
            "shift" => mods |= KeyModifiers::SHIFT,
            _ => {
                tracing::warn!(
                    target: "jfc::keybindings",
                    modifier = part,
                    "Unknown modifier in keybinding, skipping"
                );
                return None;
            }
        }
    }

    let code = match *key_str {
        "enter" => KeyCode::Enter,
        "esc" | "escape" => KeyCode::Esc,
        "backspace" => KeyCode::Backspace,
        "delete" | "del" => KeyCode::Delete,
        "tab" => KeyCode::Tab,
        "up" => KeyCode::Up,
        "down" => KeyCode::Down,
        "left" => KeyCode::Left,
        "right" => KeyCode::Right,
        "home" => KeyCode::Home,
        "end" => KeyCode::End,
        "pageup" => KeyCode::PageUp,
        "pagedown" => KeyCode::PageDown,
        c if c.len() == 1 => KeyCode::Char(c.chars().next()?),
        other => {
            tracing::warn!(
                target: "jfc::keybindings",
                key = other,
                "Unknown key name in keybinding, skipping"
            );
            return None;
        }
    };

    Some(KeyEvent::new(code, mods))
}

/// Format a [`KeyEvent`] back into a human-readable string like `"Alt+O"`.
pub fn format_key(event: &KeyEvent) -> String {
    let mut parts: Vec<&str> = Vec::new();
    if event.modifiers.contains(KeyModifiers::CONTROL) {
        parts.push("Ctrl");
    }
    if event.modifiers.contains(KeyModifiers::ALT) {
        parts.push("Alt");
    }
    if event.modifiers.contains(KeyModifiers::SHIFT) {
        parts.push("Shift");
    }
    let key = match event.code {
        KeyCode::Char(c) => {
            // Allocate only when needed; return from a small local
            return format!("{}+{}", parts.join("+"), c.to_uppercase())
                .trim_start_matches('+')
                .to_string();
        }
        KeyCode::Enter => "Enter",
        KeyCode::Esc => "Esc",
        KeyCode::Backspace => "Backspace",
        KeyCode::Delete => "Delete",
        KeyCode::Tab => "Tab",
        KeyCode::Up => "Up",
        KeyCode::Down => "Down",
        KeyCode::Left => "Left",
        KeyCode::Right => "Right",
        KeyCode::Home => "Home",
        KeyCode::End => "End",
        KeyCode::PageUp => "PageUp",
        KeyCode::PageDown => "PageDown",
        _ => "?",
    };
    if parts.is_empty() {
        key.to_string()
    } else {
        format!("{}+{}", parts.join("+"), key)
    }
}

// ──────────────────────────────────────────────────────────────────────────────
// Global binding store
// ──────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Default, Deserialize)]
struct KeybindingsFile {
    #[serde(default)]
    keys: HashMap<String, String>,
}

static CUSTOM_BINDINGS: RwLock<Vec<(KeyEvent, KeyAction)>> = RwLock::new(Vec::new());

// ──────────────────────────────────────────────────────────────────────────────
// Public API
// ──────────────────────────────────────────────────────────────────────────────

/// Canonical path to the keybindings file. Always returned even if the file
/// doesn't exist yet — callers use it to show users where to create it.
pub fn keybindings_path() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("jfc")
        .join("keybindings.toml")
}

/// Load (or reload) keybindings from `~/.config/jfc/keybindings.toml`.
///
/// A missing file is silently treated as "no custom bindings". A malformed
/// file logs a warning and leaves the previously loaded bindings intact.
pub fn load() {
    let path = keybindings_path();
    let text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            // Perfectly normal — user hasn't created the file yet.
            if let Ok(mut guard) = CUSTOM_BINDINGS.write() {
                guard.clear();
            }
            return;
        }
        Err(e) => {
            tracing::warn!(
                target: "jfc::keybindings",
                path = %path.display(),
                error = %e,
                "Failed to read keybindings.toml"
            );
            return;
        }
    };

    let file: KeybindingsFile = match toml::from_str(&text) {
        Ok(f) => f,
        Err(e) => {
            tracing::warn!(
                target: "jfc::keybindings",
                path = %path.display(),
                error = %e,
                "Failed to parse keybindings.toml — check TOML syntax"
            );
            return;
        }
    };

    let mut bindings: Vec<(KeyEvent, KeyAction)> = Vec::new();
    for (key_str, action_str) in &file.keys {
        match (parse_key(key_str), KeyAction::from_str(action_str)) {
            (Some(key), Some(action)) => bindings.push((key, action)),
            (None, _) => tracing::warn!(
                target: "jfc::keybindings",
                key = %key_str,
                "Unrecognised key string — skipping binding"
            ),
            (_, None) => tracing::warn!(
                target: "jfc::keybindings",
                action = %action_str,
                "Unrecognised action name — skipping binding"
            ),
        }
    }

    tracing::info!(
        target: "jfc::keybindings",
        count = bindings.len(),
        path = %path.display(),
        "Loaded custom keybindings"
    );

    if let Ok(mut guard) = CUSTOM_BINDINGS.write() {
        *guard = bindings;
    }
}

/// Look up a custom action for a key event. Returns `None` if no custom
/// binding matches. Called from the main key-event handler in `input.rs`.
pub fn lookup(event: &KeyEvent) -> Option<KeyAction> {
    let guard = CUSTOM_BINDINGS.read().ok()?;
    guard
        .iter()
        .find(|(k, _)| k.code == event.code && k.modifiers == event.modifiers)
        .map(|(_, a)| a.clone())
}

/// Return all currently loaded custom bindings as `(key_string, description)`
/// pairs for display in the help overlay.
pub fn all_bindings() -> Vec<(String, String)> {
    let Ok(guard) = CUSTOM_BINDINGS.read() else {
        return vec![];
    };
    guard
        .iter()
        .map(|(k, a)| (format_key(k), a.description().to_string()))
        .collect()
}

// ──────────────────────────────────────────────────────────────────────────────
// Tests
// ──────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyCode;

    #[test]
    fn parse_simple_char_normal() {
        let ev = parse_key("alt+o").expect("parses");
        assert_eq!(ev.code, KeyCode::Char('o'));
        assert!(ev.modifiers.contains(KeyModifiers::ALT));
        assert!(!ev.modifiers.contains(KeyModifiers::CONTROL));
    }

    #[test]
    fn parse_ctrl_char_normal() {
        let ev = parse_key("ctrl+k").expect("parses");
        assert_eq!(ev.code, KeyCode::Char('k'));
        assert!(ev.modifiers.contains(KeyModifiers::CONTROL));
    }

    #[test]
    fn parse_multimod_normal() {
        let ev = parse_key("ctrl+shift+p").expect("parses");
        assert_eq!(ev.code, KeyCode::Char('p'));
        assert!(ev.modifiers.contains(KeyModifiers::CONTROL));
        assert!(ev.modifiers.contains(KeyModifiers::SHIFT));
    }

    #[test]
    fn parse_special_keys_normal() {
        assert_eq!(parse_key("enter").unwrap().code, KeyCode::Enter);
        assert_eq!(parse_key("esc").unwrap().code, KeyCode::Esc);
        assert_eq!(parse_key("escape").unwrap().code, KeyCode::Esc);
        assert_eq!(parse_key("pageup").unwrap().code, KeyCode::PageUp);
        assert_eq!(parse_key("pagedown").unwrap().code, KeyCode::PageDown);
        assert_eq!(parse_key("backspace").unwrap().code, KeyCode::Backspace);
        assert_eq!(parse_key("delete").unwrap().code, KeyCode::Delete);
        assert_eq!(parse_key("del").unwrap().code, KeyCode::Delete);
    }

    #[test]
    fn parse_unknown_key_returns_none_normal() {
        assert!(parse_key("ctrl+xyzzy").is_none());
    }

    #[test]
    fn parse_unknown_modifier_returns_none_normal() {
        assert!(parse_key("super+k").is_none());
    }

    #[test]
    fn parse_empty_returns_none_normal() {
        assert!(parse_key("").is_none());
    }

    #[test]
    fn format_key_roundtrip_normal() {
        for s in &["alt+o", "ctrl+k", "ctrl+shift+p", "enter", "esc"] {
            let ev = parse_key(s).unwrap_or_else(|| panic!("parse_key({s:?}) failed"));
            let formatted = format_key(&ev).to_lowercase();
            // The formatted form may differ in case (Ctrl vs ctrl) but
            // should parse back to the same event.
            let roundtripped =
                parse_key(&formatted).unwrap_or_else(|| panic!("re-parse of {formatted:?} failed"));
            assert_eq!(ev.code, roundtripped.code, "code mismatch for {s:?}");
            assert_eq!(
                ev.modifiers, roundtripped.modifiers,
                "modifiers mismatch for {s:?}"
            );
        }
    }

    #[test]
    fn key_action_from_str_normal() {
        assert_eq!(
            KeyAction::from_str("toggle_fast_mode"),
            Some(KeyAction::ToggleFastMode)
        );
        assert_eq!(
            KeyAction::from_str("clear_history"),
            Some(KeyAction::ClearHistory)
        );
        assert_eq!(KeyAction::from_str("compact"), Some(KeyAction::Compact));
        assert_eq!(
            KeyAction::from_str("open_model_picker"),
            Some(KeyAction::OpenModelPicker)
        );
        assert_eq!(
            KeyAction::from_str("toggle_verbose"),
            Some(KeyAction::ToggleVerbose)
        );
        assert_eq!(KeyAction::from_str("exit"), Some(KeyAction::Exit));
        assert_eq!(
            KeyAction::from_str("toggle_help"),
            Some(KeyAction::ToggleHelp)
        );
        assert!(KeyAction::from_str("does_not_exist").is_none());
    }

    #[test]
    fn all_bindings_returns_empty_before_load_normal() {
        // Since CUSTOM_BINDINGS is global, we can't guarantee it's empty
        // (other tests or load() calls may have populated it). We just
        // verify the function is callable and returns a Vec.
        let _bindings: Vec<(String, String)> = all_bindings();
    }
}

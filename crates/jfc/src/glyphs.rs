//! Centralized glyph vocabulary for the TUI.
//!
//! Historically the spinner frames, scrollbar chars, reasoning markers, and
//! the network-recovery symbol were scattered as string literals, which let
//! the vocabulary drift (two spinner sets defined independently, a hardcoded
//! `"!"`). This module is the single source of truth for the glyphs that are
//! genuinely *shared* across call sites.
//!
//! Note: deliberately NOT a dumping ground for every glyph. Context-specific
//! cycles — the moon/dice prompt-mode frames, the sparkline bar ramp
//! (`▁▂▃▄▅▆▇█`), the per-tool pending markers — stay local to their renderers
//! because the same character (e.g. `◐`) means different things in each, and
//! hoisting them here would conflate distinct uses.

// ── Reasoning / thinking block ───────────────────────────────────────────
/// Header marker for the extended-thinking block (`∴ Thinking`).
pub const REASONING_HEADER: &str = "∴";
/// Per-line left rail for expanded reasoning content.
pub const REASONING_RIBBON: &str = "┃";

// ── Scrollbar ────────────────────────────────────────────────────────────
pub const SCROLLBAR_BEGIN: &str = "▲";
pub const SCROLLBAR_END: &str = "▼";
pub const SCROLLBAR_THUMB: &str = "█";
pub const SCROLLBAR_TRACK: &str = "│";

// ── Spinner frames ───────────────────────────────────────────────────────
/// The streaming status row's spinner (the `✦`-style star cycle), used by the
/// honest status line in `spinner.rs`.
pub const STATUS_FRAMES: &[&str] = &["·", "✢", "✦", "✶", "✻", "✽"];
/// The braille spinner used by the task/subagent panels. Intentionally distinct
/// from [`STATUS_FRAMES`]: the panels show many small concurrent spinners where
/// the braille cycle reads better than the larger stars. Both live here so the
/// distinction is explicit rather than accidental.
pub const TASK_FRAMES: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

/// Network-recovery marker shown in place of the spinner glyph while a provider
/// retries a transient failure (was a bare `"!"` literal in `messages.rs`).
pub const RECOVERY: &str = "!";

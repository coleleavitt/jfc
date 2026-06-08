//! Codex streaming-render toolkit — ported from `openai/codex`
//! (`codex-rs/tui/src/`).
//!
//! These are the **protocol-free** rendering/pacing utilities lifted from
//! Codex's TUI and grafted onto JFC. They depend only on `std` + `ratatui` +
//! text crates — never on Codex's `app-server-protocol` (which is what makes a
//! wholesale Codex-TUI adoption infeasible). This is stage 1 of the selective
//! TUI port: each piece lands as a self-contained, unit-tested toolkit and is
//! wired into JFC's `EngineEvent` stream in a later stage.
//!
//! Provenance + porting notes are recorded per file. Codex builds against a
//! `ratatui` 0.29 git fork + `pulldown-cmark` 0.10; JFC is on `ratatui` 0.30 +
//! `pulldown-cmark` 0.12, so the `ratatui`/markdown-touching files are adapted
//! (not verbatim). `chunking` is pure `std::time` and ports verbatim.

// Stage 1: each piece lands standalone; the policies/widgets are driven by the
// streaming controller + composer in later stages. Allow dead_code until that
// wiring lands so the not-yet-consumed public surface doesn't warn-spam.

/// Adaptive stream-pacing policy (smooth vs catch-up). Pure `std::time`; verbatim.
#[allow(dead_code)]
pub(crate) mod chunking;

/// View-layer reveal pacer: animates display of the engine's (already-immediate)
/// streaming text at the adaptive cadence, driving [`chunking`] from the render
/// frame. JFC-authored adapter over the ported policy — no engine changes.
#[allow(dead_code)]
pub(crate) mod stream_pacer;

/// Markdown table-boundary detector (fences, headers, delimiter rows). Pure; verbatim.
#[allow(dead_code)]
pub(crate) mod table_detect;

/// Non-bracketed-paste burst detector for terminals without bracketed paste.
/// Pure `std::time`; verbatim.
#[allow(dead_code)]
pub(crate) mod paste_burst;

/// Cross-terminal key-binding matcher (canonicalizes raw C0 / unshifted-upper).
/// `crossterm` only; verbatim.
#[allow(dead_code)]
pub(crate) mod key_hint;

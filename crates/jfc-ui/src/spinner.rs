//! v126-style "Fermenting…" spinner state.
//!
//! Mirrors the architecture from `cli.js` lines 233823-234022 (verb list)
//! and 322930-323289 (frame cycle, stall thresholds, "almost done thinking"
//! trigger). Pure formatting — the renderer reads `App` fields and calls
//! into here; no mutation, no I/O.
//!
//! ## Pieces
//!
//! - **Frames**: 6-char cycle `[· ✢ * ✶ ✻ ✽]` (matches v126's default
//!   spinner; ghostty variant unused here for simplicity).
//! - **Verbs**: a 32-entry subset of v126's 177-verb list, picked by hash
//!   of `frame_seed` so the verb is **stable for runs of the same seed**.
//!   v126 randomizes per-render which feels chaotic in a TUI; we rotate
//!   every ~2s instead by deriving the seed from `(elapsed.as_secs() / 2)`.
//! - **Sub-status**: `almost done thinking` appears when **>=60s** has
//!   passed since the last token (matches v126 `VW_=60s`, line 323283).
//!
//! ## Format (one line)
//!
//! ```text
//! * Fermenting… (5m 10s · ↓ 14.6k tokens · almost done thinking)
//! ```

use std::{sync::OnceLock, time::Duration};

/// Pre-computed status row split into the four logical pieces the
/// renderer needs to color independently. The shimmer animation only
/// applies to the verb segment, so we hand it back separately rather
/// than baking the full string and asking the renderer to re-parse it.
///
/// Mirrors v126's `<GlimmerMessage>` decomposition (cli.js around
/// 322930 — Spinner/GlimmerMessage.tsx) where the message text is
/// re-rendered per-grapheme so a sweep can light up ±1 cells.
pub struct StatusSegments {
    /// Spinner glyph (e.g. `*`) — accent color, no shimmer.
    pub glyph: &'static str,
    /// Verb root (e.g. `Fermenting`) — accent base, shimmer overlay.
    pub verb: &'static str,
    /// Trailing parenthesised body (e.g. `(5m 10s · ↓ 14.6k tokens)`).
    /// Rendered in muted color, no shimmer — it's metadata, not the
    /// active label, so animating it would compete with the verb for
    /// the user's eye.
    pub body: String,
}

/// Whether all UI animations should flatten to static colors. Honored by
/// every shimmer/pulse/sweep helper in this module so a single
/// `JFC_REDUCED_MOTION=1` flips the whole UI to a still image. Mirrors
/// v126's `reducedMotion` prop threaded through every animated
/// component (cli.js around `useReducedMotion`). The env var is cached
/// because render code calls this on every frame and a running process's
/// environment is not a live configuration channel.
pub fn reduced_motion() -> bool {
    static REDUCED_MOTION: OnceLock<bool> = OnceLock::new();
    *REDUCED_MOTION.get_or_init(|| {
        matches!(
            std::env::var("JFC_REDUCED_MOTION").as_deref(),
            Ok("1") | Ok("true") | Ok("yes")
        )
    })
}

/// Linear-interpolate between two RGB triples with `t ∈ [0, 1]`.
/// Mirrors v126's `interpolateColor` (Spinner/utils.ts:14). Used by
/// the shimmer pass to blend the verb base color toward the accent
/// at the cells covered by the glimmer index, so the sweep reads as
/// a smooth highlight rather than a hard color flip.
pub fn interpolate_rgb(c1: (u8, u8, u8), c2: (u8, u8, u8), t: f32) -> (u8, u8, u8) {
    let t = t.clamp(0.0, 1.0);
    let lerp = |a: u8, b: u8| -> u8 {
        let af = a as f32;
        let bf = b as f32;
        (af + (bf - af) * t).round().clamp(0.0, 255.0) as u8
    };
    (lerp(c1.0, c2.0), lerp(c1.1, c2.1), lerp(c1.2, c2.2))
}

/// HSL hue (0..360) → RGB. Mirrors v126's `hueToRgb` in
/// Spinner/utils.ts:32. Used for rainbow gradient text where each
/// char gets `hueToRgb((phase + i * step) % 360)` so the colors sweep
/// along the text on each animation frame. Saturation 0.7 / lightness
/// 0.6 match v126's voice-mode waveform parameters — bright enough
/// to read as colorful, muted enough not to clash with surrounding
/// muted prose.
pub fn hue_to_rgb(hue: f32) -> (u8, u8, u8) {
    let h = ((hue % 360.0) + 360.0) % 360.0;
    let s = 0.7_f32;
    let l = 0.6_f32;
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let x = c * (1.0 - (((h / 60.0) % 2.0) - 1.0).abs());
    let m = l - c / 2.0;
    let (r, g, b) = if h < 60.0 {
        (c, x, 0.0)
    } else if h < 120.0 {
        (x, c, 0.0)
    } else if h < 180.0 {
        (0.0, c, x)
    } else if h < 240.0 {
        (0.0, x, c)
    } else if h < 300.0 {
        (x, 0.0, c)
    } else {
        (c, 0.0, x)
    };
    (
        ((r + m) * 255.0).round().clamp(0.0, 255.0) as u8,
        ((g + m) * 255.0).round().clamp(0.0, 255.0) as u8,
        ((b + m) * 255.0).round().clamp(0.0, 255.0) as u8,
    )
}

/// Compute the current glimmer-sweep index for a verb of `verb_width`
/// cells. The index sweeps from `-10` to `verb_width + 10` over time
/// — chars within ±1 of the index get the shimmer color, everything
/// else stays at the base. The 10-cell pre/post overshoot is what
/// makes the highlight slide *into* and *out of* the verb cleanly
/// instead of teleporting at the edges.
///
/// `tick_ms` controls cycle speed. v126 uses 50ms during `requesting`
/// (faster, more attention-grabbing) and 200ms during `tool-use`
/// (calmer pulse). We mirror that — the renderer picks 50ms while
/// streaming, 200ms while idle.
pub fn glimmer_index(elapsed: Duration, verb_width: usize, tick_ms: u64) -> i32 {
    if verb_width == 0 || tick_ms == 0 {
        return -100;
    }
    let cycle_position = (elapsed.as_millis() / tick_ms as u128) as i64;
    let cycle_length = verb_width as i64 + 20;
    let pos = (cycle_position % cycle_length) - 10;
    pos as i32
}

/// 6-frame spinner cycle. Matches v126's `nAH()` default (cli.js:170248).
pub const FRAMES: &[&str] = &["·", "✢", "*", "✶", "✻", "✽"];

/// Present-tense verb pool — expanded toward cli.js v143's `iD6` array
/// (~100 entries). Cycled in 5-second buckets so the same word stays on
/// screen long enough to read but the spinner doesn't feel stuck.
pub const VERBS: &[&str] = &[
    "Fermenting",
    "Pondering",
    "Cooking",
    "Brewing",
    "Mulling",
    "Simmering",
    "Crafting",
    "Forging",
    "Untangling",
    "Synthesizing",
    "Spelunking",
    "Wrangling",
    "Distilling",
    "Marinating",
    "Whittling",
    "Plotting",
    "Computing",
    "Sketching",
    "Excavating",
    "Tinkering",
    "Sculpting",
    "Surfacing",
    "Threading",
    "Weaving",
    "Polishing",
    "Composing",
    "Architecting",
    "Calibrating",
    "Mapping",
    "Auditing",
    "Reasoning",
    "Investigating",
    "Accomplishing",
    "Brainstorming",
    "Cogitating",
    "Computing",
    "Conjuring",
    "Constructing",
    "Crunching",
    "Deliberating",
    "Designing",
    "Divining",
    "Drafting",
    "Dreaming",
    "Engineering",
    "Envisioning",
    "Exploring",
    "Extrapolating",
    "Fashioning",
    "Figuring",
    "Finessing",
    "Formulating",
    "Generating",
    "Germinating",
    "Hatching",
    "Hypothesizing",
    "Iterating",
    "Kneading",
    "Loading",
    "Manifesting",
    "Moonwalking",
    "Noodling",
    "Optimizing",
    "Orchestrating",
    "Percolating",
    "Piecing",
    "Planning",
    "Processing",
    "Prototyping",
    "Puzzling",
    "Quilting",
    "Refining",
    "Researching",
    "Resolving",
    "Sautéing",
    "Scheming",
    "Spinning",
    "Strategizing",
    "Structuring",
    "Studying",
    "Stitching",
    "Steeping",
    "Surveying",
    "Tessellating",
    "Tracing",
    "Translating",
    "Unraveling",
    "Working",
];

/// Past-tense verbs for finished agents. Mirrors cli.js v143's `rD6` array
/// so completed sub-agent rows in the fan read with a finished tone ("Baked
/// for 1m 5s") instead of stale present-tense ("Fermenting").
pub const VERBS_PAST: &[&str] = &[
    "Baked",
    "Brewed",
    "Churned",
    "Cogitated",
    "Cooked",
    "Crunched",
    "Sautéed",
    "Simmered",
    "Worked",
    "Wrought",
];

/// Picks a frame index from a tick counter. Caller is expected to bump the
/// tick on every redraw — typically every 80ms (one `UiEvent::Tick`).
pub fn frame_for(tick: usize) -> &'static str {
    FRAMES[tick % FRAMES.len()]
}

/// Picks a verb based on the elapsed time so the verb stays stable for
/// ~5-second windows (less jittery than per-frame randomization, and
/// long enough that a glance at the spinner finds the same word it
/// did a moment ago — 2s was too jumpy in practice).
pub fn verb_for(elapsed: Duration) -> &'static str {
    let bucket = (elapsed.as_secs() / 5) as usize;
    VERBS[bucket % VERBS.len()]
}

/// Pick a past-tense verb deterministically from a task-id-ish seed.
/// Completed agents in the fan row read "Baked for 1m 5s" instead of a
/// stale present-tense "Fermenting" — matches cli.js v143's `rD6`/`XgH()`
/// pair where each task captures its own past verb at completion.
pub fn verb_past_for(seed: &str) -> &'static str {
    let h: usize = seed.bytes().map(|b| b as usize).sum();
    VERBS_PAST[h % VERBS_PAST.len()]
}

/// Format an elapsed Duration as `XmYs` or `Xs`. Mirrors v126 `h4()`
/// (line 323177). Sub-second always shows as `0s` so the line doesn't
/// flicker between `0s` and missing.
pub fn fmt_elapsed(elapsed: Duration) -> String {
    let secs = elapsed.as_secs();
    if secs >= 60 {
        let m = secs / 60;
        let s = secs % 60;
        format!("{m}m {s}s")
    } else {
        format!("{secs}s")
    }
}

/// Format a token count compactly: `1.4k`, `15k`, `234`. v126 uses `I4()`
/// which clamps to 1 decimal for k/M. Below 1000 we show the raw count
/// since exact small numbers are useful (e.g. `42 tokens` mid-stream).
pub fn fmt_tokens(n: u64) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else if n >= 10_000 {
        format!("{}k", n / 1000)
    } else if n >= 1_000 {
        format!("{:.1}k", n as f64 / 1000.0)
    } else {
        n.to_string()
    }
}

/// Sub-status string when the stream has been quiet a while. v126 has 4
/// thresholds (`ZW_=15s`, `TW_=30s`, `vW_=45s`, `VW_=60s`). At the longest
/// it appends "almost done thinking" — the user-facing reassurance.
pub fn stall_status(time_since_last_token: Duration) -> Option<&'static str> {
    let s = time_since_last_token.as_secs();
    if s >= 60 {
        Some("almost done thinking")
    } else if s >= 45 {
        Some("still thinking")
    } else if s >= 30 {
        Some("thinking")
    } else if s >= 15 {
        Some("warming up")
    } else {
        None
    }
}

/// Past-tense verbs for the `Cooked for Nm Ns` post-turn marker. Sourced
/// from v126 cli.js:233999-234008; falls back to "Worked" if the bucket
/// math overflows. Same 2-second-window rotation as the live verb so the
/// label stays stable for a glance even though the time-bucket changes
/// across the duration.
pub const COOKED_VERBS: &[&str] = &[
    "Baked",
    "Brewed",
    "Churned",
    "Cogitated",
    "Cooked",
    "Crunched",
    "Sautéed",
    "Worked",
];

/// Tips rotated under the spinner when no task is open. Mirrors v126
/// cli.js:323851 (`Tip: ${WH}` fallback when `m` task is None) — gives
/// the user something to read while waiting and surfaces less-obvious
/// keybindings. Picked deterministically by elapsed-bucket so the tip
/// is stable for ~10s windows.
pub const TIPS: &[&str] = &[
    "Press Esc to dismiss popups",
    "Ctrl+B opens the sessions sidebar",
    "Ctrl+P opens the command palette",
    "Ctrl+M switches model",
    "Ctrl+T opens the task panel",
    "Ctrl+Y yanks the last assistant message",
    "Type @ to autocomplete file paths",
    "/compact summarizes long conversations",
    "/check re-runs cargo diagnostics",
    "/auto-mode on enables the LLM tool classifier",
];

/// Same as `tip_for_with_state(elapsed, false)` but skips popup-related tips when no popup is
/// open. The "Press Esc to dismiss popups" tip used to surface even
/// when nothing was dismissable, which read as a fake instruction —
/// the user would scan the screen for a popup that didn't exist.
pub fn tip_for_with_state(elapsed: Duration, any_popup_open: bool) -> &'static str {
    // Build the visible tip set on demand. Tips containing "Esc"
    // (popup-dismissal hints) are filtered out when no popup is open.
    let visible: Vec<&'static str> = TIPS
        .iter()
        .copied()
        .filter(|t| any_popup_open || !t.contains("Esc"))
        .collect();
    if visible.is_empty() {
        return TIPS[0];
    }
    let bucket = (elapsed.as_secs() / 10) as usize;
    visible[bucket % visible.len()]
}

/// Pick a past-tense verb for the post-turn duration footer. Mirrors v126
/// cli.js:341376 (`${A} for ${w}` where `A = Av_() = zJ(hpH) ?? "Worked"`).
/// Bucketed by 2-second windows of the elapsed duration so different
/// turns get different verbs but a single turn's display is stable.
pub fn cooked_verb_for(elapsed: Duration) -> &'static str {
    let bucket = (elapsed.as_secs() / 2) as usize;
    COOKED_VERBS[bucket % COOKED_VERBS.len()]
}

/// Format the post-turn marker shown under each completed assistant
/// message. v126 cli.js:341376 — `<verb> for <duration>`.
pub fn format_finished(elapsed: Duration) -> String {
    format!("{} for {}", cooked_verb_for(elapsed), fmt_elapsed(elapsed))
}

/// Live token count for the spinner: the **maximum** of the wire-truth
/// cumulative `output_tokens` and the chars-divided-by-4 estimate.
///
/// ## Why max, not "prefer wire"
///
/// Anthropic's `message_delta` events arrive in *batches* — typically one
/// every few hundred ms with the count of all output tokens since the
/// stream began. The chars/4 estimate, by contrast, updates on every
/// SSE byte. If the spinner just preferred wire whenever non-zero, the
/// counter would freeze at the last delta value (e.g. 7) for hundreds
/// of ms and then jump (e.g. to 200) when the next delta arrived —
/// what the user reported.
///
/// By taking the max, the counter advances fluidly with every chunk
/// (estimate side) AND corrects upward when wire-truth catches up.
/// Importantly: max is **monotonic** — the counter never moves
/// backward, which would read as a bug to the user. (chars/4 tends to
/// over-count whitespace + code fences slightly so it usually leads
/// wire; the max picks whichever is larger at any instant.)
pub fn live_token_count(wire_output: u64, char_estimate: u64) -> u64 {
    wire_output.max(char_estimate)
}

/// Live-vs-finished thinking signal for `format_status`. Mirrors v126's
/// `thinkingStatus` prop on the spinner component (cli.js:323189): the
/// model is either *currently* producing reasoning, *has finished*
/// reasoning (and we know the duration), or hasn't reasoned this turn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThinkingStatus {
    /// Reasoning chunks are arriving — show `thinking…`.
    Live,
    /// Reasoning ended; first text byte has arrived. Display
    /// `thought for Ns` instead of the live verb.
    Done(Duration),
}

/// Compose the full status line shown above the input bar:
/// `"* Fermenting… (5m 10s · ↓ 14.6k tokens · almost done thinking)"`.
///
/// Returns just the *content* — the renderer wraps it in styled spans.
///
/// `thinking` overrides `time_since_last_token`'s stall messages while
/// the model is actively thinking, and shows a `thought for Ns` chip
/// once thinking has ended.
/// Decomposed form of `format_status` — same inputs, but returns the
/// glyph / verb / parens-body separately so the renderer can shimmer
/// just the verb without re-parsing a packed string.
pub fn status_segments(
    tick: usize,
    elapsed: Duration,
    output_tokens: u64,
    time_since_last_token: Duration,
    thinking: Option<ThinkingStatus>,
) -> StatusSegments {
    let mut parts: Vec<String> = vec![fmt_elapsed(elapsed)];
    if output_tokens > 0 {
        parts.push(format!("↓ {} tokens", fmt_tokens(output_tokens)));
        let elapsed_secs = elapsed.as_secs_f64();
        if elapsed_secs >= 2.0 {
            let rate = output_tokens as f64 / elapsed_secs;
            if rate > 0.5 {
                parts.push(format!("{:.0} tok/s", rate));
            }
        }
    }
    match thinking {
        Some(ThinkingStatus::Live) => {
            // Reuse the main spinner's star cycle (`· ✢ * ✶ ✻ ✽`) for
            // the live-thinking glyph too instead of the braille
            // dots. The user said the star pulse is what reads as
            // "Claude" to them; mixing in braille split the visual
            // language. Single source of truth, single glyph family.
            let glyph = frame_for(tick + 3);
            let phase = match elapsed.as_secs() {
                0..=11 => "planning",
                12..=29 => "considering",
                _ => "drafting",
            };
            parts.push(format!("thinking {glyph} {phase}"));
        }
        Some(ThinkingStatus::Done(d)) => {
            let secs = d.as_secs().max(1);
            parts.push(format!("thought for {secs}s"));
            if let Some(s) = stall_status(time_since_last_token) {
                parts.push(s.to_string());
            }
        }
        None => {
            if let Some(s) = stall_status(time_since_last_token) {
                parts.push(s.to_string());
            }
        }
    }
    StatusSegments {
        glyph: frame_for(tick),
        verb: verb_for(elapsed),
        body: format!("({})", parts.join(" · ")),
    }
}

pub fn format_status(
    tick: usize,
    elapsed: Duration,
    output_tokens: u64,
    time_since_last_token: Duration,
    thinking: Option<ThinkingStatus>,
) -> String {
    let mut parts: Vec<String> = vec![fmt_elapsed(elapsed)];
    if output_tokens > 0 {
        parts.push(format!("↓ {} tokens", fmt_tokens(output_tokens)));
        // Live token rate. Skipping the first 2 seconds because the
        // initial burst is dominated by start-up latency and gives a
        // misleading rate. Once we have meaningful data, show the
        // rolling estimate so the user can spot when the stream
        // actually slowed down vs. just feels slow.
        let elapsed_secs = elapsed.as_secs_f64();
        if elapsed_secs >= 2.0 {
            let rate = output_tokens as f64 / elapsed_secs;
            if rate > 0.5 {
                parts.push(format!("{:.0} tok/s", rate));
            }
        }
    }
    // Thinking signal beats stall_status while live (mid-reasoning the
    // wire is silent for tens of seconds and the user would otherwise
    // see "almost done thinking" the whole time). Once thinking ended,
    // show the duration chip; *also* layer stall_status on top of that
    // when the post-thinking text stream goes quiet for >=15s.
    match thinking {
        Some(ThinkingStatus::Live) => {
            // Reuse the star cycle (`· ✢ * ✶ ✻ ✽`) here too — keeps
            // the live-thinking row in the same visual language as
            // the main spinner glyph instead of mixing in braille
            // dots that read as a separate animation system.
            let glyph = frame_for(tick + 3);
            let phase = match elapsed.as_secs() {
                0..=11 => "planning",
                12..=29 => "considering",
                _ => "drafting",
            };
            parts.push(format!("thinking {glyph} {phase}"));
        }
        Some(ThinkingStatus::Done(d)) => {
            let secs = d.as_secs().max(1);
            parts.push(format!("thought for {secs}s"));
            if let Some(s) = stall_status(time_since_last_token) {
                parts.push(s.to_string());
            }
        }
        None => {
            if let Some(s) = stall_status(time_since_last_token) {
                parts.push(s.to_string());
            }
        }
    }
    format!(
        "{} {}… ({})",
        frame_for(tick),
        verb_for(elapsed),
        parts.join(" · ")
    )
}

/// Compact-mode spinner body. Mirrors v126's `setStreamMode("compacting")`
/// UI: braille spinner + verb + elapsed + magnitude + live output.
///
/// - `pre_tokens` — pre-compact context size, shown as the input
///   magnitude (`412k tokens`).
/// - `output_chars` — cumulative summary text length collected so far
///   from the streaming compact response. Divided by 4 to estimate
///   tokens (same chars/4 heuristic as the regular streaming spinner).
///   Mirrors v126's `addResponseLength` callback in PB7
///   (cli.js:396989) — fires on every text_delta during summarization.
///
/// Without the live output piece, a 1m+ compact looks like a frozen
/// UI even though the API is happily streaming summary text.
pub fn format_compact_status(
    tick: usize,
    elapsed: Duration,
    pre_tokens: u64,
    output_chars: u64,
) -> String {
    let mut parts: Vec<String> = vec![fmt_elapsed(elapsed)];
    if pre_tokens > 0 {
        parts.push(format!("{} tokens", fmt_tokens(pre_tokens)));
    }
    if output_chars > 0 {
        let out_tokens = output_chars / 4;
        parts.push(format!("↓ {} tokens", fmt_tokens(out_tokens)));
    }
    format!("{} Compacting… ({})", frame_for(tick), parts.join(" · "))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn elapsed_format_under_60s_normal() {
        assert_eq!(fmt_elapsed(Duration::from_secs(0)), "0s");
        assert_eq!(fmt_elapsed(Duration::from_secs(7)), "7s");
        assert_eq!(fmt_elapsed(Duration::from_secs(59)), "59s");
    }

    #[test]
    fn elapsed_format_minutes_normal() {
        assert_eq!(fmt_elapsed(Duration::from_secs(60)), "1m 0s");
        assert_eq!(fmt_elapsed(Duration::from_secs(310)), "5m 10s");
        assert_eq!(fmt_elapsed(Duration::from_secs(3661)), "61m 1s");
    }

    #[test]
    fn token_format_thresholds_normal() {
        assert_eq!(fmt_tokens(0), "0");
        assert_eq!(fmt_tokens(42), "42");
        assert_eq!(fmt_tokens(999), "999");
        assert_eq!(fmt_tokens(1_000), "1.0k");
        assert_eq!(fmt_tokens(1_456), "1.5k");
        assert_eq!(fmt_tokens(14_600), "14k");
        assert_eq!(fmt_tokens(2_000_000), "2.0M");
    }

    #[test]
    fn stall_status_thresholds_match_v126_normal() {
        assert_eq!(stall_status(Duration::from_secs(0)), None);
        assert_eq!(stall_status(Duration::from_secs(14)), None);
        assert_eq!(stall_status(Duration::from_secs(15)), Some("warming up"));
        assert_eq!(stall_status(Duration::from_secs(29)), Some("warming up"));
        assert_eq!(stall_status(Duration::from_secs(30)), Some("thinking"));
        assert_eq!(stall_status(Duration::from_secs(44)), Some("thinking"));
        assert_eq!(
            stall_status(Duration::from_secs(45)),
            Some("still thinking")
        );
        assert_eq!(
            stall_status(Duration::from_secs(59)),
            Some("still thinking")
        );
        assert_eq!(
            stall_status(Duration::from_secs(60)),
            Some("almost done thinking")
        );
        assert_eq!(
            stall_status(Duration::from_secs(600)),
            Some("almost done thinking")
        );
    }

    #[test]
    fn frame_cycle_wraps_robust() {
        assert_eq!(frame_for(0), "·");
        assert_eq!(frame_for(1), "✢");
        assert_eq!(frame_for(5), "✽");
        assert_eq!(frame_for(6), "·"); // wraps
        assert_eq!(frame_for(usize::MAX), FRAMES[usize::MAX % FRAMES.len()]);
    }

    #[test]
    fn verb_changes_every_five_seconds_robust() {
        // Bucket widened from 2s → 5s after the 2s cadence felt jumpy
        // in practice (the user's complaint: spinner verb out of sync
        // with the per-second elapsed clock). 5s windows let a glance
        // back at the spinner find the same word it did a moment ago.
        let v0 = verb_for(Duration::from_secs(0));
        let v4 = verb_for(Duration::from_secs(4));
        let v5 = verb_for(Duration::from_secs(5));
        assert_eq!(v0, v4, "verb stable within a 5s window");
        assert_ne!(v0, v5, "verb advances at the 5s boundary");
    }

    #[test]
    fn format_status_includes_all_pieces_normal() {
        let s = format_status(
            2,
            Duration::from_secs(310),
            14_600,
            Duration::from_secs(70),
            None,
        );
        assert!(s.contains("…"), "verb ellipsis missing: {s}");
        assert!(s.contains("5m 10s"), "elapsed missing: {s}");
        assert!(s.contains("14k tokens"), "token line missing: {s}");
        assert!(
            s.contains("almost done thinking"),
            "stall hint missing: {s}"
        );
    }

    #[test]
    fn format_status_omits_tokens_when_zero_robust() {
        let s = format_status(0, Duration::from_secs(3), 0, Duration::from_secs(0), None);
        assert!(
            !s.contains("tokens"),
            "should hide token suffix when 0: {s}"
        );
        assert!(s.contains("3s"));
    }

    #[test]
    fn format_status_omits_stall_when_fresh_robust() {
        let s = format_status(0, Duration::from_secs(5), 100, Duration::from_secs(2), None);
        assert!(
            !s.contains("thinking"),
            "fresh stream shouldn't say 'thinking': {s}"
        );
    }

    // Live thinking: spinner shows `thinking` instead of stall messages.
    #[test]
    fn format_status_live_thinking_shows_thinking_normal() {
        let s = format_status(
            0,
            Duration::from_secs(20),
            500,
            Duration::from_secs(20),
            Some(ThinkingStatus::Live),
        );
        assert!(s.contains("thinking"), "expected live thinking: {s}");
        // While live, we suppress stall messages so a 20s gap doesn't
        // double-display "warming up · thinking".
        assert!(
            !s.contains("warming up"),
            "live thinking should hide stall: {s}"
        );
    }

    // Done thinking: spinner shows `thought for Ns`. Mirrors v126's
    // `thought for ${Math.max(1, Math.round(G / 1e3))}s` formatter.
    #[test]
    fn format_status_done_thinking_shows_duration_normal() {
        let s = format_status(
            0,
            Duration::from_secs(60),
            5_000,
            Duration::from_secs(0),
            Some(ThinkingStatus::Done(Duration::from_secs(12))),
        );
        assert!(s.contains("thought for 12s"), "expected duration: {s}");
    }

    // Sub-second thinking still renders as `thought for 1s` (v126 floors
    // to 1).
    #[test]
    fn format_status_done_thinking_floors_to_one_second_robust() {
        let s = format_status(
            0,
            Duration::from_secs(5),
            100,
            Duration::from_secs(0),
            Some(ThinkingStatus::Done(Duration::from_millis(400))),
        );
        assert!(s.contains("thought for 1s"), "expected 1s floor: {s}");
    }

    // Compact spinner shows the verb, elapsed, AND pre-compact token
    // magnitude — without this last piece a 60s compact looks frozen.
    #[test]
    fn format_compact_status_includes_pre_tokens_normal() {
        let s = format_compact_status(0, Duration::from_secs(8), 412_000, 0);
        assert!(s.contains("Compacting"), "verb missing: {s}");
        assert!(s.contains("8s"), "elapsed missing: {s}");
        assert!(s.contains("412k tokens"), "pre-token chip missing: {s}");
    }

    // When pre_tokens is 0 (e.g. a brand-new session compacting trivial
    // content, or the renderer hasn't recomputed yet) drop the chip
    // rather than showing a useless `0 tokens`.
    #[test]
    fn format_compact_status_omits_chip_when_pre_zero_robust() {
        let s = format_compact_status(0, Duration::from_secs(2), 0, 0);
        assert!(s.contains("Compacting"), "verb missing: {s}");
        assert!(s.contains("2s"), "elapsed missing: {s}");
        assert!(!s.contains("0 tokens"), "shouldn't show 0-token chip: {s}");
    }

    // Live output during compact streaming: `output_chars` divided by 4
    // gives the token estimate (matches the regular streaming spinner's
    // chars/4 fallback). Mirrors v126's PB7 addResponseLength feed.
    #[test]
    fn format_compact_status_shows_live_output_tokens_normal() {
        // 4_800 chars ≈ 1.2k tokens (4_800 / 4 = 1200).
        let s = format_compact_status(0, Duration::from_secs(15), 412_000, 4_800);
        assert!(s.contains("Compacting"), "verb missing: {s}");
        assert!(s.contains("412k tokens"), "pre-token chip missing: {s}");
        assert!(s.contains("↓"), "down-arrow missing: {s}");
        assert!(s.contains("1.2k tokens"), "output token chip missing: {s}");
    }

    // Robust: 0 output_chars (just started, no chunks yet) drops the
    // ↓ chip — same shape as the regular spinner before any token
    // arrives.
    #[test]
    fn format_compact_status_omits_output_chip_when_zero_robust() {
        let s = format_compact_status(0, Duration::from_secs(3), 100_000, 0);
        assert!(!s.contains("↓"), "shouldn't show empty output chip: {s}");
    }

    #[test]
    fn live_token_count_takes_max_normal() {
        // Behavior changed from "prefer wire" → "take max". Reason:
        // wire-truth arrives in batches (one `message_delta` every few
        // hundred ms); the estimate updates per-byte. With prefer-wire,
        // the counter froze between deltas and jumped on each one (the
        // user-reported "jumps from 7 to 200"). Max keeps the counter
        // fluid AND monotonic.
        assert_eq!(
            live_token_count(150, 200),
            200,
            "estimate higher → estimate wins"
        );
        assert_eq!(live_token_count(200, 150), 200, "wire higher → wire wins");
        assert_eq!(live_token_count(0, 200), 200, "no wire yet → estimate");
        assert_eq!(live_token_count(200, 0), 200, "no estimate → wire");
    }

    #[test]
    fn live_token_count_zero_when_both_zero_robust() {
        // Pre-stream / first-frame state — nothing to show, no panic.
        assert_eq!(live_token_count(0, 0), 0);
    }

    #[test]
    fn live_token_count_monotonic_across_arrivals_robust() {
        // Simulate a typical stream: chunks come in, then a delayed
        // wire delta arrives at a *lower* value than the current
        // estimate (the over-counting pattern). Counter must never
        // visibly drop.
        let mut last = 0u64;
        for (wire, est) in [(0, 5), (0, 50), (0, 100), (40, 100), (40, 150), (200, 150)] {
            let n = live_token_count(wire, est);
            assert!(
                n >= last,
                "count went backward: prev={last} now={n} (wire={wire} est={est})"
            );
            last = n;
        }
    }

    #[test]
    fn cooked_verb_in_pool_normal() {
        // Whatever verb comes back must be from the v126 pool — bucket
        // wraparound math should never produce a string outside the array.
        for secs in [0u64, 1, 2, 7, 60, 600, 3600, 86_400] {
            let v = cooked_verb_for(Duration::from_secs(secs));
            assert!(
                COOKED_VERBS.contains(&v),
                "elapsed={secs}s → {v:?} not in COOKED_VERBS"
            );
        }
    }

    #[test]
    fn cooked_verb_changes_across_buckets_normal() {
        // Different turn durations should sometimes pick different verbs
        // — otherwise every assistant turn would always say "Cooked".
        let mut seen = std::collections::HashSet::new();
        for secs in 0..32 {
            seen.insert(cooked_verb_for(Duration::from_secs(secs)));
        }
        assert!(
            seen.len() >= 4,
            "expected at least 4 distinct verbs across 32s window, got {seen:?}"
        );
    }

    #[test]
    fn cooked_verb_stable_within_bucket_robust() {
        // The 2-second bucket from cli.js means `0s` and `1s` should pick
        // the same verb (display stability for the brief moment between
        // 0s and 1s of elapsed when the message just resolved).
        assert_eq!(
            cooked_verb_for(Duration::from_secs(0)),
            cooked_verb_for(Duration::from_secs(1))
        );
    }

    #[test]
    fn format_finished_matches_v126_layout_normal() {
        // v126 cli.js:341376: `${A} for ${w}` where w is "Xm Ys" or "Ns".
        let s = format_finished(Duration::from_secs(310));
        assert!(s.contains(" for 5m 10s"), "got: {s}");
        assert!(
            COOKED_VERBS.iter().any(|v| s.starts_with(v)),
            "must start with a verb from the pool; got: {s}"
        );
    }

    #[test]
    fn format_finished_short_duration_robust() {
        let s = format_finished(Duration::from_secs(3));
        assert!(s.ends_with(" for 3s"), "short-duration format: {s}");
    }
}

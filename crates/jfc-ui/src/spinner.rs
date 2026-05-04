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

use std::time::Duration;

/// 6-frame spinner cycle. Matches v126's `nAH()` default (cli.js:170248).
pub const FRAMES: &[&str] = &["·", "✢", "*", "✶", "✻", "✽"];

/// Curated subset of v126's verb list (cli.js:233823-234022). The full set
/// is 177 entries; we keep a representative ~32 so the rotation feels lively
/// without piling on novelty words. All verbs end without a suffix — the
/// `…` ellipsis is appended at format time.
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
];

/// Picks a frame index from a tick counter. Caller is expected to bump the
/// tick on every redraw — typically every 80ms (one `AppEvent::Tick`).
pub fn frame_for(tick: usize) -> &'static str {
    FRAMES[tick % FRAMES.len()]
}

/// Picks a verb based on the elapsed time so the verb stays stable for
/// ~2-second windows (less jittery than per-frame randomization).
pub fn verb_for(elapsed: Duration) -> &'static str {
    let bucket = (elapsed.as_secs() / 2) as usize;
    VERBS[bucket % VERBS.len()]
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

/// Choose between the wire-truth `output_tokens` (cumulative count from
/// Anthropic `message_delta`, OWUI/OpenAI `message_stop`) and the
/// chars-divided-by-4 fallback estimate. Wire wins whenever it's non-zero;
/// the estimate covers the brief window before the first delta lands and
/// the providers that don't emit cumulative usage mid-stream at all.
pub fn live_token_count(wire_output: u64, char_estimate: u64) -> u64 {
    if wire_output > 0 {
        wire_output
    } else {
        char_estimate
    }
}

/// Compose the full status line shown above the input bar:
/// `"* Fermenting… (5m 10s · ↓ 14.6k tokens · almost done thinking)"`.
///
/// Returns just the *content* — the renderer wraps it in styled spans.
pub fn format_status(
    tick: usize,
    elapsed: Duration,
    output_tokens: u64,
    time_since_last_token: Duration,
) -> String {
    let mut parts: Vec<String> = vec![fmt_elapsed(elapsed)];
    if output_tokens > 0 {
        parts.push(format!("↓ {} tokens", fmt_tokens(output_tokens)));
    }
    if let Some(s) = stall_status(time_since_last_token) {
        parts.push(s.to_string());
    }
    format!(
        "{} {}… ({})",
        frame_for(tick),
        verb_for(elapsed),
        parts.join(" · ")
    )
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
    fn verb_changes_every_two_seconds_robust() {
        let v0 = verb_for(Duration::from_secs(0));
        let v1 = verb_for(Duration::from_secs(1));
        let v2 = verb_for(Duration::from_secs(2));
        assert_eq!(v0, v1, "verb stable within a 2s window");
        assert_ne!(
            v0, v2,
            "verb advances at the 2s boundary (otherwise display is stuck)"
        );
    }

    #[test]
    fn format_status_includes_all_pieces_normal() {
        let s = format_status(2, Duration::from_secs(310), 14_600, Duration::from_secs(70));
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
        let s = format_status(0, Duration::from_secs(3), 0, Duration::from_secs(0));
        assert!(
            !s.contains("tokens"),
            "should hide token suffix when 0: {s}"
        );
        assert!(s.contains("3s"));
    }

    #[test]
    fn format_status_omits_stall_when_fresh_robust() {
        let s = format_status(0, Duration::from_secs(5), 100, Duration::from_secs(2));
        assert!(
            !s.contains("thinking"),
            "fresh stream shouldn't say 'thinking': {s}"
        );
    }

    #[test]
    fn wire_truth_beats_estimate_when_present_normal() {
        // Anthropic SSE message_delta arrived → wire is truth.
        assert_eq!(live_token_count(150, 200), 150);
    }

    #[test]
    fn estimate_used_when_wire_zero_normal() {
        // OWUI / OpenAI haven't emitted usage mid-stream → fall back.
        assert_eq!(live_token_count(0, 200), 200);
    }

    #[test]
    fn both_zero_yields_zero_robust() {
        // Pre-stream / first-frame state — nothing to show, but no panic.
        assert_eq!(live_token_count(0, 0), 0);
    }

    #[test]
    fn wire_smaller_than_estimate_still_wins_robust() {
        // Anthropic's count tends to be smaller than chars/4 (the estimate
        // double-counts whitespace and code fences). When wire is present,
        // we trust it even when it makes the displayed count drop briefly.
        assert_eq!(live_token_count(50, 9999), 50);
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

//! Honest streaming-status model for the spinner row.
//!
//! Every string this module produces reflects something the stream
//! actually told us — elapsed time, real token counts, real thinking
//! tokens, measured throughput, or measured silence. There are no
//! decorative verbs, no shimmer sweeps, no fabricated reassurance
//! ("almost done thinking"), and no animation that runs when nothing is
//! happening. The renderer reads `App` fields and asks this module to
//! format them; this module is pure formatting — no mutation, no I/O.
//!
//! ## Format (one line)
//!
//! ```text
//! ✦ Thinking · 1m04s · 1.2k thinking · 18 tok/s
//! ✦ Responding · 1m22s · 2.4k tokens · 47 tok/s
//! ```
//!
//! The glyph advances one frame per render tick *while streaming* — that
//! cycle is the only motion, and it stops the instant the stream ends.
//! When the wire has been genuinely silent for a while the renderer dims
//! the row (see [`StatusSegments::dim`]) and a `quiet 47s` chip says so
//! plainly, instead of tinting the verb red or claiming progress.

use std::{sync::OnceLock, time::Duration};

/// Pre-computed status row, split so the renderer can style the glyph,
/// the phase label, and the trailing metadata independently without
/// re-parsing a packed string.
pub struct StatusSegments {
    /// Spinner glyph for this frame (e.g. `✦`).
    pub glyph: &'static str,
    /// Trailing metadata, already `·`-joined and prefixed with a leading
    /// separator: e.g. ` · 1m04s · 2.4k tokens · 47 tok/s`. Rendered
    /// muted — it's context, not the active label.
    pub body: String,
    /// True once the wire has been silent past [`QUIET_DIM_SECS`]. The
    /// renderer dims the glyph + label when set, so a stalled stream
    /// reads as "quiet" rather than "actively working".
    pub dim: bool,
}

/// Whether all UI animation should flatten to a still image. Honored by
/// the renderer's glyph cycle so `JFC_REDUCED_MOTION=1` freezes the
/// spinner. Cached because render code calls this every frame and a
/// running process's environment is not a live configuration channel.
pub fn reduced_motion() -> bool {
    static REDUCED_MOTION: OnceLock<bool> = OnceLock::new();
    *REDUCED_MOTION.get_or_init(|| {
        matches!(
            std::env::var("JFC_REDUCED_MOTION").as_deref(),
            Ok("1") | Ok("true") | Ok("yes")
        )
    })
}

/// Linear-interpolate between two RGB triples with `t ∈ [0, 1]`. A plain
/// numeric utility kept here because the renderer's `pulse_color` blends
/// the glyph toward muted as the stream goes quiet.
pub fn interpolate_rgb(c1: (u8, u8, u8), c2: (u8, u8, u8), t: f32) -> (u8, u8, u8) {
    let t = t.clamp(0.0, 1.0);
    let lerp = |a: u8, b: u8| -> u8 {
        let af = a as f32;
        let bf = b as f32;
        (af + (bf - af) * t).round().clamp(0.0, 255.0) as u8
    };
    (lerp(c1.0, c2.0), lerp(c1.1, c2.1), lerp(c1.2, c2.2))
}

/// Spinner glyph cycle. A small star set — the user reads the star pulse
/// as "Claude". The renderer advances it one frame per tick while
/// streaming and holds it on a single frame otherwise.
pub const FRAMES: &[&str] = crate::glyphs::STATUS_FRAMES;

/// Glyph for `tick`. Caller bumps the tick once per redraw while
/// streaming; a held tick freezes the glyph.
pub fn frame_for(tick: usize) -> &'static str {
    FRAMES[tick % FRAMES.len()]
}

/// Format an elapsed duration compactly: `4s`, `47s`, `1m04s`, `61m01s`.
/// Seconds are zero-padded in the minutes case so the clock doesn't jump
/// width as it ticks (`1m09s` → `1m10s`, not `1m9s` → `1m10s`).
pub fn fmt_elapsed(elapsed: Duration) -> String {
    let secs = elapsed.as_secs();
    if secs >= 60 {
        format!("{}m{:02}s", secs / 60, secs % 60)
    } else {
        format!("{secs}s")
    }
}

/// Format a token count compactly: `234`, `1.4k`, `15k`, `2.0M`. Below
/// 1000 the exact count is useful mid-stream; above that we clamp to one
/// decimal for k/M.
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

/// Seconds of wire silence before the `quiet Ns` chip appears. Short
/// gaps between deltas are normal, so we wait a beat before saying so.
pub const QUIET_CHIP_SECS: u64 = 8;

/// Seconds of wire silence before the renderer dims the row. Past this
/// the stream has plausibly stalled; the dim is the honest "it's gone
/// quiet" signal that replaces the old red "rusting" fade.
pub const QUIET_DIM_SECS: u64 = 30;

/// Honest silence chip: how long since the last text/reasoning delta.
/// `None` while the stream is fresh. Says exactly what it knows ("quiet
/// for N seconds") and never claims the model is "almost done".
pub fn quiet_status(time_since_last_token: Duration) -> Option<String> {
    let secs = time_since_last_token.as_secs();
    (secs >= QUIET_CHIP_SECS).then(|| format!("quiet {}", fmt_elapsed(time_since_last_token)))
}

/// Trailing window over which the live tokens/sec rate is measured. Short
/// enough to reflect *current* speed (self-smoothing over the bursty
/// `message_delta` batches) rather than a lifetime average that lags once
/// a fast opening burst tapers.
pub const TOKEN_RATE_WINDOW: Duration = Duration::from_secs(5);

/// Minimum spread between oldest and newest in-window sample before a rate
/// is reported. Below this the denominator is too small to be stable, so
/// the chip is suppressed rather than flickering a wild number.
const TOKEN_RATE_MIN_SPAN: Duration = Duration::from_millis(1200);

/// Floor below which the rate chip is hidden — a sub-1 tok/s reading is
/// almost always a stalled tail, better communicated by the quiet chip.
const TOKEN_RATE_FLOOR: f64 = 1.0;

/// Drop samples older than `TOKEN_RATE_WINDOW` relative to the newest.
/// `samples` is `(elapsed_from_stream_start, cumulative_token_count)`,
/// monotonic in both coordinates. Pure (operates on `Duration`) so it's
/// unit-testable without sleeping.
pub fn trim_token_samples(samples: &mut std::collections::VecDeque<(Duration, u64)>) {
    let Some(&(newest, _)) = samples.back() else {
        return;
    };
    let cutoff = newest.saturating_sub(TOKEN_RATE_WINDOW);
    while samples.len() > 2 {
        match samples.front() {
            Some(&(t, _)) if t < cutoff => {
                samples.pop_front();
            }
            _ => break,
        }
    }
}

/// Tokens/sec over the in-window samples: `Δtokens / Δseconds` between the
/// oldest and newest retained sample. `None` when there isn't enough
/// spread to be meaningful (so the caller hides the chip).
pub fn windowed_token_rate(samples: &std::collections::VecDeque<(Duration, u64)>) -> Option<f64> {
    let (&(oldest_t, oldest_tok), &(newest_t, newest_tok)) = samples.front().zip(samples.back())?;
    let span = newest_t.saturating_sub(oldest_t);
    if span < TOKEN_RATE_MIN_SPAN {
        return None;
    }
    let delta_tokens = newest_tok.saturating_sub(oldest_tok) as f64;
    let rate = delta_tokens / span.as_secs_f64();
    (rate >= TOKEN_RATE_FLOOR).then_some(rate)
}

/// Live-vs-finished thinking signal. The model is either *currently*
/// producing reasoning, or it *has finished* (and we know how long it
/// took). `None` means it didn't use extended thinking this turn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThinkingStatus {
    /// Reasoning chunks are arriving — the phase label reads `Thinking`.
    Live,
    /// Reasoning ended; text has started. We show a `thought Ns` chip.
    Done(Duration),
}

// ── Spinner phase state machine (anti-flicker) ───────────────────────────
//
// The label used to be derived raw at render time, so it flipped the instant
// a driving field changed (Thinking→Responding on the first text byte;
// Working→Responding on the first token), producing visible strobing and
// "stuck" reads in the agentic gap. We instead drive an explicit phase with a
// minimum dwell so a label can't change faster than `MIN_PHASE_DWELL`, plus a
// minimum thinking-display so a brief reasoning burst still reads as
// "Thinking" for a beat. Transitions are only ever *delayed*, never
// fabricated — every phase maps to a real stream signal.

/// Minimum time a soft phase stays on screen before a different soft phase can
/// replace it. Suppresses sub-frame thrash (e.g. Responding↔Working across the
/// stream-end → tool-run → next-stream agentic gap).
pub const MIN_PHASE_DWELL: Duration = Duration::from_millis(400);
/// Minimum time `Thinking` is held before `Responding` may take over, so a
/// short reasoning burst doesn't strobe. Matches Claude Code's 2s.
pub const MIN_THINKING_DISPLAY: Duration = Duration::from_millis(2000);

/// The honest spinner phase. Each variant maps 1:1 to a real stream signal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpinnerPhase {
    /// Request sent, connection live, no bytes yet.
    Requesting,
    /// Extended-thinking chunks arriving.
    Thinking,
    /// Output text/tokens streaming.
    Responding,
    /// Stream ended; tools executing in the agentic gap.
    ToolUse,
    /// Turn active but none of the above (startup gap / waiting).
    Working,
    /// Pre-submit context compaction.
    Compacting,
    /// Provider retry after a transient network/API failure.
    NetworkRecovery,
}

impl SpinnerPhase {
    /// Honest one-word label. `Compacting`/`NetworkRecovery` render through
    /// their own row shapes in `spinner_row`, so this label is only read for
    /// the streaming phases.
    pub fn label(self) -> &'static str {
        match self {
            SpinnerPhase::Requesting => "Requesting",
            SpinnerPhase::Thinking => "Thinking",
            SpinnerPhase::Responding => "Responding",
            // Tools run between sub-streams; "Working" is the honest umbrella.
            SpinnerPhase::ToolUse | SpinnerPhase::Working => "Working",
            SpinnerPhase::Compacting => "Compacting",
            SpinnerPhase::NetworkRecovery => "Recovering",
        }
    }
}

/// Raw per-tick signals the phase machine reduces into a phase.
#[derive(Debug, Clone, Copy)]
pub struct RawPhaseInputs {
    pub compacting: bool,
    pub network_recovery: bool,
    pub is_streaming: bool,
    pub thinking_live: bool,
    pub thinking_ended: bool,
    pub output_started: bool,
    pub tools_pending: bool,
    pub turn_active: bool,
}

/// Stored phase plus the timestamps the hysteresis rules read. Lives on `App`,
/// advanced once per tick by [`next_phase`].
#[derive(Debug, Clone, Copy)]
pub struct SpinnerState {
    pub phase: SpinnerPhase,
    pub entered_at: std::time::Instant,
    /// When `Thinking` was first seen this turn (drives `MIN_THINKING_DISPLAY`).
    pub thinking_first_seen_at: Option<std::time::Instant>,
}

impl SpinnerState {
    pub fn new(now: std::time::Instant) -> Self {
        Self {
            phase: SpinnerPhase::Working,
            entered_at: now,
            thinking_first_seen_at: None,
        }
    }
}

/// The phase the raw signals *want*, before hysteresis.
fn desired_phase(r: &RawPhaseInputs) -> SpinnerPhase {
    if r.compacting {
        return SpinnerPhase::Compacting;
    }
    if r.network_recovery {
        return SpinnerPhase::NetworkRecovery;
    }
    if r.thinking_live {
        return SpinnerPhase::Thinking;
    }
    if r.is_streaming && (r.thinking_ended || r.output_started) {
        return SpinnerPhase::Responding;
    }
    if r.tools_pending {
        return SpinnerPhase::ToolUse;
    }
    if r.is_streaming && r.turn_active {
        return SpinnerPhase::Requesting;
    }
    SpinnerPhase::Working
}

/// Compute the next phase from the current one, applying hysteresis. Pure —
/// all clocks are passed in, so it unit-tests without sleeping.
pub fn next_phase(
    current: SpinnerPhase,
    entered_at: std::time::Instant,
    thinking_first_seen_at: Option<std::time::Instant>,
    now: std::time::Instant,
    r: RawPhaseInputs,
) -> SpinnerPhase {
    let desired = desired_phase(&r);
    // Hard, non-oscillating states win immediately (entering AND leaving): the
    // user must see compaction / network recovery the instant it's true, and
    // these fields don't flicker so there's nothing to debounce.
    if matches!(
        desired,
        SpinnerPhase::Compacting | SpinnerPhase::NetworkRecovery
    ) || matches!(
        current,
        SpinnerPhase::Compacting | SpinnerPhase::NetworkRecovery
    ) {
        return desired;
    }
    // Hold Thinking for a beat so a brief reasoning burst doesn't strobe to
    // Responding on the first text byte.
    if current == SpinnerPhase::Thinking
        && desired == SpinnerPhase::Responding
        && thinking_first_seen_at.is_some_and(|t| now.duration_since(t) < MIN_THINKING_DISPLAY)
    {
        return SpinnerPhase::Thinking;
    }
    if desired == current {
        return current;
    }
    // Soft transition: enforce the dwell floor so labels can't flip per-frame.
    if now.duration_since(entered_at) < MIN_PHASE_DWELL {
        return current;
    }
    desired
}

/// Build the honest status row. `thinking` selects the phase label and
/// which token counter leads; `output_tokens` / `thinking_tokens` /
/// `token_rate` come straight off the wire; `time_since_last_token`
/// drives the quiet chip and the `dim` flag.
pub fn status_segments(
    tick: usize,
    elapsed: Duration,
    output_tokens: u64,
    token_rate: Option<f64>,
    time_since_last_token: Duration,
    thinking: Option<ThinkingStatus>,
    thinking_tokens: u64,
) -> StatusSegments {
    let mut parts: Vec<String> = vec![fmt_elapsed(elapsed)];
    let push_rate = |parts: &mut Vec<String>| {
        if let Some(rate) = token_rate {
            parts.push(format!("{rate:.0} tok/s"));
        }
    };

    match thinking {
        Some(ThinkingStatus::Live) => {
            // The leading `elapsed` chip already shows the live duration (and
            // during the Thinking phase that duration *is* the thinking time),
            // so don't repeat it as a redundant `thinking {elapsed}` chip —
            // that rendered the same number twice (`Thinking · 1m04s ·
            // thinking 1m04s`). Show the cumulative thinking-token total
            // instead: the same honest count Claude Code surfaces (not a
            // `~`-marked estimate). The server's running total may plateau at
            // a steady state, but it's still the real count.
            if thinking_tokens > 0 {
                parts.push(format!("{} thinking", fmt_tokens(thinking_tokens)));
            }
            push_rate(&mut parts);
        }
        Some(ThinkingStatus::Done(d)) => {
            parts.push(format!("thought {}s", d.as_secs().max(1)));
            if output_tokens > 0 {
                parts.push(format!("{} tokens", fmt_tokens(output_tokens)));
                push_rate(&mut parts);
            }
        }
        None => {
            if output_tokens > 0 {
                parts.push(format!("{} tokens", fmt_tokens(output_tokens)));
                push_rate(&mut parts);
            }
        }
    }

    if let Some(q) = quiet_status(time_since_last_token) {
        parts.push(q);
    }

    StatusSegments {
        glyph: frame_for(tick),
        body: format!(" · {}", parts.join(" · ")),
        dim: time_since_last_token.as_secs() >= QUIET_DIM_SECS,
    }
}

/// Post-turn footer shown dim under each completed assistant message:
/// just the honest elapsed time. The caller may append the turn's cost
/// (`2m04s · $0.04`). No decorative past-tense verb.
pub fn format_finished(elapsed: Duration) -> String {
    fmt_elapsed(elapsed)
}

/// Compact-mode status body. Same honest, paren-free shape as the
/// streaming row: glyph + `Compacting` + elapsed + input magnitude +
/// live summary output.
///
/// - `pre_tokens` — pre-compact context size (`412k tokens`).
/// - `output_chars` — cumulative summary text length, divided by 4 for a
///   token estimate. Without it a 1m+ compact looks frozen even though
///   the API is streaming summary text.
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
        parts.push(format!("↓ {} tokens", fmt_tokens(output_chars / 4)));
    }
    format!("{} Compacting · {}", frame_for(tick), parts.join(" · "))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn streaming_responding() -> RawPhaseInputs {
        RawPhaseInputs {
            compacting: false,
            network_recovery: false,
            is_streaming: true,
            thinking_live: false,
            thinking_ended: true,
            output_started: true,
            tools_pending: false,
            turn_active: true,
        }
    }

    #[test]
    fn phase_holds_thinking_for_min_display_normal() {
        use std::time::Instant;
        let t0 = Instant::now();
        let raw = streaming_responding();
        // 500ms after Thinking began → still Thinking (held).
        assert_eq!(
            next_phase(
                SpinnerPhase::Thinking,
                t0,
                Some(t0),
                t0 + Duration::from_millis(500),
                raw
            ),
            SpinnerPhase::Thinking
        );
        // Past the 2s floor → Responding allowed.
        assert_eq!(
            next_phase(
                SpinnerPhase::Thinking,
                t0,
                Some(t0),
                t0 + Duration::from_millis(2100),
                raw
            ),
            SpinnerPhase::Responding
        );
    }

    #[test]
    fn phase_dwell_suppresses_fast_soft_switch_robust() {
        use std::time::Instant;
        let t0 = Instant::now();
        let raw = RawPhaseInputs {
            is_streaming: false,
            tools_pending: true,
            ..streaming_responding()
        };
        // Only 100ms in Responding → ToolUse switch suppressed.
        assert_eq!(
            next_phase(
                SpinnerPhase::Responding,
                t0,
                None,
                t0 + Duration::from_millis(100),
                raw
            ),
            SpinnerPhase::Responding
        );
        // After the dwell floor → transition allowed.
        assert_eq!(
            next_phase(
                SpinnerPhase::Responding,
                t0,
                None,
                t0 + Duration::from_millis(500),
                raw
            ),
            SpinnerPhase::ToolUse
        );
    }

    #[test]
    fn phase_compacting_overrides_dwell_immediately_robust() {
        use std::time::Instant;
        let t0 = Instant::now();
        let raw = RawPhaseInputs {
            compacting: true,
            ..streaming_responding()
        };
        // Entered Responding 10ms ago, but compaction must win now.
        assert_eq!(
            next_phase(
                SpinnerPhase::Responding,
                t0,
                None,
                t0 + Duration::from_millis(10),
                raw
            ),
            SpinnerPhase::Compacting
        );
    }

    #[test]
    fn elapsed_format_under_60s_normal() {
        assert_eq!(fmt_elapsed(Duration::from_secs(0)), "0s");
        assert_eq!(fmt_elapsed(Duration::from_secs(7)), "7s");
        assert_eq!(fmt_elapsed(Duration::from_secs(59)), "59s");
    }

    #[test]
    fn elapsed_format_minutes_zero_padded_normal() {
        assert_eq!(fmt_elapsed(Duration::from_secs(60)), "1m00s");
        assert_eq!(fmt_elapsed(Duration::from_secs(64)), "1m04s");
        assert_eq!(fmt_elapsed(Duration::from_secs(310)), "5m10s");
        assert_eq!(fmt_elapsed(Duration::from_secs(3661)), "61m01s");
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
    fn frame_cycle_wraps_robust() {
        assert_eq!(frame_for(0), "·");
        assert_eq!(frame_for(2), "✦");
        assert_eq!(frame_for(5), "✽");
        assert_eq!(frame_for(6), "·"); // wraps
        assert_eq!(frame_for(usize::MAX), FRAMES[usize::MAX % FRAMES.len()]);
    }

    #[test]
    fn quiet_status_is_honest_normal() {
        // Fresh / brief gaps say nothing.
        assert_eq!(quiet_status(Duration::from_secs(0)), None);
        assert_eq!(quiet_status(Duration::from_secs(7)), None);
        // Past the threshold it states the measured silence — and never
        // fabricates "almost done thinking".
        assert_eq!(
            quiet_status(Duration::from_secs(8)).as_deref(),
            Some("quiet 8s")
        );
        assert_eq!(
            quiet_status(Duration::from_secs(62)).as_deref(),
            Some("quiet 1m02s")
        );
    }

    #[test]
    fn status_responding_shows_tokens_and_rate_normal() {
        let s = status_segments(
            2,
            Duration::from_secs(82),
            2_400,
            Some(47.0),
            Duration::from_secs(1),
            None,
            0,
        );
        assert_eq!(s.glyph, frame_for(2));
        assert!(s.body.contains("1m22s"), "elapsed missing: {}", s.body);
        assert!(s.body.contains("2.4k tokens"), "tokens missing: {}", s.body);
        assert!(s.body.contains("47 tok/s"), "rate missing: {}", s.body);
        assert!(!s.dim, "fresh stream should not be dim");
    }

    #[test]
    fn status_live_thinking_shows_thinking_tokens_normal() {
        let s = status_segments(
            0,
            Duration::from_secs(64),
            0,
            Some(18.0),
            Duration::from_secs(2),
            Some(ThinkingStatus::Live),
            1_200,
        );
        // The leading elapsed chip shows the live duration once.
        assert!(s.body.contains("1m04s"), "elapsed missing: {}", s.body);
        // Regression guard: the duration must NOT be repeated as a redundant
        // `thinking {elapsed}` chip — that rendered "1m04s" twice
        // (`Thinking · 1m04s · thinking 1m04s`).
        assert!(
            !s.body.contains("thinking 1m04s"),
            "duration should not be doubled as a `thinking {{elapsed}}` chip: {}",
            s.body
        );
        assert_eq!(
            s.body.matches("1m04s").count(),
            1,
            "elapsed should appear exactly once: {}",
            s.body
        );
        // Cumulative thinking-token total, rendered as `1.2k thinking`
        // (matches the module-doc status line), no `~` estimate marker.
        assert!(
            s.body.contains("1.2k thinking"),
            "thinking token total missing: {}",
            s.body
        );
        assert!(
            !s.body.contains('~'),
            "thinking tokens should be the total, not a ~ estimate: {}",
            s.body
        );
        assert!(
            s.body.contains("18 tok/s"),
            "thinking rate missing: {}",
            s.body
        );
    }

    #[test]
    fn status_done_thinking_shows_duration_then_output_normal() {
        let s = status_segments(
            0,
            Duration::from_secs(60),
            5_000,
            Some(40.0),
            Duration::from_secs(0),
            Some(ThinkingStatus::Done(Duration::from_secs(12))),
            0,
        );
        assert!(
            s.body.contains("thought 12s"),
            "thought chip missing: {}",
            s.body
        );
        assert!(
            s.body.contains("5.0k tokens"),
            "output tokens missing: {}",
            s.body
        );
    }

    #[test]
    fn status_done_thinking_floors_to_one_second_robust() {
        let s = status_segments(
            0,
            Duration::from_secs(5),
            100,
            None,
            Duration::from_secs(0),
            Some(ThinkingStatus::Done(Duration::from_millis(400))),
            0,
        );
        assert!(
            s.body.contains("thought 1s"),
            "expected 1s floor: {}",
            s.body
        );
    }

    #[test]
    fn status_working_before_any_output_robust() {
        let s = status_segments(
            0,
            Duration::from_secs(2),
            0,
            None,
            Duration::from_secs(0),
            None,
            0,
        );
        assert!(!s.body.contains("tokens"), "no token chip yet: {}", s.body);
        assert!(s.body.contains("2s"));
    }

    #[test]
    fn status_dims_and_chips_when_quiet_normal() {
        let s = status_segments(
            0,
            Duration::from_secs(90),
            500,
            None,
            Duration::from_secs(47),
            None,
            0,
        );
        assert!(s.dim, "47s of silence should dim the row");
        assert!(
            s.body.contains("quiet 47s"),
            "quiet chip missing: {}",
            s.body
        );
    }

    #[test]
    fn windowed_rate_basic_normal() {
        let mut samples = std::collections::VecDeque::new();
        samples.push_back((Duration::from_millis(0), 0u64));
        samples.push_back((Duration::from_millis(2000), 100u64));
        let rate = windowed_token_rate(&samples).expect("should produce a rate");
        assert!((rate - 50.0).abs() < 0.1, "expected 50 tok/s, got {rate}");
    }

    #[test]
    fn trim_drops_stale_samples_normal() {
        let mut samples = std::collections::VecDeque::new();
        for (t, tok) in [(0u64, 0u64), (1000, 50), (2000, 100), (10_000, 500)] {
            samples.push_back((Duration::from_millis(t), tok));
        }
        trim_token_samples(&mut samples);
        assert!(samples.len() >= 2, "must keep ≥2 samples: {:?}", samples);
        assert_eq!(
            *samples.back().unwrap(),
            (Duration::from_millis(10_000), 500)
        );
    }

    #[test]
    fn windowed_rate_single_sample_returns_none_robust() {
        let mut samples = std::collections::VecDeque::new();
        samples.push_back((Duration::from_millis(0), 0u64));
        assert!(windowed_token_rate(&samples).is_none());
    }

    #[test]
    fn windowed_rate_below_min_span_returns_none_robust() {
        let mut samples = std::collections::VecDeque::new();
        samples.push_back((Duration::from_millis(0), 0u64));
        samples.push_back((Duration::from_millis(500), 100u64));
        assert!(windowed_token_rate(&samples).is_none());
    }

    #[test]
    fn windowed_rate_empty_returns_none_robust() {
        let samples: std::collections::VecDeque<(Duration, u64)> =
            std::collections::VecDeque::new();
        assert!(windowed_token_rate(&samples).is_none());
    }

    #[test]
    fn format_finished_is_just_elapsed_normal() {
        assert_eq!(format_finished(Duration::from_secs(310)), "5m10s");
        assert_eq!(format_finished(Duration::from_secs(3)), "3s");
    }

    #[test]
    fn format_compact_status_includes_pieces_normal() {
        let s = format_compact_status(0, Duration::from_secs(15), 412_000, 4_800);
        assert!(s.contains("Compacting"), "verb missing: {s}");
        assert!(s.contains("15s"), "elapsed missing: {s}");
        assert!(s.contains("412k tokens"), "pre-token chip missing: {s}");
        assert!(
            s.contains("↓ 1.2k tokens"),
            "output token chip missing: {s}"
        );
    }

    #[test]
    fn format_compact_status_omits_empty_chips_robust() {
        let s = format_compact_status(0, Duration::from_secs(2), 0, 0);
        assert!(s.contains("Compacting"), "verb missing: {s}");
        assert!(s.contains("2s"), "elapsed missing: {s}");
        assert!(!s.contains("0 tokens"), "shouldn't show 0-token chip: {s}");
        assert!(!s.contains("↓"), "shouldn't show empty output chip: {s}");
    }
}

//! Synthetic PQRST trace generator for the sidebar network monitor.
//!
//! A real EKG has five named features per heartbeat — P, Q, R, S, T —
//! that map to the depolarization / repolarization phases of the
//! cardiac cycle. We model them as a 14-sample beat at 80ms/tick
//! (~52 BPM, comfortably resting-range), with the R wave's amplitude
//! scaling with recent network activity.
//!
//! Pattern derivation (from NeuroKit2 `ecg_simulate.py` and `signalz`
//! `ecgsyn`): the canonical PQRST extrema are roughly
//!   P: amp +1.2, Q: amp −5, R: amp +30, S: amp −7.5, T: amp +0.75.
//! Q and S are *negative* deflections — they dip below the
//! isoelectric line. To represent that in a 1-row block-element
//! sparkline (which has no "below zero" cell), we lift the
//! isoelectric baseline up to 2 so Q and S can drop to 0 and still
//! read as undershoots.
//!
//! ```text
//!     8 |          R─┐ (scales with activity)
//!     7 |          │
//!     6 |          │
//!     5 |          │
//!     4 |       T─┐│   ┌─T (crest)
//!     3 |    P─┐ ││   ┌┘
//!     2 |───┐  │ │└──┐│        ← isoelectric baseline
//!     1 |               └─ diastolic rest
//!     0 |     └Q┘  └S┘
//!         0 1 2 3 4 5 6 7 8 9 10 11 12 13
//! ```
//!
//! Cell layout (14 phases):
//!   0:  isoelectric baseline (2)
//!   1:  P wave rising  (3)
//!   2:  P wave crest   (4)
//!   3:  P wave falling (2)
//!   4:  Q dip          (0) — sharp negative deflection
//!   5:  R peak         (R_PEAK_BASE..=R_PEAK_MAX, scales with activity)
//!   6:  S dip          (0) — sharp negative deflection
//!   7:  ST segment     (2) — back to baseline
//!   8:  T rising       (3)
//!   9:  T crest        (4)
//!  10:  T falling      (2)
//!  11-13: diastolic rest (1) — slight dip during refractory period
//!
//! Inspired by:
//!   * NeuroKit2 / `signalz.ecgsyn` for the extrema amplitudes
//!   * btop's network-graph sparkline renderer (block-element scale,
//!     leading-edge color, scroll-from-right pattern)
//!   * ratatui's `Sparkline` with `RenderDirection::RightToLeft`

/// Number of phases in one heartbeat cycle. At 80ms/tick → 1120ms per
/// beat → ~54 BPM. Adjust to retime the beat.
pub const PATTERN_LEN: usize = 14;

/// Baseline R-wave amplitude (idle traffic). The trace still beats at
/// this minimum so the user sees the system is alive even between
/// turns. Real ECGs at rest hit R ~30; our idle is "small enough to
/// read as resting heart" → block-scale 5.
pub const R_PEAK_BASE: f32 = 5.0;

/// Maximum R-wave amplitude at full network activity — saturates at
/// the top of the block-element scale (`█`).
pub const R_PEAK_MAX: f32 = 8.0;

/// EMA smoothing factor for `network_activity`. Lower = more lag
/// (slower to ramp up and decay); higher = snappier. 0.18 was tuned
/// against a sustained SSE stream — bursts grow the spike over ~3-4
/// ticks (~300ms) and decay over ~10 ticks (~800ms), close to a real
/// monitor's heart-rate-response feel.
pub const ACTIVITY_EMA_ALPHA: f32 = 0.18;

/// Isoelectric baseline amplitude. Above 0 so Q/S can deflect down.
pub const ISOELECTRIC: u8 = 2;

/// Compute the EKG sample for a given phase index and current activity
/// factor. `activity` is clamped to 0.0..=1.0.
pub fn sample(phase: usize, activity: f32) -> u8 {
    let activity = activity.clamp(0.0, 1.0);
    let r_amp = R_PEAK_BASE + (R_PEAK_MAX - R_PEAK_BASE) * activity;
    let r_amp_u8 = r_amp.round().clamp(0.0, 8.0) as u8;
    match phase % PATTERN_LEN {
        0 => ISOELECTRIC,  // PR onset baseline
        1 => 3,            // P rising
        2 => 4,            // P crest
        3 => ISOELECTRIC,  // P falling back
        4 => 0,            // Q dip (negative deflection)
        5 => r_amp_u8,     // R peak (scales with activity)
        6 => 0,            // S dip (negative deflection)
        7 => ISOELECTRIC,  // ST segment
        8 => 3,            // T rising
        9 => 4,            // T crest
        10 => ISOELECTRIC, // T falling
        11 => 1,           // diastolic rest 1
        12 => 1,           // diastolic rest 2
        13 => 1,           // diastolic rest 3 (TP segment)
        _ => ISOELECTRIC,
    }
}

/// Advance the EKG one tick — event-triggered semantics.
///
/// The trace flat-lines (baseline cells) when there's no network
/// activity. Real data (text, reasoning, tool deltas, usage frames…)
/// triggers a *beat*: the next `PATTERN_LEN` ticks walk through the
/// full PQRST waveform, after which the trace returns to flat-line
/// until the next byte arrives. This makes the widget a true network
/// monitor instead of a synthetic constant heartbeat — typing in the
/// input box no longer draws a beat, because typing doesn't move
/// network bytes.
///
/// `beat_remaining` is the runtime's beat-state: 0 = idle (flat-line),
/// `>0` = mid-beat. The caller stores it on `App`.
///
/// EMA-eased `activity` factor is still smoothed so a burst-of-bursts
/// produces taller QRS spikes for the duration of the activity window
/// rather than amplitude-thrashing per tick.
pub fn tick(
    samples: &mut std::collections::VecDeque<u8>,
    phase: &mut usize,
    activity: &mut f32,
    beat_remaining: &mut usize,
    delta_bytes: u64,
) {
    // Map per-tick byte delta to a 0.0..=1.0 activity factor on a log
    // scale. The thresholds match a typical SSE chunk-size distribution.
    let target = match delta_bytes {
        0 => 0.0,
        1..=16 => 0.20,
        17..=64 => 0.35,
        65..=256 => 0.55,
        257..=1024 => 0.70,
        1025..=4096 => 0.85,
        4097..=16_384 => 0.95,
        _ => 1.0,
    };
    *activity = *activity + (target - *activity) * ACTIVITY_EMA_ALPHA;

    // Network event arrived this tick → arm a fresh beat. We re-arm
    // (rather than ignore the new delta) so sustained streaming keeps
    // the trace beating — burst at t=0, finish PATTERN_LEN ticks
    // later, but if a second burst lands at t=4 it restarts the
    // cycle. Without re-arm a steady stream would beat once then go
    // flat for a tick before the next event re-armed.
    if delta_bytes > 0 {
        *beat_remaining = PATTERN_LEN;
        // Snap the phase so the new beat starts at the *beginning* of
        // the pattern (PR onset → P → Q → R → …). Otherwise the user
        // might see a beat that's already mid-T-wave.
        *phase = 0;
    }

    let v = if *beat_remaining > 0 {
        let s = sample(*phase, *activity);
        *phase = phase.wrapping_add(1);
        *beat_remaining -= 1;
        s
    } else {
        // True flat-line: the isoelectric baseline so the trace reads
        // as "the wire is up, just no traffic" rather than a missing
        // widget (height 0 collapses the row).
        ISOELECTRIC
    };

    samples.push_back(v);
    while samples.len() > 80 {
        samples.pop_front();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pattern_has_clear_r_peak_at_idle_normal() {
        // At idle (activity=0), the R-wave is still the tallest phase.
        let amps: Vec<u8> = (0..PATTERN_LEN).map(|i| sample(i, 0.0)).collect();
        let max_idx = amps
            .iter()
            .enumerate()
            .max_by_key(|(_, v)| *v)
            .map(|(i, _)| i)
            .unwrap();
        assert_eq!(max_idx, 5, "R-wave should be at phase 5");
    }

    #[test]
    fn r_peak_scales_with_activity_normal() {
        let idle = sample(5, 0.0);
        let active = sample(5, 1.0);
        assert!(active > idle, "active R must exceed idle R: {idle} {active}");
        assert_eq!(active, 8, "full activity should saturate R at 8");
    }

    #[test]
    fn q_and_s_deflect_below_isoelectric_normal() {
        // The Q (phase 4) and S (phase 6) deflections are the
        // characteristic "negative spikes" surrounding R; in our
        // 1-row block-scale model they drop to 0 against an
        // ISOELECTRIC baseline of 2.
        for a in [0.0, 0.5, 1.0] {
            assert_eq!(sample(4, a), 0, "Q dip at phase 4 must reach 0");
            assert_eq!(sample(6, a), 0, "S dip at phase 6 must reach 0");
        }
    }

    #[test]
    fn isoelectric_baseline_is_above_zero_robust() {
        // The PR onset (phase 0) sits at the isoelectric baseline so
        // Q/S have somewhere to drop *from*. If this ever regressed
        // to 0, the Q/S undershoots would disappear visually.
        assert_eq!(sample(0, 0.0), ISOELECTRIC);
        assert!(ISOELECTRIC > 0);
    }

    #[test]
    fn pattern_completes_a_cycle_normal() {
        for a in [0.0, 0.5, 1.0] {
            assert_eq!(sample(0, a), sample(PATTERN_LEN, a));
            assert_eq!(sample(5, a), sample(PATTERN_LEN + 5, a));
        }
    }

    #[test]
    fn idle_ticks_stay_flat_normal() {
        // Zero-byte ticks must NOT advance the beat — flat-line is the
        // whole point of event-triggered mode.
        let mut samples = std::collections::VecDeque::new();
        let mut phase: usize = 0;
        let mut activity: f32 = 0.0;
        let mut beat_remaining: usize = 0;
        for _ in 0..20 {
            tick(&mut samples, &mut phase, &mut activity, &mut beat_remaining, 0);
        }
        // Every pushed sample should be the isoelectric baseline.
        assert!(
            samples.iter().all(|v| *v == ISOELECTRIC),
            "idle samples should all be ISOELECTRIC, got {:?}",
            samples
        );
        assert_eq!(beat_remaining, 0);
    }

    #[test]
    fn event_triggers_one_full_beat_normal() {
        // A single non-zero delta arms a beat that walks the full
        // PATTERN_LEN, then returns to flat-line. The R-wave (phase 5)
        // must show up among the pushed samples.
        let mut samples = std::collections::VecDeque::new();
        let mut phase: usize = 0;
        let mut activity: f32 = 0.0;
        let mut beat_remaining: usize = 0;
        // Burst at t=0.
        tick(&mut samples, &mut phase, &mut activity, &mut beat_remaining, 4096);
        // Quiet for the rest of the beat.
        for _ in 0..(PATTERN_LEN - 1) {
            tick(&mut samples, &mut phase, &mut activity, &mut beat_remaining, 0);
        }
        // Now flat-line.
        for _ in 0..5 {
            tick(&mut samples, &mut phase, &mut activity, &mut beat_remaining, 0);
        }
        assert_eq!(beat_remaining, 0);
        assert_eq!(samples.len(), PATTERN_LEN + 5);
        // The last 5 samples should be flat (idle after the beat).
        for v in samples.iter().rev().take(5) {
            assert_eq!(*v, ISOELECTRIC, "trailing samples should be flat");
        }
        // The R-wave should be among the first PATTERN_LEN samples.
        let beat_samples: Vec<u8> = samples.iter().take(PATTERN_LEN).copied().collect();
        let r_value = beat_samples.iter().max().copied().unwrap();
        assert!(r_value >= R_PEAK_BASE as u8, "expected R-peak in beat, got {:?}", beat_samples);
    }

    #[test]
    fn re_arm_during_active_beat_restarts_pattern_robust() {
        // A second burst mid-beat should restart the pattern at phase
        // 0, not let the trace finish the original cycle first.
        // Sustained streaming should produce overlapping beats.
        let mut samples = std::collections::VecDeque::new();
        let mut phase: usize = 0;
        let mut activity: f32 = 0.0;
        let mut beat_remaining: usize = 0;
        tick(&mut samples, &mut phase, &mut activity, &mut beat_remaining, 4096);
        // We're now at phase=1 with beat_remaining=PATTERN_LEN-1.
        // A second burst should snap phase back to 0 and re-arm.
        tick(&mut samples, &mut phase, &mut activity, &mut beat_remaining, 4096);
        // After the second burst's tick we should be at phase=1
        // again with beat_remaining=PATTERN_LEN-1, NOT PATTERN_LEN-2.
        assert_eq!(beat_remaining, PATTERN_LEN - 1);
        assert_eq!(phase, 1);
    }

    #[test]
    fn tick_caps_buffer_at_80_robust() {
        let mut samples = std::collections::VecDeque::new();
        let mut phase: usize = 0;
        let mut activity: f32 = 0.0;
        let mut beat_remaining: usize = 0;
        for _ in 0..200 {
            tick(&mut samples, &mut phase, &mut activity, &mut beat_remaining, 0);
        }
        assert_eq!(samples.len(), 80);
    }

    #[test]
    fn activity_eases_toward_target_normal() {
        let mut samples = std::collections::VecDeque::new();
        let mut phase: usize = 0;
        let mut activity: f32 = 0.0;
        let mut beat_remaining: usize = 0;
        tick(&mut samples, &mut phase, &mut activity, &mut beat_remaining, 100_000);
        assert!(activity > 0.0 && activity < 1.0, "got {activity}");
    }
}

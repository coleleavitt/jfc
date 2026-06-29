//! Animated recording cursor — port of Claude Code 2.1.177's `RM8` (and its
//! `SM8` HSL→RGB helper).
//!
//! While recording, the cursor glyph reflects the live microphone level
//! (`" ▁▂▃▄▅▆▇█"`, smoothed) and its color rotates through the hue wheel at
//! 90°/s; quiet input renders gray. While processing, a single block pulses
//! between two grays on a 2 s cycle. The level data is real (per-chunk RMS from
//! the capture pipeline), so this animates only on genuine audio activity.

use ratatui::style::Color;

/// Cursor glyphs by intensity (`vI6`): index 0 is blank, 1..=8 the rising bars.
pub const GLYPHS: [char; 9] = [' ', '▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];

/// Level-smoothing factor (`HL7`): new = prev*0.7 + scaled*0.3.
const SMOOTH: f32 = 0.7;
/// Level scale before clamping to 1.0. The CLI used 1.8 to amplify its float
/// RMS; our level is already mapped to a perceptual [0,1] dBFS window upstream
/// (`recorder::normalize_level`), so no extra gain is applied here.
const SCALE: f32 = 1.0;
/// Below this level the glyph renders gray, not hue-rotated (~quiet-room noise
/// floor on the dBFS scale; CC's `yq5` was 0.15 against its un-windowed level).
const GRAY_BELOW: f32 = 0.10;
/// Hue rotation rate, degrees per second.
const HUE_DEG_PER_SEC: f32 = 90.0;
/// Hue quantization buckets for non-truecolor terminals (`tX7`).
const HUE_BUCKETS: f32 = 8.0;

/// Compute the recording cursor glyph + color from the recent RMS levels
/// (`levels`, newest last) and elapsed recording time. `truecolor` selects
/// continuous vs. bucket-quantized hue.
///
/// The CLI keeps a running EMA across 50 ms samples; we fold the same EMA over
/// the retained level ring so the result is deterministic per render (no
/// persistent animation state to thread through the immutable render pass).
pub fn recording_glyph(levels: &[f32], elapsed_ms: u128, truecolor: bool) -> (char, Color) {
    let mut smoothed = 0.0f32;
    for &lvl in levels {
        let scaled = (lvl * SCALE).min(1.0);
        smoothed = smoothed * SMOOTH + scaled * (1.0 - SMOOTH);
    }
    let latest = levels.last().copied().unwrap_or(0.0);

    let max_idx = GLYPHS.len() - 1;
    let idx = ((smoothed * max_idx as f32).round() as usize).clamp(1, max_idx);
    let glyph = GLYPHS[idx];

    let color = if latest < GRAY_BELOW {
        Color::Rgb(128, 128, 128)
    } else {
        let mut hue = (elapsed_ms as f32 / 1000.0 * HUE_DEG_PER_SEC) % 360.0;
        if !truecolor {
            hue = quantize_hue(hue);
        }
        let (r, g, b) = hsl_to_rgb(hue, 0.7, 0.6);
        Color::Rgb(r, g, b)
    };
    (glyph, color)
}

/// Processing-state pulse color: tweens between RGB(153) and RGB(185) on a 2 s
/// cosine cycle (`VoiceIndicator` processing animation).
pub fn processing_pulse(elapsed_ms: u128) -> Color {
    let phase = (1.0 - (2.0 * std::f32::consts::PI * (elapsed_ms as f32) / 2000.0).cos()) / 2.0;
    let v = (153.0 + (185.0 - 153.0) * phase).round() as u8;
    Color::Rgb(v, v, v)
}

/// Quantize a hue to one of [`HUE_BUCKETS`] steps around the wheel (`hM8`).
fn quantize_hue(hue: f32) -> f32 {
    ((hue / 360.0 * HUE_BUCKETS).round() / HUE_BUCKETS) * 360.0
}

/// HSL→RGB for the fixed S=0.7, L=0.6 the cursor uses (`SM8`). `hue` in degrees.
fn hsl_to_rgb(hue: f32, sat: f32, light: f32) -> (u8, u8, u8) {
    let h = hue.rem_euclid(360.0);
    let c = (1.0 - (2.0 * light - 1.0).abs()) * sat;
    let x = c * (1.0 - ((h / 60.0) % 2.0 - 1.0).abs());
    let m = light - c / 2.0;
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
        ((r + m) * 255.0).round() as u8,
        ((g + m) * 255.0).round() as u8,
        ((b + m) * 255.0).round() as u8,
    )
}

/// Whether the terminal advertises 24-bit color (`COLORTERM`), gating the
/// continuous vs. quantized hue path.
pub fn terminal_truecolor() -> bool {
    std::env::var("COLORTERM")
        .map(|v| {
            let v = v.to_lowercase();
            v.contains("truecolor") || v.contains("24bit")
        })
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn glyph_rises_with_level_normal() {
        // Silence → minimum glyph (index clamped to 1, the lowest bar).
        let (g, _) = recording_glyph(&[0.0], 0, true);
        assert_eq!(g, GLYPHS[1]);
        // Sustained loud input → near the top bar.
        let loud = [1.0f32; 16];
        let (g, _) = recording_glyph(&loud, 0, true);
        assert_eq!(g, GLYPHS[8]);
        // Empty ring → still a valid (minimum) glyph, no panic.
        let (g, _) = recording_glyph(&[], 0, true);
        assert_eq!(g, GLYPHS[1]);
    }

    #[test]
    fn quiet_is_gray_loud_is_hued_normal() {
        // Latest sample below the gray threshold → flat gray.
        let (_, c) = recording_glyph(&[0.05], 0, true);
        assert_eq!(c, Color::Rgb(128, 128, 128));
        // Above threshold → a saturated hue (not the gray sentinel).
        let (_, c) = recording_glyph(&[0.9], 0, true);
        assert_ne!(c, Color::Rgb(128, 128, 128));
    }

    #[test]
    fn hue_rotates_over_time_normal() {
        // Two distinct elapsed times above threshold → different colors.
        let levels = [0.9f32];
        let a = recording_glyph(&levels, 0, true).1;
        let b = recording_glyph(&levels, 1500, true).1; // +135° hue
        assert_ne!(a, b);
    }

    #[test]
    fn hsl_to_rgb_primaries_normal() {
        // Hue 0 (red-ish), 120 (green-ish), 240 (blue-ish) at S=0.7,L=0.6.
        let (r, g, b) = hsl_to_rgb(0.0, 0.7, 0.6);
        assert!(r > g && r > b, "hue 0 should be red-dominant: {r},{g},{b}");
        let (r, g, b) = hsl_to_rgb(120.0, 0.7, 0.6);
        assert!(
            g > r && g > b,
            "hue 120 should be green-dominant: {r},{g},{b}"
        );
        let (r, g, b) = hsl_to_rgb(240.0, 0.7, 0.6);
        assert!(
            b > r && b > g,
            "hue 240 should be blue-dominant: {r},{g},{b}"
        );
    }

    #[test]
    fn processing_pulse_stays_in_band_robust() {
        for t in (0..2000).step_by(100) {
            if let Color::Rgb(r, g, b) = processing_pulse(t) {
                assert_eq!(r, g);
                assert_eq!(g, b);
                assert!((153..=185).contains(&r), "pulse out of band at {t}: {r}");
            } else {
                panic!("processing_pulse must return Rgb");
            }
        }
    }

    #[test]
    fn quantize_hue_buckets_robust() {
        // Quantized hues snap to multiples of 45° (360/8).
        for h in [0.0, 10.0, 50.0, 200.0, 359.0] {
            let q = quantize_hue(h);
            assert!(
                (q / 45.0).fract().abs() < 1e-3,
                "{h} → {q} not a 45° bucket"
            );
        }
    }
}

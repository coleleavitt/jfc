// Thin re-export for backwards compatibility — all types live in jfc-theme.
pub use jfc_theme::*;

// ─── Color Utilities (moved from swarm/types.rs — engine code must not
//     produce ratatui colors; teammate colors travel as hex strings) ─────────

/// Parse a hex color string (e.g., "#4FC3F7") into a ratatui `Color::Rgb`.
/// Returns `Color::White` if parsing fails.
pub fn hex_to_color(hex: &str) -> ratatui::style::Color {
    let hex = hex.trim_start_matches('#');
    if hex.len() != 6 {
        return ratatui::style::Color::White;
    }
    let r = u8::from_str_radix(&hex[0..2], 16).unwrap_or(255);
    let g = u8::from_str_radix(&hex[2..4], 16).unwrap_or(255);
    let b = u8::from_str_radix(&hex[4..6], 16).unwrap_or(255);
    ratatui::style::Color::Rgb(r, g, b)
}

/// Get a ratatui Color for a teammate, falling back to White if no color set.
pub fn teammate_color(color: Option<&str>) -> ratatui::style::Color {
    match color {
        Some(hex) => hex_to_color(hex),
        None => ratatui::style::Color::White,
    }
}

#[cfg(test)]
mod color_util_tests {
    use super::*;
    use ratatui::style::Color;

    #[test]
    fn hex_to_color_parses_six_digit_hex_normal() {
        assert_eq!(hex_to_color("#FF0000"), Color::Rgb(255, 0, 0));
        assert_eq!(hex_to_color("00FF00"), Color::Rgb(0, 255, 0));
        assert_eq!(hex_to_color("#0000ff"), Color::Rgb(0, 0, 255));
    }

    #[test]
    fn hex_to_color_returns_white_on_bad_input_robust() {
        // Wrong length / not hex → fall back to White instead of panicking.
        assert_eq!(hex_to_color("abc"), Color::White);
        assert_eq!(hex_to_color("#1234567"), Color::White);
        assert_eq!(hex_to_color(""), Color::White);
    }

    #[test]
    fn teammate_color_falls_back_to_white_normal() {
        assert_eq!(teammate_color(None), Color::White);
        assert_eq!(teammate_color(Some("#FFFFFF")), Color::Rgb(255, 255, 255));
    }
}

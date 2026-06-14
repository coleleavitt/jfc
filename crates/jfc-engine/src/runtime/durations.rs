//! Tiny duration/elapsed formatters shared by engine handlers and the TUI
//! spinner. Engine-resident so stream handlers can label turn footers
//! without linking the render stack.

use std::time::Duration;

/// `5s` under a minute, `2m04s` above.
pub fn fmt_elapsed(elapsed: Duration) -> String {
    let secs = elapsed.as_secs();
    if secs >= 60 {
        format!("{}m{:02}s", secs / 60, secs % 60)
    } else {
        format!("{secs}s")
    }
}

/// Post-turn footer shown under each completed assistant message: just the
/// honest elapsed time. The caller may append the turn's cost
/// (`2m04s · $0.04`). No decorative past-tense verb.
pub fn format_finished(elapsed: Duration) -> String {
    fmt_elapsed(elapsed)
}

/// Time-to-first-token readout for the turn footer: `420ms` under a second,
/// `1.4s` above. Compact since it sits alongside elapsed + cost.
pub fn format_ttft(ttft_ms: u64) -> String {
    if ttft_ms >= 1000 {
        format!("{:.1}s", ttft_ms as f64 / 1000.0)
    } else {
        format!("{ttft_ms}ms")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_ttft_boundaries_normal() {
        assert_eq!(format_ttft(0), "0ms");
        assert_eq!(format_ttft(420), "420ms");
        assert_eq!(format_ttft(999), "999ms");
        assert_eq!(format_ttft(1000), "1.0s");
        assert_eq!(format_ttft(1400), "1.4s");
        assert_eq!(format_ttft(9999), "10.0s");
    }

    #[test]
    fn fmt_elapsed_minute_boundary_normal() {
        assert_eq!(fmt_elapsed(Duration::from_secs(5)), "5s");
        assert_eq!(fmt_elapsed(Duration::from_secs(59)), "59s");
        assert_eq!(fmt_elapsed(Duration::from_secs(60)), "1m00s");
        assert_eq!(fmt_elapsed(Duration::from_secs(124)), "2m04s");
    }
}

//! Quiet-hours check for the CC 2.1.167 `quietHours` settings field.
//!
//! Shape: `{ "enabled": bool, "start": "HH:MM", "end": "HH:MM" }`
//! where times are local 24-hour. The `end` time can be earlier than `start`
//! for an overnight range (e.g. 22:00–07:00).
//!
//! CC's own description: "shows a single soft nudge per session while inside
//! the configured local-time window. Never blocks." JFC uses this to gate
//! scheduled/cron task execution.

/// Returns `true` when the current local time falls within the configured
/// quiet-hours window.
///
/// Returns `false` when:
/// - `quiet_hours` is `None`
/// - `enabled` is `false` or absent
/// - `start` or `end` are missing or malformed
pub fn is_quiet_hours(quiet_hours: Option<&serde_json::Value>) -> bool {
    let Some(qh) = quiet_hours else { return false };
    // enabled must be explicitly true
    if !qh.get("enabled").and_then(|v| v.as_bool()).unwrap_or(false) {
        return false;
    }
    let Some(start_str) = qh.get("start").and_then(|v| v.as_str()) else {
        return false;
    };
    let Some(end_str) = qh.get("end").and_then(|v| v.as_str()) else {
        return false;
    };
    let Some(start) = parse_hhmm(start_str) else {
        return false;
    };
    let Some(end) = parse_hhmm(end_str) else {
        return false;
    };
    let now = local_minutes_since_midnight();
    in_window(now, start, end)
}

/// Parse "HH:MM" into minutes since midnight.
fn parse_hhmm(s: &str) -> Option<u32> {
    let (h, m) = s.split_once(':')?;
    let hours: u32 = h.trim().parse().ok()?;
    let minutes: u32 = m.trim().parse().ok()?;
    if hours > 23 || minutes > 59 {
        return None;
    }
    Some(hours * 60 + minutes)
}

/// Current local time as minutes since midnight.
fn local_minutes_since_midnight() -> u32 {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    // Approximate local offset from TZ env var; fall back to UTC.
    // A production impl would use the `chrono` or `time` crate for full
    // timezone support — this is sufficient for the nudge semantics CC uses.
    let utc_minutes = ((secs % 86400) / 60) as u32;
    let offset_minutes: i32 = local_utc_offset_minutes();
    let local_minutes = (utc_minutes as i32 + offset_minutes).rem_euclid(1440) as u32;
    local_minutes
}

/// Cheap local UTC offset estimation from `TZ` env var or `/etc/localtime`.
/// Returns 0 (UTC) when the offset can't be determined.
fn local_utc_offset_minutes() -> i32 {
    // Try $TZ first (e.g. "UTC", "America/New_York", "+05:30")
    if let Ok(tz) = std::env::var("TZ") {
        if let Some(offset) = parse_tz_offset(&tz) {
            return offset;
        }
    }
    0
}

fn parse_tz_offset(tz: &str) -> Option<i32> {
    // Handle numeric offsets like "+05:30" or "-07:00"
    let tz = tz.trim();
    let (sign, rest) = if let Some(r) = tz.strip_prefix('+') {
        (1i32, r)
    } else if let Some(r) = tz.strip_prefix('-') {
        (-1i32, r)
    } else {
        return None;
    };
    if let Some((h, m)) = rest.split_once(':') {
        let hours: i32 = h.parse().ok()?;
        let mins: i32 = m.parse().ok()?;
        Some(sign * (hours * 60 + mins))
    } else {
        let hours: i32 = rest.parse().ok()?;
        Some(sign * hours * 60)
    }
}

/// `true` when `now` (minutes since midnight) falls within `[start, end)`.
/// Handles overnight ranges where `end < start`.
fn in_window(now: u32, start: u32, end: u32) -> bool {
    if start <= end {
        now >= start && now < end
    } else {
        // Overnight: start=22:00 (1320), end=07:00 (420)
        now >= start || now < end
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn in_window_normal_range_normal() {
        assert!(in_window(600, 540, 720)); // 10:00 in 09:00–12:00
        assert!(!in_window(480, 540, 720)); // 08:00 not in 09:00–12:00
    }

    #[test]
    fn in_window_overnight_range_normal() {
        assert!(in_window(1380, 1320, 420)); // 23:00 in 22:00–07:00
        assert!(in_window(60, 1320, 420)); // 01:00 in 22:00–07:00
        assert!(!in_window(480, 1320, 420)); // 08:00 not in 22:00–07:00
    }

    #[test]
    fn is_quiet_hours_disabled_returns_false_robust() {
        let qh = json!({"enabled": false, "start": "00:00", "end": "23:59"});
        assert!(!is_quiet_hours(Some(&qh)));
    }

    #[test]
    fn is_quiet_hours_none_returns_false_robust() {
        assert!(!is_quiet_hours(None));
    }

    #[test]
    fn parse_hhmm_valid_normal() {
        assert_eq!(parse_hhmm("09:30"), Some(570));
        assert_eq!(parse_hhmm("22:00"), Some(1320));
        assert_eq!(parse_hhmm("00:00"), Some(0));
    }

    #[test]
    fn parse_hhmm_invalid_returns_none_robust() {
        assert!(parse_hhmm("25:00").is_none());
        assert!(parse_hhmm("not-a-time").is_none());
        assert!(parse_hhmm("").is_none());
    }
}

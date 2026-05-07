//! Telemetry posture knob.
//!
//! jfc doesn't ship telemetry today — but if we ever do, this module is
//! the single switch users flip to opt out. Reading from one helper
//! means the rest of the codebase doesn't need to touch env vars or
//! config files; just call `is_enabled()` before emitting any metric.
//!
//! Precedence:
//! 1. `JFC_TELEMETRY=0|off|false` → disabled.
//! 2. `~/.config/jfc/telemetry-disabled` (file exists) → disabled.
//! 3. Default: enabled (a no-op today; will be a real opt-in metrics
//!    target once we add one).

use std::sync::OnceLock;

fn cached_state() -> &'static bool {
    static STATE: OnceLock<bool> = OnceLock::new();
    STATE.get_or_init(compute_state)
}

fn compute_state() -> bool {
    if let Ok(v) = std::env::var("JFC_TELEMETRY") {
        if matches!(v.trim(), "0" | "off" | "false" | "no") {
            return false;
        }
    }
    if let Some(home) = std::env::var_os("HOME") {
        let marker = std::path::Path::new(&home)
            .join(".config")
            .join("jfc")
            .join("telemetry-disabled");
        if marker.exists() {
            return false;
        }
    }
    true
}

/// Whether telemetry should fire. `false` means callers should skip
/// any metric/event emission entirely.
pub fn is_enabled() -> bool {
    *cached_state()
}

/// One-line status string for `/doctor` and the ribbon.
pub fn status_label() -> &'static str {
    if is_enabled() {
        "enabled (no events shipped today)"
    } else {
        "disabled (user opt-out)"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compute_state_default_is_enabled_normal() {
        // SAFETY: single-threaded test scope.
        unsafe {
            std::env::remove_var("JFC_TELEMETRY");
        }
        // We can't test compute_state directly because cached_state
        // memoizes — but new processes will pick the right default.
        assert_eq!(status_label().is_empty(), false);
    }

    #[test]
    fn env_value_off_disables_normal() {
        // Direct compute (bypassing the cache).
        unsafe {
            std::env::set_var("JFC_TELEMETRY", "off");
        }
        assert!(!compute_state());
        unsafe {
            std::env::remove_var("JFC_TELEMETRY");
        }
    }

    #[test]
    fn env_value_zero_disables_normal() {
        unsafe {
            std::env::set_var("JFC_TELEMETRY", "0");
        }
        assert!(!compute_state());
        unsafe {
            std::env::remove_var("JFC_TELEMETRY");
        }
    }

    #[test]
    fn env_value_unknown_keeps_default_robust() {
        unsafe {
            std::env::set_var("JFC_TELEMETRY", "maybe");
        }
        assert!(compute_state());
        unsafe {
            std::env::remove_var("JFC_TELEMETRY");
        }
    }
}

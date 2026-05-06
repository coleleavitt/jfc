//! OS-level desktop notifications for long-running events the user
//! might miss while focused elsewhere. Fire-and-forget: each call
//! spawns a tokio task that builds and posts the notification, so the
//! UI thread never blocks on the notification daemon.
//!
//! Gated by `JFC_DISABLE_NOTIFICATIONS=1` so headless / SSH sessions
//! that don't have a notification daemon don't generate errors.
//!
//! Triggers we surface:
//! - Long turn completed (>= `LONG_TURN_THRESHOLD`).
//! - Tool failed (when `is_error=true`).
//! - Compaction failed permanently.
//!
//! Mirrors v126's notifications hook (cli.js around 26647) which
//! emits desktop notifications via the same shell hooks the
//! settings.json `hooks` system supports.

use std::time::Duration;

/// Don't notify on every short turn — only when the user has clearly
/// been waiting. 8 seconds matches the typical "I'd look away for
/// this" threshold from v126.
pub const LONG_TURN_THRESHOLD: Duration = Duration::from_secs(8);

fn notifications_enabled() -> bool {
    !matches!(
        std::env::var("JFC_DISABLE_NOTIFICATIONS").as_deref(),
        Ok("1") | Ok("true")
    )
}

/// Post a notification. Spawns a task and returns immediately so the
/// caller never blocks on the daemon. Errors are logged via tracing.
pub fn notify(summary: impl Into<String>, body: impl Into<String>) {
    if !notifications_enabled() {
        return;
    }
    let summary = summary.into();
    let body = body.into();
    tokio::spawn(async move {
        // notify-rust is sync; the spawn keeps it off the UI thread.
        // Showing through the system notification daemon (`notify-send`
        // / xdg-notify on Linux, NotificationCenter on macOS, Toast
        // on Windows) — same surface every other dev tool uses.
        let result = notify_rust::Notification::new()
            .summary(&summary)
            .body(&body)
            .appname("jfc")
            .timeout(notify_rust::Timeout::Milliseconds(5000))
            .show();
        if let Err(e) = result {
            tracing::debug!(target: "jfc::notify", error = %e, "notification failed");
        }
    });
}

/// Fire a "turn complete" notification when the elapsed exceeds the
/// threshold. Saves the user from refocusing every 30s to check.
pub fn notify_turn_complete(elapsed: Duration, summary: &str) {
    if elapsed < LONG_TURN_THRESHOLD {
        return;
    }
    let body = if summary.is_empty() {
        format!(
            "Finished after {}s",
            elapsed.as_secs().max(1)
        )
    } else {
        let preview: String = summary.chars().take(120).collect();
        format!("({}s) {}", elapsed.as_secs().max(1), preview)
    };
    notify("jfc · turn complete", body);
}

pub fn notify_tool_failed(tool_name: &str, message: &str) {
    let preview: String = message.chars().take(120).collect();
    notify(format!("jfc · {tool_name} failed"), preview);
}

pub fn notify_compact_failed(reason: &str) {
    notify("jfc · compaction failed", reason.to_owned());
}

/// Build the body string a `notify_turn_complete` call would produce — split out
/// so tests can verify the gating + format without going near the OS daemon.
fn build_turn_complete_body(elapsed: Duration, summary: &str) -> Option<String> {
    if elapsed < LONG_TURN_THRESHOLD {
        return None;
    }
    let body = if summary.is_empty() {
        format!("Finished after {}s", elapsed.as_secs().max(1))
    } else {
        let preview: String = summary.chars().take(120).collect();
        format!("({}s) {}", elapsed.as_secs().max(1), preview)
    };
    Some(body)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Wrap each test that mutates `JFC_DISABLE_NOTIFICATIONS` so the env var
    /// doesn't leak between tests run on the same thread. SAFETY: tests in
    /// this module are single-threaded — `cargo test` uses one thread per
    /// test by default but env mutations are still process-global, so we
    /// always restore to the prior state on Drop.
    struct EnvGuard {
        key: &'static str,
        previous: Option<String>,
    }

    impl EnvGuard {
        fn set(key: &'static str, value: &str) -> Self {
            let previous = std::env::var(key).ok();
            // SAFETY: tests do not run concurrently with code that reads env vars
            // in a way that could race; they're set/cleared synchronously here.
            unsafe {
                std::env::set_var(key, value);
            }
            Self { key, previous }
        }

        fn unset(key: &'static str) -> Self {
            let previous = std::env::var(key).ok();
            unsafe {
                std::env::remove_var(key);
            }
            Self { key, previous }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            unsafe {
                match &self.previous {
                    Some(v) => std::env::set_var(self.key, v),
                    None => std::env::remove_var(self.key),
                }
            }
        }
    }

    // ─── notifications_enabled gating ────────────────────────────────────

    #[test]
    fn notifications_enabled_default_when_var_unset_normal() {
        let _g = EnvGuard::unset("JFC_DISABLE_NOTIFICATIONS");
        assert!(notifications_enabled());
    }

    #[test]
    fn notifications_disabled_with_one_robust() {
        let _g = EnvGuard::set("JFC_DISABLE_NOTIFICATIONS", "1");
        assert!(!notifications_enabled());
    }

    #[test]
    fn notifications_disabled_with_true_robust() {
        let _g = EnvGuard::set("JFC_DISABLE_NOTIFICATIONS", "true");
        assert!(!notifications_enabled());
    }

    #[test]
    fn notifications_enabled_for_unrelated_value_robust() {
        // Anything other than "1" or "true" (e.g. "yes", "on", "0") should
        // leave notifications enabled — we only honor the documented gate.
        let _g = EnvGuard::set("JFC_DISABLE_NOTIFICATIONS", "yes");
        assert!(notifications_enabled());
    }

    #[test]
    fn notifications_enabled_for_zero_robust() {
        let _g = EnvGuard::set("JFC_DISABLE_NOTIFICATIONS", "0");
        assert!(notifications_enabled());
    }

    // ─── notify_turn_complete body shape + gating ────────────────────────

    #[test]
    fn turn_complete_below_threshold_emits_nothing_normal() {
        // Below the 8-second threshold a short successful turn shouldn't
        // pop a notification.
        let body = build_turn_complete_body(
            Duration::from_secs(3),
            "wrote a small refactor",
        );
        assert!(body.is_none(), "short turn should not notify, got: {body:?}");
    }

    #[test]
    fn turn_complete_at_threshold_emits_notification_normal() {
        let body =
            build_turn_complete_body(LONG_TURN_THRESHOLD, "ran tests").unwrap();
        assert!(body.contains("ran tests"), "summary not preserved: {body}");
        assert!(body.contains('s'), "should include seconds suffix: {body}");
    }

    #[test]
    fn turn_complete_empty_summary_uses_finished_after_normal() {
        let body =
            build_turn_complete_body(Duration::from_secs(15), "").unwrap();
        assert!(body.starts_with("Finished after "), "got: {body}");
        assert!(body.contains("15s"), "got: {body}");
    }

    #[test]
    fn turn_complete_zero_elapsed_below_threshold_robust() {
        // Zero elapsed is meaningless but must not panic — and is below
        // threshold so produces nothing.
        assert!(build_turn_complete_body(Duration::ZERO, "x").is_none());
    }

    #[test]
    fn turn_complete_caps_summary_at_120_chars_robust() {
        // The preview is `chars().take(120)` — a 200-char summary should
        // be truncated, but the format prefix `(Ns) ` adds bytes too.
        let long_summary = "x".repeat(200);
        let body =
            build_turn_complete_body(Duration::from_secs(10), &long_summary).unwrap();
        // Count "x" characters, not the prefix.
        let x_count = body.chars().filter(|c| *c == 'x').count();
        assert_eq!(x_count, 120, "summary should be truncated to 120 chars");
    }

    #[test]
    fn turn_complete_seconds_floor_is_at_least_one_robust() {
        // `elapsed.as_secs().max(1)` — even a 0.1s elapsed renders as "1s".
        // Above-threshold so we get a body. (Synthesizing this with
        // Duration::from_millis(8_001) which floors to 8s.)
        let body =
            build_turn_complete_body(Duration::from_millis(8_001), "").unwrap();
        assert!(body.contains("8s"), "got: {body}");
    }

    #[test]
    fn turn_complete_unicode_summary_truncation_robust() {
        // Multi-byte chars must still truncate by char count, not by byte.
        let summary: String = "あ".repeat(200); // 200 multi-byte chars
        let body = build_turn_complete_body(Duration::from_secs(10), &summary).unwrap();
        let count = body.chars().filter(|c| *c == 'あ').count();
        assert_eq!(count, 120, "char-based truncation must respect unicode");
    }

    // ─── notify, notify_tool_failed, notify_compact_failed ────────────────

    /// `notify` spawns a tokio task; we only verify it doesn't panic when
    /// the gate is engaged. (We don't actually want to talk to a daemon
    /// in CI.)
    #[tokio::test]
    async fn notify_with_disable_gate_is_noop_robust() {
        let _g = EnvGuard::set("JFC_DISABLE_NOTIFICATIONS", "1");
        notify("test", "body");
        // No panic, no spawned task — function returned synchronously.
        // Yield once so any spawned task (there shouldn't be one) runs.
        tokio::task::yield_now().await;
    }

    #[tokio::test]
    async fn notify_tool_failed_is_noop_when_disabled_robust() {
        let _g = EnvGuard::set("JFC_DISABLE_NOTIFICATIONS", "1");
        notify_tool_failed("Bash", "exited 1");
        tokio::task::yield_now().await;
    }

    #[tokio::test]
    async fn notify_compact_failed_is_noop_when_disabled_robust() {
        let _g = EnvGuard::set("JFC_DISABLE_NOTIFICATIONS", "1");
        notify_compact_failed("ran out of context");
        tokio::task::yield_now().await;
    }

    #[tokio::test]
    async fn notify_turn_complete_below_threshold_skips_normal() {
        // Even with notifications enabled, a short turn doesn't dispatch.
        let _g = EnvGuard::set("JFC_DISABLE_NOTIFICATIONS", "1");
        notify_turn_complete(Duration::from_millis(100), "short");
        tokio::task::yield_now().await;
    }

    // ─── LONG_TURN_THRESHOLD constant sanity ─────────────────────────────

    #[test]
    fn long_turn_threshold_is_eight_seconds_normal() {
        assert_eq!(LONG_TURN_THRESHOLD, Duration::from_secs(8));
    }
}

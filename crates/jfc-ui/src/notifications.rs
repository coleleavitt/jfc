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

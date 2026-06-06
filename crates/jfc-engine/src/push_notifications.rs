//! Optional mobile push transport.
//!
//! When the user sets `JFC_PUSH_URL` to an ntfy.sh / Pushover / generic
//! webhook endpoint, jfc POSTs a small JSON payload there alongside
//! every desktop notification. This is the "I'm AFK, ping my phone"
//! escape hatch.
//!
//! Wire shape (POST JSON body):
//! ```json
//! {
//!   "title": "jfc · turn complete",
//!   "body":  "(126s) Refactored the auth middleware",
//!   "kind":  "turn_complete"
//! }
//! ```
//!
//! ntfy.sh accepts this directly (the JSON gets surfaced as title +
//! message in the mobile notification). Pushover-compatible endpoints
//! work the same. For generic webhooks, the receiver does whatever it
//! wants with the JSON.
//!
//! Failures are swallowed via tracing — push is informational, never
//! critical.

use serde::Serialize;

#[derive(Debug, Clone, Copy)]
pub enum PushKind {
    TurnComplete,
    ToolFailed,
    BudgetWarning,
}

impl PushKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::TurnComplete => "turn_complete",
            Self::ToolFailed => "tool_failed",
            Self::BudgetWarning => "budget_warning",
        }
    }
}

#[derive(Debug, Serialize)]
struct PushBody<'a> {
    title: &'a str,
    body: &'a str,
    kind: &'static str,
}

/// Fire a push notification. Reads target URL from `JFC_PUSH_URL`. No-op
/// when unset. Spawned async so the caller never blocks on the wire.
pub fn push(title: &str, body: &str, kind: PushKind) {
    let Ok(url) = std::env::var("JFC_PUSH_URL") else {
        return;
    };
    let url = url.trim().to_owned();
    if url.is_empty() {
        return;
    }
    let title = title.to_owned();
    let body = body.to_owned();
    let kind_label = kind.label();
    tokio::spawn(async move {
        let client = match reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(5))
            .build()
        {
            Ok(c) => c,
            Err(e) => {
                tracing::debug!(target: "jfc::push", error = %e, "client build");
                return;
            }
        };
        let payload = PushBody {
            title: &title,
            body: &body,
            kind: kind_label,
        };
        match client.post(&url).json(&payload).send().await {
            Ok(r) => {
                tracing::debug!(
                    target: "jfc::push",
                    status = %r.status(),
                    kind = kind_label,
                    "pushed"
                );
            }
            Err(e) => {
                tracing::debug!(target: "jfc::push", error = %e, "push failed");
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn push_no_env_is_noop_robust() {
        // SAFETY: clear before test.
        unsafe {
            std::env::remove_var("JFC_PUSH_URL");
        }
        // Should not panic, should not spawn anything that errors.
        push("title", "body", PushKind::TurnComplete);
    }

    #[test]
    fn push_kind_labels_normal() {
        assert_eq!(PushKind::TurnComplete.label(), "turn_complete");
        assert_eq!(PushKind::ToolFailed.label(), "tool_failed");
        assert_eq!(PushKind::BudgetWarning.label(), "budget_warning");
    }
}

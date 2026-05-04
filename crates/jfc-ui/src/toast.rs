//! Auto-expiring toast notifications. Each toast is a brief one-line
//! message anchored to the top of the message pane that fades after a
//! fixed TTL. Used for things v126's `notification()` (cli.js around the
//! 26647 area) surfaces — "Compaction running…", "Bell muted",
//! "Session saved", etc.
//!
//! Pure data model + lifecycle here; the renderer reads `app.toasts`
//! and the main-loop `Tick` handler calls `prune_expired` periodically.

use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToastKind {
    Info,
    Success,
    Warning,
    Error,
}

#[derive(Clone, Debug)]
pub struct Toast {
    pub kind: ToastKind,
    pub text: String,
    pub created_at: Instant,
    pub ttl: Duration,
}

impl Toast {
    pub fn new(kind: ToastKind, text: impl Into<String>) -> Self {
        Self {
            kind,
            text: text.into(),
            created_at: Instant::now(),
            ttl: default_ttl_for(kind),
        }
    }

    pub fn with_ttl(mut self, ttl: Duration) -> Self {
        self.ttl = ttl;
        self
    }

    pub fn is_expired_at(&self, now: Instant) -> bool {
        now.duration_since(self.created_at) >= self.ttl
    }
}

/// Default TTL per kind. Errors stick around longer than info because
/// the user is more likely to need to read them; success messages are
/// brief because the action already happened.
fn default_ttl_for(kind: ToastKind) -> Duration {
    match kind {
        ToastKind::Info => Duration::from_secs(4),
        ToastKind::Success => Duration::from_secs(3),
        ToastKind::Warning => Duration::from_secs(6),
        ToastKind::Error => Duration::from_secs(8),
    }
}

/// Drop expired toasts from the queue. Called from the `Tick` handler so
/// the toast strip auto-clears without requiring user input. Preserves
/// insertion order for the survivors.
pub fn prune_expired(toasts: &mut Vec<Toast>, now: Instant) {
    toasts.retain(|t| !t.is_expired_at(now));
}

/// Cap the number of toasts kept in memory. If the model spams toasts
/// faster than they expire we drop the oldest so the UI doesn't grow
/// unboundedly. Default cap matches `notification()`'s assumption that
/// a few messages are visible at once.
pub const MAX_TOASTS: usize = 5;

pub fn push_with_cap(toasts: &mut Vec<Toast>, t: Toast) {
    if toasts.len() >= MAX_TOASTS {
        toasts.remove(0);
    }
    toasts.push(t);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ttl_per_kind_normal() {
        // Errors stick longest; success is briefest. v126 uses a similar
        // tiering for its terminal notifications.
        assert!(default_ttl_for(ToastKind::Error) > default_ttl_for(ToastKind::Warning));
        assert!(default_ttl_for(ToastKind::Warning) > default_ttl_for(ToastKind::Info));
        assert!(default_ttl_for(ToastKind::Info) > default_ttl_for(ToastKind::Success));
    }

    #[test]
    fn expiration_check_normal() {
        let t = Toast::new(ToastKind::Info, "hi").with_ttl(Duration::from_millis(100));
        let now = t.created_at;
        assert!(!t.is_expired_at(now), "fresh toast not expired");
        assert!(
            !t.is_expired_at(now + Duration::from_millis(50)),
            "mid-life toast not expired"
        );
        assert!(
            t.is_expired_at(now + Duration::from_millis(200)),
            "past-TTL toast IS expired"
        );
    }

    #[test]
    fn prune_drops_only_expired_normal() {
        let now = Instant::now();
        let mut toasts = vec![
            Toast {
                kind: ToastKind::Info,
                text: "old".into(),
                created_at: now - Duration::from_secs(10),
                ttl: Duration::from_secs(2),
            },
            Toast {
                kind: ToastKind::Info,
                text: "fresh".into(),
                created_at: now,
                ttl: Duration::from_secs(2),
            },
        ];
        prune_expired(&mut toasts, now);
        assert_eq!(toasts.len(), 1);
        assert_eq!(toasts[0].text, "fresh");
    }

    #[test]
    fn prune_preserves_order_robust() {
        let now = Instant::now();
        let fresh_ttl = Duration::from_secs(60);
        let mut toasts = vec![
            Toast {
                kind: ToastKind::Info,
                text: "a".into(),
                created_at: now,
                ttl: fresh_ttl,
            },
            Toast {
                kind: ToastKind::Info,
                text: "stale".into(),
                created_at: now - Duration::from_secs(100),
                ttl: Duration::from_secs(1),
            },
            Toast {
                kind: ToastKind::Info,
                text: "b".into(),
                created_at: now,
                ttl: fresh_ttl,
            },
            Toast {
                kind: ToastKind::Info,
                text: "c".into(),
                created_at: now,
                ttl: fresh_ttl,
            },
        ];
        prune_expired(&mut toasts, now);
        let order: Vec<&str> = toasts.iter().map(|t| t.text.as_str()).collect();
        assert_eq!(order, vec!["a", "b", "c"], "survivor order preserved");
    }

    #[test]
    fn cap_drops_oldest_robust() {
        let mut toasts = Vec::new();
        for i in 0..(MAX_TOASTS + 3) {
            push_with_cap(&mut toasts, Toast::new(ToastKind::Info, format!("t{i}")));
        }
        assert_eq!(toasts.len(), MAX_TOASTS, "cap enforced");
        // After overflow, the oldest entries (`t0`..`t2`) are gone.
        assert_eq!(toasts.first().unwrap().text, "t3");
        assert_eq!(toasts.last().unwrap().text, format!("t{}", MAX_TOASTS + 2));
    }

    #[test]
    fn empty_prune_no_panic_robust() {
        let mut toasts: Vec<Toast> = Vec::new();
        prune_expired(&mut toasts, Instant::now());
        assert!(toasts.is_empty());
    }
}

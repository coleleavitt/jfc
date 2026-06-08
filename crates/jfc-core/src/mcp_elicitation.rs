//! MCP elicitation — user-input collection during tool execution.
//!
//! CC 2.1.167 implements the MCP `elicitation/create` protocol: an MCP server
//! can pause a tool call and request structured input from the user. This module
//! owns:
//!
//! - [`ElicitationKind`] — the two elicitation modes (form + URL)
//! - [`PendingElicitation`] — a live request waiting for user response
//! - The global pending queue + resolver mechanism
//!
//! ## Flow
//!
//! ```text
//! MCP server ─elicitation/create─► JfcClientHandler::create_elicitation
//!                                      │
//!                              push_pending(req) ─► EngineEvent::Frontend(ElicitationRequest)
//!                                      │
//!                              wait on oneshot ◄── ControlEvent::ResolveElicitation
//!                                      │
//!                              return result to MCP server
//!                              fire OnElicitation + OnElicitationResult hooks
//! ```
//!
//! For URL mode an additional `notifications/elicitation/complete` notification
//! arrives from the server after the user finishes at the URL. The handler
//! resolves the pending request automatically.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use serde_json::Value;
use tokio::sync::oneshot;

// ── Public types ─────────────────────────────────────────────────────────────

/// Which elicitation mode the server requested.
#[derive(Debug, Clone)]
pub enum ElicitationKind {
    /// Server wants form input. `schema` describes the fields.
    Form {
        /// Human-readable prompt shown to the user.
        message: String,
        /// Simplified field schema: `{ field_name -> { type, description?, ... } }`.
        /// We keep it as raw JSON so the TUI can render it without depending on
        /// the rmcp schema types.
        schema: Value,
    },
    /// Server wants the user to visit a URL.
    Url {
        /// Human-readable prompt shown to the user.
        message: String,
        /// The URL to open / display.
        url: String,
        /// Server-assigned ID used to match the completion notification.
        elicitation_id: String,
    },
}

impl ElicitationKind {
    /// Short one-line label for logging and toasts.
    pub fn label(&self) -> &str {
        match self {
            Self::Form { .. } => "form",
            Self::Url { .. } => "url",
        }
    }

    pub fn message(&self) -> &str {
        match self {
            Self::Form { message, .. } => message,
            Self::Url { message, .. } => message,
        }
    }
}

/// User response to an elicitation.
#[derive(Debug, Clone)]
pub enum ElicitationResponse {
    /// User filled in the form and accepted.
    Accept {
        /// Key-value pairs matching the requested schema.
        content: Value,
    },
    /// User saw the request but declined to provide input.
    Decline,
    /// User cancelled — operation should abort.
    Cancel,
}

/// A pending elicitation waiting for a user response.
pub struct PendingElicitation {
    /// Which MCP server this came from.
    pub server_name: String,
    /// Unique ID for this pending request (auto-generated).
    pub id: String,
    /// What the server is asking for.
    pub kind: ElicitationKind,
    /// Oneshot sender — resolve this to unblock `create_elicitation`.
    pub(crate) resolver: oneshot::Sender<ElicitationResponse>,
}

impl std::fmt::Debug for PendingElicitation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PendingElicitation")
            .field("server_name", &self.server_name)
            .field("id", &self.id)
            .field("kind", &self.kind.label())
            .finish()
    }
}

// ── Cross-crate notification channel ────────────────────────────────────────

/// Event emitted by `JfcClientHandler` toward the engine when a new
/// elicitation arrives or is resolved. Since `jfc-mcp` cannot import
/// `jfc-engine` types (circular dep), this lightweight event type lives in
/// `jfc-core` so both crates can use it.
#[derive(Debug)]
pub enum ElicitationEvent {
    /// A new elicitation arrived — the engine should emit
    /// `FrontendEvent::ElicitationRequest` with the given snapshot.
    Arrived(ElicitationSnapshot),
    /// The elicitation was resolved (user responded or connection dropped).
    /// The engine fires `OnElicitation` / `OnElicitationResult` hooks.
    Resolved {
        id: String,
        server_name: String,
        mode: String,
        action: String,
    },
}

/// Process-global channel from jfc-mcp → jfc-engine for elicitation events.
static ELICITATION_TX: std::sync::OnceLock<
    std::sync::Mutex<Option<tokio::sync::mpsc::Sender<ElicitationEvent>>>,
> = std::sync::OnceLock::new();

fn elicitation_tx_slot(
) -> &'static std::sync::Mutex<Option<tokio::sync::mpsc::Sender<ElicitationEvent>>> {
    ELICITATION_TX.get_or_init(|| std::sync::Mutex::new(None))
}

/// Register the channel the engine listens on. Called once at engine startup.
pub fn register_elicitation_event_sender(tx: tokio::sync::mpsc::Sender<ElicitationEvent>) {
    if let Ok(mut guard) = elicitation_tx_slot().lock() {
        *guard = Some(tx);
    }
}

/// Send an elicitation event toward the engine (best-effort, non-blocking).
pub fn send_elicitation_event(ev: ElicitationEvent) {
    if let Ok(guard) = elicitation_tx_slot().lock() {
        if let Some(ref tx) = *guard {
            if tx.try_send(ev).is_err() {
                tracing::debug!(
                    target: "jfc::mcp::elicitation",
                    "elicitation event channel full or closed — engine may not see this event"
                );
            }
        }
    }
}

// ── Global queue ─────────────────────────────────────────────────────────────

/// Global map of pending elicitations, keyed by their `id`.
///
/// `JfcClientHandler` (in jfc-mcp) pushes entries and then waits on the
/// paired oneshot receiver. The engine event loop's `ResolveElicitation`
/// handler pops the entry and sends on the resolver.
///
/// Separately, URL elicitations can be auto-resolved by the
/// `ElicitationCompletionNotification` handler keyed on `elicitation_id`.
static PENDING: std::sync::OnceLock<Arc<Mutex<HashMap<String, PendingElicitation>>>> =
    std::sync::OnceLock::new();

fn pending() -> &'static Arc<Mutex<HashMap<String, PendingElicitation>>> {
    PENDING.get_or_init(|| Arc::new(Mutex::new(HashMap::new())))
}

/// Push a new pending elicitation. Returns the paired receiver which
/// `JfcClientHandler::create_elicitation` awaits to get the user's response.
pub fn push(
    server_name: String,
    kind: ElicitationKind,
) -> (String, oneshot::Receiver<ElicitationResponse>) {
    let id = uuid_v4();
    let (tx, rx) = oneshot::channel();
    let entry = PendingElicitation {
        server_name,
        id: id.clone(),
        kind,
        resolver: tx,
    };
    pending().lock().unwrap().insert(id.clone(), entry);
    (id, rx)
}

/// Pop and resolve a pending elicitation by `id`. Returns `true` if found.
pub fn resolve(id: &str, response: ElicitationResponse) -> bool {
    if let Some(entry) = pending().lock().unwrap().remove(id) {
        if entry.resolver.send(response).is_err() {
            // Receiver was dropped — the caller that initiated the elicitation
            // already gave up (e.g. the tool call was cancelled). Not an error.
            tracing::debug!(
                target: "jfc::mcp::elicitation",
                id = %id,
                "elicitation resolved but caller already dropped the receiver"
            );
        }
        true
    } else {
        false
    }
}

/// Pop and resolve a pending URL elicitation by its `elicitation_id` (the
/// server-assigned ID, stored in `ElicitationKind::Url::elicitation_id`).
/// Called by the `ElicitationCompletionNotification` handler.
pub fn resolve_by_elicitation_id(elicitation_id: &str, response: ElicitationResponse) -> bool {
    let id = {
        let guard = pending().lock().unwrap();
        guard
            .iter()
            .find(|(_, v)| {
                if let ElicitationKind::Url { elicitation_id: eid, .. } = &v.kind {
                    eid == elicitation_id
                } else {
                    false
                }
            })
            .map(|(k, _)| k.clone())
    };
    if let Some(id) = id {
        resolve(&id, response)
    } else {
        false
    }
}

/// Snapshot of all pending elicitations for the UI to render.
pub fn snapshot() -> Vec<ElicitationSnapshot> {
    pending()
        .lock()
        .unwrap()
        .values()
        .map(|e| ElicitationSnapshot {
            id: e.id.clone(),
            server_name: e.server_name.clone(),
            kind: e.kind.clone(),
        })
        .collect()
}

/// Non-owning snapshot for the UI (no resolver).
#[derive(Debug, Clone)]
pub struct ElicitationSnapshot {
    pub id: String,
    pub server_name: String,
    pub kind: ElicitationKind,
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn uuid_v4() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let t = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("elicit-{t:x}")
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn push_and_resolve_form_normal() {
        let kind = ElicitationKind::Form {
            message: "Enter name".to_owned(),
            schema: json!({"name": {"type": "string"}}),
        };
        let (id, mut rx) = push("test-server".to_owned(), kind);
        let resolved = resolve(
            &id,
            ElicitationResponse::Accept {
                content: json!({"name": "alice"}),
            },
        );
        assert!(resolved);
        let result = rx.try_recv().expect("should have resolved");
        assert!(matches!(result, ElicitationResponse::Accept { .. }));
    }

    #[test]
    fn resolve_unknown_id_returns_false_robust() {
        assert!(!resolve("does-not-exist", ElicitationResponse::Cancel));
    }

    #[test]
    fn push_and_resolve_url_by_elicitation_id_normal() {
        let elicitation_id = "srv-elicit-abc123".to_owned();
        let kind = ElicitationKind::Url {
            message: "Visit URL".to_owned(),
            url: "https://example.com/auth".to_owned(),
            elicitation_id: elicitation_id.clone(),
        };
        let (_id, _rx) = push("url-server".to_owned(), kind);
        let resolved =
            resolve_by_elicitation_id(&elicitation_id, ElicitationResponse::Accept {
                content: json!({}),
            });
        assert!(resolved);
    }

    #[test]
    fn elicitation_kind_labels_normal() {
        let form = ElicitationKind::Form {
            message: "m".to_owned(),
            schema: json!({}),
        };
        let url = ElicitationKind::Url {
            message: "m".to_owned(),
            url: "https://x.com".to_owned(),
            elicitation_id: "x".to_owned(),
        };
        assert_eq!(form.label(), "form");
        assert_eq!(url.label(), "url");
    }
}

//! Key handler for the MCP elicitation modal.
//!
//! Routes key input when `app.engine.pending_elicitations` is non-empty.
//! The first pending elicitation is the active one; resolving it pops it
//! and exposes the next (if any).
//!
//! Key bindings:
//! - **Tab** — move focus to the next form field
//! - **Enter** — accept (submit current field values / dismiss URL modal)
//! - **Esc** — decline (allow operation to continue without input)
//! - **q** — cancel (abort the operation)
//! - **Backspace** — delete last char in active form field
//! - **Any printable char** — type into the active form field

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use tokio::sync::mpsc;
use tracing;

use jfc_core::mcp_elicitation::{ElicitationKind, ElicitationResponse};

use crate::app::App;
use crate::render::elicitation::ElicitationInputState;

/// Handle a key event for the elicitation modal.
///
/// Returns `true` if the key was consumed (caller should skip other handlers).
pub(super) fn handle_elicitation_key(
    app: &mut App,
    key: KeyEvent,
    tx: &mpsc::Sender<crate::runtime::EngineEvent>,
) -> bool {
    if app.engine.pending_elicitations.is_empty() {
        return false;
    }

    // Peek at the front without popping — we need it to determine form vs url
    let is_form = matches!(
        app.engine.pending_elicitations.front().map(|e| &e.kind),
        Some(ElicitationKind::Form { .. })
    );

    match (key.modifiers, key.code) {
        // Tab — next field (form only)
        (KeyModifiers::NONE | KeyModifiers::SHIFT, KeyCode::Tab) if is_form => {
            app.elicitation_input.next_field();
            true
        }

        // Enter — accept
        (KeyModifiers::NONE, KeyCode::Enter) => {
            resolve_elicitation(
                app,
                tx,
                ElicitationResponse::Accept {
                    content: app.elicitation_input.to_json(),
                },
            );
            true
        }

        // Esc — decline
        (KeyModifiers::NONE, KeyCode::Esc) => {
            resolve_elicitation(app, tx, ElicitationResponse::Decline);
            true
        }

        // q — cancel
        (KeyModifiers::NONE, KeyCode::Char('q')) => {
            resolve_elicitation(app, tx, ElicitationResponse::Cancel);
            true
        }

        // Backspace — delete in active field (form only)
        (KeyModifiers::NONE, KeyCode::Backspace) if is_form => {
            app.elicitation_input.backspace();
            true
        }

        // Printable char — type into active field (form only)
        (KeyModifiers::NONE | KeyModifiers::SHIFT, KeyCode::Char(c)) if is_form => {
            app.elicitation_input.type_char(c);
            true
        }

        _ => false,
    }
}

/// Pop the front pending elicitation, send `ResolveElicitation`, and reset
/// input state. If another elicitation is queued, initialize input for it.
fn resolve_elicitation(
    app: &mut App,
    tx: &mpsc::Sender<crate::runtime::EngineEvent>,
    response: ElicitationResponse,
) {
    let Some(pending) = app.engine.pending_elicitations.pop_front() else {
        return;
    };

    let ev =
        crate::runtime::EngineEvent::Control(crate::runtime::ControlEvent::ResolveElicitation {
            id: pending.id,
            response,
        });
    // Best-effort send — channel full or closed means the engine shut down,
    // which is not a problem we can recover from here.
    if tx.try_send(ev).is_err() {
        tracing::debug!(
            target: "jfc::input::elicitation",
            "failed to send ResolveElicitation — engine bus closed or full"
        );
    }

    // Re-initialize input state for the next queued elicitation (if any).
    if let Some(next) = app.engine.pending_elicitations.front() {
        app.elicitation_input = match &next.kind {
            ElicitationKind::Form { schema, .. } => ElicitationInputState::from_schema(schema),
            ElicitationKind::Url { .. } => ElicitationInputState::default(),
        };
    } else {
        app.elicitation_input = ElicitationInputState::default();
    }
}

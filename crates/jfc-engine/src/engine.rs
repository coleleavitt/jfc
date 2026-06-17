//! The `Engine` handle — the blessed embedding API for jfc frontends.
//!
//! A frontend owns an [`Engine`] plus the receiving half of its event
//! channel, and runs a loop of three moves:
//!
//! ```ignore
//! let (tx, mut rx) = jfc_engine::engine::channel();
//! let mut engine = Engine::new(provider, model, tx);
//! engine.submit("do the thing".into(), Vec::new(), None).await?;
//! while let Some(ev) = rx.recv().await {
//!     match engine.handle_event(ev).await? {
//!         Some(FrontendDirective::SubmitPrompt(text)) => {
//!             engine.submit(text, Vec::new(), None).await?;
//!         }
//!         Some(FrontendDirective::RunCommand(_)) | None => {}
//!     }
//!     for effect in engine.drain_effects() {
//!         // apply scroll/cache/picker reactions — or ignore when headless
//!         let _ = effect;
//!     }
//!     if engine.is_idle() {
//!         break;
//!     }
//! }
//! ```
//!
//! Everything here is sugar over [`EngineState`] + [`runtime::ops`] +
//! [`runtime::handle_engine_event`] — frontends with bespoke needs (the TUI
//! embeds `EngineState` directly inside its `App`) can keep using those
//! primitives; both layers are supported.

use std::sync::Arc;

use crate::app::{EngineEffect, EngineState};
use crate::runtime::{
    self, APP_EVENT_BUFFER, EngineEvent, EventReceiver, EventSender, FrontendDirective,
};

/// Create the engine event channel with the standard buffer size.
pub fn channel() -> (EventSender, EventReceiver) {
    tokio::sync::mpsc::channel(APP_EVENT_BUFFER)
}

/// An owned engine instance: the state plus the sending half of its event
/// bus. See the module docs for the canonical frontend loop.
pub struct Engine {
    pub state: EngineState,
    tx: EventSender,
}

impl Engine {
    /// Construct the engine and register the global tool-event sender so
    /// detached producers (plan-mode tools, economy agents, schedulers)
    /// reach this engine's bus — the same wiring every frontend previously
    /// hand-rolled.
    pub fn new(
        provider: Arc<dyn jfc_provider::Provider>,
        model: impl Into<jfc_provider::ModelId>,
        tx: EventSender,
    ) -> Self {
        crate::tools::register_event_sender(tx.clone());
        // Register the elicitation event channel so jfc-mcp transports can
        // notify the engine when elicitation/create arrives from an MCP server.
        let engine_tx_for_elicit = tx.clone();
        let (elicit_tx, mut elicit_rx) =
            tokio::sync::mpsc::channel::<jfc_core::mcp_elicitation::ElicitationEvent>(64);
        jfc_core::mcp_elicitation::register_elicitation_event_sender(elicit_tx);
        tokio::spawn(async move {
            while let Some(ev) = elicit_rx.recv().await {
                match ev {
                    jfc_core::mcp_elicitation::ElicitationEvent::Arrived(snapshot) => {
                        let fe = crate::runtime::EngineEvent::Frontend(
                            crate::runtime::FrontendEvent::ElicitationRequest {
                                id: snapshot.id.clone(),
                                server_name: snapshot.server_name.clone(),
                                kind: snapshot.kind,
                            },
                        );
                        if engine_tx_for_elicit.send(fe).await.is_err() {
                            break; // engine shut down
                        }
                        // Fire OnElicitation hook
                        crate::hooks::fire_async(
                            crate::hooks::HookPoint::OnElicitation,
                            &crate::hooks::HookContext::for_session("<mcp-elicitation>")
                                .with_extra("server_name", snapshot.server_name.clone())
                                .with_extra("elicitation_id", snapshot.id.clone()),
                        );
                        // Also fire the unified OnUserInputRequired hook —
                        // elicitation blocks the turn until the user responds.
                        crate::hooks::fire_async(
                            crate::hooks::HookPoint::OnUserInputRequired,
                            &crate::hooks::HookContext::for_session("<mcp-elicitation>")
                                .with_extra("kind", "elicitation")
                                .with_extra(
                                    "message",
                                    format!(
                                        "MCP server '{}' is requesting structured input",
                                        snapshot.server_name
                                    ),
                                ),
                        );
                    }
                    jfc_core::mcp_elicitation::ElicitationEvent::Resolved {
                        id,
                        server_name,
                        mode,
                        action,
                    } => {
                        // Fire OnElicitationResult hook
                        crate::hooks::fire_async(
                            crate::hooks::HookPoint::OnElicitationResult,
                            &crate::hooks::HookContext::for_session("<mcp-elicitation>")
                                .with_extra("server_name", server_name)
                                .with_extra("elicitation_id", id)
                                .with_extra("mode", mode)
                                .with_extra("action", action),
                        );
                    }
                }
            }
        });
        Self {
            state: EngineState::new(provider, model),
            tx,
        }
    }

    /// A clone of the event sender for detached producers (remote hosts,
    /// schedulers, background workers).
    pub fn sender(&self) -> EventSender {
        self.tx.clone()
    }

    /// Dispatch one engine event. Returns the directives the engine cannot
    /// interpret on its own (prompt submission, slash commands) for the
    /// frontend to act on.
    pub async fn handle_event(
        &mut self,
        ev: EngineEvent,
    ) -> anyhow::Result<Option<FrontendDirective>> {
        runtime::handle_engine_event(&mut self.state, &self.tx, ev).await
    }

    /// Submit a user prompt (hooks, mention resolution, compaction gate,
    /// message push, stream spawn). The frontend pre-processes its own
    /// surface first (paste expansion, staged attachments, edit cursors).
    pub async fn submit(
        &mut self,
        text: String,
        attachments: Vec<jfc_core::Attachment>,
        edit_at: Option<usize>,
    ) -> anyhow::Result<runtime::ops::SubmitOutcome> {
        runtime::ops::submit_prompt(&mut self.state, &self.tx, text, attachments, edit_at).await
    }

    /// Start a turn over an externally seeded transcript (session resume,
    /// stream-json input).
    pub async fn start_turn_from_transcript(&mut self, turn_text: &str) {
        runtime::ops::start_turn_from_transcript(&mut self.state, &self.tx, turn_text).await;
    }

    /// Interrupt the current turn: cancel the stream, abort in-flight tools,
    /// deny pending approvals, kill bash subprocesses.
    pub fn interrupt(&mut self) {
        runtime::ops::interrupt(&mut self.state, &self.tx);
    }

    /// Load a session by id; view-side resets ride on the
    /// [`EngineEffect::SessionSwitched`] effect.
    pub async fn load_session(&mut self, id: jfc_core::SessionId) {
        runtime::ops::load_session(&mut self.state, id).await;
    }

    /// Resolve a parked tool approval by id (modal keys, remote control,
    /// headless permission policies all funnel here).
    pub fn resolve_approval(&mut self, tool_use_id: String, approved: bool) {
        runtime::approvals::handle_remote_approval_response(
            &mut self.state,
            &self.tx,
            tool_use_id,
            approved,
        );
    }

    /// Drain the view-facing effects queued since the last call. Headless
    /// frontends typically drop these; interactive ones map them onto
    /// scroll/cache/picker reactions.
    pub fn drain_effects(&mut self) -> Vec<EngineEffect> {
        std::mem::take(&mut self.state.effects)
    }

    /// True when no turn is running and nothing is parked: no live stream,
    /// no in-flight tools, no pending approvals, no queued prompts, no
    /// compaction. The standard headless termination check.
    pub fn is_idle(&self) -> bool {
        !self.state.has_interruptible_work()
            && self.state.pending_approval.is_none()
            && self.state.approval_queue.is_empty()
            && self.state.queued_prompts.is_empty()
            && self.state.compacting_started_at.is_none()
    }
}

//! `ProviderEvent::*` handlers. All seven variants are simple state
//! assignments (no `tx.send` fan-out, no spawned tasks), so they read
//! best as one file rather than scattered match arms.

use crate::app::{EngineEffect, EngineState};
use crate::runtime::ProviderEvent;

pub(crate) fn handle_provider_event(state: &mut EngineState, ev: ProviderEvent) {
    match ev {
        ProviderEvent::McpUpdated { servers } => {
            state.mcp_servers = servers;
        }
        ProviderEvent::LspUpdated { servers } => {
            state.lsp_servers = servers;
        }
        ProviderEvent::DiagnosticsUpdated { entries } => {
            // Mirror the snapshot into the global so `stream_response`
            // can inject diagnostics into the system prompt without
            // having to touch every call site to thread through an
            // `&[DiagnosticEntry]` parameter.
            crate::diagnostics::set_global_snapshot(entries.clone());
            state.diagnostics = entries;
            // Toast-on-transition was disabled by user request — the
            // dim summary row above the spinner already surfaces the
            // count, and Ctrl+O opens the full panel. Spawning a
            // separate toast on launch (when cargo-check produced
            // its initial set) doubled the noise. The transition
            // toast is intentionally left commented out rather than
            // deleted so it can be reinstated behind a setting if
            // wanted later.
            // let was_empty = state.diagnostics.is_empty();
            // let is_empty = entries.is_empty();
            // ...
        }
        ProviderEvent::ModelsLoaded { provider, models } => {
            state.provider_models.insert(provider, models);
            state.sync_selected_context_window();
            // The model-picker refresh (query cache, open-picker reload) is
            // view state — the frontend applies it when draining this effect.
            state.push_effect(EngineEffect::ModelsRefreshed);
        }
        ProviderEvent::ProfileLoaded {
            seat_tier,
            subscription_type,
            email,
        } => {
            state.seat_tier = seat_tier;
            state.subscription_type = subscription_type;
            state.account_email = email;
            state.push_effect(EngineEffect::ModelsRefreshed);
        }
        ProviderEvent::AnthropicSnapshotUpdated { snapshot } => {
            state.anthropic_account_snapshot = snapshot;
        }
        ProviderEvent::ClaudeStatusUpdated(update) => {
            if let Some(snapshot) = update.snapshot {
                state.claude_status = Some(snapshot);
                state.claude_status_error = None;
            } else if let Some(error) = update.error {
                state.claude_status_error = Some(error);
            }
        }
    }
}

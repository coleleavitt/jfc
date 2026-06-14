//! Prompt-rewrite proposal modal: accept / reject / edit.
//!
//! When the over-refusal gate proposes a reworded prompt, the engine emits
//! `EngineEffect::PromptRewriteProposed`, which the event loop stores in
//! `app.pending_rewrite_proposal`. This module renders that decision as a
//! blocking modal so the rewrite is NEVER applied silently (the SPEC
//! "never silent; require confirmation" contract):
//!
//! - **A**ccept / Enter → send the reworded prompt.
//! - **R**eject       → send the user's original prompt unchanged.
//! - **E**dit / Esc   → drop the rewrite into the composer for hand-editing.
//!
//! Ctrl-C falls through to the global interrupt handler.

use crossterm::event::{self, KeyCode, KeyModifiers};
use ratatui_textarea::TextArea;
use tokio::sync::mpsc;

use crate::app::{App, EngineEvent};

/// Route a key to the active rewrite-proposal modal. Returns `true` when a
/// proposal is pending (key consumed), mirroring `handle_question_key`.
pub(super) async fn handle_prompt_rewrite_key(
    app: &mut App,
    key: event::KeyEvent,
    tx: &mpsc::Sender<EngineEvent>,
) -> bool {
    if app.pending_rewrite_proposal.is_none() {
        return false;
    }
    // Let Ctrl-C fall through to the global interrupt handler.
    if key.modifiers.contains(KeyModifiers::CONTROL)
        && matches!(key.code, KeyCode::Char('c') | KeyCode::Char('C'))
    {
        return false;
    }

    match key.code {
        KeyCode::Char('a') | KeyCode::Char('A') | KeyCode::Enter => {
            let proposal = app.pending_rewrite_proposal.take().expect("checked above");
            // Persist the accepted rewrite for experience replay (few-shot
            // exemplars on future pipeline builds).
            jfc_engine::runtime::prompt_rewrite_gate::record_accepted(
                proposal.original_intent.clone(),
                proposal.rewrite.clone(),
                proposal.rationale.clone(),
            );
            send_prompt(proposal.rewrite, tx).await;
        }
        KeyCode::Char('r') | KeyCode::Char('R') => {
            let proposal = app.pending_rewrite_proposal.take().expect("checked above");
            send_prompt(proposal.original, tx).await;
        }
        KeyCode::Char('e') | KeyCode::Char('E') | KeyCode::Esc => {
            // Load the rewrite into the composer for hand-editing; do not send.
            let proposal = app.pending_rewrite_proposal.take().expect("checked above");
            app.textarea = TextArea::from(
                proposal
                    .rewrite
                    .lines()
                    .map(|l| l.to_string())
                    .collect::<Vec<_>>(),
            );
        }
        // Any other key is intentionally swallowed so the modal stays modal
        // (the user must explicitly accept/reject/edit — never type past it).
        other => {
            tracing::trace!(target: "jfc::prompt_rewrite", ?other, "ignored key in rewrite modal");
        }
    }
    true
}

/// Submit a chosen prompt through the normal submit path, re-entering the gate.
/// The gate is idempotent on an accepted rewrite (already scope-bounded), and on
/// the original the user has explicitly chosen to send it as-is.
async fn send_prompt(text: String, tx: &mpsc::Sender<EngineEvent>) {
    let _ = tx
        .send(EngineEvent::Control(
            crate::runtime::ControlEvent::SubmitPrompt(text),
        ))
        .await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{App, PromptRewriteProposal};
    use jfc_provider::{EventStream, ModelInfo, Provider, ProviderMessage, StreamOptions};
    use std::sync::Arc;

    struct TestProvider;
    impl jfc_provider::seal::Sealed for TestProvider {}
    #[async_trait::async_trait]
    impl Provider for TestProvider {
        fn name(&self) -> &str {
            "test"
        }
        fn available_models(&self) -> Vec<ModelInfo> {
            Vec::new()
        }
        async fn stream(
            &self,
            _m: Vec<ProviderMessage>,
            _o: &StreamOptions,
        ) -> anyhow::Result<EventStream> {
            Ok(Box::pin(futures::stream::empty()))
        }
    }

    fn app_with_proposal() -> App {
        let mut app = App::new(Arc::new(TestProvider), "test-model");
        app.pending_rewrite_proposal = Some(PromptRewriteProposal {
            original: "ORIGINAL".into(),
            rewrite: "REWRITE".into(),
            rationale: "removed evasion wording".into(),
            original_intent: "understand classifiers".into(),
        });
        app
    }

    fn key(c: char) -> event::KeyEvent {
        event::KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)
    }

    #[tokio::test]
    async fn no_proposal_is_not_consumed() {
        let mut app = App::new(Arc::new(TestProvider), "test-model");
        let (tx, _rx) = mpsc::channel::<EngineEvent>(8);
        assert!(!handle_prompt_rewrite_key(&mut app, key('a'), &tx).await);
    }

    #[tokio::test]
    async fn accept_sends_rewrite() {
        let mut app = app_with_proposal();
        let (tx, mut rx) = mpsc::channel::<EngineEvent>(8);
        assert!(handle_prompt_rewrite_key(&mut app, key('a'), &tx).await);
        assert!(app.pending_rewrite_proposal.is_none());
        let Ok(EngineEvent::Control(crate::runtime::ControlEvent::SubmitPrompt(t))) = rx.try_recv()
        else {
            panic!("expected SubmitPrompt(REWRITE)");
        };
        assert_eq!(t, "REWRITE");
    }

    #[tokio::test]
    async fn reject_sends_original() {
        let mut app = app_with_proposal();
        let (tx, mut rx) = mpsc::channel::<EngineEvent>(8);
        assert!(handle_prompt_rewrite_key(&mut app, key('r'), &tx).await);
        assert!(app.pending_rewrite_proposal.is_none());
        let Ok(EngineEvent::Control(crate::runtime::ControlEvent::SubmitPrompt(t))) = rx.try_recv()
        else {
            panic!("expected SubmitPrompt(ORIGINAL)");
        };
        assert_eq!(t, "ORIGINAL");
    }

    #[tokio::test]
    async fn edit_loads_composer_without_sending() {
        let mut app = app_with_proposal();
        let (tx, mut rx) = mpsc::channel::<EngineEvent>(8);
        assert!(handle_prompt_rewrite_key(&mut app, key('e'), &tx).await);
        assert!(app.pending_rewrite_proposal.is_none());
        assert_eq!(app.textarea.lines().join("\n"), "REWRITE");
        assert!(rx.try_recv().is_err(), "edit must not send a prompt");
        drop(rx);
    }
}

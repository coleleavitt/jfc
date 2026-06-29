//! Slash handlers: inter-session inbox utilities.

use crate::commands::prelude::*;

pub(super) async fn cmd_inbox(
    state: &mut EngineState,
    parts: &[&str],
    _text: &str,
    _tx: Option<&mpsc::Sender<EngineEvent>>,
) {
    let sub = parts.get(1).map(|s| s.trim()).unwrap_or("");

    // Resolve target session: by default use current, otherwise allow `/inbox <session_id>` or
    // `/inbox clear <session_id>`.
    let mut action = "list";
    let mut target: Option<String> = state
        .current_session_id
        .as_ref()
        .map(|s| s.as_str().to_owned());

    if !sub.is_empty() {
        let mut it = sub.split_whitespace();
        let first = it.next().unwrap_or("");
        if first == "clear" {
            action = "clear";
            target = it.next().map(|s| s.to_owned()).or(target);
        } else {
            // treat as `/inbox <session_id>` → list that inbox
            target = Some(first.to_owned());
        }
    }

    let Some(target_id) = target else {
        state.messages.push(ChatMessage::assistant(
            "No active session. Usage: `/inbox [clear] [<session_id>]`".into(),
        ));
        return;
    };

    if action == "clear" {
        match jfc_session::clear_inbox_for_session(&target_id).await {
            Ok(()) => state.messages.push(ChatMessage::assistant(format!(
                "Cleared inbox for `{}`.",
                target_id
            ))),
            Err(e) => state.messages.push(ChatMessage::assistant(format!(
                "**Error** clearing inbox `{}`: {}",
                target_id, e
            ))),
        }
        return;
    }

    // list
    let msgs = jfc_session::read_inbox_for_session(&target_id).await;
    if msgs.is_empty() {
        state.messages.push(ChatMessage::assistant(format!(
            "Inbox for `{}` is empty.",
            target_id
        )));
        return;
    }
    let mut body = format!(
        "**{}** message(s) in inbox `{}`:\n\n",
        msgs.len(),
        target_id
    );
    for (i, m) in msgs.iter().enumerate() {
        let from = m.from.as_deref().unwrap_or("(unknown)");
        let status = if m.read { "read" } else { "unread" };
        body.push_str(&format!(
            "{}. [{}] from `{}` at {}\n   {}\n",
            i + 1,
            status,
            from,
            m.timestamp,
            m.text
        ));
    }
    state.messages.push(ChatMessage::assistant(body));
}

/// If the target session has pending inter-session messages, append a
/// system-reminder so the model sees them as background context (not fresh user input).
pub(super) async fn inject_inbox_reminder(state: &mut EngineState, session_id: &str) {
    let msgs = jfc_session::read_inbox_for_session(session_id).await;
    if msgs.is_empty() {
        return;
    }
    let mut lines = vec![format!(
        "{} pending inter-session message(s) for this session:",
        msgs.len()
    )];
    for m in msgs.iter().take(10) {
        let from = m.from.as_deref().unwrap_or("(unknown)");
        let preview: String = m.text.chars().take(200).collect();
        lines.push(format!("- from `{}` at {}: {}", from, m.timestamp, preview));
    }
    if msgs.len() > 10 {
        lines.push(format!("... and {} more", msgs.len() - 10));
    }
    let body = crate::system_reminder::format(&lines.join("\n"));
    state.messages.push(ChatMessage::user(body));
}

#[cfg(test)]
mod tests {
    use super::*;

    use futures::stream::empty;
    use jfc_provider::{CompletionResponse, Provider, ProviderMessage, StreamOptions, TokenUsage};
    use std::sync::Arc;
    use std::sync::Mutex;
    use tempfile::TempDir;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    struct NoopProvider;
    impl jfc_provider::seal::Sealed for NoopProvider {}
    #[async_trait::async_trait]
    impl Provider for NoopProvider {
        fn name(&self) -> &str {
            "noop"
        }
        fn available_models(&self) -> Vec<jfc_provider::ModelInfo> {
            Vec::new()
        }
        async fn stream(
            &self,
            _messages: Vec<ProviderMessage>,
            _options: &StreamOptions,
        ) -> anyhow::Result<jfc_provider::EventStream> {
            Ok(Box::pin(empty()))
        }
        async fn complete(
            &self,
            _messages: Vec<ProviderMessage>,
            _options: &StreamOptions,
        ) -> anyhow::Result<CompletionResponse> {
            Ok(CompletionResponse {
                content: String::new(),
                usage: TokenUsage::default(),
                context_signals: None,
                reasoning: None,
            })
        }
    }

    fn test_state() -> EngineState {
        EngineState::new(Arc::new(NoopProvider), "test-model")
    }

    fn set_temp_config_home() -> TempDir {
        let _g = ENV_LOCK.lock().unwrap();
        let dir = TempDir::new().unwrap();
        // Safety: tests are serialized via ENV_LOCK
        unsafe {
            std::env::set_var("XDG_CONFIG_HOME", dir.path());
        }
        dir
    }

    #[serial_test::serial]
    #[tokio::test]
    async fn inbox_list_and_clear_commands_normal() {
        let _home = set_temp_config_home();
        // Seed two messages in target inbox
        jfc_session::clear_inbox_for_session("ses_target")
            .await
            .unwrap();
        jfc_session::write_inbox_message("ses_target", Some("ses_src"), "hello")
            .await
            .unwrap();
        jfc_session::write_inbox_message("ses_target", Some("ses_src"), "world")
            .await
            .unwrap();

        let mut state = test_state();
        state.current_session_id = Some(crate::ids::SessionId::new("ses_target"));

        // List
        cmd_inbox(&mut state, &["/inbox", ""], "/inbox", None).await;
        let last = state.messages.last().unwrap().parts[0].text_only();
        assert!(last.contains("message(s) in inbox `ses_target`"));

        // Clear
        cmd_inbox(&mut state, &["/inbox", "clear"], "/inbox clear", None).await;
        let last = state.messages.last().unwrap().parts[0].text_only();
        assert!(
            last.contains("Cleared inbox for`ses_target`")
                || last.contains("Cleared inbox for `ses_target`")
        );

        let remaining = jfc_session::read_inbox_for_session("ses_target").await;
        assert!(remaining.is_empty());
    }

    #[serial_test::serial]
    #[tokio::test]
    async fn inject_inbox_reminder_pushes_system_reminder_normal() {
        let _home = set_temp_config_home();
        jfc_session::clear_inbox_for_session("ses_r1")
            .await
            .unwrap();
        jfc_session::write_inbox_message("ses_r1", Some("ses_src"), "a message")
            .await
            .unwrap();

        let mut state = test_state();
        inject_inbox_reminder(&mut state, "ses_r1").await;
        let last = state.messages.last().unwrap();
        let text = last.parts[0].text_only();
        assert!(text.contains("<system-reminder>"));
    }
}

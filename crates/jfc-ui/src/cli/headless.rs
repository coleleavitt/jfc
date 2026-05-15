/// `--print` headless one-shot mode. Builds a minimal stream against
/// the active provider, prints text deltas to stdout as they arrive,
/// exits with the stream's stop_reason. No TUI, no session save, no
/// tool dispatch (tools require user approval which is meaningless in
/// headless mode — callers needing tools should drive the TUI).
pub(super) async fn run_print_mode(
    provider: std::sync::Arc<dyn crate::provider::Provider>,
    model: crate::provider::ModelId,
    prompt: String,
) -> anyhow::Result<()> {
    use crate::provider::{
        ProviderContent, ProviderMessage, ProviderRole, StreamEvent, StreamOptions,
    };
    use futures::StreamExt;
    use std::io::Write;

    let messages = vec![ProviderMessage {
        role: ProviderRole::User,
        content: vec![ProviderContent::Text(prompt)],
    }];
    let opts = StreamOptions::new(model.clone()).max_tokens(8192);
    let mut stream = provider
        .stream(messages, &opts)
        .await
        .map_err(|e| anyhow::anyhow!("stream open failed: {e}"))?;
    let mut stdout = std::io::stdout().lock();
    let mut exit_code = 0;
    while let Some(event) = stream.next().await {
        match event {
            Ok(StreamEvent::TextDelta { delta, .. }) => {
                let _ = stdout.write_all(delta.as_bytes());
                let _ = stdout.flush();
            }
            Ok(StreamEvent::Done { .. }) => break,
            Ok(_) => {}
            Err(e) => {
                eprintln!("\n[stream error: {e}]");
                exit_code = 1;
                break;
            }
        }
    }
    let _ = stdout.write_all(b"\n");
    let _ = stdout.flush();
    if exit_code != 0 {
        std::process::exit(exit_code);
    }
    Ok(())
}

/// `--remote-session <id>` entry. Streams events from a managed-agent
/// session to stdout. Minimal first cut — full TUI integration with
/// rendering of v132's 17 event types lives in `managed_session.rs`
/// and ships behind a follow-on flag once the eventer is verified.
pub(super) async fn run_remote_session(
    client: jfc_anthropic_sdk::Client,
    session_id: String,
) -> anyhow::Result<()> {
    use futures::StreamExt;

    let session = crate::managed_session::ManagedSession::new(client, session_id.clone());
    eprintln!("--remote-session: subscribing to session {session_id}");
    let mut stream = session
        .connect()
        .await
        .map_err(|e| anyhow::anyhow!("session connect: {e}"))?;
    while let Some(event) = stream.next().await {
        match event {
            Ok(ev) => {
                println!("{}", crate::managed_session::render_event_line(&ev));
            }
            Err(e) => {
                eprintln!("[stream error: {e}]");
                break;
            }
        }
    }
    Ok(())
}

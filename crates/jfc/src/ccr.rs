//! CCR (Claude Code Remote) — remote session management.
//!
//! Provides a client for spawning and interacting with remote Claude Code
//! sessions hosted by Anthropic's infrastructure. This mirrors the
//! `claude-code-remote` surface from v2.1.144+:
//!
//! - Spawn a remote session against a given environment
//! - Poll status (Running, Idle, Terminated)
//! - Stream live events via SSE
//!
//! The remote session executes in an isolated cloud environment with
//! filesystem, network, and tool access — the local TUI only relays
//! events for rendering and forwards user input.

use std::pin::Pin;
use std::time::Instant;

use futures::stream::Stream;
use serde::{Deserialize, Serialize};

/// Status of a CCR remote session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CcrStatus {
    /// Session is actively executing (model running, tools in-flight).
    Running,
    /// Session is idle — awaiting user input.
    Idle,
    /// Session has terminated (completed, errored, or timed out).
    Terminated,
}

/// Events streamed from a CCR remote session.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CcrEvent {
    /// A text message from the agent.
    Message { content: String, timestamp: String },
    /// The agent is invoking a tool.
    ToolUse {
        tool_name: String,
        tool_input: serde_json::Value,
        tool_use_id: String,
        timestamp: String,
    },
    /// Session status transition.
    StatusChange {
        from: CcrStatus,
        to: CcrStatus,
        timestamp: String,
    },
}

/// A live CCR remote session handle.
#[derive(Debug, Clone)]
pub struct CcrSession {
    /// URL for the session's API endpoint.
    pub session_url: String,
    /// The environment this session is running in.
    pub environment_id: String,
    /// Current known status.
    pub status: CcrStatus,
    /// Session ID returned by the API.
    pub session_id: String,
    /// When the session was created locally.
    pub created_at: Instant,
}

/// Parameters for spawning a remote session.
#[derive(Debug, Clone, Serialize)]
struct SpawnRequest {
    prompt: String,
    environment_id: String,
}

/// API response from session creation.
#[derive(Debug, Clone, Deserialize)]
struct SpawnResponse {
    session_id: String,
    session_url: String,
    status: CcrStatus,
}

/// Spawn a new CCR remote session.
///
/// POSTs to the Anthropic API to create a remote session running the given
/// prompt in the specified environment. Returns a `CcrSession` handle that
/// can be used to poll status or stream events.
pub async fn spawn_remote_session(
    client: &reqwest::Client,
    api_key: &str,
    base_url: &str,
    prompt: String,
    environment_id: String,
) -> anyhow::Result<CcrSession> {
    let url = format!("{base_url}/v1/beta/ccr/sessions");
    let body = SpawnRequest {
        prompt,
        environment_id: environment_id.clone(),
    };

    let resp = client
        .post(&url)
        .header("x-api-key", api_key)
        .header("anthropic-version", "2023-06-01")
        .header("anthropic-beta", "ccr-2026-04-01")
        .json(&body)
        .send()
        .await?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        anyhow::bail!("CCR spawn failed ({}): {}", status, text);
    }

    let spawn: SpawnResponse = resp.json().await?;

    Ok(CcrSession {
        session_url: spawn.session_url,
        environment_id,
        status: spawn.status,
        session_id: spawn.session_id,
        created_at: Instant::now(),
    })
}

/// Poll the current status of a CCR remote session.
pub async fn poll_remote_status(
    client: &reqwest::Client,
    api_key: &str,
    session: &CcrSession,
) -> anyhow::Result<CcrStatus> {
    let url = format!("{}/status", session.session_url);

    let resp = client
        .get(&url)
        .header("x-api-key", api_key)
        .header("anthropic-version", "2023-06-01")
        .header("anthropic-beta", "ccr-2026-04-01")
        .send()
        .await?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        anyhow::bail!("CCR status poll failed ({}): {}", status, text);
    }

    #[derive(Deserialize)]
    struct StatusResponse {
        status: CcrStatus,
    }

    let body: StatusResponse = resp.json().await?;
    Ok(body.status)
}

/// Open an SSE connection to stream live events from a CCR remote session.
///
/// Returns a stream of `CcrEvent`s. The stream ends when the session
/// terminates or the connection drops.
pub async fn stream_remote_events(
    client: &reqwest::Client,
    api_key: &str,
    session: &CcrSession,
) -> anyhow::Result<Pin<Box<dyn Stream<Item = anyhow::Result<CcrEvent>> + Send>>> {
    let url = format!("{}/events/stream", session.session_url);

    let resp = client
        .get(&url)
        .header("x-api-key", api_key)
        .header("anthropic-version", "2023-06-01")
        .header("anthropic-beta", "ccr-2026-04-01")
        .header("accept", "text/event-stream")
        .send()
        .await?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        anyhow::bail!("CCR event stream failed ({}): {}", status, text);
    }

    use futures::StreamExt;
    let frames = jfc_anthropic_sdk::sse::response_event_stream(resp);

    let event_stream = frames.filter_map(|frame| async move {
        match frame {
            Ok(frame) => {
                let data = frame.data.trim();
                if data == "[DONE]" {
                    return None;
                }
                match serde_json::from_str::<CcrEvent>(data) {
                    Ok(event) => Some(Ok(event)),
                    Err(e) => Some(Err(anyhow::anyhow!("decode CcrEvent from SSE frame: {e}"))),
                }
            }
            Err(e) => Some(Err(anyhow::anyhow!(
                "CCR event stream transport error: {e}"
            ))),
        }
    });

    Ok(Box::pin(event_stream))
}

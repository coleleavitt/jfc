//! `jfc rc` — remote-control CLI subcommands.
//!
//! - `jfc rc connect <url> --token <tok>` — connect to a running host's
//!   remote-control server and mirror its session to this terminal. Type a
//!   line to send a prompt; Ctrl-C to disconnect. When a permission or
//!   plan-approval prompt arrives, `y`/`n` sends the appropriate
//!   `ApprovalResponse`/`PlanApprovalResponse` instead of a `UserPrompt`.
//! - `jfc rc status` — report whether a local RC server appears reachable.

use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use clap::Subcommand;

use jfc_remote::auth::{self, SeqTracker};
use jfc_remote::protocol::{RemoteEnvelope, SessionState};

#[derive(Subcommand, Debug)]
pub(super) enum RcSubcommand {
    /// Connect to a running session's remote-control server as a client.
    Connect {
        /// WebSocket URL, e.g. `ws://localhost:4242` (or via a tunnel).
        url: String,
        /// Pairing token printed by the host when remote control started.
        #[arg(long)]
        token: String,
    },
    /// Check whether a remote-control server is reachable at a URL.
    Status {
        /// WebSocket URL to probe.
        #[arg(default_value = "ws://localhost:4242")]
        url: String,
        /// Pairing token (required for the auth handshake).
        #[arg(long)]
        token: Option<String>,
    },
}

pub(super) async fn run_rc_subcommand(sub: RcSubcommand) -> anyhow::Result<()> {
    match sub {
        RcSubcommand::Connect { url, token } => connect_client(&url, &token).await,
        RcSubcommand::Status { url, token } => probe_status(&url, token.as_deref()).await,
    }
}

/// Shared state between the stdin-reader and the event renderer so the
/// reader knows whether to interpret `y`/`n` as an approval or a prompt.
#[derive(Clone)]
struct ClientState {
    awaiting_approval: Arc<AtomicBool>,
    awaiting_plan: Arc<AtomicBool>,
    pending_tool_use_id: Arc<Mutex<String>>,
}

impl ClientState {
    fn new() -> Self {
        Self {
            awaiting_approval: Arc::new(AtomicBool::new(false)),
            awaiting_plan: Arc::new(AtomicBool::new(false)),
            pending_tool_use_id: Arc::new(Mutex::new(String::new())),
        }
    }
}

async fn connect_client(url: &str, token: &str) -> anyhow::Result<()> {
    let (tx, mut rx) = jfc_remote::ws::connect(url, token)
        .await
        .map_err(|e| anyhow::anyhow!("failed to connect: {e}"))?;

    println!("● connected to {url}");
    println!("  type a message + Enter to send · Ctrl-C to disconnect\n");

    let state = ClientState::new();

    // Outbound: stdin lines → envelopes.
    let token_out = token.to_string();
    let state_for_input = state.clone();
    tokio::spawn(async move {
        let mut out_seq: u64 = 1;
        let stdin = tokio::io::BufReader::new(tokio::io::stdin());
        use tokio::io::AsyncBufReadExt;
        let mut lines = stdin.lines();
        while let Ok(Some(line)) = lines.next_line().await {
            let line = line.trim().to_string();
            if line.is_empty() {
                continue;
            }

            let envelope = if line == "/interrupt" {
                RemoteEnvelope::Interrupt
            } else if state_for_input.awaiting_plan.load(Ordering::Relaxed) {
                state_for_input
                    .awaiting_plan
                    .store(false, Ordering::Relaxed);
                let approve = line.eq_ignore_ascii_case("y") || line.eq_ignore_ascii_case("yes");
                let feedback = if approve { None } else { Some(line.clone()) };
                RemoteEnvelope::PlanApprovalResponse { approve, feedback }
            } else if state_for_input.awaiting_approval.load(Ordering::Relaxed) {
                state_for_input
                    .awaiting_approval
                    .store(false, Ordering::Relaxed);
                let approved = line.eq_ignore_ascii_case("y") || line.eq_ignore_ascii_case("yes");
                let tool_use_id = state_for_input.pending_tool_use_id.lock().unwrap().clone();
                RemoteEnvelope::ApprovalResponse {
                    tool_use_id,
                    approved,
                }
            } else {
                RemoteEnvelope::UserPrompt { text: line }
            };

            let frame = auth::build_signed_frame(&token_out, out_seq, envelope);
            out_seq += 1;
            if tx.send(frame).await.is_err() {
                break;
            }
        }
    });

    // Inbound: host frames → stdout.
    let mut seq_tracker = SeqTracker::new();
    while let Some(frame) = rx.recv().await {
        if !auth::verify_frame(token, &frame) {
            eprintln!("⚠ dropped frame (bad HMAC)");
            continue;
        }
        if !seq_tracker.accept(frame.seq) {
            continue;
        }
        render_envelope(&frame.payload, &state);
    }

    println!("\n● disconnected");
    Ok(())
}

/// Render one inbound envelope to stdout.
fn render_envelope(env: &RemoteEnvelope, state: &ClientState) {
    match env {
        RemoteEnvelope::AssistantDelta { text, .. } => {
            if let Some(t) = text {
                print!("{t}");
                std::io::stdout().flush().ok();
            }
        }
        RemoteEnvelope::ToolUse {
            name,
            input_preview,
            ..
        } => {
            let preview = input_preview.as_deref().unwrap_or("");
            println!("\n  ⚙ {name}({preview})");
        }
        RemoteEnvelope::ToolResult {
            is_error,
            output_preview,
            ..
        } => {
            let marker = if *is_error { "✗" } else { "✓" };
            let preview = output_preview.as_deref().unwrap_or("");
            let first_line = preview.lines().next().unwrap_or("");
            println!("  {marker} {first_line}");
        }
        RemoteEnvelope::SessionStatus { status, message } => {
            let label = match status {
                SessionState::Running => "running",
                SessionState::Idle => "idle",
                SessionState::WaitingApproval => "waiting for approval",
                SessionState::Terminated => "terminated",
                SessionState::Error => "error",
            };
            match message {
                Some(m) => println!("\n[{label}] {m}"),
                None => println!("\n[{label}]"),
            }
        }
        RemoteEnvelope::PermissionRequest {
            tool_use_id,
            tool_name,
            summary,
            diff,
        } => render_permission(state, tool_use_id, tool_name, summary, diff.as_deref()),
        RemoteEnvelope::PlanApprovalRequest { plan } => {
            println!("\n📋 plan approval requested:");
            for line in plan.lines().take(40) {
                println!("  {line}");
            }
            println!("   → y to approve, n to reject (then Enter)");
            state.awaiting_plan.store(true, Ordering::Relaxed);
        }
        RemoteEnvelope::Toast { kind, text } => {
            println!("\n[{kind}] {text}");
        }
        RemoteEnvelope::Heartbeat => {}
        other => {
            tracing::debug!(target: "jfc::remote", ?other, "client ignoring host-bound envelope");
        }
    }
}

fn render_permission(
    state: &ClientState,
    tool_use_id: &str,
    tool_name: &str,
    summary: &str,
    diff: Option<&str>,
) {
    println!("\n🔒 permission: {tool_name} — {summary}");
    if let Some(d) = diff {
        for line in d.lines().take(20) {
            println!("  {line}");
        }
        let total = d.lines().count();
        if total > 20 {
            println!("  … {total} lines total");
        }
    }
    println!("   → y to approve, n to reject (then Enter)");
    state.awaiting_approval.store(true, Ordering::Relaxed);
    *state.pending_tool_use_id.lock().unwrap() = tool_use_id.to_string();
}

/// Probe a remote-control server: attempt to connect and report success.
async fn probe_status(url: &str, token: Option<&str>) -> anyhow::Result<()> {
    let Some(token) = token else {
        println!("remote-control status: provide --token to probe {url}");
        return Ok(());
    };
    match jfc_remote::ws::connect(url, token).await {
        Ok(_) => {
            println!("remote-control: reachable at {url} ✓");
            Ok(())
        }
        Err(e) => {
            println!("remote-control: unreachable at {url} — {e}");
            Ok(())
        }
    }
}

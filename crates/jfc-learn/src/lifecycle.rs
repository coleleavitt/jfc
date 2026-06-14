//! Post-session learning lifecycle hooks.
//!
//! Fires the jfc-learn historian at session end to extract facts from the
//! transcript. Best-effort: failures are logged, never surfaced to the user.

use tracing::{debug, warn};

use jfc_core::ChatMessage;

/// Called on session start to process any pending historian transcripts
/// from previous sessions. Runs the dreamer cycle (consolidation, archival).
/// Best-effort: failures are logged.
pub fn on_session_start(cwd: &str) {
    let pending_dir = std::path::Path::new(cwd)
        .join(".jfc")
        .join("learn")
        .join("pending");

    if !pending_dir.exists() {
        return;
    }

    let entries: Vec<_> = match std::fs::read_dir(&pending_dir) {
        Ok(rd) => rd
            .filter_map(|e| e.ok())
            .filter(|e| e.path().extension().is_some_and(|ext| ext == "json"))
            .collect(),
        Err(_) => return,
    };

    if entries.is_empty() {
        return;
    }

    debug!(
        target: "jfc::learn",
        pending_count = entries.len(),
        "on_session_start: found pending transcripts"
    );

    // We can't run the full historian here (needs LLM), but we log the
    // presence so the dreamer knows to schedule historization. The actual
    // processing will happen when `execute_learn_historize` is called by
    // the agent or the dreamer fires during idle time.
}

/// Called after the main event loop exits. Extracts the transcript into
/// (role, content) tuples and queues it for the historian to process on
/// next session start (since the LLM provider is unavailable at exit time).
pub fn on_session_end(messages: &[ChatMessage], cwd: &str) {
    let transcript = build_transcript(messages);
    if transcript.is_empty() {
        debug!(target: "jfc::learn", "on_session_end: empty transcript, skipping");
        return;
    }
    if transcript.len() < 4 {
        debug!(target: "jfc::learn", turns = transcript.len(), "on_session_end: too few turns, skipping");
        return;
    }

    debug!(
        target: "jfc::learn",
        turns = transcript.len(),
        cwd,
        "on_session_end: queuing transcript for historian"
    );

    let pending_dir = std::path::Path::new(cwd)
        .join(".jfc")
        .join("learn")
        .join("pending");

    if let Err(e) = std::fs::create_dir_all(&pending_dir) {
        warn!(target: "jfc::learn", error = %e, "on_session_end: failed to create pending dir");
        return;
    }

    let timestamp = chrono::Utc::now().format("%Y%m%d_%H%M%S").to_string();
    let pending_path = pending_dir.join(format!("{timestamp}.json"));

    match serde_json::to_string(&transcript) {
        Ok(json) => {
            if let Err(e) = std::fs::write(&pending_path, json) {
                warn!(target: "jfc::learn", error = %e, path = %pending_path.display(), "on_session_end: failed to write pending transcript");
            } else {
                debug!(target: "jfc::learn", path = %pending_path.display(), "on_session_end: queued transcript for historian");
            }
        }
        Err(e) => {
            warn!(target: "jfc::learn", error = %e, "on_session_end: failed to serialize transcript");
        }
    }
}

/// Convert ChatMessages to (role, content) tuples for the historian.
fn build_transcript(messages: &[ChatMessage]) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for msg in messages {
        let role = msg.role.to_string();
        let content: String = msg
            .parts
            .iter()
            .map(|p| p.text_only())
            .collect::<Vec<_>>()
            .join("\n");
        if !content.is_empty() {
            out.push((role, content));
        }
    }
    out
}

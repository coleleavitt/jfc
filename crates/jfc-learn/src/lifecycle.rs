//! Post-session learning lifecycle hooks.
//!
//! Fires the jfc-learn historian at session end to extract facts from the
//! transcript. Best-effort: failures are logged, never surfaced to the user.

use tracing::{debug, warn};

use jfc_core::ChatMessage;

const LEARN_PENDING_TRANSCRIPT_KIND: &str = "learn_pending_transcript";

fn project_session_id(cwd: &str) -> String {
    format!(
        "project:{}",
        jfc_knowledge::project_key(std::path::Path::new(cwd))
    )
}

/// Called on session start to process any pending historian transcripts
/// from previous sessions. Runs the dreamer cycle (consolidation, archival).
/// Best-effort: failures are logged.
pub async fn on_session_start(cwd: &str) {
    let _linkscope_start = linkscope::phase("learn.lifecycle.session_start");
    linkscope::event_fields(
        "learn.lifecycle.session_start",
        [linkscope::TraceField::text("cwd", cwd.to_owned())],
    );
    let pending_count = match jfc_knowledge::KnowledgeStore::open_default().await {
        Ok(store) => {
            match store
                .list_session_artifacts(
                    &project_session_id(cwd),
                    LEARN_PENDING_TRANSCRIPT_KIND,
                    10_000,
                )
                .await
            {
                Ok(rows) => rows.len(),
                Err(_) => 0,
            }
        }
        Err(_) => 0,
    };

    if pending_count == 0 {
        linkscope::event_fields(
            "learn.lifecycle.session_start.result",
            [linkscope::TraceField::count("pending", 0)],
        );
        return;
    }

    debug!(
        target: "jfc::learn",
        pending_count,
        "on_session_start: found pending transcripts"
    );
    linkscope::event_fields(
        "learn.lifecycle.session_start.result",
        [linkscope::TraceField::count(
            "pending",
            u64::try_from(pending_count).unwrap_or(u64::MAX),
        )],
    );

    // We can't run the full historian here (needs LLM), but we log the
    // presence so the dreamer knows to schedule historization. The actual
    // processing will happen when `execute_learn_historize` is called by
    // the agent or the dreamer fires during idle time.
}

/// Called after the main event loop exits. Extracts the transcript into
/// (role, content) tuples and queues it for the historian to process on
/// next session start (since the LLM provider is unavailable at exit time).
pub async fn on_session_end(messages: &[ChatMessage], cwd: &str) {
    let _linkscope_end = linkscope::phase("learn.lifecycle.session_end");
    linkscope::event_fields(
        "learn.lifecycle.session_end",
        [
            linkscope::TraceField::count(
                "messages",
                u64::try_from(messages.len()).unwrap_or(u64::MAX),
            ),
            linkscope::TraceField::text("cwd", cwd.to_owned()),
        ],
    );
    let transcript = build_transcript(messages);
    if transcript.is_empty() {
        linkscope::event_fields(
            "learn.lifecycle.session_end.result",
            [linkscope::TraceField::text("status", "empty_transcript")],
        );
        debug!(target: "jfc::learn", "on_session_end: empty transcript, skipping");
        return;
    }
    if transcript.len() < 4 {
        linkscope::event_fields(
            "learn.lifecycle.session_end.result",
            [
                linkscope::TraceField::text("status", "too_few_turns"),
                linkscope::TraceField::count(
                    "turns",
                    u64::try_from(transcript.len()).unwrap_or(u64::MAX),
                ),
            ],
        );
        debug!(target: "jfc::learn", turns = transcript.len(), "on_session_end: too few turns, skipping");
        return;
    }

    debug!(
        target: "jfc::learn",
        turns = transcript.len(),
        cwd,
        "on_session_end: queuing transcript for historian"
    );

    let timestamp = chrono::Utc::now().format("%Y%m%d_%H%M%S").to_string();

    match serde_json::to_string(&transcript) {
        Ok(json) => {
            let key = format!("{timestamp}-{}", uuid::Uuid::new_v4());
            let session_id = project_session_id(cwd);
            match jfc_knowledge::KnowledgeStore::open_default().await {
                Ok(store) => {
                    match store
                        .upsert_session_artifact(
                            &session_id,
                            LEARN_PENDING_TRANSCRIPT_KIND,
                            &key,
                            &json,
                        )
                        .await
                    {
                        Ok(_) => {
                            linkscope::event_fields(
                                "learn.lifecycle.session_end.result",
                                [
                                    linkscope::TraceField::text("status", "queued"),
                                    linkscope::TraceField::text("key", key.clone()),
                                    linkscope::TraceField::bytes(
                                        "json_bytes",
                                        u64::try_from(json.len()).unwrap_or(u64::MAX),
                                    ),
                                ],
                            );
                            debug!(target: "jfc::learn", key, "on_session_end: queued transcript for historian");
                        }
                        Err(e) => {
                            warn!(target: "jfc::learn", error = %e, key, "on_session_end: failed to persist pending transcript");
                        }
                    }
                }
                Err(e) => {
                    warn!(target: "jfc::learn", error = %e, key, "on_session_end: failed to open knowledge store");
                }
            }
        }
        Err(e) => {
            warn!(target: "jfc::learn", error = %e, "on_session_end: failed to serialize transcript");
        }
    }
}

/// Convert ChatMessages to (role, content) tuples for the historian.
fn build_transcript(messages: &[ChatMessage]) -> Vec<(String, String)> {
    let _linkscope_build = linkscope::phase("learn.lifecycle.build_transcript");
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
    linkscope::event_fields(
        "learn.lifecycle.build_transcript.result",
        [linkscope::TraceField::count(
            "turns",
            u64::try_from(out.len()).unwrap_or(u64::MAX),
        )],
    );
    out
}

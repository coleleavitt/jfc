//! jfc-engine — the frontend-neutral agentic runtime extracted from the jfc
//! TUI binary: conversation/turn state (`EngineState`), the event bus
//! (`EngineEvent`), the dispatch pump (`runtime::handle_engine_event`), the
//! engine verbs (`runtime::ops`), streaming, tool execution, compaction,
//! sessions, swarm/teams, workflows, hooks, and service integrations.
//!
//! Invariant: this crate must never depend on ratatui/crossterm or any
//! frontend state. Frontends (TUI, headless print mode, SDK bridge, remote
//! control, daemon workers) drive the engine exclusively through
//! `EngineState` + `ops` + `handle_engine_event`, and apply `EngineEffect`s
//! their own way.
//!
//! Module shape note: the tree intentionally mirrors the binary it was
//! extracted from (stage 5) so history stays traceable; stage 6/7 flatten
//! the names.

pub mod access_policy;
pub mod advisor;
pub mod agentic_vocabulary;
pub mod agents;
pub mod app;
pub mod atomic_write;
pub mod attachments;
pub mod auth;
pub mod auto_classifier;
pub mod auto_mode;
pub mod auto_review;
pub mod autonomous_loop;
pub mod bash_processes;
pub mod bridge_attestation;
pub mod cache_lineage;
pub mod ccr;
pub mod changeset;
pub mod claude_status;
pub mod coach;
pub mod command_spec;
pub mod commands;
pub mod compact;
pub mod compact_archive;
pub mod config;
pub mod context;
pub mod cost;
pub mod council;
pub mod council_directives;
pub mod council_session;
pub mod daemon;
pub mod diagnostics;
pub mod diagnostics_producer;
pub mod document_formats;
pub mod dreamer_scheduler;
pub mod effort;
pub mod engine;
pub mod env_context;
pub mod exploration;
pub mod external_agent;
pub mod feature_gates;
pub mod file_checkpoint;
pub mod git_context;
pub mod github;
pub mod goal;
pub mod guards;
#[cfg(feature = "hashline")]
pub mod hashline;
pub mod headless;
#[cfg(feature = "hooks")]
pub mod hooks;
/// No-op hooks facade for feature-off builds — API-identical to the gated
/// module so call sites need no cfg-gating.
#[cfg(not(feature = "hooks"))]
pub mod hooks {
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum HookPoint {
        OnUserPromptSubmit,
        BeforeStream,
        AfterStream,
        OnHeartbeat,
        // CC 2.1.167 additions
        OnSetup,
        OnUserPromptExpansion,
        OnMessageDisplay,
        OnElicitation,
        OnElicitationResult,
        PostToolBatch,
        PostCompact,
        SubagentStart,
        WorktreeCreate,
        WorktreeRemove,
        ConfigChange,
        StopFailure,
        // pre-existing additional points kept for no-op parity
        BeforeToolDispatch,
        AfterToolDispatch,
        PostToolUseFailure,
        SubagentStop,
        Stop,
        OnSessionStart,
        OnSessionEnd,
        BeforeCompact,
        AfterCompact,
        OnPermissionRequest,
        OnPermissionGranted,
        OnPermissionDenied,
        OnFileChanged,
        OnCwdChanged,
        OnAgentSpawned,
        OnAgentTerminated,
        OnTeammateIdle,
        OnMessageSent,
        OnMessageReceived,
        OnConfigChanged,
        OnInstructionsLoaded,
        OnMemoryCreated,
        OnMemoryDeleted,
        OnTaskCreated,
        OnTaskCompleted,
        OnToolError,
        OnToolApproval,
        BeforeToolBatch,
        AfterToolBatch,
        OnModelResponse,
        // New in hook-surface expansion v2
        OnUserInterrupt,
        OnUserInputRequired,
    }

    pub struct HookContext;

    impl HookContext {
        pub fn for_session(_session_id: impl AsRef<str>) -> Self {
            Self
        }
        pub fn for_tool(_tool_name: &str, _tool_input: &str, _session_id: impl AsRef<str>) -> Self {
            Self
        }
        pub fn for_agent(_agent_name: &str, _session_id: impl AsRef<str>) -> Self {
            Self
        }
        pub fn for_file(_file_path: &str, _session_id: impl AsRef<str>) -> Self {
            Self
        }
        #[must_use]
        pub fn with_extra(self, _key: impl Into<String>, _value: impl Into<String>) -> Self {
            self
        }
    }

    pub enum HookAction {
        Continue,
        Abort(String),
    }

    /// No-op metrics stub (feature = "hooks" is off).
    #[derive(Debug, Clone, Default)]
    pub struct HookMetrics {
        pub fire_count: u64,
        pub last_fired_at: Option<std::time::SystemTime>,
        pub total_duration_ms: u64,
    }

    pub fn fire(_point: HookPoint, _ctx: &HookContext) -> HookAction {
        HookAction::Continue
    }

    pub fn fire_async(_point: HookPoint, _ctx: &HookContext) {}

    pub fn has_hooks(_point: HookPoint) -> bool {
        false
    }

    /// No-op — returns empty map when hooks feature is disabled.
    pub fn metrics_snapshot() -> std::collections::HashMap<String, HookMetrics> {
        std::collections::HashMap::new()
    }

    /// No-op — returns empty vec when hooks feature is disabled.
    pub fn registered_hooks_summary() -> Vec<(HookPoint, usize)> {
        Vec::new()
    }

    pub struct HookRegistry;

    pub fn default_registry() -> HookRegistry {
        HookRegistry
    }

    pub fn init_global(_registry: HookRegistry) {}
}
pub mod idle_prefetch;
pub mod ids;
pub mod inline_tools;
#[cfg(feature = "intent-gate")]
pub mod intent;
pub mod interaction_mode;
pub mod keywords;
pub mod learn_lifecycle;
pub mod lsp_client;
pub mod lsp_rpc;
pub mod managed_session;
pub mod mcp;
pub mod mcp_elicitation;
pub mod memory;
pub mod memory_recall;
pub mod notifications;
pub mod output_style;
#[cfg(feature = "permission-automation")]
pub mod permissions;
pub mod plan;
pub mod plan_dreamer;
pub mod plan_recall;
pub mod prompt_context_cache;
pub mod prompt_executor;
pub mod proof_oracles;
pub mod providers;
pub mod push_notifications;
pub mod remote_host;
pub mod research;
pub mod response_processor;
pub mod review;
pub mod runtime;
pub mod rust_lex;
pub mod sandbox;
pub mod scaffold_detector;
pub mod scheduler;
pub mod sdk_bridge;
pub mod session;
pub mod session_naming;
pub mod session_recap;
pub mod slate;
pub mod slop_guard;
pub mod speculation;
pub mod sprint;
pub mod stream;
pub mod swarm;
pub mod system_reminder;
pub mod team_onboarding;
pub mod toast;
pub mod tools;
pub mod total_tokens_reminder;
pub mod ultraplan;
pub mod web_cache;
pub mod web_search;
pub mod workflows;
pub mod worktrees;

// ── Curated top-level surface ────────────────────────────────────────────────
// The blessed embedding API. Module internals are also public (stage-5
// blanket publication for the extraction); prefer these names — internals
// may re-privatize as the API settles.
pub use app::{EngineEffect, EngineState, PendingApproval, PendingQuestion, PermissionMode};
pub use engine::{Engine, channel};

/// Mirror a session header into the `jfc-knowledge` session index (PLAN TODO 22).
/// ADDITIVE dual-write: the JSON file stays the canonical transcript; this only
/// updates a queryable index. Best-effort and silent on error — a failed index
/// write must never affect session saving. Runs the blocking SQLite work inline
/// (callers already invoke it off the hot path / after the atomic JSON write).
#[allow(clippy::too_many_arguments)]
pub fn index_session(
    id: &str,
    cwd: Option<&str>,
    model: Option<&str>,
    created_at: Option<&str>,
    updated_at: Option<&str>,
    first_prompt: Option<&str>,
    title: Option<&str>,
    message_count: i64,
) {
    let row = jfc_knowledge::SessionRow {
        id: id.to_owned(),
        cwd: cwd.map(str::to_owned),
        model: model.map(str::to_owned),
        created_at: created_at.map(str::to_owned),
        updated_at: updated_at.map(str::to_owned),
        first_prompt: first_prompt.map(str::to_owned),
        title: title.map(str::to_owned),
        message_count,
    };
    match jfc_knowledge::KnowledgeStore::open_default().and_then(|s| s.upsert_session(&row)) {
        Ok(()) => {}
        Err(e) => tracing::debug!(
            target: "jfc::knowledge",
            session_id = id,
            error = %e,
            "session index upsert skipped (JSON remains canonical)"
        ),
    }
}

/// Map serialized session messages → the knowledge crate's `SessionMessage`
/// (text flattened for FTS, full message JSON kept verbatim in `meta` for a
/// lossless round trip). Borrowing, so no `Clone` on the serialized type tree.
pub(crate) fn to_session_messages(
    serialized_messages: &[crate::session::serialization::SerializedMessage],
) -> Vec<jfc_knowledge::SessionMessage> {
    serialized_messages
        .iter()
        .enumerate()
        .map(|(i, m)| {
            let content = m
                .parts
                .iter()
                .filter_map(|p| match p {
                    crate::session::serialization::SerializedPart::Text { content } => {
                        Some(content.as_str())
                    }
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join("\n");
            jfc_knowledge::SessionMessage {
                seq: i as i64,
                role: m.role.clone(),
                content,
                meta: serde_json::to_string(m).ok(),
            }
        })
        .collect()
}

/// Shadow-write a session's full transcript into the DB (PLAN TODO 23). ADDITIVE:
/// the JSON file stays canonical; this mirrors the messages into the
/// `session_messages` table so a future read-flip (gated on the parity verifier)
/// can serve resume/search from the DB. Best-effort and silent on error — never
/// affects the save.
pub fn shadow_session_transcript(
    row: jfc_knowledge::SessionRow,
    messages: Vec<jfc_knowledge::SessionMessage>,
) {
    let id = row.id.clone();
    match jfc_knowledge::KnowledgeStore::open_default()
        .and_then(|mut s| s.replace_transcript(&row, &messages))
    {
        Ok(()) => {}
        Err(e) => tracing::debug!(
            target: "jfc::knowledge",
            session_id = id,
            error = %e,
            "session transcript shadow-write skipped (JSON remains canonical)"
        ),
    }
}

/// Result of the session→DB parity verifier (PLAN TODO 23 / F8). The flip gate
/// (council decision 2) is `mismatch == 0` among deserializable sessions, with a
/// recorded disposition for every `undeserializable` one.
#[derive(Debug, Default, Clone)]
pub struct SessionParityReport {
    pub checked: usize,
    pub passed: usize,
    pub mismatched: Vec<String>,
    pub undeserializable: Vec<String>,
}

impl SessionParityReport {
    /// Safe to flip reads to the DB: every deserializable session matched.
    pub fn flip_safe(&self) -> bool {
        self.mismatched.is_empty() && self.checked > 0
    }
}

/// Backfill the DB transcript store from the canonical JSON sessions AND verify
/// parity (council decision 2). Reads every `ses_*.json`, shadow-writes its
/// transcript, then reloads from the DB and asserts the canonicalized message
/// stream (role + text per seq, message count) matches. Sessions whose JSON the
/// current reader can't deserialize are bucketed as `undeserializable` (already
/// dead — excluded from the mismatch denominator), never silently dropped.
/// READ-ONLY w.r.t. the JSON files; report-only — performs no read flip.
pub fn backfill_and_verify_sessions(sessions_dir: &std::path::Path) -> SessionParityReport {
    use crate::session::serialization::SerializedSession;
    let mut report = SessionParityReport::default();
    let Ok(entries) = std::fs::read_dir(sessions_dir) else {
        return report;
    };
    let Ok(store) = jfc_knowledge::KnowledgeStore::open_default() else {
        return report;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
        if !name.starts_with("ses_") || !name.ends_with(".json") || name.contains("goal") {
            continue;
        }
        report.checked += 1;
        let id = name.trim_end_matches(".json").to_owned();
        let Ok(raw) = std::fs::read_to_string(&path) else {
            report.undeserializable.push(id);
            continue;
        };
        let Ok(session) = serde_json::from_str::<SerializedSession>(&raw) else {
            report.undeserializable.push(id);
            continue;
        };
        let row = jfc_knowledge::SessionRow {
            id: session.id.clone(),
            cwd: session.cwd.clone(),
            model: session.model.clone(),
            created_at: Some(session.created_at.clone()),
            updated_at: session.updated_at.clone(),
            first_prompt: session.first_prompt.clone(),
            title: session.title.clone(),
            message_count: session.messages.len() as i64,
        };
        // FULL-TREE parity (not just text): the DB stores each message's verbatim
        // serialized JSON in `meta`, so the expected stream is (role, full message
        // JSON) per message — this covers tool parts, diff hunks, usage, and
        // created_at, not only Text. A future resume rebuilds messages by
        // deserializing `meta`, so meta-equality is exactly the fidelity the read
        // flip needs.
        let expected: Vec<(String, Option<String>)> = session
            .messages
            .iter()
            .map(|m| (m.role.clone(), serde_json::to_string(m).ok()))
            .collect();

        shadow_session_transcript(row, to_session_messages(&session.messages));

        // Reload from the DB and compare the full per-message JSON stream.
        let actual: Vec<(String, Option<String>)> = match store.load_transcript(&session.id) {
            Ok(msgs) => msgs.into_iter().map(|m| (m.role, m.meta)).collect(),
            Err(_) => {
                report.mismatched.push(id);
                continue;
            }
        };
        if actual == expected {
            report.passed += 1;
        } else {
            report.mismatched.push(id);
        }
    }
    report
}

/// Run one autonomous cross-project knowledge maintenance pass (import + mine +
/// consolidate + auto-promote). Thin re-export so UI/binary crates don't depend
/// on `jfc-knowledge` directly. Returns the maintenance summary.
pub fn knowledge_maintain(
    project_root: &std::path::Path,
    sessions_dir: Option<&std::path::Path>,
    user_memory_dir: Option<&std::path::Path>,
    project_memory_dir: Option<&std::path::Path>,
) -> jfc_knowledge::Result<jfc_knowledge::MaintainReport> {
    jfc_knowledge::auto_maintain(
        project_root,
        sessions_dir,
        user_memory_dir,
        project_memory_dir,
    )
}
pub use runtime::{
    ControlEvent, EngineEvent, EventReceiver, EventSender, FrontendDirective, FrontendEvent,
    handle_engine_event, ops,
};

/// Domain-type facade mirroring the binary's historical `crate::types`
/// surface (canonical definitions live in jfc-core).
pub mod types {
    pub use jfc_core::ToolInputError;
    pub use jfc_core::{
        ExecutionStatus, ModelUsage, ReplacementMode, TaskInput, TaskLifecycle, TaskStatusPart,
        ToolInput, ToolKind, ToolStatus,
    };

    // Module paths preserved for `crate::types::tool_call::X`-style imports.
    pub use jfc_core::{diff, tool_call, tool_display, tool_output};

    pub use jfc_core::diff::*;
    pub use jfc_core::tool_call::{InvalidToolTransition, ToolCall, ToolUndoEntry};
    pub use jfc_core::tool_display::ToolDisplayState;
    pub use jfc_core::tool_output::{LargeText, ToolOutput, format_server_tool_result_text_public};

    // Former `message.rs` / `status.rs` / `tool.rs` glob surfaces.
    pub use jfc_core::{
        ChatMessage, LspServerInfo, LspStatus, McpServerInfo, McpStatus, MessagePart, Role,
        TurnInvariantError, merge_consecutive_text_parts, sample_tool_harness_message,
        validate_turn_invariants, validate_turn_invariants_inner,
    };
}

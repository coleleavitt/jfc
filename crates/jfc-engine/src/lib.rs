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
pub mod context_accounting;
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

    #[derive(Debug, Clone)]
    pub struct HookContext {
        pub tool_name: String,
        pub tool_input: String,
        pub session_id: String,
        pub intent: Option<String>,
        pub file_path: Option<String>,
        pub agent_name: Option<String>,
        pub extra: Vec<(String, String)>,
        pub env_vars: Vec<(String, String)>,
    }

    impl HookContext {
        pub fn for_session(session_id: impl AsRef<str>) -> Self {
            Self {
                tool_name: String::new(),
                tool_input: String::new(),
                session_id: session_id.as_ref().to_string(),
                intent: None,
                file_path: None,
                agent_name: None,
                extra: Vec::new(),
                env_vars: Vec::new(),
            }
        }
        pub fn for_tool(tool_name: &str, tool_input: &str, session_id: impl AsRef<str>) -> Self {
            Self {
                tool_name: tool_name.to_string(),
                tool_input: tool_input.to_string(),
                session_id: session_id.as_ref().to_string(),
                intent: None,
                file_path: None,
                agent_name: None,
                extra: Vec::new(),
                env_vars: Vec::new(),
            }
        }
        pub fn for_agent(agent_name: &str, session_id: impl AsRef<str>) -> Self {
            let mut ctx = Self::for_session(session_id);
            ctx.agent_name = Some(agent_name.to_string());
            ctx
        }
        pub fn for_file(file_path: &str, session_id: impl AsRef<str>) -> Self {
            let mut ctx = Self::for_session(session_id);
            ctx.file_path = Some(file_path.to_string());
            ctx
        }
        #[must_use]
        pub fn with_extra(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
            self.extra.push((key.into(), value.into()));
            self
        }
    }

    #[derive(Debug, Clone)]
    pub struct HookMetadata {
        pub key: String,
        pub value: String,
    }

    #[derive(Debug, Clone)]
    pub enum HookAction {
        Continue,
        Skip,
        Replace(String),
        Abort(String),
        Emit(HookMetadata),
    }

    #[derive(Debug, Clone)]
    pub enum HookHandler {
        Logger,
        PermissionCheck,
        IntentEnricher,
        CommentChecker,
        ShellCommand {
            command: String,
        },
        Shell {
            command: String,
            async_mode: bool,
            matcher: Option<String>,
        },
        Custom {
            name: String,
            action: HookAction,
        },
    }

    impl HookHandler {
        pub fn execute(&self, _point: HookPoint, _ctx: &HookContext) -> HookAction {
            match self {
                Self::Custom { action, .. } => action.clone(),
                _ => HookAction::Continue,
            }
        }
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

    #[derive(Default)]
    pub struct HookRegistry {
        hooks: Vec<(HookPoint, HookHandler)>,
    }

    impl HookRegistry {
        pub fn new() -> Self {
            Self { hooks: Vec::new() }
        }

        pub fn register(&mut self, point: HookPoint, handler: HookHandler) {
            self.hooks.push((point, handler));
        }

        pub fn register_multi(&mut self, points: &[HookPoint], handler: HookHandler) {
            for &point in points {
                self.register(point, handler.clone());
            }
        }

        pub fn fire(&self, point: HookPoint, ctx: &HookContext) -> HookAction {
            for (hook_point, handler) in &self.hooks {
                if *hook_point == point {
                    match handler.execute(point, ctx) {
                        HookAction::Continue | HookAction::Emit(_) => {}
                        action => return action,
                    }
                }
            }
            HookAction::Continue
        }

        pub fn fire_async(&self, point: HookPoint, ctx: &HookContext) {
            let _ = self.fire(point, ctx);
        }

        pub fn len(&self) -> usize {
            self.hooks.len()
        }

        pub fn is_empty(&self) -> bool {
            self.hooks.is_empty()
        }

        pub fn has_hooks(&self, point: HookPoint) -> bool {
            self.hooks
                .iter()
                .any(|(hook_point, _)| *hook_point == point)
        }

        pub fn registered_points(&self) -> Vec<HookPoint> {
            let mut points: Vec<HookPoint> = self.hooks.iter().map(|(point, _)| *point).collect();
            points.dedup();
            points
        }

        pub fn clear_point(&mut self, point: HookPoint) {
            self.hooks.retain(|(hook_point, _)| *hook_point != point);
        }

        pub fn clear_all(&mut self) {
            self.hooks.clear();
        }

        pub fn register_from_config(&mut self, _config: &crate::config::Config) {}

        pub fn metrics_snapshot(&self) -> std::collections::HashMap<String, HookMetrics> {
            std::collections::HashMap::new()
        }
    }

    pub fn default_registry() -> HookRegistry {
        HookRegistry::new()
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

/// Mirror a session header into the `jfc-knowledge` session index.
/// Best-effort and silent on error: failed indexing must never block a session
/// compatibility save.
// CLIPPY-ALLOW: public compatibility facade mirrors persisted session index fields.
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
    let result = jfc_knowledge::block_on_knowledge(async {
        let s = jfc_knowledge::KnowledgeStore::open_default().await?;
        s.upsert_session(&row).await
    });
    match result {
        Ok(()) => {}
        Err(e) => tracing::debug!(
            target: "jfc::knowledge",
            session_id = id,
            error = %e,
            "session index upsert skipped"
        ),
    }
}

/// Map serialized session messages to DB transcript rows. `content` is the
/// searchable learning surface; `meta` is the lossless resume payload.
pub(crate) fn to_session_messages(
    serialized_messages: &[crate::session::serialization::SerializedMessage],
) -> Vec<jfc_knowledge::SessionMessage> {
    serialized_messages
        .iter()
        .enumerate()
        .map(|(i, m)| jfc_knowledge::SessionMessage {
            seq: i as i64,
            role: m.role.clone(),
            content: serialized_message_search_text(m),
            meta: serde_json::to_string(m).ok(),
        })
        .collect()
}

fn serialized_message_search_text(
    message: &crate::session::serialization::SerializedMessage,
) -> String {
    use crate::session::serialization::SerializedPart;

    let mut parts = Vec::new();
    for part in &message.parts {
        match part {
            SerializedPart::Text { content }
            | SerializedPart::Reasoning { content }
            | SerializedPart::Advisor { content } => parts.push(content.trim().to_owned()),
            SerializedPart::ReasoningSignature { .. } => {}
            SerializedPart::Tool { tool } => {
                parts.push(tool.kind.clone());
                parts.push(tool.status.clone());
                if let Some(input) = &tool.input
                    && let Ok(text) = serde_json::to_string(input)
                {
                    parts.push(text);
                }
                if let Some(output) = &tool.output
                    && let Ok(text) = serde_json::to_string(output)
                {
                    parts.push(text);
                }
            }
            SerializedPart::TaskStatus {
                description,
                status,
                summary,
                error,
                ..
            } => {
                parts.push(description.clone());
                parts.push(status.clone());
                if let Some(summary) = summary {
                    parts.push(summary.clone());
                }
                if let Some(error) = error {
                    parts.push(error.clone());
                }
            }
            SerializedPart::CompactBoundary { pre_tokens } => {
                parts.push(format!("compact boundary {pre_tokens} tokens"));
            }
            SerializedPart::RedactedThinking { data } => {
                parts.push(data.clone());
            }
        }
    }
    parts
        .into_iter()
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

/// Extract the chain-of-thought (reasoning) text persisted in a session
/// message's `meta` JSON (the structured [`crate::session::serialization::SerializedMessage`]).
///
/// Reasoning is already persisted on every assistant turn (as
/// `SerializedPart::Reasoning` inside `meta`), but it's interleaved with text +
/// tool I/O in the searchable `content`. This is the clean read-back primitive
/// the self-improvement / RSI critique passes use to evaluate the assistant's
/// own reasoning — not just its tool outcomes. Returns `None` when the message
/// has no reasoning parts or `meta` can't be parsed.
pub fn session_message_reasoning(meta: Option<&str>) -> Option<String> {
    use crate::session::serialization::{SerializedMessage, SerializedPart};
    let parsed: SerializedMessage = serde_json::from_str(meta?).ok()?;
    let reasoning = parsed
        .parts
        .iter()
        .filter_map(|part| match part {
            SerializedPart::Reasoning { content } => {
                let trimmed = content.trim();
                (!trimmed.is_empty()).then(|| trimmed.to_owned())
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    (!reasoning.is_empty()).then(|| reasoning.join("\n"))
}

/// Build self-critique [`TurnSample`](jfc_learn::self_critique::TurnSample)s from
/// a persisted transcript: each assistant turn paired with its reasoning (CoT),
/// its output, and whether a user correction or an unrecovered tool error
/// followed. This is the bridge that lets the content-aware self-critique run
/// over real past sessions ("Claude improves Claude").
pub fn self_critique_samples(
    session_id: &str,
    messages: &[jfc_knowledge::SessionMessage],
) -> Vec<jfc_learn::self_critique::TurnSample> {
    use jfc_learn::self_critique::TurnSample;
    let mut samples = Vec::new();
    for (i, m) in messages.iter().enumerate() {
        if m.role != "assistant" {
            continue;
        }
        let reasoning = session_message_reasoning(m.meta.as_deref());
        let thinking_chars = reasoning.as_deref().map_or(0, str::len);
        let followed_by_correction = messages
            .get(i + 1)
            .filter(|next| next.role == "user")
            .is_some_and(|next| looks_like_correction(&next.content));
        let had_unrecovered_error = meta_has_failed_tool(m.meta.as_deref());
        samples.push(TurnSample {
            session_id: session_id.to_owned(),
            seq: m.seq,
            reasoning,
            output: m.content.clone(),
            followed_by_correction,
            had_unrecovered_error,
            thinking_chars,
        });
    }
    samples
}

/// Heuristic: does a user message read like a correction of the prior turn?
fn looks_like_correction(text: &str) -> bool {
    let t = text.trim().to_lowercase();
    if t.is_empty() {
        return false;
    }
    const STARTS: &[&str] = &["no,", "no ", "nope", "actually", "stop", "don't", "wrong"];
    const CONTAINS: &[&str] = &[
        "that's wrong",
        "thats wrong",
        "incorrect",
        "you didn't",
        "you didnt",
        "not what i",
        "you hallucina",
        "you broke",
    ];
    STARTS.iter().any(|p| t.starts_with(p)) || CONTAINS.iter().any(|p| t.contains(p))
}

/// Heuristic: did this assistant turn contain a tool call that failed? Cheap
/// substring check over the serialized `meta` (tool parts carry a `status`).
fn meta_has_failed_tool(meta: Option<&str>) -> bool {
    meta.is_some_and(|m| m.contains("\"status\":\"failed\"") || m.contains("\"status\":\"error\""))
}

/// Convert self-critique [`ImprovementProposal`](jfc_learn::self_critique::ImprovementProposal)s
/// into [`MinedLesson`](jfc_knowledge::session_mine::MinedLesson)s so they flow
/// through the SAME persistence + dedup + recall path as error/preference
/// mining. Recorded as `Finding`s (actionable blind-spots), `Unverified` until a
/// later proof pass confirms them, deduped by a stable `norm_key`.
pub fn self_critique_proposals_to_lessons(
    proposals: &[jfc_learn::self_critique::ImprovementProposal],
) -> Vec<jfc_knowledge::session_mine::MinedLesson> {
    proposals
        .iter()
        .map(|p| jfc_knowledge::session_mine::MinedLesson {
            kind: jfc_knowledge::Kind::Finding,
            trigger: p.title.clone(),
            claim: p.body.clone(),
            outcome: jfc_knowledge::Outcome::Unverified,
            norm_key: format!("critique:{}:{}", p.kind.slug(), norm_slug(&p.title)),
            session_id: p.source_session_id.clone(),
        })
        .collect()
}

/// Stable dedup slug: lowercase, non-alphanumerics collapsed to single dashes.
fn norm_slug(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut prev_dash = true; // trims leading dashes
    for ch in text.to_lowercase().chars() {
        if ch.is_alphanumeric() {
            out.push(ch);
            prev_dash = false;
        } else if !prev_dash {
            out.push('-');
            prev_dash = true;
        }
    }
    out.trim_end_matches('-').to_owned()
}

/// Namespace for self-critique RSI definitions (lets promotion reconstruct ids).
const SELF_CRITIQUE_NS: &str = "self_critique";

/// The definition name for a self-critique proposal (shared by staging +
/// promotion so the stable `definition_id` matches).
fn self_critique_definition_name(title: &str) -> String {
    format!("self-critique-{}", norm_slug(title))
}

/// Stage self-critique proposals as RSI definition CANDIDATES — the actual
/// self-mutation pipeline for skills / tool defs / system prompts / reasoning
/// policies. `Global` scope (self-improvements are cross-project) + `Candidate`
/// status: they land where the request assembler looks, but it only injects
/// `status = 'active'`, so they stay inert until evidence promotes them.
/// `MemoryRule`-style proposals (no definition kind) are skipped — they ride the
/// knowledge/recall path instead.
pub fn self_critique_proposals_to_definitions(
    proposals: &[jfc_learn::self_critique::ImprovementProposal],
) -> Vec<jfc_knowledge::NewDefinition> {
    proposals
        .iter()
        .filter_map(|p| {
            let kind = p.kind.definition_kind()?; // None for MemoryRule
            Some(jfc_knowledge::NewDefinition {
                kind: kind.to_owned(),
                scope: jfc_knowledge::DefinitionScope::Global,
                project_key: None,
                namespace: Some(SELF_CRITIQUE_NS.to_owned()),
                name: self_critique_definition_name(&p.title),
                title: Some(p.title.clone()),
                description: Some(p.evidence.clone()),
                body: p.body.clone(),
                metadata_json: serde_json::json!({
                    "rsi": {
                        "source": "self_critique",
                        "session_id": p.source_session_id,
                        "confidence": p.confidence,
                    }
                })
                .to_string(),
                source_path: Some(format!("rsi:definition:self_critique:{}", p.kind.slug())),
                source_hash: None,
                status: jfc_knowledge::DefinitionStatus::Candidate,
                created_by: "self_critique".to_owned(),
            })
        })
        .collect()
}

/// Convert self-critique proposals into trackable `scope="self"` backlog items
/// (JFC improving JFC). The backlog is the single queryable ledger of what the
/// system suggested for itself, with status proposed → proven → applied — so
/// improvement is measurable, not prose.
pub fn self_critique_proposals_to_backlog(
    proposals: &[jfc_learn::self_critique::ImprovementProposal],
) -> Vec<jfc_knowledge::BacklogItem> {
    proposals
        .iter()
        .map(|p| jfc_knowledge::BacklogItem {
            scope: "self".to_owned(),
            project_key: None,
            // The DEFINITION kind (so promotion can reconstruct the definition
            // id); falls back to the candidate slug for non-definition kinds.
            category: p
                .kind
                .definition_kind()
                .unwrap_or(p.kind.slug())
                .to_owned(),
            title: p.title.clone(),
            body: p.body.clone(),
            evidence: p.evidence.clone(),
            confidence: p.confidence,
            source_session_id: Some(p.source_session_id.clone()),
        })
        .collect()
}

/// One self-improvement pass over a finished session: critique the assistant's
/// own reasoning + outputs (not just tool errors), fold the proposals into the
/// knowledge store as recallable `Finding` lessons (immediate, advisory), stage
/// prompt/skill/tool/reasoning proposals as `Candidate` definitions (the
/// self-mutation pipeline, pending proof), AND record them on the trackable
/// self-improvement backlog. This is the closed "Claude improves Claude" loop
/// for outputs/CoT. Returns `(proposals, lessons_inserted, definitions_staged)`.
/// Best-effort; logs and continues on store error.
pub async fn run_self_critique_pass(
    store: &jfc_knowledge::KnowledgeStore,
    project_key: &str,
    session_id: &str,
    messages: &[jfc_knowledge::SessionMessage],
) -> (usize, usize, usize) {
    let samples = self_critique_samples(session_id, messages);
    // Record an error-pattern SIGNATURE for each failed turn under the `live`
    // variant. Over time this is the distribution of *how* real sessions fail
    // (perception / reasoning / verification / …), not just how often — the
    // comparison surface the eval harness reads to tell variants apart.
    for s in &samples {
        if let Some(kind) = jfc_learn::self_critique::classify_failure(s) {
            let _ = store
                .record_eval_error_signature("live", session_id, kind.signature())
                .await;
        }
    }
    let proposals = jfc_learn::self_critique::critique_turns(
        &jfc_learn::self_critique::HeuristicJudge,
        &samples,
    );
    if proposals.is_empty() {
        return (0, 0, 0);
    }
    let lessons = self_critique_proposals_to_lessons(&proposals);
    let inserted = match store.ingest_mined(project_key, &lessons).await {
        Ok((inserted, _compounded)) => inserted,
        Err(err) => {
            tracing::debug!(target: "jfc::learn::self_critique", error = %err, "self-critique ingest failed");
            0
        }
    };
    let definitions = self_critique_proposals_to_definitions(&proposals);
    let mut staged = 0usize;
    for def in &definitions {
        if store.upsert_definition(def).await.is_ok() {
            staged += 1;
        }
    }
    for item in &self_critique_proposals_to_backlog(&proposals) {
        let _ = store.upsert_backlog_item(item).await;
    }
    tracing::info!(
        target: "jfc::learn::self_critique",
        proposals = proposals.len(),
        lessons = inserted,
        definitions_staged = staged,
        session_id,
        "self-critique: folded reasoning/output lessons + staged prompt/skill/tool candidates + backlog"
    );
    (proposals.len(), inserted, staged)
}

/// Recurrence threshold to auto-promote a self-critique candidate to live. A
/// pattern independently re-flagged across this many sessions is treated as
/// proven enough for these conservative, additive, prompt-safe instructions.
pub const SELF_CRITIQUE_PROMOTE_MIN_RECURRENCE: i64 = 8;

/// Evidence-gated PROMOTION: graduate well-evidenced self-critique candidates to
/// `Active`, so the request assembler injects them into the LIVE system prompt.
/// "Proof" here is RECURRENCE — the pattern was re-flagged across many
/// independent sessions (the backlog's evidence weight). A conservative
/// threshold plus the assembler's own guards (RSI-tagged, prompt-safe, char
/// capped, Active-only) keep it safe; promotion stamps the backlog `applied` for
/// auditability/rollback. Returns the number promoted. A stronger A-B replay
/// proof can raise the bar later without changing this surface.
pub async fn promote_evidenced_self_critique(
    store: &jfc_knowledge::KnowledgeStore,
    min_recurrence: i64,
) -> usize {
    let items = store
        .list_backlog(Some("self"), Some("proposed"), 10_000)
        .await
        .unwrap_or_default();
    let mut promoted = 0usize;
    for item in items {
        if item.recurrence < min_recurrence {
            continue;
        }
        // Only definition-backed categories can be applied to the prompt/tools.
        if !matches!(
            item.category.as_str(),
            "system_prompt"
                | "reasoning_policy"
                | "skill"
                | "tool_definition"
                | "context_playbook"
                | "budget_policy"
                | "harness_patch"
        ) {
            continue;
        }
        let def_id = jfc_knowledge::definitions::definition_id(
            &item.category,
            jfc_knowledge::DefinitionScope::Global.slug(),
            None,
            Some(SELF_CRITIQUE_NS),
            &self_critique_definition_name(&item.title),
        );
        // Only mark applied if a real staged definition actually flipped to
        // Active (skill *suggestions* have no definition → no-op, stay proposed).
        if store
            .set_definition_status(&def_id, "active")
            .await
            .unwrap_or(0)
            > 0
        {
            let _ = store.set_backlog_status(&item.id, "applied").await;
            promoted += 1;
            tracing::info!(
                target: "jfc::learn::self_critique",
                category = %item.category,
                recurrence = item.recurrence,
                title = %item.title,
                "promoted self-critique candidate to ACTIVE — now injected into the live prompt"
            );
        }
    }
    promoted
}

/// Mine recurring user-prompt intents across all sessions and record the
/// frequent ones as `skill` suggestions on the self-improvement backlog ("you
/// keep asking this — make it a skill"). Clusters any wording of the same
/// request together. Returns the number of suggestions written.
pub async fn mine_user_prompt_skills_from_store(
    store: &jfc_knowledge::KnowledgeStore,
    min_count: usize,
) -> usize {
    let sessions = store.list_sessions(None, 1_000_000).await.unwrap_or_default();
    let mut prompts: Vec<String> = Vec::new();
    for s in &sessions {
        if let Ok(messages) = store.load_transcript(&s.id).await {
            for m in messages {
                if m.role == "user" {
                    prompts.push(m.content);
                }
            }
        }
    }
    let clusters = jfc_learn::prompt_miner::mine_user_prompt_skills(&prompts, min_count);
    let mut written = 0usize;
    for cluster in &clusters {
        let intent = cluster.signature.join(" ");
        let item = jfc_knowledge::BacklogItem {
            scope: "self".to_owned(),
            project_key: None,
            category: "skill".to_owned(),
            title: format!("Recurring request → make a skill: {intent}"),
            body: format!(
                "This request recurs ~{} times across sessions (reworded). Consider a skill or \
                 slash-command for it. Examples: {}",
                cluster.count,
                cluster.examples.join(" | ")
            ),
            evidence: format!("{} occurrences", cluster.count),
            confidence: (cluster.count as f64 / (cluster.count as f64 + 5.0)).min(0.95),
            source_session_id: None,
        };
        if store.upsert_backlog_item(&item).await.is_ok() {
            written += 1;
        }
    }
    tracing::info!(
        target: "jfc::learn::prompt_miner",
        clusters = clusters.len(),
        written,
        "mined recurring user-prompt intents into skill suggestions"
    );
    written
}

/// Write a session's full transcript into the DB. Best-effort and silent on
/// error because the caller should keep the chat loop alive, but the DB is the
/// only runtime transcript store.
pub fn save_session_transcript_to_db(
    row: jfc_knowledge::SessionRow,
    messages: Vec<jfc_knowledge::SessionMessage>,
) {
    let id = row.id.clone();
    let result = jfc_knowledge::block_on_knowledge(async {
        let s = jfc_knowledge::KnowledgeStore::open_default().await?;
        s.replace_transcript(&row, &messages).await
    });
    match result {
        Ok(()) => {}
        Err(e) => tracing::debug!(
            target: "jfc::knowledge",
            session_id = id,
            error = %e,
            "session transcript DB write skipped"
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
        self.mismatched.is_empty() && self.passed > 0
    }
}

/// Backfill the DB transcript store from legacy JSON sessions and verify
/// parity. Reads every `ses_*.json`, writes its transcript, then reloads from
/// the DB and asserts the canonicalized message stream matches. Sessions whose
/// JSON the current reader can't deserialize are bucketed as `undeserializable`.
/// READ-ONLY w.r.t. the JSON files.
pub fn backfill_and_verify_sessions(sessions_dir: &std::path::Path) -> SessionParityReport {
    use crate::session::serialization::SerializedSession;
    let mut report = SessionParityReport::default();
    let Ok(entries) = std::fs::read_dir(sessions_dir) else {
        return report;
    };
    let store = match jfc_knowledge::block_on_knowledge(async {
        jfc_knowledge::KnowledgeStore::open_default().await
    }) {
        Ok(s) => s,
        Err(_) => return report,
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

        save_session_transcript_to_db(row, to_session_messages(&session.messages));

        // Reload from the DB and compare the full per-message JSON stream.
        let actual: Vec<(String, Option<String>)> = match jfc_knowledge::block_on_knowledge(async {
            store.load_transcript(&session.id).await
        }) {
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
    jfc_knowledge::block_on_knowledge(async {
        jfc_knowledge::auto_maintain(
            project_root,
            sessions_dir,
            user_memory_dir,
            project_memory_dir,
        )
        .await
    })
}

fn knowledge_maintain_disabled_by_env() -> bool {
    matches!(
        std::env::var("JFC_DISABLE_KNOWLEDGE_MAINTAIN").as_deref(),
        Ok("1") | Ok("true")
    )
}

fn knowledge_maintain_interval_secs() -> u64 {
    std::env::var("JFC_KNOWLEDGE_MAINTAIN_INTERVAL_SECS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .filter(|s| *s > 0)
        .unwrap_or(30 * 60)
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct KnowledgeMaintenancePaths {
    sessions: Option<std::path::PathBuf>,
    user_mem: Option<std::path::PathBuf>,
    project_mem: std::path::PathBuf,
}

fn knowledge_maintenance_paths(project_root: &std::path::Path) -> KnowledgeMaintenancePaths {
    knowledge_maintenance_paths_with_config(project_root, dirs::config_dir())
}

fn knowledge_maintenance_paths_with_config(
    project_root: &std::path::Path,
    config_dir: Option<std::path::PathBuf>,
) -> KnowledgeMaintenancePaths {
    KnowledgeMaintenancePaths {
        sessions: config_dir.as_ref().map(|c| c.join("jfc").join("sessions")),
        user_mem: config_dir.as_ref().map(|c| c.join("jfc").join("memory")),
        project_mem: project_root.join(".jfc").join("memory"),
    }
}

fn run_knowledge_maintenance_pass(
    project_root: &std::path::Path,
) -> Option<jfc_knowledge::MaintainReport> {
    let paths = knowledge_maintenance_paths(project_root);
    match knowledge_maintain(
        project_root,
        paths.sessions.as_deref(),
        paths.user_mem.as_deref(),
        Some(paths.project_mem.as_path()),
    ) {
        Ok(report) => {
            tracing::info!(
                target: "jfc::knowledge",
                imported = report.imported,
                mined = report.mined_inserted,
                compounded = report.mined_compounded,
                consolidated = report.consolidated,
                auto_promoted = report.auto_promoted,
                "cross-project knowledge maintenance pass"
            );
            Some(report)
        }
        Err(e) => {
            tracing::debug!(
                target: "jfc::knowledge",
                error = %e,
                "knowledge maintenance skipped"
            );
            None
        }
    }
}

/// Run a bounded maintenance pass before session-start recall. This closes the
/// loop between observing old results and reading updated memory before acting,
/// without letting slow session mining stall the user's first turn.
pub async fn warm_knowledge_before_prompt(
    project_root: std::path::PathBuf,
    deadline: std::time::Duration,
) {
    if knowledge_maintain_disabled_by_env() {
        return;
    }
    let maintenance =
        tokio::task::spawn_blocking(move || run_knowledge_maintenance_pass(&project_root));
    match tokio::time::timeout(deadline, maintenance).await {
        Ok(Ok(_)) => {}
        Ok(Err(e)) => tracing::debug!(
            target: "jfc::knowledge",
            error = %e,
            "prompt-start knowledge maintenance task failed"
        ),
        Err(_) => tracing::debug!(
            target: "jfc::knowledge",
            deadline_ms = deadline.as_millis() as u64,
            "prompt-start knowledge maintenance exceeded deadline; using existing knowledge"
        ),
    }
}

/// Start the self-driving knowledge/RSI maintenance loop. Prompt recall can be
/// disabled separately with `cross_project_recall_enabled=false`; this loop is
/// intentionally tied only to the explicit maintenance kill switch so JFC keeps
/// importing, mining, consolidating, and auto-promoting in the background.
pub fn spawn_knowledge_maintenance_loop(project_root: std::path::PathBuf) {
    if knowledge_maintain_disabled_by_env() {
        return;
    }
    let tick = knowledge_maintain_interval_secs();
    tokio::spawn(async move {
        loop {
            let project_root = project_root.clone();
            let _ = tokio::task::spawn_blocking(move || {
                run_knowledge_maintenance_pass(&project_root);
            })
            .await;
            tokio::time::sleep(std::time::Duration::from_secs(tick)).await;
        }
    });
}

/// Kick one background maintenance pass. Used by short-lived frontends such as
/// `--print`, where a recurring loop may not live long enough to matter.
pub fn spawn_knowledge_maintenance_once(project_root: std::path::PathBuf) {
    if knowledge_maintain_disabled_by_env() {
        return;
    }
    tokio::spawn(async move {
        let _ = tokio::task::spawn_blocking(move || {
            run_knowledge_maintenance_pass(&project_root);
        })
        .await;
    });
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

#[cfg(test)]
mod knowledge_maintenance_tests {
    use super::*;

    #[test]
    fn session_message_reasoning_extracts_cot_from_meta_normal() {
        // Reasoning is persisted as a tagged `{"type":"reasoning",...}` part in
        // the message `meta`; the accessor pulls just the CoT back out.
        let meta = r#"{"role":"assistant","parts":[
            {"type":"reasoning","content":"Check the file's exact bytes first."},
            {"type":"text","content":"Done."},
            {"type":"reasoning","content":"Then verify with cargo check."}
        ]}"#;
        assert_eq!(
            super::session_message_reasoning(Some(meta)).as_deref(),
            Some("Check the file's exact bytes first.\nThen verify with cargo check.")
        );
        // No reasoning / unparsable / absent → None.
        assert_eq!(
            super::session_message_reasoning(Some(r#"{"role":"user","parts":[{"type":"text","content":"hi"}]}"#)),
            None
        );
        assert_eq!(super::session_message_reasoning(Some("not json")), None);
        assert_eq!(super::session_message_reasoning(None), None);
    }

    #[test]
    fn self_critique_samples_pairs_reasoning_with_correction_normal() {
        use jfc_knowledge::SessionMessage;
        let messages = vec![
            SessionMessage {
                seq: 0,
                role: "assistant".into(),
                content: "I'll edit the file.".into(),
                meta: Some(
                    r#"{"role":"assistant","parts":[{"type":"reasoning","content":"assuming camelCase"}]}"#
                        .into(),
                ),
            },
            SessionMessage {
                seq: 1,
                role: "user".into(),
                content: "no, that's wrong".into(),
                meta: None,
            },
        ];
        let samples = super::self_critique_samples("ses_x", &messages);
        assert_eq!(samples.len(), 1); // only the assistant turn
        assert_eq!(samples[0].reasoning.as_deref(), Some("assuming camelCase"));
        assert!(samples[0].followed_by_correction);

        // End to end: the heuristic judge turns this into a proposal.
        let props = jfc_learn::self_critique::critique_turns(
            &jfc_learn::self_critique::HeuristicJudge,
            &samples,
        );
        assert!(!props.is_empty(), "hedge + correction should yield a proposal");
    }

    #[test]
    fn correction_and_failed_tool_heuristics_normal() {
        assert!(super::looks_like_correction("no, that's wrong"));
        assert!(super::looks_like_correction("actually you broke the build"));
        assert!(!super::looks_like_correction("thanks, looks great"));
        assert!(super::meta_has_failed_tool(Some(
            r#"{"parts":[{"type":"tool","tool":{"status":"failed"}}]}"#
        )));
        assert!(!super::meta_has_failed_tool(Some(r#"{"status":"complete"}"#)));
        assert!(!super::meta_has_failed_tool(None));
    }

    #[test]
    fn self_critique_proposals_to_definitions_stage_as_candidates_regression() {
        use jfc_learn::rsi_curator::CandidateKind;
        use jfc_learn::self_critique::ImprovementProposal;
        let proposals = vec![
            ImprovementProposal {
                kind: CandidateKind::ReasoningPolicy,
                title: "Verify assumptions".into(),
                body: "check first".into(),
                evidence: "ev".into(),
                source_session_id: "s".into(),
                source_seq: 1,
                confidence: 0.6,
            },
            ImprovementProposal {
                kind: CandidateKind::MemoryRule, // no definition kind → skipped
                title: "x".into(),
                body: "y".into(),
                evidence: String::new(),
                source_session_id: "s".into(),
                source_seq: 2,
                confidence: 0.5,
            },
        ];
        let defs = super::self_critique_proposals_to_definitions(&proposals);
        assert_eq!(defs.len(), 1, "MemoryRule has no definition kind");
        assert_eq!(defs[0].kind, "reasoning_policy");
        // SAFETY INVARIANT: staged as Candidate, never Active — so an unproven
        // self-critique never auto-mutates the live prompt/tools.
        assert_eq!(defs[0].status, jfc_knowledge::DefinitionStatus::Candidate);
        assert!(
            defs[0]
                .source_path
                .as_deref()
                .unwrap()
                .starts_with("rsi:definition:"),
            "must be recognized as an RSI definition"
        );
    }

    #[test]
    fn session_db_rows_include_reasoning_and_tool_io_normal() {
        use crate::session::serialization::{
            SerializedMessage, SerializedPart, SerializedToolInput, SerializedToolOutput,
            SerializedToolPart,
        };

        let message = SerializedMessage {
            role: "assistant".into(),
            agent_name: None,
            model_name: None,
            cost_tier: None,
            elapsed: None,
            usage: None,
            created_at: 0,
            parts: vec![
                SerializedPart::Reasoning {
                    content: "thinking through sqlite migration".into(),
                },
                SerializedPart::Tool {
                    tool: Box::new(SerializedToolPart {
                        id: "tool_1".into(),
                        kind: "BashOutput".into(),
                        status: "failed".into(),
                        is_collapsed: false,
                        input: Some(SerializedToolInput::BashOutput {
                            task_id: "bash_bad".into(),
                            offset: None,
                            limit: None,
                            block: None,
                            timeout: None,
                            wait_up_to: None,
                        }),
                        output: Some(SerializedToolOutput::Text {
                            content: "Unknown Bash task id".into(),
                        }),
                        thought_signature: None,
                    }),
                },
            ],
        };

        let rows = to_session_messages(&[message]);

        assert_eq!(rows.len(), 1);
        assert!(
            rows[0]
                .content
                .contains("thinking through sqlite migration")
        );
        assert!(rows[0].content.contains("bash_bad"));
        assert!(rows[0].content.contains("Unknown Bash task id"));
        assert!(rows[0].meta.is_some());
    }

    #[test]
    fn knowledge_maintenance_paths_respect_config_and_project_roots_normal() {
        let project = std::path::PathBuf::from("/repo");
        let config = std::path::PathBuf::from("/cfg");

        let paths = knowledge_maintenance_paths_with_config(&project, Some(config));

        assert_eq!(
            paths.sessions,
            Some(std::path::PathBuf::from("/cfg/jfc/sessions"))
        );
        assert_eq!(
            paths.user_mem,
            Some(std::path::PathBuf::from("/cfg/jfc/memory"))
        );
        assert_eq!(
            paths.project_mem,
            std::path::PathBuf::from("/repo/.jfc/memory")
        );
    }

    #[test]
    fn knowledge_maintenance_paths_tolerate_missing_config_robust() {
        let project = std::path::PathBuf::from("/repo");

        let paths = knowledge_maintenance_paths_with_config(&project, None);

        assert!(paths.sessions.is_none());
        assert!(paths.user_mem.is_none());
        assert_eq!(
            paths.project_mem,
            std::path::PathBuf::from("/repo/.jfc/memory")
        );
    }

    #[test]
    fn session_parity_flip_requires_verified_db_roundtrip_regression() {
        let report = SessionParityReport {
            checked: 2,
            passed: 0,
            mismatched: Vec::new(),
            undeserializable: vec!["ses_bad_1".into(), "ses_bad_2".into()],
        };

        assert!(
            !report.flip_safe(),
            "undeserializable-only legacy candidates must not authorize DB-only reads"
        );
    }

    #[test]
    fn session_parity_flip_safe_when_at_least_one_session_passes_normal() {
        let report = SessionParityReport {
            checked: 2,
            passed: 1,
            mismatched: Vec::new(),
            undeserializable: vec!["ses_bad".into()],
        };

        assert!(report.flip_safe());
    }
}

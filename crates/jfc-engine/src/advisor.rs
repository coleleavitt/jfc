//! Advisor mode — a parallel AI assistant that provides guidance without
//! affecting the main conversation.
//!
//! Mirrors Claude Code v2.1.132's `tengu_advisor_*` telemetry surface (command,
//! dialog_shown, tool_call, tool_interrupted, tool_token_usage). The advisor:
//!
//! - Sees a SNAPSHOT of the main transcript at query time (read-only context).
//! - Runs as a separate non-streaming `provider.complete()` call so it doesn't
//!   share the main agent's stream channel or token accounting.
//! - Has no tools — it answers in prose only.
//! - Maintains its own per-session token budget; when exhausted, queries return
//!   `Err` and the user must reset the session (or wait for a future
//!   `/advisor-reset`).
//! - Doesn't mutate the main `Vec<ChatMessage>` — the caller is responsible for
//!   surfacing the reply (typically by appending an `Advisor`-tagged
//!   `MessagePart` so the renderer can style it distinctly).
//!
//! Default OFF semantics are upheld two ways:
//!   1. The slash command is gated by `app.engine.advisor_enabled` (config flag).
//!   2. Even when "always available", the per-session token budget defaults to
//!      a small ceiling (`DEFAULT_TOKEN_BUDGET`), so a runaway loop can't drain
//!      the user's account.
//!
//! Side-pane rendering is intentionally not implemented here — the renderer
//! consumes `MessagePart::Advisor(String)` inline. See the follow-up note in
//! `render.rs` for where a split-pane would hook in.

use std::sync::{Arc, OnceLock, RwLock};

use anyhow::{Result, anyhow};
use uuid::Uuid;

use crate::types::{ChatMessage, MessagePart, Role, ToolCall, ToolOutput};
use jfc_provider::{
    ModelId, ModelSpec, Provider, ProviderContent, ProviderError, ProviderErrorKind, ProviderId,
    ProviderMessage, ProviderRole, StreamOptions,
};

/// Default per-session token budget. Conservative — about three round-trips
/// worth of advisor calls on a 200K-context model. Users can override via
/// [`AdvisorSession::with_budget`].
pub const DEFAULT_TOKEN_BUDGET: u64 = 50_000;

/// Hard cap on the number of main-transcript chars we'll inline into the
/// advisor's user message. Long sessions otherwise blow past the model's
/// context window AND eat the entire token budget on the first call.
const MAX_SNAPSHOT_CHARS: usize = 40_000;

/// Per-tool output preview cap inside the advisor snapshot. The whole snapshot
/// is capped separately; this prevents one giant Read/Bash result from crowding
/// out every other turn before the final tail-truncation pass.
const MAX_TOOL_RESULT_PREVIEW_CHARS: usize = 2_000;

/// System prompt prepended to every advisor query. Spelled out in full so the
/// model knows it has no tools, no authority to "go do something", and is
/// expected to be terse.
pub const ADVISOR_SYSTEM_PROMPT: &str = "You are an advisor. The user's main agent is currently working on a task. \
     Their transcript so far is below. Answer their advisor question concisely. \
     You don't have tools — just give advice.";

/// Claude Code 2.1.153 server-side advisor prompt. When the Anthropic
/// `advisor_20260301` server tool is enabled this is appended to the main
/// system prompt so the model knows when to call `advisor()`.
pub const SERVER_ADVISOR_SYSTEM_PROMPT: &str = r#"# Advisor Tool

You have access to an `advisor` tool backed by a stronger reviewer model. It takes NO parameters -- when you call advisor(), your entire conversation history is automatically forwarded. They see the task, every tool call you've made, every result you've seen.

Call advisor BEFORE substantive work -- before writing, before committing to an interpretation, before building on an assumption. If the task requires orientation first (finding files, fetching a source, seeing what's there), do that, then call advisor. Orientation is not substantive work. Writing, editing, and declaring an answer are.

Also call advisor:
- When you believe the task is complete. BEFORE this call, make your deliverable durable: write the file, save the result, commit the change. The advisor call takes time; if the session ends during it, a durable result persists and an unwritten one doesn't.
- When stuck -- errors recurring, approach not converging, results that don't fit.
- When considering a change of approach.

On tasks longer than a few steps, call advisor at least once before committing to an approach and once before declaring done. On short reactive tasks where the next action is dictated by tool output you just read, you don't need to keep calling -- the advisor adds most of its value on the first call, before the approach crystallizes.

Give the advice serious weight. If you follow a step and it fails empirically, or you have primary-source evidence that contradicts a specific claim (the file says X, the paper states Y), adapt. A passing self-test is not evidence the advice is wrong -- it's evidence your test doesn't check what the advice is checking.

If you've already retrieved data pointing one way and the advisor points another: don't silently switch. Surface the conflict in one more advisor call -- "I found X, you suggest Y, which constraint breaks the tie?" The advisor saw your evidence but may have underweighted it; a reconcile call is cheaper than committing to the wrong branch."#;

static SERVER_ADVISOR_MODEL: OnceLock<RwLock<Option<ModelId>>> = OnceLock::new();
static LOCAL_ADVISOR_MODEL: OnceLock<RwLock<Option<ModelId>>> = OnceLock::new();
static LOCAL_ADVISOR_PROVIDER: OnceLock<RwLock<Option<ProviderId>>> = OnceLock::new();
static LOCAL_ADVISOR_TOOL_SESSION: OnceLock<tokio::sync::Mutex<Option<AdvisorSession>>> =
    OnceLock::new();

fn server_advisor_slot() -> &'static RwLock<Option<ModelId>> {
    SERVER_ADVISOR_MODEL.get_or_init(|| RwLock::new(None))
}

fn local_advisor_slot() -> &'static RwLock<Option<ModelId>> {
    LOCAL_ADVISOR_MODEL.get_or_init(|| RwLock::new(None))
}

fn local_advisor_provider_slot() -> &'static RwLock<Option<ProviderId>> {
    LOCAL_ADVISOR_PROVIDER.get_or_init(|| RwLock::new(None))
}

fn local_advisor_tool_session() -> &'static tokio::sync::Mutex<Option<AdvisorSession>> {
    LOCAL_ADVISOR_TOOL_SESSION.get_or_init(|| tokio::sync::Mutex::new(None))
}

pub fn active_server_advisor_model() -> Option<ModelId> {
    server_advisor_slot()
        .read()
        .ok()
        .and_then(|guard| guard.clone())
}

pub fn set_active_server_advisor_model(model: Option<ModelId>) {
    if let Ok(mut guard) = server_advisor_slot().write() {
        *guard = model;
    }
}

pub fn active_local_advisor_model() -> Option<ModelId> {
    local_advisor_slot()
        .read()
        .ok()
        .and_then(|guard| guard.clone())
}

pub fn active_local_advisor_provider() -> Option<ProviderId> {
    local_advisor_provider_slot()
        .read()
        .ok()
        .and_then(|guard| guard.clone())
}

pub fn set_active_local_advisor_model(model: Option<ModelId>) {
    if let Ok(mut guard) = local_advisor_slot().write() {
        *guard = model;
    }
}

pub fn set_active_local_advisor_provider(provider: Option<ProviderId>) {
    if let Ok(mut guard) = local_advisor_provider_slot().write() {
        *guard = provider;
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalAdvisorTarget {
    pub provider: Option<ProviderId>,
    pub model: ModelId,
}

impl LocalAdvisorTarget {
    pub fn config_value(&self) -> String {
        match &self.provider {
            Some(provider) => {
                ModelSpec::qualified(provider.clone(), self.model.clone()).to_string()
            }
            None => self.model.to_string(),
        }
    }
}

#[derive(Clone)]
pub struct LocalAdvisorProviderTarget {
    pub provider: Arc<dyn Provider>,
    pub model: ModelId,
}

impl LocalAdvisorProviderTarget {
    fn new(provider: Arc<dyn Provider>, model: ModelId) -> Self {
        Self { provider, model }
    }

    fn same_provider_model(&self, provider: &dyn Provider, model: &ModelId) -> bool {
        self.provider.name() == provider.name() && self.model == *model
    }
}

pub const DEFAULT_LOCAL_ADVISOR_FALLBACK_MODEL_SPECS: &[&str] = &["openai/gpt-5.5", "gpt-5.5"];

pub fn resolve_local_advisor_provider(
    providers: &[Arc<dyn Provider>],
    active_provider: Arc<dyn Provider>,
    configured_provider: Option<&ProviderId>,
    advisor_model: &ModelId,
) -> Result<Arc<dyn Provider>, String> {
    if let Some(configured) = configured_provider {
        return providers
            .iter()
            .find(|provider| provider.name() == configured.as_str())
            .cloned()
            .ok_or_else(|| format!("advisor provider `{configured}` is not configured"));
    }

    if let Some(resolved) =
        crate::runtime::bootstrap::resolve_provider_model(providers, advisor_model.as_str())
    {
        return Ok(resolved.provider);
    }

    if provider_can_run_model(active_provider.as_ref(), advisor_model.as_str()) {
        return Ok(active_provider);
    }

    Err(format!(
        "advisor model `{advisor_model}` does not match any configured provider"
    ))
}

pub fn resolve_local_advisor_provider_targets(
    providers: &[Arc<dyn Provider>],
    active_provider: Arc<dyn Provider>,
    configured_provider: Option<&ProviderId>,
    advisor_model: &ModelId,
) -> Result<Vec<LocalAdvisorProviderTarget>, String> {
    let primary = resolve_local_advisor_provider(
        providers,
        active_provider,
        configured_provider,
        advisor_model,
    )?;
    let mut targets = vec![LocalAdvisorProviderTarget::new(
        primary,
        advisor_model.clone(),
    )];

    for spec in DEFAULT_LOCAL_ADVISOR_FALLBACK_MODEL_SPECS {
        let Some(resolved) = crate::runtime::bootstrap::resolve_provider_model(providers, spec)
        else {
            continue;
        };
        if targets
            .iter()
            .any(|target| target.same_provider_model(resolved.provider.as_ref(), &resolved.model))
        {
            continue;
        }
        targets.push(LocalAdvisorProviderTarget::new(
            resolved.provider,
            resolved.model,
        ));
    }

    Ok(targets)
}

fn provider_can_run_model(provider: &dyn Provider, model: &str) -> bool {
    if provider
        .available_models()
        .iter()
        .any(|info| info.id.as_str() == model)
    {
        return true;
    }

    let lower = model.to_ascii_lowercase();
    match provider.name() {
        "anthropic" | "anthropic-oauth" => lower.starts_with("claude-"),
        "openai" => {
            lower.starts_with("gpt-")
                || lower.starts_with("o1")
                || lower.starts_with("o3")
                || lower.starts_with("o4")
        }
        "codex" => lower.contains("codex"),
        "gemini" | "antigravity" => lower.starts_with("gemini"),
        "litellm" | "openwebui" | "openrouter" => true,
        _ => false,
    }
}

fn env_truthy(name: &str) -> bool {
    std::env::var(name)
        .ok()
        .map(|v| {
            let v = v.trim();
            !(v.is_empty()
                || v == "0"
                || v.eq_ignore_ascii_case("false")
                || v.eq_ignore_ascii_case("no"))
        })
        .unwrap_or(false)
}

pub fn normalize_server_advisor_model(raw: &str) -> Result<ModelId, String> {
    let trimmed = raw.trim();
    let model = if trimmed.is_empty() {
        crate::providers::anthropic_models::ALIAS_OPUS.to_owned()
    } else {
        match trimmed.to_ascii_lowercase().as_str() {
            "opus" => crate::providers::anthropic_models::ALIAS_OPUS.to_owned(),
            "sonnet" => crate::providers::anthropic_models::ALIAS_SONNET.to_owned(),
            _ => ModelSpec::parse_lenient(trimmed)
                .map(|spec| spec.into_model().to_string())
                .unwrap_or_else(|_| trimmed.to_owned()),
        }
    };
    Ok(ModelId::from(model))
}

pub fn normalize_local_advisor_model(
    raw: &str,
    fallback: &ModelId,
) -> Result<LocalAdvisorTarget, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(LocalAdvisorTarget {
            provider: None,
            model: fallback.clone(),
        });
    }
    let target = match trimmed.to_ascii_lowercase().as_str() {
        "opus" => LocalAdvisorTarget {
            provider: None,
            model: ModelId::from(crate::providers::anthropic_models::ALIAS_OPUS.to_owned()),
        },
        "sonnet" => LocalAdvisorTarget {
            provider: None,
            model: ModelId::from(crate::providers::anthropic_models::ALIAS_SONNET.to_owned()),
        },
        "haiku" => LocalAdvisorTarget {
            provider: None,
            model: ModelId::from(crate::providers::anthropic_models::ALIAS_HAIKU.to_owned()),
        },
        _ => {
            let spec = ModelSpec::parse_lenient(trimmed)
                .map_err(|e| format!("invalid advisor model `{trimmed}`: {e}"))?;
            LocalAdvisorTarget {
                provider: spec.provider().cloned(),
                model: spec.into_model(),
            }
        }
    };
    Ok(target)
}

pub fn resolve_local_advisor_model(
    base_model: &ModelId,
    configured: Option<&str>,
    force_enable: bool,
    configured_enabled: Option<bool>,
) -> Result<Option<LocalAdvisorTarget>, String> {
    let env_model = std::env::var("JFC_ADVISOR_MODEL").ok();
    let env_enabled = env_truthy("JFC_ADVISOR_ENABLED");
    let env_disabled = env_truthy("JFC_ADVISOR_DISABLED") || env_truthy("JFC_DISABLE_ADVISOR");
    if env_disabled && !force_enable {
        return Ok(None);
    }
    let raw = configured
        .filter(|s| !s.trim().is_empty())
        .map(str::to_owned)
        .or(env_model);
    if matches!(configured_enabled, Some(false)) && !force_enable && raw.is_none() && !env_enabled {
        return Ok(None);
    }
    // Local advisor is default-on. With no explicit model, use the active model
    // as the side reviewer. `advisor_enabled = false`, `--no-advisor`, or
    // `JFC_ADVISOR_DISABLED=1` are the opt-out paths.
    normalize_local_advisor_model(raw.as_deref().unwrap_or(""), base_model).map(Some)
}

pub fn supports_server_advisor_model(model: &str) -> bool {
    let lower = model.to_ascii_lowercase();
    lower.contains("opus-4-7") || lower.contains("opus-4-6") || lower.contains("sonnet-4-6")
}

pub fn server_advisor_env_enabled() -> bool {
    if env_truthy("CLAUDE_CODE_DISABLE_ADVISOR_TOOL") {
        return false;
    }
    env_truthy("CLAUDE_CODE_ENABLE_EXPERIMENTAL_ADVISOR_TOOL")
        || env_truthy("JFC_SERVER_ADVISOR_ENABLED")
        || crate::feature_gates::is_enabled(crate::feature_gates::FeatureGate::TenguSageCompass2)
}

pub fn resolve_server_advisor_model(
    base_model: &ModelId,
    configured: Option<&str>,
    force_enable: bool,
    strict: bool,
) -> Result<Option<ModelId>, String> {
    if env_truthy("CLAUDE_CODE_DISABLE_ADVISOR_TOOL") {
        if strict || force_enable {
            return Err("advisor is disabled by CLAUDE_CODE_DISABLE_ADVISOR_TOOL".to_owned());
        }
        return Ok(None);
    }

    let env_model = std::env::var("JFC_SERVER_ADVISOR_MODEL")
        .ok()
        .or_else(|| std::env::var("CLAUDE_CODE_ADVISOR_MODEL").ok());
    let raw = configured
        .filter(|s| !s.trim().is_empty())
        .map(str::to_owned)
        .or(env_model);
    let should_enable = force_enable || raw.is_some() || server_advisor_env_enabled();
    if !should_enable {
        return Ok(None);
    }

    if !supports_server_advisor_model(base_model.as_str()) {
        let msg = format!(
            "advisor requires a base model containing opus-4-7, opus-4-6, or sonnet-4-6; active model is {base_model}"
        );
        if strict || force_enable {
            return Err(msg);
        }
        tracing::warn!(target: "jfc::advisor", %msg);
        return Ok(None);
    }

    let advisor = normalize_server_advisor_model(raw.as_deref().unwrap_or("opus"))?;
    if !supports_server_advisor_model(advisor.as_str()) {
        return Err(format!(
            "advisor model must contain opus-4-7, opus-4-6, or sonnet-4-6; got {advisor}"
        ));
    }

    Ok(Some(advisor))
}

pub const LOCAL_ADVISOR_TOOL_QUERY: &str = "Review my conversation so far. Flag anything I'm missing, any assumption I should verify, and any risk I'm overlooking. Be specific and terse.";

pub struct AdvisorReply {
    pub content: String,
    pub provider: ProviderId,
    pub model: ModelId,
    pub fallback_from: Option<ModelId>,
}

impl AdvisorReply {
    pub fn model_note(&self) -> String {
        match &self.fallback_from {
            Some(original) => format!(
                "local advisor model: {} (fallback from {})",
                self.model, original
            ),
            None => format!("local advisor model: {}", self.model),
        }
    }
}

pub async fn ask_local_advisor_tool(
    targets: &[LocalAdvisorProviderTarget],
    main_transcript_snapshot: &[ChatMessage],
) -> Result<String> {
    let primary = targets
        .first()
        .ok_or_else(|| anyhow!("local advisor has no provider target"))?;
    let mut guard = local_advisor_tool_session().lock().await;
    let reset = guard
        .as_ref()
        .map(|session| !targets.iter().any(|target| target.model == session.model))
        .unwrap_or(true);
    if reset {
        *guard = Some(AdvisorSession::new(primary.model.clone()));
    }
    let session = guard
        .as_mut()
        .expect("advisor session should be initialized");
    let reply = ask_advisor_with_fallback(
        targets,
        session,
        LOCAL_ADVISOR_TOOL_QUERY.to_owned(),
        main_transcript_snapshot,
    )
    .await?;
    Ok(format!(
        "{}\n\n_({}; budget: {} of {} tokens remaining)_",
        reply.content,
        reply.model_note(),
        session.tokens_remaining(),
        session.token_budget
    ))
}

/// Per-session advisor state. Owns its own transcript (separate from the main
/// agent) and tracks token usage for budget enforcement.
///
/// Constructed lazily by callers (typically `App::ensure_advisor_session()`)
/// so a user that never invokes `/advisor` pays no allocation cost.
#[derive(Debug, Clone)]
pub struct AdvisorSession {
    pub id: String,
    pub transcript: Vec<ChatMessage>,
    pub model: ModelId,
    pub token_budget: u64,
    pub tokens_used: u64,
}

impl AdvisorSession {
    /// New session with the default budget.
    pub fn new(model: impl Into<ModelId>) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            transcript: Vec::new(),
            model: model.into(),
            token_budget: DEFAULT_TOKEN_BUDGET,
            tokens_used: 0,
        }
    }

    /// Builder-style budget override.
    pub fn with_budget(mut self, budget: u64) -> Self {
        self.token_budget = budget;
        self
    }

    /// Tokens still available for new advisor calls.
    pub fn tokens_remaining(&self) -> u64 {
        self.token_budget.saturating_sub(self.tokens_used)
    }

    pub fn switch_model_preserving_budget(&mut self, model: impl Into<ModelId>) {
        let model = model.into();
        if self.model == model {
            return;
        }
        self.model = model;
        self.transcript.clear();
    }

    /// True when the budget has been spent and `ask_advisor` will refuse new
    /// queries.
    pub fn is_exhausted(&self) -> bool {
        self.tokens_used >= self.token_budget
    }

    /// Account a successful round-trip's token usage against the budget. The
    /// stream / completion path normally does this for us; tests poke it
    /// directly.
    pub fn record_usage(&mut self, used: u64) {
        // saturating_add: a misbehaving provider that reports `u64::MAX` won't
        // wrap us back to "plenty of budget left".
        self.tokens_used = self.tokens_used.saturating_add(used);
    }
}

/// Render the main transcript into a single user-message string the advisor
/// can read. Capped at `MAX_SNAPSHOT_CHARS`; older content gets dropped.
///
/// Format mirrors `auto_mode::build_transcript` — role-prefixed plain text, no
/// JSON. Tool calls include their input summary, status, and a capped output
/// preview so the advisor can evaluate what actually happened without hauling
/// huge file reads or command logs into the side-call.
fn render_snapshot(main_transcript: &[ChatMessage]) -> String {
    let mut out = String::new();
    for msg in main_transcript {
        let role_label = match msg.role {
            Role::User => "User",
            Role::Assistant => "Assistant",
        };
        for part in &msg.parts {
            match part {
                MessagePart::Text(s) if !s.is_empty() => {
                    out.push_str(role_label);
                    out.push_str(": ");
                    out.push_str(s);
                    out.push('\n');
                }
                MessagePart::Reasoning(s) if !s.is_empty() => {
                    out.push_str(role_label);
                    out.push_str(" (reasoning): ");
                    out.push_str(s);
                    out.push('\n');
                }
                MessagePart::Tool(tc) => render_tool_snapshot(&mut out, role_label, tc),
                MessagePart::Advisor(_) => {
                    // Don't echo prior advisor turns into the snapshot — the
                    // advisor's own transcript handles that. Including them
                    // would double-count tokens AND let the advisor
                    // accidentally treat its own past suggestions as
                    // authoritative "main agent" decisions.
                }
                MessagePart::TaskStatus(_) | MessagePart::CompactBoundary { .. } => {}
                other => tracing::debug!(
                    target: "jfc::advisor",
                    part = ?other,
                    "skipping unsupported message part in advisor snapshot"
                ),
            }
        }
    }
    if out.len() > MAX_SNAPSHOT_CHARS {
        // Tail-truncate so the most recent context is preserved. Older
        // material is replaced with a `[…elided…]` marker so the advisor
        // knows context was clipped, not that the session began mid-thought.
        let head_marker = "[…earlier transcript elided…]\n";
        let keep = MAX_SNAPSHOT_CHARS.saturating_sub(head_marker.len());
        // Snap to a UTF-8 char boundary at or after `len - keep`.
        let mut start = out.len().saturating_sub(keep);
        while start < out.len() && !out.is_char_boundary(start) {
            start += 1;
        }
        let tail = out[start..].to_owned();
        out.clear();
        out.push_str(head_marker);
        out.push_str(&tail);
    }
    out
}

fn render_tool_snapshot(out: &mut String, role_label: &str, tc: &ToolCall) {
    out.push_str(role_label);
    out.push_str(": [Tool: ");
    out.push_str(tc.kind.label());
    let input_summary = tc.input.summary();
    if !input_summary.is_empty() {
        out.push_str(" - ");
        push_preview(out, &input_summary, 300);
    }
    out.push_str("; status=");
    out.push_str(tc.status.label());
    if let Some(ms) = tc.elapsed_ms {
        out.push_str("; elapsed=");
        out.push_str(&format!("{ms}ms"));
    }
    out.push_str("]\n");

    let result = tool_output_snapshot_text(&tc.output);
    let result = result.trim();
    if !result.is_empty() {
        out.push_str("Tool result: ");
        push_preview(out, result, MAX_TOOL_RESULT_PREVIEW_CHARS);
        out.push('\n');
    }
}

fn tool_output_snapshot_text(output: &ToolOutput) -> String {
    match output {
        ToolOutput::Text(s) => s.clone(),
        ToolOutput::LargeText(lt) => lt.content.clone(),
        ToolOutput::Diff(d) => format!("Applied diff to {}", d.file_path),
        ToolOutput::FileContent { content, .. } => content.clone(),
        ToolOutput::Command {
            stdout,
            stderr,
            exit_code,
        } => format!(
            "exit: {}\nstdout: {}\nstderr: {}",
            exit_code.unwrap_or(-1),
            stdout,
            stderr
        ),
        ToolOutput::FileList(files) => files.join("\n"),
        ToolOutput::ServerToolResult { tool_kind, content } => {
            crate::types::format_server_tool_result_text_public(tool_kind, content)
        }
        ToolOutput::Empty => String::new(),
    }
}

fn push_preview(out: &mut String, text: &str, limit: usize) {
    if text.len() <= limit {
        out.push_str(text);
        return;
    }
    let mut end = limit;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    out.push_str(&text[..end]);
    out.push_str(&format!("... [truncated, {} chars total]", text.len()));
}

/// Build the messages for one advisor round-trip. The first user message
/// contains the main-transcript snapshot so the advisor has working context;
/// the second contains the user's actual advisor question. The model itself
/// is told (via the system prompt) not to invent tools.
fn build_messages(snapshot: &str, query: &str) -> Vec<ProviderMessage> {
    let snapshot_block = if snapshot.is_empty() {
        "<main-transcript>\n(empty — main agent has not started yet)\n</main-transcript>".to_owned()
    } else {
        format!("<main-transcript>\n{snapshot}</main-transcript>")
    };
    vec![
        ProviderMessage {
            role: ProviderRole::User,
            content: vec![ProviderContent::Text(snapshot_block)],
        },
        ProviderMessage {
            role: ProviderRole::User,
            content: vec![ProviderContent::Text(format!("Advisor question: {query}"))],
        },
    ]
}

/// Run one advisor round-trip. Returns the advisor's reply text on success;
/// `Err` when the budget is exhausted, when the provider errors, or when the
/// query is empty.
///
/// Does NOT mutate `main_transcript_snapshot` — that's a `&[ChatMessage]` by
/// design. The caller is responsible for surfacing the reply (e.g. by pushing
/// a `MessagePart::Advisor(reply)` onto the main transcript at their leisure).
///
/// The session's own `transcript` field IS updated: each call appends a user
/// turn (the query) and an assistant turn (the reply), so `/advisor` follow-up
/// questions can build on prior advisor context if a future revision wants to.
#[tracing::instrument(
    target = "jfc::advisor",
    skip(provider, session, main_transcript_snapshot),
    fields(
        provider = %provider.name(),
        model = %session.model,
        snapshot_msgs = main_transcript_snapshot.len(),
        budget_remaining = session.tokens_remaining(),
        session_id = %session.id,
    ),
)]
pub async fn ask_advisor(
    provider: &dyn Provider,
    session: &mut AdvisorSession,
    query: String,
    main_transcript_snapshot: &[ChatMessage],
) -> Result<String> {
    let trimmed = query.trim();
    if trimmed.is_empty() {
        return Err(anyhow!("advisor query is empty"));
    }
    if session.is_exhausted() {
        return Err(anyhow!(
            "advisor token budget exhausted ({}/{} used)",
            session.tokens_used,
            session.token_budget
        ));
    }

    let snapshot = render_snapshot(main_transcript_snapshot);
    ask_advisor_once(provider, session, trimmed, &snapshot).await
}

pub async fn ask_advisor_with_fallback(
    targets: &[LocalAdvisorProviderTarget],
    session: &mut AdvisorSession,
    query: String,
    main_transcript_snapshot: &[ChatMessage],
) -> Result<AdvisorReply> {
    let trimmed = query.trim();
    if trimmed.is_empty() {
        return Err(anyhow!("advisor query is empty"));
    }
    if session.is_exhausted() {
        return Err(anyhow!(
            "advisor token budget exhausted ({}/{} used)",
            session.tokens_used,
            session.token_budget
        ));
    }

    let snapshot = render_snapshot(main_transcript_snapshot);
    let mut last_err = None;
    let original_model = session.model.clone();
    let mut ordered_targets: Vec<&LocalAdvisorProviderTarget> = targets.iter().collect();
    if let Some(pos) = ordered_targets
        .iter()
        .position(|target| target.model == original_model)
    {
        ordered_targets.swap(0, pos);
    }
    for target in ordered_targets {
        session.switch_model_preserving_budget(target.model.clone());
        let result = ask_advisor_once(target.provider.as_ref(), session, trimmed, &snapshot).await;
        match result {
            Ok(content) => {
                let fallback_from = if target.model == original_model {
                    None
                } else {
                    Some(original_model.clone())
                };
                return Ok(AdvisorReply {
                    content,
                    provider: ProviderId::new(target.provider.name()),
                    model: target.model.clone(),
                    fallback_from,
                });
            }
            Err(e) if advisor_error_allows_model_fallback(&e) => {
                tracing::warn!(
                    target: "jfc::advisor",
                    provider = %target.provider.name(),
                    model = %target.model,
                    error = %e,
                    "local advisor model unavailable; trying fallback"
                );
                last_err = Some(e);
            }
            Err(e) => return Err(e),
        }
    }

    Err(last_err.unwrap_or_else(|| anyhow!("local advisor has no provider target")))
}

async fn ask_advisor_once(
    provider: &dyn Provider,
    session: &mut AdvisorSession,
    trimmed_query: &str,
    snapshot: &str,
) -> Result<String> {
    let messages = build_messages(snapshot, trimmed_query);
    let opts = StreamOptions::new(session.model.clone())
        .system(ADVISOR_SYSTEM_PROMPT)
        .max_tokens(2048);

    let resp = crate::prompt_executor::complete_once(provider, messages, &opts).await?;

    // Shared canonical derivation: provider-reported tokens, else floor(chars/4)
    // over the snapshot + query + reply. Keeps the advisor budget gate aligned
    // with the council budget and economy ledger.
    let fallback_chars = snapshot.len() + trimmed_query.len() + resp.content.len();
    let (used, _source) = resp.usage.billable_tokens(fallback_chars);
    session.record_usage(used);
    session
        .transcript
        .push(ChatMessage::user(trimmed_query.to_owned()));
    session
        .transcript
        .push(ChatMessage::assistant(resp.content.clone()));

    tracing::info!(
        target: "jfc::advisor",
        provider = %provider.name(),
        model = %session.model,
        used,
        tokens_used_total = session.tokens_used,
        budget_remaining = session.tokens_remaining(),
        reply_chars = resp.content.len(),
        "advisor_reply"
    );

    Ok(resp.content)
}

fn advisor_error_allows_model_fallback(error: &anyhow::Error) -> bool {
    for cause in error.chain() {
        if let Some(provider_error) = cause.downcast_ref::<ProviderError>() {
            return matches!(
                provider_error.kind,
                ProviderErrorKind::Permission
                    | ProviderErrorKind::RateLimit
                    | ProviderErrorKind::Overloaded
                    | ProviderErrorKind::NotFound
                    | ProviderErrorKind::Server
            );
        }
    }

    let message = error.to_string().to_ascii_lowercase();
    message.contains("all anthropic oauth accounts exhausted")
        || message.contains("rate limit")
        || message.contains("rate-limit")
        || message.contains("rate_limited")
        || message.contains("overloaded")
        || message.contains("529")
        || message.contains("model access denied")
        || message.contains("not enabled on your anthropic account")
        || message.contains("model not found")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{ToolInput, ToolKind};
    use async_trait::async_trait;
    use jfc_provider::{
        CompletionResponse, EventStream, ModelInfo, ProviderMessage as PMsg, StreamConvention,
        StreamOptions as SOpts,
    };

    /// Mock provider that always returns a canned `CompletionResponse`.
    /// Modelled on `auto_mode::tests::FakeProvider` — same Mutex-of-Option
    /// pattern so we can stash either Ok or Err and pop it once.
    struct FakeProvider {
        name: &'static str,
        models: Vec<ModelInfo>,
        result: std::sync::Mutex<Option<Result<CompletionResponse>>>,
    }

    impl FakeProvider {
        fn echo(reply: &str) -> Self {
            Self {
                name: "fake-advisor",
                models: Vec::new(),
                result: std::sync::Mutex::new(Some(Ok(CompletionResponse {
                    content: reply.to_owned(),
                    usage: jfc_provider::TokenUsage {
                        input_tokens: 100,
                        output_tokens: 50,
                        cache_read_tokens: 0,
                        cache_creation_tokens: 0,
                    },
                }))),
            }
        }

        fn err() -> Self {
            Self {
                name: "fake-advisor",
                models: Vec::new(),
                result: std::sync::Mutex::new(Some(Err(anyhow!("network down")))),
            }
        }

        fn catalog(name: &'static str, models: &[&str]) -> Self {
            Self::with_result(name, models, Err(anyhow!("not used")))
        }

        fn with_result(
            name: &'static str,
            models: &[&str],
            result: Result<CompletionResponse>,
        ) -> Self {
            Self {
                name,
                models: models
                    .iter()
                    .map(|id| ModelInfo::new(*id, *id, name))
                    .collect(),
                result: std::sync::Mutex::new(Some(result)),
            }
        }
    }

    #[async_trait]
    impl Provider for FakeProvider {
        fn name(&self) -> &str {
            self.name
        }
        fn available_models(&self) -> Vec<ModelInfo> {
            self.models.clone()
        }
        fn stream_convention(&self) -> StreamConvention {
            StreamConvention::AnthropicNative
        }
        async fn stream(&self, _messages: Vec<PMsg>, _options: &SOpts) -> Result<EventStream> {
            Err(anyhow!("not used in advisor tests"))
        }
        async fn complete(
            &self,
            _messages: Vec<PMsg>,
            _options: &SOpts,
        ) -> Result<CompletionResponse> {
            self.result
                .lock()
                .unwrap()
                .take()
                .expect("FakeProvider::complete called more than once")
        }
    }
    impl jfc_provider::seal::Sealed for FakeProvider {}

    // ─── Normal path ─────────────────────────────────────────────────────

    /// Normal: a fresh advisor session has the default budget and zero usage.
    #[test]
    fn advisor_session_new_has_default_budget_normal() {
        let s = AdvisorSession::new("claude-opus-4-7");
        assert_eq!(s.token_budget, DEFAULT_TOKEN_BUDGET);
        assert_eq!(s.tokens_used, 0);
        assert!(!s.is_exhausted());
        assert_eq!(s.tokens_remaining(), DEFAULT_TOKEN_BUDGET);
        assert!(!s.id.is_empty());
        assert_eq!(s.model.as_str(), "claude-opus-4-7");
    }

    /// Normal: with_budget overrides the default ceiling.
    #[test]
    fn advisor_session_with_budget_overrides_default_normal() {
        let s = AdvisorSession::new("m").with_budget(123);
        assert_eq!(s.token_budget, 123);
    }

    /// Normal: ask_advisor with a mock provider returns the canned advice
    /// verbatim (the "echo" requirement in the deliverable).
    #[tokio::test]
    async fn ask_advisor_returns_canned_advice_normal() {
        let provider = FakeProvider::echo("Yes, that approach is correct.");
        let mut session = AdvisorSession::new("test-model");
        let main_transcript = vec![ChatMessage::user("doing some work".into())];

        let reply = ask_advisor(
            &provider,
            &mut session,
            "is this approach right?".into(),
            &main_transcript,
        )
        .await
        .expect("advisor should reply");

        assert_eq!(reply, "Yes, that approach is correct.");
    }

    /// Normal: token budget is decremented per query.
    #[tokio::test]
    async fn ask_advisor_decrements_budget_normal() {
        let provider = FakeProvider::echo("ok");
        let mut session = AdvisorSession::new("test-model").with_budget(1_000);
        assert_eq!(session.tokens_used, 0);

        let _ = ask_advisor(&provider, &mut session, "hello?".into(), &[])
            .await
            .expect("advisor reply");

        // Provider reported 100 input + 50 output = 150 tokens.
        assert_eq!(session.tokens_used, 150);
        assert_eq!(session.tokens_remaining(), 850);
    }

    /// Normal: the advisor session's own transcript records the query +
    /// reply, so a follow-up advisor turn could build on prior context.
    #[tokio::test]
    async fn ask_advisor_records_session_transcript_normal() {
        let provider = FakeProvider::echo("here is advice");
        let mut session = AdvisorSession::new("test-model");

        let _ = ask_advisor(&provider, &mut session, "q?".into(), &[])
            .await
            .expect("advisor reply");

        assert_eq!(session.transcript.len(), 2);
        assert_eq!(session.transcript[0].role, Role::User);
        assert_eq!(session.transcript[1].role, Role::Assistant);
    }

    /// Normal: local advisor provider selection routes to the provider that owns
    /// the configured advisor model instead of blindly using the active provider.
    #[test]
    fn resolve_local_advisor_provider_uses_model_owner_normal() {
        let anthropic: Arc<dyn Provider> = Arc::new(FakeProvider::catalog(
            "anthropic-oauth",
            &["claude-sonnet-4-6"],
        ));
        let openai: Arc<dyn Provider> = Arc::new(FakeProvider::catalog("openai", &["gpt-5.5"]));
        let providers = vec![anthropic.clone(), openai.clone()];

        let selected =
            resolve_local_advisor_provider(&providers, anthropic, None, &ModelId::new("gpt-5.5"))
                .expect("advisor provider should resolve");

        assert_eq!(selected.name(), "openai");
    }

    /// Normal: provider-qualified advisor config preserves its provider prefix
    /// rather than stripping it to a bare model id.
    #[test]
    fn normalize_local_advisor_model_preserves_provider_prefix_normal() {
        let target = normalize_local_advisor_model("openai/gpt-5.5", &ModelId::new("fallback"))
            .expect("model should parse");

        assert_eq!(target.provider.as_ref().map(|p| p.as_str()), Some("openai"));
        assert_eq!(target.model.as_str(), "gpt-5.5");
        assert_eq!(target.config_value(), "openai/gpt-5.5");
    }

    /// Normal: advisor provider targets include the configured primary model plus
    /// a gpt-5.5 fallback when an OpenAI-compatible provider is configured.
    #[test]
    fn resolve_local_advisor_provider_targets_appends_gpt55_fallback_normal() {
        let anthropic: Arc<dyn Provider> = Arc::new(FakeProvider::catalog(
            "anthropic-oauth",
            &["claude-opus-4-8"],
        ));
        let openai: Arc<dyn Provider> = Arc::new(FakeProvider::catalog("openai", &["gpt-5.5"]));
        let providers = vec![anthropic.clone(), openai];

        let targets = resolve_local_advisor_provider_targets(
            &providers,
            anthropic,
            None,
            &ModelId::new("claude-opus-4-8"),
        )
        .expect("targets should resolve");

        assert_eq!(targets.len(), 2);
        assert_eq!(targets[0].provider.name(), "anthropic-oauth");
        assert_eq!(targets[0].model.as_str(), "claude-opus-4-8");
        assert_eq!(targets[1].provider.name(), "openai");
        assert_eq!(targets[1].model.as_str(), "gpt-5.5");
    }

    /// Normal: render_snapshot produces role-prefixed lines for text parts.
    #[test]
    fn render_snapshot_includes_user_and_assistant_text_normal() {
        let mut transcript = vec![
            ChatMessage::user("hello".into()),
            ChatMessage::assistant("hi back".into()),
        ];
        // Mix in a reasoning part to exercise that branch.
        transcript[1]
            .parts
            .push(MessagePart::Reasoning("inner thoughts".into()));

        let snap = render_snapshot(&transcript);
        assert!(snap.contains("User: hello"));
        assert!(snap.contains("Assistant: hi back"));
        assert!(snap.contains("Assistant (reasoning): inner thoughts"));
    }

    /// Normal: local advisor snapshots include tool inputs and result previews,
    /// not just the tool name. Otherwise the local advisor cannot review the
    /// actual evidence the main agent saw.
    #[test]
    fn render_snapshot_includes_tool_result_preview_normal() {
        let mut tool = ToolCall::new_pending(
            "bash-1".into(),
            ToolKind::Bash,
            ToolInput::Bash {
                command: "cargo check".into(),
                timeout: None,
                workdir: None,
                run_in_background: None,
            },
        );
        tool.mark_completed().expect("mark completed");
        tool.output = ToolOutput::Command {
            stdout: "Finished dev profile".into(),
            stderr: String::new(),
            exit_code: Some(0),
        };

        let snap = render_snapshot(&[ChatMessage::assistant_parts(vec![MessagePart::tool_boxed(
            Box::new(tool),
        )])]);

        assert!(snap.contains("Assistant: [Tool: Bash - cargo check; status=completed]"));
        assert!(snap.contains("Tool result: exit: 0"));
        assert!(snap.contains("stdout: Finished dev profile"));
    }

    // ─── Robust path ─────────────────────────────────────────────────────

    /// Robust: when the budget is exhausted, ask_advisor refuses new queries
    /// with an Err, rather than silently going over budget.
    #[tokio::test]
    async fn ask_advisor_refuses_when_budget_exhausted_robust() {
        let provider = FakeProvider::echo("should not be called");
        let mut session = AdvisorSession::new("test-model").with_budget(10);
        // Pre-spend the whole budget.
        session.record_usage(10);
        assert!(session.is_exhausted());

        let result = ask_advisor(&provider, &mut session, "anything?".into(), &[]).await;
        assert!(result.is_err());
        let msg = result.err().unwrap().to_string();
        assert!(
            msg.contains("budget exhausted"),
            "expected budget-exhausted error, got: {msg}"
        );
    }

    /// Robust: ask_advisor leaves the main transcript untouched. The
    /// "snapshot" semantics in the deliverable demand this — the advisor must
    /// not mutate the main agent's view of the conversation.
    #[tokio::test]
    async fn ask_advisor_does_not_mutate_main_transcript_robust() {
        let provider = FakeProvider::echo("advice text");
        let mut session = AdvisorSession::new("test-model");
        let main_transcript = vec![
            ChatMessage::user("u1".into()),
            ChatMessage::assistant("a1".into()),
            ChatMessage::user("u2".into()),
        ];
        let before = main_transcript.clone();

        let _ = ask_advisor(
            &provider,
            &mut session,
            "review please".into(),
            &main_transcript,
        )
        .await
        .expect("advisor reply");

        // Length unchanged.
        assert_eq!(main_transcript.len(), before.len());
        // Role + first text part unchanged for each message — full struct
        // equality isn't derived (and `Provider` field on EngineState makes that
        // hard), so we compare the visible text.
        for (m1, m2) in main_transcript.iter().zip(before.iter()) {
            assert_eq!(m1.role, m2.role);
            assert_eq!(m1.parts.len(), m2.parts.len());
        }
    }

    /// Robust: an empty query short-circuits with Err and doesn't burn budget.
    #[tokio::test]
    async fn ask_advisor_rejects_empty_query_robust() {
        let provider = FakeProvider::echo("should not be called");
        let mut session = AdvisorSession::new("test-model");
        let result = ask_advisor(&provider, &mut session, "   ".into(), &[]).await;
        assert!(result.is_err());
        assert_eq!(session.tokens_used, 0);
    }

    /// Robust: provider errors propagate without changing the budget. Without
    /// this, a flaky network would silently exhaust the budget on retries.
    #[tokio::test]
    async fn ask_advisor_provider_error_preserves_budget_robust() {
        let provider = FakeProvider::err();
        let mut session = AdvisorSession::new("test-model");
        let result = ask_advisor(&provider, &mut session, "q".into(), &[]).await;
        assert!(result.is_err());
        assert_eq!(session.tokens_used, 0);
    }

    /// Robust: a provider/model availability failure falls through to the next
    /// configured advisor target instead of surfacing a hard Advisor error.
    #[tokio::test]
    async fn ask_advisor_falls_back_to_gpt55_when_primary_unavailable_robust() {
        let primary: Arc<dyn Provider> = Arc::new(FakeProvider::with_result(
            "anthropic-oauth",
            &["claude-opus-4-8"],
            Err(anyhow!(
                "all Anthropic OAuth accounts exhausted with no successful response"
            )),
        ));
        let fallback: Arc<dyn Provider> = Arc::new(FakeProvider::with_result(
            "openai",
            &["gpt-5.5"],
            Ok(CompletionResponse {
                content: "fallback advice".to_owned(),
                usage: jfc_provider::TokenUsage {
                    input_tokens: 20,
                    output_tokens: 5,
                    cache_read_tokens: 0,
                    cache_creation_tokens: 0,
                },
            }),
        ));
        let targets = vec![
            LocalAdvisorProviderTarget::new(primary, ModelId::new("claude-opus-4-8")),
            LocalAdvisorProviderTarget::new(fallback, ModelId::new("gpt-5.5")),
        ];
        let mut session = AdvisorSession::new("claude-opus-4-8").with_budget(1_000);

        let reply = ask_advisor_with_fallback(&targets, &mut session, "review".into(), &[])
            .await
            .expect("fallback advisor should reply");

        assert_eq!(reply.content, "fallback advice");
        assert_eq!(reply.model.as_str(), "gpt-5.5");
        assert_eq!(
            reply.fallback_from.as_ref().map(|m| m.as_str()),
            Some("claude-opus-4-8")
        );
        assert_eq!(session.model.as_str(), "gpt-5.5");
        assert_eq!(session.tokens_used, 25);
    }

    /// Robust: ordinary provider failures still surface instead of falling back,
    /// so a bad network/path doesn't silently mask the root cause.
    #[tokio::test]
    async fn ask_advisor_does_not_fallback_on_plain_network_error_robust() {
        let primary: Arc<dyn Provider> = Arc::new(FakeProvider::err());
        let fallback: Arc<dyn Provider> = Arc::new(FakeProvider::echo("should not run"));
        let targets = vec![
            LocalAdvisorProviderTarget::new(primary, ModelId::new("test-model")),
            LocalAdvisorProviderTarget::new(fallback, ModelId::new("gpt-5.5")),
        ];
        let mut session = AdvisorSession::new("test-model");

        let result = ask_advisor_with_fallback(&targets, &mut session, "review".into(), &[]).await;

        assert!(result.is_err());
        assert_eq!(session.model.as_str(), "test-model");
        assert_eq!(session.tokens_used, 0);
    }

    /// Robust: a snapshot with no usable parts (only tool / task-status /
    /// boundary parts) renders to an empty string and ask_advisor still works
    /// (the snapshot block falls back to "(empty)" in build_messages).
    #[test]
    fn render_snapshot_skips_non_text_parts_robust() {
        let mut transcript = vec![ChatMessage::compact_boundary("summary", 1234)];
        transcript[0].parts.clear();
        transcript[0]
            .parts
            .push(MessagePart::CompactBoundary { pre_tokens: 1234 });

        let snap = render_snapshot(&transcript);
        assert!(snap.is_empty());
    }

    /// Robust: an oversized snapshot is tail-truncated and prefixed with the
    /// elision marker — the advisor still gets the *recent* context, which is
    /// what matters for "is this approach right?".
    #[test]
    fn render_snapshot_tail_truncates_oversized_input_robust() {
        // Build a transcript whose rendered form would dwarf MAX_SNAPSHOT_CHARS.
        let mut transcript = Vec::new();
        for i in 0..2_000 {
            transcript.push(ChatMessage::user(format!("padding-line-{i}")));
        }
        transcript.push(ChatMessage::user("FINAL_RECENT_LINE".into()));

        let snap = render_snapshot(&transcript);
        assert!(snap.len() <= MAX_SNAPSHOT_CHARS);
        assert!(snap.starts_with("[…earlier transcript elided…]"));
        // Tail-preservation: the most recent line survives.
        assert!(snap.contains("FINAL_RECENT_LINE"));
    }

    /// Robust: build_messages emits an empty-marker snapshot block when the
    /// main transcript is empty, so the advisor has a well-formed prompt
    /// even on a fresh session.
    #[test]
    fn build_messages_handles_empty_snapshot_robust() {
        let messages = build_messages("", "what should I do?");
        assert_eq!(messages.len(), 2);
        let ProviderContent::Text(s) = &messages[0].content[0] else {
            panic!("expected text");
        };
        assert!(s.contains("(empty"));
    }

    /// Robust: when the provider doesn't report token usage (zeros), we fall
    /// back to a char/4 estimate so the budget still progresses. Without
    /// this, a misconfigured provider gives the user infinite advisor calls.
    #[tokio::test]
    async fn ask_advisor_falls_back_to_char_estimate_when_no_usage_robust() {
        struct ZeroUsageProvider;
        #[async_trait]
        impl Provider for ZeroUsageProvider {
            fn name(&self) -> &str {
                "zero-usage"
            }
            fn available_models(&self) -> Vec<ModelInfo> {
                Vec::new()
            }
            async fn stream(&self, _: Vec<PMsg>, _: &SOpts) -> Result<EventStream> {
                Err(anyhow!("unused"))
            }
            async fn complete(&self, _: Vec<PMsg>, _: &SOpts) -> Result<CompletionResponse> {
                Ok(CompletionResponse {
                    content: "x".repeat(400), // 400 chars → ~100 tokens
                    usage: Default::default(),
                })
            }
        }
        impl jfc_provider::seal::Sealed for ZeroUsageProvider {}
        let mut session = AdvisorSession::new("test-model").with_budget(10_000);
        let _ = ask_advisor(&ZeroUsageProvider, &mut session, "hi".into(), &[])
            .await
            .expect("reply");
        // We can't pin the exact number (depends on snapshot/query length),
        // but it must be > 0 (budget moved) and not absurd.
        assert!(session.tokens_used > 0);
        assert!(session.tokens_used < 1_000);
    }
}

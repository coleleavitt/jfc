//! Model Council — multi-model deep-research arbitration.
//!
//! Mirrors Perplexity's `COUNCIL_RESEARCH` step (a.k.a. the ASI / "Model
//! council" surface gated behind Max). A single research question is fanned out
//! to N member models **in parallel**; each member returns an independent
//! answer ("model summary"). An arbiter model then synthesises the member
//! summaries into one consolidated answer that explicitly surfaces agreement
//! and disagreement between members.
//!
//! Design constraints (mirrors `crate::advisor`):
//! - Reuses `Provider::complete()` (with a stream-to-completion fallback) so it
//!   doesn't share the main agent's stream channel or token accounting.
//! - Each member call is a separate, tool-less prose completion.
//! - The whole council is bounded by a single [`CouncilBudget`] so a runaway
//!   fan-out can't drain the user's account.
//! - One member erroring does **not** tank the council: failures are collected
//!   and the arbiter synthesises from whatever succeeded. A council with zero
//!   successful members is the only hard error.
//!
//! This module is intentionally provider-agnostic: callers resolve a
//! `(Arc<dyn Provider>, ModelId)` per member (e.g. via
//! `runtime::bootstrap::resolve_provider_model`) and hand the council a list of
//! [`CouncilMember`]s. That keeps the orchestration logic unit-testable with
//! mock providers and free of registry coupling.

use std::{
    collections::{HashMap, hash_map::DefaultHasher},
    hash::{Hash, Hasher},
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use anyhow::{Result, anyhow};
use futures::future::join_all;
use serde::Serialize;
use tokio::time::timeout;

use jfc_provider::{
    CompletionResponse, ModelId, Provider, ProviderContent, ProviderMessage, ProviderRole,
    StreamOptions, TokenUsage,
};

/// Default max output tokens for a single member or arbiter completion.
const DEFAULT_MEMBER_MAX_TOKENS: u32 = 2048;
const DEFAULT_ARBITER_MAX_TOKENS: u32 = 3072;
const DEFAULT_MEMBER_TIMEOUT: Duration = Duration::from_secs(120);
const COUNCIL_ARCHIVE_KIND: &str = "council_archive";

/// Default token budget for one full council run (all members + arbiter).
/// Conservative — roughly a 3-member council with synthesis on a 200K model.
pub const DEFAULT_COUNCIL_BUDGET: u64 = 60_000;

/// System prompt handed to each council **member**. Each member answers
/// independently, in prose, with no tools and no knowledge of the other members.
const MEMBER_SYSTEM_PROMPT: &str = "\
You are one member of a model council convened to answer a research question. \
Answer the question directly and completely on your own. State your reasoning \
concisely, flag genuine uncertainty, and do not assume other models will cover \
anything — your answer must stand alone. You have no tools; answer from \
knowledge and the context provided.";

/// System prompt handed to the **arbiter**. It receives every member summary
/// and must synthesise — not merely concatenate — surfacing consensus and
/// conflicts.
const ARBITER_SYSTEM_PROMPT: &str = "\
You are the arbiter of a model council. You are given the original question and \
the independent answers of several member models. Produce one consolidated \
answer that: (1) leads with the best-supported conclusion, (2) explicitly notes \
where members AGREE (higher confidence) and where they DISAGREE or diverge \
(lower confidence, present the options), and (3) never invents agreement that \
isn't there. Prefer claims corroborated by multiple members. If members \
conflict on a fact, say so rather than silently picking one.";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CouncilIntent {
    Diagnose,
    Audit,
    Plan,
    Evaluate,
    Explain,
    Create,
    Perspectives,
    Freeform,
}

impl CouncilIntent {
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "diagnose" | "debug" | "root_cause" | "root-cause" => Some(Self::Diagnose),
            "audit" | "review" | "security" => Some(Self::Audit),
            "plan" | "design" | "architecture" => Some(Self::Plan),
            "evaluate" | "compare" | "choose" | "decision" => Some(Self::Evaluate),
            "explain" | "teach" | "summarize" | "summarise" => Some(Self::Explain),
            "create" | "draft" | "generate" => Some(Self::Create),
            "perspectives" | "perspective" | "brainstorm" => Some(Self::Perspectives),
            "freeform" | "general" | "default" => Some(Self::Freeform),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Diagnose => "diagnose",
            Self::Audit => "audit",
            Self::Plan => "plan",
            Self::Evaluate => "evaluate",
            Self::Explain => "explain",
            Self::Create => "create",
            Self::Perspectives => "perspectives",
            Self::Freeform => "freeform",
        }
    }

    fn member_instruction(self) -> &'static str {
        match self {
            Self::Diagnose => {
                "Intent: diagnose. Focus on likely causes, evidence, reproduction paths, and the smallest decisive next check."
            }
            Self::Audit => {
                "Intent: audit. Focus on concrete findings, severity, affected surface, evidence, and missing tests or guardrails."
            }
            Self::Plan => {
                "Intent: plan. Focus on implementation order, dependencies, tradeoffs, and risks that change the plan."
            }
            Self::Evaluate => {
                "Intent: evaluate. Compare options against explicit criteria and give a defensible recommendation."
            }
            Self::Explain => {
                "Intent: explain. Make the answer clear, accurate, and concise; call out assumptions."
            }
            Self::Create => {
                "Intent: create. Produce the requested artifact or design, plus only the constraints needed to use it."
            }
            Self::Perspectives => {
                "Intent: perspectives. Provide a distinct useful angle; avoid duplicating the obvious default view."
            }
            Self::Freeform => {
                "Intent: freeform. Answer naturally while preserving uncertainty and evidence."
            }
        }
    }

    fn arbiter_instruction(self) -> &'static str {
        match self {
            Self::Diagnose => {
                "Synthesis intent: diagnose. Lead with the most likely cause and the decisive evidence/checks."
            }
            Self::Audit => {
                "Synthesis intent: audit. Lead with actionable findings ordered by severity and confidence."
            }
            Self::Plan => {
                "Synthesis intent: plan. Lead with the recommended implementation order and risk controls."
            }
            Self::Evaluate => {
                "Synthesis intent: evaluate. Lead with the recommendation and why it beats alternatives."
            }
            Self::Explain => {
                "Synthesis intent: explain. Lead with the clearest correct explanation."
            }
            Self::Create => {
                "Synthesis intent: create. Deliver the requested artifact and reconcile member differences."
            }
            Self::Perspectives => {
                "Synthesis intent: perspectives. Preserve genuinely different useful viewpoints."
            }
            Self::Freeform => "Synthesis intent: freeform. Produce the best consolidated answer.",
        }
    }
}

/// A single council member: which provider to call and under what model id.
#[derive(Clone)]
pub struct CouncilMember {
    pub provider: Arc<dyn Provider>,
    pub model: ModelId,
    /// Optional human label (defaults to the model id in output).
    pub label: Option<String>,
}

impl CouncilMember {
    pub fn new(provider: Arc<dyn Provider>, model: impl Into<ModelId>) -> Self {
        Self {
            provider,
            model: model.into(),
            label: None,
        }
    }

    pub fn with_label(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }

    fn display_label(&self) -> String {
        self.label
            .clone()
            .unwrap_or_else(|| self.model.as_str().to_owned())
    }
}

/// The result of a single member's deliberation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MemberSummary {
    pub label: String,
    pub model: String,
    /// `Ok(answer)` on success; `Err(reason)` when the member failed.
    pub outcome: MemberOutcome,
    pub tokens_used: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub enum MemberOutcome {
    Answered(String),
    Failed(String),
}

impl MemberSummary {
    pub fn answer(&self) -> Option<&str> {
        match &self.outcome {
            MemberOutcome::Answered(a) => Some(a.as_str()),
            MemberOutcome::Failed(_) => None,
        }
    }

    pub fn succeeded(&self) -> bool {
        matches!(self.outcome, MemberOutcome::Answered(_))
    }
}

/// The full council deliberation: every member summary plus the arbiter's
/// synthesised final answer.
#[derive(Debug, Clone, Serialize)]
pub struct CouncilReport {
    pub question: String,
    pub members: Vec<MemberSummary>,
    /// The arbiter's consolidated answer.
    pub synthesis: String,
    pub tokens_used: u64,
    pub intent: Option<CouncilIntent>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub archive_dir: Option<PathBuf>,
}

impl CouncilReport {
    pub fn successful_members(&self) -> usize {
        self.members.iter().filter(|m| m.succeeded()).count()
    }

    /// Render the report as a Markdown block suitable for a chat surface:
    /// the synthesis first, then a collapsible-style per-member appendix.
    pub fn to_markdown(&self) -> String {
        let mut out = String::new();
        out.push_str("## Model Council\n\n");
        if let Some(intent) = self.intent {
            out.push_str(&format!("_Intent: {}_\n\n", intent.as_str()));
        }
        out.push_str(&self.synthesis);
        out.push_str("\n\n---\n");
        out.push_str(&format!(
            "_Council of {} ({} answered):_\n",
            self.members.len(),
            self.successful_members()
        ));
        for m in &self.members {
            match &m.outcome {
                MemberOutcome::Answered(_) => {
                    out.push_str(&format!("- ✅ `{}`\n", m.label));
                }
                MemberOutcome::Failed(reason) => {
                    out.push_str(&format!("- ⚠️ `{}` — {}\n", m.label, reason));
                }
            }
        }
        if let Some(dir) = &self.archive_dir {
            out.push_str(&format!("\n_Archive: `{}`_\n", dir.display()));
        }
        out
    }
}

/// A simple token budget for one council run. Shared across all members and the
/// arbiter; when exhausted before the arbiter runs, synthesis falls back to a
/// deterministic local merge rather than another model call.
#[derive(Debug, Clone)]
pub struct CouncilBudget {
    pub limit: u64,
    pub used: u64,
}

impl CouncilBudget {
    pub fn new(limit: u64) -> Self {
        Self { limit, used: 0 }
    }

    pub fn remaining(&self) -> u64 {
        self.limit.saturating_sub(self.used)
    }

    pub fn is_exhausted(&self) -> bool {
        self.used >= self.limit
    }

    fn record(&mut self, n: u64) {
        self.used = self.used.saturating_add(n);
    }
}

impl Default for CouncilBudget {
    fn default() -> Self {
        Self::new(DEFAULT_COUNCIL_BUDGET)
    }
}

/// Billable tokens for one member/arbiter call via the shared canonical
/// derivation ([`TokenUsage::billable_tokens`]) so the council budget gate
/// can't drift from the advisor budget and economy ledger on the boundary
/// token. The provenance flag is unused here — the council budget treats an
/// estimate the same as a reported count.
fn estimate_tokens(usage: &TokenUsage, fallback_chars: usize) -> u64 {
    usage.billable_tokens(fallback_chars).0
}

fn member_messages(
    question: &str,
    context: Option<&str>,
    intent: Option<CouncilIntent>,
) -> Vec<ProviderMessage> {
    let mut messages = Vec::new();
    if let Some(ctx) = context.filter(|c| !c.trim().is_empty()) {
        messages.push(ProviderMessage {
            role: ProviderRole::User,
            content: vec![ProviderContent::Text(format!(
                "<context>\n{ctx}\n</context>"
            ))],
        });
    }
    if let Some(intent) = intent {
        messages.push(ProviderMessage {
            role: ProviderRole::User,
            content: vec![ProviderContent::Text(
                intent.member_instruction().to_owned(),
            )],
        });
    }
    messages.push(ProviderMessage {
        role: ProviderRole::User,
        content: vec![ProviderContent::Text(format!("Question: {question}"))],
    });
    messages
}

fn arbiter_messages(
    question: &str,
    members: &[MemberSummary],
    intent: Option<CouncilIntent>,
) -> Vec<ProviderMessage> {
    let mut block = format!("Original question:\n{question}\n\nMember answers:\n");
    for (i, m) in members.iter().enumerate() {
        if let MemberOutcome::Answered(answer) = &m.outcome {
            block.push_str(&format!(
                "\n--- Member {} ({}) ---\n{}\n",
                i + 1,
                m.label,
                answer
            ));
        }
    }
    if let Some(intent) = intent {
        block.push_str(&format!("\n{}\n", intent.arbiter_instruction()));
    }
    block.push_str("\nSynthesise these into one consolidated answer.");
    vec![ProviderMessage {
        role: ProviderRole::User,
        content: vec![ProviderContent::Text(block)],
    }]
}

/// Run a single tool-less completion via the shared one-shot executor
/// ([`crate::prompt_executor::complete_once`]): native `complete()` with a
/// stream-to-completion fallback for providers that don't implement it.
async fn complete_with_fallback(
    provider: &dyn Provider,
    messages: Vec<ProviderMessage>,
    opts: &StreamOptions,
) -> Result<CompletionResponse> {
    crate::prompt_executor::complete_once(provider, messages, opts).await
}

/// Configuration for a council run.
#[derive(Debug, Clone)]
pub struct CouncilRunOptions {
    pub quorum: Option<usize>,
    pub retry_on_fail: u32,
    pub member_timeout: Option<Duration>,
    pub archive: bool,
    pub archive_root: Option<PathBuf>,
    pub intent: Option<CouncilIntent>,
}

impl Default for CouncilRunOptions {
    fn default() -> Self {
        Self {
            quorum: None,
            retry_on_fail: 0,
            member_timeout: Some(DEFAULT_MEMBER_TIMEOUT),
            archive: false,
            archive_root: None,
            intent: None,
        }
    }
}

pub struct CouncilRequest {
    pub question: String,
    /// Optional shared context (e.g. a transcript snapshot or research notes).
    pub context: Option<String>,
    pub members: Vec<CouncilMember>,
    /// Which member resolves the synthesis. If `None`, the first member is used.
    pub arbiter: Option<CouncilMember>,
    pub budget: CouncilBudget,
    pub options: CouncilRunOptions,
}

impl CouncilRequest {
    pub fn new(question: impl Into<String>, members: Vec<CouncilMember>) -> Self {
        Self {
            question: question.into(),
            context: None,
            members,
            arbiter: None,
            budget: CouncilBudget::default(),
            options: CouncilRunOptions::default(),
        }
    }

    pub fn with_context(mut self, context: impl Into<String>) -> Self {
        self.context = Some(context.into());
        self
    }

    pub fn with_arbiter(mut self, arbiter: CouncilMember) -> Self {
        self.arbiter = Some(arbiter);
        self
    }

    pub fn with_budget(mut self, budget: u64) -> Self {
        self.budget = CouncilBudget::new(budget);
        self
    }

    pub fn with_quorum(mut self, quorum: Option<usize>) -> Self {
        self.options.quorum = quorum;
        self
    }

    pub fn with_retry_on_fail(mut self, retry_on_fail: u32) -> Self {
        self.options.retry_on_fail = retry_on_fail;
        self
    }

    pub fn with_member_timeout(mut self, timeout: Option<Duration>) -> Self {
        self.options.member_timeout = timeout;
        self
    }

    pub fn with_archive(mut self, archive: bool, root: Option<PathBuf>) -> Self {
        self.options.archive = archive;
        self.options.archive_root = root;
        self
    }

    pub fn with_intent(mut self, intent: Option<CouncilIntent>) -> Self {
        self.options.intent = intent;
        self
    }

    pub fn with_intent_str(mut self, intent: Option<&str>) -> Self {
        self.options.intent = intent.and_then(CouncilIntent::parse);
        self
    }
}

/// Deliberate a single council member: one tool-less completion, mapped into a
/// [`MemberSummary`] (failures captured, never propagated).
async fn deliberate_member(
    member: CouncilMember,
    question: &str,
    context: Option<&str>,
    intent: Option<CouncilIntent>,
    timeout_after: Option<Duration>,
    retry_on_fail: u32,
) -> MemberSummary {
    let label = member.display_label();
    let model = member.model.as_str().to_owned();
    let mut attempt = 0;
    loop {
        let messages = member_messages(question, context, intent);
        let opts = StreamOptions::new(member.model.clone())
            .system(MEMBER_SYSTEM_PROMPT)
            .max_tokens(DEFAULT_MEMBER_MAX_TOKENS);
        let call = complete_with_fallback(member.provider.as_ref(), messages, &opts);
        let outcome = match timeout_after {
            Some(duration) => match timeout(duration, call).await {
                Ok(result) => result,
                Err(_) => Err(anyhow!("timed out after {}ms", duration.as_millis().max(1))),
            },
            None => call.await,
        };

        match outcome {
            Ok(resp) => {
                let used = estimate_tokens(&resp.usage, question.len() + resp.content.len());
                return MemberSummary {
                    label,
                    model,
                    outcome: MemberOutcome::Answered(resp.content),
                    tokens_used: used,
                };
            }
            Err(e) if attempt < retry_on_fail => {
                attempt += 1;
                tracing::warn!(
                    target: "jfc::council",
                    member = %label,
                    attempt,
                    error = %e,
                    "council member failed; retrying"
                );
            }
            Err(e) => {
                let reason = if attempt == 0 {
                    e.to_string()
                } else {
                    format!("{} (after {} retries)", e, attempt)
                };
                return MemberSummary {
                    label,
                    model,
                    outcome: MemberOutcome::Failed(reason),
                    tokens_used: 0,
                };
            }
        }
    }
}

/// Fan out to every member in parallel and record their usage against the budget.
async fn fan_out_members(
    members: &[CouncilMember],
    question: &str,
    context: Option<&str>,
    options: &CouncilRunOptions,
    budget: &mut CouncilBudget,
) -> Vec<MemberSummary> {
    let futures = members.iter().cloned().map(|m| {
        deliberate_member(
            m,
            question,
            context,
            options.intent,
            options.member_timeout,
            options.retry_on_fail,
        )
    });
    let summaries: Vec<MemberSummary> = join_all(futures).await;
    for s in &summaries {
        budget.record(s.tokens_used);
    }
    summaries
}

fn council_member_output_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "findings": {
                "type": "array",
                "items": { "type": "string" }
            },
            "evidence": {
                "type": "array",
                "items": { "type": "string" }
            },
            "confidence": {
                "type": "string",
                "enum": ["low", "medium", "high"]
            },
            "recommendation": { "type": "string" }
        },
        "required": ["findings", "evidence", "confidence", "recommendation"],
        "additionalProperties": true
    })
}

fn council_member_agent_def() -> crate::agents::AgentDef {
    crate::agents::AgentDef {
        name: "CouncilMember".to_owned(),
        source: PathBuf::from("builtin:council-member"),
        model: None,
        isolation: None,
        skills: Vec::new(),
        allowed_tools: vec![
            "Read".to_owned(),
            "Glob".to_owned(),
            "Grep".to_owned(),
            "LSP".to_owned(),
            "StructuredOutput".to_owned(),
        ],
        disallowed_tools: vec![
            "Write".to_owned(),
            "Edit".to_owned(),
            "MultiEdit".to_owned(),
            "ApplyPatch".to_owned(),
            "Bash".to_owned(),
            "Task".to_owned(),
        ],
        permission_mode: None,
        forks_parent_context: None,
        background: Some(false),
        color: None,
        effort: None,
        max_turns: Some(8),
        max_input_tokens: None,
        memory: None,
        mcp_servers: Vec::new(),
        hooks: HashMap::new(),
        key_trigger: None,
        use_when: Vec::new(),
        avoid_when: Vec::new(),
        cost: None,
        system_prompt: "\
You are a read-only member of a model council. Inspect the repository only when it improves the answer. \
Use Read, Glob, Grep, and LSP for evidence; do not edit files, run shell commands, or spawn agents. \
Return your final answer by calling StructuredOutput with findings, evidence, confidence, and recommendation."
            .to_owned(),
    }
}

fn agentic_member_prompt(
    question: &str,
    context: Option<&str>,
    intent: Option<CouncilIntent>,
    label: &str,
) -> String {
    let mut prompt = String::new();
    prompt.push_str(&format!("Council member: {label}\n\n"));
    if let Some(intent) = intent {
        prompt.push_str(intent.member_instruction());
        prompt.push_str("\n\n");
    }
    if let Some(ctx) = context.filter(|c| !c.trim().is_empty()) {
        prompt.push_str("<shared_context>\n");
        prompt.push_str(ctx);
        prompt.push_str("\n</shared_context>\n\n");
    }
    prompt.push_str("Question:\n");
    prompt.push_str(question);
    prompt.push_str(
        "\n\nInvestigate independently. Use read-only tools when useful. \
Return StructuredOutput with concise findings, concrete evidence, a low/medium/high confidence value, and a recommendation.",
    );
    prompt
}

async fn deliberate_agentic_member(
    member: CouncilMember,
    question: &str,
    context: Option<&str>,
    options: &CouncilRunOptions,
    task_store: Option<Arc<jfc_session::TaskStore>>,
    active_team_name: Option<String>,
    cwd: PathBuf,
) -> MemberSummary {
    let label = member.display_label();
    let model = member.model.as_str().to_owned();
    let agent_def = council_member_agent_def();
    let schema = council_member_output_schema();
    let mut attempt = 0;

    loop {
        let task_input = crate::types::TaskInput {
            description: format!("Council member `{label}`"),
            prompt: agentic_member_prompt(question, context, options.intent, &label),
            subagent_type: Some("CouncilMember".to_owned()),
            category: Some("audit".to_owned()),
            run_in_background: false,
            model: Some("inherit".to_owned()),
            launcher: None,
            effort: None,
            name: None,
            team_name: None,
            mode: Some("read-only".to_owned()),
            isolation: None,
            parent_task_id: None,
            schema: Some(schema.clone()),
            allowed_tools: vec![
                "Read".to_owned(),
                "Glob".to_owned(),
                "Grep".to_owned(),
                "LSP".to_owned(),
                "Search".to_owned(),
                "ToolSearch".to_owned(),
                "ToolSuggest".to_owned(),
                "ListMcpResources".to_owned(),
                "ReadMcpResource".to_owned(),
                "StructuredOutput".to_owned(),
            ],
            disallowed_tools: Vec::new(),
            cwd: None,
        };
        let call = crate::tools::execute_task(
            &task_input,
            member.provider.as_ref(),
            member.model.clone(),
            None,
            None,
            Some(&agent_def),
            Some(cwd.clone()),
            task_store.clone(),
            active_team_name.as_deref(),
        );
        let result = match options.member_timeout {
            Some(duration) => match timeout(duration, call).await {
                Ok(result) => result,
                Err(_) => crate::runtime::ExecutionResult::failure(format!(
                    "timed out after {}ms",
                    duration.as_millis().max(1)
                )),
            },
            None => call.await,
        };

        if !result.is_error() {
            let output = result.output;
            let answer = match serde_json::from_str::<serde_json::Value>(&output) {
                Ok(value) => serde_json::to_string_pretty(&value).unwrap_or(output),
                Err(_) => output,
            };
            // The task-backed path returns no token usage, so the budget gate
            // was previously blind to agentic members (always 0). Estimate from
            // the prompt + answer text via the same chars/4 fallback the direct
            // path uses, so a runaway agentic fan-out is still bounded.
            let prompt = agentic_member_prompt(question, context, options.intent, &label);
            let fallback_chars = prompt.len() + answer.len();
            let tokens_used = estimate_tokens(&jfc_provider::TokenUsage::default(), fallback_chars);
            return MemberSummary {
                label,
                model,
                outcome: MemberOutcome::Answered(answer),
                tokens_used,
            };
        }

        if attempt < options.retry_on_fail {
            attempt += 1;
            tracing::warn!(
                target: "jfc::council",
                member = %label,
                attempt,
                error = %result.output,
                "agentic council member failed; retrying"
            );
            continue;
        }

        let reason = if attempt == 0 {
            result.output
        } else {
            format!("{} (after {} retries)", result.output, attempt)
        };
        return MemberSummary {
            label,
            model,
            outcome: MemberOutcome::Failed(reason),
            tokens_used: 0,
        };
    }
}

async fn fan_out_agentic_members(
    members: &[CouncilMember],
    question: &str,
    context: Option<&str>,
    options: &CouncilRunOptions,
    task_store: Option<Arc<jfc_session::TaskStore>>,
    active_team_name: Option<&str>,
    cwd: PathBuf,
    budget: &mut CouncilBudget,
) -> Vec<MemberSummary> {
    let team = active_team_name.map(str::to_owned);
    let futures = members.iter().cloned().map(|m| {
        deliberate_agentic_member(
            m,
            question,
            context,
            options,
            task_store.clone(),
            team.clone(),
            cwd.clone(),
        )
    });
    let summaries: Vec<MemberSummary> = join_all(futures).await;
    for s in &summaries {
        budget.record(s.tokens_used);
    }
    summaries
}

/// Resolve the final synthesis. Calls the arbiter model unless there's nothing
/// to arbitrate (single answer) or the budget is spent; arbiter failure falls
/// back to a deterministic local merge.
async fn synthesize(
    question: &str,
    members: &[MemberSummary],
    arbiter: CouncilMember,
    intent: Option<CouncilIntent>,
    budget: &mut CouncilBudget,
) -> String {
    let messages = arbiter_messages(question, members, intent);
    let opts = StreamOptions::new(arbiter.model.clone())
        .system(ARBITER_SYSTEM_PROMPT)
        .max_tokens(DEFAULT_ARBITER_MAX_TOKENS);
    match complete_with_fallback(arbiter.provider.as_ref(), messages, &opts).await {
        Ok(resp) => {
            budget.record(estimate_tokens(
                &resp.usage,
                question.len() + resp.content.len(),
            ));
            resp.content
        }
        Err(e) => {
            tracing::warn!(
                target: "jfc::council",
                error = %e,
                "arbiter synthesis failed; using local merge"
            );
            local_synthesis(question, members)
        }
    }
}

fn archive_id(question: &str) -> String {
    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or_default();
    let mut hasher = DefaultHasher::new();
    question.hash(&mut hasher);
    format!("{now_ms}-{:016x}", hasher.finish())
}

async fn archive_council_report(root: &Path, report: &CouncilReport) -> Result<PathBuf> {
    let id = archive_id(&report.question);
    let meta = serde_json::json!({
        "schema_version": 1,
        "id": id,
        "question": &report.question,
        "intent": report.intent.map(CouncilIntent::as_str),
        "member_count": report.members.len(),
        "successful_members": report.successful_members(),
        "tokens_used": report.tokens_used,
        "synthesis": &report.synthesis,
        "members": &report.members,
    });
    let session_id = format!("project:{}", jfc_knowledge::project_key(root));
    let value_json = serde_json::to_string(&meta)?;
    let store = jfc_knowledge::KnowledgeStore::open_default().await?;
    store
        .upsert_session_artifact(&session_id, COUNCIL_ARCHIVE_KIND, &id, &value_json)
        .await?;
    Ok(PathBuf::from(format!("db:council:{id}")))
}

/// Run the full council: fan out to members in parallel, then synthesise.
///
/// Returns `Err` only when the request is empty (no question or no members) or
/// when **every** member failed (nothing to synthesise).
#[tracing::instrument(
    target = "jfc::council",
    skip(request),
    fields(
        members = request.members.len(),
        budget = request.budget.limit,
    ),
)]
pub async fn run_council(mut request: CouncilRequest) -> Result<CouncilReport> {
    let question = request.question.trim().to_owned();
    if question.is_empty() {
        return Err(anyhow!("council question is empty"));
    }
    if request.members.is_empty() {
        return Err(anyhow!("council has no members"));
    }

    let context = request.context.clone();
    let members = fan_out_members(
        &request.members,
        &question,
        context.as_deref(),
        &request.options,
        &mut request.budget,
    )
    .await;

    let answered = members.iter().filter(|m| m.succeeded()).count();
    if answered == 0 {
        return Err(anyhow!("all {} council members failed", members.len()));
    }
    let quorum = request.options.quorum.unwrap_or(1).max(1);
    if answered < quorum {
        return Err(anyhow!(
            "council quorum not met: {answered}/{quorum} members answered"
        ));
    }

    // One answer or no budget left → local merge; otherwise pay for arbitration.
    let synthesis = if answered == 1 || request.budget.is_exhausted() {
        local_synthesis(&question, &members)
    } else {
        let arbiter = request
            .arbiter
            .clone()
            .or_else(|| request.members.first().cloned())
            .expect("members non-empty checked above");
        synthesize(
            &question,
            &members,
            arbiter,
            request.options.intent,
            &mut request.budget,
        )
        .await
    };

    let mut report = CouncilReport {
        question,
        members,
        synthesis,
        tokens_used: request.budget.used,
        intent: request.options.intent,
        archive_dir: None,
    };

    if request.options.archive {
        let root = request
            .options
            .archive_root
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
        match archive_council_report(&root, &report).await {
            Ok(dir) => report.archive_dir = Some(dir),
            Err(e) => tracing::warn!(
                target: "jfc::council",
                error = %e,
                "failed to archive council run"
            ),
        }
    }

    Ok(report)
}

/// Run the council with each member as a read-only task-backed subagent. This
/// is slower than [`run_council`] but lets members inspect the codebase through
/// Read/Glob/Grep/LSP before returning a schema-validated finding bundle.
#[tracing::instrument(
    target = "jfc::council",
    skip(request, task_store),
    fields(
        members = request.members.len(),
        budget = request.budget.limit,
    ),
)]
pub async fn run_agentic_council(
    mut request: CouncilRequest,
    task_store: Option<Arc<jfc_session::TaskStore>>,
    active_team_name: Option<&str>,
    cwd: PathBuf,
) -> Result<CouncilReport> {
    let question = request.question.trim().to_owned();
    if question.is_empty() {
        return Err(anyhow!("council question is empty"));
    }
    if request.members.is_empty() {
        return Err(anyhow!("council has no members"));
    }

    let context = request.context.clone();
    let members = fan_out_agentic_members(
        &request.members,
        &question,
        context.as_deref(),
        &request.options,
        task_store,
        active_team_name,
        cwd.clone(),
        &mut request.budget,
    )
    .await;

    let answered = members.iter().filter(|m| m.succeeded()).count();
    if answered == 0 {
        return Err(anyhow!("all {} council members failed", members.len()));
    }
    let quorum = request.options.quorum.unwrap_or(1).max(1);
    if answered < quorum {
        return Err(anyhow!(
            "council quorum not met: {answered}/{quorum} members answered"
        ));
    }

    let synthesis = if answered == 1 || request.budget.is_exhausted() {
        local_synthesis(&question, &members)
    } else {
        let arbiter = request
            .arbiter
            .clone()
            .or_else(|| request.members.first().cloned())
            .expect("members non-empty checked above");
        synthesize(
            &question,
            &members,
            arbiter,
            request.options.intent,
            &mut request.budget,
        )
        .await
    };

    let mut report = CouncilReport {
        question,
        members,
        synthesis,
        tokens_used: request.budget.used,
        intent: request.options.intent,
        archive_dir: None,
    };

    if request.options.archive {
        let root = request.options.archive_root.unwrap_or(cwd);
        match archive_council_report(&root, &report).await {
            Ok(dir) => report.archive_dir = Some(dir),
            Err(e) => tracing::warn!(
                target: "jfc::council",
                error = %e,
                "failed to archive agentic council run"
            ),
        }
    }

    Ok(report)
}

/// Deterministic fallback synthesis: concatenate the successful member answers
/// under labelled headings. Used when only one member answered, the budget is
/// exhausted, or the arbiter call failed.
fn local_synthesis(_question: &str, members: &[MemberSummary]) -> String {
    let answered: Vec<&MemberSummary> = members.iter().filter(|m| m.succeeded()).collect();
    if answered.len() == 1 {
        return answered[0].answer().unwrap_or_default().to_owned();
    }
    let mut out =
        String::from("_(Arbiter unavailable — showing member answers without synthesis.)_\n");
    for m in answered {
        out.push_str(&format!(
            "\n### {}\n{}\n",
            m.label,
            m.answer().unwrap_or_default()
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Result;
    use async_trait::async_trait;
    use jfc_provider::{
        EventStream, ModelInfo, ProviderMessage as PMsg, StreamConvention, StreamOptions as SOpts,
    };
    use std::sync::Mutex;

    /// Mock provider that returns a canned answer keyed by a counter, so each
    /// `complete()` call can yield a distinct response. Modelled on
    /// `advisor::tests::FakeProvider`.
    struct ScriptedProvider {
        name: &'static str,
        replies: Mutex<Vec<Result<String>>>,
    }

    impl ScriptedProvider {
        fn answering(name: &'static str, reply: &str) -> Arc<Self> {
            Arc::new(Self {
                name,
                replies: Mutex::new(vec![Ok(reply.to_owned())]),
            })
        }

        fn failing(name: &'static str, err: &str) -> Arc<Self> {
            let e = err.to_owned();
            Arc::new(Self {
                name,
                replies: Mutex::new(vec![Err(anyhow!("{e}"))]),
            })
        }

        /// A provider that can be called multiple times, popping replies in
        /// order and repeating the last one after exhaustion.
        fn sequence(name: &'static str, replies: Vec<&str>) -> Arc<Self> {
            Arc::new(Self {
                name,
                replies: Mutex::new(replies.into_iter().map(|s| Ok(s.to_owned())).collect()),
            })
        }

        fn sequence_results(name: &'static str, replies: Vec<Result<&str>>) -> Arc<Self> {
            Arc::new(Self {
                name,
                replies: Mutex::new(
                    replies
                        .into_iter()
                        .map(|result| result.map(str::to_owned))
                        .collect(),
                ),
            })
        }
    }

    #[async_trait]
    impl Provider for ScriptedProvider {
        fn name(&self) -> &str {
            self.name
        }
        fn available_models(&self) -> Vec<ModelInfo> {
            Vec::new()
        }
        fn stream_convention(&self) -> StreamConvention {
            StreamConvention::AnthropicNative
        }
        async fn stream(&self, _messages: Vec<PMsg>, _options: &SOpts) -> Result<EventStream> {
            Err(anyhow!("stream not used in council tests"))
        }
        async fn complete(
            &self,
            _messages: Vec<PMsg>,
            _options: &SOpts,
        ) -> Result<CompletionResponse> {
            let mut guard = self.replies.lock().unwrap();
            let next = if guard.len() > 1 {
                guard.remove(0)
            } else {
                // Keep the last reply for repeated calls (e.g. arbiter reuses a
                // member provider).
                match guard.first() {
                    Some(Ok(s)) => Ok(s.clone()),
                    Some(Err(e)) => Err(anyhow!("{e}")),
                    None => Err(anyhow!("no scripted reply")),
                }
            };
            next.map(|content| CompletionResponse {
                content,
                usage: TokenUsage {
                    input_tokens: 50,
                    output_tokens: 25,
                    thinking_tokens: None,
                    cache_read_tokens: 0,
                    cache_creation_tokens: 0,
                },
                context_signals: None,
                reasoning: None,
            })
        }
    }
    impl jfc_provider::seal::Sealed for ScriptedProvider {}

    fn member(name: &'static str, model: &str, reply: &str) -> CouncilMember {
        CouncilMember::new(ScriptedProvider::answering(name, reply), model)
    }

    // ── Normal paths ────────────────────────────────────────────────────────

    #[tokio::test]
    async fn run_council_synthesises_member_answers_normal() {
        let members = vec![
            member(
                "p1",
                "model-a",
                "The sky is blue due to Rayleigh scattering.",
            ),
            member(
                "p2",
                "model-b",
                "Blue light scatters more in the atmosphere.",
            ),
        ];
        // Dedicated arbiter that returns a synthesis string.
        let arbiter = CouncilMember::new(
            ScriptedProvider::answering(
                "arb",
                "Consensus: Rayleigh scattering makes the sky blue.",
            ),
            "arbiter-model",
        );
        let req = CouncilRequest::new("Why is the sky blue?", members).with_arbiter(arbiter);

        let report = run_council(req).await.expect("council should succeed");
        assert_eq!(report.members.len(), 2);
        assert_eq!(report.successful_members(), 2);
        assert!(report.synthesis.contains("Rayleigh scattering"));
        assert!(report.tokens_used > 0);
    }

    #[tokio::test]
    async fn run_council_one_member_skips_arbiter_normal() {
        // Single answering member → local synthesis returns its answer verbatim,
        // no arbiter call.
        let members = vec![member("p1", "model-a", "Forty-two.")];
        let req = CouncilRequest::new("Answer?", members);
        let report = run_council(req).await.expect("ok");
        assert_eq!(report.successful_members(), 1);
        assert_eq!(report.synthesis, "Forty-two.");
    }

    #[tokio::test]
    async fn run_council_markdown_lists_members_normal() {
        let members = vec![
            member("p1", "model-a", "Answer A"),
            member("p2", "model-b", "Answer B"),
        ];
        let arbiter = CouncilMember::new(
            ScriptedProvider::answering("arb", "Synthesised."),
            "arbiter-model",
        );
        let report = run_council(CouncilRequest::new("Q?", members).with_arbiter(arbiter))
            .await
            .unwrap();
        let md = report.to_markdown();
        assert!(md.contains("## Model Council"));
        assert!(md.contains("Synthesised."));
        assert!(md.contains("`model-a`"));
        assert!(md.contains("`model-b`"));
    }

    // ── Robust paths ────────────────────────────────────────────────────────

    #[tokio::test]
    async fn run_council_tolerates_one_member_failure_robust() {
        let members = vec![
            CouncilMember::new(ScriptedProvider::failing("p1", "network down"), "model-a"),
            member("p2", "model-b", "I still have an answer."),
        ];
        let arbiter = CouncilMember::new(
            ScriptedProvider::answering("arb", "Synthesis from the survivor."),
            "arbiter-model",
        );
        // 2 members but only 1 answered → answered==1 → local synthesis returns
        // the survivor's answer verbatim (no arbiter needed).
        let report = run_council(CouncilRequest::new("Q?", members).with_arbiter(arbiter))
            .await
            .expect("council survives a single failure");
        assert_eq!(report.successful_members(), 1);
        assert_eq!(report.synthesis, "I still have an answer.");
        // The failed member is still recorded for transparency.
        assert!(report.members.iter().any(|m| !m.succeeded()));
    }

    #[tokio::test]
    async fn run_council_all_members_fail_is_error_robust() {
        let members = vec![
            CouncilMember::new(ScriptedProvider::failing("p1", "boom"), "model-a"),
            CouncilMember::new(ScriptedProvider::failing("p2", "boom"), "model-b"),
        ];
        let err = run_council(CouncilRequest::new("Q?", members))
            .await
            .expect_err("all-fail must error");
        assert!(err.to_string().contains("all 2 council members failed"));
    }

    #[tokio::test]
    async fn run_council_retries_failed_member_robust() {
        let members = vec![CouncilMember::new(
            ScriptedProvider::sequence_results(
                "p1",
                vec![Err(anyhow!("transient")), Ok("Recovered answer.")],
            ),
            "model-a",
        )];
        let report = run_council(CouncilRequest::new("Q?", members).with_retry_on_fail(1))
            .await
            .expect("retry should recover");
        assert_eq!(report.successful_members(), 1);
        assert_eq!(report.synthesis, "Recovered answer.");
    }

    #[tokio::test]
    async fn run_council_quorum_failure_is_error_robust() {
        let members = vec![
            CouncilMember::new(ScriptedProvider::failing("p1", "boom"), "model-a"),
            member("p2", "model-b", "Only survivor."),
        ];
        let err = run_council(CouncilRequest::new("Q?", members).with_quorum(Some(2)))
            .await
            .expect_err("quorum should fail");
        assert!(err.to_string().contains("quorum not met"));
    }

    #[tokio::test]
    async fn run_council_archives_artifact_bundle_normal() {
        let root = std::env::temp_dir().join(format!(
            "jfc-council-test-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or_default()
        ));
        let members = vec![member("p1", "model-a", "Archived answer.")];
        let report = run_council(
            CouncilRequest::new("Archive this?", members)
                .with_intent(Some(CouncilIntent::Audit))
                .with_archive(true, Some(root.clone())),
        )
        .await
        .expect("archive run succeeds");
        let archive_handle = report.archive_dir.as_ref().expect("archive handle");
        let archive_id = archive_handle
            .to_string_lossy()
            .strip_prefix("db:council:")
            .expect("db council handle")
            .to_owned();
        let row = jfc_knowledge::KnowledgeStore::open_default()
            .await
            .unwrap()
            .get_session_artifact(
                &format!("project:{}", jfc_knowledge::project_key(&root)),
                COUNCIL_ARCHIVE_KIND,
                &archive_id,
            )
            .await
            .unwrap()
            .expect("db council archive");
        assert!(row.value_json.contains("Archived answer."));
        assert!(report.to_markdown().contains("_Intent: audit_"));
    }

    #[tokio::test]
    async fn run_council_empty_question_is_error_robust() {
        let members = vec![member("p1", "model-a", "x")];
        let err = run_council(CouncilRequest::new("   ", members))
            .await
            .expect_err("empty question");
        assert!(err.to_string().contains("empty"));
    }

    #[tokio::test]
    async fn run_council_no_members_is_error_robust() {
        let err = run_council(CouncilRequest::new("Q?", Vec::new()))
            .await
            .expect_err("no members");
        assert!(err.to_string().contains("no members"));
    }

    #[tokio::test]
    async fn run_council_exhausted_budget_uses_local_merge_robust() {
        // Tiny budget so it's exhausted after members → arbiter is skipped and
        // local merge runs even with 2 answers.
        let members = vec![
            member("p1", "model-a", "Answer A"),
            member("p2", "model-b", "Answer B"),
        ];
        let arbiter = CouncilMember::new(
            ScriptedProvider::sequence("arb", vec!["SHOULD-NOT-BE-CALLED"]),
            "arbiter-model",
        );
        let req = CouncilRequest::new("Q?", members)
            .with_arbiter(arbiter)
            .with_budget(1); // exhausted by member usage
        let report = run_council(req).await.unwrap();
        // Local merge appendix marker, not the arbiter string.
        assert!(report.synthesis.contains("Arbiter unavailable"));
        assert!(report.synthesis.contains("Answer A"));
        assert!(report.synthesis.contains("Answer B"));
    }

    #[test]
    fn budget_tracks_and_exhausts_normal() {
        let mut b = CouncilBudget::new(100);
        assert_eq!(b.remaining(), 100);
        b.record(40);
        assert_eq!(b.remaining(), 60);
        assert!(!b.is_exhausted());
        b.record(70);
        assert_eq!(b.remaining(), 0);
        assert!(b.is_exhausted());
    }
}

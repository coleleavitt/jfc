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

use std::sync::Arc;

use anyhow::{Result, anyhow};
use futures::future::join_all;

use jfc_provider::{
    CompletionResponse, ModelId, Provider, ProviderContent, ProviderMessage, ProviderRole,
    StreamOptions, TokenUsage,
};

/// Default max output tokens for a single member or arbiter completion.
const DEFAULT_MEMBER_MAX_TOKENS: u32 = 2048;
const DEFAULT_ARBITER_MAX_TOKENS: u32 = 3072;

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
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemberSummary {
    pub label: String,
    pub model: String,
    /// `Ok(answer)` on success; `Err(reason)` when the member failed.
    pub outcome: MemberOutcome,
    pub tokens_used: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
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
#[derive(Debug, Clone)]
pub struct CouncilReport {
    pub question: String,
    pub members: Vec<MemberSummary>,
    /// The arbiter's consolidated answer.
    pub synthesis: String,
    pub tokens_used: u64,
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

fn member_messages(question: &str, context: Option<&str>) -> Vec<ProviderMessage> {
    let mut messages = Vec::new();
    if let Some(ctx) = context.filter(|c| !c.trim().is_empty()) {
        messages.push(ProviderMessage {
            role: ProviderRole::User,
            content: vec![ProviderContent::Text(format!(
                "<context>\n{ctx}\n</context>"
            ))],
        });
    }
    messages.push(ProviderMessage {
        role: ProviderRole::User,
        content: vec![ProviderContent::Text(format!("Question: {question}"))],
    });
    messages
}

fn arbiter_messages(question: &str, members: &[MemberSummary]) -> Vec<ProviderMessage> {
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
pub struct CouncilRequest {
    pub question: String,
    /// Optional shared context (e.g. a transcript snapshot or research notes).
    pub context: Option<String>,
    pub members: Vec<CouncilMember>,
    /// Which member resolves the synthesis. If `None`, the first member is used.
    pub arbiter: Option<CouncilMember>,
    pub budget: CouncilBudget,
}

impl CouncilRequest {
    pub fn new(question: impl Into<String>, members: Vec<CouncilMember>) -> Self {
        Self {
            question: question.into(),
            context: None,
            members,
            arbiter: None,
            budget: CouncilBudget::default(),
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
}

/// Deliberate a single council member: one tool-less completion, mapped into a
/// [`MemberSummary`] (failures captured, never propagated).
async fn deliberate_member(
    member: CouncilMember,
    question: &str,
    context: Option<&str>,
) -> MemberSummary {
    let label = member.display_label();
    let model = member.model.as_str().to_owned();
    let messages = member_messages(question, context);
    let opts = StreamOptions::new(member.model.clone())
        .system(MEMBER_SYSTEM_PROMPT)
        .max_tokens(DEFAULT_MEMBER_MAX_TOKENS);
    match complete_with_fallback(member.provider.as_ref(), messages, &opts).await {
        Ok(resp) => {
            let used = estimate_tokens(&resp.usage, question.len() + resp.content.len());
            MemberSummary {
                label,
                model,
                outcome: MemberOutcome::Answered(resp.content),
                tokens_used: used,
            }
        }
        Err(e) => MemberSummary {
            label,
            model,
            outcome: MemberOutcome::Failed(e.to_string()),
            tokens_used: 0,
        },
    }
}

/// Fan out to every member in parallel and record their usage against the budget.
async fn fan_out_members(
    members: &[CouncilMember],
    question: &str,
    context: Option<&str>,
    budget: &mut CouncilBudget,
) -> Vec<MemberSummary> {
    let futures = members
        .iter()
        .cloned()
        .map(|m| deliberate_member(m, question, context));
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
    budget: &mut CouncilBudget,
) -> String {
    let messages = arbiter_messages(question, members);
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
        &mut request.budget,
    )
    .await;

    let answered = members.iter().filter(|m| m.succeeded()).count();
    if answered == 0 {
        return Err(anyhow!("all {} council members failed", members.len()));
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
        synthesize(&question, &members, arbiter, &mut request.budget).await
    };

    Ok(CouncilReport {
        question,
        members,
        synthesis,
        tokens_used: request.budget.used,
    })
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
                    cache_read_tokens: 0,
                    cache_creation_tokens: 0,
                },
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

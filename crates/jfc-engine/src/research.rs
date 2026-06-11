//! Deep-research orchestrator — clarify → plan → multi-step search → synthesize.
//!
//! Mirrors Perplexity's `/rest/sse/perplexity_ask` research flow found in the
//! 2026-06-11 mindemon dump: an `INITIAL_QUERY` is expanded into a `PLAN` of
//! sub-queries (`pro_search_step`s), each sub-query runs a search, and a `FINAL`
//! step synthesises the gathered evidence. The optional clarifying-questions
//! gate mirrors `handle_perplexity_research_clarifying_answers`.
//!
//! Design (same shape as [`crate::council`]): the orchestration is
//! provider/transport-agnostic. Callers inject:
//! - a [`Searcher`] (production wires `jfc_web::search`; tests use a mock), and
//! - a [`Synthesizer`] (production wires an LLM / the model council; tests use a
//!   deterministic concatenator).
//!
//! That keeps the loop — intent detection, clarification gating, planning,
//! stepped search, evidence collection, synthesis — fully unit-testable without
//! network or model calls.

use async_trait::async_trait;

/// Whether a query warrants the deep-research loop.
///
/// When the `intent-gate` feature is enabled this defers to the shared
/// [`crate::intent`] classifier (Research/Investigation intents qualify);
/// otherwise it falls back to a small local keyword heuristic so the research
/// loop is usable without that feature.
pub fn wants_research(query: &str) -> bool {
    #[cfg(feature = "intent-gate")]
    {
        use crate::intent::{self, Intent};
        return matches!(
            intent::classify(query).intent,
            Intent::Research | Intent::Investigation
        );
    }
    #[cfg(not(feature = "intent-gate"))]
    {
        wants_research_heuristic(query)
    }
}

/// Local fallback for research-intent detection (used when `intent-gate` is
/// off). Looks for investigative/research verbs and question framing.
#[cfg(not(feature = "intent-gate"))]
fn wants_research_heuristic(query: &str) -> bool {
    let lower = query.to_ascii_lowercase();
    const RESEARCH_CUES: &[&str] = &[
        "research",
        "investigate",
        "find ",
        "search",
        "look into",
        "compare",
        "explain",
        "how does",
        "how do",
        "why does",
        "what is",
        "where is",
        "trace",
        "analyze",
        "analyse",
        "explore",
    ];
    RESEARCH_CUES.iter().any(|cue| lower.contains(cue))
}

/// A search backend the orchestrator can call per step. Returns formatted
/// result text (the same shape `jfc_web::search` yields) or an error string.
#[async_trait]
pub trait Searcher: Send + Sync {
    async fn search(&self, query: &str, max_results: usize) -> Result<String, String>;
}

/// Synthesises collected evidence into a final answer. Production wires an LLM
/// (or the model council); tests use a deterministic merge.
#[async_trait]
pub trait Synthesizer: Send + Sync {
    async fn synthesize(&self, question: &str, evidence: &[ResearchStep])
    -> Result<String, String>;
}

/// One planned-and-executed research step (a `pro_search_step`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResearchStep {
    /// The sub-query that was searched.
    pub sub_query: String,
    /// `Ok(result_text)` on success; `Err(reason)` when the search failed.
    pub outcome: StepOutcome,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StepOutcome {
    Found(String),
    Failed(String),
}

impl ResearchStep {
    pub fn evidence(&self) -> Option<&str> {
        match &self.outcome {
            StepOutcome::Found(t) => Some(t.as_str()),
            StepOutcome::Failed(_) => None,
        }
    }

    pub fn succeeded(&self) -> bool {
        matches!(self.outcome, StepOutcome::Found(_))
    }
}

/// A clarifying question surfaced to the user before research begins. Mirrors
/// `research_clarifying_questions_content`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClarifyingQuestion {
    pub question: String,
    /// Suggested answer options (may be empty for free-text).
    pub options: Vec<String>,
}

/// Configuration for a research run.
pub struct ResearchRequest {
    pub question: String,
    /// Pre-supplied answers to clarifying questions (e.g. from a prior
    /// AskUserQuestion round). When present, the clarification gate is skipped.
    pub clarifications: Vec<String>,
    /// Max number of sub-query steps to plan/execute.
    pub max_steps: usize,
    /// Results requested per sub-query search.
    pub results_per_step: usize,
}

impl ResearchRequest {
    pub fn new(question: impl Into<String>) -> Self {
        Self {
            question: question.into(),
            clarifications: Vec::new(),
            max_steps: 4,
            results_per_step: 5,
        }
    }

    pub fn with_clarifications(mut self, answers: Vec<String>) -> Self {
        self.clarifications = answers;
        self
    }

    pub fn with_max_steps(mut self, n: usize) -> Self {
        self.max_steps = n.max(1);
        self
    }
}

/// The full research deliverable: the plan, the per-step evidence, and the
/// synthesised final answer.
#[derive(Debug, Clone)]
pub struct ResearchReport {
    pub question: String,
    pub plan: Vec<String>,
    pub steps: Vec<ResearchStep>,
    pub synthesis: String,
}

impl ResearchReport {
    pub fn successful_steps(&self) -> usize {
        self.steps.iter().filter(|s| s.succeeded()).count()
    }

    pub fn to_markdown(&self) -> String {
        let mut out = String::from("## Research\n\n");
        out.push_str(&self.synthesis);
        out.push_str("\n\n---\n");
        out.push_str(&format!(
            "_Plan of {} step(s), {} answered:_\n",
            self.plan.len(),
            self.successful_steps()
        ));
        for step in &self.steps {
            match &step.outcome {
                StepOutcome::Found(_) => out.push_str(&format!("- ✅ {}\n", step.sub_query)),
                StepOutcome::Failed(reason) => {
                    out.push_str(&format!("- ⚠️ {} — {}\n", step.sub_query, reason))
                }
            }
        }
        out
    }
}

/// Decide whether to ask clarifying questions before researching. Returns at
/// most one question for an under-specified query (very short / vague). Returns
/// empty when the caller already supplied clarifications or the query is
/// specific enough.
pub fn clarifying_questions(request: &ResearchRequest) -> Vec<ClarifyingQuestion> {
    if !request.clarifications.is_empty() {
        return Vec::new();
    }
    let words = request.question.split_whitespace().count();
    // A very short query (<= 3 words) is usually under-specified for research.
    if words <= 3 {
        return vec![ClarifyingQuestion {
            question: format!(
                "Your query \"{}\" is broad — what specifically should the research focus on?",
                request.question.trim()
            ),
            options: vec![
                "Background / overview".to_owned(),
                "Latest developments".to_owned(),
                "Technical deep-dive".to_owned(),
            ],
        }];
    }
    Vec::new()
}

/// Build a plan of sub-queries from the question + any clarifications. This is a
/// deterministic decomposition (no LLM): the base question, a "latest"/recency
/// angle, a "how/why" mechanism angle, and one angle per clarification —
/// deduped and capped at `max_steps`. Mirrors a `PLAN` step's sub-queries.
pub fn plan_subqueries(request: &ResearchRequest) -> Vec<String> {
    let base = request.question.trim().to_owned();
    let mut plan: Vec<String> = Vec::new();
    let push_unique = |q: String, plan: &mut Vec<String>| {
        let q = q.trim().to_owned();
        if !q.is_empty() && !plan.iter().any(|e| e.eq_ignore_ascii_case(&q)) {
            plan.push(q);
        }
    };

    push_unique(base.clone(), &mut plan);
    // Clarifications sharpen the focus first (highest signal).
    for c in &request.clarifications {
        push_unique(format!("{base} {c}"), &mut plan);
    }
    // Generic research angles.
    push_unique(format!("{base} latest developments"), &mut plan);
    push_unique(format!("{base} how it works"), &mut plan);
    push_unique(format!("{base} limitations criticism"), &mut plan);

    plan.truncate(request.max_steps);
    plan
}

/// Run the full deep-research loop: plan sub-queries, search each in sequence,
/// collect evidence, and synthesise. A clarification gate is the caller's
/// responsibility (call [`clarifying_questions`] first and re-enter with
/// answers); this function always proceeds to research.
///
/// Returns `Err` only when the question is empty or **every** search step
/// failed (no evidence to synthesise).
#[tracing::instrument(target = "jfc::research", skip(request, searcher, synthesizer))]
pub async fn run_research(
    request: ResearchRequest,
    searcher: &dyn Searcher,
    synthesizer: &dyn Synthesizer,
) -> Result<ResearchReport, String> {
    let question = request.question.trim().to_owned();
    if question.is_empty() {
        return Err("research question is empty".to_owned());
    }

    let plan = plan_subqueries(&request);
    let steps = execute_plan(&plan, request.results_per_step, searcher).await;

    let answered = steps.iter().filter(|s| s.succeeded()).count();
    if answered == 0 {
        return Err(format!("all {} research steps failed", steps.len()));
    }

    let synthesis = synthesizer
        .synthesize(&question, &steps)
        .await
        .unwrap_or_else(|e| {
            tracing::warn!(target: "jfc::research", error = %e, "synthesis failed; local merge");
            local_synthesis(&question, &steps)
        });

    Ok(ResearchReport {
        question,
        plan,
        steps,
        synthesis,
    })
}

/// Execute each planned sub-query in sequence, capturing success/failure per
/// step (one failure never aborts the run).
async fn execute_plan(
    plan: &[String],
    results_per_step: usize,
    searcher: &dyn Searcher,
) -> Vec<ResearchStep> {
    let mut steps = Vec::with_capacity(plan.len());
    for sub_query in plan {
        let outcome = match searcher.search(sub_query, results_per_step).await {
            Ok(text) => StepOutcome::Found(text),
            Err(e) => StepOutcome::Failed(e),
        };
        steps.push(ResearchStep {
            sub_query: sub_query.clone(),
            outcome,
        });
    }
    steps
}

/// Deterministic fallback synthesis: concatenate successful step evidence under
/// labelled headings. Used when no LLM synthesizer is available or it errors.
fn local_synthesis(question: &str, steps: &[ResearchStep]) -> String {
    let mut out = format!("Research summary for: {question}\n");
    for step in steps.iter().filter(|s| s.succeeded()) {
        out.push_str(&format!(
            "\n### {}\n{}\n",
            step.sub_query,
            step.evidence().unwrap_or_default()
        ));
    }
    out
}

/// Production adapter: routes [`Searcher`] calls to `jfc_web::search`.
pub struct WebSearcher;

#[async_trait]
impl Searcher for WebSearcher {
    async fn search(&self, query: &str, max_results: usize) -> Result<String, String> {
        jfc_web::search(query, max_results).await
    }
}

/// A no-LLM synthesizer that just does the deterministic local merge. Useful as
/// a default when no model is wired.
pub struct LocalSynthesizer;

#[async_trait]
impl Synthesizer for LocalSynthesizer {
    async fn synthesize(
        &self,
        question: &str,
        evidence: &[ResearchStep],
    ) -> Result<String, String> {
        Ok(local_synthesis(question, evidence))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// Mock searcher that returns canned text per query, or fails for queries
    /// containing a configured "fail" marker.
    struct MockSearcher {
        /// Queries logged in call order.
        calls: Mutex<Vec<String>>,
        fail_substr: Option<String>,
    }

    impl MockSearcher {
        fn new() -> Self {
            Self {
                calls: Mutex::new(Vec::new()),
                fail_substr: None,
            }
        }

        fn failing(substr: &str) -> Self {
            Self {
                calls: Mutex::new(Vec::new()),
                fail_substr: Some(substr.to_owned()),
            }
        }

        fn always_fail() -> Self {
            Self {
                calls: Mutex::new(Vec::new()),
                fail_substr: Some(String::new()), // empty substr matches everything
            }
        }
    }

    #[async_trait]
    impl Searcher for MockSearcher {
        async fn search(&self, query: &str, _max: usize) -> Result<String, String> {
            self.calls.lock().unwrap().push(query.to_owned());
            if let Some(marker) = &self.fail_substr {
                if marker.is_empty() || query.contains(marker) {
                    return Err("mock failure".to_owned());
                }
            }
            Ok(format!("results for [{query}]"))
        }
    }

    /// Synthesizer that records that it ran and echoes step count.
    struct CountingSynth;

    #[async_trait]
    impl Synthesizer for CountingSynth {
        async fn synthesize(
            &self,
            question: &str,
            evidence: &[ResearchStep],
        ) -> Result<String, String> {
            let ok = evidence.iter().filter(|s| s.succeeded()).count();
            Ok(format!("SYNTHESIS({question}): {ok} sources"))
        }
    }

    // ── Intent gate ──────────────────────────────────────────────────────────

    #[test]
    fn wants_research_detects_research_intent_normal() {
        assert!(wants_research(
            "research the history of rust async runtimes"
        ));
        assert!(wants_research("find where the auth token is validated"));
    }

    #[test]
    fn wants_research_false_for_chitchat_robust() {
        assert!(!wants_research("hi there"));
        assert!(!wants_research("thanks!"));
    }

    // ── Clarification gate ───────────────────────────────────────────────────

    #[test]
    fn clarifying_questions_for_short_query_normal() {
        let req = ResearchRequest::new("quantum");
        let qs = clarifying_questions(&req);
        assert_eq!(qs.len(), 1);
        assert!(!qs[0].options.is_empty());
    }

    #[test]
    fn clarifying_questions_skipped_when_answered_robust() {
        let req = ResearchRequest::new("quantum").with_clarifications(vec!["overview".into()]);
        assert!(clarifying_questions(&req).is_empty());
    }

    #[test]
    fn clarifying_questions_skipped_for_specific_query_normal() {
        let req = ResearchRequest::new("how does rust tokio schedule tasks across worker threads");
        assert!(clarifying_questions(&req).is_empty());
    }

    // ── Planning ─────────────────────────────────────────────────────────────

    #[test]
    fn plan_subqueries_includes_base_and_angles_normal() {
        let req = ResearchRequest::new("rust async").with_max_steps(4);
        let plan = plan_subqueries(&req);
        assert_eq!(plan.len(), 4);
        assert_eq!(plan[0], "rust async");
        assert!(plan.iter().any(|q| q.contains("latest developments")));
    }

    #[test]
    fn plan_subqueries_incorporates_clarifications_first_normal() {
        let req = ResearchRequest::new("rust async")
            .with_clarifications(vec!["performance".into()])
            .with_max_steps(3);
        let plan = plan_subqueries(&req);
        assert_eq!(plan[0], "rust async");
        assert_eq!(plan[1], "rust async performance");
    }

    #[test]
    fn plan_subqueries_dedups_and_caps_robust() {
        let req = ResearchRequest::new("x").with_max_steps(2);
        let plan = plan_subqueries(&req);
        assert_eq!(plan.len(), 2);
        // No duplicates.
        let mut sorted = plan.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), plan.len());
    }

    // ── Full loop ────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn run_research_executes_plan_and_synthesizes_normal() {
        let searcher = MockSearcher::new();
        let synth = CountingSynth;
        let req = ResearchRequest::new("rust async runtimes").with_max_steps(3);
        let report = run_research(req, &searcher, &synth).await.expect("ok");

        assert_eq!(report.plan.len(), 3);
        assert_eq!(report.steps.len(), 3);
        assert_eq!(report.successful_steps(), 3);
        assert!(report.synthesis.contains("3 sources"));
        // Every planned sub-query was searched, in order.
        assert_eq!(searcher.calls.lock().unwrap().len(), 3);
    }

    #[tokio::test]
    async fn run_research_tolerates_partial_failure_robust() {
        // Fail only the "limitations criticism" angle; base + others succeed.
        let searcher = MockSearcher::failing("limitations");
        let synth = CountingSynth;
        let req = ResearchRequest::new("rust async runtimes").with_max_steps(4);
        let report = run_research(req, &searcher, &synth).await.expect("ok");
        assert!(report.successful_steps() >= 1);
        assert!(report.steps.iter().any(|s| !s.succeeded()));
    }

    #[tokio::test]
    async fn run_research_all_fail_is_error_robust() {
        let searcher = MockSearcher::always_fail();
        let synth = CountingSynth;
        let req = ResearchRequest::new("rust async runtimes");
        let err = run_research(req, &searcher, &synth)
            .await
            .expect_err("all fail");
        assert!(err.contains("research steps failed"));
    }

    #[tokio::test]
    async fn run_research_empty_question_is_error_robust() {
        let searcher = MockSearcher::new();
        let synth = CountingSynth;
        let err = run_research(ResearchRequest::new("   "), &searcher, &synth)
            .await
            .expect_err("empty");
        assert!(err.contains("empty"));
    }

    #[tokio::test]
    async fn run_research_local_synthesizer_merges_evidence_normal() {
        let searcher = MockSearcher::new();
        let synth = LocalSynthesizer;
        let req = ResearchRequest::new("rust async").with_max_steps(2);
        let report = run_research(req, &searcher, &synth).await.expect("ok");
        assert!(report.synthesis.contains("Research summary"));
        assert!(report.synthesis.contains("results for"));
    }

    #[tokio::test]
    async fn run_research_markdown_lists_steps_normal() {
        let searcher = MockSearcher::failing("limitations");
        let synth = CountingSynth;
        let req = ResearchRequest::new("rust async runtimes").with_max_steps(4);
        let md = run_research(req, &searcher, &synth)
            .await
            .unwrap()
            .to_markdown();
        assert!(md.contains("## Research"));
        assert!(md.contains("✅"));
        assert!(md.contains("⚠️"));
    }
}

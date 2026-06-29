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
    /// Rewrite each sub-query for retrieval before searching (mirrors
    /// Perplexity's `/rest/autosuggest/reformulate-query`). On by default.
    pub reformulate: bool,
}

impl ResearchRequest {
    pub fn new(question: impl Into<String>) -> Self {
        Self {
            question: question.into(),
            clarifications: Vec::new(),
            max_steps: 4,
            results_per_step: 5,
            reformulate: true,
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

    pub fn with_reformulation(mut self, on: bool) -> Self {
        self.reformulate = on;
        self
    }
}

/// The full research deliverable: the plan, the per-step evidence, the
/// synthesised final answer, generated follow-up questions, and the numbered
/// citation list tying claims back to the steps that produced them.
#[derive(Debug, Clone)]
pub struct ResearchReport {
    pub question: String,
    pub plan: Vec<String>,
    pub steps: Vec<ResearchStep>,
    pub synthesis: String,
    /// Suggested next questions (mirrors Perplexity's `pending_followups_block`).
    pub followups: Vec<String>,
}

/// One numbered citation: `[n]` → the successful research step (sub-query) whose
/// evidence backs it. Mirrors Perplexity's `citation_block` / inline-claim
/// source anchoring, scoped to a coding assistant (step-level, not span-level).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Citation {
    /// 1-based citation number as it appears in the rendered report.
    pub number: usize,
    /// The sub-query (source label) this citation points at.
    pub source: String,
}

/// A research artifact exported to disk: the report rendered as markdown plus a
/// machine-readable JSON sidecar. Mirrors Perplexity's
/// `/rest/deeper-research/export-asset`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResearchArtifact {
    pub markdown_path: std::path::PathBuf,
    pub json_path: std::path::PathBuf,
}

const RESEARCH_ARTIFACT_KIND: &str = "research_report";

impl ResearchReport {
    pub fn successful_steps(&self) -> usize {
        self.steps.iter().filter(|s| s.succeeded()).count()
    }

    /// The numbered citation list: one `[n]` per successful step, in order.
    /// Mirrors Perplexity's `citation_block`. The same numbering is appended to
    /// each step bullet in [`Self::to_markdown`] so a reader can trace a claim
    /// (a step's evidence) to its source.
    pub fn citations(&self) -> Vec<Citation> {
        self.steps
            .iter()
            .filter(|s| s.succeeded())
            .enumerate()
            .map(|(i, s)| Citation {
                number: i + 1,
                source: s.sub_query.clone(),
            })
            .collect()
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
        // Number successful steps so they double as citation anchors.
        let mut n = 0;
        for step in &self.steps {
            match &step.outcome {
                StepOutcome::Found(_) => {
                    n += 1;
                    out.push_str(&format!("- [{n}] ✅ {}\n", step.sub_query));
                }
                StepOutcome::Failed(reason) => {
                    out.push_str(&format!("- ⚠️ {} — {}\n", step.sub_query, reason))
                }
            }
        }
        if !self.followups.is_empty() {
            out.push_str("\n**Follow-up questions:**\n");
            for f in &self.followups {
                out.push_str(&format!("- {f}\n"));
            }
        }
        out
    }

    /// Serialise the report to a JSON value (the export sidecar payload).
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "question": self.question,
            "plan": self.plan,
            "synthesis": self.synthesis,
            "followups": self.followups,
            "citations": self.citations().iter().map(|c| {
                serde_json::json!({ "number": c.number, "source": c.source })
            }).collect::<Vec<_>>(),
            "steps": self.steps.iter().map(|s| {
                serde_json::json!({
                    "sub_query": s.sub_query,
                    "ok": s.succeeded(),
                    "evidence": s.evidence(),
                })
            }).collect::<Vec<_>>(),
        })
    }

    /// Export the report as a DB artifact. The returned paths are stable
    /// handles for UI compatibility, not filesystem paths.
    pub fn export(&self, dir: &std::path::Path) -> std::io::Result<ResearchArtifact> {
        let slug = export_slug(&self.question);
        let payload = serde_json::json!({
            "schema_version": 1,
            "slug": slug,
            "markdown": self.to_markdown(),
            "report": self.to_json(),
        });
        let session_id = format!("project:{}", jfc_knowledge::project_key(dir));
        let value_json = serde_json::to_string(&payload).map_err(std::io::Error::other)?;
        jfc_knowledge::block_on_knowledge(async {
            let store = jfc_knowledge::KnowledgeStore::open_default()
                .await
                .map_err(std::io::Error::other)?;
            store
                .upsert_session_artifact(&session_id, RESEARCH_ARTIFACT_KIND, &slug, &value_json)
                .await
                .map_err(std::io::Error::other)
        })?;
        Ok(ResearchArtifact {
            markdown_path: std::path::PathBuf::from(format!("db:research:{slug}:markdown")),
            json_path: std::path::PathBuf::from(format!("db:research:{slug}:json")),
        })
    }

    /// Convenience wrapper: export under `dir` (defaults to a `jfc-research`
    /// subdir of the system temp dir) and return the markdown artifact path.
    pub fn export_artifact(
        &self,
        dir: Option<std::path::PathBuf>,
    ) -> std::io::Result<std::path::PathBuf> {
        let dir = dir.unwrap_or_else(|| std::env::temp_dir().join("jfc-research"));
        Ok(self.export(&dir)?.markdown_path)
    }
}

/// Build a filesystem-safe slug from the question (lowercase, hyphenated,
/// capped). Empty questions fall back to `research`.
fn export_slug(question: &str) -> String {
    let mut slug = String::new();
    let mut prev_dash = false;
    for ch in question.chars().take(80) {
        if ch.is_ascii_alphanumeric() {
            slug.push(ch.to_ascii_lowercase());
            prev_dash = false;
        } else if !prev_dash {
            slug.push('-');
            prev_dash = true;
        }
    }
    let slug = slug.trim_matches('-').to_owned();
    if slug.is_empty() {
        "research".to_owned()
    } else {
        slug
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

/// Backend prefixes understood by [`jfc_web::search`]. When a sub-query starts
/// with one of these (`arxiv:`, `uni:`, …), reformulation must preserve the
/// prefix verbatim and only rewrite the remainder, or routing breaks. Kept in
/// sync with the `search()` dispatcher in `jfc-web`.
pub const BACKEND_PREFIXES: &[&str] = &[
    "arxiv",
    "scholar",
    "openalex",
    "crossref",
    "pubmed",
    "doaj",
    "core",
    "unpaywall",
    "papers",
    "brave",
    "tavily",
    "exa",
    "ddg",
    "wiki",
    "primo",
    "uni",
    "edu",
    "cn",
    "gov",
];

/// Split a leading `prefix:`/`prefix ` backend selector off a query, returning
/// `(Some(prefix), remainder)` when one of [`BACKEND_PREFIXES`] matches, else
/// `(None, query)`.
fn split_backend_prefix(query: &str) -> (Option<&'static str>, &str) {
    let trimmed = query.trim_start();
    for &p in BACKEND_PREFIXES {
        // Match `prefix:` or `prefix ` (case-insensitive on the selector only).
        let lower = trimmed.to_ascii_lowercase();
        if let Some(rest) = lower
            .strip_prefix(&format!("{p}:"))
            .or_else(|| lower.strip_prefix(&format!("{p} ")))
        {
            let cut = trimmed.len() - rest.len();
            return (Some(p), trimmed[cut..].trim_start());
        }
    }
    (None, query)
}

/// Rewrite a sub-query into a retrieval-friendly search query before it hits the
/// search backend. Mirrors Perplexity's `/rest/autosuggest/reformulate-query`
/// (which uses a model); this is a deterministic, dependency-light version:
/// strip conversational scaffolding ("can you", "please tell me about", a
/// trailing question mark), collapse whitespace, and keep the salient terms.
/// A leading backend selector (`arxiv:`, `uni:`, …) is preserved verbatim so
/// routing survives reformulation. The result is never empty — it falls back to
/// the trimmed input.
pub fn reformulate_query(query: &str) -> String {
    // Preserve a backend selector and reformulate only the remainder.
    let (prefix, body) = split_backend_prefix(query);
    if let Some(p) = prefix {
        // `uni:` carries a second `<University>: <topic>` colon structure; keep
        // its body intact (only trim) so the institution lookup still parses.
        let body_reformulated = if p == "uni" {
            body.trim().to_owned()
        } else {
            reformulate_plain(body)
        };
        if body_reformulated.is_empty() {
            return query.trim().to_owned();
        }
        return format!("{p}: {body_reformulated}");
    }
    reformulate_plain(query)
}

/// Reformulate a query with no backend prefix (the core keyword cleanup).
fn reformulate_plain(query: &str) -> String {
    let lower_trimmed = query.trim();
    if lower_trimmed.is_empty() {
        return String::new();
    }

    // Leading conversational lead-ins that add no retrieval signal.
    const LEAD_INS: &[&str] = &[
        "can you tell me about",
        "can you tell me",
        "could you tell me about",
        "please tell me about",
        "tell me about",
        "i want to know about",
        "i want to know",
        "i'd like to know about",
        "please explain",
        "can you explain",
        "explain to me",
        "what can you tell me about",
        "give me information about",
        "i need information on",
        "help me understand",
        "please find",
        "can you find",
        "find me",
    ];

    let mut working = lower_trimmed.to_owned();
    let lower = working.to_ascii_lowercase();
    for lead in LEAD_INS {
        if let Some(rest) = lower.strip_prefix(lead) {
            // Preserve the original casing of the remainder.
            let cut = working.len() - rest.len();
            working = working[cut..].trim_start().to_owned();
            break;
        }
    }

    // Drop a single trailing question mark and collapse internal whitespace.
    let working = working.trim_end_matches(['?', '.', '!', ' ']);
    let reformulated = working.split_whitespace().collect::<Vec<_>>().join(" ");
    if reformulated.is_empty() {
        lower_trimmed.to_owned()
    } else {
        reformulated
    }
}

/// Generate follow-up questions from a completed report. Mirrors Perplexity's
/// `pending_followups_block`. Deterministic: derive next-angle questions from
/// the original question, capped at three, skipping angles already covered by
/// the plan.
pub fn generate_followups(question: &str, plan: &[String]) -> Vec<String> {
    let base = question.trim().trim_end_matches(['?', '.', '!']);
    if base.is_empty() {
        return Vec::new();
    }
    let covered = |needle: &str| plan.iter().any(|p| p.to_ascii_lowercase().contains(needle));
    let mut out = Vec::new();
    let push = |q: String, out: &mut Vec<String>| {
        if out.len() < 3 && !out.contains(&q) {
            out.push(q);
        }
    };
    if !covered("limitation") && !covered("criticism") {
        push(
            format!("What are the limitations or criticisms of {base}?"),
            &mut out,
        );
    }
    if !covered("alternativ") && !covered("compare") {
        push(
            format!("What are the main alternatives to {base}?"),
            &mut out,
        );
    }
    if !covered("latest") && !covered("recent") {
        push(
            format!("What are the latest developments in {base}?"),
            &mut out,
        );
    }
    push(format!("How does {base} work in practice?"), &mut out);
    out.truncate(3);
    out
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
    let steps = execute_plan(
        &plan,
        request.results_per_step,
        request.reformulate,
        searcher,
    )
    .await;

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

    let followups = generate_followups(&question, &plan);

    Ok(ResearchReport {
        question,
        plan,
        steps,
        synthesis,
        followups,
    })
}

/// Execute each planned sub-query in sequence, capturing success/failure per
/// step (one failure never aborts the run). When `reformulate` is set, each
/// sub-query is rewritten for retrieval via [`reformulate_query`] before it
/// hits the search backend; the step records the original sub-query so the plan
/// stays human-readable.
async fn execute_plan(
    plan: &[String],
    results_per_step: usize,
    reformulate: bool,
    searcher: &dyn Searcher,
) -> Vec<ResearchStep> {
    let mut steps = Vec::with_capacity(plan.len());
    for sub_query in plan {
        let search_query = if reformulate {
            reformulate_query(sub_query)
        } else {
            sub_query.clone()
        };
        let outcome = match searcher.search(&search_query, results_per_step).await {
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

// ════════════════════════════════════════════════════════════════════════════
// Agentic, LLM-driven research loop
//
// The deterministic path above (plan_subqueries → execute_plan → local merge)
// stays as a fallback and for tests. The agentic path below mirrors how
// claude.ai web research and Perplexity pro-search actually work: a model
// REFORMULATES the next sub-query from the evidence gathered so far, and a real
// model SYNTHESISES the collected evidence into cited prose. Both reuse the
// council's provider-completion helpers (`Provider::complete()` with a
// stream-to-completion fallback) so research never shares the main agent's
// stream channel or token accounting.
// ════════════════════════════════════════════════════════════════════════════

use std::sync::Arc;

use jfc_provider::{
    CompletionResponse, ModelId, Provider, ProviderContent, ProviderMessage, ProviderRole,
    StreamOptions,
};

/// Default number of agentic search steps (model-decided queries) for one
/// research run. The loop stops earlier if the planner signals `DONE`.
pub const DEFAULT_AGENTIC_STEPS: usize = 6;
/// Max output tokens for one planner (next-query) completion. Small — it only
/// emits a single search query.
const PLANNER_MAX_TOKENS: u32 = 256;
/// Max output tokens for the final synthesis completion.
const SYNTH_MAX_TOKENS: u32 = 3072;

/// Search-backend selectors the planner may prefix onto a sub-query to route it
/// to a specialised source instead of general web search. Mirrors the prefixes
/// understood by [`jfc_web::search`]. Kept in sync with `crate::research`'s
/// knowledge of `jfc-web`; the planner is told about these so academic / domain
/// questions hit the right index (arXiv, OpenAlex, PubMed, a named university,
/// etc.) rather than always defaulting to Google.
pub const SEARCH_BACKENDS: &str = "\
Available search backends — prefix the query with one to route it (omit the \
prefix for a general web search):\n\
- `arxiv:` preprints (physics/CS/math). e.g. `arxiv: diffusion model guidance`\n\
- `openalex:` 250M+ scholarly works across all fields. e.g. `openalex: graph neural networks`\n\
- `crossref:` DOI metadata for published papers. e.g. `crossref: attention is all you need`\n\
- `pubmed:` biomedical / life-sciences literature. e.g. `pubmed: CRISPR off-target`\n\
- `scholar:` Semantic Scholar (citations, TLDRs). e.g. `scholar: retrieval augmented generation`\n\
- `dblp:` computer-science bibliography (authors, venue, DOI, links). e.g. `dblp: attention is all you need`\n\
- `gscholar:` Google Scholar query autocomplete (expand a term into canonical phrasings). e.g. `gscholar: large language model`\n\
- `papers:` arXiv + Semantic Scholar + OpenAlex merged + deduped (best for a broad academic sweep). e.g. `papers: mixture of experts`\n\
- `doaj:` open-access journals. `core:` 290M+ OA full texts. `unpaywall:` resolve a DOI to a free PDF.\n\
- `uni:` a named university's research output (any country), e.g. `uni: Tsinghua University: quantum computing`\n\
- `edu:` academic-domain web (.edu/.ac.uk/…). `gov:` government sources. `cn:` Chinese academic domains. `primo:` university library discovery.\n\
- `wiki:` Wikipedia overview. `ddg:` quick factual definitions. `brave:`/`tavily:`/`exa:` alternative web indexes.";

const PLANNER_SYSTEM_PROMPT: &str = "\
You are the planning step of a deep-research loop. Given the user's research \
question and the evidence gathered by prior searches, decide the single most \
useful NEXT search query to close the biggest remaining gap. Reply with ONLY \
that query — no preamble, no quotes, no explanation. The query must be a \
concise, retrieval-friendly set of keywords (not a sentence).\n\n\
Route to the right source by prefixing the query with a backend selector when \
it helps (e.g. an academic question → `papers:` or `arxiv:` or `pubmed:`; a \
specific institution → `uni: <University>: <topic>`; a quick definition → \
`wiki:`). Omit the prefix for ordinary web questions. Vary backends across \
steps so you triangulate rather than re-querying one index.\n\n\
If the evidence already answers the question well enough, reply with exactly DONE.";

const SYNTH_SYSTEM_PROMPT: &str = "\
You are the synthesis step of a deep-research loop. You are given the original \
question and numbered evidence blocks gathered from searches (each block is a \
source: [1], [2], …). Write a direct, well-structured answer to the question \
grounded in that evidence. Cite sources inline as [n] matching the evidence \
block numbers whenever you state a fact drawn from them. Lead with the answer, \
be concrete, and flag where the evidence is thin or conflicting. Do not invent \
sources or citation numbers that aren't in the evidence.";

/// Run a single tool-less completion via the shared one-shot executor
/// ([`crate::prompt_executor::complete_once`]). Research surfaces failures as
/// `String`, so the executor's `anyhow::Error` is flattened here.
async fn research_complete(
    provider: &dyn Provider,
    messages: Vec<ProviderMessage>,
    opts: &StreamOptions,
) -> Result<CompletionResponse, String> {
    crate::prompt_executor::complete_once(provider, messages, opts)
        .await
        .map_err(|e| e.to_string())
}

/// Build the labelled, numbered evidence block handed to the planner/synthesizer
/// so citation numbers `[n]` line up with successful steps in order.
fn numbered_evidence(steps: &[ResearchStep]) -> String {
    let mut block = String::new();
    let mut n = 0;
    for step in steps.iter().filter(|s| s.succeeded()) {
        n += 1;
        block.push_str(&format!(
            "\n[{n}] (query: {})\n{}\n",
            step.sub_query,
            step.evidence().unwrap_or_default()
        ));
    }
    block
}

/// An LLM synthesizer: produces real cited prose from the gathered evidence.
/// Falls back to the deterministic [`local_synthesis`] only if the model errors.
pub struct LlmSynthesizer {
    pub provider: Arc<dyn Provider>,
    pub model: ModelId,
}

impl LlmSynthesizer {
    pub fn new(provider: Arc<dyn Provider>, model: impl Into<ModelId>) -> Self {
        Self {
            provider,
            model: model.into(),
        }
    }
}

#[async_trait]
impl Synthesizer for LlmSynthesizer {
    async fn synthesize(
        &self,
        question: &str,
        evidence: &[ResearchStep],
    ) -> Result<String, String> {
        if evidence.iter().filter(|s| s.succeeded()).count() == 0 {
            return Err("no successful evidence to synthesise".to_owned());
        }
        let block = format!(
            "Original question:\n{question}\n\nEvidence:\n{}\n\nWrite the cited answer now.",
            numbered_evidence(evidence)
        );
        let messages = vec![ProviderMessage {
            role: ProviderRole::User,
            content: vec![ProviderContent::Text(block)],
        }];
        let opts = StreamOptions::new(self.model.clone())
            .system(SYNTH_SYSTEM_PROMPT)
            .max_tokens(SYNTH_MAX_TOKENS);
        let resp = research_complete(self.provider.as_ref(), messages, &opts).await?;
        let answer = resp.content.trim().to_owned();
        if answer.is_empty() {
            Err("synthesizer returned empty content".to_owned())
        } else {
            Ok(answer)
        }
    }
}

/// Proposes the next sub-query from the evidence gathered so far. This is the
/// agentic core: the model reads prior results and decides where to look next
/// (or signals completion).
#[async_trait]
pub trait Planner: Send + Sync {
    /// Returns `Some(query)` for the next search, or `None` to stop (the model
    /// judged the evidence sufficient, or there is no useful next step).
    async fn next_query(&self, question: &str, steps: &[ResearchStep]) -> Option<String>;
}

/// An LLM planner: asks the model for the single most useful next query given
/// the evidence so far. A reply of `DONE` (or empty) stops the loop.
pub struct LlmPlanner {
    pub provider: Arc<dyn Provider>,
    pub model: ModelId,
}

impl LlmPlanner {
    pub fn new(provider: Arc<dyn Provider>, model: impl Into<ModelId>) -> Self {
        Self {
            provider,
            model: model.into(),
        }
    }
}

#[async_trait]
impl Planner for LlmPlanner {
    async fn next_query(&self, question: &str, steps: &[ResearchStep]) -> Option<String> {
        let evidence = numbered_evidence(steps);
        let block = if evidence.trim().is_empty() {
            format!(
                "{SEARCH_BACKENDS}\n\nResearch question:\n{question}\n\nNo evidence gathered yet. Give the first search query (prefix a backend selector if it helps)."
            )
        } else {
            format!(
                "{SEARCH_BACKENDS}\n\nResearch question:\n{question}\n\nEvidence gathered so far:\n{evidence}\n\nGive the next search query (prefix a backend selector if it helps), or DONE."
            )
        };
        let messages = vec![ProviderMessage {
            role: ProviderRole::User,
            content: vec![ProviderContent::Text(block)],
        }];
        let opts = StreamOptions::new(self.model.clone())
            .system(PLANNER_SYSTEM_PROMPT)
            .max_tokens(PLANNER_MAX_TOKENS);
        let resp = research_complete(self.provider.as_ref(), messages, &opts)
            .await
            .ok()?;
        let q = resp.content.trim().trim_matches('"').trim().to_owned();
        if q.is_empty() || q.eq_ignore_ascii_case("done") {
            None
        } else {
            Some(q)
        }
    }
}

/// Run the agentic research loop: the planner proposes the first query from the
/// question, each search result feeds back into the planner to choose the next
/// query, and a real synthesizer writes the cited answer. Stops when the planner
/// signals completion or `max_steps` is reached.
///
/// Returns `Err` only when the question is empty or **every** search failed.
#[tracing::instrument(
    target = "jfc::research",
    skip(request, planner, searcher, synthesizer)
)]
pub async fn run_research_agentic(
    request: ResearchRequest,
    planner: &dyn Planner,
    searcher: &dyn Searcher,
    synthesizer: &dyn Synthesizer,
) -> Result<ResearchReport, String> {
    let question = request.question.trim().to_owned();
    if question.is_empty() {
        return Err("research question is empty".to_owned());
    }

    let mut steps: Vec<ResearchStep> = Vec::new();
    let mut plan: Vec<String> = Vec::new();
    for _ in 0..request.max_steps {
        let Some(sub_query) = planner.next_query(&question, &steps).await else {
            break;
        };
        // Stop if the planner repeats a query (no forward progress).
        if plan.iter().any(|p| p.eq_ignore_ascii_case(&sub_query)) {
            break;
        }
        plan.push(sub_query.clone());
        let search_query = if request.reformulate {
            reformulate_query(&sub_query)
        } else {
            sub_query.clone()
        };
        let outcome = match searcher
            .search(&search_query, request.results_per_step)
            .await
        {
            Ok(text) => StepOutcome::Found(text),
            Err(e) => StepOutcome::Failed(e),
        };
        steps.push(ResearchStep { sub_query, outcome });
    }

    // If the planner never produced a usable query (e.g. model unavailable),
    // fall back to the deterministic plan so research still returns something.
    if steps.is_empty() {
        let fallback_plan = plan_subqueries(&request);
        steps = execute_plan(
            &fallback_plan,
            request.results_per_step,
            request.reformulate,
            searcher,
        )
        .await;
        plan = fallback_plan;
    }

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

    let followups = generate_followups(&question, &plan);

    Ok(ResearchReport {
        question,
        plan,
        steps,
        synthesis,
        followups,
    })
}

/// Production adapter: searches the local codebase via ripgrep rooted at `root`.
/// Returns formatted match text (path:line + matched line) or an error string.
/// Lets research investigate the repo, not just the web.
pub struct LocalSearcher {
    pub root: std::path::PathBuf,
}

impl LocalSearcher {
    pub fn new(root: impl Into<std::path::PathBuf>) -> Self {
        Self { root: root.into() }
    }
}

#[async_trait]
impl Searcher for LocalSearcher {
    async fn search(&self, query: &str, max_results: usize) -> Result<String, String> {
        local_codebase_search(&self.root, query, max_results).await
    }
}

/// Run a ripgrep search over `root` for the salient terms in `query`. Uses
/// `rg --json` when available; the query is reduced to keywords and OR-joined so
/// a natural-language sub-query still matches. Returns up to `max_results`
/// formatted hits.
async fn local_codebase_search(
    root: &std::path::Path,
    query: &str,
    max_results: usize,
) -> Result<String, String> {
    // Reduce the sub-query to salient keywords (drop short/stop words) and build
    // a case-insensitive alternation pattern for ripgrep.
    const STOP: &[&str] = &[
        "the", "a", "an", "of", "to", "in", "on", "for", "and", "or", "is", "are", "how", "what",
        "does", "do", "with", "this", "that", "it", "be", "by", "as", "at",
    ];
    let terms: Vec<String> = query
        .split(|c: char| !c.is_alphanumeric() && c != '_')
        .filter(|w| w.len() >= 3 && !STOP.contains(&w.to_ascii_lowercase().as_str()))
        .take(8)
        .map(|w| regex_escape(w))
        .collect();
    if terms.is_empty() {
        return Err(format!("no searchable terms in query: {query}"));
    }
    let pattern = terms.join("|");

    let output = tokio::process::Command::new("rg")
        .arg("--no-heading")
        .arg("--line-number")
        .arg("--ignore-case")
        .arg("--max-count")
        .arg("3")
        .arg("--max-columns")
        .arg("200")
        .arg("-e")
        .arg(&pattern)
        .arg(root)
        .output()
        .await
        .map_err(|e| format!("ripgrep unavailable: {e}"))?;

    let text = String::from_utf8_lossy(&output.stdout);
    let root_str = root.to_string_lossy();
    let mut lines: Vec<String> = Vec::new();
    for line in text.lines().take(max_results.saturating_mul(3)) {
        // Trim the root prefix for readable, relative paths.
        let shown = line
            .strip_prefix(root_str.as_ref())
            .map(|s| s.trim_start_matches('/'))
            .unwrap_or(line);
        lines.push(shown.to_owned());
        if lines.len() >= max_results {
            break;
        }
    }
    if lines.is_empty() {
        Ok(format!(
            "Local codebase search for \"{pattern}\" — 0 matches in {root_str}"
        ))
    } else {
        Ok(format!(
            "Local codebase matches for \"{pattern}\" ({} shown):\n{}",
            lines.len(),
            lines.join("\n")
        ))
    }
}

/// Minimal regex metacharacter escaping for ripgrep alternation terms.
fn regex_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        if "\\.+*?()|[]{}^$".contains(ch) {
            out.push('\\');
        }
        out.push(ch);
    }
    out
}

/// A searcher that queries the web and the local codebase, merging both result
/// sets so a single research step draws on external and repo evidence.
pub struct CombinedSearcher {
    pub web: WebSearcher,
    pub local: LocalSearcher,
}

impl CombinedSearcher {
    pub fn new(root: impl Into<std::path::PathBuf>) -> Self {
        Self {
            web: WebSearcher,
            local: LocalSearcher::new(root),
        }
    }
}

#[async_trait]
impl Searcher for CombinedSearcher {
    async fn search(&self, query: &str, max_results: usize) -> Result<String, String> {
        // A backend-prefixed query (`arxiv:`, `openalex:`, `uni:`, …) is an
        // explicit external/academic lookup — route it only to the web backend
        // and skip the local codebase (the prefix would otherwise be searched
        // as a literal term and never match). Local search uses the prefix
        // stripped off so repo grep sees real terms.
        let (prefix, body) = split_backend_prefix(query);
        let local_fut = async {
            if prefix.is_some() {
                Err("skipped local search for backend-routed query".to_owned())
            } else {
                self.local.search(query, max_results).await
            }
        };
        let _ = body;
        let (web, local) = tokio::join!(self.web.search(query, max_results), local_fut);
        let mut out = String::new();
        if let Ok(w) = &web {
            out.push_str("## Web\n");
            out.push_str(w);
            out.push('\n');
        }
        if let Ok(l) = &local
            && !l.contains("0 matches")
        {
            out.push_str("\n## Local codebase\n");
            out.push_str(l);
            out.push('\n');
        }
        if out.trim().is_empty() {
            // Surface whichever error we have so the step records a reason.
            match (web, local) {
                (Err(e), _) => Err(e),
                (_, Err(e)) => Err(e),
                _ => Err("no results from web or local search".to_owned()),
            }
        } else {
            Ok(out)
        }
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

    /// Planner that emits a scripted list of queries (popped in order), then
    /// stops. Records how many times it was consulted and the evidence size it
    /// saw on each call, so tests can assert the feedback loop ran.
    struct ScriptedPlanner {
        queries: Mutex<Vec<String>>,
        evidence_seen: Mutex<Vec<usize>>,
    }

    impl ScriptedPlanner {
        fn new(queries: Vec<&str>) -> Self {
            Self {
                queries: Mutex::new(queries.into_iter().rev().map(String::from).collect()),
                evidence_seen: Mutex::new(Vec::new()),
            }
        }
    }

    #[async_trait]
    impl Planner for ScriptedPlanner {
        async fn next_query(&self, _question: &str, steps: &[ResearchStep]) -> Option<String> {
            self.evidence_seen.lock().unwrap().push(steps.len());
            self.queries.lock().unwrap().pop()
        }
    }

    // ── Backend routing / prefix preservation ───────────────────────────────

    #[test]
    fn reformulate_preserves_backend_prefix_normal() {
        // The selector survives; only the body is cleaned up.
        assert_eq!(
            reformulate_query("arxiv: can you tell me about diffusion models?"),
            "arxiv: diffusion models"
        );
        assert_eq!(
            reformulate_query("pubmed: CRISPR off-target effects"),
            "pubmed: CRISPR off-target effects"
        );
    }

    #[test]
    fn reformulate_preserves_uni_two_colon_structure_robust() {
        // `uni:` carries `<University>: <topic>` — the inner colon must survive.
        assert_eq!(
            reformulate_query("uni: Tsinghua University: quantum computing"),
            "uni: Tsinghua University: quantum computing"
        );
    }

    #[test]
    fn reformulate_plain_query_unaffected_by_prefix_logic_normal() {
        // A query that merely contains a word like "core" mid-sentence is NOT a
        // backend prefix — it's reformulated as a plain query (lead-ins stripped,
        // whitespace collapsed; stopwords are kept, matching reformulate_plain).
        assert_eq!(
            reformulate_query("tell me about the core scheduler"),
            "the core scheduler"
        );
    }

    #[test]
    fn split_backend_prefix_detects_known_selectors_normal() {
        assert_eq!(
            split_backend_prefix("openalex: graph neural nets"),
            (Some("openalex"), "graph neural nets")
        );
        assert_eq!(
            split_backend_prefix("just a normal query"),
            (None, "just a normal query")
        );
    }

    #[tokio::test]
    async fn combined_searcher_skips_local_for_backend_query_robust() {
        // A backend-prefixed query must not produce a "## Local codebase" block.
        let searcher = CombinedSearcher::new(std::env::temp_dir());
        // Web will likely fail offline, but the local-skip path is what we test:
        // the result must never contain a local-codebase section for a prefixed
        // query. (If web also fails we get Err, which is fine — no local block.)
        let res = searcher.search("arxiv: nonexistent-xyzzy-term", 2).await;
        if let Ok(text) = res {
            assert!(!text.contains("## Local codebase"));
        }
    }

    #[test]
    fn search_backends_doc_lists_core_indexes_normal() {
        for needle in ["arxiv:", "openalex:", "pubmed:", "uni:", "papers:"] {
            assert!(
                SEARCH_BACKENDS.contains(needle),
                "planner backend doc must mention {needle}"
            );
        }
    }

    // ── Agentic loop ─────────────────────────────────────────────────────────

    #[tokio::test]
    async fn agentic_loop_feeds_evidence_back_to_planner_normal() {
        let planner = ScriptedPlanner::new(vec!["first query", "second query", "third query"]);
        let searcher = MockSearcher::new();
        let synth = CountingSynth;
        let req = ResearchRequest::new("how does X work").with_max_steps(6);
        let report = run_research_agentic(req, &planner, &searcher, &synth)
            .await
            .expect("ok");
        // All three scripted queries ran, in order.
        assert_eq!(
            report.plan,
            vec!["first query", "second query", "third query"]
        );
        assert_eq!(report.steps.len(), 3);
        // The planner saw growing evidence each call: 0, 1, 2, then a 4th call
        // that returned None (3 steps visible) to stop.
        let seen = planner.evidence_seen.lock().unwrap().clone();
        assert_eq!(seen, vec![0, 1, 2, 3]);
    }

    #[tokio::test]
    async fn agentic_loop_respects_max_steps_robust() {
        let planner = ScriptedPlanner::new(vec!["q1", "q2", "q3", "q4", "q5"]);
        let searcher = MockSearcher::new();
        let synth = CountingSynth;
        let req = ResearchRequest::new("topic").with_max_steps(2);
        let report = run_research_agentic(req, &planner, &searcher, &synth)
            .await
            .expect("ok");
        assert_eq!(report.steps.len(), 2);
    }

    #[tokio::test]
    async fn agentic_loop_stops_on_repeated_query_robust() {
        let planner = ScriptedPlanner::new(vec!["dup", "dup", "dup"]);
        let searcher = MockSearcher::new();
        let synth = CountingSynth;
        let req = ResearchRequest::new("topic").with_max_steps(6);
        let report = run_research_agentic(req, &planner, &searcher, &synth)
            .await
            .expect("ok");
        // Only the first "dup" runs; the repeat breaks the loop.
        assert_eq!(report.plan, vec!["dup"]);
    }

    #[tokio::test]
    async fn agentic_loop_falls_back_when_planner_silent_normal() {
        // Planner immediately returns None → deterministic plan kicks in.
        let planner = ScriptedPlanner::new(vec![]);
        let searcher = MockSearcher::new();
        let synth = CountingSynth;
        let req = ResearchRequest::new("rust async runtimes").with_max_steps(3);
        let report = run_research_agentic(req, &planner, &searcher, &synth)
            .await
            .expect("ok");
        // Fell back to plan_subqueries (base + angles).
        assert!(report.steps.len() >= 1);
        assert_eq!(report.plan[0], "rust async runtimes");
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

    // ── Export ───────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn export_writes_db_artifact_normal() {
        let searcher = MockSearcher::new();
        let synth = CountingSynth;
        let req = ResearchRequest::new("rust async runtimes").with_max_steps(2);
        let report = run_research(req, &searcher, &synth).await.unwrap();

        let dir = std::env::temp_dir().join(format!("jfc-research-test-{}", std::process::id()));
        let artifact = report.export(&dir).expect("export ok");
        assert!(
            artifact
                .markdown_path
                .to_string_lossy()
                .starts_with("db:research:")
        );
        assert!(
            artifact
                .json_path
                .to_string_lossy()
                .starts_with("db:research:")
        );

        let row = jfc_knowledge::KnowledgeStore::open_default()
            .await
            .unwrap()
            .get_session_artifact(
                &format!("project:{}", jfc_knowledge::project_key(&dir)),
                RESEARCH_ARTIFACT_KIND,
                "rust-async-runtimes",
            )
            .await
            .unwrap()
            .expect("research artifact row");
        assert!(row.value_json.contains("Research"));
        assert!(row.value_json.contains("rust async runtimes"));
    }

    #[test]
    fn export_slug_is_filesystem_safe_robust() {
        assert_eq!(export_slug("Why is the sky blue?"), "why-is-the-sky-blue");
        assert_eq!(export_slug("   "), "research");
        assert!(
            export_slug("a/b\\c:d")
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-')
        );
    }

    // ── Query reformulation ────────────────────────────────────────────────────

    #[test]
    fn reformulate_strips_conversational_leadins_normal() {
        assert_eq!(
            reformulate_query("can you tell me about rust async runtimes?"),
            "rust async runtimes"
        );
        assert_eq!(
            reformulate_query("Please explain how tokio schedules tasks"),
            "how tokio schedules tasks"
        );
        assert_eq!(
            reformulate_query("find me the fastest sort"),
            "the fastest sort"
        );
    }

    #[test]
    fn reformulate_preserves_plain_query_and_collapses_ws_normal() {
        assert_eq!(
            reformulate_query("rust   async    runtimes"),
            "rust async runtimes"
        );
        assert_eq!(reformulate_query("borrow checker"), "borrow checker");
    }

    #[test]
    fn reformulate_never_empty_robust() {
        assert_eq!(reformulate_query(""), "");
        // A query that is *only* a lead-in falls back to the trimmed input.
        assert_eq!(reformulate_query("tell me about"), "tell me about");
        assert_eq!(reformulate_query("   ?  "), "?");
    }

    #[tokio::test]
    async fn run_research_reformulates_before_search_normal() {
        // A searcher that records the exact query string it received.
        struct RecordingSearcher {
            seen: std::sync::Mutex<Vec<String>>,
        }
        #[async_trait]
        impl Searcher for RecordingSearcher {
            async fn search(&self, query: &str, _max: usize) -> Result<String, String> {
                self.seen.lock().unwrap().push(query.to_owned());
                Ok(format!("results for [{query}]"))
            }
        }
        let searcher = RecordingSearcher {
            seen: std::sync::Mutex::new(Vec::new()),
        };
        let req = ResearchRequest::new("can you tell me about rust async").with_max_steps(1);
        let _ = run_research(req, &searcher, &CountingSynth).await.unwrap();
        // The base sub-query "can you tell me about rust async" was reformulated.
        let seen = searcher.seen.lock().unwrap();
        assert!(seen.iter().any(|q| q == "rust async"), "saw: {seen:?}");
    }

    #[tokio::test]
    async fn run_research_reformulation_off_searches_raw_robust() {
        struct RecordingSearcher {
            seen: std::sync::Mutex<Vec<String>>,
        }
        #[async_trait]
        impl Searcher for RecordingSearcher {
            async fn search(&self, query: &str, _max: usize) -> Result<String, String> {
                self.seen.lock().unwrap().push(query.to_owned());
                Ok("x".to_owned())
            }
        }
        let searcher = RecordingSearcher {
            seen: std::sync::Mutex::new(Vec::new()),
        };
        let req = ResearchRequest::new("tell me about rust")
            .with_max_steps(1)
            .with_reformulation(false);
        let _ = run_research(req, &searcher, &CountingSynth).await.unwrap();
        let seen = searcher.seen.lock().unwrap();
        assert!(
            seen.iter().any(|q| q == "tell me about rust"),
            "saw: {seen:?}"
        );
    }

    // ── Follow-up generation ───────────────────────────────────────────────────

    #[test]
    fn generate_followups_produces_distinct_questions_normal() {
        let fups = generate_followups("rust ownership", &["rust ownership".to_owned()]);
        assert!(!fups.is_empty() && fups.len() <= 3);
        assert!(fups.iter().all(|f| f.contains("rust ownership")));
        // No duplicates.
        let mut sorted = fups.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), fups.len());
    }

    #[test]
    fn generate_followups_skips_covered_angles_robust() {
        // Plan already covers limitations + latest → those follow-ups suppressed.
        let plan = vec![
            "x limitations criticism".to_owned(),
            "x latest developments".to_owned(),
        ];
        let fups = generate_followups("x", &plan);
        assert!(!fups.iter().any(|f| f.contains("limitations")));
        assert!(!fups.iter().any(|f| f.contains("latest developments")));
    }

    #[test]
    fn generate_followups_empty_question_is_empty_robust() {
        assert!(generate_followups("  ", &[]).is_empty());
    }

    #[tokio::test]
    async fn run_research_report_includes_followups_normal() {
        let searcher = MockSearcher::new();
        let req = ResearchRequest::new("rust async runtimes").with_max_steps(2);
        let report = run_research(req, &searcher, &CountingSynth).await.unwrap();
        assert!(!report.followups.is_empty());
        assert!(report.to_markdown().contains("Follow-up questions"));
    }

    // ── Citation anchoring ─────────────────────────────────────────────────────

    #[tokio::test]
    async fn report_citations_number_successful_steps_normal() {
        let searcher = MockSearcher::failing("limitations");
        let req = ResearchRequest::new("rust async runtimes").with_max_steps(4);
        let report = run_research(req, &searcher, &CountingSynth).await.unwrap();

        let cites = report.citations();
        // One citation per successful step, numbered from 1.
        assert_eq!(cites.len(), report.successful_steps());
        assert_eq!(cites.first().unwrap().number, 1);
        // Markdown anchors successful steps as [n]; JSON carries the citation list.
        let md = report.to_markdown();
        assert!(md.contains("- [1] ✅"));
        let json = report.to_json();
        assert!(json["citations"].as_array().unwrap().len() == cites.len());
    }
}

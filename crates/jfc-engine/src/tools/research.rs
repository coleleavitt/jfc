//! Execution entry point for the model-invocable `Research` tool.
//!
//! Routes a `Research` tool call to the deep-research orchestrator
//! ([`crate::research`]). When a provider + model are available (the normal
//! dispatch path), it runs the **agentic** loop
//! ([`crate::research::run_research_agentic`]): an [`crate::research::LlmPlanner`]
//! reformulates the next query from accumulated evidence, a
//! [`crate::research::CombinedSearcher`] searches the web *and* the local
//! codebase, and an [`crate::research::LlmSynthesizer`] writes the cited answer.
//! With no provider (older `execute_tool` callers) it falls back to the
//! deterministic plan + local merge so research still returns something.

use std::sync::Arc;

use jfc_provider::{ModelId, Provider};

use super::ExecutionResult;
use crate::research::{
    CombinedSearcher, DEFAULT_AGENTIC_STEPS, LlmPlanner, LlmSynthesizer, LocalSynthesizer,
    ResearchRequest, WebSearcher, run_research, run_research_agentic,
};

/// Deterministic, provider-free research pass (kept for callers without a
/// provider handy). Plans fixed angle queries, web-searches each, and merges
/// the evidence locally. Prefer [`execute_research_agentic`] when a model is
/// available.
pub async fn execute_research(question: &str, export: bool) -> ExecutionResult {
    let question = question.trim();
    if question.is_empty() {
        return ExecutionResult::failure("Research requires a non-empty `question`.");
    }

    let request = ResearchRequest::new(question);
    let searcher = WebSearcher;
    let synthesizer = LocalSynthesizer;
    let report = match run_research(request, &searcher, &synthesizer).await {
        Ok(report) => report,
        Err(e) => return ExecutionResult::failure(format!("Research failed: {e}")),
    };

    let mut body = report.to_markdown();
    if export {
        body.push_str(&export_suffix(&report));
    }
    ExecutionResult::success(body)
}

/// Agentic research pass: the model drives query reformulation and synthesis,
/// searching both the web and the local codebase. This is the production path
/// wired from the batched dispatcher (which has the active provider + model).
pub async fn execute_research_agentic(
    question: &str,
    export: bool,
    provider: Arc<dyn Provider>,
    model: ModelId,
) -> ExecutionResult {
    let question = question.trim();
    if question.is_empty() {
        return ExecutionResult::failure("Research requires a non-empty `question`.");
    }

    let request = ResearchRequest::new(question).with_max_steps(DEFAULT_AGENTIC_STEPS);
    let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    let searcher = CombinedSearcher::new(cwd);
    let planner = LlmPlanner::new(provider.clone(), model.clone());
    let synthesizer = LlmSynthesizer::new(provider, model);

    let report = match run_research_agentic(request, &planner, &searcher, &synthesizer).await {
        Ok(report) => report,
        Err(e) => return ExecutionResult::failure(format!("Research failed: {e}")),
    };

    let mut body = report.to_markdown();
    if export {
        body.push_str(&export_suffix(&report));
    }
    ExecutionResult::success(body)
}

/// Export the report to a temp artifact bundle and return a one-line status
/// suffix for the tool output.
fn export_suffix(report: &crate::research::ResearchReport) -> String {
    let dir = std::env::temp_dir().join("jfc-research");
    match report.export(&dir) {
        Ok(artifact) => format!(
            "\n\n_Artifact saved: `{}`_",
            artifact.markdown_path.display()
        ),
        Err(e) => format!("\n\n_(export failed: {e})_"),
    }
}

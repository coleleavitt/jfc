//! Execution entry point for the model-invocable `Research` tool.
//!
//! Routes a `Research` tool call to the deep-research orchestrator
//! ([`crate::research::run_research`]). It runs out-of-band — it does its own
//! web searches via [`crate::research::WebSearcher`] and a deterministic
//! [`crate::research::LocalSynthesizer`], so it needs no provider/transcript
//! context and fits the plain `execute_tool` dispatch path.

use super::ExecutionResult;
use crate::research::{LocalSynthesizer, ResearchRequest, WebSearcher, run_research};

/// Run a research pass and return its markdown report (optionally exporting a
/// durable artifact bundle).
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

//! Unified search orchestrator.
//!
//! Dispatches queries to multiple backends in parallel based on query
//! classification, then merges results via Reciprocal Rank Fusion (RRF).

use tokio::time::timeout;

use crate::backend::{BackendId, QueryClass, SearchResult, format_results, merge_rrf};
use crate::backends::backends_for_ids;

/// Default RRF k parameter. Higher values reduce the advantage of top-ranked items.
const RRF_K: f64 = 60.0;

/// Execute a unified search across multiple backends.
///
/// 1. Classifies the query to determine relevant backends
/// 2. Fires all backends in parallel with timeout
/// 3. Collects successful results, ignoring failures
/// 4. Merges via RRF deduplication
/// 5. Returns formatted output
pub async fn unified_search(query: &str, max_results: usize) -> Result<String, String> {
    let class = QueryClass::classify(query);
    let backend_ids = class.backends();

    let backend_results = search_backends(query, max_results, &backend_ids).await;

    // Flatten to check if we have any results
    let has_results = backend_results.iter().any(|v| !v.is_empty());
    if !has_results {
        return Err(format!(
            "All backends failed for query: \"{query}\" (class: {class:?})"
        ));
    }

    // Collect unique sources
    let unique_sources: Vec<BackendId> = {
        let mut seen = std::collections::HashSet::new();
        backend_results
            .iter()
            .flatten()
            .filter_map(|r| {
                if seen.insert(r.source) {
                    Some(r.source)
                } else {
                    None
                }
            })
            .collect()
    };

    // Merge via RRF (pass separate backend results for proper rank fusion)
    let merged = merge_rrf(backend_results, RRF_K, max_results);
    Ok(format_results(query, &merged, &unique_sources))
}

/// Search multiple backends in parallel, returning results grouped by backend.
async fn search_backends(
    query: &str,
    max_results: usize,
    backend_ids: &[BackendId],
) -> Vec<Vec<SearchResult>> {
    let backends = backends_for_ids(backend_ids);

    if backends.is_empty() {
        tracing::warn!("No available backends for query classification");
        return Vec::new();
    }

    // Fire all backends in parallel
    let futures: Vec<_> = backends
        .iter()
        .map(|backend| {
            let query = query.to_owned();
            let backend_timeout = backend.timeout();
            async move {
                let id = backend.id();
                match timeout(backend_timeout, backend.search(&query, max_results)).await {
                    Ok(Ok(results)) => {
                        tracing::debug!(
                            backend = %id.name(),
                            count = results.len(),
                            "Backend returned results"
                        );
                        results
                    }
                    Ok(Err(e)) => {
                        tracing::debug!(backend = %id.name(), error = %e, "Backend failed");
                        Vec::new()
                    }
                    Err(_) => {
                        tracing::debug!(backend = %id.name(), "Backend timed out");
                        Vec::new()
                    }
                }
            }
        })
        .collect();

    // Return results grouped by backend (don't flatten — needed for RRF)
    futures::future::join_all(futures).await
}

/// Search with explicit backend selection (for power users / prefix overrides).
pub async fn search_explicit(
    query: &str,
    max_results: usize,
    backend_id: BackendId,
) -> Result<String, String> {
    let backend_results = search_backends(query, max_results, &[backend_id]).await;
    let results: Vec<SearchResult> = backend_results.into_iter().flatten().collect();

    if results.is_empty() {
        return Err(format!(
            "{} returned no results for: \"{query}\"",
            backend_id.name()
        ));
    }

    Ok(format_results(query, &results, &[backend_id]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_class_determines_backends_normal() {
        let academic = QueryClass::Academic.backends();
        assert!(academic.contains(&BackendId::ArXiv));
        assert!(academic.contains(&BackendId::SemanticScholar));

        let general = QueryClass::General.backends();
        assert!(general.contains(&BackendId::Google));
        assert!(general.contains(&BackendId::Brave));
    }
}

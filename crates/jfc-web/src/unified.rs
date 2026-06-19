//! Unified search orchestrator.
//!
//! Routes search by query class. General web-style queries fuse Google,
//! Million Short, 4get, OpenAlex, DBLP, and Primo as the core result set;
//! academic/reference queries still fan out to specialized indexes.

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
    if fused_web_class(class) {
        return fused_web_search(query, max_results, class).await;
    }

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

fn fused_web_class(class: QueryClass) -> bool {
    matches!(
        class,
        QueryClass::General | QueryClass::Code | QueryClass::News
    )
}

fn core_web_backend(id: BackendId) -> bool {
    matches!(
        id,
        BackendId::Google
            | BackendId::MillionShort
            | BackendId::FourGet
            | BackendId::OpenAlex
            | BackendId::DBLP
            | BackendId::Primo
    )
}

#[cfg(test)]
fn support_backend_ids(class: QueryClass) -> Vec<BackendId> {
    class
        .backends()
        .into_iter()
        .filter(|id| !core_web_backend(*id))
        .collect()
}

async fn fused_web_search(
    query: &str,
    max_results: usize,
    class: QueryClass,
) -> Result<String, String> {
    let backend_results = search_backends(query, max_results, &class.backends()).await;
    if !backend_results.iter().any(|v| !v.is_empty()) {
        return Err(format!(
            "All backends failed for query: \"{query}\" (class: {class:?})"
        ));
    }

    let (core, support) = split_core_support_results(backend_results);
    let merged = if core.is_empty() {
        merge_rrf(support, RRF_K, max_results)
    } else {
        append_unique_result_groups(core, support, max_results)
    };
    let sources = unique_sources(&merged);
    Ok(format_results(query, &merged, &sources))
}

fn split_core_support_results(
    backend_results: Vec<Vec<SearchResult>>,
) -> (Vec<Vec<SearchResult>>, Vec<Vec<SearchResult>>) {
    let mut core = Vec::new();
    let mut support = Vec::new();
    for group in backend_results {
        if group.first().is_some_and(|r| core_web_backend(r.source)) {
            core.push(group);
        } else if !group.is_empty() {
            support.push(group);
        }
    }
    (core, support)
}

fn append_unique_result_groups(
    core: Vec<Vec<SearchResult>>,
    fallback: Vec<Vec<SearchResult>>,
    max_results: usize,
) -> Vec<SearchResult> {
    let mut seen = std::collections::HashSet::new();
    let mut merged = Vec::new();

    let core_depth = core.iter().map(Vec::len).max().unwrap_or(0);
    for rank in 0..core_depth {
        for group in &core {
            if let Some(result) = group.get(rank) {
                push_unique_result(result.clone(), &mut seen, &mut merged);
                if merged.len() >= max_results {
                    return merged;
                }
            }
        }
    }

    for result in fallback.into_iter().flatten() {
        push_unique_result(result, &mut seen, &mut merged);
        if merged.len() >= max_results {
            break;
        }
    }
    merged
}

fn push_unique_result(
    result: SearchResult,
    seen: &mut std::collections::HashSet<String>,
    merged: &mut Vec<SearchResult>,
) {
    let keys = result.dedup_keys();
    if keys.iter().any(|key| seen.contains(key)) {
        return;
    }
    for key in keys {
        seen.insert(key);
    }
    merged.push(result);
}

fn unique_sources(results: &[SearchResult]) -> Vec<BackendId> {
    let mut seen = std::collections::HashSet::new();
    results
        .iter()
        .filter_map(|r| {
            if seen.insert(r.source) {
                Some(r.source)
            } else {
                None
            }
        })
        .collect()
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
        assert!(general.contains(&BackendId::MillionShort));
        assert!(general.contains(&BackendId::FourGet));
        assert!(general.contains(&BackendId::OpenAlex));
        assert!(general.contains(&BackendId::DBLP));
        assert!(general.contains(&BackendId::Primo));
        assert!(!general.contains(&BackendId::ArXiv));
        assert!(!general.contains(&BackendId::Brave));
        assert!(!general.contains(&BackendId::SearXNG));
        assert!(!general.contains(&BackendId::DuckDuckGo));
    }

    #[test]
    fn general_code_news_use_fused_web_core_normal() {
        assert!(fused_web_class(QueryClass::General));
        assert!(fused_web_class(QueryClass::Code));
        assert!(fused_web_class(QueryClass::News));
        assert!(!fused_web_class(QueryClass::Academic));
        assert!(!fused_web_class(QueryClass::Reference));
    }

    #[test]
    fn support_backend_ids_exclude_core_backends_normal() {
        let support = support_backend_ids(QueryClass::General);

        assert!(!support.contains(&BackendId::Google));
        assert!(!support.contains(&BackendId::MillionShort));
        assert!(!support.contains(&BackendId::FourGet));
        assert!(!support.contains(&BackendId::OpenAlex));
        assert!(!support.contains(&BackendId::DBLP));
        assert!(!support.contains(&BackendId::Primo));
        assert!(support.is_empty());
    }

    #[test]
    fn append_unique_results_preserves_core_order_normal() {
        let google = SearchResult {
            title: "Google hit".into(),
            url: "https://example.com/google".into(),
            snippet: "primary".into(),
            doi: None,
            arxiv_id: None,
            source: BackendId::Google,
            rank: 1,
        };
        let million_short = SearchResult {
            title: "Million Short hit".into(),
            url: "https://example.com/million".into(),
            snippet: "discovery".into(),
            doi: None,
            arxiv_id: None,
            source: BackendId::MillionShort,
            rank: 1,
        };
        let fallback = SearchResult {
            title: "Fallback hit".into(),
            url: "https://example.com/fallback".into(),
            snippet: "fallback".into(),
            doi: None,
            arxiv_id: None,
            source: BackendId::Wikipedia,
            rank: 1,
        };

        let merged = append_unique_result_groups(
            vec![vec![google], vec![million_short]],
            vec![vec![fallback]],
            5,
        );

        assert_eq!(merged.len(), 3);
        assert_eq!(merged[0].source, BackendId::Google);
        assert_eq!(merged[0].title, "Google hit");
        assert_eq!(merged[1].source, BackendId::MillionShort);
        assert_eq!(merged[2].source, BackendId::Wikipedia);
    }

    #[test]
    fn split_core_support_results_groups_google_and_millionshort_normal() {
        let result = |source, title: &str| SearchResult {
            title: title.into(),
            url: format!("https://example.com/{title}"),
            snippet: String::new(),
            doi: None,
            arxiv_id: None,
            source,
            rank: 1,
        };

        let (core, support) = split_core_support_results(vec![
            vec![result(BackendId::Google, "google")],
            vec![result(BackendId::MillionShort, "million")],
            vec![result(BackendId::FourGet, "fourget")],
            vec![result(BackendId::OpenAlex, "openalex")],
            vec![result(BackendId::DBLP, "dblp")],
            vec![result(BackendId::Primo, "primo")],
            vec![result(BackendId::ArXiv, "arxiv")],
            vec![result(BackendId::Wikipedia, "wiki")],
        ]);

        assert_eq!(core.len(), 6);
        assert_eq!(core[0][0].source, BackendId::Google);
        assert_eq!(core[1][0].source, BackendId::MillionShort);
        assert_eq!(core[2][0].source, BackendId::FourGet);
        assert_eq!(core[3][0].source, BackendId::OpenAlex);
        assert_eq!(core[4][0].source, BackendId::DBLP);
        assert_eq!(core[5][0].source, BackendId::Primo);
        assert_eq!(support.len(), 2);
        assert_eq!(support[0][0].source, BackendId::ArXiv);
        assert_eq!(support[1][0].source, BackendId::Wikipedia);
    }

    #[test]
    fn fused_core_round_robins_google_and_millionshort_normal() {
        let result = |source, title: &str| SearchResult {
            title: title.into(),
            url: format!("https://example.com/{title}"),
            snippet: String::new(),
            doi: None,
            arxiv_id: None,
            source,
            rank: 1,
        };

        let merged = append_unique_result_groups(
            vec![
                vec![
                    result(BackendId::Google, "g1"),
                    result(BackendId::Google, "g2"),
                ],
                vec![
                    result(BackendId::MillionShort, "m1"),
                    result(BackendId::MillionShort, "m2"),
                ],
            ],
            Vec::new(),
            4,
        );

        assert_eq!(merged[0].title, "g1");
        assert_eq!(merged[1].title, "m1");
        assert_eq!(merged[2].title, "g2");
        assert_eq!(merged[3].title, "m2");
    }
}

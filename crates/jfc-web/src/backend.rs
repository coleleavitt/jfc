//! Search backend abstraction layer.
//!
//! Each backend implements the `SearchBackend` trait. The `UnifiedSearcher`
//! dispatches queries to multiple backends in parallel based on query
//! classification, then merges results via Reciprocal Rank Fusion (RRF).

use async_trait::async_trait;
use std::time::Duration;

// ═══════════════════════════════════════════════════════════════════════════
// Core types
// ═══════════════════════════════════════════════════════════════════════════

/// A single search result from any backend.
#[derive(Debug, Clone)]
pub struct SearchResult {
    /// Result title
    pub title: String,
    /// URL to the result
    pub url: String,
    /// Snippet/description
    pub snippet: String,
    /// Optional DOI for academic results
    pub doi: Option<String>,
    /// Optional arXiv ID
    pub arxiv_id: Option<String>,
    /// Source backend that produced this result
    pub source: BackendId,
    /// Rank within the backend's results (1-indexed)
    pub rank: usize,
}

impl SearchResult {
    /// All possible dedup keys for this result.
    /// A result can match another if ANY of their keys overlap.
    pub fn dedup_keys(&self) -> Vec<String> {
        let mut keys = Vec::new();
        if let Some(doi) = &self.doi {
            keys.push(format!("doi:{}", doi.to_lowercase()));
        }
        if let Some(arxiv) = &self.arxiv_id {
            keys.push(format!("arxiv:{}", arxiv.to_lowercase()));
        }
        // Always include normalized URL as fallback
        let url = self
            .url
            .trim_start_matches("https://")
            .trim_start_matches("http://")
            .trim_start_matches("www.")
            .trim_end_matches('/');
        keys.push(format!("url:{}", url.to_lowercase()));
        keys
    }

    /// Primary dedup key (for backwards compat / display).
    pub fn dedup_key(&self) -> String {
        self.dedup_keys().into_iter().next().unwrap_or_default()
    }
}

/// Backend identifier for tracking result sources.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BackendId {
    // Web search
    Google,
    Brave,
    SearXNG,
    DuckDuckGo,
    Tavily,
    Exa,
    // Academic
    ArXiv,
    SemanticScholar,
    OpenAlex,
    DBLP,
    Crossref,
    PubMed,
    DOAJ,
    CORE,
    // Reference
    Wikipedia,
    // Specialized
    Primo,
    Unpaywall,
}

impl BackendId {
    pub fn name(&self) -> &'static str {
        match self {
            Self::Google => "Google",
            Self::Brave => "Brave",
            Self::SearXNG => "SearXNG",
            Self::DuckDuckGo => "DuckDuckGo",
            Self::Tavily => "Tavily",
            Self::Exa => "Exa",
            Self::ArXiv => "arXiv",
            Self::SemanticScholar => "Semantic Scholar",
            Self::OpenAlex => "OpenAlex",
            Self::DBLP => "DBLP",
            Self::Crossref => "Crossref",
            Self::PubMed => "PubMed",
            Self::DOAJ => "DOAJ",
            Self::CORE => "CORE",
            Self::Wikipedia => "Wikipedia",
            Self::Primo => "Primo",
            Self::Unpaywall => "Unpaywall",
        }
    }

    /// Whether this backend requires an API key.
    pub fn requires_key(&self) -> bool {
        matches!(self, Self::Brave | Self::Tavily | Self::Exa | Self::CORE)
    }

    /// Category for query routing.
    pub fn category(&self) -> BackendCategory {
        match self {
            Self::Google
            | Self::Brave
            | Self::SearXNG
            | Self::DuckDuckGo
            | Self::Tavily
            | Self::Exa => BackendCategory::Web,
            Self::ArXiv
            | Self::SemanticScholar
            | Self::OpenAlex
            | Self::DBLP
            | Self::Crossref
            | Self::PubMed
            | Self::DOAJ
            | Self::CORE => BackendCategory::Academic,
            Self::Wikipedia => BackendCategory::Reference,
            Self::Primo | Self::Unpaywall => BackendCategory::Specialized,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackendCategory {
    Web,
    Academic,
    Reference,
    Specialized,
}

// ═══════════════════════════════════════════════════════════════════════════
// Query classification
// ═══════════════════════════════════════════════════════════════════════════

/// Classified query type determines which backends to query.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueryClass {
    /// General web search — news, docs, tutorials, etc.
    General,
    /// Academic/research — papers, citations, preprints
    Academic,
    /// Code/programming — libraries, frameworks, APIs
    Code,
    /// Reference/factual — definitions, encyclopedic
    Reference,
    /// News/current events
    News,
}

/// Term lists for query classification.
mod terms {
    pub const ACADEMIC: &[&str] = &[
        "paper",
        "papers",
        "arxiv",
        "research",
        "study",
        "journal",
        "citation",
        "preprint",
        "publication",
        "thesis",
        "dissertation",
        "peer review",
        "methodology",
        "hypothesis",
        "experiment",
        "findings",
        "literature",
        "survey",
        "benchmark",
        "dataset",
        "neural network",
        "machine learning",
        "deep learning",
        "transformer",
        "attention mechanism",
        "llm",
        "gpt",
        "bert",
        "diffusion",
        "reinforcement learning",
        "et al",
        "doi:",
        "10.",
    ];

    pub const CODE: &[&str] = &[
        "rust",
        "python",
        "javascript",
        "typescript",
        "golang",
        "java",
        "crate",
        "npm",
        "pip",
        "cargo",
        "library",
        "framework",
        "api",
        "sdk",
        "implementation",
        "example",
        "tutorial",
        "documentation",
        "docs",
        "github",
        "gitlab",
        "stackoverflow",
        "error",
        "bug",
        "fix",
        "how to",
        "async",
        "trait",
        "struct",
        "enum",
        "function",
        "method",
    ];

    pub const REFERENCE: &[&str] = &[
        "what is",
        "who is",
        "when was",
        "where is",
        "definition",
        "meaning",
        "wikipedia",
        "history of",
        "biography",
        "explain",
    ];

    pub const NEWS: &[&str] = &[
        "news",
        "latest",
        "today",
        "yesterday",
        "2024",
        "2025",
        "2026",
        "announcement",
        "release",
        "update",
        "breaking",
        "reported",
    ];

    /// Count how many terms from the list appear in the query.
    pub fn score(query: &str, terms: &[&str]) -> usize {
        terms.iter().filter(|t| query.contains(*t)).count()
    }
}

impl QueryClass {
    /// Classify a query based on content signals.
    pub fn classify(query: &str) -> Self {
        let q = query.to_lowercase();

        let scores = [
            (Self::Academic, terms::score(&q, terms::ACADEMIC)),
            (Self::Code, terms::score(&q, terms::CODE)),
            (Self::Reference, terms::score(&q, terms::REFERENCE)),
            (Self::News, terms::score(&q, terms::NEWS)),
        ];

        scores
            .into_iter()
            .filter(|(_, score)| *score > 0)
            .max_by_key(|(_, score)| *score)
            .map(|(class, _)| class)
            .unwrap_or(Self::General)
    }

    /// Get the backends to query for this class.
    pub fn backends(&self) -> Vec<BackendId> {
        match self {
            Self::General => vec![BackendId::Google, BackendId::Brave, BackendId::SearXNG],
            Self::Academic => vec![
                BackendId::ArXiv,
                BackendId::SemanticScholar,
                BackendId::OpenAlex,
                BackendId::DBLP,
                BackendId::Google, // Also include web for broader coverage
            ],
            Self::Code => vec![BackendId::Google, BackendId::Brave, BackendId::SearXNG],
            Self::Reference => vec![
                BackendId::Wikipedia,
                BackendId::Google,
                BackendId::DuckDuckGo,
            ],
            Self::News => vec![BackendId::Google, BackendId::Brave, BackendId::SearXNG],
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Backend trait
// ═══════════════════════════════════════════════════════════════════════════

/// Result from a backend search attempt.
pub type BackendResult = Result<Vec<SearchResult>, String>;

/// A search backend that can be queried.
#[async_trait]
pub trait SearchBackend: Send + Sync {
    /// Backend identifier.
    fn id(&self) -> BackendId;

    /// Check if this backend is available (has required API keys, etc).
    fn is_available(&self) -> bool;

    /// Execute a search query.
    async fn search(&self, query: &str, max_results: usize) -> BackendResult;

    /// Timeout for this backend's requests.
    fn timeout(&self) -> Duration {
        Duration::from_secs(10)
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Reciprocal Rank Fusion (RRF)
// ═══════════════════════════════════════════════════════════════════════════

/// Merge results from multiple backends using Reciprocal Rank Fusion.
///
/// RRF score for each result: `score = Σ 1/(k + rank)` across all backends
/// where k is a constant (typically 60) that prevents high-ranked items from
/// dominating too strongly.
///
/// Deduplication uses union-find: two results are the same if they share ANY
/// identifier (DOI, arXiv ID, or normalized URL).
pub fn merge_rrf(results: Vec<Vec<SearchResult>>, k: f64, max_results: usize) -> Vec<SearchResult> {
    use std::collections::HashMap;

    // Union-find: map each key to a canonical group ID
    let mut key_to_group: HashMap<String, usize> = HashMap::new();
    let mut group_results: HashMap<usize, Vec<(SearchResult, f64)>> = HashMap::new();
    let mut next_group: usize = 0;

    for backend_results in results {
        for result in backend_results {
            let keys = result.dedup_keys();
            let rrf_score = 1.0 / (k + result.rank as f64);

            // Find if any key already belongs to a group
            let existing_group = keys.iter().find_map(|k| key_to_group.get(k).copied());

            let group_id = match existing_group {
                Some(g) => g,
                None => {
                    let g = next_group;
                    next_group += 1;
                    g
                }
            };

            // Assign all keys to this group
            for key in keys {
                key_to_group.insert(key, group_id);
            }

            // Add result to group
            group_results
                .entry(group_id)
                .or_default()
                .push((result, rrf_score));
        }
    }

    // For each group: sum RRF scores, keep best result (most metadata)
    let mut final_results: Vec<(f64, SearchResult)> = group_results
        .into_values()
        .map(|results| {
            let total_score: f64 = results.iter().map(|(_, s)| s).sum();
            // Pick result with most metadata
            let best = results
                .into_iter()
                .map(|(r, _)| r)
                .max_by_key(|r| {
                    let mut score = 0;
                    if r.doi.is_some() {
                        score += 2;
                    }
                    if r.arxiv_id.is_some() {
                        score += 1;
                    }
                    score
                })
                .unwrap();
            (total_score, best)
        })
        .collect();

    // Sort by RRF score descending
    final_results.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));

    // Take top results
    final_results
        .into_iter()
        .take(max_results)
        .map(|(_, r)| r)
        .collect()
}

// ═══════════════════════════════════════════════════════════════════════════
// Format results
// ═══════════════════════════════════════════════════════════════════════════

/// Format merged results as a readable string.
pub fn format_results(query: &str, results: &[SearchResult], sources: &[BackendId]) -> String {
    let source_names: Vec<_> = sources.iter().map(|b| b.name()).collect();
    let mut out = format!(
        "Search: \"{}\" — {} results from {}\n\n",
        query,
        results.len(),
        source_names.join(", ")
    );

    if results.is_empty() {
        out.push_str("No results found.\n");
        return out;
    }

    for (i, result) in results.iter().enumerate() {
        out.push_str(&format!("{}. {}\n", i + 1, result.title));
        out.push_str(&format!("   URL: {}\n", result.url));
        if let Some(doi) = &result.doi {
            out.push_str(&format!("   DOI: {doi}\n"));
        }
        if let Some(arxiv) = &result.arxiv_id {
            out.push_str(&format!("   arXiv: {arxiv}\n"));
        }
        out.push_str(&format!("   {}\n", result.snippet));
        out.push_str(&format!("   [{}]\n\n", result.source.name()));
    }

    out
}

// ═══════════════════════════════════════════════════════════════════════════
// Tests
// ═══════════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_class_academic_detection_normal() {
        assert_eq!(
            QueryClass::classify("transformer attention paper arxiv"),
            QueryClass::Academic
        );
        assert_eq!(
            QueryClass::classify("deep learning research survey"),
            QueryClass::Academic
        );
        assert_eq!(
            QueryClass::classify("neural network benchmark dataset"),
            QueryClass::Academic
        );
    }

    #[test]
    fn query_class_code_detection_normal() {
        assert_eq!(
            QueryClass::classify("rust async trait implementation"),
            QueryClass::Code
        );
        assert_eq!(
            QueryClass::classify("python pip install tutorial"),
            QueryClass::Code
        );
    }

    #[test]
    fn query_class_reference_detection_normal() {
        assert_eq!(
            QueryClass::classify("what is a monad definition"),
            QueryClass::Reference
        );
        assert_eq!(
            QueryClass::classify("who is Alan Turing biography"),
            QueryClass::Reference
        );
    }

    #[test]
    fn query_class_general_fallback_normal() {
        assert_eq!(
            QueryClass::classify("best restaurants in tokyo"),
            QueryClass::General
        );
        assert_eq!(
            QueryClass::classify("weather forecast"),
            QueryClass::General
        );
    }

    #[test]
    fn dedup_key_prefers_doi_normal() {
        let r = SearchResult {
            title: "Test".into(),
            url: "https://example.com/paper".into(),
            snippet: "...".into(),
            doi: Some("10.1234/test".into()),
            arxiv_id: Some("2401.00001".into()),
            source: BackendId::SemanticScholar,
            rank: 1,
        };
        assert_eq!(r.dedup_key(), "doi:10.1234/test");
    }

    #[test]
    fn dedup_key_falls_back_to_arxiv_normal() {
        let r = SearchResult {
            title: "Test".into(),
            url: "https://arxiv.org/abs/2401.00001".into(),
            snippet: "...".into(),
            doi: None,
            arxiv_id: Some("2401.00001".into()),
            source: BackendId::ArXiv,
            rank: 1,
        };
        assert_eq!(r.dedup_key(), "arxiv:2401.00001");
    }

    #[test]
    fn dedup_key_normalizes_url_robust() {
        let r = SearchResult {
            title: "Test".into(),
            url: "https://www.Example.COM/Page/".into(),
            snippet: "...".into(),
            doi: None,
            arxiv_id: None,
            source: BackendId::Google,
            rank: 1,
        };
        assert_eq!(r.dedup_key(), "url:example.com/page");
    }

    #[test]
    fn merge_rrf_deduplicates_and_ranks_normal() {
        let backend1 = vec![
            SearchResult {
                title: "Paper A".into(),
                url: "https://arxiv.org/abs/2401.00001".into(),
                snippet: "From arXiv".into(),
                doi: None,
                arxiv_id: Some("2401.00001".into()),
                source: BackendId::ArXiv,
                rank: 1,
            },
            SearchResult {
                title: "Paper B".into(),
                url: "https://example.com/b".into(),
                snippet: "...".into(),
                doi: None,
                arxiv_id: None,
                source: BackendId::ArXiv,
                rank: 2,
            },
        ];

        let backend2 = vec![
            SearchResult {
                title: "Paper A (S2)".into(),
                url: "https://semanticscholar.org/paper/123".into(),
                snippet: "From S2".into(),
                doi: Some("10.1234/a".into()),
                arxiv_id: Some("2401.00001".into()), // Same paper, different URL
                source: BackendId::SemanticScholar,
                rank: 1,
            },
            SearchResult {
                title: "Paper C".into(),
                url: "https://example.com/c".into(),
                snippet: "...".into(),
                doi: None,
                arxiv_id: None,
                source: BackendId::SemanticScholar,
                rank: 2,
            },
        ];

        let merged = merge_rrf(vec![backend1, backend2], 60.0, 10);

        // Paper A should be first (appears in both backends, highest combined RRF)
        assert_eq!(merged.len(), 3); // A (deduped), B, C
        assert!(merged[0].arxiv_id.as_deref() == Some("2401.00001"));
        // Should have DOI from S2 version (more metadata)
        assert!(merged[0].doi.is_some());
    }

    #[test]
    fn backend_category_classification_normal() {
        assert_eq!(BackendId::Google.category(), BackendCategory::Web);
        assert_eq!(BackendId::ArXiv.category(), BackendCategory::Academic);
        assert_eq!(BackendId::Wikipedia.category(), BackendCategory::Reference);
    }

    #[test]
    fn backend_requires_key_normal() {
        assert!(BackendId::Brave.requires_key());
        assert!(BackendId::Tavily.requires_key());
        assert!(!BackendId::ArXiv.requires_key());
        assert!(!BackendId::Wikipedia.requires_key());
    }
}

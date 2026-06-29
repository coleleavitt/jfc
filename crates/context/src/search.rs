use crate::ContextSkeletonError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SearchSourceKind {
    Memory,
    Session,
    Git,
    Codegraph,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SearchStatus {
    Searched,
    NoSources,
    NoAvailableSources,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SearchSourceStatus {
    Searched,
    Missing,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SearchQuery(String);

impl SearchQuery {
    pub fn new(query: impl Into<String>) -> Result<Self, ContextSkeletonError> {
        let query = query.into();
        if query.trim().is_empty() {
            return Err(ContextSkeletonError::EmptySearchQuery);
        }

        Ok(Self(query))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchCandidate {
    title: String,
    excerpt: String,
    score: u16,
}

impl SearchCandidate {
    pub fn new(title: impl Into<String>, excerpt: impl Into<String>, score: u16) -> Self {
        Self {
            title: title.into(),
            excerpt: excerpt.into(),
            score,
        }
    }

    pub fn title(&self) -> &str {
        &self.title
    }

    pub fn excerpt(&self) -> &str {
        &self.excerpt
    }

    pub fn score(&self) -> u16 {
        self.score
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchHit {
    source: SearchSourceKind,
    title: String,
    excerpt: String,
    score: u16,
}

impl SearchHit {
    fn from_candidate(source: SearchSourceKind, candidate: SearchCandidate) -> Self {
        Self {
            source,
            title: candidate.title,
            excerpt: candidate.excerpt,
            score: candidate.score,
        }
    }

    pub fn source(&self) -> SearchSourceKind {
        self.source
    }

    pub fn title(&self) -> &str {
        &self.title
    }

    pub fn excerpt(&self) -> &str {
        &self.excerpt
    }

    pub fn score(&self) -> u16 {
        self.score
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchSourceOutput {
    candidates: Vec<SearchCandidate>,
}

impl SearchSourceOutput {
    pub fn new(candidates: impl Into<Vec<SearchCandidate>>) -> Self {
        Self {
            candidates: candidates.into(),
        }
    }

    pub fn candidates(&self) -> &[SearchCandidate] {
        &self.candidates
    }
}

pub trait ContextSearchSource: Send + Sync {
    fn kind(&self) -> SearchSourceKind;
    fn search(&self, query: &SearchQuery) -> SearchSourceOutput;
}

pub enum SearchSourceSlot {
    Available(Box<dyn ContextSearchSource>),
    Missing(SearchSourceKind),
}

impl SearchSourceSlot {
    pub fn available(source: Box<dyn ContextSearchSource>) -> Self {
        Self::Available(source)
    }

    pub fn missing(kind: SearchSourceKind) -> Self {
        Self::Missing(kind)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchSourceReport {
    kind: SearchSourceKind,
    status: SearchSourceStatus,
    result_count: usize,
}

impl SearchSourceReport {
    fn searched(kind: SearchSourceKind, result_count: usize) -> Self {
        Self {
            kind,
            status: SearchSourceStatus::Searched,
            result_count,
        }
    }

    fn missing(kind: SearchSourceKind) -> Self {
        Self {
            kind,
            status: SearchSourceStatus::Missing,
            result_count: 0,
        }
    }

    pub fn kind(&self) -> SearchSourceKind {
        self.kind
    }

    pub fn status(&self) -> SearchSourceStatus {
        self.status
    }

    pub fn result_count(&self) -> usize {
        self.result_count
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchResponse {
    query: SearchQuery,
    status: SearchStatus,
    results: Vec<SearchHit>,
    sources: Vec<SearchSourceReport>,
}

impl SearchResponse {
    pub fn query(&self) -> &SearchQuery {
        &self.query
    }

    pub fn status(&self) -> SearchStatus {
        self.status
    }

    pub fn results(&self) -> &[SearchHit] {
        &self.results
    }

    pub fn sources(&self) -> &[SearchSourceReport] {
        &self.sources
    }
}

pub struct ContextSearchFacade {
    slots: Vec<SearchSourceSlot>,
}

impl ContextSearchFacade {
    pub fn new(slots: impl IntoIterator<Item = SearchSourceSlot>) -> Self {
        Self {
            slots: slots.into_iter().collect(),
        }
    }

    pub fn search(&self, query: &SearchQuery) -> SearchResponse {
        if self.slots.is_empty() {
            return SearchResponse {
                query: query.clone(),
                status: SearchStatus::NoSources,
                results: Vec::new(),
                sources: Vec::new(),
            };
        }

        let mut ranked_hits = Vec::new();
        let mut reports = Vec::with_capacity(self.slots.len());
        let mut available_sources = 0;

        for (source_order, slot) in self.slots.iter().enumerate() {
            match slot {
                SearchSourceSlot::Available(source) => {
                    available_sources += 1;
                    let kind = source.kind();
                    let output = source.search(query);
                    let result_count = output.candidates.len();

                    ranked_hits.extend(output.candidates.into_iter().enumerate().map(
                        |(hit_order, candidate)| RankedSearchHit {
                            source_order,
                            hit_order,
                            hit: SearchHit::from_candidate(kind, candidate),
                        },
                    ));
                    reports.push(SearchSourceReport::searched(kind, result_count));
                }
                SearchSourceSlot::Missing(kind) => {
                    reports.push(SearchSourceReport::missing(*kind));
                }
            }
        }

        ranked_hits.sort_by(|left, right| {
            right
                .hit
                .score
                .cmp(&left.hit.score)
                .then_with(|| left.source_order.cmp(&right.source_order))
                .then_with(|| left.hit_order.cmp(&right.hit_order))
        });

        SearchResponse {
            query: query.clone(),
            status: if available_sources == 0 {
                SearchStatus::NoAvailableSources
            } else {
                SearchStatus::Searched
            },
            results: ranked_hits.into_iter().map(|ranked| ranked.hit).collect(),
            sources: reports,
        }
    }
}

struct RankedSearchHit {
    source_order: usize,
    hit_order: usize,
    hit: SearchHit,
}

#[cfg(test)]
mod tests {
    use super::{
        ContextSearchFacade, ContextSearchSource, SearchCandidate, SearchQuery, SearchSourceKind,
        SearchSourceOutput, SearchSourceSlot, SearchSourceStatus, SearchStatus,
    };

    #[derive(Debug)]
    struct FakeSource {
        kind: SearchSourceKind,
        candidates: Vec<SearchCandidate>,
    }

    impl ContextSearchSource for FakeSource {
        fn kind(&self) -> SearchSourceKind {
            self.kind
        }

        fn search(&self, _query: &SearchQuery) -> SearchSourceOutput {
            SearchSourceOutput::new(self.candidates.clone())
        }
    }

    #[test]
    fn fake_sources_are_merged_by_descending_score_normal() {
        let facade = ContextSearchFacade::new([
            SearchSourceSlot::available(Box::new(FakeSource {
                kind: SearchSourceKind::Memory,
                candidates: vec![SearchCandidate::new("memory-low", "older memory", 30)],
            })),
            SearchSourceSlot::available(Box::new(FakeSource {
                kind: SearchSourceKind::Session,
                candidates: vec![SearchCandidate::new("session-high", "recent session", 90)],
            })),
        ]);
        let query = SearchQuery::new("recent decision").expect("valid query");
        let response = facade.search(&query);

        assert_eq!(response.status(), SearchStatus::Searched);
        assert_eq!(response.results().len(), 2);
        assert_eq!(response.results()[0].title(), "session-high");
        assert_eq!(response.results()[0].source(), SearchSourceKind::Session);
        assert_eq!(response.results()[1].title(), "memory-low");
    }

    #[test]
    fn empty_facade_returns_typed_no_sources_result_malformed() {
        let facade = ContextSearchFacade::new([]);
        let query = SearchQuery::new("anything").expect("valid query");
        let response = facade.search(&query);

        assert_eq!(response.status(), SearchStatus::NoSources);
        assert!(response.results().is_empty());
        assert!(response.sources().is_empty());
    }

    #[test]
    fn missing_source_is_reported_without_panicking_malformed() {
        let facade =
            ContextSearchFacade::new([SearchSourceSlot::missing(SearchSourceKind::Codegraph)]);
        let query = SearchQuery::new("symbol path").expect("valid query");
        let response = facade.search(&query);

        assert_eq!(response.status(), SearchStatus::NoAvailableSources);
        assert!(response.results().is_empty());
        assert_eq!(response.sources().len(), 1);
        assert_eq!(response.sources()[0].kind(), SearchSourceKind::Codegraph);
        assert_eq!(response.sources()[0].status(), SearchSourceStatus::Missing);
    }
}

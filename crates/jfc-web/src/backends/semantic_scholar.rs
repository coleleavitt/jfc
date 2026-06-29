//! Semantic Scholar backend.

use crate::backend::{BackendId, BackendResult, SearchBackend};
use async_trait::async_trait;

pub struct SemanticScholarBackend;

#[async_trait]
impl SearchBackend for SemanticScholarBackend {
    fn id(&self) -> BackendId {
        BackendId::SemanticScholar
    }

    fn is_available(&self) -> bool {
        // Semantic Scholar public API is always available
        true
    }

    async fn search(&self, query: &str, max_results: usize) -> BackendResult {
        crate::search_semantic_scholar_structured(query, max_results).await
    }
}

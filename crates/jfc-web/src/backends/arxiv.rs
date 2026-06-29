//! arXiv backend.

use crate::backend::{BackendId, BackendResult, SearchBackend};
use async_trait::async_trait;

pub struct ArXivBackend;

#[async_trait]
impl SearchBackend for ArXivBackend {
    fn id(&self) -> BackendId {
        BackendId::ArXiv
    }

    fn is_available(&self) -> bool {
        // arXiv is always available (no key required)
        true
    }

    async fn search(&self, query: &str, max_results: usize) -> BackendResult {
        crate::search_arxiv_structured(query, max_results).await
    }
}

//! Wikipedia backend.

use crate::backend::{BackendId, BackendResult, SearchBackend};
use async_trait::async_trait;

pub struct WikipediaBackend;

#[async_trait]
impl SearchBackend for WikipediaBackend {
    fn id(&self) -> BackendId {
        BackendId::Wikipedia
    }

    fn is_available(&self) -> bool {
        // Wikipedia is always available (no key required)
        true
    }

    async fn search(&self, query: &str, max_results: usize) -> BackendResult {
        crate::search_wikipedia_structured(query, max_results).await
    }
}

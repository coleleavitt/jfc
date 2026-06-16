//! SearXNG meta-search backend.

use crate::backend::{BackendId, BackendResult, SearchBackend};
use async_trait::async_trait;

pub struct SearXNGBackend;

#[async_trait]
impl SearchBackend for SearXNGBackend {
    fn id(&self) -> BackendId {
        BackendId::SearXNG
    }

    fn is_available(&self) -> bool {
        // SearXNG is always available (uses public instances by default)
        true
    }

    async fn search(&self, query: &str, max_results: usize) -> BackendResult {
        crate::search_searxng_structured(query, max_results).await
    }
}

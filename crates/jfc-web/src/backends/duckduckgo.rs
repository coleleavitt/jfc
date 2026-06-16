//! DuckDuckGo Instant Answer backend.

use crate::backend::{BackendId, BackendResult, SearchBackend};
use async_trait::async_trait;

pub struct DuckDuckGoBackend;

#[async_trait]
impl SearchBackend for DuckDuckGoBackend {
    fn id(&self) -> BackendId {
        BackendId::DuckDuckGo
    }

    fn is_available(&self) -> bool {
        // DuckDuckGo Instant Answer is always available (no key required)
        true
    }

    async fn search(&self, query: &str, max_results: usize) -> BackendResult {
        crate::search_duckduckgo_structured(query, max_results).await
    }
}

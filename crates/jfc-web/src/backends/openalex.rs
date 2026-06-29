//! OpenAlex backend.

use crate::backend::{BackendId, BackendResult, SearchBackend};
use async_trait::async_trait;

pub struct OpenAlexBackend;

#[async_trait]
impl SearchBackend for OpenAlexBackend {
    fn id(&self) -> BackendId {
        BackendId::OpenAlex
    }

    fn is_available(&self) -> bool {
        // OpenAlex is always available (no key required)
        true
    }

    async fn search(&self, query: &str, max_results: usize) -> BackendResult {
        crate::search_openalex_structured(query, max_results).await
    }
}

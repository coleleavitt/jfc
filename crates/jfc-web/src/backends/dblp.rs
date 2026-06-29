//! DBLP backend.

use crate::backend::{BackendId, BackendResult, SearchBackend};
use async_trait::async_trait;

pub struct DBLPBackend;

#[async_trait]
impl SearchBackend for DBLPBackend {
    fn id(&self) -> BackendId {
        BackendId::DBLP
    }

    fn is_available(&self) -> bool {
        // DBLP is always available (no key required)
        true
    }

    async fn search(&self, query: &str, max_results: usize) -> BackendResult {
        crate::search_dblp_structured(query, max_results).await
    }
}

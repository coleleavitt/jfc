//! Google CSE backend.

use crate::backend::{BackendId, BackendResult, SearchBackend};
use crate::key_pool;
use async_trait::async_trait;

pub struct GoogleBackend;

#[async_trait]
impl SearchBackend for GoogleBackend {
    fn id(&self) -> BackendId {
        BackendId::Google
    }

    fn is_available(&self) -> bool {
        key_pool().next().is_some()
    }

    async fn search(&self, query: &str, max_results: usize) -> BackendResult {
        crate::search_google_structured(query, max_results).await
    }
}

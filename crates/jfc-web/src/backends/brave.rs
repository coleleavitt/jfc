//! Brave Search backend.

use crate::backend::{BackendId, BackendResult, SearchBackend};
use async_trait::async_trait;

pub struct BraveBackend;

#[async_trait]
impl SearchBackend for BraveBackend {
    fn id(&self) -> BackendId {
        BackendId::Brave
    }

    fn is_available(&self) -> bool {
        std::env::var("BRAVE_API_KEY").is_ok()
    }

    async fn search(&self, query: &str, max_results: usize) -> BackendResult {
        crate::search_brave_structured(query, max_results).await
    }
}

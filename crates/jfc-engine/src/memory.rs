//! Memory system — delegated to the `jfc-memory` crate.
//!
//! Re-exports the public API so existing `crate::memory::*` call sites
//! continue to compile without modification.

pub use jfc_memory::{
    MemoryLevel, MemoryScope, MemoryType, create_memory, delete_memory, format_existing_memories,
    load_all_memories, sync_team_memory,
};

pub async fn create_project_context_memory(
    body: &str,
    project_root: &std::path::Path,
) -> Result<String, String> {
    create_memory(
        MemoryLevel::Project,
        MemoryType::Context,
        MemoryScope::Private,
        body,
        project_root,
    )
    .await
}

//! Memory system — delegated to the `jfc-memory` crate.
//!
//! Re-exports the public API so existing `crate::memory::*` call sites
//! continue to compile without modification.

pub use jfc_memory::{
    MemoryLevel, MemoryScope, MemoryType, create_memory, delete_memory, format_existing_memories,
    load_all_memories, render_memories_section, sync_team_memory,
};

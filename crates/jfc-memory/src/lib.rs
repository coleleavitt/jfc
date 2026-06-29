//! Memory recall and persistence: manages user-level and project-level
//! memories stored as Markdown files under `.jfc/memory/`. Provides two-phase
//! LLM-driven recall (select → inject) so the agent can surface relevant past
//! context at the start of each turn.

pub mod recall;
pub mod store;

pub use recall::{
    Fact, cached_recall, format_recall_block, is_enabled, run_recall, run_recall_excluding_visible,
    select_relevant_memories, set_runtime_override, synthesize_memories,
};
pub use store::{
    CreateMemoryResult, MemoryEntry, MemoryFrontmatter, MemoryLevel, MemoryScope, MemoryType,
    TeamMemoryConflict, TeamMemorySyncReport, create_memory, create_memory_checked, delete_memory,
    format_existing_memories, load_all_memories, project_memory_dir, sync_team_memory,
    team_memory_dir, user_memory_dir,
};

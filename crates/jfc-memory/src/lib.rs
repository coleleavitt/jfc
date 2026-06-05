//! Memory recall and persistence: manages user-level and project-level
//! memories stored as Markdown files under `.jfc/memory/`. Provides two-phase
//! LLM-driven recall (select → inject) so the agent can surface relevant past
//! context at the start of each turn.

pub mod recall;
pub mod store;

pub use recall::{
    Fact, cached_recall, format_recall_block, is_enabled, run_recall, select_relevant_memories,
    set_runtime_override, synthesize_memories,
};
pub use store::{
    MemoryEntry, MemoryFrontmatter, MemoryLevel, MemoryScope, MemoryType, TeamMemoryConflict,
    TeamMemorySyncReport, create_memory, delete_memory, format_existing_memories, is_memory_path,
    load_all_memories, project_memory_dir, render_memories_section, sync_team_memory,
    team_memory_dir, user_memory_dir,
};

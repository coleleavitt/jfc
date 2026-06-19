use super::super::memory::append_memory_recall_context;

#[test]
fn memory_recall_context_does_not_dump_all_memories_regression() {
    let memory = jfc_memory::MemoryEntry {
        path: std::path::PathBuf::from("big-memory.md"),
        level: jfc_memory::MemoryLevel::Project,
        frontmatter: jfc_memory::MemoryFrontmatter::new(
            jfc_memory::MemoryType::Context,
            jfc_memory::MemoryScope::Team,
        ),
        body: format!("request-only-big-memory {}", "x".repeat(16_000)),
    };
    let mut system = String::from("base prompt");

    let recalled_chars = append_memory_recall_context(&mut system, None, &[memory], false, false);

    assert_eq!(recalled_chars, 0);
    assert!(
        !system.contains("request-only-big-memory"),
        "memory body must not be dumped into every request"
    );
}

use super::super::memory::append_memory_recall_context;

#[test]
fn memory_recall_context_does_not_dump_all_memories_regression() {
    let memory = jfc_memory::MemoryEntry {
        id: Some("test:big-memory".to_owned()),
        path: Some(std::path::PathBuf::from("big-memory.md")),
        level: jfc_memory::MemoryLevel::Project,
        frontmatter: jfc_memory::MemoryFrontmatter::new(
            jfc_memory::MemoryType::Context,
            jfc_memory::MemoryScope::Team,
        ),
        body: format!("request-only-big-memory {}", "x".repeat(16_000)),
    };
    let mut system = String::from("base prompt");

    let stats = append_memory_recall_context(&mut system, None, &[memory], false, false);

    assert_eq!(stats.prompt_chars, 0);
    assert_eq!(stats.fresh_recall_chars, 0);
    assert!(
        !system.contains("request-only-big-memory"),
        "memory body must not be dumped into every request"
    );
}

#[test]
fn cached_memory_recall_counts_prompt_context_without_duplicate_toast_regression() {
    let mut system = String::from("base prompt");
    let block = "\n\n## Recalled memory\ncached fact".to_owned();

    let stats = append_memory_recall_context(&mut system, Some(&block), &[], true, false);

    assert_eq!(stats.prompt_chars, block.len());
    assert_eq!(stats.fresh_recall_chars, 0);
    assert!(system.ends_with(&block));
}

//! E2E integration test proving the self-learning loop works across 3 simulated sessions.
//!
//! All LLM calls stubbed with a deterministic mock provider.

use std::collections::HashSet;
use std::fs;

use jfc_learn::LearnError;
use jfc_learn::auto_hints::run_pre_turn_hint_with;
use jfc_learn::dreamer::{Dreamer, DreamerTask, MemoryRecord};
use jfc_learn::historian::{
    CandidateFact, Historian, HistorianConfig, HistorianProvider, MemoryLookup,
};
use jfc_learn::key_files::{KeyFileStore, ReadEvent};
use jfc_learn::normalize_hash::normalize_and_hash;
use jfc_learn::user_memory::{UserMemoryPipeline, UserObservation};
use jfc_learn::verifier::{LlmVerifier, PromotionVerifier, VerifierVerdict};

use tempfile::TempDir;

// ─── Mock Provider ──────────────────────────────────────────────────────────

/// A deterministic mock provider that returns canned facts mimicking a session
/// where the user has a distinct communication style and references types.rs.
struct MockProvider;

impl HistorianProvider for MockProvider {
    fn extract_facts(&self, _system: &str, _user: &str) -> Result<String, LearnError> {
        // Each session returns the same consistent facts (simulating stable behavior).
        Ok(serde_json::json!({
            "facts": [
                {
                    "category": "USER_PREFERENCES",
                    "content": "User prefers concise, no-preamble responses",
                    "turn_ordinal": 2,
                    "confidence": 0.95
                },
                {
                    "category": "USER_DIRECTIVES",
                    "content": "Use traits for polymorphism, not free functions",
                    "turn_ordinal": 4,
                    "confidence": 0.9
                },
                {
                    "category": "ARCHITECTURE_DECISIONS",
                    "content": "jfc-graph types live in crates/jfc-graph/src/types.rs",
                    "turn_ordinal": 3,
                    "confidence": 1.0
                }
            ]
        })
        .to_string())
    }
}

/// A confirming LLM verifier that always says "yes".
struct AlwaysConfirmLlm;

impl LlmVerifier for AlwaysConfirmLlm {
    fn verify_promotion(&self, _fact: &CandidateFact) -> Result<VerifierVerdict, LearnError> {
        Ok(VerifierVerdict::Confirm {
            rationale: "confirmed by mock".into(),
        })
    }
}

// ─── In-memory store tracking seen_count ────────────────────────────────────

struct InMemoryStore {
    hashes: HashSet<String>,
    seen_counts: std::collections::HashMap<String, u32>,
}

impl InMemoryStore {
    fn new() -> Self {
        Self {
            hashes: HashSet::new(),
            seen_counts: std::collections::HashMap::new(),
        }
    }

    fn insert(&mut self, hash: &str) {
        self.hashes.insert(hash.to_string());
        *self.seen_counts.entry(hash.to_string()).or_insert(0) += 1;
    }

    fn bump(&mut self, hash: &str) {
        *self.seen_counts.entry(hash.to_string()).or_insert(0) += 1;
    }

    fn seen_count(&self, hash: &str) -> u32 {
        self.seen_counts.get(hash).copied().unwrap_or(0)
    }
}

impl MemoryLookup for InMemoryStore {
    fn hash_exists(&self, hash: &str) -> bool {
        self.hashes.contains(hash)
    }
}

// ─── Transcript fixtures ────────────────────────────────────────────────────

fn make_transcript() -> Vec<(String, String)> {
    vec![
        (
            "user".into(),
            "look at crates/jfc-graph/src/types.rs ig and tell me what's there".into(),
        ),
        (
            "assistant".into(),
            "The types.rs file contains the core graph node types: FunctionNode, StructNode, EnumNode, TraitNode, and ModuleNode, each with location spans and visibility info.".into(),
        ),
        (
            "user".into(),
            "ok ig can you refactor the polymorphism there to use a trait instead of free fns".into(),
        ),
        (
            "assistant".into(),
            "I'll refactor the free functions into a `GraphNode` trait with blanket implementations for each node type.".into(),
        ),
        (
            "user".into(),
            "no do it this way using the trait not a free fn — I want impl blocks per type".into(),
        ),
        (
            "assistant".into(),
            "Done. Each node type now has its own `impl GraphNode for FooNode` block with the relevant methods.".into(),
        ),
        (
            "user".into(),
            "also check crates/jfc-graph/src/types.rs again for any leftover free fns".into(),
        ),
        (
            "assistant".into(),
            "Checked — all free functions have been moved into trait impls. types.rs is clean.".into(),
        ),
        (
            "user".into(),
            "perfect ig thanks. one more look at crates/jfc-graph/src/types.rs for the Display impls"
                .into(),
        ),
        (
            "assistant".into(),
            "Display impls are in place for all five node types, using the qualified name format.".into(),
        ),
    ]
}

// ─── The actual E2E test ────────────────────────────────────────────────────

#[test]
fn learn_e2e_three_session_lifecycle() {
    let tmp = TempDir::new().unwrap();
    let project_root = tmp.path();

    // Create the target file so key-file pinning can hash it.
    let types_path = project_root.join("crates/jfc-graph/src/types.rs");
    fs::create_dir_all(types_path.parent().unwrap()).unwrap();
    fs::write(
        &types_path,
        "pub struct FunctionNode { pub name: String }\n",
    )
    .unwrap();

    let store = std::cell::RefCell::new(InMemoryStore::new());
    let verifier = PromotionVerifier::with_default_contracts();
    let llm = AlwaysConfirmLlm;
    let transcript = make_transcript();

    let pref_hash = normalize_and_hash("User prefers concise, no-preamble responses");

    // We use a simple wrapper that delegates to the RefCell-guarded store.
    struct StoreRef<'a>(&'a std::cell::RefCell<InMemoryStore>);
    impl MemoryLookup for StoreRef<'_> {
        fn hash_exists(&self, hash: &str) -> bool {
            self.0.borrow().hashes.contains(hash)
        }
    }

    // ─── Session 1 ──────────────────────────────────────────────────────

    {
        let provider = MockProvider;
        let historian = Historian::new(provider, StoreRef(&store), HistorianConfig::default());

        let quarantine_path = project_root.join(".jfc/quarantine.jsonl");
        let report = historian
            .process_session_with_verifier(&transcript, &verifier, &llm, &quarantine_path)
            .unwrap();

        // At least 1 USER_PREFERENCES fact
        assert!(
            report
                .processed
                .iter()
                .any(|p| p.fact.category == "USER_PREFERENCES"),
            "session 1: expected USER_PREFERENCES fact"
        );
        assert!(report.facts_promoted >= 1);
        assert_eq!(report.facts_quarantined, 0);

        // Persist to in-memory store
        for pf in &report.processed {
            if !pf.deduped {
                store.borrow_mut().insert(&pf.normalized_hash);
            }
        }

        // Record user observation
        let pipeline = UserMemoryPipeline::new(project_root);
        pipeline
            .record_observation(&UserObservation {
                facet: "communication_style".into(),
                observation: "Uses 'ig' filler, prefers blunt direct answers".into(),
                evidence_turns: vec![0, 2, 4, 8],
                session_id: "session-001".into(),
                observed_at: 1700000000,
            })
            .unwrap();

        // Record file reads (types.rs read 3 times in the transcript)
        let key_store = KeyFileStore::open(project_root).unwrap();
        for i in 0..3 {
            key_store
                .record_read(&ReadEvent {
                    file_path: "crates/jfc-graph/src/types.rs".into(),
                    session_id: "session-001".into(),
                    read_at_ms: 1700000000 + i * 1000,
                })
                .unwrap();
        }

        // Verify reads tracked
        let reads = key_store.load_read_history().unwrap();
        assert_eq!(
            reads
                .iter()
                .filter(|r| r.file_path == "crates/jfc-graph/src/types.rs")
                .count(),
            3
        );
    }

    assert_eq!(store.borrow().seen_count(&pref_hash), 1);

    // ─── Session 2 ──────────────────────────────────────────────────────

    {
        let provider = MockProvider;
        let historian = Historian::new(provider, StoreRef(&store), HistorianConfig::default());

        let report = historian.process_session(&transcript).unwrap();

        // The same facts exist — should be deduped
        assert!(report.facts_deduped >= 1, "session 2: expected dedup hits");

        // Bump seen_count for deduped facts
        for pf in &report.processed {
            if pf.deduped {
                store.borrow_mut().bump(&pf.normalized_hash);
            } else {
                store.borrow_mut().insert(&pf.normalized_hash);
            }
        }

        // Another user observation
        let pipeline = UserMemoryPipeline::new(project_root);
        pipeline
            .record_observation(&UserObservation {
                facet: "communication_style".into(),
                observation: "Uses 'ig' filler, prefers blunt direct answers".into(),
                evidence_turns: vec![0, 2, 8],
                session_id: "session-002".into(),
                observed_at: 1700001000,
            })
            .unwrap();

        // More reads from session 2
        let key_store = KeyFileStore::open(project_root).unwrap();
        for i in 0..2 {
            key_store
                .record_read(&ReadEvent {
                    file_path: "crates/jfc-graph/src/types.rs".into(),
                    session_id: "session-002".into(),
                    read_at_ms: 1700001000 + i * 1000,
                })
                .unwrap();
        }
    }

    assert_eq!(store.borrow().seen_count(&pref_hash), 2);

    // ─── Session 3 ──────────────────────────────────────────────────────

    {
        let provider = MockProvider;
        let historian = Historian::new(provider, StoreRef(&store), HistorianConfig::default());

        let report = historian.process_session(&transcript).unwrap();
        assert!(report.facts_deduped >= 1, "session 3: expected dedup hits");

        for pf in &report.processed {
            if pf.deduped {
                store.borrow_mut().bump(&pf.normalized_hash);
            } else {
                store.borrow_mut().insert(&pf.normalized_hash);
            }
        }

        // Third observation — should trigger promotion threshold (≥3 sessions)
        let pipeline = UserMemoryPipeline::new(project_root);
        pipeline
            .record_observation(&UserObservation {
                facet: "communication_style".into(),
                observation: "Uses 'ig' filler, prefers blunt direct answers".into(),
                evidence_turns: vec![0, 4],
                session_id: "session-003".into(),
                observed_at: 1700002000,
            })
            .unwrap();

        // More reads from session 3
        let key_store = KeyFileStore::open(project_root).unwrap();
        key_store
            .record_read(&ReadEvent {
                file_path: "crates/jfc-graph/src/types.rs".into(),
                session_id: "session-003".into(),
                read_at_ms: 1700002000,
            })
            .unwrap();

        // ─── Check user profile promotion ───────────────────────────────
        let candidates = UserMemoryPipeline::load_candidates(project_root).unwrap();
        assert!(
            candidates.len() >= 3,
            "expected ≥3 candidates across sessions"
        );

        let promoted = UserMemoryPipeline::check_promotion(&candidates);
        assert!(
            promoted.iter().any(|p| p.facet == "communication_style"),
            "communication_style should be promoted after 3 sessions"
        );

        // ─── Key file pinning ───────────────────────────────────────────
        let reads = key_store.load_read_history().unwrap();
        let pin_candidates = KeyFileStore::identify_candidates(&reads, 3);
        assert!(
            pin_candidates.contains(&"crates/jfc-graph/src/types.rs".to_string()),
            "types.rs should be a pin candidate (read in ≥3 sessions)"
        );

        // Pin it
        key_store
            .pin(
                types_path.to_str().unwrap(),
                "frequently accessed across sessions",
            )
            .unwrap();

        let pinned = key_store.list_pinned().unwrap();
        assert_eq!(pinned.len(), 1);
        assert!(pinned[0].file_path.contains("types.rs"));

        // ─── Dreamer cycle ──────────────────────────────────────────────
        let lease_path = project_root.join(".jfc/dreamer.lock");
        let dreamer = Dreamer::new(lease_path).with_project_root(project_root.to_path_buf());

        let mut memories: Vec<MemoryRecord> = store
            .borrow()
            .hashes
            .iter()
            .enumerate()
            .map(|(i, hash)| MemoryRecord {
                path: format!("mem-{}.md", i),
                category: Some("USER_PREFERENCES".into()),
                normalized_hash: Some(hash.clone()),
                content: format!("fact-{}", i),
                last_seen_at: Some(
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap()
                        .as_millis() as u64,
                ),
                memory_status: Some("active".into()),
            })
            .collect();

        let tasks = vec![
            DreamerTask::Consolidate,
            DreamerTask::Verify,
            DreamerTask::ArchiveStale,
            DreamerTask::Improve,
            DreamerTask::MaintainDocs,
        ];
        let report = dreamer.run_cycle(&tasks, &mut memories).unwrap();
        assert!(!report.circuit_breaker_fired);
        assert_eq!(report.tasks_run.len(), 5);

        // MaintainDocs should have written ARCHITECTURE.md
        let arch_path = project_root.join(".jfc/ARCHITECTURE.md");
        assert!(
            arch_path.exists(),
            "ARCHITECTURE.md should have been written by dreamer"
        );
        let arch_content = fs::read_to_string(&arch_path).unwrap();
        assert!(arch_content.contains("# Architecture Overview"));

        // Consolidate found no dupes (all hashes are unique across categories in our store)
        let consolidate_result = &report.tasks_run[0];
        // Our test store has unique hashes — consolidate should find 0 dupes
        // (it only dedupes same category + same hash)
        assert_eq!(consolidate_result.actions_taken, 0);
    }

    assert_eq!(store.borrow().seen_count(&pref_hash), 3);

    // ─── Session 4: Verify recall works ─────────────────────────────────

    {
        // Render user profile block
        let candidates = UserMemoryPipeline::load_candidates(project_root).unwrap();
        let promoted = UserMemoryPipeline::check_promotion(&candidates);
        let profile_block = UserMemoryPipeline::render_profile_block(&promoted);

        assert!(
            profile_block.contains("<user-profile>"),
            "should render user-profile block"
        );
        assert!(
            profile_block.contains("communication_style"),
            "profile should contain communication_style facet"
        );

        // Render key-files block
        let key_store = KeyFileStore::open(project_root).unwrap();
        let pinned = key_store.list_pinned().unwrap();
        let key_files_block = KeyFileStore::render_key_files_block(&pinned, 2000);

        assert!(
            key_files_block.contains("<key-files>"),
            "should render key-files block"
        );
        assert!(
            key_files_block.contains("types.rs"),
            "key-files should mention types.rs"
        );
        assert!(
            key_files_block.contains("FunctionNode"),
            "key-files should include the file content"
        );

        // Auto-search hint: write a memory file that mentions types.rs, then query
        let mem_dir = project_root.join(".jfc/memory");
        fs::create_dir_all(&mem_dir).unwrap();
        fs::write(
            mem_dir.join("arch-types.md"),
            "---\ntype: project\nscope: team\n---\n\
             Core graph node types defined in crates/jfc-graph/src/types.rs include \
             FunctionNode and StructNode.",
        )
        .unwrap();

        let hint = run_pre_turn_hint_with(
            "what's in crates/jfc-graph/src/types.rs",
            project_root,
            0.3,
            5,
        );
        assert!(hint.is_some(), "should produce a recall hint for types.rs");
        let hint_text = hint.unwrap();
        assert!(
            hint_text.contains("<!-- recall:"),
            "hint should be in recall comment format"
        );
        assert!(
            hint_text.contains("types.rs") || hint_text.contains("FunctionNode"),
            "hint should reference the file or its content"
        );
    }
}

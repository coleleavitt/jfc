use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

const ENGINE_ROOT: &str = "crates/jfc-engine/src";
const ENGINE_ROOT_FILE_POLICY: &str = "Engine Root Module Freeze";
const SYNTHETIC_FORBIDDEN_ROOT_FILES: &[&str] = &[
    "crates/jfc-engine/src/new_feature.rs",
    "crates/jfc-engine/src/context_pipeline.rs",
    "crates/jfc-engine/src/session_store.rs",
];
const ENGINE_ROOT_FILE_ALLOWLIST: &[&str] = &[
    "crates/jfc-engine/src/access_policy.rs",
    "crates/jfc-engine/src/advisor.rs",
    "crates/jfc-engine/src/agentic_vocabulary.rs",
    "crates/jfc-engine/src/atomic_write.rs",
    "crates/jfc-engine/src/attachments.rs",
    "crates/jfc-engine/src/auto_classifier.rs",
    "crates/jfc-engine/src/auto_mode.rs",
    "crates/jfc-engine/src/auto_review.rs",
    "crates/jfc-engine/src/autonomous_loop.rs",
    "crates/jfc-engine/src/bash_processes.rs",
    "crates/jfc-engine/src/bridge_attestation.rs",
    "crates/jfc-engine/src/cache_lineage.rs",
    "crates/jfc-engine/src/ccr.rs",
    "crates/jfc-engine/src/changeset.rs",
    "crates/jfc-engine/src/claude_status.rs",
    "crates/jfc-engine/src/coach.rs",
    "crates/jfc-engine/src/command_spec.rs",
    "crates/jfc-engine/src/compact_archive.rs",
    "crates/jfc-engine/src/config.rs",
    "crates/jfc-engine/src/context.rs",
    "crates/jfc-engine/src/context_accounting.rs",
    "crates/jfc-engine/src/cost.rs",
    "crates/jfc-engine/src/council.rs",
    "crates/jfc-engine/src/council_directives.rs",
    "crates/jfc-engine/src/council_session.rs",
    "crates/jfc-engine/src/daemon_services.rs",
    "crates/jfc-engine/src/diagnostics.rs",
    "crates/jfc-engine/src/diagnostics_producer.rs",
    "crates/jfc-engine/src/document_formats.rs",
    "crates/jfc-engine/src/dreamer_scheduler.rs",
    "crates/jfc-engine/src/effort.rs",
    "crates/jfc-engine/src/engine.rs",
    "crates/jfc-engine/src/env_context.rs",
    "crates/jfc-engine/src/exploration.rs",
    "crates/jfc-engine/src/feature_gates.rs",
    "crates/jfc-engine/src/file_checkpoint.rs",
    "crates/jfc-engine/src/git_context.rs",
    "crates/jfc-engine/src/goal.rs",
    "crates/jfc-engine/src/guards.rs",
    "crates/jfc-engine/src/hashline.rs",
    "crates/jfc-engine/src/headless.rs",
    "crates/jfc-engine/src/idle_prefetch.rs",
    "crates/jfc-engine/src/ids.rs",
    "crates/jfc-engine/src/inline_tools.rs",
    "crates/jfc-engine/src/interaction_mode.rs",
    "crates/jfc-engine/src/keywords.rs",
    "crates/jfc-engine/src/learn_lifecycle.rs",
    "crates/jfc-engine/src/lib.rs",
    "crates/jfc-engine/src/lsp_client.rs",
    "crates/jfc-engine/src/lsp_rpc.rs",
    "crates/jfc-engine/src/managed_session.rs",
    "crates/jfc-engine/src/mcp_elicitation.rs",
    "crates/jfc-engine/src/memory.rs",
    "crates/jfc-engine/src/memory_recall.rs",
    "crates/jfc-engine/src/notifications.rs",
    "crates/jfc-engine/src/output_style.rs",
    "crates/jfc-engine/src/permissions.rs",
    "crates/jfc-engine/src/plan.rs",
    "crates/jfc-engine/src/plan_dreamer.rs",
    "crates/jfc-engine/src/plan_recall.rs",
    "crates/jfc-engine/src/prompt_context_cache.rs",
    "crates/jfc-engine/src/prompt_executor.rs",
    "crates/jfc-engine/src/proof_oracles.rs",
    "crates/jfc-engine/src/push_notifications.rs",
    "crates/jfc-engine/src/remote_host.rs",
    "crates/jfc-engine/src/research.rs",
    "crates/jfc-engine/src/response_processor.rs",
    "crates/jfc-engine/src/review.rs",
    "crates/jfc-engine/src/rust_lex.rs",
    "crates/jfc-engine/src/scaffold_detector.rs",
    "crates/jfc-engine/src/scheduler.rs",
    "crates/jfc-engine/src/sdk_bridge.rs",
    "crates/jfc-engine/src/session_naming.rs",
    "crates/jfc-engine/src/session_recap.rs",
    "crates/jfc-engine/src/slate.rs",
    "crates/jfc-engine/src/slop_guard.rs",
    "crates/jfc-engine/src/speculation.rs",
    "crates/jfc-engine/src/sprint.rs",
    "crates/jfc-engine/src/stream.rs",
    "crates/jfc-engine/src/system_reminder.rs",
    "crates/jfc-engine/src/team_onboarding.rs",
    "crates/jfc-engine/src/toast.rs",
    "crates/jfc-engine/src/total_tokens_reminder.rs",
    "crates/jfc-engine/src/ultraplan.rs",
    "crates/jfc-engine/src/web_cache.rs",
    "crates/jfc-engine/src/web_search.rs",
    "crates/jfc-engine/src/worktrees.rs",
];

#[test]
fn workspace_dependency_rules_engine_root_files_match_allowlist()
-> Result<(), Box<dyn std::error::Error>> {
    let root_files = current_engine_root_files()?;
    let forbidden_root_files = forbidden_engine_root_files(root_files.iter().map(String::as_str));

    print_engine_root_allowlist();
    println!("current root file count: {}", root_files.len());

    assert!(
        forbidden_root_files.is_empty(),
        "{ENGINE_ROOT_FILE_POLICY} rejected new root-level product-domain files: {forbidden_root_files:#?}"
    );

    Ok(())
}

#[test]
fn workspace_dependency_rules_engine_root_file_guard_rejects_synthetic_path() {
    let forbidden_root_files =
        forbidden_engine_root_files(SYNTHETIC_FORBIDDEN_ROOT_FILES.iter().copied());

    print_engine_root_allowlist();
    println!("synthetic rejected paths:");
    for path in SYNTHETIC_FORBIDDEN_ROOT_FILES {
        println!("- {path}");
    }

    assert_eq!(
        forbidden_root_files,
        SYNTHETIC_FORBIDDEN_ROOT_FILES
            .iter()
            .map(|path| (*path).to_owned())
            .collect::<Vec<_>>()
    );
}

fn current_engine_root_files() -> Result<Vec<String>, Box<dyn std::error::Error>> {
    let root = workspace_root();
    let engine_root = root.join(ENGINE_ROOT);
    let mut root_files = fs::read_dir(engine_root)?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().and_then(|extension| extension.to_str()) == Some("rs"))
        .filter_map(|path| workspace_relative_path(&root, &path))
        .collect::<Vec<_>>();
    root_files.sort();
    Ok(root_files)
}

fn forbidden_engine_root_files<'a>(root_files: impl IntoIterator<Item = &'a str>) -> Vec<String> {
    let allowlist = ENGINE_ROOT_FILE_ALLOWLIST
        .iter()
        .copied()
        .collect::<BTreeSet<_>>();

    root_files
        .into_iter()
        .filter(|path| is_engine_root_file(path))
        .filter(|path| !allowlist.contains(path))
        .map(str::to_owned)
        .collect()
}

fn is_engine_root_file(path: &str) -> bool {
    path.strip_prefix(ENGINE_ROOT)
        .and_then(|path| path.strip_prefix('/'))
        .is_some_and(|path| path.ends_with(".rs") && !path.contains('/'))
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")))
}

fn workspace_relative_path(root: &Path, path: &Path) -> Option<String> {
    path.strip_prefix(root)
        .ok()
        .and_then(Path::to_str)
        .map(str::to_owned)
}

fn print_engine_root_allowlist() {
    println!(
        "{ENGINE_ROOT_FILE_POLICY} allowlist ({} entries):",
        ENGINE_ROOT_FILE_ALLOWLIST.len()
    );
    for path in ENGINE_ROOT_FILE_ALLOWLIST {
        println!("- {path}");
    }
}

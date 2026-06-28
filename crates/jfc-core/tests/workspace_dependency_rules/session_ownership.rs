use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

const ENGINE_SESSION_ROOT: &str = "crates/jfc-engine/src/session";
const ENGINE_SESSION_POLICY: &str = "Engine Session Compatibility Shim Freeze";
const SYNTHETIC_FORBIDDEN_ENGINE_SESSION_FILES: &[&str] = &[
    "crates/jfc-engine/src/session/new_serializer.rs",
    "crates/jfc-engine/src/session/persistence_writer.rs",
    "crates/jfc-engine/src/session/entry_log_persistence.rs",
    "crates/jfc-engine/src/session/serialization/new_codec.rs",
];
const SYNTHETIC_ALLOWED_MALFORMED_ENGINE_SESSION_PATHS: &[&str] = &[
    "crates/jfc-engine/src/session",
    "crates/jfc-engine/src/session/",
    "crates/jfc-engine/src/session/new_serializer.txt",
    "crates/jfc-engine/src/session_notes/new_serializer.rs",
    "crates/jfc-engine/src/sessionish/persistence_writer.rs",
    "crates/jfc-engine/src/session_store.rs",
];
const ENGINE_SESSION_COMPATIBILITY_SHIM_ALLOWLIST: &[&str] = &[
    "crates/jfc-engine/src/session/compaction.rs",
    "crates/jfc-engine/src/session/core.rs",
    "crates/jfc-engine/src/session/deserialize.rs",
    "crates/jfc-engine/src/session/mod.rs",
    "crates/jfc-engine/src/session/serialization.rs",
    "crates/jfc-engine/src/session/serialization_tests.rs",
    "crates/jfc-engine/src/session/serialize.rs",
    "crates/jfc-engine/src/session/store.rs",
];

#[test]
fn workspace_dependency_rules_engine_session_files_match_compatibility_shim_allowlist()
-> Result<(), Box<dyn std::error::Error>> {
    let session_files = current_engine_session_files()?;
    let forbidden_session_files =
        forbidden_engine_session_files(session_files.iter().map(String::as_str));

    print_engine_session_allowlist();
    println!("current engine session file count: {}", session_files.len());

    assert!(
        forbidden_session_files.is_empty(),
        "{ENGINE_SESSION_POLICY} rejected new engine-owned session serialization/persistence files: {forbidden_session_files:#?}"
    );

    Ok(())
}

#[test]
fn workspace_dependency_rules_engine_session_guard_rejects_synthetic_paths() {
    let forbidden_session_files =
        forbidden_engine_session_files(SYNTHETIC_FORBIDDEN_ENGINE_SESSION_FILES.iter().copied());

    print_engine_session_allowlist();
    println!("synthetic rejected engine session paths:");
    for path in SYNTHETIC_FORBIDDEN_ENGINE_SESSION_FILES {
        println!("- {path}");
    }

    assert_eq!(
        forbidden_session_files,
        SYNTHETIC_FORBIDDEN_ENGINE_SESSION_FILES
            .iter()
            .map(|path| (*path).to_owned())
            .collect::<Vec<_>>()
    );
}

#[test]
fn workspace_dependency_rules_engine_session_guard_ignores_malformed_non_session_paths() {
    let forbidden_session_files = forbidden_engine_session_files(
        SYNTHETIC_ALLOWED_MALFORMED_ENGINE_SESSION_PATHS
            .iter()
            .copied(),
    );

    assert!(
        forbidden_session_files.is_empty(),
        "{ENGINE_SESSION_POLICY} rejected malformed or non-session paths: {forbidden_session_files:#?}"
    );
}

fn current_engine_session_files() -> Result<Vec<String>, Box<dyn std::error::Error>> {
    let root = workspace_root();
    let session_root = root.join(ENGINE_SESSION_ROOT);
    let mut session_files = Vec::new();
    collect_rust_files(&root, &session_root, &mut session_files)?;
    session_files.sort();
    Ok(session_files)
}

fn collect_rust_files(
    workspace_root: &Path,
    directory: &Path,
    rust_files: &mut Vec<String>,
) -> Result<(), Box<dyn std::error::Error>> {
    for entry in fs::read_dir(directory)? {
        let path = entry?.path();
        if path.is_dir() {
            collect_rust_files(workspace_root, &path, rust_files)?;
        } else if path.extension().and_then(|extension| extension.to_str()) == Some("rs")
            && let Some(path) = workspace_relative_path(workspace_root, &path)
        {
            rust_files.push(path);
        }
    }
    Ok(())
}

fn forbidden_engine_session_files<'a>(
    session_files: impl IntoIterator<Item = &'a str>,
) -> Vec<String> {
    let allowlist = ENGINE_SESSION_COMPATIBILITY_SHIM_ALLOWLIST
        .iter()
        .copied()
        .collect::<BTreeSet<_>>();

    session_files
        .into_iter()
        .filter(|path| is_engine_session_rust_file(path))
        .filter(|path| !allowlist.contains(path))
        .map(str::to_owned)
        .collect()
}

fn is_engine_session_rust_file(path: &str) -> bool {
    path.strip_prefix(ENGINE_SESSION_ROOT)
        .and_then(|path| path.strip_prefix('/'))
        .is_some_and(|path| path.ends_with(".rs"))
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

fn print_engine_session_allowlist() {
    println!(
        "{ENGINE_SESSION_POLICY} compatibility shim allowlist ({} entries):",
        ENGINE_SESSION_COMPATIBILITY_SHIM_ALLOWLIST.len()
    );
    for path in ENGINE_SESSION_COMPATIBILITY_SHIM_ALLOWLIST {
        println!("- {path}");
    }
}

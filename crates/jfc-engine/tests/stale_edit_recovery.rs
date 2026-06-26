use std::sync::Arc;

use jfc_engine::context::ReadDedupCache;
use jfc_engine::tools::execute_tool;
use jfc_engine::types::{ReplacementMode, ToolInput, ToolKind};
use tokio::sync::Mutex;

#[tokio::test]
async fn edit_requires_read_after_stale_old_string_miss() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("fmt.rs");
    tokio::fs::write(&path, "fn main(){println!(\"old\");}\n")
        .await
        .unwrap();

    let cache = Arc::new(Mutex::new(ReadDedupCache::new()));
    let file_path = path.to_string_lossy().to_string();

    let initial_read = execute_tool(
        ToolKind::Read,
        ToolInput::Read {
            file_path: file_path.clone(),
            offset: None,
            limit: None,
        },
        dir.path().to_path_buf(),
        Some(cache.clone()),
        None,
        None,
    )
    .await;
    assert!(!initial_read.is_error(), "{}", initial_read.output);

    tokio::fs::write(&path, "fn main() {\n    println!(\"old\");\n}\n")
        .await
        .unwrap();

    let miss = execute_tool(
        ToolKind::Edit,
        ToolInput::Edit {
            file_path: file_path.clone(),
            old_string: "fn main(){println!(\"old\");}\n".into(),
            new_string: "fn main(){println!(\"new\");}\n".into(),
            replacement: ReplacementMode::FirstOnly,
        },
        dir.path().to_path_buf(),
        Some(cache.clone()),
        None,
        None,
    )
    .await;
    assert!(miss.is_error(), "{}", miss.output);
    assert!(
        miss.output.contains("stale-read recovery"),
        "{}",
        miss.output
    );

    let blocked_retry = execute_tool(
        ToolKind::Edit,
        ToolInput::Edit {
            file_path: file_path.clone(),
            old_string: "    println!(\"old\");".into(),
            new_string: "    println!(\"new\");".into(),
            replacement: ReplacementMode::FirstOnly,
        },
        dir.path().to_path_buf(),
        Some(cache.clone()),
        None,
        None,
    )
    .await;
    assert!(blocked_retry.is_error(), "{}", blocked_retry.output);
    assert!(
        blocked_retry.output.contains("Run Read on this file first"),
        "{}",
        blocked_retry.output
    );

    let refresh = execute_tool(
        ToolKind::Read,
        ToolInput::Read {
            file_path: file_path.clone(),
            offset: None,
            limit: None,
        },
        dir.path().to_path_buf(),
        Some(cache.clone()),
        None,
        None,
    )
    .await;
    assert!(!refresh.is_error(), "{}", refresh.output);
    assert!(
        !refresh
            .output
            .contains("File unchanged since last full read"),
        "stale refresh must return current contents: {}",
        refresh.output
    );
    assert!(
        refresh.output.contains("println!(\"old\")"),
        "{}",
        refresh.output
    );

    let ok = execute_tool(
        ToolKind::Edit,
        ToolInput::Edit {
            file_path,
            old_string: "    println!(\"old\");".into(),
            new_string: "    println!(\"new\");".into(),
            replacement: ReplacementMode::FirstOnly,
        },
        dir.path().to_path_buf(),
        Some(cache),
        None,
        None,
    )
    .await;
    assert!(!ok.is_error(), "{}", ok.output);
    let after = tokio::fs::read_to_string(&path).await.unwrap();
    assert!(after.contains("println!(\"new\")"), "{after}");
}

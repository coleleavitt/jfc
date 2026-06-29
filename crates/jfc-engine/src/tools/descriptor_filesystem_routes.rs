use std::path::{Path, PathBuf};
use std::sync::Arc;

use tokio::sync::Mutex;

use crate::context::ReadDedupCache;
use crate::runtime::ExecutionResult;
use crate::types::ReplacementMode;

use super::filesystem::{
    acquire_file_lock, apply_one_edit, build_edit_diff_view, edit_error_needs_stale_recovery_hint,
    execute_edit, execute_read, execute_write, stale_edit_requires_read_message,
    stale_read_recovery_hint, validate_file_mutation,
};
use super::notebook::{execute_notebook_edit, execute_notebook_read};
use super::safe_tools::maybe_run_slop_guard;

pub(crate) async fn execute_read_route(
    file_path: &str,
    offset: Option<u64>,
    limit: Option<u64>,
    dedup: Option<&Arc<Mutex<ReadDedupCache>>>,
) -> ExecutionResult {
    execute_read(file_path, offset, limit, dedup).await
}

pub(crate) async fn execute_write_route(
    file_path: &str,
    content: &str,
    cwd: &Path,
    dedup: Option<&Arc<Mutex<ReadDedupCache>>>,
) -> ExecutionResult {
    let target_path = match crate::speculation::overlay_path_for(Path::new(file_path)) {
        Some(overlay) => {
            if let Some(parent) = overlay.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            overlay.display().to_string()
        }
        None => file_path.to_owned(),
    };
    let old_content = tokio::fs::read_to_string(&target_path).await.ok();
    checkpoint_before_mutation(Path::new(&target_path), "Write");
    let result = execute_write(&target_path, content).await;
    if result.is_error() {
        return result;
    }
    if let Some(cache) = dedup {
        cache.lock().await.invalidate(Path::new(file_path));
    }
    maybe_run_slop_guard(
        result,
        Path::new(file_path),
        content,
        old_content.as_deref(),
        cwd,
    )
    .await
}

pub(crate) async fn execute_edit_route(
    file_path: &str,
    old_string: &str,
    new_string: &str,
    replacement: ReplacementMode,
    cwd: &Path,
    dedup: Option<&Arc<Mutex<ReadDedupCache>>>,
) -> ExecutionResult {
    if let Some(cache) = dedup {
        let guard = cache.lock().await;
        if guard.requires_stale_edit_refresh(Path::new(file_path)) {
            return ExecutionResult::failure(stale_edit_requires_read_message(file_path));
        }
    }
    let old_content = tokio::fs::read_to_string(file_path).await.ok();
    checkpoint_before_mutation(Path::new(file_path), "Edit");
    let result = execute_edit(file_path, old_string, new_string, replacement).await;
    if !result.is_error() {
        if let Some(cache) = dedup {
            cache.lock().await.invalidate(Path::new(file_path));
        }
        let post_content = tokio::fs::read_to_string(file_path)
            .await
            .unwrap_or_default();
        return maybe_run_slop_guard(
            result,
            Path::new(file_path),
            &post_content,
            old_content.as_deref(),
            cwd,
        )
        .await;
    }
    if edit_error_needs_stale_recovery_hint(&result.output)
        && let Some(cache) = dedup
    {
        cache
            .lock()
            .await
            .mark_stale_edit_miss(PathBuf::from(file_path));
    }
    result
}

pub(crate) async fn execute_multi_edit_route(
    file_path: &str,
    edits: &serde_json::Value,
    cwd: &Path,
    dedup: Option<&Arc<Mutex<ReadDedupCache>>>,
) -> ExecutionResult {
    if let Some(cache) = dedup {
        let guard = cache.lock().await;
        if guard.requires_stale_edit_refresh(Path::new(file_path)) {
            return ExecutionResult::failure(stale_edit_requires_read_message(file_path));
        }
    }
    let lock = acquire_file_lock(file_path).await;
    let _guard = lock.lock().await;
    let path = PathBuf::from(file_path);
    let mut content = match tokio::fs::read_to_string(&path).await {
        Ok(content) => content,
        Err(error) => {
            return ExecutionResult::failure(format!(
                "MultiEdit: cannot read {file_path}: {error}"
            ));
        }
    };
    let old_content = content.clone();
    checkpoint_before_mutation(&path, "MultiEdit");
    let edit_array = match edits.as_array() {
        Some(edits) => edits,
        None => {
            return ExecutionResult::failure(
                "MultiEdit: `edits` must be an array of {old_string, new_string} objects",
            );
        }
    };
    let mut applied = 0usize;
    for (index, edit) in edit_array.iter().enumerate() {
        let old = edit
            .get("old_string")
            .and_then(|value| value.as_str())
            .unwrap_or("");
        let new = edit
            .get("new_string")
            .and_then(|value| value.as_str())
            .unwrap_or("");
        let replace_all = edit
            .get("replace_all")
            .and_then(|value| value.as_bool())
            .unwrap_or(false);
        let label = format!(
            "MultiEdit: edit {} of {} (earlier applied: {applied})",
            index + 1,
            edit_array.len()
        );
        match apply_one_edit(&content, old, new, replace_all, &label) {
            Ok(updated) => content = updated,
            Err(error) => {
                if edit_error_needs_stale_recovery_hint(&error) {
                    let hint = stale_read_recovery_hint(file_path, &content);
                    if let Some(cache) = dedup {
                        cache
                            .lock()
                            .await
                            .mark_stale_edit_miss(PathBuf::from(file_path));
                    }
                    return ExecutionResult::failure(format!("{error}\n{hint}"));
                }
                return ExecutionResult::failure(error);
            }
        }
        applied += 1;
    }
    if let Err(reason) =
        validate_file_mutation(file_path, Some(&old_content), &content, "MultiEdit")
    {
        return ExecutionResult::failure(reason);
    }
    if let Err(error) = tokio::fs::write(&path, &content).await {
        return ExecutionResult::failure(format!("MultiEdit: write {file_path}: {error}"));
    }
    if let Some(cache) = dedup {
        cache.lock().await.invalidate(Path::new(file_path));
    }
    tracing::info!(
        target: "jfc::tools::multi_edit",
        file_path = %file_path,
        applied,
        bytes = content.len(),
        "MultiEdit applied"
    );
    let diff = build_edit_diff_view(file_path, &old_content, &content);
    let result = ExecutionResult::success(format!("Applied {applied} edits to {file_path}."))
        .with_diff(diff);
    maybe_run_slop_guard(
        result,
        Path::new(file_path),
        &content,
        Some(&old_content),
        cwd,
    )
    .await
}

pub(crate) async fn execute_notebook_read_route(path: &str) -> ExecutionResult {
    execute_notebook_read(path).await
}

pub(crate) async fn execute_notebook_edit_route(
    path: &str,
    cell_id: &str,
    new_source: &str,
    edit_mode: Option<&str>,
) -> ExecutionResult {
    execute_notebook_edit(path, cell_id, new_source, edit_mode).await
}

fn checkpoint_before_mutation(path: &Path, tool: &str) {
    match crate::file_checkpoint::checkpoint_file(path) {
        Ok(backup) => {
            tracing::debug!(
                target: "jfc::file_checkpoint",
                tool,
                path = %path.display(),
                backup = %backup.display(),
                "created file checkpoint before mutation"
            );
        }
        Err(error) => {
            tracing::warn!(
                target: "jfc::file_checkpoint",
                tool,
                path = %path.display(),
                error = %error,
                "failed to create file checkpoint before mutation"
            );
        }
    }
}

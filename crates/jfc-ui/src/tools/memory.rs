use std::path::Path;

use super::ExecutionResult;

pub(super) fn execute_memory_create(
    level: &str,
    memory_type: &str,
    scope: &str,
    body: &str,
    project_root: &Path,
) -> ExecutionResult {
    use crate::memory;

    let mem_level = match level.to_lowercase().as_str() {
        "user" => memory::MemoryLevel::User,
        "project" => memory::MemoryLevel::Project,
        other => {
            return ExecutionResult::failure(format!(
                "Invalid level '{other}'. Use 'user' or 'project'."
            ));
        }
    };

    let mem_type = match memory_type.parse::<memory::MemoryType>() {
        Ok(t) => t,
        Err(e) => return ExecutionResult::failure(e),
    };

    let mem_scope = match scope.parse::<memory::MemoryScope>() {
        Ok(s) => s,
        Err(e) => return ExecutionResult::failure(e),
    };

    if body.trim().is_empty() {
        return ExecutionResult::failure("Memory body cannot be empty.");
    }

    match memory::create_memory(mem_level, mem_type, mem_scope, body.trim(), project_root) {
        Ok(path) => ExecutionResult::success(format!(
            "Memory saved to: {}\n\nThis memory will be included in future conversations.",
            path.display()
        )),
        Err(e) => ExecutionResult::failure(format!("Failed to create memory: {e}")),
    }
}

pub(super) fn execute_memory_delete(path_str: &str) -> ExecutionResult {
    use crate::memory;
    use std::path::PathBuf;

    let path = PathBuf::from(path_str);

    if !path.exists() {
        return ExecutionResult::failure(format!("File not found: {}", path.display()));
    }

    match memory::delete_memory(&path) {
        Ok(()) => ExecutionResult::success(format!(
            "Memory deleted: {}\n\nThis memory will no longer be included in future conversations.",
            path.display()
        )),
        Err(e) => ExecutionResult::failure(format!("Failed to delete memory: {e}")),
    }
}


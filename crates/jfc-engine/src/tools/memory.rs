use std::path::Path;

use super::ExecutionResult;

pub fn execute_memory_create(
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

    let body = body.trim().to_owned();
    let project_root = project_root.to_owned();
    match jfc_knowledge::block_on_knowledge(async move {
        memory::create_memory(mem_level, mem_type, mem_scope, &body, &project_root).await
    }) {
        Ok(id) => ExecutionResult::success(format!(
            "Memory saved with id: {id}\n\nThis memory will be included in future conversations."
        )),
        Err(e) => ExecutionResult::failure(format!("Failed to create memory: {e}")),
    }
}

/// Delete a memory by its DB id (post MD→DB cutover — memories are rows, not
/// files). Accepts the id the create/list surface reports.
pub fn execute_memory_delete(id: &str) -> ExecutionResult {
    use crate::memory;

    let id_for_format = id.trim().to_owned();
    let id_for_call = id_for_format.clone();
    match jfc_knowledge::block_on_knowledge(
        async move { memory::delete_memory(&id_for_call).await },
    ) {
        Ok(()) => ExecutionResult::success(format!(
            "Memory deleted: {id_for_format}\n\nThis memory will no longer be included in future conversations."
        )),
        Err(e) => ExecutionResult::failure(format!("Failed to delete memory: {e}")),
    }
}

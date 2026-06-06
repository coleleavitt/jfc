//! Notebook tool — thin wrapper delegating to `jfc_tools::notebook`.

use super::ExecutionResult;

pub(super) async fn execute_notebook_read(path_str: &str) -> ExecutionResult {
    jfc_tools::notebook::execute_notebook_read(path_str).await
}

pub(super) async fn execute_notebook_edit(
    path_str: &str,
    cell_id: &str,
    new_source: &str,
    edit_mode: Option<&str>,
) -> ExecutionResult {
    jfc_tools::notebook::execute_notebook_edit(path_str, cell_id, new_source, edit_mode).await
}

#[cfg(test)]
pub(crate) fn notebook_read_text(text: &str) -> Result<String, String> {
    jfc_tools::notebook::notebook_read_text(text)
}

#[cfg(test)]
pub(crate) fn notebook_edit_text(
    notebook_json: &str,
    cell_id: &str,
    new_source: &str,
    edit_mode: &str,
) -> Result<String, String> {
    jfc_tools::notebook::notebook_edit_text(notebook_json, cell_id, new_source, edit_mode)
}

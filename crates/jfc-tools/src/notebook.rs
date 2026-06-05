//! Notebook (`.ipynb`) read and edit operations.

use jfc_core::ExecutionResult;

/// Read and render a notebook file.
pub async fn execute_notebook_read(path_str: &str) -> ExecutionResult {
    let path = std::path::PathBuf::from(path_str);
    if !path.is_absolute() {
        return ExecutionResult::failure(format!(
            "notebook_read: path must be absolute (got '{path_str}')"
        ));
    }
    let text = match tokio::fs::read_to_string(&path).await {
        Ok(s) => s,
        Err(e) => {
            return ExecutionResult::failure(format!("notebook_read: cannot read {path_str}: {e}"));
        }
    };
    match notebook_read_text(&text) {
        Ok(rendered) => ExecutionResult::success(rendered),
        Err(e) => ExecutionResult::failure(format!("notebook_read: {e}")),
    }
}

/// Parse a notebook JSON document and emit a human-readable rendering.
pub fn notebook_read_text(text: &str) -> Result<String, String> {
    let v: serde_json::Value =
        serde_json::from_str(text).map_err(|e| format!("invalid notebook JSON: {e}"))?;
    let cells = v
        .get("cells")
        .and_then(|c| c.as_array())
        .ok_or_else(|| "notebook missing `cells` array".to_owned())?;
    let mut out = String::new();
    out.push_str(&format!("Notebook: {} cells\n", cells.len()));
    for (i, cell) in cells.iter().enumerate() {
        let id = cell
            .get("id")
            .and_then(|v| v.as_str())
            .map(str::to_owned)
            .unwrap_or_else(|| format!("cell-{i}"));
        let kind = cell
            .get("cell_type")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        let source = collect_cell_source(cell);
        out.push_str(&format!("\n--- [{i}] {kind} (id={id}) ---\n"));
        out.push_str(&source);
        if !source.ends_with('\n') {
            out.push('\n');
        }
        if kind == "code"
            && let Some(outputs) = cell.get("outputs").and_then(|o| o.as_array())
            && !outputs.is_empty()
        {
            out.push_str(&format!("--- outputs ({}) ---\n", outputs.len()));
            for (j, output) in outputs.iter().enumerate() {
                let text_block = collect_output_text(output);
                if text_block.is_empty() {
                    let kind = output
                        .get("output_type")
                        .and_then(|v| v.as_str())
                        .unwrap_or("?");
                    out.push_str(&format!("  [{j}] {kind} (binary or no text)\n"));
                } else {
                    out.push_str(&format!("  [{j}] {text_block}\n"));
                }
            }
        }
    }
    Ok(out)
}

fn collect_cell_source(cell: &serde_json::Value) -> String {
    match cell.get("source") {
        Some(serde_json::Value::String(s)) => s.clone(),
        Some(serde_json::Value::Array(arr)) => arr
            .iter()
            .filter_map(|v| v.as_str())
            .collect::<Vec<_>>()
            .join(""),
        _ => String::new(),
    }
}

fn collect_output_text(output: &serde_json::Value) -> String {
    if let Some(s) = output.get("text") {
        return collect_string_or_array(s);
    }
    if let Some(data) = output.get("data")
        && let Some(plain) = data.get("text/plain")
    {
        return collect_string_or_array(plain);
    }
    if let Some(name) = output.get("evalue").and_then(|v| v.as_str()) {
        return name.to_owned();
    }
    String::new()
}

fn collect_string_or_array(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Array(arr) => arr
            .iter()
            .filter_map(|v| v.as_str())
            .collect::<Vec<_>>()
            .join(""),
        _ => v.to_string(),
    }
}

/// Edit a notebook cell by ID.
pub async fn execute_notebook_edit(
    path_str: &str,
    cell_id: &str,
    new_source: &str,
    edit_mode: Option<&str>,
) -> ExecutionResult {
    let path = std::path::PathBuf::from(path_str);
    if !path.is_absolute() {
        return ExecutionResult::failure(format!(
            "notebook_edit: path must be absolute (got '{path_str}')"
        ));
    }
    let text = match tokio::fs::read_to_string(&path).await {
        Ok(s) => s,
        Err(e) => {
            return ExecutionResult::failure(format!("notebook_edit: cannot read {path_str}: {e}"));
        }
    };
    let mode = edit_mode.unwrap_or("replace");
    match notebook_edit_text(&text, cell_id, new_source, mode) {
        Ok(new_text) => match tokio::fs::write(&path, &new_text).await {
            Ok(_) => ExecutionResult::success(format!(
                "notebook_edit: {mode} on {path_str}#{cell_id} ({} bytes written)",
                new_text.len()
            )),
            Err(e) => {
                ExecutionResult::failure(format!("notebook_edit: write to {path_str} failed: {e}"))
            }
        },
        Err(e) => ExecutionResult::failure(format!("notebook_edit: {e}")),
    }
}

/// Pure helper: returns the modified notebook JSON.
pub fn notebook_edit_text(
    notebook_json: &str,
    cell_id: &str,
    new_source: &str,
    edit_mode: &str,
) -> Result<String, String> {
    if !matches!(edit_mode, "replace" | "insert" | "delete") {
        return Err(format!(
            "invalid edit_mode '{edit_mode}'. Must be one of: replace | insert | delete"
        ));
    }
    let mut v: serde_json::Value =
        serde_json::from_str(notebook_json).map_err(|e| format!("invalid notebook JSON: {e}"))?;
    let cells = v
        .get_mut("cells")
        .and_then(|c| c.as_array_mut())
        .ok_or_else(|| "notebook missing `cells` array".to_owned())?;

    let idx = cells
        .iter()
        .position(|c| {
            c.get("id")
                .and_then(|v| v.as_str())
                .map(|s| s == cell_id)
                .unwrap_or(false)
        })
        .ok_or_else(|| format!("cell with id '{cell_id}' not found"))?;

    match edit_mode {
        "replace" => {
            if let Some(obj) = cells[idx].as_object_mut() {
                obj.insert(
                    "source".into(),
                    serde_json::Value::String(new_source.to_owned()),
                );
                if obj.contains_key("outputs") {
                    obj.insert("outputs".into(), serde_json::Value::Array(Vec::new()));
                }
                if obj.contains_key("execution_count") {
                    obj.insert("execution_count".into(), serde_json::Value::Null);
                }
            }
        }
        "insert" => {
            let new_id = format!("{cell_id}-new-{}", uuid::Uuid::new_v4().simple());
            let new_cell = serde_json::json!({
                "cell_type": "code",
                "id": new_id,
                "metadata": {},
                "source": new_source,
                "outputs": [],
                "execution_count": null,
            });
            cells.insert(idx + 1, new_cell);
        }
        "delete" => {
            cells.remove(idx);
        }
        _ => unreachable!(),
    }

    serde_json::to_string_pretty(&v).map_err(|e| format!("re-serialize failed: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notebook_read_text_basic() {
        let nb = r#"{"cells": [{"cell_type": "code", "id": "abc", "source": "print(1)", "outputs": []}]}"#;
        let out = notebook_read_text(nb).unwrap();
        assert!(out.contains("abc"));
        assert!(out.contains("print(1)"));
    }

    #[test]
    fn notebook_edit_replace() {
        let nb = r#"{"cells": [{"cell_type": "code", "id": "c1", "source": "old", "outputs": [], "execution_count": 5}]}"#;
        let result = notebook_edit_text(nb, "c1", "new code", "replace").unwrap();
        let v: serde_json::Value = serde_json::from_str(&result).unwrap();
        let src = v["cells"][0]["source"].as_str().unwrap();
        assert_eq!(src, "new code");
        assert!(v["cells"][0]["execution_count"].is_null());
    }

    #[test]
    fn notebook_edit_insert() {
        let nb = r#"{"cells": [{"cell_type": "code", "id": "c1", "source": "x", "outputs": []}]}"#;
        let result = notebook_edit_text(nb, "c1", "inserted", "insert").unwrap();
        let v: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert_eq!(v["cells"].as_array().unwrap().len(), 2);
        assert_eq!(v["cells"][1]["source"].as_str().unwrap(), "inserted");
    }

    #[test]
    fn notebook_edit_delete() {
        let nb = r#"{"cells": [{"cell_type": "code", "id": "c1", "source": "x", "outputs": []}, {"cell_type": "code", "id": "c2", "source": "y", "outputs": []}]}"#;
        let result = notebook_edit_text(nb, "c1", "", "delete").unwrap();
        let v: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert_eq!(v["cells"].as_array().unwrap().len(), 1);
        assert_eq!(v["cells"][0]["id"].as_str().unwrap(), "c2");
    }

    #[test]
    fn notebook_edit_invalid_mode() {
        let nb = r#"{"cells": [{"cell_type": "code", "id": "c1", "source": "x", "outputs": []}]}"#;
        let err = notebook_edit_text(nb, "c1", "", "bad").unwrap_err();
        assert!(err.contains("invalid edit_mode"));
    }
}

//! Pure filesystem tool operations: read, write, edit.
//!
//! These are the core file manipulation primitives that don't depend on
//! app state (no undo stack, no dedup cache, no slop guard). The jfc-ui
//! dispatch layer wraps these with caching, permissions, and side-effects.

use std::path::Path;

use jfc_core::ExecutionResult;

/// Read a file with optional line offset and limit.
pub async fn read_file(
    file_path: &str,
    offset: Option<u64>,
    limit: Option<u64>,
) -> ExecutionResult {
    let path = Path::new(file_path);
    if !path.is_absolute() {
        return ExecutionResult::failure(format!(
            "read: path must be absolute (got '{file_path}')"
        ));
    }
    let content = match tokio::fs::read_to_string(path).await {
        Ok(s) => s,
        Err(e) => {
            return ExecutionResult::failure(format!("read: cannot read {file_path}: {e}"));
        }
    };

    let lines: Vec<&str> = content.lines().collect();
    let total = lines.len();
    let start = offset.unwrap_or(1).max(1) as usize - 1; // 1-indexed
    let count = limit.unwrap_or(2000) as usize;

    if start >= total {
        return ExecutionResult::success(format!(
            "(file has {total} lines, offset {start_1} is past end)",
            start_1 = start + 1
        ));
    }

    let end = (start + count).min(total);
    let mut out = String::new();
    for (i, line) in lines[start..end].iter().enumerate() {
        let line_num = start + i + 1;
        out.push_str(&format!("{line_num}: {line}\n"));
    }
    if end < total {
        out.push_str(&format!(
            "\n(... {remaining} more lines)\n",
            remaining = total - end
        ));
    }
    ExecutionResult::success(out)
}

/// Write content to a file, creating parent directories as needed.
pub async fn write_file(file_path: &str, content: &str) -> ExecutionResult {
    let path = Path::new(file_path);
    if !path.is_absolute() {
        return ExecutionResult::failure(format!(
            "write: path must be absolute (got '{file_path}')"
        ));
    }
    if let Some(parent) = path.parent()
        && let Err(e) = tokio::fs::create_dir_all(parent).await
    {
        return ExecutionResult::failure(format!(
            "write: cannot create parent dirs for {file_path}: {e}"
        ));
    }
    match tokio::fs::write(path, content).await {
        Ok(_) => ExecutionResult::success(format!(
            "Successfully wrote {} bytes to {file_path}",
            content.len()
        )),
        Err(e) => ExecutionResult::failure(format!("write: cannot write {file_path}: {e}")),
    }
}

/// Perform a string replacement edit on a file.
pub async fn edit_file(
    file_path: &str,
    old_string: &str,
    new_string: &str,
    replace_all: bool,
) -> ExecutionResult {
    let path = Path::new(file_path);
    if !path.is_absolute() {
        return ExecutionResult::failure(format!(
            "edit: path must be absolute (got '{file_path}')"
        ));
    }
    let content = match tokio::fs::read_to_string(path).await {
        Ok(s) => s,
        Err(e) => {
            return ExecutionResult::failure(format!("edit: cannot read {file_path}: {e}"));
        }
    };

    if old_string.is_empty() {
        return ExecutionResult::failure("edit: old_string must not be empty".to_string());
    }

    let count = content.matches(old_string).count();
    if count == 0 {
        return ExecutionResult::failure(format!(
            "edit: old_string not found in {file_path}. Make sure it matches exactly (including whitespace)."
        ));
    }
    if count > 1 && !replace_all {
        return ExecutionResult::failure(format!(
            "edit: old_string appears {count} times in {file_path}. Use replace_all=true for multiple replacements, or provide a more specific match."
        ));
    }

    let new_content = if replace_all {
        content.replace(old_string, new_string)
    } else {
        content.replacen(old_string, new_string, 1)
    };

    match tokio::fs::write(path, &new_content).await {
        Ok(_) => ExecutionResult::success(format!(
            "Successfully edited {file_path} ({count} replacement(s))"
        )),
        Err(e) => ExecutionResult::failure(format!("edit: cannot write {file_path}: {e}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn write_and_read_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.txt");
        let path_str = path.to_str().unwrap();

        let result = write_file(path_str, "line1\nline2\nline3\n").await;
        assert!(!result.is_error());

        let result = read_file(path_str, None, None).await;
        assert!(!result.is_error());
        assert!(result.output.contains("1: line1"));
        assert!(result.output.contains("2: line2"));
    }

    #[tokio::test]
    async fn edit_replaces_text() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("edit.txt");
        let path_str = path.to_str().unwrap();

        write_file(path_str, "hello world").await;
        let result = edit_file(path_str, "world", "rust", false).await;
        assert!(!result.is_error());

        let content = tokio::fs::read_to_string(&path).await.unwrap();
        assert_eq!(content, "hello rust");
    }

    #[tokio::test]
    async fn edit_rejects_ambiguous_match() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("dup.txt");
        let path_str = path.to_str().unwrap();

        write_file(path_str, "foo bar foo baz").await;
        let result = edit_file(path_str, "foo", "qux", false).await;
        assert!(result.is_error());
        assert!(result.output.contains("2 times"));
    }

    #[tokio::test]
    async fn edit_replace_all_works() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("all.txt");
        let path_str = path.to_str().unwrap();

        write_file(path_str, "aaa bbb aaa").await;
        let result = edit_file(path_str, "aaa", "ccc", true).await;
        assert!(!result.is_error());

        let content = tokio::fs::read_to_string(&path).await.unwrap();
        assert_eq!(content, "ccc bbb ccc");
    }

    #[tokio::test]
    async fn read_with_offset_and_limit() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("lines.txt");
        let path_str = path.to_str().unwrap();

        let content = (1..=10)
            .map(|i| format!("line {i}"))
            .collect::<Vec<_>>()
            .join("\n");
        write_file(path_str, &content).await;

        let result = read_file(path_str, Some(3), Some(2)).await;
        assert!(!result.is_error());
        assert!(result.output.contains("3: line 3"));
        assert!(result.output.contains("4: line 4"));
        assert!(!result.output.contains("5: line 5"));
    }
}

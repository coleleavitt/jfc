//! Pure filesystem tool operations: read, write, edit.
//!
//! These are the core file manipulation primitives that don't depend on
//! app state (no undo stack, no dedup cache, no slop guard). The jfc
//! dispatch layer wraps these with caching, permissions, and side-effects.

use std::path::Path;

use jfc_core::ExecutionResult;

/// Read a file with optional line offset and limit.
pub async fn read_file(
    file_path: &str,
    offset: Option<u64>,
    limit: Option<u64>,
) -> ExecutionResult {
    let _linkscope_read = linkscope::phase("tools.filesystem.read");
    let path = Path::new(file_path);
    if !path.is_absolute() {
        linkscope::record_items("tools.filesystem.read.relative_path", 1);
        return ExecutionResult::failure(format!(
            "read: path must be absolute (got '{file_path}')"
        ));
    }
    let content = match tokio::fs::read_to_string(path).await {
        Ok(s) => s,
        Err(e) => {
            linkscope::record_items("tools.filesystem.read.error", 1);
            return ExecutionResult::failure(format!("read: cannot read {file_path}: {e}"));
        }
    };
    linkscope::record_bytes(
        "tools.filesystem.read.bytes",
        usize_to_u64_saturating(content.len()),
    );

    let lines: Vec<&str> = content.lines().collect();
    let total = lines.len();
    let start = offset.unwrap_or(1).max(1) as usize - 1; // 1-indexed
    let count = limit.unwrap_or(2000) as usize;

    if start >= total {
        linkscope::record_items("tools.filesystem.read.past_end", 1);
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
    let _linkscope_write = linkscope::phase("tools.filesystem.write");
    let path = Path::new(file_path);
    if !path.is_absolute() {
        linkscope::record_items("tools.filesystem.write.relative_path", 1);
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
        Ok(_) => {
            linkscope::record_bytes(
                "tools.filesystem.write.bytes",
                usize_to_u64_saturating(content.len()),
            );
            ExecutionResult::success(format!(
                "Successfully wrote {} bytes to {file_path}",
                content.len()
            ))
        }
        Err(e) => {
            linkscope::record_items("tools.filesystem.write.error", 1);
            ExecutionResult::failure(format!("write: cannot write {file_path}: {e}"))
        }
    }
}

/// Perform a string replacement edit on a file.
pub async fn edit_file(
    file_path: &str,
    old_string: &str,
    new_string: &str,
    replace_all: bool,
) -> ExecutionResult {
    let _linkscope_edit = linkscope::phase("tools.filesystem.edit");
    let path = Path::new(file_path);
    if !path.is_absolute() {
        linkscope::record_items("tools.filesystem.edit.relative_path", 1);
        return ExecutionResult::failure(format!(
            "edit: path must be absolute (got '{file_path}')"
        ));
    }
    let content = match tokio::fs::read_to_string(path).await {
        Ok(s) => s,
        Err(e) => {
            linkscope::record_items("tools.filesystem.edit.read_error", 1);
            return ExecutionResult::failure(format!("edit: cannot read {file_path}: {e}"));
        }
    };

    if old_string.is_empty() {
        linkscope::record_items("tools.filesystem.edit.empty_old_string", 1);
        return ExecutionResult::failure("edit: old_string must not be empty".to_string());
    }

    let count = content.matches(old_string).count();

    // Tier 1: exact match (fast path, fully backwards-compatible).
    if count == 1 || (count > 1 && replace_all) {
        let new_content = if replace_all {
            content.replace(old_string, new_string)
        } else {
            content.replacen(old_string, new_string, 1)
        };
        linkscope::record_items(
            "tools.filesystem.edit.exact_match",
            usize_to_u64_saturating(count),
        );
        return write_edit(path, file_path, &new_content, count.max(1)).await;
    }
    if count > 1 && !replace_all {
        linkscope::record_items("tools.filesystem.edit.ambiguous_exact", 1);
        return ExecutionResult::failure(format!(
            "edit: old_string appears {count} times in {file_path}. Use replace_all=true for multiple replacements, or provide a more specific match."
        ));
    }

    // Tier 2: whitespace-tolerant fallback. The most common cause of
    // "old_string not found" is the model reproducing a block with slightly
    // different indentation or trailing whitespace than the file on disk. We
    // re-locate the block by comparing whitespace-normalized lines, and — only
    // when the match is UNIQUE — replace the real byte range. The "unique or
    // fail" safety property is preserved: an ambiguous normalized match still
    // fails rather than guessing.
    match locate_whitespace_insensitive(&content, old_string) {
        WsMatch::Unique(range) => {
            linkscope::record_items("tools.filesystem.edit.ws_match", 1);
            let mut new_content = String::with_capacity(content.len());
            new_content.push_str(&content[..range.start]);
            new_content.push_str(new_string);
            new_content.push_str(&content[range.end..]);
            write_edit(path, file_path, &new_content, 1).await
        }
        WsMatch::Ambiguous(n) => ExecutionResult::failure(format!(
            "edit: old_string not found exactly, and the whitespace-insensitive match is ambiguous ({n} candidates) in {file_path}. Provide a more specific old_string (include surrounding lines)."
        )),
        WsMatch::None => ExecutionResult::failure(format!(
            "edit: old_string not found in {file_path}. Make sure it matches exactly (including whitespace)."
        )),
    }
}

/// Write the edited content and produce the standard success/failure result.
async fn write_edit(
    path: &Path,
    file_path: &str,
    new_content: &str,
    replacements: usize,
) -> ExecutionResult {
    match tokio::fs::write(path, new_content).await {
        Ok(_) => {
            linkscope::record_items(
                "tools.filesystem.edit.replacements",
                usize_to_u64_saturating(replacements),
            );
            linkscope::record_bytes(
                "tools.filesystem.edit.bytes",
                usize_to_u64_saturating(new_content.len()),
            );
            ExecutionResult::success(format!(
                "Successfully edited {file_path} ({replacements} replacement(s))"
            ))
        }
        Err(e) => {
            linkscope::record_items("tools.filesystem.edit.write_error", 1);
            ExecutionResult::failure(format!("edit: cannot write {file_path}: {e}"))
        }
    }
}

fn usize_to_u64_saturating(value: usize) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
}

/// Outcome of a whitespace-insensitive search for `needle` in `haystack`.
enum WsMatch {
    /// Exactly one matching region — carries its real byte range in `haystack`.
    Unique(std::ops::Range<usize>),
    /// More than one region matched — too risky to auto-pick (carries count).
    Ambiguous(usize),
    /// No region matched.
    None,
}

/// Fold Unicode punctuation variants LLMs substitute for ASCII (smart quotes,
/// em/en dashes, non-breaking space) back to ASCII so an edit differing only by
/// `"`/`"` or `—`/`-` still matches. Mirrors Codex's Unicode-normalization tier.
fn fold_unicode_punct(ch: char) -> char {
    match ch {
        '\u{2018}' | '\u{2019}' | '\u{201B}' | '\u{2032}' => '\'',
        '\u{201C}' | '\u{201D}' | '\u{201F}' | '\u{2033}' => '"',
        '\u{2010}' | '\u{2011}' | '\u{2012}' | '\u{2013}' | '\u{2014}' | '\u{2015}' => '-',
        '\u{00A0}' | '\u{2007}' | '\u{202F}' => ' ',
        other => other,
    }
}

/// Normalize a line for whitespace-insensitive comparison: fold Unicode
/// punctuation variants to ASCII, trim leading/trailing whitespace, and collapse
/// internal whitespace runs to a single space. This makes the fallback tolerant
/// of re-indentation, trailing-space drift, and smart-quote/dash substitution
/// while still requiring the *content* to match.
fn normalize_ws(line: &str) -> String {
    let folded: String = line.chars().map(fold_unicode_punct).collect();
    folded.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Locate `needle` in `haystack` comparing line-by-line on whitespace-normalized
/// content. Operates on whole lines: it finds a run of consecutive `haystack`
/// lines whose normalized forms equal the normalized `needle` lines, and returns
/// the real byte range spanning those lines (so the original text — including its
/// real indentation — is what gets replaced). Returns the byte range only when
/// the match is unique.
fn locate_whitespace_insensitive(haystack: &str, needle: &str) -> WsMatch {
    let needle_norm: Vec<String> = needle.lines().map(normalize_ws).collect();
    // Drop trailing empty normalized lines from the needle so a stray newline in
    // old_string doesn't prevent a match.
    let needle_norm: Vec<String> = {
        let mut v = needle_norm;
        while v.last().map(|s| s.is_empty()).unwrap_or(false) {
            v.pop();
        }
        v
    };
    if needle_norm.is_empty() {
        return WsMatch::None;
    }

    // Build the list of haystack lines with their byte offsets (start, end)
    // where end excludes the line terminator.
    let mut line_spans: Vec<(usize, usize, String)> = Vec::new();
    let mut idx = 0usize;
    for line in haystack.split_inclusive('\n') {
        let trimmed_len = line.trim_end_matches('\n').len();
        let start = idx;
        let end = idx + trimmed_len;
        line_spans.push((start, end, normalize_ws(&haystack[start..end])));
        idx += line.len();
    }

    let window = needle_norm.len();
    if window > line_spans.len() {
        return WsMatch::None;
    }

    let mut matches: Vec<std::ops::Range<usize>> = Vec::new();
    for i in 0..=(line_spans.len() - window) {
        let candidate = &line_spans[i..i + window];
        if candidate
            .iter()
            .zip(&needle_norm)
            .all(|((_, _, norm), need)| norm == need)
        {
            let start = candidate[0].0;
            let end = candidate[window - 1].1;
            matches.push(start..end);
        }
    }

    match matches.len() {
        0 => WsMatch::None,
        1 => WsMatch::Unique(matches.into_iter().next().unwrap()),
        n => WsMatch::Ambiguous(n),
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
    async fn edit_tolerates_indentation_drift_normal() {
        // The file uses 8-space indentation; the model supplies the block with
        // 4-space indentation. Tier 1 (exact) misses; Tier 2 (whitespace) hits.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("indent.rs");
        let path_str = path.to_str().unwrap();
        write_file(
            path_str,
            "fn f() {\n        let x = 1;\n        let y = 2;\n}\n",
        )
        .await;

        let result = edit_file(
            path_str,
            "    let x = 1;\n    let y = 2;", // 4-space indent
            "        let x = 10;\n        let y = 20;",
            false,
        )
        .await;
        assert!(
            !result.is_error(),
            "ws-tolerant edit should succeed: {}",
            result.output
        );

        let content = tokio::fs::read_to_string(&path).await.unwrap();
        assert!(content.contains("let x = 10;"));
        assert!(content.contains("let y = 20;"));
    }

    #[tokio::test]
    async fn edit_tolerates_trailing_whitespace_drift_robust() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("trail.txt");
        let path_str = path.to_str().unwrap();
        // File line has a trailing space; model's old_string doesn't.
        write_file(path_str, "alpha \nbeta\n").await;
        let result = edit_file(path_str, "alpha", "ALPHA", false).await;
        assert!(!result.is_error());
        let content = tokio::fs::read_to_string(&path).await.unwrap();
        assert!(content.contains("ALPHA"));
    }

    #[tokio::test]
    async fn edit_ambiguous_ws_match_still_fails_robust() {
        // Two whitespace-equivalent regions → must fail, not silently pick one.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("amb.rs");
        let path_str = path.to_str().unwrap();
        write_file(path_str, "    foo();\nbar();\n        foo();\n").await;
        // Exact match for "foo();" appears twice (replace_all=false) → exact tier
        // already reports the duplicate; ensure that path is unchanged.
        let result = edit_file(path_str, "foo();", "baz();", false).await;
        assert!(result.is_error());
    }

    #[tokio::test]
    async fn edit_no_match_still_reports_not_found_robust() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("none.txt");
        let path_str = path.to_str().unwrap();
        write_file(path_str, "hello world\n").await;
        let result = edit_file(path_str, "nonexistent block", "x", false).await;
        assert!(result.is_error());
        assert!(result.output.contains("not found"));
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

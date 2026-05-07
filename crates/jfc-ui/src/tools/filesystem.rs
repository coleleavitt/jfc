use std::path::PathBuf;
use std::sync::Arc;

use tokio::sync::Mutex;
use tracing::{debug, info, trace, warn};

use crate::context::ReadDedupCache;
use crate::types::ReplacementMode;
use super::{ExecutionResult, push_pending_tool_attachment};

pub(super) async fn execute_read(
    file_path: &str,
    offset: Option<u64>,
    limit: Option<u64>,
    dedup: Option<&Arc<Mutex<ReadDedupCache>>>,
) -> ExecutionResult {
    debug!(target: "jfc::tools", file_path, offset, limit, "read: starting");

    // v132 idle prefetch fast-path: if the model referenced this file
    // mid-stream, the cache may already hold the body. Whole-file reads
    // (offset = None, limit = None) are cacheable; partial reads bypass
    // since the cache is keyed by full content.
    if offset.is_none() && limit.is_none() {
        if let Some(cached) = crate::idle_prefetch::get(file_path, None, None) {
            tracing::debug!(
                target: "jfc::tools::prefetch",
                file_path,
                cached_bytes = cached.len(),
                "Read cache HIT (idle prefetch)"
            );
            return ExecutionResult::success(cached);
        }
    }

    let path = PathBuf::from(file_path);

    // PDF fast-path: load the file as binary, stage it as an
    // attachment for the next outgoing request, return a textual
    // summary so the tool_result row in the transcript stays
    // human-readable. Without this branch, `tokio::fs::read_to_string`
    // below would either fail (non-UTF8) or produce mojibake the
    // model can't use.
    let is_pdf = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.eq_ignore_ascii_case("pdf"))
        .unwrap_or(false);
    if is_pdf && !path.is_dir() {
        match crate::attachments::read_pdf_file(&path) {
            Ok(att) => {
                let bytes = att.bytes.len();
                push_pending_tool_attachment(att);
                tracing::info!(
                    target: "jfc::tools",
                    file_path,
                    bytes,
                    "read: staged PDF as attachment"
                );
                return ExecutionResult::success(format!(
                    "Loaded PDF {} ({} bytes). The full document is attached \
                     to this tool_result and will be sent to the model as a \
                     `document` content block — you can reason about its \
                     pages, text, and embedded images directly.",
                    file_path, bytes
                ));
            }
            Err(e) => {
                tracing::warn!(target: "jfc::tools", file_path, error = %e, "read: PDF load failed");
                return ExecutionResult::failure(format!("Cannot read PDF: {e}"));
            }
        }
    }

    if path.is_dir() {
        match tokio::fs::read_dir(&path).await {
            Ok(mut entries) => {
                let mut names = Vec::new();
                while let Ok(Some(entry)) = entries.next_entry().await {
                    let name = entry.file_name().to_string_lossy().to_string();
                    if entry.path().is_dir() {
                        names.push(format!("{name}/"));
                    } else {
                        names.push(name);
                    }
                }
                names.sort();
                debug!(target: "jfc::tools", entry_count = names.len(), "read: directory listed");
                ExecutionResult::success(names.join("\n"))
            }
            Err(e) => {
                warn!(target: "jfc::tools", file_path, error = %e, "read: cannot read directory");
                ExecutionResult::failure(format!("Cannot read directory: {e}"))
            }
        }
    } else {
        // Dedup only applies to a full re-read (no offset, no limit).
        // Paginated reads (offset/limit set) are how the model walks
        // long files — blocking those leaves it stuck after the first
        // page. The previous behavior treated every Read as "already
        // saw it" because the cache keyed on path alone, so attempts
        // to read line 2000+ of a file got the unchanged stub.
        let is_full_read = offset.is_none() && limit.is_none();
        if is_full_read {
            if let Some(cache) = dedup {
                let guard = cache.lock().await;
                if guard.is_unchanged(&path) {
                    trace!(target: "jfc::tools", file_path, "read: dedup cache hit on full re-read");
                    return ExecutionResult::success(
                        "File unchanged since last full read. The content from \
                         the earlier Read tool_result in this conversation is \
                         still current — refer to that, or pass `offset`/`limit` \
                         to read a specific range."
                            .to_string(),
                    );
                }
                drop(guard);
            }
        }

        match tokio::fs::read_to_string(&path).await {
            Ok(content) => {
                let max_lines = limit.unwrap_or(2000) as usize;
                let start = offset.unwrap_or(1).saturating_sub(1) as usize;
                let lines: Vec<&str> = content.lines().collect();
                let total_lines = lines.len();
                let slice = &lines[start.min(total_lines)..];
                let slice = &slice[..slice.len().min(max_lines)];
                let numbered: String = slice
                    .iter()
                    .enumerate()
                    .map(|(i, line)| format!("{}: {line}", start + i + 1))
                    .collect::<Vec<_>>()
                    .join("\n");

                // Only record a "full read" in the cache so partial
                // reads don't poison subsequent full reads with a
                // false-positive unchanged stub.
                if is_full_read {
                    if let Some(cache) = dedup {
                        cache.lock().await.record_read(path);
                    }
                }

                debug!(
                    target: "jfc::tools",
                    file_path, line_count = slice.len(), total_lines, start,
                    "read: success"
                );

                // v132 parity: surface a `<system-reminder>` when the
                // file is empty or the offset overshoots the line
                // count — without it the model sees a blank tool
                // result and often re-reads. The reminder makes the
                // root cause visible.
                if total_lines == 0 {
                    return ExecutionResult::success(crate::system_reminder::format(
                        &format!(
                            "Warning: the file at {file_path} exists but its contents are empty."
                        ),
                    ));
                }
                if start >= total_lines {
                    return ExecutionResult::success(crate::system_reminder::format(
                        &format!(
                            "Warning: the file at {file_path} exists but is shorter \
                             than the provided offset ({}). The file has {} lines.",
                            start + 1,
                            total_lines
                        ),
                    ));
                }
                ExecutionResult::success(numbered)
            }
            Err(e) => {
                warn!(target: "jfc::tools", file_path, error = %e, "read: cannot read file");
                ExecutionResult::failure(format!("Cannot read file: {e}"))
            }
        }
    }
}

pub(super) async fn execute_write(file_path: &str, content: &str) -> ExecutionResult {
    info!(target: "jfc::tools", file_path, content_len = content.len(), "write: starting");
    let path = PathBuf::from(file_path);
    if let Some(parent) = path.parent() {
        if let Err(e) = tokio::fs::create_dir_all(parent).await {
            warn!(target: "jfc::tools", file_path, error = %e, "write: cannot create directories");
            return ExecutionResult::failure(format!("Cannot create directories: {e}"));
        }
    }
    // Capture the prior contents so we can emit a real diff when this
    // is an *overwrite* (Edit-shaped change) instead of a new file.
    // v126 always renders a diff for Write so the user sees what
    // actually changed; a bare "Written 97 bytes" tells them nothing.
    let prior = tokio::fs::read_to_string(&path).await.ok();
    match tokio::fs::write(&path, content).await {
        Ok(_) => {
            let line_count = content.lines().count();
            let bytes = content.len();
            debug!(target: "jfc::tools", file_path, bytes, line_count, "write: success");
            let header = match &prior {
                Some(_) => format!("Updated {file_path} ({bytes} bytes, {line_count} lines)"),
                None => format!("Wrote {file_path} ({bytes} bytes, {line_count} lines)"),
            };
            // Output clean, unprefixed code — the renderer's syntax
            // highlighter (`render_highlighted_with_line_numbers` →
            // syntect) needs valid source to colorize. Earlier the
            // body had each line prefixed with `+ ` for diff-style
            // visual cues, but that turned every line into invalid
            // syntax (`+ const std = ...` parses as a stray binary-
            // add expression in every language) so highlighting
            // silently fell back to plain text. The diff/sigil
            // semantics belong on `ToolOutput::Diff`, not on a
            // Write's plain text output. The header stays on its own
            // line at the top — it's not part of the highlighted body.
            const PREVIEW_LINES: usize = 30;
            let preview: String = content
                .lines()
                .take(PREVIEW_LINES)
                .collect::<Vec<_>>()
                .join("\n");
            let footer = if line_count > PREVIEW_LINES {
                format!(
                    "\n\n… ({} more lines, full content on disk)",
                    line_count - PREVIEW_LINES
                )
            } else {
                String::new()
            };
            ExecutionResult::success(format!("{header}\n\n{preview}{footer}"))
        }
        Err(e) => {
            warn!(target: "jfc::tools", file_path, error = %e, "write: cannot write file");
            ExecutionResult::failure(format!("Cannot write file: {e}"))
        }
    }
}

pub(super) async fn execute_edit(
    file_path: &str,
    old_string: &str,
    new_string: &str,
    replacement: ReplacementMode,
) -> ExecutionResult {
    let replace_all = replacement.replace_all();
    info!(target: "jfc::tools", file_path, old_len = old_string.len(), new_len = new_string.len(), replace_all, "edit: starting");
    match tokio::fs::read_to_string(file_path).await {
        Ok(content) => {
            if old_string.is_empty() && !content.is_empty() {
                return ExecutionResult::failure(
                    "old_string is empty but file is not empty. Provide text to replace.",
                );
            }
            let count = content.matches(old_string).count();
            if count == 0 {
                warn!(target: "jfc::tools", file_path, "edit: old_string not found");
                return ExecutionResult::failure(format!("old_string not found in {file_path}"));
            }
            if count > 1 && !replacement.replace_all() {
                warn!(target: "jfc::tools", file_path, count, "edit: multiple matches found");
                return ExecutionResult::failure(format!(
                    "Found {count} matches for old_string in {file_path}. Use replace_all=true or provide more context."
                ));
            }
            let new_content = if replacement.replace_all() {
                content.replace(old_string, new_string)
            } else {
                content.replacen(old_string, new_string, 1)
            };
            match tokio::fs::write(file_path, &new_content).await {
                Ok(_) => {
                    debug!(target: "jfc::tools", file_path, count, "edit: success");
                    // Compute line-level diff stats (matches v126's "Added N lines, Removed M lines")
                    let old_lines = old_string.lines().count();
                    let new_lines = new_string.lines().count();
                    let lines_added = new_lines.saturating_sub(old_lines);
                    let lines_removed = old_lines.saturating_sub(new_lines);
                    let line_summary = match (lines_added, lines_removed) {
                        (0, 0) => format!("{} lines changed", old_lines.max(1)),
                        (a, 0) => format!("+{a} lines"),
                        (0, r) => format!("-{r} lines"),
                        (a, r) => format!("+{a}/-{r} lines"),
                    };
                    // Build a structured DiffView so the renderer
                    // shows a colorized hunk like Write does for new
                    // files. The previous "file updated successfully"
                    // string told the user nothing about WHAT changed
                    // — they had to open the file to verify. Mirrors
                    // v126's Edit-tool diff display.
                    let diff = build_edit_diff_view(file_path, &content, &new_content);
                    let header = if replacement.replace_all() && count > 1 {
                        format!("{file_path} updated ({line_summary}, {count} occurrences)")
                    } else {
                        format!("{file_path} updated ({line_summary})")
                    };
                    ExecutionResult::success(header).with_diff(diff)
                }
                Err(e) => {
                    warn!(target: "jfc::tools", file_path, error = %e, "edit: cannot write after edit");
                    ExecutionResult::failure(format!("Cannot write file after edit: {e}"))
                }
            }
        }
        Err(_) if old_string.is_empty() => match tokio::fs::write(file_path, new_string).await {
            Ok(_) => {
                debug!(target: "jfc::tools", file_path, "edit: created new file");
                ExecutionResult::success(format!("Created new file {file_path}"))
            }
            Err(e2) => {
                warn!(target: "jfc::tools", file_path, error = %e2, "edit: cannot create file");
                ExecutionResult::failure(format!("Cannot create file: {e2}"))
            }
        },
        Err(e) => {
            warn!(target: "jfc::tools", file_path, error = %e, "edit: cannot read file");
            ExecutionResult::failure(format!("Cannot read file: {e}"))
        }
    }
}

/// Build a `DiffView` that walks the line-by-line difference between
/// `old` and `new` and groups changed-region(s) into hunks with a few
/// lines of context. Not as fancy as a real LCS-based diff (no min-edit
/// guarantees) but adequate for Edit-tool display where the change is a
/// localized old_string→new_string replacement. Mirrors what unified
/// diff renders look like, fed straight into the existing
/// `ToolOutput::Diff` renderer.
pub(super) fn build_edit_diff_view(file_path: &str, old: &str, new: &str) -> crate::types::DiffView {
    use crate::types::{DiffHunk, DiffLine, DiffLineKind, DiffView};
    const CONTEXT: usize = 3;
    let old_lines: Vec<&str> = old.lines().collect();
    let new_lines: Vec<&str> = new.lines().collect();

    // Find the first and last lines that differ. If the file is
    // unchanged, this yields an empty hunk list and the renderer just
    // shows the title — matches v126's "no-op edit" rendering.
    let mut first = 0;
    while first < old_lines.len() && first < new_lines.len() && old_lines[first] == new_lines[first]
    {
        first += 1;
    }
    let mut last_old = old_lines.len();
    let mut last_new = new_lines.len();
    while last_old > first && last_new > first && old_lines[last_old - 1] == new_lines[last_new - 1]
    {
        last_old -= 1;
        last_new -= 1;
    }

    let mut additions = 0usize;
    let mut deletions = 0usize;
    let mut hunks: Vec<DiffHunk> = Vec::new();
    let has_change = last_old > first || last_new > first;
    if has_change {
        let ctx_start = first.saturating_sub(CONTEXT);
        let ctx_end_old = (last_old + CONTEXT).min(old_lines.len());
        let ctx_end_new = (last_new + CONTEXT).min(new_lines.len());
        let mut lines: Vec<DiffLine> = Vec::new();
        // Leading context (from old; identical in new at this offset).
        let mut old_lineno = ctx_start + 1;
        let mut new_lineno = ctx_start + 1;
        for line in &old_lines[ctx_start..first] {
            lines.push(DiffLine {
                kind: DiffLineKind::Context,
                old_line: Some(old_lineno),
                new_line: Some(new_lineno),
                content: (*line).to_owned(),
            });
            old_lineno += 1;
            new_lineno += 1;
        }
        // Removed lines.
        for line in &old_lines[first..last_old] {
            lines.push(DiffLine {
                kind: DiffLineKind::Removed,
                old_line: Some(old_lineno),
                new_line: None,
                content: (*line).to_owned(),
            });
            old_lineno += 1;
            deletions += 1;
        }
        // Added lines.
        for line in &new_lines[first..last_new] {
            lines.push(DiffLine {
                kind: DiffLineKind::Added,
                old_line: None,
                new_line: Some(new_lineno),
                content: (*line).to_owned(),
            });
            new_lineno += 1;
            additions += 1;
        }
        // Trailing context.
        for (i, line) in old_lines[last_old..ctx_end_old].iter().enumerate() {
            lines.push(DiffLine {
                kind: DiffLineKind::Context,
                old_line: Some(old_lineno + i),
                new_line: Some(new_lineno + i),
                content: (*line).to_owned(),
            });
        }
        let _ = ctx_end_new; // reserved for future LCS-based hunks
        let header = format!(
            "@@ -{old_start},{old_count} +{new_start},{new_count} @@",
            old_start = ctx_start + 1,
            old_count = ctx_end_old - ctx_start,
            new_start = ctx_start + 1,
            new_count = (ctx_end_old - ctx_start) + new_lines.len() - old_lines.len(),
        );
        hunks.push(DiffHunk {
            old_start: ctx_start + 1,
            new_start: ctx_start + 1,
            header,
            lines,
        });
    }

    DiffView {
        file_path: file_path.to_owned(),
        hunks,
        additions,
        deletions,
    }
}


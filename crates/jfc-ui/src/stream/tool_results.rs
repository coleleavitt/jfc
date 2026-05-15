use crate::types::ToolOutput;

/// Reuse the same cap that `ToolOutput::approx_text_len` enforces. The wire
/// truncation here and the local token estimate must agree to a byte, or
/// `compact_level` will fire on phantom-large outputs that the API never sees.
pub(crate) const MAX_TOOL_RESULT_CHARS: usize = ToolOutput::APPROX_LEN_CAP;

/// Bytes shown at each end of a truncated tool result.
pub(crate) const TRUNCATION_PREVIEW_CHARS: usize = 2_000;

/// Tool results above this size get spilled to a temp file on disk instead of
/// being held entirely in memory plus the conversation.
pub(crate) const TOOL_RESULT_DISK_PERSIST_BYTES: usize = 400_000;

/// Delete tool-result spill files older than `max_age`. Called at startup and
/// on session end to prevent unbounded /tmp growth.
pub(crate) fn cleanup_tool_result_spills(max_age: std::time::Duration) {
    let dir = std::env::temp_dir().join("jfc-tool-results");
    let entries = match std::fs::read_dir(&dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    let cutoff = std::time::SystemTime::now()
        .checked_sub(max_age)
        .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
    let mut deleted = 0u64;
    let mut freed = 0u64;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("txt") {
            continue;
        }
        let Ok(meta) = std::fs::metadata(&path) else {
            continue;
        };
        let Ok(mtime) = meta.modified() else {
            continue;
        };
        if mtime < cutoff {
            freed += meta.len();
            if std::fs::remove_file(&path).is_ok() {
                deleted += 1;
            }
        }
    }
    if deleted > 0 {
        tracing::info!(
            target: "jfc::stream",
            deleted,
            freed_bytes = freed,
            "cleaned up stale tool-result spill files"
        );
    }
}

/// Persist `body` to a temp file under `/tmp/jfc-tool-results/` and return a
/// v131-style `<persisted-output>` reference the model can read.
pub(crate) fn persist_tool_result(body: &str) -> String {
    use std::io::Write as _;
    let dir = std::env::temp_dir().join("jfc-tool-results");
    if let Err(e) = std::fs::create_dir_all(&dir) {
        tracing::warn!(
            target: "jfc::stream",
            error = %e,
            "failed to create tool-result spill dir, falling back to in-memory truncation"
        );
        return truncate_tool_result(body);
    }
    let id = uuid::Uuid::new_v4().simple().to_string();
    let path = dir.join(format!("{id}.txt"));
    let file_open = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path);
    let mut file = match file_open {
        Ok(f) => f,
        Err(e) => {
            tracing::warn!(
                target: "jfc::stream",
                path = %path.display(),
                error = %e,
                "failed to create tool-result spill file, falling back"
            );
            return truncate_tool_result(body);
        }
    };
    if let Err(e) = file.write_all(body.as_bytes()) {
        tracing::warn!(
            target: "jfc::stream",
            path = %path.display(),
            error = %e,
            "failed to write tool-result spill, falling back"
        );
        let _ = std::fs::remove_file(&path);
        return truncate_tool_result(body);
    }
    let preview_end = floor_char_boundary(body, TRUNCATION_PREVIEW_CHARS);
    let preview = &body[..preview_end];
    let total = body.len();
    format!(
        "<persisted-output original_bytes=\"{total}\" path=\"{}\">\n\
         Output too large for inline conversation ({total} bytes). \
         Full output saved to: {}\n\n\
         Preview (first {preview_end} chars):\n\
         {preview}\n…\n\
         </persisted-output>",
        path.display(),
        path.display()
    )
}

/// Apply the appropriate cap to a tool result: spill to disk above 400KB,
/// head/tail truncate above 50KB, otherwise pass through.
pub(crate) fn cap_tool_result(body: &str) -> String {
    if body.len() > TOOL_RESULT_DISK_PERSIST_BYTES {
        persist_tool_result(body)
    } else {
        truncate_tool_result(body)
    }
}

/// Truncate `s` to at most `MAX_TOOL_RESULT_CHARS` bytes when oversized.
/// Slice boundaries are snapped to UTF-8 codepoints.
pub(crate) fn truncate_tool_result(s: &str) -> String {
    if s.len() <= MAX_TOOL_RESULT_CHARS {
        return s.to_owned();
    }
    let preview = TRUNCATION_PREVIEW_CHARS.min(MAX_TOOL_RESULT_CHARS / 2);
    let head_end = floor_char_boundary(s, preview);
    let tail_start = ceil_char_boundary(s, s.len().saturating_sub(preview));
    let head = &s[..head_end];
    let tail = &s[tail_start..];
    let omitted = s.len() - head_end - (s.len() - tail_start);
    let total = s.len();
    format!(
        "<truncated-output original_bytes=\"{total}\" omitted_bytes=\"{omitted}\">\n\
         Output too large for the conversation. Showing first {preview} \
         chars and last {preview} chars; {omitted} bytes omitted from the \
         middle. If you need the elided section, ask the user or re-invoke \
         the tool with a narrower scope (smaller path / line range / Grep \
         pattern).\n\n\
         --- preview head ---\n\
         {head}\n\
         --- preview tail ---\n\
         {tail}\n\
         </truncated-output>"
    )
}

fn floor_char_boundary(s: &str, mut i: usize) -> usize {
    if i >= s.len() {
        return s.len();
    }
    while i > 0 && !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

fn ceil_char_boundary(s: &str, mut i: usize) -> usize {
    if i >= s.len() {
        return s.len();
    }
    while i < s.len() && !s.is_char_boundary(i) {
        i += 1;
    }
    i
}

#[cfg(test)]
mod truncate_tests {
    use super::*;

    #[test]
    fn truncate_short_passes_through_normal() {
        assert_eq!(truncate_tool_result("hello"), "hello");
    }

    #[test]
    fn truncate_does_not_panic_on_multibyte_char_at_split_boundary_robust() {
        let prefix_bytes = MAX_TOOL_RESULT_CHARS / 2 - 2;
        let mut s = String::with_capacity(MAX_TOOL_RESULT_CHARS * 2);
        for _ in 0..prefix_bytes {
            s.push('a');
        }
        s.push('🦀');
        for _ in 0..(MAX_TOOL_RESULT_CHARS) {
            s.push('b');
        }
        let _ = truncate_tool_result(&s);
    }

    #[test]
    fn truncate_output_is_valid_utf8_robust() {
        let s: String = std::iter::repeat("héllo 🌟 ").take(5000).collect();
        let out = truncate_tool_result(&s);
        let _ = out.chars().count();
    }

    #[test]
    fn truncate_keeps_head_and_tail_normal() {
        let mid: String = "x".repeat(MAX_TOOL_RESULT_CHARS * 2);
        let s = format!("HEAD{mid}TAIL");
        let out = truncate_tool_result(&s);
        assert!(out.starts_with("<truncated-output"));
        assert!(out.contains("HEAD"));
        assert!(out.contains("TAIL"));
        assert!(out.contains("omitted_bytes"));
        assert!(out.ends_with("</truncated-output>"));
    }

    #[test]
    fn truncate_marker_includes_original_byte_count_normal() {
        let s = "x".repeat(MAX_TOOL_RESULT_CHARS * 3);
        let out = truncate_tool_result(&s);
        let expected = format!("original_bytes=\"{}\"", s.len());
        assert!(out.contains(&expected), "marker missing byte count: {out}");
    }
}

#[cfg(test)]
mod truncate_more_tests {
    use super::*;

    #[test]
    fn truncate_at_exact_cap_passes_through_robust() {
        let s: String = "x".repeat(MAX_TOOL_RESULT_CHARS);
        let out = truncate_tool_result(&s);
        assert_eq!(out, s);
        assert!(!out.contains("<truncated-output"));
    }

    #[test]
    fn truncate_one_over_cap_does_truncate_robust() {
        let s: String = "y".repeat(MAX_TOOL_RESULT_CHARS + 1);
        let out = truncate_tool_result(&s);
        assert!(out.contains("<truncated-output"));
    }

    #[test]
    fn cap_tool_result_small_body_pass_through_normal() {
        let body = "tiny output";
        assert_eq!(cap_tool_result(body), body);
    }

    #[test]
    fn cap_tool_result_medium_body_truncates_inline_normal() {
        let body: String = "x".repeat(100_000);
        let out = cap_tool_result(&body);
        assert!(
            out.contains("<truncated-output"),
            "expected inline truncation marker"
        );
        assert!(
            !out.contains("<persisted-output"),
            "should not persist below 400KB threshold"
        );
    }

    #[test]
    fn cap_tool_result_large_body_persists_to_disk_normal() {
        let body: String = "y".repeat(TOOL_RESULT_DISK_PERSIST_BYTES + 100);
        let out = cap_tool_result(&body);
        assert!(
            out.contains("<persisted-output"),
            "expected persisted-output reference: {}",
            &out[..200.min(out.len())]
        );
        assert!(out.contains("path=\""));
        assert!(out.contains(&format!("original_bytes=\"{}\"", body.len())));
        let path_start = out.find("path=\"").map(|p| p + "path=\"".len()).unwrap();
        let path_end = out[path_start..].find('"').map(|p| path_start + p).unwrap();
        let path = &out[path_start..path_end];
        let on_disk = std::fs::read_to_string(path).expect("spilled file should exist");
        assert_eq!(on_disk.len(), body.len());
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn cap_tool_result_persisted_includes_preview_robust() {
        let head = "HEADMARKER";
        let body = format!("{head}{}", "z".repeat(TOOL_RESULT_DISK_PERSIST_BYTES));
        let out = cap_tool_result(&body);
        assert!(
            out.contains(head),
            "preview missing head marker: {}",
            &out[..200.min(out.len())]
        );
        if let Some(s) = out.find("path=\"") {
            let s = s + "path=\"".len();
            if let Some(e) = out[s..].find('"') {
                let _ = std::fs::remove_file(&out[s..s + e]);
            }
        }
    }

    #[test]
    fn floor_char_boundary_endpoints_normal() {
        let s = "hello";
        assert_eq!(floor_char_boundary(s, 0), 0);
        assert_eq!(floor_char_boundary(s, s.len()), s.len());
        assert_eq!(floor_char_boundary(s, 100), s.len());
    }

    #[test]
    fn ceil_char_boundary_endpoints_normal() {
        let s = "hello";
        assert_eq!(ceil_char_boundary(s, 0), 0);
        assert_eq!(ceil_char_boundary(s, s.len()), s.len());
        assert_eq!(ceil_char_boundary(s, 100), s.len());
    }
}

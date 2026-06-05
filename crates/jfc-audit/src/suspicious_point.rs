use regex::Regex;
use serde::{Deserialize, Serialize};

/// The kind of suspicious pattern detected.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TriggerKind {
    UnsafeBlock,
    Unwrap,
    Expect,
    Panic,
    Unreachable,
    ArrayIndex,
    FfiCall,
    TaintedLoop,
    UnsafeTransmute,
    RawPointer,
}

/// A suspicious point detected by pattern matching.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SuspiciousPoint {
    pub handle: String,
    pub file: String,
    pub region_lines: (u32, u32),
    pub trigger_kind: TriggerKind,
    pub context_snippet: String,
    pub surrounding_function: String,
}

/// Pattern-based scanner for suspicious code points.
pub struct SuspiciousPointFinder {
    patterns: Vec<(TriggerKind, Regex)>,
}

impl SuspiciousPointFinder {
    pub fn new() -> Self {
        let patterns = vec![
            (
                TriggerKind::UnsafeBlock,
                Regex::new(r"unsafe\s*\{").unwrap(),
            ),
            (TriggerKind::Unwrap, Regex::new(r"\.unwrap\(\)").unwrap()),
            (TriggerKind::Expect, Regex::new(r"\.expect\(").unwrap()),
            (TriggerKind::Panic, Regex::new(r"panic!\(").unwrap()),
            (
                TriggerKind::Unreachable,
                Regex::new(r"unreachable!\(").unwrap(),
            ),
            (
                TriggerKind::UnsafeTransmute,
                Regex::new(r"mem::transmute").unwrap(),
            ),
            (
                TriggerKind::RawPointer,
                Regex::new(
                    r"(Box::from_raw|Vec::set_len|slice::from_raw_parts|Vec::from_raw_parts)",
                )
                .unwrap(),
            ),
            (
                TriggerKind::ArrayIndex,
                Regex::new(r"\w+\s*\[[^\]]*[a-zA-Z_]\w*[^\]]*\]").unwrap(),
            ),
        ];
        Self { patterns }
    }

    /// Scan a source file's text for suspicious points.
    pub fn scan_file(&self, file_path: &str, source: &str) -> Vec<SuspiciousPoint> {
        let mut results = Vec::new();
        let lines: Vec<&str> = source.lines().collect();

        // Track current function context
        let fn_regex = Regex::new(r"\bfn\s+([A-Za-z_][A-Za-z0-9_]*)\b").unwrap();

        let mut current_fn: Option<String> = None;
        let mut fn_brace_depth: i32 = 0;
        let mut fn_body_started = false;
        let mut in_block_comment = false;

        for (line_idx, line) in lines.iter().enumerate() {
            let line_num = (line_idx + 1) as u32;
            let code = sanitized_code_line(line, &mut in_block_comment);

            // Update function context
            let mut brace_scan_start = 0;
            if current_fn.is_none()
                && let Some(caps) = fn_regex.captures(&code)
                && let Some(name) = caps.get(1)
            {
                current_fn = Some(name.as_str().to_string());
                fn_brace_depth = 0;
                fn_body_started = false;
                brace_scan_start = caps.get(0).map(|m| m.start()).unwrap_or(0);
            }

            // Check each pattern
            for (kind, pattern) in &self.patterns {
                if pattern.is_match(&code) {
                    // Get surrounding context (up to 2 lines before/after)
                    let start = line_idx.saturating_sub(2);
                    let end = (line_idx + 3).min(lines.len());
                    let snippet: String = lines[start..end].join("\n");
                    let surrounding_function =
                        current_fn.clone().unwrap_or_else(|| "<module>".to_string());

                    results.push(SuspiciousPoint {
                        handle: format!("fn:{surrounding_function}"),
                        file: file_path.to_string(),
                        region_lines: (line_num, line_num),
                        trigger_kind: *kind,
                        context_snippet: snippet,
                        surrounding_function,
                    });
                }
            }

            if current_fn.is_some() {
                let brace_segment = code.get(brace_scan_start..).unwrap_or("");
                for ch in brace_segment.chars() {
                    match ch {
                        '{' => {
                            fn_brace_depth += 1;
                            fn_body_started = true;
                        }
                        '}' => {
                            fn_brace_depth -= 1;
                        }
                        _ => {}
                    }
                }
                if fn_body_started && fn_brace_depth <= 0 {
                    current_fn = None;
                    fn_brace_depth = 0;
                    fn_body_started = false;
                } else if !fn_body_started && code.trim_end().ends_with(';') {
                    current_fn = None;
                }
            }
        }

        results
    }

    /// Scan multiple files. Each entry is (path, source_text).
    pub fn scan_files(&self, files: &[(&str, &str)]) -> Vec<SuspiciousPoint> {
        files
            .iter()
            .flat_map(|(path, source)| self.scan_file(path, source))
            .collect()
    }
}

fn sanitized_code_line(line: &str, in_block_comment: &mut bool) -> String {
    let mut out = String::with_capacity(line.len());
    let mut chars = line.chars().peekable();
    let mut in_string = false;
    let mut string_delim = '\0';
    let mut escaped = false;

    while let Some(ch) = chars.next() {
        if *in_block_comment {
            if ch == '*' && chars.peek() == Some(&'/') {
                chars.next();
                *in_block_comment = false;
                out.push(' ');
                out.push(' ');
            } else {
                out.push(' ');
            }
            continue;
        }

        if in_string {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == string_delim {
                in_string = false;
            }
            out.push(' ');
            continue;
        }

        if ch == '/' && chars.peek() == Some(&'/') {
            break;
        }
        if ch == '/' && chars.peek() == Some(&'*') {
            chars.next();
            *in_block_comment = true;
            out.push(' ');
            out.push(' ');
            continue;
        }
        if ch == '"' {
            in_string = true;
            string_delim = ch;
            out.push(' ');
            continue;
        }

        out.push(ch);
    }

    out
}

impl Default for SuspiciousPointFinder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_unsafe_and_unwrap_normal() {
        let source = r#"
pub fn process(data: &[u8]) {
    let val = data.get(0).unwrap();
    unsafe {
        std::ptr::read(data.as_ptr());
    }
}

pub fn other() {
    panic!("oh no");
}
"#;

        let finder = SuspiciousPointFinder::new();
        let points = finder.scan_file("src/lib.rs", source);

        let kinds: Vec<TriggerKind> = points.iter().map(|p| p.trigger_kind).collect();
        assert!(kinds.contains(&TriggerKind::Unwrap));
        assert!(kinds.contains(&TriggerKind::UnsafeBlock));
        assert!(kinds.contains(&TriggerKind::Panic));
        assert!(points.len() >= 3);

        // Check that surrounding function is tracked
        let unwrap_point = points
            .iter()
            .find(|p| p.trigger_kind == TriggerKind::Unwrap)
            .unwrap();
        assert_eq!(unwrap_point.surrounding_function, "process");
    }

    #[test]
    fn empty_file_returns_empty_robust() {
        let finder = SuspiciousPointFinder::new();
        let points = finder.scan_file("empty.rs", "");
        assert!(points.is_empty());

        let points2 = finder.scan_file("whitespace.rs", "   \n\n   \n");
        assert!(points2.is_empty());
    }

    #[test]
    fn finds_raw_pointer_patterns_normal() {
        let source = r#"
pub unsafe fn bad(ptr: *mut u8, len: usize) -> Vec<u8> {
    Vec::from_raw_parts(ptr, len, len)
}

pub fn transmute_bad() {
    let x: u64 = unsafe { mem::transmute([0u8; 8]) };
}
"#;

        let finder = SuspiciousPointFinder::new();
        let points = finder.scan_file("src/ffi.rs", source);

        let kinds: Vec<TriggerKind> = points.iter().map(|p| p.trigger_kind).collect();
        assert!(kinds.contains(&TriggerKind::RawPointer));
        assert!(kinds.contains(&TriggerKind::UnsafeTransmute));
    }

    #[test]
    fn ignores_comments_and_strings_robust() {
        let source = r#"
pub fn clean() {
    // panic!("not real");
    let text = ".unwrap() unsafe { Vec::from_raw_parts(ptr, len, len) }";
    /*
       unreachable!();
    */
}
"#;

        let finder = SuspiciousPointFinder::new();
        let points = finder.scan_file("src/lib.rs", source);
        assert!(points.is_empty());
    }

    #[test]
    fn function_context_resets_after_closing_brace_robust() {
        let source = r#"
pub fn first() {
    let x = Some(1).unwrap();
}

let y = values[idx];
"#;

        let finder = SuspiciousPointFinder::new();
        let points = finder.scan_file("src/lib.rs", source);
        let module_point = points
            .iter()
            .find(|p| p.trigger_kind == TriggerKind::ArrayIndex)
            .expect("top-level array index should be reported");
        assert_eq!(module_point.surrounding_function, "<module>");
    }

    #[test]
    fn lifetime_parameters_do_not_break_function_scope_robust() {
        let source = r#"
pub fn borrowed<'a>(values: &'a [usize], idx: usize) -> usize {
    values[idx]
}
"#;

        let finder = SuspiciousPointFinder::new();
        let points = finder.scan_file("src/lib.rs", source);
        let point = points
            .iter()
            .find(|p| p.trigger_kind == TriggerKind::ArrayIndex)
            .expect("array index should be reported");
        assert_eq!(point.surrounding_function, "borrowed");
    }
}

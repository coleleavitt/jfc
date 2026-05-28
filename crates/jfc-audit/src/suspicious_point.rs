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
        let fn_regex = Regex::new(r"(pub\s+)?(unsafe\s+)?(fn\s+(\w+))").unwrap();

        let mut current_fn = String::new();

        for (line_idx, line) in lines.iter().enumerate() {
            let line_num = (line_idx + 1) as u32;

            // Update function context
            if let Some(caps) = fn_regex.captures(line)
                && let Some(name) = caps.get(4)
            {
                current_fn = name.as_str().to_string();
            }

            // Check each pattern
            for (kind, pattern) in &self.patterns {
                if pattern.is_match(line) {
                    // Get surrounding context (up to 2 lines before/after)
                    let start = line_idx.saturating_sub(2);
                    let end = (line_idx + 3).min(lines.len());
                    let snippet: String = lines[start..end].join("\n");

                    results.push(SuspiciousPoint {
                        handle: format!("fn:{current_fn}"),
                        file: file_path.to_string(),
                        region_lines: (line_num, line_num),
                        trigger_kind: *kind,
                        context_snippet: snippet,
                        surrounding_function: current_fn.clone(),
                    });
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
}

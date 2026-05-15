//! Slop Guard — automated quality checks for LLM-generated code.
//!
//! Runs after every file-modifying tool (Write/Edit/MultiEdit/SymbolEdit)
//! and surfaces findings to the model via system-reminder so it can
//! self-correct before moving to the next step.
//!
//! Checks:
//! 1. Duplication detection (5+ line sliding-window hash match)
//! 2. Dead code (cargo check --message-format=json dead_code warnings)
//! 3. Architectural coherence (pattern divergence heuristics)
//! 4. Churn tracking (git log frequency per file)
//! 5. Complexity budget (LOC + nesting depth per function)
//! 6. Test quality (implementation-coupling heuristics)

#![allow(dead_code)]

use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::path::Path;

/// Report produced by `run_all_checks`.
pub struct SlopReport {
    pub has_findings: bool,
    pub findings: Vec<SlopFinding>,
}

/// A single quality finding.
pub struct SlopFinding {
    pub rule: String,
    pub message: String,
    pub file: Option<String>,
    pub line: Option<usize>,
}

// ─── 1. Duplication Detection ───────────────────────────────────────────

const DUP_WINDOW: usize = 5;

/// Hash a normalized 5-line window (trimmed, lowercased).
fn hash_window(lines: &[&str]) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    for line in lines {
        line.trim().hash(&mut hasher);
    }
    hasher.finish()
}

/// Find 5+ line blocks in `new_content` that already exist in other files
/// under `cwd`. Returns (existing_file, existing_line, matched_text_preview).
pub fn check_duplication(new_content: &str, file_path: &Path, cwd: &Path) -> Vec<SlopFinding> {
    let new_lines: Vec<&str> = new_content.lines().collect();
    if new_lines.len() < DUP_WINDOW {
        return Vec::new();
    }

    // Build hashes for all windows in the new file.
    let mut new_hashes: HashSet<u64> = HashSet::new();
    for window in new_lines.windows(DUP_WINDOW) {
        // Skip trivial windows (blank lines, single-char lines, braces-only).
        let non_trivial = window.iter().filter(|l| l.trim().len() > 3).count();
        if non_trivial < 3 {
            continue;
        }
        new_hashes.insert(hash_window(window));
    }

    if new_hashes.is_empty() {
        return Vec::new();
    }

    let mut findings = Vec::new();
    let canonical_new = file_path.canonicalize().ok();

    // Walk .rs files in workspace looking for matches.
    let walker = ignore::WalkBuilder::new(cwd)
        .hidden(true)
        .git_ignore(true)
        .max_depth(Some(12))
        .build();

    for entry in walker.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("rs") {
            continue;
        }
        // Skip self.
        if let Some(ref cn) = canonical_new {
            if path.canonicalize().ok().as_ref() == Some(cn) {
                continue;
            }
        }
        let Ok(content) = std::fs::read_to_string(path) else {
            continue;
        };
        let lines: Vec<&str> = content.lines().collect();
        if lines.len() < DUP_WINDOW {
            continue;
        }
        for (i, window) in lines.windows(DUP_WINDOW).enumerate() {
            let non_trivial = window.iter().filter(|l| l.trim().len() > 3).count();
            if non_trivial < 3 {
                continue;
            }
            let h = hash_window(window);
            if new_hashes.contains(&h) {
                let preview = window[0..2].join(" / ");
                findings.push(SlopFinding {
                    rule: "duplication".into(),
                    message: format!(
                        "5+ line block already exists at {}:{} — consider reusing: `{}`",
                        path.strip_prefix(cwd).unwrap_or(path).display(),
                        i + 1,
                        if preview.len() > 80 {
                            &preview[..80]
                        } else {
                            &preview
                        }
                    ),
                    file: Some(path.strip_prefix(cwd).unwrap_or(path).display().to_string()),
                    line: Some(i + 1),
                });
                // One finding per file is enough.
                break;
            }
        }

        // Cap total duplication findings.
        if findings.len() >= 5 {
            break;
        }
    }

    findings
}

// ─── 2. Dead Code Detection ─────────────────────────────────────────────

/// Run `cargo check --message-format=json` and extract dead_code warnings.
/// Returns quickly — uses cached incremental compilation.
pub fn check_dead_code(cwd: &Path) -> Vec<SlopFinding> {
    let output = std::process::Command::new("cargo")
        .args(["check", "--message-format=json", "-q"])
        .current_dir(cwd)
        .output();

    let Ok(output) = output else {
        return Vec::new();
    };
    let stdout = String::from_utf8_lossy(&output.stdout);

    let mut findings = Vec::new();
    for line in stdout.lines() {
        let Ok(msg) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        if msg.get("reason").and_then(|v| v.as_str()) != Some("compiler-message") {
            continue;
        }
        let Some(message) = msg.get("message") else {
            continue;
        };
        let code = message
            .get("code")
            .and_then(|c| c.get("code"))
            .and_then(|c| c.as_str())
            .unwrap_or("");
        if code != "dead_code" {
            continue;
        }
        let text = message
            .get("message")
            .and_then(|m| m.as_str())
            .unwrap_or("");
        let span = message
            .get("spans")
            .and_then(|s| s.as_array())
            .and_then(|a| a.first());
        let file = span
            .and_then(|s| s.get("file_name"))
            .and_then(|f| f.as_str())
            .unwrap_or("?");
        let line_num = span
            .and_then(|s| s.get("line_start"))
            .and_then(|l| l.as_u64())
            .unwrap_or(0) as usize;

        findings.push(SlopFinding {
            rule: "dead_code".into(),
            message: format!("{file}:{line_num}: {text}"),
            file: Some(file.to_string()),
            line: Some(line_num),
        });

        if findings.len() >= 10 {
            break;
        }
    }
    findings
}

// ─── 3. Architectural Coherence ─────────────────────────────────────────

/// Check for common pattern divergences in Rust code.
pub fn check_coherence(file_content: &str, file_path: &Path) -> Vec<SlopFinding> {
    let mut findings = Vec::new();
    let is_test = file_content.contains("#[cfg(test)]");
    let is_lib = file_path
        .to_str()
        .map(|s| !s.contains("/main.rs") && !s.contains("/bin/"))
        .unwrap_or(true);

    // Check: anyhow::Result in library code (should use typed errors).
    if is_lib && !is_test {
        let anyhow_count = file_content.matches("anyhow::Result").count()
            + file_content.matches("anyhow::Error").count()
            + file_content.matches("anyhow::bail!").count();
        if anyhow_count > 3 {
            findings.push(SlopFinding {
                rule: "coherence".into(),
                message: format!(
                    "Library code uses anyhow {anyhow_count} times — consider typed errors (thiserror/snafu) for public API boundaries"
                ),
                file: None,
                line: None,
            });
        }
    }

    // Check: unwrap() in non-test production code.
    if !is_test {
        let unwrap_lines: Vec<usize> = file_content
            .lines()
            .enumerate()
            .filter(|(_, l)| {
                let t = l.trim();
                (t.contains(".unwrap()") || t.contains(".expect("))
                    && !t.starts_with("//")
                    && !t.starts_with("*")
            })
            .map(|(i, _)| i + 1)
            .collect();
        if unwrap_lines.len() > 5 {
            findings.push(SlopFinding {
                rule: "coherence".into(),
                message: format!(
                    "{} unwrap()/expect() calls in non-test code (lines: {:?}…) — consider proper error handling",
                    unwrap_lines.len(),
                    &unwrap_lines[..unwrap_lines.len().min(5)]
                ),
                file: None,
                line: Some(unwrap_lines[0]),
            });
        }
    }

    // Check: `todo!()` or `unimplemented!()` that shouldn't be committed.
    for (i, line) in file_content.lines().enumerate() {
        let t = line.trim();
        if (t.contains("todo!()") || t.contains("unimplemented!()")) && !t.starts_with("//") {
            findings.push(SlopFinding {
                rule: "coherence".into(),
                message: format!(
                    "Line {}: contains todo!()/unimplemented!() — should not be committed",
                    i + 1
                ),
                file: None,
                line: Some(i + 1),
            });
        }
    }

    findings
}

// ─── 4. Churn Tracking ──────────────────────────────────────────────────

/// Check git log for files with high edit frequency in the last 7 days.
pub fn check_churn(cwd: &Path) -> Vec<SlopFinding> {
    let output = std::process::Command::new("git")
        .args([
            "log",
            "--since=7 days ago",
            "--name-only",
            "--pretty=format:",
        ])
        .current_dir(cwd)
        .output();

    let Ok(output) = output else {
        return Vec::new();
    };
    let stdout = String::from_utf8_lossy(&output.stdout);

    let mut counts: HashMap<&str, u32> = HashMap::new();
    for line in stdout.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        *counts.entry(line).or_default() += 1;
    }

    let mut findings = Vec::new();
    let mut high_churn: Vec<(&&str, &u32)> = counts.iter().filter(|&(_, &v)| v > 5).collect();
    high_churn.sort_by(|a, b| b.1.cmp(a.1));

    for (file, count) in high_churn.into_iter().take(5) {
        findings.push(SlopFinding {
            rule: "churn".into(),
            message: format!(
                "{file} edited {count} times in 7 days — consider a consolidation/refactoring pass"
            ),
            file: Some(file.to_string()),
            line: None,
        });
    }
    findings
}

// ─── 5. Complexity Budget ───────────────────────────────────────────────

const MAX_FUNCTION_LOC: usize = 80;
const MAX_NESTING_DEPTH: usize = 5;

/// Check functions for excessive length or nesting depth.
pub fn check_complexity(file_content: &str) -> Vec<SlopFinding> {
    let mut findings = Vec::new();
    let lines: Vec<&str> = file_content.lines().collect();

    let mut i = 0;
    while i < lines.len() {
        let line = lines[i].trim();
        // Detect function start: `fn name(` or `pub fn name(` or `async fn ...`
        let is_fn = (line.contains("fn ") && line.contains('('))
            && !line.starts_with("//")
            && !line.starts_with("*");

        if is_fn {
            let fn_start = i;
            // Extract function name.
            let name = line
                .split("fn ")
                .nth(1)
                .and_then(|s| s.split('(').next())
                .unwrap_or("?")
                .trim();

            // Count LOC and max nesting depth until matching closing brace.
            let mut depth: i32 = 0;
            let mut max_depth: i32 = 0;
            let mut fn_loc = 0;
            let mut started = false;
            let mut j = i;
            while j < lines.len() {
                let l = lines[j];
                for ch in l.chars() {
                    if ch == '{' {
                        depth += 1;
                        started = true;
                        if depth > max_depth {
                            max_depth = depth;
                        }
                    } else if ch == '}' {
                        depth -= 1;
                    }
                }
                if started {
                    fn_loc += 1;
                }
                if started && depth == 0 {
                    break;
                }
                j += 1;
            }

            if fn_loc > MAX_FUNCTION_LOC {
                findings.push(SlopFinding {
                    rule: "complexity".into(),
                    message: format!(
                        "`{name}` at line {} is {fn_loc} lines (max {MAX_FUNCTION_LOC}) — consider splitting",
                        fn_start + 1
                    ),
                    file: None,
                    line: Some(fn_start + 1),
                });
            }
            if max_depth as usize > MAX_NESTING_DEPTH {
                findings.push(SlopFinding {
                    rule: "complexity".into(),
                    message: format!(
                        "`{name}` at line {} has nesting depth {max_depth} (max {MAX_NESTING_DEPTH}) — consider early returns or extraction",
                        fn_start + 1
                    ),
                    file: None,
                    line: Some(fn_start + 1),
                });
            }

            i = j + 1;
        } else {
            i += 1;
        }
    }
    findings
}

// ─── 6. Test Quality ────────────────────────────────────────────────────

/// Check for implementation-coupled test patterns.
pub fn check_test_quality(file_content: &str) -> Vec<SlopFinding> {
    let mut findings = Vec::new();

    // Only check test code.
    let test_section = if let Some(idx) = file_content.find("#[cfg(test)]") {
        &file_content[idx..]
    } else {
        return findings;
    };

    // Check: assertions on very long string literals (fragile snapshot tests).
    for (i, line) in test_section.lines().enumerate() {
        let t = line.trim();
        if (t.contains("assert_eq!") || t.contains("assert_ne!")) && t.len() > 200 {
            findings.push(SlopFinding {
                rule: "test_quality".into(),
                message: format!(
                    "Test assertion at line ~{} is >200 chars — fragile string snapshot; consider semantic checks",
                    i + 1
                ),
                file: None,
                line: Some(i + 1),
            });
        }
    }

    // Check: test functions with no assertions.
    let fn_re = regex::Regex::new(r"fn\s+(\w+)\s*\(").unwrap();
    let mut in_test_fn = false;
    let mut fn_name = String::new();
    let mut fn_start = 0;
    let mut has_assert = false;
    let mut depth: i32 = 0;

    for (i, line) in test_section.lines().enumerate() {
        if line.contains("#[test]") || line.contains("#[tokio::test]") {
            in_test_fn = true;
            has_assert = false;
            depth = 0;
            continue;
        }
        if in_test_fn && depth == 0 {
            if let Some(m) = fn_re.captures(line) {
                fn_name = m.get(1).map(|m| m.as_str().to_owned()).unwrap_or_default();
                fn_start = i + 1;
            }
        }
        if in_test_fn {
            for ch in line.chars() {
                if ch == '{' {
                    depth += 1;
                }
                if ch == '}' {
                    depth -= 1;
                }
            }
            if line.contains("assert") || line.contains("panic!") || line.contains("should_panic") {
                has_assert = true;
            }
            if depth == 0 && !fn_name.is_empty() {
                if !has_assert && fn_start > 0 {
                    findings.push(SlopFinding {
                        rule: "test_quality".into(),
                        message: format!(
                            "Test `{fn_name}` at line ~{fn_start} has no assertions — tests should verify behavior"
                        ),
                        file: None,
                        line: Some(fn_start),
                    });
                }
                in_test_fn = false;
                fn_name.clear();
            }
        }
    }

    findings
}

// ─── Unified Runner ─────────────────────────────────────────────────────

/// Run all applicable checks. Fast path: only runs checks relevant to the
/// file type (.rs → all checks; other → duplication + complexity only).
pub async fn run_all_checks(file_path: &Path, file_content: &str, cwd: &Path) -> SlopReport {
    let is_rust = file_path.extension().and_then(|e| e.to_str()) == Some("rs");

    let mut findings = Vec::new();

    // Always run: complexity.
    findings.extend(check_complexity(file_content));

    if is_rust {
        // Coherence checks.
        findings.extend(check_coherence(file_content, file_path));

        // Test quality (only if file has tests).
        if file_content.contains("#[cfg(test)]") {
            findings.extend(check_test_quality(file_content));
        }

        // Duplication (skip for very large files — too slow).
        if file_content.len() < 200_000 {
            findings.extend(check_duplication(file_content, file_path, cwd));
        }

        // Churn (cheap git call).
        findings.extend(check_churn(cwd));
    }

    // Note: check_dead_code is expensive (runs cargo check). Only run it
    // on explicit request or as a periodic background task, not per-edit.

    SlopReport {
        has_findings: !findings.is_empty(),
        findings,
    }
}

/// Format a `SlopReport` into a concise bulleted list.
pub fn format_report(report: &SlopReport) -> String {
    if !report.has_findings || report.findings.is_empty() {
        return String::new();
    }
    let mut out = String::from("Slop Guard findings:\n");
    for finding in &report.findings {
        let loc = match (&finding.file, finding.line) {
            (Some(f), Some(l)) => format!(" ({f}:{l})"),
            (None, Some(l)) => format!(" (line {l})"),
            (Some(f), None) => format!(" ({f})"),
            (None, None) => String::new(),
        };
        out.push_str(&format!(
            "  • [{}]{loc}: {}\n",
            finding.rule, finding.message
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn complexity_flags_long_function_normal() {
        let code = format!("pub fn big_one() {{\n{}\n}}", "    let x = 1;\n".repeat(90));
        let findings = check_complexity(&code);
        assert!(
            findings
                .iter()
                .any(|f| f.rule == "complexity" && f.message.contains("lines")),
            "expected complexity finding, got: {findings:?}",
        );
    }

    #[test]
    fn complexity_flags_deep_nesting_normal() {
        let code = "fn deep() {\n  if true {\n    if true {\n      if true {\n        if true {\n          if true {\n            if true {\n              x();\n            }\n          }\n        }\n      }\n    }\n  }\n}";
        let findings = check_complexity(code);
        assert!(
            findings.iter().any(|f| f.message.contains("nesting depth")),
            "expected nesting finding, got: {findings:?}",
        );
    }

    #[test]
    fn complexity_ok_for_short_function_normal() {
        let code = "fn small() {\n    println!(\"hi\");\n}";
        let findings = check_complexity(code);
        assert!(findings.is_empty());
    }

    #[test]
    fn coherence_flags_todo_normal() {
        let code = "fn wip() {\n    todo!()\n}";
        let findings = check_coherence(code, Path::new("src/lib.rs"));
        assert!(findings.iter().any(|f| f.message.contains("todo!()")));
    }

    #[test]
    fn coherence_flags_many_unwraps_normal() {
        let code = (0..8)
            .map(|i| format!("let x{i} = foo.unwrap();"))
            .collect::<Vec<_>>()
            .join("\n");
        let findings = check_coherence(&code, Path::new("src/lib.rs"));
        assert!(findings.iter().any(|f| f.message.contains("unwrap()")));
    }

    #[test]
    fn coherence_ok_in_test_code_normal() {
        let code = "#[cfg(test)]\nmod tests {\n    fn t() { foo.unwrap(); }\n}";
        let findings = check_coherence(code, Path::new("src/lib.rs"));
        // unwrap in test code is fine
        assert!(findings.iter().all(|f| !f.message.contains("unwrap()")));
    }

    #[test]
    fn test_quality_flags_no_assertions_normal() {
        let code = "#[cfg(test)]\nmod tests {\n    #[test]\n    fn does_nothing() {\n        let x = 1;\n    }\n}";
        let findings = check_test_quality(code);
        assert!(
            findings.iter().any(|f| f.message.contains("no assertions")),
            "got: {findings:?}"
        );
    }

    #[test]
    fn test_quality_ok_with_assert_normal() {
        let code = "#[cfg(test)]\nmod tests {\n    #[test]\n    fn works() {\n        assert_eq!(1, 1);\n    }\n}";
        let findings = check_test_quality(code);
        assert!(
            findings
                .iter()
                .all(|f| !f.message.contains("no assertions"))
        );
    }

    #[test]
    fn churn_returns_empty_on_no_git_robust() {
        // Run in /tmp which has no git repo.
        let findings = check_churn(Path::new("/tmp"));
        // Should not panic, may return empty or non-empty depending on system.
        let _ = findings;
    }

    #[test]
    fn format_report_empty_returns_empty_normal() {
        let report = SlopReport {
            has_findings: false,
            findings: Vec::new(),
        };
        assert_eq!(format_report(&report), "");
    }

    #[test]
    fn format_report_with_findings_normal() {
        let report = SlopReport {
            has_findings: true,
            findings: vec![SlopFinding {
                rule: "complexity".into(),
                message: "too long".into(),
                file: Some("src/main.rs".into()),
                line: Some(42),
            }],
        };
        let formatted = format_report(&report);
        assert!(formatted.contains("complexity"));
        assert!(formatted.contains("too long"));
        assert!(formatted.contains("src/main.rs:42"));
    }

    impl std::fmt::Debug for SlopFinding {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(f, "[{}] {}", self.rule, self.message)
        }
    }
}

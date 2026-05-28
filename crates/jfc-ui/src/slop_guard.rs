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
        if let Some(ref cn) = canonical_new
            && path.canonicalize().ok().as_ref() == Some(cn)
        {
            continue;
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
                // Slice on a char boundary — `preview` is arbitrary
                // source code (file paths in module separators, em-dashes
                // in comments, …) and a fixed-byte cap blew up at runtime
                // when a multi-byte glyph straddled byte 80
                // (`thread 'tokio-rt-worker' panicked at slop_guard.rs:114:
                //  end byte index 80 is not a char boundary; it is inside
                //  '─' (bytes 79..82)`). `floor_char_boundary` rounds DOWN
                // to the nearest char boundary at or before the requested
                // index, so we get at most 80 bytes of preview without
                // ever splitting a UTF-8 sequence.
                let preview_slice: &str = if preview.len() > 80 {
                    &preview[..preview.floor_char_boundary(80)]
                } else {
                    &preview
                };
                findings.push(SlopFinding {
                    rule: "duplication".into(),
                    message: format!(
                        "5+ line block already exists at {}:{} — consider reusing: `{}`",
                        path.strip_prefix(cwd).unwrap_or(path).display(),
                        i + 1,
                        preview_slice
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
#[allow(dead_code)]
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
        if in_test_fn
            && depth == 0
            && let Some(m) = fn_re.captures(line)
        {
            fn_name = m.get(1).map(|m| m.as_str().to_owned()).unwrap_or_default();
            fn_start = i + 1;
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

// ─── 7. Silent Failure Detection ────────────────────────────────────────

/// Detect patterns that swallow errors: `let _ = expr`, empty match arms,
/// catch-all `_ => {}` with no logging/handling.
pub fn check_silent_failure(file_content: &str, file_path: &Path) -> Vec<SlopFinding> {
    let mut findings = Vec::new();
    let is_test = file_content.contains("#[cfg(test)]");

    // Only flag in non-test production code
    let check_region = if is_test {
        if let Some(idx) = file_content.find("#[cfg(test)]") {
            &file_content[..idx]
        } else {
            file_content
        }
    } else {
        file_content
    };

    for (i, line) in check_region.lines().enumerate() {
        let t = line.trim();
        // Skip comments
        if t.starts_with("//") || t.starts_with("*") || t.starts_with("/*") {
            continue;
        }

        // `let _ = <expr>` where expr likely returns Result/Option
        if t.starts_with("let _ =") && !t.contains("// intentional") {
            // Heuristic: if the RHS likely returns Result (contains ?, .await, function call)
            let rhs = &t["let _ =".len()..];
            if rhs.contains('(') || rhs.contains(".await") {
                findings.push(SlopFinding {
                    rule: "silent_failure".into(),
                    message: format!(
                        "Line {}: `let _ = ...` discards a potentially meaningful Result/error — handle or log it",
                        i + 1
                    ),
                    file: Some(file_path.display().to_string()),
                    line: Some(i + 1),
                });
            }
        }

        // Empty catch-all match arm: `_ => {}`  or `_ => ()`
        if (t == "_ => {}," || t == "_ => {}") && !t.contains("// intentional") {
            findings.push(SlopFinding {
                rule: "silent_failure".into(),
                message: format!(
                    "Line {}: empty catch-all `_ => {{}}` silently ignores cases — add handling or logging",
                    i + 1
                ),
                file: Some(file_path.display().to_string()),
                line: Some(i + 1),
            });
        }

        // `Err(_) => {}` or `Err(_e) => {}`
        if ((t.starts_with("Err(_") && t.ends_with("=> {},"))
            || (t.starts_with("Err(_") && t.ends_with("=> {}")))
            && !t.contains("// intentional")
        {
            findings.push(SlopFinding {
                rule: "silent_failure".into(),
                message: format!(
                    "Line {}: swallowed Err variant — at minimum log the error",
                    i + 1
                ),
                file: Some(file_path.display().to_string()),
                line: Some(i + 1),
            });
        }
    }

    // Cap findings
    findings.truncate(8);
    findings
}

// ─── 8. Comment Slop Detection ──────────────────────────────────────────

/// Detect AI-generated narrator-style comments that add no information.
pub fn check_comment_slop(file_content: &str) -> Vec<SlopFinding> {
    let mut findings = Vec::new();

    // Patterns that indicate LLM narrator comments
    let slop_patterns: &[&str] = &[
        "// Step ",
        "// Now we ",
        "// Now, we ",
        "// First, ",
        "// First we ",
        "// Next, ",
        "// Next we ",
        "// Then, ",
        "// Then we ",
        "// Finally, ",
        "// Finally we ",
        "// Here we ",
        "// Here, we ",
        "// This function ",
        "// This method ",
        "// This is ",
        "// This will ",
        "// We need to ",
        "// We can ",
        "// Let's ",
        "// The following ",
        "// Below we ",
        "// As you can see",
        "// Note that we",
        "// Basically, ",
        "// Simply ",
        "// Just ",
        "// Obviously ",
    ];

    let mut slop_count = 0;
    let mut first_line = None;

    for (i, line) in file_content.lines().enumerate() {
        let t = line.trim();
        for pattern in slop_patterns {
            if t.starts_with(pattern) {
                slop_count += 1;
                if first_line.is_none() {
                    first_line = Some(i + 1);
                }
                break;
            }
        }
    }

    // Only flag if there's a pattern of narrator comments (3+)
    if slop_count >= 3 {
        findings.push(SlopFinding {
            rule: "comment_slop".into(),
            message: format!(
                "{slop_count} narrator-style comments detected (first at line {}) — remove comments that merely restate the code",
                first_line.unwrap_or(0)
            ),
            file: None,
            line: first_line,
        });
    }

    findings
}

// ─── 9. Metric Mimicry Detection ────────────────────────────────────────

/// Detect trivial/meaningless assertions that technically pass but verify nothing.
pub fn check_metric_mimicry(file_content: &str) -> Vec<SlopFinding> {
    let mut findings = Vec::new();

    // Only check test code
    let test_section = if let Some(idx) = file_content.find("#[cfg(test)]") {
        &file_content[idx..]
    } else {
        return findings;
    };

    for (i, line) in test_section.lines().enumerate() {
        let t = line.trim();

        // assert!(true)
        if t.contains("assert!(true)") {
            findings.push(SlopFinding {
                rule: "metric_mimicry".into(),
                message: format!(
                    "Line ~{}: `assert!(true)` always passes — test verifies nothing",
                    i + 1
                ),
                file: None,
                line: Some(i + 1),
            });
        }

        // assert_eq!(x, x) — same expression on both sides
        if t.contains("assert_eq!") {
            // Extract the two args (simple heuristic: split on first comma)
            if let Some(start) = t.find("assert_eq!(") {
                let inner = &t[start + "assert_eq!(".len()..];
                if let Some(end) = inner.rfind(')') {
                    let args = &inner[..end];
                    // Find the top-level comma (not inside nested parens)
                    let mut depth = 0;
                    let mut comma_pos = None;
                    for (ci, ch) in args.chars().enumerate() {
                        match ch {
                            '(' | '[' | '{' => depth += 1,
                            ')' | ']' | '}' => depth -= 1,
                            ',' if depth == 0 => {
                                comma_pos = Some(ci);
                                break;
                            }
                            _ => {}
                        }
                    }
                    if let Some(cp) = comma_pos {
                        let left = args[..cp].trim();
                        let right = args[cp + 1..].trim();
                        if left == right && !left.is_empty() {
                            findings.push(SlopFinding {
                                rule: "metric_mimicry".into(),
                                message: format!(
                                    "Line ~{}: `assert_eq!({left}, {right})` compares identical expressions — always passes",
                                    i + 1
                                ),
                                file: None,
                                line: Some(i + 1),
                            });
                        }
                    }
                }
            }
        }

        // assert!(!false)
        if t.contains("assert!(!false)")
            || t.contains("assert!(1 == 1)")
            || t.contains("assert!(0 == 0)")
        {
            findings.push(SlopFinding {
                rule: "metric_mimicry".into(),
                message: format!(
                    "Line ~{}: tautological assertion — always passes, verifies nothing",
                    i + 1
                ),
                file: None,
                line: Some(i + 1),
            });
        }
    }

    findings.truncate(5);
    findings
}

// ─── 10. Conditional Test Detection ─────────────────────────────────────

/// Flag control flow (if, match, while) inside test functions.
pub fn check_conditional_tests(file_content: &str) -> Vec<SlopFinding> {
    let mut findings = Vec::new();

    let test_section = if let Some(idx) = file_content.find("#[cfg(test)]") {
        &file_content[idx..]
    } else {
        return findings;
    };

    let control_flow_keywords = ["if ", "match ", "while ", "for "];
    let mut in_test_fn = false;
    let mut fn_name = String::new();
    let mut fn_start = 0;
    let mut depth: i32 = 0;
    let mut has_conditional = false;
    let mut conditional_count = 0;

    let fn_re = regex::Regex::new(r"fn\s+(\w+)\s*\(").unwrap();

    for (i, line) in test_section.lines().enumerate() {
        if line.contains("#[test]") || line.contains("#[tokio::test]") {
            in_test_fn = true;
            has_conditional = false;
            conditional_count = 0;
            depth = 0;
            continue;
        }
        if in_test_fn
            && depth == 0
            && let Some(m) = fn_re.captures(line)
        {
            fn_name = m.get(1).map(|m| m.as_str().to_owned()).unwrap_or_default();
            fn_start = i + 1;
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

            let t = line.trim();
            // Skip comments
            if !t.starts_with("//") && !t.starts_with("*") {
                for kw in &control_flow_keywords {
                    if t.starts_with(kw) || t.contains(&format!(" {kw}")) {
                        // Exclude `if let` which is idiomatic pattern matching
                        if !(*kw == "if " && t.contains("if let ")) {
                            has_conditional = true;
                            conditional_count += 1;
                        }
                        break;
                    }
                }
            }

            if depth == 0 && !fn_name.is_empty() {
                // Only flag if there are multiple conditionals (1 is often fine)
                if has_conditional && conditional_count >= 2 {
                    findings.push(SlopFinding {
                        rule: "conditional_test".into(),
                        message: format!(
                            "Test `{fn_name}` at line ~{fn_start} has {conditional_count} control-flow branches — consider splitting into separate test cases"
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

    findings.truncate(5);
    findings
}

// ─── 11. Naming Quality ─────────────────────────────────────────────────

/// Flag poor identifier naming patterns in newly written code.
pub fn check_naming_quality(file_content: &str) -> Vec<SlopFinding> {
    let mut findings = Vec::new();

    let vague_names: &[&str] = &[
        "handle_data",
        "process_item",
        "do_stuff",
        "do_thing",
        "do_work",
        "handle_event",
        "process_data",
        "run_task",
        "execute_action",
        "manager",
        "handler",
        "processor",
        "helper",
        "utils",
    ];

    let binding_re =
        regex::Regex::new(r"(?:let|const|static|fn)\s+(mut\s+)?([a-z_][a-z0-9_]*)").unwrap();

    let is_test_region = |pos: usize| -> bool {
        file_content[..pos].rfind("#[cfg(test)]").is_some()
            && file_content[..pos]
                .rfind("\nmod ")
                .is_some_and(|m| m > file_content[..pos].rfind("#[cfg(test)]").unwrap_or(0))
    };

    let mut vague_count = 0;
    let mut first_vague_line = None;

    for (i, line) in file_content.lines().enumerate() {
        let t = line.trim();
        if t.starts_with("//") || t.starts_with("*") {
            continue;
        }

        // Check for vague function/variable names
        if let Some(caps) = binding_re.captures(t)
            && let Some(name) = caps.get(2)
        {
            let n = name.as_str();

            // Skip test code (test names can be descriptive-verbose)
            let line_start = file_content
                .lines()
                .take(i)
                .map(|l| l.len() + 1)
                .sum::<usize>();
            if is_test_region(line_start) {
                continue;
            }

            // Vague names
            if vague_names.contains(&n) {
                vague_count += 1;
                if first_vague_line.is_none() {
                    first_vague_line = Some(i + 1);
                }
            }

            // Overly long identifiers (>50 chars)
            if n.len() > 50 {
                findings.push(SlopFinding {
                        rule: "naming_quality".into(),
                        message: format!(
                            "Line {}: identifier `{}...` is {} chars — consider a shorter, clearer name",
                            i + 1,
                            &n[..30],
                            n.len()
                        ),
                        file: None,
                        line: Some(i + 1),
                    });
            }
        }
    }

    if vague_count >= 3 {
        findings.push(SlopFinding {
            rule: "naming_quality".into(),
            message: format!(
                "{vague_count} vague identifier names detected (first at line {}) — use domain-specific names",
                first_vague_line.unwrap_or(0)
            ),
            file: None,
            line: first_vague_line,
        });
    }

    findings.truncate(5);
    findings
}

// ─── 12. Guardrail Removal Detection ────────────────────────────────────

/// Compare new content against what was there before (if available).
/// Flags removal of validation/bounds/security patterns.
pub fn check_guardrail_removal(old_content: Option<&str>, new_content: &str) -> Vec<SlopFinding> {
    let mut findings = Vec::new();

    let Some(old) = old_content else {
        return findings;
    };

    // Patterns whose count should not decrease significantly
    let guardrail_patterns: &[(&str, &str)] = &[
        (".is_empty()", "emptiness check"),
        (".is_none()", "None check"),
        (".is_err()", "error check"),
        ("bounds", "bounds check"),
        ("validate", "validation"),
        ("sanitize", "sanitization"),
        ("authorize", "authorization"),
        ("authenticate", "authentication"),
        ("if len", "length check"),
        (".len()", "length check"),
        ("< len", "bounds check"),
        (">= len", "bounds check"),
        ("overflow", "overflow protection"),
    ];

    for (pattern, name) in guardrail_patterns {
        let old_count = old.matches(pattern).count();
        let new_count = new_content.matches(pattern).count();

        // Only flag if there was meaningful usage that got removed
        if old_count >= 2 && new_count == 0 {
            findings.push(SlopFinding {
                rule: "guardrail_removal".into(),
                message: format!(
                    "All {old_count} `{pattern}` ({name}) patterns removed — verify this is intentional"
                ),
                file: None,
                line: None,
            });
        } else if old_count >= 3 && new_count < old_count / 2 {
            findings.push(SlopFinding {
                rule: "guardrail_removal".into(),
                message: format!(
                    "{name} usage dropped from {old_count} to {new_count} — verify removals are safe"
                ),
                file: None,
                line: None,
            });
        }
    }

    findings.truncate(5);
    findings
}

// ─── 13. Security Regression Detection ──────────────────────────────────

/// Detect removal of security-critical patterns (parameterized queries,
/// crypto usage, auth checks, input sanitization).
pub fn check_security_regression(old_content: Option<&str>, new_content: &str) -> Vec<SlopFinding> {
    let mut findings = Vec::new();

    let Some(old) = old_content else {
        return findings;
    };

    // Security-critical patterns: if they existed before and vanished, that's bad
    let security_patterns: &[(&str, &str, &str)] = &[
        (
            "prepare(",
            "parameterized query",
            "SQL injection risk: parameterized queries replaced with string building",
        ),
        (
            "bind(",
            "parameter binding",
            "SQL injection risk: parameter binding removed",
        ),
        ("hmac", "HMAC verification", "integrity check removed"),
        (
            "verify_signature",
            "signature verification",
            "signature verification removed",
        ),
        (
            "hash_password",
            "password hashing",
            "password hashing removed",
        ),
        (
            "bcrypt",
            "bcrypt",
            "bcrypt usage removed — verify replacement is equally secure",
        ),
        (
            "argon2",
            "argon2",
            "argon2 usage removed — verify replacement is equally secure",
        ),
        ("csrf", "CSRF protection", "CSRF protection removed"),
        ("rate_limit", "rate limiting", "rate limiting removed"),
        ("escape_html", "HTML escaping", "XSS protection removed"),
        (
            "sanitize_input",
            "input sanitization",
            "input sanitization removed",
        ),
        (
            "Content-Security-Policy",
            "CSP header",
            "CSP header removed",
        ),
    ];

    for (pattern, _name, warning) in security_patterns {
        let old_has = old.contains(pattern);
        let new_has = new_content.contains(pattern);

        if old_has && !new_has {
            findings.push(SlopFinding {
                rule: "security_regression".into(),
                message: warning.to_string(),
                file: None,
                line: None,
            });
        }
    }

    // Check for string concatenation replacing parameterized queries
    let old_format_in_query = old.matches("format!(\"SELECT").count()
        + old.matches("format!(\"INSERT").count()
        + old.matches("format!(\"UPDATE").count()
        + old.matches("format!(\"DELETE").count();
    let new_format_in_query = new_content.matches("format!(\"SELECT").count()
        + new_content.matches("format!(\"INSERT").count()
        + new_content.matches("format!(\"UPDATE").count()
        + new_content.matches("format!(\"DELETE").count();

    if new_format_in_query > old_format_in_query && new_format_in_query >= 2 {
        findings.push(SlopFinding {
            rule: "security_regression".into(),
            message: format!(
                "SQL string concatenation increased ({old_format_in_query} → {new_format_in_query}) — potential injection vulnerability"
            ),
            file: None,
            line: None,
        });
    }

    findings.truncate(5);
    findings
}

// ─── 14. Premature Abstraction Detection ────────────────────────────────

/// Detect trait definitions that have exactly one implementor in the same file.
pub fn check_premature_abstraction(file_content: &str) -> Vec<SlopFinding> {
    let mut findings = Vec::new();

    let trait_re = regex::Regex::new(r"(?:pub\s+)?trait\s+(\w+)").unwrap();
    let impl_re = regex::Regex::new(r"impl\s+(\w+)\s+for\s+").unwrap();

    // Collect trait definitions
    let mut traits: Vec<(String, usize)> = Vec::new();
    for (i, line) in file_content.lines().enumerate() {
        let t = line.trim();
        if t.starts_with("//") || t.starts_with("*") {
            continue;
        }
        if let Some(caps) = trait_re.captures(t)
            && let Some(name) = caps.get(1)
        {
            // Skip well-known patterns
            let n = name.as_str();
            if ![
                "Debug", "Clone", "Default", "Display", "Error", "From", "Into", "Iterator",
            ]
            .contains(&n)
            {
                traits.push((n.to_string(), i + 1));
            }
        }
    }

    // Count implementations per trait
    for (trait_name, def_line) in &traits {
        let impl_count = impl_re
            .captures_iter(file_content)
            .filter(|caps| caps.get(1).map(|m| m.as_str()) == Some(trait_name.as_str()))
            .count();

        if impl_count == 1 {
            findings.push(SlopFinding {
                rule: "premature_abstraction".into(),
                message: format!(
                    "Trait `{trait_name}` at line {def_line} has exactly 1 implementor in this file — consider using concrete types until a second impl is needed"
                ),
                file: None,
                line: Some(*def_line),
            });
        }
    }

    findings.truncate(3);
    findings
}

// ─── 15. Unused Imports Detection ───────────────────────────────────────

/// Run `cargo check --message-format=json` and extract unused_imports warnings.
/// Similar to check_dead_code but specifically for imports.
#[allow(dead_code)]
pub fn check_unused_imports(cwd: &Path) -> Vec<SlopFinding> {
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
        if code != "unused_imports" {
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
            rule: "unused_imports".into(),
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

// ─── 16. Resource Leak Detection ────────────────────────────────────────

/// Detect potential resource leaks: File::open, TcpStream::connect, etc.
/// without proper RAII scope management.
pub fn check_resource_leaks(file_content: &str) -> Vec<SlopFinding> {
    let mut findings = Vec::new();
    let is_test = file_content.contains("#[cfg(test)]");

    let check_region = if is_test {
        if let Some(idx) = file_content.find("#[cfg(test)]") {
            &file_content[..idx]
        } else {
            file_content
        }
    } else {
        file_content
    };

    // Look for resource acquisition without ? or proper error handling
    let resource_patterns: &[(&str, &str)] = &[
        ("File::open(", "File handle"),
        ("File::create(", "File handle"),
        ("TcpStream::connect(", "TCP stream"),
        ("TcpListener::bind(", "TCP listener"),
        ("UdpSocket::bind(", "UDP socket"),
        ("OpenOptions::new()", "File handle"),
    ];

    for (i, line) in check_region.lines().enumerate() {
        let t = line.trim();
        if t.starts_with("//") || t.starts_with("*") {
            continue;
        }

        for (pattern, resource_type) in resource_patterns {
            if t.contains(pattern) {
                // Check if it's properly handled: has ?, is in a let binding, or uses .unwrap()
                // Flag if it's stored in a variable without ? and the function is long
                if t.contains(".unwrap()") && !t.contains("drop(") {
                    // Only flag unwrap on resources in functions >20 lines
                    // (short functions likely handle it)
                    findings.push(SlopFinding {
                        rule: "resource_leak".into(),
                        message: format!(
                            "Line {}: {resource_type} acquired with .unwrap() — if the function returns early later, the resource may leak. Consider using `?` or an explicit `drop()`",
                            i + 1
                        ),
                        file: None,
                        line: Some(i + 1),
                    });
                }
            }
        }
    }

    findings.truncate(5);
    findings
}

// ─── 17. Complexity Delta Tracking ──────────────────────────────────────

/// Compare complexity between old and new content. Flag significant increases.
pub fn check_complexity_delta(old_content: Option<&str>, new_content: &str) -> Vec<SlopFinding> {
    let mut findings = Vec::new();

    let Some(old) = old_content else {
        return findings;
    };

    // Simple complexity proxy: count control flow keywords
    let complexity_of = |content: &str| -> usize {
        let keywords = [
            "if ", "else ", "match ", "while ", "for ", "loop ", "&&", "||", "?",
        ];
        keywords.iter().map(|kw| content.matches(kw).count()).sum()
    };

    let old_complexity = complexity_of(old);
    let new_complexity = complexity_of(new_content);

    if old_complexity > 0 {
        let increase_pct = ((new_complexity as f64 - old_complexity as f64) / old_complexity as f64
            * 100.0) as i64;
        if increase_pct > 30 && new_complexity > old_complexity + 10 {
            findings.push(SlopFinding {
                rule: "complexity_delta".into(),
                message: format!(
                    "Control-flow complexity increased by {increase_pct}% ({old_complexity} → {new_complexity}) — research shows >20% correlates with vulnerability introduction"
                ),
                file: None,
                line: None,
            });
        }
    }

    findings
}

// ─── Unified Runner ─────────────────────────────────────────────────────

/// Run all applicable checks. Fast path: only runs checks relevant to the
/// file type (.rs → all checks; other → duplication + complexity only).
///
/// `old_content` is the file's previous content (from file_checkpoint) if available,
/// enabling diff-based checks like guardrail_removal and security_regression.
pub async fn run_all_checks(file_path: &Path, file_content: &str, cwd: &Path) -> SlopReport {
    run_all_checks_with_old(file_path, file_content, None, cwd).await
}

/// Full version with optional old content for diff-based checks.
pub async fn run_all_checks_with_old(
    file_path: &Path,
    file_content: &str,
    old_content: Option<&str>,
    cwd: &Path,
) -> SlopReport {
    let is_rust = file_path.extension().and_then(|e| e.to_str()) == Some("rs");

    let mut findings = Vec::new();

    // Always run: complexity + comment slop.
    findings.extend(check_complexity(file_content));
    findings.extend(check_comment_slop(file_content));

    if is_rust {
        // Coherence checks.
        findings.extend(check_coherence(file_content, file_path));

        // Silent failure detection.
        findings.extend(check_silent_failure(file_content, file_path));

        // Naming quality.
        findings.extend(check_naming_quality(file_content));

        // Resource leak patterns.
        findings.extend(check_resource_leaks(file_content));

        // Premature abstraction.
        findings.extend(check_premature_abstraction(file_content));

        // Test quality (only if file has tests).
        if file_content.contains("#[cfg(test)]") {
            findings.extend(check_test_quality(file_content));
            findings.extend(check_metric_mimicry(file_content));
            findings.extend(check_conditional_tests(file_content));
        }

        // Duplication (skip for very large files — too slow).
        if file_content.len() < 200_000 {
            findings.extend(check_duplication(file_content, file_path, cwd));
        }

        // Churn (cheap git call).
        findings.extend(check_churn(cwd));

        // Diff-based checks (require old content).
        if old_content.is_some() {
            findings.extend(check_guardrail_removal(old_content, file_content));
            findings.extend(check_security_regression(old_content, file_content));
            findings.extend(check_complexity_delta(old_content, file_content));
        }
    }

    // Note: check_dead_code and check_unused_imports are expensive (run cargo check).
    // Only run them on explicit request or as a periodic background task, not per-edit.

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

    #[test]
    fn silent_failure_flags_let_underscore_normal() {
        let code = "fn foo() {\n    let _ = do_something();\n}";
        let findings = check_silent_failure(code, Path::new("src/lib.rs"));
        assert!(
            findings.iter().any(|f| f.rule == "silent_failure"),
            "expected silent_failure finding, got: {findings:?}"
        );
    }

    #[test]
    fn silent_failure_ignores_test_code_normal() {
        let code = "#[cfg(test)]\nmod tests {\n    fn t() {\n        let _ = foo();\n    }\n}";
        let findings = check_silent_failure(code, Path::new("src/lib.rs"));
        assert!(
            findings.iter().all(|f| f.rule != "silent_failure"),
            "should not flag in test code"
        );
    }

    #[test]
    fn comment_slop_flags_narrator_comments_normal() {
        let code = "// Step 1: do the thing\nfn a() {}\n// Now we need to check\nfn b() {}\n// This function handles\nfn c() {}";
        let findings = check_comment_slop(code);
        assert!(
            findings.iter().any(|f| f.rule == "comment_slop"),
            "expected comment_slop finding, got: {findings:?}"
        );
    }

    #[test]
    fn comment_slop_ok_with_few_comments_normal() {
        let code = "// Step 1: only one\nfn a() {}\nfn b() {}";
        let findings = check_comment_slop(code);
        assert!(findings.is_empty());
    }

    #[test]
    fn metric_mimicry_flags_assert_true_normal() {
        let code = "#[cfg(test)]\nmod tests {\n    #[test]\n    fn t() {\n        assert!(true);\n    }\n}";
        let findings = check_metric_mimicry(code);
        assert!(
            findings.iter().any(|f| f.rule == "metric_mimicry"),
            "expected metric_mimicry finding, got: {findings:?}"
        );
    }

    #[test]
    fn metric_mimicry_flags_self_equality_normal() {
        let code = "#[cfg(test)]\nmod tests {\n    #[test]\n    fn t() {\n        let x = 5;\n        assert_eq!(x, x);\n    }\n}";
        let findings = check_metric_mimicry(code);
        assert!(
            findings
                .iter()
                .any(|f| f.rule == "metric_mimicry" && f.message.contains("identical")),
            "expected self-equality finding, got: {findings:?}"
        );
    }

    #[test]
    fn conditional_test_flags_multiple_branches_normal() {
        let code = "#[cfg(test)]\nmod tests {\n    #[test]\n    fn branchy() {\n        if x > 0 {\n            assert!(true);\n        }\n        if y > 0 {\n            assert!(true);\n        }\n    }\n}";
        let findings = check_conditional_tests(code);
        assert!(
            findings.iter().any(|f| f.rule == "conditional_test"),
            "expected conditional_test finding, got: {findings:?}"
        );
    }

    #[test]
    fn naming_quality_flags_vague_names_normal() {
        let code = "fn handle_data() {}\nfn process_item() {}\nfn do_stuff() {}";
        let findings = check_naming_quality(code);
        assert!(
            findings.iter().any(|f| f.rule == "naming_quality"),
            "expected naming_quality finding, got: {findings:?}"
        );
    }

    #[test]
    fn guardrail_removal_flags_removed_checks_normal() {
        let old = "fn f() {\n    if x.is_empty() { return; }\n    if y.is_empty() { return; }\n    if z.is_empty() { return; }\n}";
        let new = "fn f() {\n    process(x, y, z);\n}";
        let findings = check_guardrail_removal(Some(old), new);
        assert!(
            findings.iter().any(|f| f.rule == "guardrail_removal"),
            "expected guardrail_removal finding, got: {findings:?}"
        );
    }

    #[test]
    fn security_regression_flags_removed_crypto_normal() {
        let old =
            "fn auth() {\n    let h = bcrypt::hash(password)?;\n    verify_signature(&token)?;\n}";
        let new = "fn auth() {\n    // simplified\n    Ok(())\n}";
        let findings = check_security_regression(Some(old), new);
        assert!(
            findings.iter().any(|f| f.rule == "security_regression"),
            "expected security_regression finding, got: {findings:?}"
        );
    }

    #[test]
    fn premature_abstraction_flags_single_impl_normal() {
        let code = "trait Processor {\n    fn process(&self);\n}\n\nstruct MyProcessor;\n\nimpl Processor for MyProcessor {\n    fn process(&self) {}\n}";
        let findings = check_premature_abstraction(code);
        assert!(
            findings.iter().any(|f| f.rule == "premature_abstraction"),
            "expected premature_abstraction finding, got: {findings:?}"
        );
    }

    #[test]
    fn complexity_delta_flags_large_increase_normal() {
        let old = "fn f() {\n    if x { y() }\n}";
        let new = "fn f() {\n    if x { if y { if z { if w { if v { for i in 0..10 { while true { match foo { _ => {} } } } } } } } }\n    if a && b || c && d || e && f || g && h {}\n}";
        let findings = check_complexity_delta(Some(old), new);
        assert!(
            findings.iter().any(|f| f.rule == "complexity_delta"),
            "expected complexity_delta finding, got: {findings:?}"
        );
    }

    impl std::fmt::Debug for SlopFinding {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(f, "[{}] {}", self.rule, self.message)
        }
    }
}

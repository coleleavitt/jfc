//! Weighted scaffold / stub / shim detector.
//!
//! Replaces the old binary `STUB_PATTERNS` substring list with a weighted,
//! category-aware detector. The weights and vocabulary were derived from an
//! audit of the jfc session corpus (`~/.config/jfc/sessions/`), ranking the
//! incompleteness language the model actually emits when it leaves work
//! half-done (stub, fallback, placeholder, scaffold, no-op, deferred, …).
//!
//! Design goals from that audit:
//! - **Weighted, not flat reject**: a lone `// TODO` or a legitimate
//!   `let _ = tx.send(..)` should not block task completion, but a runtime
//!   `unimplemented!()` always should.
//! - **Case-insensitive** regex matching.
//! - **Context-aware**: distinguish code from comments and test files; `shim`
//!   is only suspicious when paired with stub/temporary/fake/no-op nearby
//!   ("compatibility shim" is legitimate in this codebase).

use std::path::Path;
use std::sync::LazyLock;

use regex::Regex;

/// Severity tier for a scaffold finding. The numeric weight feeds the
/// cumulative gate in the task-completion evaluator.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    /// Weak signal — shim, dropped, simplified. Only matters in aggregate.
    Info,
    /// Low signal — fallback, minimal, temporary, "for now".
    Low,
    /// Medium signal — scaffold, skeleton, no-op, deferred, incomplete.
    Medium,
    /// High signal — placeholder/stubbed/fake/hardcoded, "// TODO".
    High,
    /// Runtime-fatal — unimplemented!(), todo!(), panic!("not implemented").
    Critical,
}

impl Severity {
    pub fn weight(self) -> u32 {
        match self {
            Severity::Info => 15,
            Severity::Low => 35,
            Severity::Medium => 60,
            Severity::High => 80,
            Severity::Critical => 100,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Severity::Info => "info",
            Severity::Low => "low",
            Severity::Medium => "medium",
            Severity::High => "high",
            Severity::Critical => "critical",
        }
    }
}

/// Category groups related patterns for reporting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Category {
    /// Code that panics or is explicitly not wired at runtime.
    Unimplemented,
    /// Placeholder/dummy/fake/hardcoded behavior standing in for real logic.
    Placeholder,
    /// Structural scaffolding — skeletons, no-ops, deferred work.
    Scaffold,
    /// Soft hedges — fallback, minimal, temporary, workaround.
    Hedge,
    /// Compatibility shims and dropped/simplified handling.
    Shim,
}

impl Category {
    pub fn label(self) -> &'static str {
        match self {
            Category::Unimplemented => "unimplemented",
            Category::Placeholder => "placeholder",
            Category::Scaffold => "scaffold",
            Category::Hedge => "hedge",
            Category::Shim => "shim",
        }
    }
}

struct Pattern {
    re: Regex,
    severity: Severity,
    category: Category,
    /// When true, the pattern only counts on code lines (not comments/prose).
    code_only: bool,
    /// When true, the pattern needs a corroborating stub word on the same line
    /// to count (used for ambiguous words like `shim`).
    needs_corroboration: bool,
}

/// A single detected scaffold/stub indicator.
#[derive(Debug, Clone)]
pub struct ScaffoldFinding {
    pub line: usize,
    pub matched: String,
    pub severity: Severity,
    pub category: Category,
    pub context: String,
}

fn rx(pat: &str) -> Regex {
    Regex::new(pat).expect("scaffold_detector: invalid built-in regex")
}

static PATTERNS: LazyLock<Vec<Pattern>> = LazyLock::new(|| {
    vec![
        // ─── Critical (100): runtime-fatal / explicitly not wired ──────────
        Pattern {
            re: rx(r"\bunimplemented!\s*\("),
            severity: Severity::Critical,
            category: Category::Unimplemented,
            code_only: true,
            needs_corroboration: false,
        },
        Pattern {
            re: rx(r"\btodo!\s*\("),
            severity: Severity::Critical,
            category: Category::Unimplemented,
            code_only: true,
            needs_corroboration: false,
        },
        Pattern {
            re: rx(r#"panic!\s*\(\s*"not (yet )?implemented"#),
            severity: Severity::Critical,
            category: Category::Unimplemented,
            code_only: true,
            needs_corroboration: false,
        },
        Pattern {
            re: rx(r"(?i)\bnot wired\b"),
            severity: Severity::Critical,
            category: Category::Unimplemented,
            code_only: false,
            needs_corroboration: false,
        },
        Pattern {
            re: rx(r"(?i)\bleft unimplemented\b"),
            severity: Severity::Critical,
            category: Category::Unimplemented,
            code_only: false,
            needs_corroboration: false,
        },
        Pattern {
            re: rx(r"\bNotImplementedError\b"),
            severity: Severity::Critical,
            category: Category::Unimplemented,
            code_only: true,
            needs_corroboration: false,
        },
        // ─── High (80): placeholder / fake / hardcoded ─────────────────────
        Pattern {
            re: rx(r"(?i)\bplaceholder\b"),
            severity: Severity::High,
            category: Category::Placeholder,
            code_only: false,
            needs_corroboration: false,
        },
        Pattern {
            re: rx(r"(?i)\bstubbed out\b"),
            severity: Severity::High,
            category: Category::Placeholder,
            code_only: false,
            needs_corroboration: false,
        },
        Pattern {
            re: rx(r"(?i)\bdummy (impl|implementation|value|data|response)"),
            severity: Severity::High,
            category: Category::Placeholder,
            code_only: false,
            needs_corroboration: false,
        },
        Pattern {
            re: rx(r"(?i)\bfake (impl|implementation|runtime|response|data)"),
            severity: Severity::High,
            category: Category::Placeholder,
            code_only: false,
            needs_corroboration: false,
        },
        Pattern {
            re: rx(r"(?i)hard.?coded"),
            severity: Severity::High,
            category: Category::Placeholder,
            code_only: false,
            needs_corroboration: false,
        },
        Pattern {
            re: rx(r"//\s*(TODO|FIXME|STUB|HACK|XXX)\b"),
            severity: Severity::High,
            category: Category::Scaffold,
            code_only: false,
            needs_corroboration: false,
        },
        Pattern {
            re: rx(r"#\s*(TODO|FIXME|STUB|HACK|XXX)\b"),
            severity: Severity::High,
            category: Category::Scaffold,
            code_only: false,
            needs_corroboration: false,
        },
        Pattern {
            re: rx(r"/\*\s*(TODO|FIXME|STUB)\b"),
            severity: Severity::High,
            category: Category::Scaffold,
            code_only: false,
            needs_corroboration: false,
        },
        // ─── Medium (60): scaffold / skeleton / no-op / deferred ───────────
        Pattern {
            re: rx(r"(?i)\bscaffold(ed|ing)?\b"),
            severity: Severity::Medium,
            category: Category::Scaffold,
            code_only: false,
            needs_corroboration: false,
        },
        Pattern {
            re: rx(r"(?i)\bskeleton\b"),
            severity: Severity::Medium,
            category: Category::Scaffold,
            code_only: false,
            needs_corroboration: false,
        },
        Pattern {
            re: rx(r"(?i)\bno-?op\b"),
            severity: Severity::Medium,
            category: Category::Scaffold,
            code_only: false,
            needs_corroboration: false,
        },
        Pattern {
            re: rx(r"(?i)\bdeferred\b"),
            severity: Severity::Medium,
            category: Category::Scaffold,
            code_only: false,
            needs_corroboration: false,
        },
        Pattern {
            re: rx(r"(?i)\bfuture work\b"),
            severity: Severity::Medium,
            category: Category::Scaffold,
            code_only: false,
            needs_corroboration: false,
        },
        Pattern {
            re: rx(r"(?i)\bincomplete\b"),
            severity: Severity::Medium,
            category: Category::Scaffold,
            code_only: false,
            needs_corroboration: false,
        },
        Pattern {
            re: rx(r"(?i)\bnot (yet )?fully (implemented|wired|supported)"),
            severity: Severity::Medium,
            category: Category::Scaffold,
            code_only: false,
            needs_corroboration: false,
        },
        // ─── Low (35): fallback / minimal / temporary / for now ────────────
        Pattern {
            re: rx(r"(?i)\bminimal (impl|implementation|version|stub)"),
            severity: Severity::Low,
            category: Category::Hedge,
            code_only: false,
            needs_corroboration: false,
        },
        Pattern {
            re: rx(r"(?i)\btemporary (impl|implementation|hack|workaround|fix)"),
            severity: Severity::Low,
            category: Category::Hedge,
            code_only: false,
            needs_corroboration: false,
        },
        Pattern {
            re: rx(r"(?i)\bworkaround\b"),
            severity: Severity::Low,
            category: Category::Hedge,
            code_only: false,
            needs_corroboration: false,
        },
        Pattern {
            re: rx(r"(?i)\bfor now\b"),
            severity: Severity::Low,
            category: Category::Hedge,
            code_only: false,
            needs_corroboration: false,
        },
        // ─── Info (15): shim / dropped / simplified (need corroboration) ───
        Pattern {
            re: rx(r"(?i)\bshim\b"),
            severity: Severity::Info,
            category: Category::Shim,
            code_only: false,
            needs_corroboration: true,
        },
        Pattern {
            re: rx(r"(?i)\bsimplified\b"),
            severity: Severity::Info,
            category: Category::Shim,
            code_only: false,
            needs_corroboration: true,
        },
        Pattern {
            re: rx(r"(?i)\bbest.?effort\b"),
            severity: Severity::Info,
            category: Category::Shim,
            code_only: false,
            needs_corroboration: true,
        },
    ]
});

/// Words that corroborate an otherwise-ambiguous match (e.g. `shim`).
static CORROBORATING: LazyLock<Regex> = LazyLock::new(|| {
    rx(
        r"(?i)\b(stub|temporary|placeholder|not wired|fake|no-?op|todo|unimplemented|deferred|incomplete)\b",
    )
});

/// True if the trimmed line is a comment (Rust/TS/JS `//` `/*` `*`, Python/sh `#`).
fn is_comment_line(line: &str) -> bool {
    let t = line.trim_start();
    t.starts_with("//")
        || t.starts_with("/*")
        || t.starts_with('*')
        || t.starts_with("#")
        || t.starts_with("<!--")
}

/// Scan a block of text for scaffold/stub indicators.
///
/// `is_test` downgrades every finding by one severity tier (test fixtures
/// legitimately contain `dummy`, `mock`, `fake`, etc.).
pub fn scan_text(text: &str, is_test: bool) -> Vec<ScaffoldFinding> {
    let mut findings = Vec::new();
    for (idx, line) in text.lines().enumerate() {
        // Skip overly long lines (minified/generated) to avoid pathological regex cost.
        if line.len() > 4000 {
            continue;
        }
        let comment = is_comment_line(line);
        for p in PATTERNS.iter() {
            if p.code_only && comment {
                continue;
            }
            if let Some(m) = p.re.find(line) {
                if p.needs_corroboration && !CORROBORATING.is_match(line) {
                    continue;
                }
                let severity = if is_test {
                    downgrade(p.severity)
                } else {
                    p.severity
                };
                findings.push(ScaffoldFinding {
                    line: idx + 1,
                    matched: m.as_str().to_string(),
                    severity,
                    category: p.category,
                    context: line.trim().chars().take(160).collect(),
                });
            }
        }
    }
    findings
}

fn downgrade(s: Severity) -> Severity {
    match s {
        Severity::Critical => Severity::Medium,
        Severity::High => Severity::Low,
        Severity::Medium => Severity::Low,
        Severity::Low => Severity::Info,
        Severity::Info => Severity::Info,
    }
}

/// True if the path looks like test/fixture code.
pub fn is_test_path(path: &Path) -> bool {
    let s = path.to_string_lossy();
    s.contains("/tests/")
        || s.contains("/test/")
        || s.contains("/fixtures/")
        || s.ends_with("_test.rs")
        || s.ends_with("_tests.rs")
        || s.contains(".test.")
        || s.contains(".spec.")
        || s.ends_with("/tests.rs")
}

/// Scan a file on disk. Returns empty if unreadable or not a source file.
pub fn scan_file(path: &Path) -> Vec<ScaffoldFinding> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    scan_text(&text, is_test_path(path))
}

/// Cumulative weight of a finding set.
pub fn total_weight(findings: &[ScaffoldFinding]) -> u32 {
    findings.iter().map(|f| f.severity.weight()).sum()
}

/// Whether a finding set should block task completion.
///
/// Gate: any `Critical` finding (runtime-fatal stub) blocks immediately;
/// otherwise the cumulative weight must cross `threshold` (default 160 — e.g.
/// two High findings, or one High plus two Mediums). A single `// TODO` (80)
/// or a lone `let _ =` (now not a pattern at all) will not trip it.
pub fn should_block(findings: &[ScaffoldFinding], threshold: u32) -> bool {
    if findings.iter().any(|f| f.severity == Severity::Critical) {
        return true;
    }
    total_weight(findings) >= threshold
}

/// Default cumulative-weight threshold for the task-completion gate.
pub const DEFAULT_BLOCK_THRESHOLD: u32 = 160;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn critical_macros_score_100() {
        let f = scan_text("fn foo() { unimplemented!() }", false);
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].severity, Severity::Critical);
        assert_eq!(f[0].category, Category::Unimplemented);
    }

    #[test]
    fn todo_macro_is_critical() {
        let f = scan_text("    todo!(\"finish this\")", false);
        assert!(f.iter().any(|x| x.severity == Severity::Critical));
    }

    #[test]
    fn bare_let_underscore_is_not_flagged() {
        // The old detector false-positived on every fire-and-forget send.
        let f = scan_text("let _ = tx.send(event).await;", false);
        assert!(
            f.is_empty(),
            "let _ = should not be a scaffold pattern, got {f:?}"
        );
    }

    #[test]
    fn shim_alone_is_not_flagged() {
        // "compatibility shim" is legitimate in this codebase.
        let f = scan_text("// a thin compatibility shim over the old API", false);
        assert!(
            f.is_empty(),
            "bare shim should need corroboration, got {f:?}"
        );
    }

    #[test]
    fn shim_with_corroboration_is_flagged() {
        let f = scan_text("// temporary shim — stub until real impl lands", false);
        assert!(f.iter().any(|x| x.category == Category::Shim));
    }

    #[test]
    fn placeholder_is_high() {
        let f = scan_text("// placeholder implementation, returns empty", false);
        assert!(f.iter().any(|x| x.severity == Severity::High));
    }

    #[test]
    fn test_files_downgrade_severity() {
        let prod = scan_text("let x = unimplemented!();", false);
        let test = scan_text("let x = unimplemented!();", true);
        assert_eq!(prod[0].severity, Severity::Critical);
        assert_eq!(test[0].severity, Severity::Medium);
    }

    #[test]
    fn single_todo_does_not_block() {
        let f = scan_text("// TODO: revisit caching later", false);
        assert!(!should_block(&f, DEFAULT_BLOCK_THRESHOLD));
    }

    #[test]
    fn critical_always_blocks() {
        let f = scan_text("fn x() { todo!() }", false);
        assert!(should_block(&f, DEFAULT_BLOCK_THRESHOLD));
    }

    #[test]
    fn multiple_high_findings_block() {
        let text = "// placeholder value\nlet y = hardcoded_token();\n// TODO wire this";
        let f = scan_text(text, false);
        assert!(
            should_block(&f, DEFAULT_BLOCK_THRESHOLD),
            "weight={}",
            total_weight(&f)
        );
    }

    #[test]
    fn code_only_pattern_skips_comments() {
        // unimplemented!() mentioned in a comment is not runtime-fatal.
        let f = scan_text("// we used to call unimplemented!() here", false);
        assert!(
            !f.iter()
                .any(|x| x.category == Category::Unimplemented && x.severity == Severity::Critical),
            "comment mention of unimplemented!() should not be critical: {f:?}"
        );
    }

    #[test]
    fn is_test_path_detects_common_layouts() {
        assert!(is_test_path(Path::new("crates/foo/tests/bar.rs")));
        assert!(is_test_path(Path::new("src/foo_test.rs")));
        assert!(is_test_path(Path::new("src/app.spec.ts")));
        assert!(!is_test_path(Path::new("src/app/state.rs")));
    }
}

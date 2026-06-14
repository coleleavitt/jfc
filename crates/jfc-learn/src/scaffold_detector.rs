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
    /// Leftover debug output — dbg!(), console.log, eprintln debugging.
    Debug,
    /// Work the model explicitly punted — "out of scope", "revisit later".
    Punted,
}

impl Category {
    pub fn label(self) -> &'static str {
        match self {
            Category::Unimplemented => "unimplemented",
            Category::Placeholder => "placeholder",
            Category::Scaffold => "scaffold",
            Category::Hedge => "hedge",
            Category::Shim => "shim",
            Category::Debug => "debug",
            Category::Punted => "punted",
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
        Pattern {
            re: rx(r"\braise NotImplementedError\b"),
            severity: Severity::Critical,
            category: Category::Unimplemented,
            code_only: true,
            needs_corroboration: false,
        },
        Pattern {
            // JS/TS: throw new Error("not implemented")
            re: rx(r#"(?i)throw new Error\(\s*["'][^"']*(not impl|unimplement|todo)"#),
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
        Pattern {
            re: rx(r"\bWIP\b"),
            severity: Severity::High,
            category: Category::Scaffold,
            code_only: false,
            needs_corroboration: false,
        },
        Pattern {
            re: rx(r"(?i)work in progress\b"),
            severity: Severity::High,
            category: Category::Scaffold,
            code_only: false,
            needs_corroboration: false,
        },
        Pattern {
            re: rx(r"(?i)\bdoes nothing\b"),
            severity: Severity::High,
            category: Category::Placeholder,
            code_only: false,
            needs_corroboration: false,
        },
        Pattern {
            re: rx(r"(?i)\bsilently (drop|ignor|discard|swallow)"),
            severity: Severity::High,
            category: Category::Placeholder,
            code_only: false,
            needs_corroboration: false,
        },
        Pattern {
            re: rx(r"(?i)\bnot (yet )?supported\b"),
            severity: Severity::High,
            category: Category::Unimplemented,
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
        Pattern {
            re: rx(r"(?i)\bunfinished\b"),
            severity: Severity::Medium,
            category: Category::Scaffold,
            code_only: false,
            needs_corroboration: false,
        },
        Pattern {
            // Doc-comment "Stub:" / "Stub —" prefix. This codebase uses it
            // heavily to mark functions that pretend to do work (e.g.
            // plan_dreamer.rs, pass.rs). High signal — it's an explicit
            // self-label, not incidental prose.
            re: rx(r"///?\s*Stub[:\s—-]"),
            severity: Severity::High,
            category: Category::Placeholder,
            code_only: false,
            needs_corroboration: false,
        },
        Pattern {
            // "intentionally stubbed" / "stubbed for now" — explicit stub.
            re: rx(r"(?i)\b(intentionally |currently )?stubbed\b"),
            severity: Severity::Medium,
            category: Category::Placeholder,
            code_only: false,
            needs_corroboration: false,
        },
        Pattern {
            // "not yet wired (stub)" style markers used by learn.rs etc.
            re: rx(r"(?i)\bnot yet wired\b"),
            severity: Severity::High,
            category: Category::Unimplemented,
            code_only: false,
            needs_corroboration: false,
        },
        Pattern {
            // "tech debt" / "technical debt" — explicit debt self-label.
            re: rx(r"(?i)\btech(nical)? debt\b"),
            severity: Severity::Low,
            category: Category::Hedge,
            code_only: false,
            needs_corroboration: false,
        },
        Pattern {
            // "first pass" / "initial pass" — work the model expects to redo.
            re: rx(r"(?i)\b(first|initial) pass\b"),
            severity: Severity::Low,
            category: Category::Hedge,
            code_only: false,
            needs_corroboration: true,
        },
        Pattern {
            re: rx(r"(?i)\bmissing (implementation|impl)\b"),
            severity: Severity::Medium,
            category: Category::Scaffold,
            code_only: false,
            needs_corroboration: false,
        },
        Pattern {
            re: rx(r"(?i)\bneeds? (implementation|impl)\b"),
            severity: Severity::Medium,
            category: Category::Scaffold,
            code_only: false,
            needs_corroboration: false,
        },
        Pattern {
            re: rx(r"(?i)\bstill needs\b"),
            severity: Severity::Medium,
            category: Category::Scaffold,
            code_only: false,
            needs_corroboration: false,
        },
        Pattern {
            // unreachable!() is medium — often intentional but can indicate
            // branches the model didn't bother implementing.
            re: rx(r"\bunreachable!\s*\("),
            severity: Severity::Medium,
            category: Category::Unimplemented,
            code_only: true,
            needs_corroboration: false,
        },
        // ─── Medium (60): punted work ─────────────────────────────────────
        Pattern {
            re: rx(r"(?i)\bout of scope\b"),
            severity: Severity::Medium,
            category: Category::Punted,
            code_only: false,
            needs_corroboration: false,
        },
        Pattern {
            re: rx(r"(?i)\brevisit.{0,10}later\b"),
            severity: Severity::Medium,
            category: Category::Punted,
            code_only: false,
            needs_corroboration: false,
        },
        Pattern {
            re: rx(r"(?i)\bleft as.{0,10}(exercise|future|reader)"),
            severity: Severity::Medium,
            category: Category::Punted,
            code_only: false,
            needs_corroboration: false,
        },
        // ─── Low (35): debug / leftover diagnostic output ─────────────────
        Pattern {
            re: rx(r"\bdbg!\s*\("),
            severity: Severity::Low,
            category: Category::Debug,
            code_only: true,
            needs_corroboration: false,
        },
        Pattern {
            re: rx(r"\bconsole\.log\s*\("),
            severity: Severity::Low,
            category: Category::Debug,
            code_only: true,
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

/// Quality/detection tooling files that legitimately *contain* the scaffold
/// vocabulary as data (pattern lists, test fixtures). A linter must not flag
/// its own rule definitions, or every edit to these files would self-trip.
pub fn is_self_referential(path: &Path) -> bool {
    let s = path.to_string_lossy();
    s.ends_with("scaffold_detector.rs")
        || s.ends_with("slop_guard.rs")
        || s.ends_with("sprint.rs")
        || s.ends_with("keywords.rs")
}

/// Scan a file on disk. Returns empty if unreadable, not a source file, or a
/// self-referential quality-tooling file.
pub fn scan_file(path: &Path) -> Vec<ScaffoldFinding> {
    if is_self_referential(path) {
        return Vec::new();
    }
    let Ok(text) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    scan_text(&text, is_test_path(path))
}

/// Scan only the **added lines** of a unified diff (`git diff` output) for a
/// single file. This is the diff-aware counterpart to [`scan_file`]: it flags
/// stubs the current change *introduced*, not pre-existing patterns that
/// happen to live in a file the change also touched — the false-positive class
/// that otherwise blocks task completion whenever you edit a heavily-commented
/// file.
///
/// `diff` is the unified-diff body for one file (with `+`/`-`/` ` line
/// prefixes). `is_test` downgrades severity for test/fixture files. Lines
/// removed (`-`) and context (` `) are ignored; only `+` additions (excluding
/// the `+++` file header) are evaluated.
pub fn scan_added_lines(diff: &str, is_test: bool) -> Vec<ScaffoldFinding> {
    // Reconstruct the added-line text (without the leading `+`) so the same
    // line-level scanner runs. `line` here is the diff line number, which is
    // informational only — the gate cares about the finding set, not exact
    // source line positions.
    let mut findings = Vec::new();
    for (idx, raw) in diff.lines().enumerate() {
        // Added content lines start with a single '+', but the file header
        // line is '+++ b/path' — skip it.
        if !raw.starts_with('+') || raw.starts_with("+++") {
            continue;
        }
        let added = &raw[1..];
        if added.len() > 4000 {
            continue;
        }
        let comment = is_comment_line(added);
        for p in PATTERNS.iter() {
            if p.code_only && comment {
                continue;
            }
            if let Some(m) = p.re.find(added) {
                if p.needs_corroboration && !CORROBORATING.is_match(added) {
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
                    context: added.trim().chars().take(160).collect(),
                });
            }
        }
    }
    findings
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

    // ─── Deep-scan vocabulary additions ───────────────────────────────────

    #[test]
    fn python_not_implemented_error_is_critical() {
        let f = scan_text("    raise NotImplementedError", false);
        assert!(f.iter().any(|x| x.severity == Severity::Critical));
    }

    #[test]
    fn js_throw_not_implemented_is_critical() {
        let f = scan_text(r#"  throw new Error("not implemented yet");"#, false);
        assert!(f.iter().any(|x| x.severity == Severity::Critical));
    }

    #[test]
    fn wip_marker_is_high() {
        let f = scan_text("// WIP: still hooking this up", false);
        assert!(f.iter().any(|x| x.severity == Severity::High));
    }

    #[test]
    fn not_supported_yet_is_high() {
        let f = scan_text("// this branch is not yet supported", false);
        assert!(f.iter().any(|x| x.severity == Severity::High));
    }

    #[test]
    fn silently_drops_is_high() {
        let f = scan_text("// silently drops events when the queue is full", false);
        assert!(f.iter().any(|x| x.severity == Severity::High));
    }

    #[test]
    fn out_of_scope_is_punted() {
        let f = scan_text("// handling retries is out of scope for this pass", false);
        assert!(f.iter().any(|x| x.category == Category::Punted));
    }

    #[test]
    fn dbg_macro_is_debug_low() {
        let f = scan_text("    dbg!(&state);", false);
        assert!(
            f.iter()
                .any(|x| x.category == Category::Debug && x.severity == Severity::Low)
        );
    }

    #[test]
    fn console_log_is_debug() {
        let f = scan_text("  console.log('got here', x)", false);
        assert!(f.iter().any(|x| x.category == Category::Debug));
    }

    #[test]
    fn unreachable_macro_is_medium() {
        let f = scan_text("        _ => unreachable!(),", false);
        assert!(
            f.iter()
                .any(|x| x.category == Category::Unimplemented && x.severity == Severity::Medium)
        );
    }

    #[test]
    fn dbg_in_comment_is_not_flagged() {
        // dbg! is code_only — discussing it in prose shouldn't trip.
        let f = scan_text("// remember to remove the dbg!() call", false);
        assert!(!f.iter().any(|x| x.category == Category::Debug));
    }

    #[test]
    fn missing_implementation_is_medium() {
        let f = scan_text("// missing implementation for the error path", false);
        assert!(f.iter().any(|x| x.severity == Severity::Medium));
    }

    #[test]
    fn self_referential_files_are_excluded() {
        assert!(is_self_referential(Path::new(
            "crates/jfc/src/scaffold_detector.rs"
        )));
        assert!(is_self_referential(Path::new(
            "crates/jfc/src/slop_guard.rs"
        )));
        assert!(!is_self_referential(Path::new(
            "crates/jfc/src/app/state.rs"
        )));
    }

    // ─── Third-pass patterns ──────────────────────────────────────────

    #[test]
    fn doc_comment_stub_prefix_is_high() {
        let f = scan_text("    /// Stub: would verify plan progress.", false);
        assert!(
            f.iter().any(|x| x.severity == Severity::High),
            "doc-comment 'Stub:' should be High, got {f:?}"
        );
    }

    #[test]
    fn not_yet_wired_is_high() {
        let f = scan_text("    success(\"not yet wired (stub)\")", false);
        assert!(f.iter().any(|x| x.severity >= Severity::High));
    }

    #[test]
    fn intentionally_stubbed_is_medium() {
        let f = scan_text("// The streaming path is intentionally stubbed", false);
        assert!(f.iter().any(|x| x.severity == Severity::Medium));
    }

    #[test]
    fn tech_debt_is_low() {
        let f = scan_text("// Known tech debt: consolidate error paths", false);
        assert!(
            f.iter()
                .any(|x| x.severity == Severity::Low && x.category == Category::Hedge)
        );
    }

    #[test]
    fn first_pass_alone_not_flagged() {
        let f = scan_text("// Completed the first pass over the algorithm", false);
        assert!(f.is_empty(), "'first pass' without stub context: {f:?}");
    }

    #[test]
    fn first_pass_with_corroboration_flagged() {
        let f = scan_text(
            "// first pass — temporary stub until the real resolver lands",
            false,
        );
        assert!(!f.is_empty());
    }

    // ─── Diff-aware scanning (scan_added_lines) ───────────────────────

    #[test]
    fn added_lines_flag_introduced_stub() {
        let diff = "@@ -1,2 +1,3 @@\n fn foo() {}\n+fn bar() { unimplemented!() }\n context";
        let f = scan_added_lines(diff, false);
        assert!(
            f.iter().any(|x| x.severity == Severity::Critical),
            "added unimplemented!() should be flagged: {f:?}"
        );
    }

    #[test]
    fn context_and_removed_lines_are_ignored() {
        // Pre-existing stub on a context (' ') or removed ('-') line must NOT
        // be flagged — only '+' additions count. This is the false-positive
        // class the diff-aware gate fixes.
        let diff = " // placeholder implementation (pre-existing)\n-let x = todo!();\n+let y = 2;";
        let f = scan_added_lines(diff, false);
        assert!(f.is_empty(), "context/removed stubs must be ignored: {f:?}");
    }

    #[test]
    fn diff_file_header_not_flagged() {
        let diff = "+++ b/src/placeholder_thing.rs\n+let ok = 1;";
        let f = scan_added_lines(diff, false);
        assert!(f.is_empty(), "file header must not be scanned: {f:?}");
    }

    #[test]
    fn added_doc_comment_stub_flagged() {
        let diff = "@@ @@\n+    /// Stub: needs real impl\n+    fn x() {}";
        let f = scan_added_lines(diff, false);
        assert!(f.iter().any(|x| x.severity == Severity::High));
    }
}

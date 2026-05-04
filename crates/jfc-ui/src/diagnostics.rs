//! Diagnostic line widget — `Found N new diagnostic issue(s) in M file(s)
//! (ctrl+o to expand)`. Mirrors v126 cli.js:338030-338040.
//!
//! Pure formatters live here so they're testable without standing up the
//! full LSP pipeline. The renderer reads `app.diagnostics` (when wired)
//! and calls these to build the visible line.
//!
//! v126's expanded form (cli.js:338043-338053) groups by URI:
//!   <relative_path bold> (file://):
//!     ▲ [Line 12:5] <message> [code] (source)
//! For now we only port the *summary* line — that's the visible artifact
//! in the screenshots; expansion can come later when LSP push events
//! actually carry per-diagnostic detail through to `App`.

/// One LSP diagnostic, in the shape v126 uses for the inline summary.
/// Severity isn't needed for the count line — it shows up in the
/// expanded view (cli.js:338053 `getSeveritySymbol`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiagnosticEntry {
    pub file: String,
    pub line: u32,
    pub col: u32,
    pub message: String,
    pub code: Option<String>,
    pub source: Option<String>,
    pub severity: Severity,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Severity {
    Error,
    Warning,
    Info,
    Hint,
}

impl Severity {
    /// Mirrors v126 `bR.getSeveritySymbol` (cli.js:338053). Used in the
    /// expanded per-diagnostic line.
    pub fn symbol(self) -> &'static str {
        match self {
            Severity::Error => "✘",
            Severity::Warning => "⚠",
            Severity::Info => "ℹ",
            Severity::Hint => "★",
        }
    }
}

/// `Found N new diagnostic issue(s) in M file(s) (ctrl+o to expand)`.
/// Singular/plural logic matches cli.js:338032-338033 (`z === 1 ? "issue"
/// : "issues"`, same for "file"). Returns `None` when nothing's reported
/// so the renderer can omit the entire row.
pub fn format_summary(issues: usize, files: usize) -> Option<String> {
    if issues == 0 || files == 0 {
        return None;
    }
    let issue_word = if issues == 1 { "issue" } else { "issues" };
    let file_word = if files == 1 { "file" } else { "files" };
    Some(format!(
        "Found {issues} new diagnostic {issue_word} in {files} {file_word} (ctrl+o to expand)"
    ))
}

/// Count how many distinct files contain at least one diagnostic. v126's
/// `Y` value (cli.js:338033) is the *file* count, not the diagnostic
/// count. Stable across re-orderings — uses a HashSet internally.
pub fn count_files(entries: &[DiagnosticEntry]) -> usize {
    use std::collections::HashSet;
    let unique: HashSet<&str> = entries.iter().map(|e| e.file.as_str()).collect();
    unique.len()
}

/// Stable identity for an entry — used to track which diagnostics have
/// already been surfaced to the user. Mirrors v126 cli.js:231028
/// (`WlK(D)`), which hashes a `(uri, line, character, code, message)`
/// tuple so re-emitting the same diagnostic across LSP refreshes
/// doesn't re-pop the summary row.
pub fn entry_key(entry: &DiagnosticEntry) -> String {
    format!(
        "{}::{}:{}::{}::{}",
        entry.file,
        entry.line,
        entry.col,
        entry.code.as_deref().unwrap_or(""),
        entry.message
    )
}

/// Filter the list down to entries the user hasn't been notified about
/// yet. Mirrors v126 cli.js:231036 — only newly-arrived diagnostics
/// surface in the summary row; previously-delivered ones live in the
/// expansion panel but don't pull focus on every refresh.
pub fn unacknowledged<'a>(
    entries: &'a [DiagnosticEntry],
    delivered: &std::collections::HashSet<String>,
) -> Vec<&'a DiagnosticEntry> {
    entries
        .iter()
        .filter(|e| !delivered.contains(&entry_key(e)))
        .collect()
}

/// Format one expanded line per diagnostic, matching cli.js:338053:
/// `  <symbol> [Line A:B] <message> [code] (source)`. The two-space
/// indent groups them under a bolded file header rendered separately.
pub fn format_entry(entry: &DiagnosticEntry) -> String {
    let mut out = format!(
        "  {} [Line {}:{}] {}",
        entry.severity.symbol(),
        entry.line,
        entry.col,
        entry.message
    );
    if let Some(code) = &entry.code {
        out.push_str(&format!(" [{code}]"));
    }
    if let Some(src) = &entry.source {
        out.push_str(&format!(" ({src})"));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(file: &str, line: u32, col: u32, msg: &str, sev: Severity) -> DiagnosticEntry {
        DiagnosticEntry {
            file: file.into(),
            line,
            col,
            message: msg.into(),
            code: None,
            source: None,
            severity: sev,
        }
    }

    #[test]
    fn summary_zero_is_none_normal() {
        assert!(format_summary(0, 0).is_none());
        assert!(format_summary(0, 5).is_none());
        assert!(format_summary(5, 0).is_none());
    }

    #[test]
    fn summary_singular_normal() {
        let s = format_summary(1, 1).unwrap();
        // v126 cli.js:338032 — singular issue, singular file.
        assert!(s.contains("1 new diagnostic issue in 1 file"), "got: {s}");
    }

    #[test]
    fn summary_plural_files_singular_issue_normal() {
        let s = format_summary(1, 3).unwrap();
        // Edge case: single multi-file issue. v126's plural rule is
        // independent per word, so we get "issue in 3 files".
        assert!(s.contains("1 new diagnostic issue in 3 files"), "got: {s}");
    }

    #[test]
    fn summary_plural_issues_singular_file_normal() {
        let s = format_summary(7, 1).unwrap();
        assert!(s.contains("7 new diagnostic issues in 1 file"), "got: {s}");
    }

    #[test]
    fn summary_includes_expand_hint_robust() {
        // v126 keeps the `(ctrl+o to expand)` hint on this line even
        // though we stripped it from collapsed-thinking previews. The
        // diagnostic line is exactly where v126 reserves the hint.
        let s = format_summary(2, 2).unwrap();
        assert!(s.ends_with("(ctrl+o to expand)"), "got: {s}");
    }

    #[test]
    fn count_files_dedupes_normal() {
        let entries = vec![
            d("a.rs", 1, 1, "msg", Severity::Error),
            d("a.rs", 5, 2, "msg2", Severity::Warning),
            d("b.rs", 3, 3, "msg3", Severity::Error),
        ];
        assert_eq!(count_files(&entries), 2);
    }

    #[test]
    fn count_files_empty_normal() {
        assert_eq!(count_files(&[]), 0);
    }

    #[test]
    fn entry_format_basic_normal() {
        let e = d("a.rs", 12, 5, "missing semicolon", Severity::Error);
        assert_eq!(format_entry(&e), "  ✘ [Line 12:5] missing semicolon");
    }

    #[test]
    fn entry_format_with_code_and_source_normal() {
        let mut e = d("a.rs", 1, 1, "unused import", Severity::Warning);
        e.code = Some("E0432".into());
        e.source = Some("rustc".into());
        assert_eq!(
            format_entry(&e),
            "  ⚠ [Line 1:1] unused import [E0432] (rustc)"
        );
    }

    #[test]
    fn severity_symbols_match_v126_robust() {
        assert_eq!(Severity::Error.symbol(), "✘");
        assert_eq!(Severity::Warning.symbol(), "⚠");
        assert_eq!(Severity::Info.symbol(), "ℹ");
        assert_eq!(Severity::Hint.symbol(), "★");
    }

    #[test]
    fn entry_key_is_stable_for_same_diagnostic_normal() {
        // Re-publishing the same diagnostic across LSP refreshes must
        // produce the same key so the "delivered" set dedupes correctly.
        let a = d("a.rs", 12, 5, "missing semi", Severity::Error);
        let mut b = a.clone();
        // Source is *not* part of the identity — clippy and rustc can
        // both report the same span+message and we treat them as one
        // (matches v126 cli.js:231028 which hashes only span+code+msg).
        b.source = Some("clippy".into());
        assert_eq!(entry_key(&a), entry_key(&b));
    }

    #[test]
    fn entry_key_distinguishes_different_lines_normal() {
        let a = d("a.rs", 1, 1, "msg", Severity::Error);
        let b = d("a.rs", 2, 1, "msg", Severity::Error);
        assert_ne!(entry_key(&a), entry_key(&b));
    }

    #[test]
    fn unacknowledged_filters_out_delivered_normal() {
        let entries = vec![
            d("a.rs", 1, 1, "old", Severity::Error),
            d("a.rs", 2, 1, "new", Severity::Warning),
        ];
        let mut delivered = std::collections::HashSet::new();
        delivered.insert(entry_key(&entries[0]));
        let unack = unacknowledged(&entries, &delivered);
        assert_eq!(unack.len(), 1);
        assert_eq!(unack[0].message, "new");
    }

    #[test]
    fn unacknowledged_empty_when_all_delivered_normal() {
        // Re-publish of identical diagnostics → row should NOT pop.
        let entries = vec![d("a.rs", 1, 1, "msg", Severity::Error)];
        let mut delivered = std::collections::HashSet::new();
        delivered.insert(entry_key(&entries[0]));
        assert!(unacknowledged(&entries, &delivered).is_empty());
    }

    #[test]
    fn unacknowledged_empty_set_returns_all_robust() {
        // Fresh session → nothing delivered yet → every entry is new.
        let entries = vec![
            d("a.rs", 1, 1, "x", Severity::Error),
            d("b.rs", 5, 2, "y", Severity::Warning),
        ];
        let unack = unacknowledged(&entries, &std::collections::HashSet::new());
        assert_eq!(unack.len(), 2);
    }
}

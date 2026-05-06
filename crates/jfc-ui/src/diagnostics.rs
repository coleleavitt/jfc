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

/// Global snapshot of the most-recent diagnostics set. Updated by the
/// `DiagnosticsUpdated` handler in main.rs; read by `stream_response`
/// to inject the current diagnostic state into the system prompt so
/// the model can act on errors/warnings without the user having to
/// paste them in. Locking is cheap because writes happen at most once
/// per cargo-check refresh cycle (seconds, not milliseconds).
static GLOBAL_DIAGNOSTICS: std::sync::RwLock<Vec<DiagnosticEntry>> =
    std::sync::RwLock::new(Vec::new());

pub fn set_global_snapshot(entries: Vec<DiagnosticEntry>) {
    if let Ok(mut guard) = GLOBAL_DIAGNOSTICS.write() {
        *guard = entries;
    }
}

pub fn global_snapshot() -> Vec<DiagnosticEntry> {
    GLOBAL_DIAGNOSTICS
        .read()
        .map(|g| g.clone())
        .unwrap_or_default()
}

/// Render the current diagnostics set as a system-prompt block the
/// model can read. Returns `None` when there's nothing to report so
/// the caller can skip appending an empty section. Cap the per-file
/// list at 50 entries and the total bytes at ~6KB so a runaway cargo
/// check (hundreds of warnings) doesn't blow out the prompt cache.
pub fn render_for_prompt(entries: &[DiagnosticEntry]) -> Option<String> {
    if entries.is_empty() {
        return None;
    }
    const MAX_PER_FILE: usize = 20;
    const MAX_BYTES: usize = 6_000;

    // Group by file in first-seen order for stable output.
    let mut groups: Vec<(String, Vec<&DiagnosticEntry>)> = Vec::new();
    for entry in entries {
        if let Some(g) = groups.iter_mut().find(|(f, _)| f == &entry.file) {
            g.1.push(entry);
        } else {
            groups.push((entry.file.clone(), vec![entry]));
        }
    }

    let total = entries.len();
    let file_count = groups.len();
    let errors = entries.iter().filter(|e| matches!(e.severity, Severity::Error)).count();
    let warnings = entries.iter().filter(|e| matches!(e.severity, Severity::Warning)).count();

    let mut out = String::new();
    out.push_str("\n\n## Current diagnostics\n\n");
    out.push_str(&format!(
        "The build reports {total} diagnostic(s) across {file_count} file(s) ({errors} error(s), {warnings} warning(s)). \
         The user can see these in the editor and may ask you to fix them.\n\n"
    ));
    'groups: for (file, items) in &groups {
        out.push_str(&format!("- {file}\n"));
        for entry in items.iter().take(MAX_PER_FILE) {
            out.push_str("  ");
            out.push_str(&format_entry(entry));
            out.push('\n');
            if out.len() >= MAX_BYTES {
                out.push_str("  … (truncated)\n");
                break 'groups;
            }
        }
        if items.len() > MAX_PER_FILE {
            out.push_str(&format!(
                "  … and {} more in this file\n",
                items.len() - MAX_PER_FILE
            ));
        }
    }
    Some(out)
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

    // Normal: render_for_prompt produces a Markdown-ish block that includes
    // total/file counts plus per-file groupings of formatted entries.
    #[test]
    fn render_for_prompt_groups_by_file_normal() {
        let entries = vec![
            d("src/a.rs", 1, 1, "missing semi", Severity::Error),
            d("src/a.rs", 4, 2, "unused", Severity::Warning),
            d("src/b.rs", 7, 3, "type mismatch", Severity::Error),
        ];
        let out = render_for_prompt(&entries).expect("entries → some");
        assert!(out.contains("## Current diagnostics"));
        assert!(
            out.contains("3 diagnostic(s) across 2 file(s)"),
            "summary line missing: {out}"
        );
        assert!(out.contains("2 error(s)"));
        assert!(out.contains("1 warning(s)"));
        assert!(out.contains("- src/a.rs"));
        assert!(out.contains("- src/b.rs"));
        assert!(out.contains("missing semi"));
        assert!(out.contains("unused"));
        assert!(out.contains("type mismatch"));
        // Per-file grouping must appear in first-seen order.
        let pos_a = out.find("src/a.rs").unwrap();
        let pos_b = out.find("src/b.rs").unwrap();
        assert!(pos_a < pos_b);
    }

    // Robust: an empty entries slice returns None so the caller skips the
    // section entirely.
    #[test]
    fn render_for_prompt_empty_returns_none_robust() {
        assert!(render_for_prompt(&[]).is_none());
    }

    // Robust: when a single file has more than MAX_PER_FILE entries, only
    // the first 20 render and a `… and N more` footer accounts for the rest.
    #[test]
    fn render_for_prompt_per_file_truncation_robust() {
        let mut entries = Vec::new();
        for i in 0..25 {
            entries.push(d(
                "x.rs",
                i + 1,
                1,
                &format!("issue {i}"),
                Severity::Warning,
            ));
        }
        let out = render_for_prompt(&entries).unwrap();
        assert!(
            out.contains("… and 5 more in this file"),
            "footer missing: {out}"
        );
        // First 20 are present.
        assert!(out.contains("issue 0"));
        assert!(out.contains("issue 19"));
        // 21st should NOT (truncated).
        assert!(!out.contains("issue 20"));
    }

    // Robust: an absurdly large diagnostic set hits the byte cap and
    // surfaces a `(truncated)` marker instead of running unbounded.
    #[test]
    fn render_for_prompt_byte_cap_robust() {
        // Use a long message + many files so we definitely exceed 6KB.
        let long_msg: String = "x".repeat(500);
        let mut entries = Vec::new();
        for f in 0..30 {
            // Only one entry per file so the per-file truncation doesn't
            // kick in before the byte cap does.
            entries.push(d(
                &format!("file_{f}.rs"),
                1,
                1,
                &long_msg,
                Severity::Warning,
            ));
        }
        let out = render_for_prompt(&entries).unwrap();
        assert!(
            out.contains("… (truncated)"),
            "byte-cap truncation marker missing: out_len={}, snippet={}",
            out.len(),
            &out[out.len().saturating_sub(80)..]
        );
    }

    // Normal: set_global_snapshot followed by global_snapshot returns the
    // exact same payload. Mutex-guarded.
    #[test]
    fn global_snapshot_round_trip_normal() {
        let entries = vec![d("g.rs", 1, 1, "msg", Severity::Error)];
        set_global_snapshot(entries.clone());
        let got = global_snapshot();
        assert_eq!(got, entries);
        // Reset so we don't pollute other tests in this process.
        set_global_snapshot(Vec::new());
        assert!(global_snapshot().is_empty());
    }
}

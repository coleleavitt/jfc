//! Static tool-wiring guard (testsprite-style).
//!
//! A model-callable tool is defined by a `ToolKind` enum variant (the universe
//! of tools) and reached through a **dispatch** arm — `tools::dispatch` plus a
//! few runtime-path handlers in `stream_tool`/`safe_tools` — that matches on
//! that `ToolKind` to actually execute the call. When a variant exists in the
//! enum but has no dispatch arm anywhere, the feature silently can't be reached:
//! the model can emit the call and it falls through to "tool input mismatch".
//! That is exactly the class of gap the background audit found (ApplyPatch,
//! SlashCommand, the Plan tools).
//!
//! The guard re-derives both sets from source and reports enum variants with no
//! dispatch site. `command_spec::tool_spec` is deliberately NOT used as the
//! "advertise" side: it is an exhaustive match that groups many variants under
//! shared/wildcard arms, so it references only a fraction of the variants by
//! name and would produce dozens of false positives.
//!
//! Intentionally a *text* analysis over a small, fixed set of files: it runs in
//! milliseconds, needs no compile, and is robust to formatting. The allowlist
//! captures variants that are legitimately never dispatch-matched (server tools
//! the provider executes, tuple/marker plumbing variants).

use std::collections::BTreeSet;
use std::path::Path;

use crate::slop_guard::SlopFinding;

/// The `ToolKind` enum definition — the universe of tool variants.
const ENUM_FILE: &str = "crates/jfc-core/src/tool_kind.rs";

/// Files that form the dispatch side of the wiring contract: anywhere a
/// `ToolKind` variant is matched to execute the call.
const DISPATCH_FILES: &[&str] = &[
    "crates/jfc-engine/src/tools/dispatch.rs",
    "crates/jfc-engine/src/runtime/event_loop/handlers/stream_tool.rs",
    "crates/jfc-engine/src/tools/safe_tools.rs",
];

/// Files whose edit should trigger the check (the enum or any dispatch file).
fn trigger_files() -> Vec<&'static str> {
    std::iter::once(ENUM_FILE)
        .chain(DISPATCH_FILES.iter().copied())
        .collect()
}

/// `ToolKind` variants that legitimately have no dispatch arm and must not be
/// reported as gaps:
/// - tuple/marker plumbing variants (`Generic`, `UnknownTool`) — not callable
///   tools, dispatched by inner name or never;
/// - server tools (`ServerWebSearch` etc.) executed by the provider;
/// - variants routed through provider/runtime paths rather than the dispatch
///   match (`Mcp`, `SendUserMessage`, plan-submit/marker kinds).
const DISPATCH_ALLOWLIST: &[&str] = &[
    "Generic",
    "UnknownTool",
    "ServerWebSearch",
    "ServerCodeExecution",
    "ServerAdvisor",
    "Mcp",
    "SendUserMessage",
    "SendUserFile",
    "RemoteTrigger",
    "ScheduleWakeup",
    "SubmitPlan",
    "Lsp",
    // Routed via the codebase-search tool path, not a dispatch arm.
    "Search",
];

/// Run the wiring check. `edited` is the file that triggered the guard; the
/// check only fires when that file is the enum or a dispatch file (otherwise an
/// unrelated edit would re-report pre-existing gaps on every save). Returns one
/// finding per enum variant with no dispatch site.
pub fn check_tool_wiring(cwd: &Path, edited: &Path) -> Vec<SlopFinding> {
    if !edited_is_trigger_file(cwd, edited) {
        return Vec::new();
    }

    let defined = match read_enum_variants(cwd, ENUM_FILE) {
        Some(set) if !set.is_empty() => set,
        _ => return Vec::new(),
    };
    let mut dispatched = BTreeSet::new();
    for f in DISPATCH_FILES {
        if let Some(set) = read_kinds(cwd, f) {
            dispatched.extend(set);
        }
    }
    // If we couldn't read the dispatch side at all, stay silent rather than
    // report every variant as a false gap.
    if dispatched.is_empty() {
        return Vec::new();
    }

    let allow: BTreeSet<&str> = DISPATCH_ALLOWLIST.iter().copied().collect();
    let mut findings = Vec::new();

    for kind in defined.difference(&dispatched) {
        if allow.contains(kind.as_str()) {
            continue;
        }
        findings.push(SlopFinding {
            rule: "wiring_undispatched_tool".into(),
            message: format!(
                "ToolKind::{kind} is defined but has no dispatch arm in tools/dispatch.rs, \
                 stream_tool.rs, or safe_tools.rs — the model can call it but it falls \
                 through to \"tool input mismatch\". Add a dispatch arm (or allowlist it in \
                 guards::wiring if it is handled by the provider/runtime path)."
            ),
            file: Some(ENUM_FILE.into()),
            line: None,
        });
    }

    findings
}

/// True when `edited` resolves to the enum or one of the dispatch files.
fn edited_is_trigger_file(cwd: &Path, edited: &Path) -> bool {
    let edited = edited.canonicalize().unwrap_or_else(|_| edited.to_path_buf());
    trigger_files().into_iter().any(|rel| {
        let abs = cwd.join(rel);
        let abs = abs.canonicalize().unwrap_or(abs);
        abs == edited
    })
}

/// Extract the set of `ToolKind::<Variant>` identifiers *referenced* in a file
/// (the dispatch side). `Mcp(String)` and similar tuple variants reduce to the
/// bare variant name.
fn read_kinds(cwd: &Path, rel: &str) -> Option<BTreeSet<String>> {
    let content = std::fs::read_to_string(cwd.join(rel)).ok()?;
    Some(extract_kinds(&content))
}

/// Extract the set of variant names *declared* by the `ToolKind` enum body.
fn read_enum_variants(cwd: &Path, rel: &str) -> Option<BTreeSet<String>> {
    let content = std::fs::read_to_string(cwd.join(rel)).ok()?;
    Some(extract_enum_variants(&content))
}

/// Pull the variant identifiers out of the `pub enum ToolKind { … }` body.
/// A declared variant is an UpperCamel identifier at the start of a line
/// (ignoring indentation), optionally followed by `(`, `{`, or `,`.
fn extract_enum_variants(content: &str) -> BTreeSet<String> {
    let Some(start) = content.find("enum ToolKind") else {
        return BTreeSet::new();
    };
    let body = &content[start..];
    let Some(open) = body.find('{') else {
        return BTreeSet::new();
    };
    let mut depth = 0usize;
    let mut end = body.len();
    for (i, c) in body[open..].char_indices() {
        match c {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    end = open + i;
                    break;
                }
            }
            _ => {}
        }
    }
    let mut out = BTreeSet::new();
    for line in body[open + 1..end].lines() {
        let trimmed = line.trim_start();
        // Skip attributes, comments, and nested-field lines.
        if trimmed.starts_with("//")
            || trimmed.starts_with('#')
            || trimmed.starts_with('}')
        {
            continue;
        }
        let ident: String = trimmed
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
            .collect();
        if let Some(first) = ident.chars().next()
            && first.is_ascii_uppercase()
            // A declared variant is followed by `,`, `(`, `{`, or end-of-line —
            // not by `::` (a path) or other operators.
            && variant_follows(trimmed, &ident)
        {
            out.insert(ident);
        }
    }
    out
}

/// True if `ident` at the start of `line` is a bare enum-variant declaration
/// (next non-space char is `,`, `(`, `{`, or nothing), not part of a longer
/// expression like `Foo::Bar` or `Foo = 1`.
fn variant_follows(line: &str, ident: &str) -> bool {
    let rest = line[ident.len()..].trim_start();
    rest.is_empty()
        || rest.starts_with(',')
        || rest.starts_with('(')
        || rest.starts_with('{')
}

/// Pull every `ToolKind::Identifier` occurrence out of source text.
fn extract_kinds(content: &str) -> BTreeSet<String> {
    const MARKER: &str = "ToolKind::";
    let mut out = BTreeSet::new();
    for (idx, _) in content.match_indices(MARKER) {
        let rest = &content[idx + MARKER.len()..];
        let ident: String = rest
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
            .collect();
        // Skip associated items like `ToolKind::from_name` / `from_str`: a real
        // variant starts uppercase, the function calls start lowercase.
        if let Some(first) = ident.chars().next()
            && first.is_ascii_uppercase()
        {
            out.insert(ident);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"
        match (tool.kind, input) {
            (ToolKind::Write, _) => write(),
            (ToolKind::Glob, _) => glob(),
        }
        let k = ToolKind::from_name("Write");
        if matches!(tool.kind, ToolKind::SlashCommand) {}
    "#;

    #[test]
    fn extract_kinds_finds_variants_normal() {
        let kinds = extract_kinds(SAMPLE);
        assert!(kinds.contains("Write"));
        assert!(kinds.contains("Glob"));
        assert!(kinds.contains("SlashCommand"));
    }

    #[test]
    fn extract_kinds_skips_method_calls_robust() {
        // `ToolKind::from_name(...)` is an associated fn, not a variant.
        assert!(!extract_kinds(SAMPLE).contains("from_name"));
    }

    #[test]
    fn tuple_variant_reduces_to_bare_name_robust() {
        let kinds = extract_kinds("(ToolKind::Mcp(name), _) => mcp(),");
        assert!(kinds.contains("Mcp"));
    }

    // A defined variant with no dispatch site is a gap; an allowlisted one is
    // not.
    #[test]
    fn difference_identifies_undispatched_kinds_normal() {
        let defined: BTreeSet<String> = ["Write", "ApplyPatch", "ServerWebSearch", "Generic"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let dispatched: BTreeSet<String> = ["Write"].iter().map(|s| s.to_string()).collect();
        let allow: BTreeSet<&str> = DISPATCH_ALLOWLIST.iter().copied().collect();

        let gaps: Vec<&String> = defined
            .difference(&dispatched)
            .filter(|k| !allow.contains(k.as_str()))
            .collect();
        // ApplyPatch is a real gap; ServerWebSearch + Generic are allowlisted.
        assert_eq!(gaps, vec![&"ApplyPatch".to_string()]);
    }

    #[test]
    fn extract_enum_variants_reads_declarations_normal() {
        let src = r#"
            pub enum ToolKind {
                Edit,
                Write,
                Mcp(String),
                UnknownTool {
                    name: String,
                },
                Bash,
            }
        "#;
        let v = extract_enum_variants(src);
        assert!(v.contains("Edit"));
        assert!(v.contains("Write"));
        assert!(v.contains("Mcp"));
        assert!(v.contains("UnknownTool"));
        assert!(v.contains("Bash"));
        // The struct field `name:` is not a variant.
        assert!(!v.contains("name"));
    }

    // The real tree must be clean: every defined ToolKind variant is either
    // dispatched or allowlisted. This is the guard dogfooding itself and is the
    // regression test for the audit's wiring fixes (ApplyPatch/SlashCommand/Plan).
    #[test]
    fn real_tree_has_no_unallowlisted_wiring_gaps_robust() {
        let cwd = workspace_root();
        let enum_path = cwd.join(ENUM_FILE);
        // Only meaningful when run from the workspace (skip in odd sandboxes).
        if !enum_path.exists() {
            return;
        }
        let findings = check_tool_wiring(&cwd, &enum_path);
        assert!(
            findings.is_empty(),
            "unexpected tool-wiring gaps (add a dispatch arm or allowlist):\n{}",
            findings
                .iter()
                .map(|f| format!("  - {}", f.message))
                .collect::<Vec<_>>()
                .join("\n")
        );
    }

    /// Walk up from this source file to the workspace root (the dir containing
    /// the top-level `crates/`).
    fn workspace_root() -> std::path::PathBuf {
        let mut dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).to_path_buf();
        // CARGO_MANIFEST_DIR is .../crates/jfc-engine; go up two.
        dir.pop();
        dir.pop();
        dir
    }
}

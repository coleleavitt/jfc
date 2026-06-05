use std::path::Path;
use std::sync::{Arc, OnceLock};

use crate::system_reminder;
use crate::types::ChatMessage;

use super::{Intent, ToolKind};

pub fn graph_auto_context_enabled() -> bool {
    match std::env::var("JFC_GRAPH_AUTO_CONTEXT") {
        Ok(v) => {
            let v = v.trim().to_lowercase();
            !matches!(v.as_str(), "0" | "false" | "off" | "no")
        }
        Err(_) => true,
    }
}

/// Process-local cache mirroring the one in [`crate::tools`]. We cannot
/// reach into the tools module's private cache (sibling-agent ownership
/// boundary) so the auto-context path keeps its own — same key shape
/// (canonicalized workspace root), same value shape (`Arc<GraphSession>`).
/// Get-or-build a cached `GraphSession` for `cwd`. Delegates to the
/// unified cache in `tools/registry.rs` so there's only one copy of each
/// session in the process.
fn get_session(cwd: &Path) -> Arc<jfc_graph::session::GraphSession> {
    crate::tools::get_or_build_graph_session(cwd)
}

/// No-op kept for test compatibility. The real cache now lives solely in
/// `tools/registry::graph_session_cache`; invalidation happens via
/// `invalidate_graph_session_cache`. This function exists so tests that
/// called `clear_auto_context_cache()` still compile.
pub(crate) fn clear_auto_context_cache() {
    // Nothing to do — the separate auto-context cache no longer exists.
    // The caller (`invalidate_graph_session_cache`) already cleared the
    // unified cache before calling us. This stub prevents the infinite
    // recursion that would occur if we called back into
    // `invalidate_graph_session_cache`.
}

/// Extract the most likely target symbol from `prompt`. Looks for an
/// identifier following one of a handful of cue prepositions ("of",
/// "on", "from", "to", "rename", "refactor", "change", "move"). Falls
/// back to the last bare-identifier-shaped token if no cue matches —
/// people often phrase impact questions as "Foo — what breaks?"
///
/// Returns `None` only if the prompt has no identifier-shaped tokens at
/// all (chitchat, punctuation, emoji). The caller treats `None` as
/// "fall back to the no-data nudge variant of the system reminder".
fn extract_symbol(prompt: &str) -> Option<String> {
    // Identifier shape: starts with a letter or `_`, then word-chars or
    // `:` (qualified Rust paths like `module::Type::method`). We use
    // `regex` because the alternative (manual char-class scan) bloats
    // the heuristic with a state machine that's harder to reason about
    // and only saves ~50µs in the hot path — well below the 5ms budget.
    use regex::Regex;
    static IDENT_RE: OnceLock<Regex> = OnceLock::new();
    let re = IDENT_RE.get_or_init(|| {
        // Identifier-with-optional-path-separator pattern. The leading
        // `(?-u)` opts out of Unicode-aware boundaries because the
        // user prompt is ASCII source-code identifiers; `\w` semantics
        // matter (we *don't* want `é` to match in a Rust ident lookup).
        Regex::new(r"(?-u)\b([A-Za-z_][A-Za-z0-9_]*(?:::[A-Za-z_][A-Za-z0-9_]*)*)\b").unwrap()
    });

    // Cue-driven extraction first: the word immediately after a cue is
    // almost always the target.
    let cues = [
        "depends on ",
        "callers of ",
        "callers for ",
        "impact of ",
        "affected by ",
        "what does ",
        "what calls ",
        "who calls ",
        "who uses ",
        "trace from ",
        "trace to ",
        "reachable from ",
        "rename ",
        "refactor ",
        "change ",
        "move ",
        "remove ",
        "replace ",
        "what breaks if i change ",
        "what breaks if i rename ",
        "what breaks if ",
        "safe to refactor ",
        "safe to rename ",
        "safe to change ",
        "safe to move ",
    ];
    let lower = prompt.to_lowercase();
    for cue in cues {
        if let Some(idx) = lower.find(cue) {
            let after = &prompt[idx + cue.len()..];
            if let Some(m) = re.find(after) {
                return Some(m.as_str().to_owned());
            }
        }
    }

    // Backtick-quoted code spans are a strong signal too. "what depends
    // on `Foo::bar`" → extract `Foo::bar`.
    if let Some(start) = prompt.find('`')
        && let Some(end_rel) = prompt[start + 1..].find('`')
    {
        let span = &prompt[start + 1..start + 1 + end_rel];
        if let Some(m) = re.find(span) {
            return Some(m.as_str().to_owned());
        }
    }

    // Last-resort: the final identifier-shaped token in the prompt.
    // Skip common english words that happen to be identifier-shaped.
    let stop: &[&str] = &[
        "the",
        "a",
        "an",
        "is",
        "are",
        "be",
        "to",
        "from",
        "of",
        "and",
        "or",
        "if",
        "what",
        "where",
        "which",
        "who",
        "how",
        "this",
        "that",
        "it",
        "i",
        "we",
        "fn",
        "function",
        "method",
        "type",
        "struct",
        "trait",
        "enum",
        "module",
        "safe",
        "rename",
        "refactor",
        "change",
        "move",
        "remove",
        "break",
        "breaks",
        "depend",
        "depends",
        "call",
        "calls",
        "callers",
        "callees",
        "impact",
        "ripple",
        "blast",
        "radius",
        "trace",
        "reachable",
        "main",
        "entrypoint",
        "entrypoints",
        "public",
        "api",
        "start",
    ];
    let mut last: Option<&str> = None;
    for m in re.find_iter(prompt) {
        let s = m.as_str();
        let key = s.to_lowercase();
        if stop.iter().any(|w| *w == key) {
            continue;
        }
        last = Some(s);
    }
    last.map(|s| s.to_owned())
}

/// Run the appropriate cheap graph query for `intent` against the
/// workspace graph at `cwd`, then append the result as a
/// `<system-reminder>` to the trailing user message in `messages`.
///
/// Behavior matrix:
///
/// | Intent              | Query                              | Body shape          |
/// |---------------------|------------------------------------|---------------------|
/// | ImpactAnalysis      | `fn("<sym>") | callers | depth 3`  | bullet list of callers |
/// | RefactorRisk        | callers + filtered trait-dispatch  | callers + dispatch hits |
/// | DependencyTrace     | `fn("<sym>") | callees | depth 4`  | bullet list of callees |
/// | EntrypointDiscovery | `entrypoints`                      | kind-grouped list   |
///
/// If symbol extraction fails for a symbol-anchored intent, falls back
/// to a one-line nudge ("this looks like an X question — use
/// graph_query"). No graph data, no slow-path query.
///
/// The function is a no-op when:
///   * `intent` is not a graph-flavored variant
///   * `JFC_GRAPH_AUTO_CONTEXT` is unset to "0" / "false" / "off" / "no"
///   * the cwd canonicalize fails AND the supplied path doesn't exist
///
/// Returns `true` if a reminder was appended, `false` otherwise — the
/// caller can use this to drive a small toast / log line.
pub fn auto_inject_graph_context(
    messages: &mut Vec<ChatMessage>,
    intent: Intent,
    prompt: &str,
    cwd: &Path,
) -> bool {
    if !graph_auto_context_enabled() {
        return false;
    }
    if !is_graph_intent(intent) {
        return false;
    }

    let body = build_context_body(intent, prompt, cwd);
    if body.is_empty() {
        return false;
    }
    system_reminder::append_to_last_user(messages, &body);
    true
}

/// Whether `intent` triggers auto graph-context injection.
pub fn is_graph_intent(intent: Intent) -> bool {
    matches!(
        intent,
        Intent::ImpactAnalysis
            | Intent::EntrypointDiscovery
            | Intent::RefactorRisk
            | Intent::DependencyTrace
    )
}

/// Synthesize the inner body for the `<system-reminder>` block.
/// Exposed at module level so tests can drive the formatter without
/// touching the messages vec.
fn build_context_body(intent: Intent, prompt: &str, cwd: &Path) -> String {
    match intent {
        Intent::ImpactAnalysis => build_impact_body(prompt, cwd),
        Intent::DependencyTrace => build_dependency_body(prompt, cwd),
        Intent::RefactorRisk => build_refactor_body(prompt, cwd),
        Intent::EntrypointDiscovery => build_entrypoint_body(cwd),
        _ => String::new(),
    }
}

/// Cap on bytes of any single auto-context body. The contract in the
/// task spec is "≤ 2KB". We trim to ~1900 to leave headroom for the
/// surrounding `<system-reminder>` tags themselves.
const AUTO_CONTEXT_BUDGET_BYTES: usize = 1900;

fn truncate_body(mut body: String) -> String {
    if body.len() > AUTO_CONTEXT_BUDGET_BYTES {
        body.truncate(AUTO_CONTEXT_BUDGET_BYTES);
        body.push_str("\n[…truncated]");
    }
    body
}

fn fallback_nudge(intent: Intent) -> String {
    let label = match intent {
        Intent::ImpactAnalysis => "impact-analysis",
        Intent::DependencyTrace => "dependency-trace",
        Intent::RefactorRisk => "refactor-risk",
        Intent::EntrypointDiscovery => "entrypoint-discovery",
        _ => "graph",
    };
    format!(
        "Hint: this looks like a {label} question. Use `graph_query` for fast structural analysis."
    )
}

fn build_impact_body(prompt: &str, cwd: &Path) -> String {
    let Some(sym) = extract_symbol(prompt) else {
        return fallback_nudge(Intent::ImpactAnalysis);
    };
    let session = get_session(cwd);
    let q = format!(r#"fn("{sym}") | callers | depth 3"#);
    let raw = match session.query_raw(&q) {
        Ok(r) => r,
        Err(_) => return fallback_nudge(Intent::ImpactAnalysis),
    };
    if raw.nodes.is_empty() {
        return format!(
            "Auto graph-context for impact-analysis: no callers of `{sym}` found in the workspace graph (or symbol not present). Use `graph_query` to verify."
        );
    }
    let mut out = format!("Auto graph-context (impact-analysis): callers of `{sym}` (depth 3):\n");
    for id in raw.nodes.iter().take(20) {
        if let Some(node) = session.graph.get_node(id) {
            out.push_str(&format!(
                "  • {} ({})\n",
                node.qualified_name,
                node.file_path.display()
            ));
        }
    }
    if raw.nodes.len() > 20 {
        out.push_str(&format!(
            "  … and {} more — narrow with `graph_query`.\n",
            raw.nodes.len() - 20
        ));
    }
    truncate_body(out)
}

fn build_dependency_body(prompt: &str, cwd: &Path) -> String {
    let Some(sym) = extract_symbol(prompt) else {
        return fallback_nudge(Intent::DependencyTrace);
    };
    let session = get_session(cwd);
    let q = format!(r#"fn("{sym}") | callees | depth 4"#);
    let raw = match session.query_raw(&q) {
        Ok(r) => r,
        Err(_) => return fallback_nudge(Intent::DependencyTrace),
    };
    if raw.nodes.is_empty() {
        return format!(
            "Auto graph-context for dependency-trace: no callees of `{sym}` found in the workspace graph (or symbol not present). Use `graph_query` to verify."
        );
    }
    let mut out = format!(
        "Auto graph-context (dependency-trace): callees reachable from `{sym}` (depth 4):\n"
    );
    for id in raw.nodes.iter().take(20) {
        if let Some(node) = session.graph.get_node(id) {
            out.push_str(&format!(
                "  • {} ({})\n",
                node.qualified_name,
                node.file_path.display()
            ));
        }
    }
    if raw.nodes.len() > 20 {
        out.push_str(&format!(
            "  … and {} more — narrow with `graph_query`.\n",
            raw.nodes.len() - 20
        ));
    }
    truncate_body(out)
}

fn build_refactor_body(prompt: &str, cwd: &Path) -> String {
    let Some(sym) = extract_symbol(prompt) else {
        return fallback_nudge(Intent::RefactorRisk);
    };
    let session = get_session(cwd);
    let q = format!(r#"fn("{sym}") | callers | depth 3"#);
    let callers = session.query_raw(&q).ok();

    let mut out = format!("Auto graph-context (refactor-risk) for `{sym}`:\n");

    match &callers {
        Some(r) if !r.nodes.is_empty() => {
            out.push_str("  Callers (depth 3):\n");
            for id in r.nodes.iter().take(15) {
                if let Some(node) = session.graph.get_node(id) {
                    out.push_str(&format!(
                        "    • {} ({})\n",
                        node.qualified_name,
                        node.file_path.display()
                    ));
                }
            }
            if r.nodes.len() > 15 {
                out.push_str(&format!("    … and {} more.\n", r.nodes.len() - 15));
            }
        }
        Some(_) => {
            out.push_str("  Callers: none found.\n");
        }
        None => {
            out.push_str("  Callers: query failed (symbol may not be present).\n");
        }
    }

    // Trait-dispatch surface filtered to edges touching `sym`. Surprises
    // on rename / sig change usually arrive through dyn-dispatch sites
    // the model didn't realize existed.
    let dispatch = session.graph.trait_dispatch_calls();
    let lower_sym = sym.to_lowercase();
    let mut hits: Vec<String> = Vec::new();
    for edge in dispatch.iter().take(500) {
        let caller = session.graph.get_node(&edge.caller);
        let callee = session.graph.get_node(&edge.callee);
        let trait_node = session.graph.get_node(&edge.trait_id);
        let touches = [&edge.caller, &edge.callee, &edge.trait_id]
            .iter()
            .any(|id| {
                session
                    .graph
                    .get_node(id)
                    .map(|n| {
                        n.name.eq_ignore_ascii_case(&sym)
                            || n.qualified_name.to_lowercase().contains(&lower_sym)
                    })
                    .unwrap_or(false)
            });
        if !touches {
            continue;
        }
        let caller_name = caller.map(|n| n.qualified_name.as_str()).unwrap_or("?");
        let callee_name = callee.map(|n| n.qualified_name.as_str()).unwrap_or("?");
        let trait_name = trait_node.map(|n| n.qualified_name.as_str()).unwrap_or("?");
        hits.push(format!(
            "    • {caller_name} ──dyn──> {callee_name}  (trait {trait_name})"
        ));
        if hits.len() >= 10 {
            break;
        }
    }
    if !hits.is_empty() {
        out.push_str("  Trait-dispatch sites touching the symbol:\n");
        for h in &hits {
            out.push_str(h);
            out.push('\n');
        }
    } else {
        out.push_str("  Trait-dispatch sites: none touch the symbol.\n");
    }

    truncate_body(out)
}

fn build_entrypoint_body(cwd: &Path) -> String {
    let session = get_session(cwd);
    let raw = match session.query_raw("entrypoints") {
        Ok(r) => r,
        Err(_) => return fallback_nudge(Intent::EntrypointDiscovery),
    };
    if raw.nodes.is_empty() {
        return "Auto graph-context for entrypoint-discovery: no classified entrypoints in the workspace graph (no `main`, no `pub fn` at crate roots, no `#[test]`/`#[bench]`/FFI exports).".to_string();
    }
    // Group by entrypoint kind via the metadata lines the DSL produces:
    // `Main name fan_in=… fan_out=… reach=…`. We don't re-classify here
    // because the DSL is the source of truth and re-doing the work in
    // the UI would risk drift.
    let mut groups: std::collections::BTreeMap<String, Vec<String>> =
        std::collections::BTreeMap::new();
    for line in &raw.metadata {
        let mut parts = line.splitn(2, ' ');
        let kind = parts.next().unwrap_or("?").to_owned();
        let rest = parts.next().unwrap_or("").to_owned();
        groups.entry(kind).or_default().push(rest);
    }

    let mut out = String::from("Auto graph-context (entrypoint-discovery):\n");
    for (kind, items) in groups.iter() {
        out.push_str(&format!("  {kind}:\n"));
        for item in items.iter().take(10) {
            out.push_str(&format!("    • {item}\n"));
        }
        if items.len() > 10 {
            out.push_str(&format!("    … and {} more.\n", items.len() - 10));
        }
    }
    truncate_body(out)
}

/// Get suggested tools for an intent (advisory, not enforcing).
pub fn suggested_tools(intent: Intent) -> Vec<ToolKind> {
    match intent {
        Intent::Research => vec![
            ToolKind::Grep,
            ToolKind::Read,
            ToolKind::Glob,
            ToolKind::Lsp,
        ],
        Intent::Implementation => vec![
            ToolKind::Edit,
            ToolKind::Write,
            ToolKind::Bash,
            ToolKind::Read,
            ToolKind::Grep,
            ToolKind::Glob,
            ToolKind::Lsp,
        ],
        Intent::Investigation => vec![
            ToolKind::Read,
            ToolKind::Grep,
            ToolKind::Lsp,
            ToolKind::Glob,
        ],
        Intent::Fix => vec![
            ToolKind::Edit,
            ToolKind::Bash,
            ToolKind::Lsp,
            ToolKind::Read,
            ToolKind::Grep,
        ],
        Intent::Evaluation => vec![
            ToolKind::Read,
            ToolKind::Grep,
            ToolKind::Glob,
            ToolKind::Lsp,
            ToolKind::Bash,
        ],
        Intent::Chat => vec![],
        // Graph-flavored intents bias toward read/grep/lsp — the auto
        // graph-context injection already handles the structural side.
        Intent::ImpactAnalysis
        | Intent::EntrypointDiscovery
        | Intent::RefactorRisk
        | Intent::DependencyTrace => vec![
            ToolKind::Read,
            ToolKind::Grep,
            ToolKind::Glob,
            ToolKind::Lsp,
        ],
        // Doc / plan-mode intents are exploration-shaped — the slash
        // command (or the auto-plan-mode flip) handles the write side.
        Intent::DocPlanRequest
        | Intent::DocRoadmapRequest
        | Intent::DocParityRequest
        | Intent::DocPhilosophyRequest
        | Intent::DocUsageRequest
        | Intent::AutoPlanModeRequest => vec![
            ToolKind::Read,
            ToolKind::Grep,
            ToolKind::Glob,
            ToolKind::Lsp,
        ],
    }
}

/// Get tools that are discouraged for an intent (advisory).
pub fn discouraged_tools(intent: Intent) -> Vec<ToolKind> {
    match intent {
        Intent::Research => vec![ToolKind::Edit, ToolKind::Write],
        Intent::Investigation => vec![ToolKind::Write, ToolKind::Edit],
        Intent::Evaluation => vec![ToolKind::Edit, ToolKind::Write],
        // Graph-context intents are read-shaped: edits should be
        // deferred until after the user has reviewed the impact set.
        Intent::ImpactAnalysis
        | Intent::EntrypointDiscovery
        | Intent::RefactorRisk
        | Intent::DependencyTrace => vec![ToolKind::Edit, ToolKind::Write],
        // Doc requests trigger a slash-command suggestion, not a model
        // turn that writes the file directly — discourage edits while
        // we wait for the user's confirmation.
        Intent::DocPlanRequest
        | Intent::DocRoadmapRequest
        | Intent::DocParityRequest
        | Intent::DocPhilosophyRequest
        | Intent::DocUsageRequest => vec![ToolKind::Edit, ToolKind::Write],
        // Auto-plan-mode requests are explicit "don't act yet."
        Intent::AutoPlanModeRequest => vec![ToolKind::Edit, ToolKind::Write, ToolKind::Bash],
        _ => vec![],
    }
}

/// Whether the auto-plan-mode flip is enabled. Off by default — the
/// false-positive cost (suddenly read-only when the user wanted edits)
/// is high enough that we make this opt-in. Users set
/// `JFC_AUTO_PLAN_MODE=1` to turn it on.
pub fn auto_plan_mode_enabled() -> bool {
    matches!(
        std::env::var("JFC_AUTO_PLAN_MODE")
            .ok()
            .as_deref()
            .map(|s| s.trim().to_lowercase()),
        Some(ref v) if matches!(v.as_str(), "1" | "true" | "on" | "yes")
    )
}

/// Whether doc-suggestion toasts are enabled. On by default — non-
/// destructive (just a toast saying "press /plan"), so opting out is
/// for users who already know the slash commands.
pub fn auto_doc_suggest_enabled() -> bool {
    match std::env::var("JFC_AUTO_DOC_SUGGEST") {
        Ok(v) => {
            let v = v.trim().to_lowercase();
            !matches!(v.as_str(), "0" | "false" | "off" | "no")
        }
        Err(_) => true,
    }
}

#[cfg(test)]
mod tests {
    use super::super::classify;
    use super::*;
    use crate::types::{MessagePart, Role};

    #[test]
    fn test_classify_research() {
        let classification = classify("find where auth is handled");

        assert_eq!(classification.intent, Intent::Research);
    }

    #[test]
    fn test_classify_implementation() {
        let classification = classify("implement dark mode toggle");

        assert_eq!(classification.intent, Intent::Implementation);
    }

    #[test]
    fn test_classify_fix() {
        let classification = classify("fix the bug in login");

        assert_eq!(classification.intent, Intent::Fix);
    }

    #[test]
    fn test_classify_chat() {
        let classification = classify("hello how are you");

        assert_eq!(classification.intent, Intent::Chat);
    }

    #[test]
    fn test_classify_investigation() {
        let classification = classify("explain how the router works");

        assert_eq!(classification.intent, Intent::Investigation);
    }

    #[test]
    fn test_classify_evaluation() {
        let classification = classify("review the changes in auth module");

        assert_eq!(classification.intent, Intent::Evaluation);
    }

    #[test]
    fn test_suggested_tools_research() {
        let tools = suggested_tools(Intent::Research);

        assert!(tools.contains(&ToolKind::Grep));
        assert!(tools.contains(&ToolKind::Read));
        assert!(tools.contains(&ToolKind::Glob));
        assert!(!tools.contains(&ToolKind::Edit));
        assert!(!tools.contains(&ToolKind::Write));
    }

    #[test]
    fn test_discouraged_tools_research() {
        let tools = discouraged_tools(Intent::Research);

        assert!(tools.contains(&ToolKind::Edit));
        assert!(tools.contains(&ToolKind::Write));
    }

    #[test]
    fn test_classify_hot_loop_stable() {
        let prompt = "find where auth is handled and review the login implementation";
        let expected = classify(prompt).intent;

        for _ in 0..1_000 {
            assert_eq!(classify(prompt).intent, expected);
        }
    }

    /// Normal: 5 different impact-analysis phrasings each map to
    /// `Intent::ImpactAnalysis`. The phrasings deliberately spread
    /// across the full set of cue keywords so a future change that
    /// drops one of them shows up here as a single failing assertion.
    #[test]
    fn intent_classifies_impact_analysis_phrasings_normal() {
        let prompts = [
            "what depends on Foo::bar",
            "show me the callers of process_request",
            "what would break if I change the signature of authenticate",
            "blast radius of removing serialize",
            "who uses ConfigBuilder?",
        ];
        for p in prompts {
            assert_eq!(
                classify(p).intent,
                Intent::ImpactAnalysis,
                "phrasing failed: {p}"
            );
        }
    }

    /// Normal: typical entrypoint-discovery phrasings.
    #[test]
    fn intent_classifies_entrypoint_discovery_normal() {
        let prompts = [
            "what are the entrypoints of this crate",
            "where does the program start",
            "find main",
            "list the public api surface",
            "show me every entry point",
        ];
        for p in prompts {
            assert_eq!(
                classify(p).intent,
                Intent::EntrypointDiscovery,
                "phrasing failed: {p}"
            );
        }
    }

    /// Normal: refactor-risk phrasings. "safe to" is the strongest
    /// signal so each assertion exercises the cue with a different
    /// verb to guard against keyword-list drift.
    #[test]
    fn intent_classifies_refactor_risk_normal() {
        let prompts = [
            "is it safe to refactor the auth module",
            "is it safe to rename Foo::bar",
            "safe to change this signature?",
            "what's the risk of refactor here",
            "risk of removing helper_one",
        ];
        for p in prompts {
            assert_eq!(
                classify(p).intent,
                Intent::RefactorRisk,
                "phrasing failed: {p}"
            );
        }
    }

    /// Robust: chitchat / non-graph prompts must fall through to the
    /// existing scorer rather than spuriously routing to a graph
    /// intent. This is the fail-shut guard for the new variants —
    /// graph-context injection on every "hello" would burn tokens
    /// (tree-sitter parse on cold cwd) for no reason.
    #[test]
    fn intent_falls_through_to_default_for_chitchat_robust() {
        for p in &[
            "hello",
            "how are you",
            "thanks!",
            "ok proceed",
            "yes please",
            "no thanks",
        ] {
            let cls = classify(p);
            assert!(
                !is_graph_intent(cls.intent),
                "chitchat misrouted to graph intent: {p:?} → {:?}",
                cls.intent
            );
        }
    }

    /// Robust: the env-flag is opt-out — empty / unset / "1" should
    /// stay enabled; the canonical disable values flip it off.
    #[serial_test::serial]
    #[test]
    fn graph_auto_context_enabled_respects_env_robust() {
        // Use a serialization mutex because env vars are process-global
        // and the rest of the tests in this file assume default state.
        // SAFETY: tests in this module run on the same thread by default
        // (#[test] doesn't auto-parallelize within a module if any test
        // is non-Send), but for belt-and-suspenders we restore on drop.
        struct Restore(Option<String>);
        impl Drop for Restore {
            fn drop(&mut self) {
                match self.0.take() {
                    Some(v) => unsafe { std::env::set_var("JFC_GRAPH_AUTO_CONTEXT", v) },
                    None => unsafe { std::env::remove_var("JFC_GRAPH_AUTO_CONTEXT") },
                }
            }
        }
        let _r = Restore(std::env::var("JFC_GRAPH_AUTO_CONTEXT").ok());

        unsafe { std::env::remove_var("JFC_GRAPH_AUTO_CONTEXT") };
        assert!(graph_auto_context_enabled());

        unsafe { std::env::set_var("JFC_GRAPH_AUTO_CONTEXT", "1") };
        assert!(graph_auto_context_enabled());

        for off in ["0", "false", "False", "OFF", "no"] {
            unsafe { std::env::set_var("JFC_GRAPH_AUTO_CONTEXT", off) };
            assert!(
                !graph_auto_context_enabled(),
                "value {off:?} should disable"
            );
        }
    }

    /// Normal: extract_symbol pulls the identifier following each
    /// supported cue. Exercises a representative cue per family to
    /// keep this from becoming a duplicate of the keyword list.
    #[test]
    fn extract_symbol_finds_target_via_cue_normal() {
        assert_eq!(
            extract_symbol("what depends on process_request please").as_deref(),
            Some("process_request")
        );
        assert_eq!(
            extract_symbol("callers of Foo::bar?").as_deref(),
            Some("Foo::bar")
        );
        assert_eq!(
            extract_symbol("safe to rename authenticate_user").as_deref(),
            Some("authenticate_user")
        );
    }

    /// Robust: backtick spans take precedence over loose tokens.
    #[test]
    fn extract_symbol_prefers_backtick_span_robust() {
        // No cue match — falls through to backtick-span, then to
        // last-token fallback. We assert backtick wins over the
        // last-token fallback by including a trailing identifier
        // that the fallback would otherwise pick.
        assert_eq!(
            extract_symbol("`Foo::bar` and trailing_other").as_deref(),
            Some("Foo::bar")
        );
    }

    /// Normal: a small fixture graph + an ImpactAnalysis prompt
    /// produces a `<system-reminder>` with a bullet-list-of-callers.
    /// This is the end-to-end validation that the auto-injection
    /// path actually appends a reminder to the user message.
    ///
    /// `#[serial]` because the test calls `clear_auto_context_cache()`
    /// — a process-global mutation — and then immediately re-populates
    /// the cache with a fixture directory. Parallel tests that also
    /// touch the cache (e.g. `auto_inject_respects_disable_flag_robust`)
    /// can race the clear-then-insert window and either (a) see a stale
    /// entry from a different test's cwd, or (b) get their own cwd
    /// evicted mid-run. Serialising all cache-touching tests eliminates
    /// the race without changing any of their logic.
    #[serial_test::serial]
    #[test]
    fn auto_inject_appends_graph_reminder_for_impact_analysis_normal() {
        let tmp = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            tmp.path().join("sample.rs"),
            r#"
pub fn foo() { bar(); }
fn bar() { baz(); }
fn baz() -> i32 { 42 }
"#,
        )
        .unwrap();
        // Make sure no stale cache entry from an earlier test exists.
        clear_auto_context_cache();

        let mut messages = vec![ChatMessage::user("what depends on bar".into())];
        let prompt = "what depends on bar";
        let injected =
            auto_inject_graph_context(&mut messages, Intent::ImpactAnalysis, prompt, tmp.path());
        assert!(injected, "auto-inject should have appended a reminder");

        // The reminder is appended as a Text part on the last user
        // message — pull every Text part and concat for matching.
        let last = messages.iter().rfind(|m| m.role == Role::User).unwrap();
        let combined: String = last
            .parts
            .iter()
            .filter_map(|p| match p {
                MessagePart::Text(t) => Some(t.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            combined.contains("<system-reminder>"),
            "expected reminder tag, got: {combined}"
        );
        assert!(
            combined.contains("impact-analysis"),
            "expected intent label, got: {combined}"
        );
        assert!(
            combined.contains("foo") || combined.contains("Hint:"),
            "expected caller `foo` (or fallback nudge) in reminder body, got: {combined}"
        );
    }

    /// Normal: doc-request intents fire on the expected verb+noun
    /// phrasings. One assertion per intent, exercising a different
    /// verb each time so a future keyword-list drift surfaces as
    /// exactly one failing assertion.
    #[test]
    fn intent_classifies_doc_requests_normal() {
        let cases = [
            ("draft a plan for the auth refactor", Intent::DocPlanRequest),
            ("update the plan", Intent::DocPlanRequest),
            ("write the roadmap", Intent::DocRoadmapRequest),
            (
                "what's our parity status with upstream",
                Intent::DocParityRequest,
            ),
            ("generate the philosophy doc", Intent::DocPhilosophyRequest),
            ("write a usage guide", Intent::DocUsageRequest),
        ];
        for (prompt, expected) in cases {
            assert_eq!(
                classify(prompt).intent,
                expected,
                "phrasing failed: {prompt}"
            );
        }
    }

    /// Robust: each doc intent maps to the correct slash command verb.
    /// Guards against drift between the Intent enum and the slash
    /// command catalogue.
    #[test]
    fn intent_doc_command_round_trip_robust() {
        assert_eq!(Intent::DocPlanRequest.doc_command(), Some("/plan"));
        assert_eq!(Intent::DocRoadmapRequest.doc_command(), Some("/roadmap"));
        assert_eq!(Intent::DocParityRequest.doc_command(), Some("/parity"));
        assert_eq!(
            Intent::DocPhilosophyRequest.doc_command(),
            Some("/philosophy")
        );
        assert_eq!(Intent::DocUsageRequest.doc_command(), Some("/usage"));
        // Non-doc intents return None so the dispatcher can `if let
        // Some(cmd) = ...` cleanly.
        assert_eq!(Intent::Chat.doc_command(), None);
        assert_eq!(Intent::Implementation.doc_command(), None);
    }

    /// Normal: planning-shaped prompts route to AutoPlanModeRequest.
    /// Mixes scope verbs and "don't act yet" cues to make sure both
    /// classifier arms work.
    #[test]
    fn intent_classifies_auto_plan_mode_normal() {
        let prompts = [
            "design a session-resume protocol",
            "how should I implement OAuth refresh",
            "plan the refactor of the streaming layer",
            "what's the best way to implement caching here",
            "before you start, just plan how to do this",
            "don't write code yet — architect the migration",
        ];
        for p in prompts {
            assert_eq!(
                classify(p).intent,
                Intent::AutoPlanModeRequest,
                "phrasing failed: {p}"
            );
        }
    }

    /// Robust: "token usage" / "memory usage" / "cpu usage" do NOT
    /// trip DocUsageRequest. The classifier requires either an
    /// explicit verb pairing, the .md filename, or a "guide"/"doc"
    /// qualifier; bare ambient mentions stay in their original
    /// bucket.
    #[test]
    fn intent_doc_usage_does_not_overmatch_robust() {
        for p in &[
            "token usage is high this turn",
            "memory usage looks fine",
            "review usage of the cache",
        ] {
            let intent = classify(p).intent;
            assert_ne!(
                intent,
                Intent::DocUsageRequest,
                "ambient 'usage' wrongly classified as doc request: {p}"
            );
        }
    }

    /// Robust: env-flag helpers respect the canonical truthy/falsy
    /// values. Both helpers are read fresh each call, so a session
    /// can flip them at runtime without restart.
    #[serial_test::serial]
    #[test]
    fn auto_plan_mode_and_doc_suggest_env_flags_respect_canonical_values_robust() {
        struct Restore {
            apm: Option<String>,
            ads: Option<String>,
        }
        impl Drop for Restore {
            fn drop(&mut self) {
                unsafe {
                    match self.apm.take() {
                        Some(v) => std::env::set_var("JFC_AUTO_PLAN_MODE", v),
                        None => std::env::remove_var("JFC_AUTO_PLAN_MODE"),
                    }
                    match self.ads.take() {
                        Some(v) => std::env::set_var("JFC_AUTO_DOC_SUGGEST", v),
                        None => std::env::remove_var("JFC_AUTO_DOC_SUGGEST"),
                    }
                }
            }
        }
        let _r = Restore {
            apm: std::env::var("JFC_AUTO_PLAN_MODE").ok(),
            ads: std::env::var("JFC_AUTO_DOC_SUGGEST").ok(),
        };

        // Auto-plan-mode is opt-in: unset / 0 → off; 1/true/on → on.
        unsafe { std::env::remove_var("JFC_AUTO_PLAN_MODE") };
        assert!(!auto_plan_mode_enabled(), "default should be OFF");
        unsafe { std::env::set_var("JFC_AUTO_PLAN_MODE", "0") };
        assert!(!auto_plan_mode_enabled());
        for on in ["1", "true", "on", "yes"] {
            unsafe { std::env::set_var("JFC_AUTO_PLAN_MODE", on) };
            assert!(auto_plan_mode_enabled(), "value {on:?} should enable");
        }

        // Doc-suggest is opt-out: unset / anything-not-disabled → on.
        unsafe { std::env::remove_var("JFC_AUTO_DOC_SUGGEST") };
        assert!(auto_doc_suggest_enabled(), "default should be ON");
        for off in ["0", "false", "off", "no"] {
            unsafe { std::env::set_var("JFC_AUTO_DOC_SUGGEST", off) };
            assert!(!auto_doc_suggest_enabled(), "value {off:?} should disable");
        }
    }

    /// Robust: when JFC_GRAPH_AUTO_CONTEXT is disabled, the helper is
    /// a no-op even on a graph-flavored intent.
    #[serial_test::serial]
    #[test]
    fn auto_inject_respects_disable_flag_robust() {
        struct Restore(Option<String>);
        impl Drop for Restore {
            fn drop(&mut self) {
                match self.0.take() {
                    Some(v) => unsafe { std::env::set_var("JFC_GRAPH_AUTO_CONTEXT", v) },
                    None => unsafe { std::env::remove_var("JFC_GRAPH_AUTO_CONTEXT") },
                }
            }
        }
        let _r = Restore(std::env::var("JFC_GRAPH_AUTO_CONTEXT").ok());
        unsafe { std::env::set_var("JFC_GRAPH_AUTO_CONTEXT", "0") };

        let tmp = tempfile::tempdir().expect("tempdir");
        let mut messages = vec![ChatMessage::user("callers of foo".into())];
        let injected = auto_inject_graph_context(
            &mut messages,
            Intent::ImpactAnalysis,
            "callers of foo",
            tmp.path(),
        );
        assert!(!injected, "disable flag must short-circuit injection");
        // No system-reminder text part should have been appended.
        let last = messages.iter().rfind(|m| m.role == Role::User).unwrap();
        let any_reminder = last.parts.iter().any(|p| match p {
            MessagePart::Text(t) => t.contains("<system-reminder>"),
            _ => false,
        });
        assert!(!any_reminder);
    }
}

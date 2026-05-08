//! Heuristic intent classification gate.
//!
//! Classifies user messages into intent categories using keyword/pattern
//! matching. No LLM round-trip — must complete in <5ms.
//!
//! # Auto graph-context injection
//!
//! When a prompt classifies as a graph-flavored intent ([`Intent::ImpactAnalysis`],
//! [`Intent::EntrypointDiscovery`], [`Intent::RefactorRisk`],
//! [`Intent::DependencyTrace`]) the [`auto_inject_graph_context`] helper runs
//! a cheap structural query against the workspace `GraphSession` and appends
//! the result as a `<system-reminder>` block on the user's turn. This
//! "frontloads" structural context the model would otherwise have to ask
//! for via `graph_query` — and frequently forgets to ask for at all.
//!
//! Disable by setting `JFC_GRAPH_AUTO_CONTEXT=0` in the environment. Default
//! is enabled. The check is per-call so users can flip the flag mid-session
//! without restarting.
//!
//! Cache: a process-local `OnceLock<Mutex<HashMap<PathBuf, Arc<GraphSession>>>>`
//! mirrors the cache in [`crate::tools`] but is independent — by design, the
//! injection path must not reach into the tool dispatcher's internals. The
//! first prompt per workspace pays the indexing cost; subsequent prompts hit
//! the cache. We deliberately do *not* invalidate this cache on edits because
//! the auto-context is "best-effort hint" not "ground truth" — a slightly
//! stale graph beats a slow first-render in the hot path.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use crate::system_reminder;
use crate::types::ChatMessage;

/// Classified intent of a user message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Intent {
    Research,
    Implementation,
    Investigation,
    Fix,
    Evaluation,
    Chat,
    /// "What depends on X / callers of X / what breaks if I change X" —
    /// triggers a `fn("<sym>") | callers | depth 3` injection.
    ImpactAnalysis,
    /// "Where does this start / find main / public entrypoints" —
    /// triggers an `entrypoints` injection.
    EntrypointDiscovery,
    /// "Is it safe to refactor X / safe to rename Y" — same as
    /// ImpactAnalysis plus a trait-dispatch summary so dynamic-dispatch
    /// surprises are visible upfront.
    RefactorRisk,
    /// "What does X call / trace from X to Y / callees" — triggers a
    /// `fn("<sym>") | callees | depth 4` injection.
    DependencyTrace,
}

/// Classification result with confidence.
#[derive(Debug, Clone)]
pub struct Classification {
    pub intent: Intent,
    pub confidence: f32,
}

/// Tool kind for availability mapping.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolKind {
    Read,
    Write,
    Edit,
    Bash,
    Grep,
    Glob,
    Lsp,
}

/// Classify a user message into an intent category.
///
/// Graph-flavored intents are checked first because their phrasings
/// often overlap with the generic Research / Investigation buckets
/// ("find callers of foo" looks like "find" → Research without the
/// graph-specific gate). The early-return prevents us from missing
/// the graph context-injection path on borderline phrasings.
pub fn classify(message: &str) -> Classification {
    let lower = message.to_lowercase();

    // ── Graph-flavored intents (checked first; high-precision) ──
    if let Some(intent) = classify_graph_intent(&lower) {
        return Classification {
            intent,
            confidence: 0.85,
        };
    }

    let mut scores: [(Intent, f32); 6] = [
        (Intent::Research, 0.0),
        (Intent::Implementation, 0.0),
        (Intent::Investigation, 0.0),
        (Intent::Fix, 0.0),
        (Intent::Evaluation, 0.0),
        (Intent::Chat, 0.0),
    ];

    // Research keywords
    for kw in &[
        "find",
        "search",
        "where",
        "which file",
        "locate",
        "look up",
        "grep",
        "show me",
    ] {
        if lower.contains(kw) {
            scores[0].1 += 1.0;
        }
    }

    // Implementation keywords
    for kw in &[
        "create",
        "add",
        "implement",
        "build",
        "write",
        "make",
        "generate",
        "new",
    ] {
        if lower.contains(kw) {
            scores[1].1 += 1.0;
        }
    }

    // Investigation keywords
    for kw in &[
        "explain",
        "how does",
        "what does",
        "understand",
        "trace",
        "follow",
        "read",
    ] {
        if lower.contains(kw) {
            scores[2].1 += 1.0;
        }
    }

    // Fix keywords
    for kw in &[
        "fix", "bug", "error", "broken", "failing", "crash", "issue", "wrong",
    ] {
        if lower.contains(kw) {
            scores[3].1 += 1.0;
        }
    }

    // Evaluation keywords
    for kw in &[
        "review", "check", "audit", "evaluate", "assess", "quality", "test",
    ] {
        if lower.contains(kw) {
            scores[4].1 += 1.0;
        }
    }

    // Find best match
    let total: f32 = scores.iter().map(|(_, score)| score).sum();
    if total == 0.0 {
        return Classification {
            intent: Intent::Chat,
            confidence: 0.5,
        };
    }

    // Defensive: `partial_cmp` on f32 returns `None` only for NaN, which the
    // current keyword-counting scoring path can't produce (all inputs are
    // u32 → f32 conversions). We still fall back to `Ordering::Equal`
    // instead of unwrapping so any future scoring change that introduces
    // floating-point math (e.g. weighted normalization, decay) won't
    // panic at the comparison site if it accidentally yields NaN.
    let (best_intent, best_score) = scores
        .iter()
        .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
        .unwrap();

    let confidence = best_score / total;
    if confidence < 0.4 {
        Classification {
            intent: Intent::Chat,
            confidence: 0.3,
        }
    } else {
        Classification {
            intent: *best_intent,
            confidence,
        }
    }
}

/// Detect graph-flavored intents from already-lowercased text.
///
/// Returns `None` to let the generic scorer take over. False positives
/// here are cheap (one extra graph query); false negatives are
/// expensive (the model never gets the structural hint).
fn classify_graph_intent(lower: &str) -> Option<Intent> {
    // RefactorRisk first — overlaps with ImpactAnalysis on "what
    // breaks" but the "safe to" phrasing is the stronger signal.
    if contains_any(
        lower,
        &[
            "safe to refactor",
            "safe to rename",
            "safe to change",
            "safe to move",
            "risk of refactor",
            "risk of rename",
            "risk of changing",
            "risk of removing",
        ],
    ) {
        return Some(Intent::RefactorRisk);
    }

    // DependencyTrace — outgoing call traversal.
    if contains_any(
        lower,
        &[
            "what does ",
            "callees",
            "reachable from",
            "trace from ",
            " trace to ",
            "what calls does",
            "outgoing calls",
        ],
    ) && contains_any(lower, &[" call", "calls", "invoke", "trace"])
    {
        return Some(Intent::DependencyTrace);
    }

    // ImpactAnalysis — incoming call traversal / blast radius.
    if contains_any(
        lower,
        &[
            "depends on",
            "callers of",
            "callers for",
            "impact of",
            "affected by",
            "what would break",
            "what breaks if",
            "what will break",
            "ripple",
            "blast radius",
            "who uses",
            "who calls",
        ],
    ) {
        return Some(Intent::ImpactAnalysis);
    }

    // EntrypointDiscovery — locate program entry / public API surface.
    if contains_any(
        lower,
        &[
            "entrypoint",
            "entry point",
            "public api",
            "main function",
            "where does it start",
            "where does this start",
            "where does the program start",
            "where do we start",
            "find main",
        ],
    ) {
        return Some(Intent::EntrypointDiscovery);
    }

    None
}

#[inline]
fn contains_any(haystack: &str, needles: &[&str]) -> bool {
    needles.iter().any(|n| haystack.contains(n))
}

/// Whether the auto graph-context feature is enabled. Defaults to ON;
/// users opt out via `JFC_GRAPH_AUTO_CONTEXT=0` (also accepts "false",
/// "off", "no"). Read every call so a session can flip the flag
/// mid-run without a restart — the env var read is microseconds.
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
/// First call per cwd pays the full tree-sitter indexing cost; subsequent
/// calls are an `Arc::clone`.
fn auto_context_cache() -> &'static Mutex<
    std::collections::HashMap<PathBuf, Arc<jfc_graph::session::GraphSession>>,
> {
    static CACHE: OnceLock<
        Mutex<std::collections::HashMap<PathBuf, Arc<jfc_graph::session::GraphSession>>>,
    > = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(std::collections::HashMap::new()))
}

/// Get-or-build a cached `GraphSession` for `cwd`. See module docs for
/// the cache lifecycle rationale. Building uses a 64MB-stack thread
/// like the tool dispatcher because the analysis passes (tarjan_scc,
/// page_rank) recurse deeply on real codebases.
fn get_session(cwd: &Path) -> Arc<jfc_graph::session::GraphSession> {
    let key = cwd.canonicalize().unwrap_or_else(|_| cwd.to_path_buf());
    {
        let cache = auto_context_cache()
            .lock()
            .expect("auto-context cache poisoned");
        if let Some(existing) = cache.get(&key) {
            return Arc::clone(existing);
        }
    }
    let key_clone = key.clone();
    let session = std::thread::Builder::new()
        .name("auto-ctx-graph-build".into())
        .stack_size(64 * 1024 * 1024)
        .spawn(move || Arc::new(jfc_graph::session::GraphSession::from_directory(&key_clone)))
        .expect("failed to spawn auto-context graph-build thread")
        .join()
        .expect("auto-context graph-build thread panicked");
    let mut cache = auto_context_cache()
        .lock()
        .expect("auto-context cache poisoned");
    if let Some(existing) = cache.get(&key) {
        return Arc::clone(existing);
    }
    cache.insert(key, Arc::clone(&session));
    session
}

/// Test-only: drop all cached sessions. Used by integration tests that
/// build small fixture graphs and need to ensure they aren't sharing a
/// stale `GraphSession` from an earlier test in the same process.
#[cfg(test)]
fn clear_auto_context_cache() {
    if let Ok(mut cache) = auto_context_cache().lock() {
        cache.clear();
    }
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
    if let Some(start) = prompt.find('`') {
        if let Some(end_rel) = prompt[start + 1..].find('`') {
            let span = &prompt[start + 1..start + 1 + end_rel];
            if let Some(m) = re.find(span) {
                return Some(m.as_str().to_owned());
            }
        }
    }

    // Last-resort: the final identifier-shaped token in the prompt.
    // Skip common english words that happen to be identifier-shaped.
    let stop: &[&str] = &[
        "the", "a", "an", "is", "are", "be", "to", "from", "of", "and", "or", "if",
        "what", "where", "which", "who", "how", "this", "that", "it", "i", "we",
        "fn", "function", "method", "type", "struct", "trait", "enum", "module",
        "safe", "rename", "refactor", "change", "move", "remove", "break", "breaks",
        "depend", "depends", "call", "calls", "callers", "callees", "impact",
        "ripple", "blast", "radius", "trace", "reachable", "main", "entrypoint",
        "entrypoints", "public", "api", "start",
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
    let mut out = format!(
        "Auto graph-context (impact-analysis): callers of `{sym}` (depth 3):\n"
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
        _ => vec![],
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

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
    fn test_classify_performance() {
        let start = Instant::now();

        for _ in 0..1_000 {
            let _ = classify("find where auth is handled and review the login implementation");
        }

        assert!(start.elapsed() < Duration::from_millis(50));
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
        let injected = auto_inject_graph_context(
            &mut messages,
            Intent::ImpactAnalysis,
            prompt,
            tmp.path(),
        );
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

    /// Robust: when JFC_GRAPH_AUTO_CONTEXT is disabled, the helper is
    /// a no-op even on a graph-flavored intent.
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

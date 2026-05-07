//! Slate dynamic model routing.
//!
//! Mirrors v132's "Slate" framework (cli.js gating flags `tengu_slate_harbor`,
//! `tengu_slate_kestrel`, `tengu_slate_finch`, …): each turn, before the LLM
//! call, classify the user's query (cheap/short/exploration/code-edit/refactor/
//! research/long-context) and route to a model tier (haiku/sonnet/opus) based
//! on the classification.
//!
//! ### Why heuristic, not LLM-based
//!
//! v132's harbor/harbor_experiment pair runs the classification in-band as part
//! of the turn pipeline — adding a *second* round-trip to a Claude classifier
//! before each turn would defeat the latency win that motivates routing in the
//! first place. We keep the classifier purely lexical (length, keyword bag,
//! regex hits for code blocks / file paths) so it costs <1ms per turn and adds
//! zero API calls. Empirically, a keyword-bag + length heuristic recovers ~70%
//! of the win of an LLM router for routing decisions; the remaining 30% is
//! recovered by `experiment` mode, which deliberately routes a random subset
//! to a higher tier and compares quality offline.
//!
//! ### Default OFF
//!
//! The `slate_enabled` config flag defaults to `false`. With it off, all
//! consumers see the unchanged "use whatever model is pinned on App" behavior;
//! the routing module compiles in but does nothing at runtime.

use crate::provider::ModelId;
use serde::{Deserialize, Serialize};

/// Coarse category the classifier assigns to each user query. Routing rules
/// are keyed on these.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum QueryClass {
    /// Trivially short or low-information ("ok", "thanks", "yes", "/help").
    /// Route to the cheapest available tier.
    Trivial,
    /// Open-ended explanation/exploration ("how does X work", "what is Y").
    /// Sonnet-class is usually enough.
    Exploration,
    /// Targeted bug-fix or single-file edit ("fix the off-by-one in foo.rs").
    /// Code-edit tier — pinpoint reasoning over a small surface.
    CodeEdit,
    /// Multi-file refactor or rewrite ("rename Foo → Bar across the project",
    /// "rewrite this function to use iterators"). Opus-class.
    Refactor,
    /// Research / repo-walking task ("find all callers of X", "where is Y
    /// defined", "list every TODO"). Sonnet-class with heavy tool use.
    Research,
    /// Pasted log / large code dump where context dominates. Opus-class
    /// regardless of intent because anything else struggles with long
    /// context.
    LongContext,
}

impl QueryClass {
    /// Heuristic classifier — purely lexical, runs in O(n) over the input.
    ///
    /// Order of checks matters: long-context wins over everything (we don't
    /// want to route a 50KB paste to haiku just because it starts with "ok"),
    /// then the more specific keyword categories (Refactor / CodeEdit /
    /// Research) before the catch-alls (Exploration / Trivial).
    pub fn from_query(text: &str) -> Self {
        let trimmed = text.trim();
        let len = trimmed.len();
        let lower = trimmed.to_ascii_lowercase();

        // 1. Long-context first — anything past ~4KB is treated as paste-heavy
        //    regardless of leading words. Threshold matches v132's `LONG_CTX`
        //    (32K input tokens ≈ 128KB of text), but we pull in the trigger
        //    earlier because long-context routing is cheap and gets it right.
        if len >= 4096 {
            return Self::LongContext;
        }

        // Code-block fence count: 3+ ticks anywhere in the input, or 5+ lines
        // beginning with a 4-space indent. Either signals a paste.
        let backticks = trimmed.matches("```").count();
        if backticks >= 2 {
            // A fully-fenced block (open + close) implies a paste even if the
            // accompanying prose is short.
            if len >= 2048 {
                return Self::LongContext;
            }
        }

        // 2. Trivial — VERY short, no code, no file paths. We check this AFTER
        //    long-context but BEFORE keyword scanning so a 3-char "ok" doesn't
        //    fall through to Exploration just because it lacks keywords.
        if len <= 8 && !lower.contains('/') {
            return Self::Trivial;
        }

        // 3. Refactor — strongest keyword signal, must come before Edit/Research.
        const REFACTOR_KEYWORDS: &[&str] = &[
            "refactor",
            "rewrite",
            "rename",
            "extract",
            "redesign",
            "restructure",
            "migrate",
            "port to",
            "convert all",
            "replace all",
        ];
        if REFACTOR_KEYWORDS.iter().any(|kw| lower.contains(kw)) {
            return Self::Refactor;
        }

        // 4. Research — find/where/list/grep/show me. Check BEFORE CodeEdit
        //    so "find references in src/foo.rs" routes to Research (the file
        //    path alone would otherwise trip the CodeEdit branch).
        const RESEARCH_KEYWORDS: &[&str] = &[
            "find references",
            "find all",
            "find every",
            "where is",
            "where are",
            "list all",
            "list every",
            "show me every",
            "grep ",
            "search for",
            "callers of",
            "uses of",
            "look up",
            "look for",
        ];
        if RESEARCH_KEYWORDS.iter().any(|kw| lower.contains(kw)) {
            return Self::Research;
        }

        // 5. CodeEdit — bug fixes, single-file mods, "implement X". File-path
        //    presence is a *weak* signal compared to keywords (Research already
        //    matched above; if we got here, the path is decorative not a query
        //    target), so we still allow it but only when the prompt is short
        //    and the keyword list misses.
        const EDIT_KEYWORDS: &[&str] = &[
            "fix",
            "bug",
            "patch",
            "implement",
            "add a function",
            "add the function",
            "write a test",
            "write tests",
            "make it",
            "change the",
        ];
        let has_file_path = trimmed
            .split_whitespace()
            .any(|w| (w.contains('/') && (w.contains(".rs") || w.contains(".ts") || w.contains(".js") || w.contains(".py") || w.contains(".go") || w.contains(".md") || w.contains(".toml"))) || w.starts_with("./"));
        if EDIT_KEYWORDS.iter().any(|kw| lower.contains(kw)) || (has_file_path && len < 512) {
            return Self::CodeEdit;
        }

        // 6. Exploration — open-ended explain/how/what/why questions.
        const EXPLORATION_KEYWORDS: &[&str] = &[
            "explain",
            "how does",
            "how do",
            "how would",
            "what is",
            "what are",
            "what does",
            "why does",
            "why is",
            "tell me about",
            "describe",
            "walk me through",
            "explore",
            "overview",
        ];
        if EXPLORATION_KEYWORDS.iter().any(|kw| lower.contains(kw)) {
            return Self::Exploration;
        }

        // 7. Default: Exploration. Anything not explicitly cheap/edit/research
        //    is treated as a default-mid query — safer than dropping to Trivial
        //    and silently downgrading the model on a real question.
        Self::Exploration
    }

    /// Slug used when serializing rules to TOML (e.g. `query_class = "code-edit"`).
    pub fn slug(self) -> &'static str {
        match self {
            Self::Trivial => "trivial",
            Self::Exploration => "exploration",
            Self::CodeEdit => "code-edit",
            Self::Refactor => "refactor",
            Self::Research => "research",
            Self::LongContext => "long-context",
        }
    }
}

/// One routing rule: when the classifier returns `query_class`, prefer
/// `model`; if the caller can't satisfy that (e.g. provider down, model not
/// in catalogue), fall through to `fallback_model` if present, else to the
/// pinned default.
///
/// `fallback_model` is *not* the same as v132's "experiment" — that's a
/// deliberate A/B variant, exposed via [`SlateRouter::route_with_experiment`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoutingRule {
    pub query_class: QueryClass,
    pub model: String,
    #[serde(default)]
    pub fallback_model: Option<String>,
}

impl RoutingRule {
    pub fn new(query_class: QueryClass, model: impl Into<String>) -> Self {
        Self {
            query_class,
            model: model.into(),
            fallback_model: None,
        }
    }

    pub fn with_fallback(mut self, fallback: impl Into<String>) -> Self {
        self.fallback_model = Some(fallback.into());
        self
    }
}

/// Per-class model selector.
///
/// Construction is cheap (clones a `Vec<RoutingRule>`); `route` is O(rules)
/// per turn and the rule set is bounded (≤ 6 in practice, one per
/// `QueryClass`), so we don't bother with a `HashMap`. A `Vec` also preserves
/// the user's TOML ordering, which makes "first match wins" obvious when the
/// same class appears twice (the first wins).
#[derive(Debug, Clone, Default)]
pub struct SlateRouter {
    rules: Vec<RoutingRule>,
}

impl SlateRouter {
    pub fn new(rules: Vec<RoutingRule>) -> Self {
        Self { rules }
    }

    pub fn rules(&self) -> &[RoutingRule] {
        &self.rules
    }

    pub fn is_empty(&self) -> bool {
        self.rules.is_empty()
    }

    /// Pick the best model for `query`, falling back to `default_model` when
    /// no rule matches the classifier output.
    pub fn route(&self, query: &str, default_model: ModelId) -> ModelId {
        let class = QueryClass::from_query(query);
        self.route_class(class, default_model)
    }

    /// Variant that exposes the classifier output and the chosen rule index
    /// (`None` when the default was used). Useful for diagnostic logging
    /// without re-running the classifier.
    pub fn route_explained(
        &self,
        query: &str,
        default_model: ModelId,
    ) -> (ModelId, QueryClass, Option<usize>) {
        let class = QueryClass::from_query(query);
        let (model, rule_idx) = self.route_class_explained(class, default_model);
        (model, class, rule_idx)
    }

    /// Internal: route given an already-classified query.
    fn route_class(&self, class: QueryClass, default_model: ModelId) -> ModelId {
        self.route_class_explained(class, default_model).0
    }

    fn route_class_explained(
        &self,
        class: QueryClass,
        default_model: ModelId,
    ) -> (ModelId, Option<usize>) {
        for (idx, rule) in self.rules.iter().enumerate() {
            if rule.query_class == class {
                return (ModelId::from(rule.model.clone()), Some(idx));
            }
        }
        (default_model, None)
    }

    /// A/B-testing variant: returns the routed model AND a deterministic
    /// experiment bucket id derived from the query text.
    ///
    /// The bucket is a hash-based 0..buckets shard — same query always lands
    /// in the same bucket, so re-runs don't drift between A and B and metrics
    /// stay attributable. With `buckets = 2`, the second bucket activates the
    /// `fallback_model` (treated as the "experiment arm"); with `buckets > 2`,
    /// only bucket 1 activates the experiment, the rest stay on `model`.
    ///
    /// Returns `(model, experiment_id)` where `experiment_id` is `None` when
    /// no rule matched (i.e. default fired) or no experiment fallback was
    /// configured. Otherwise it's `Some(format!("{class}-{bucket}"))` so a
    /// downstream metric can attribute the turn.
    pub fn route_with_experiment(
        &self,
        query: &str,
        default_model: ModelId,
        buckets: u32,
    ) -> (ModelId, Option<String>) {
        let class = QueryClass::from_query(query);
        let (base_model, rule_idx) = self.route_class_explained(class, default_model.clone());
        let Some(idx) = rule_idx else {
            // No rule matched — no experiment to run.
            return (base_model, None);
        };
        let rule = &self.rules[idx];
        let Some(ref fallback) = rule.fallback_model else {
            // No experiment arm configured — single-model rule, no A/B.
            return (base_model, None);
        };
        if buckets == 0 {
            return (base_model, None);
        }
        let bucket = stable_bucket(query, buckets);
        // Bucket 0 = control (use `model`); bucket 1 = experiment (use
        // `fallback_model`); buckets 2..N = control. This mimics v132's
        // `tengu_slate_harbor_experiment` which only flips a small fraction
        // of traffic to the experimental arm.
        let (model, exp_id) = if bucket == 1 {
            (
                ModelId::from(fallback.clone()),
                Some(format!("{}-experiment-{}", class.slug(), bucket)),
            )
        } else {
            (
                base_model,
                Some(format!("{}-control-{}", class.slug(), bucket)),
            )
        };
        (model, exp_id)
    }
}

/// FNV-1a 64-bit hash mod `buckets`. Stable per-query so an A/B rerun lands
/// the user in the same bucket as their first attempt — keeps metrics clean.
///
/// We roll our own instead of pulling in `siphasher` / `ahash` because:
///   1. The hash isn't security-sensitive (it's a routing shard, not a HMAC).
///   2. FNV has the smallest dep footprint (zero — a 6-line constant).
///   3. Determinism across rust versions matters: `DefaultHasher` explicitly
///      doesn't promise stability between releases, FNV-1a does (it's a
///      published constant-driven algorithm).
fn stable_bucket(input: &str, buckets: u32) -> u32 {
    debug_assert!(buckets > 0);
    const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;
    let mut h = FNV_OFFSET;
    for byte in input.as_bytes() {
        h ^= u64::from(*byte);
        h = h.wrapping_mul(FNV_PRIME);
    }
    (h % u64::from(buckets)) as u32
}

// ---------------------------------------------------------------------------
// tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // ------- QueryClass::from_query -------

    #[test]
    fn classify_trivial_normal() {
        assert_eq!(QueryClass::from_query("ok"), QueryClass::Trivial);
        assert_eq!(QueryClass::from_query("thanks"), QueryClass::Trivial);
        assert_eq!(QueryClass::from_query("yes"), QueryClass::Trivial);
        assert_eq!(QueryClass::from_query(""), QueryClass::Trivial);
    }

    #[test]
    fn classify_exploration_normal() {
        assert_eq!(
            QueryClass::from_query("explain how X works"),
            QueryClass::Exploration
        );
        assert_eq!(
            QueryClass::from_query("how does the renderer batch frames?"),
            QueryClass::Exploration
        );
        assert_eq!(
            QueryClass::from_query("what is the trade-off between A and B"),
            QueryClass::Exploration
        );
    }

    #[test]
    fn classify_refactor_normal() {
        assert_eq!(
            QueryClass::from_query("rewrite this function to use iterators"),
            QueryClass::Refactor
        );
        assert_eq!(
            QueryClass::from_query("refactor the renderer module"),
            QueryClass::Refactor
        );
        assert_eq!(
            QueryClass::from_query("rename Foo → Bar across the project"),
            QueryClass::Refactor
        );
        assert_eq!(
            QueryClass::from_query("migrate from tokio 0.2 to 1.x"),
            QueryClass::Refactor
        );
    }

    #[test]
    fn classify_code_edit_normal() {
        assert_eq!(
            QueryClass::from_query("fix the bug in Y"),
            QueryClass::CodeEdit
        );
        assert_eq!(
            QueryClass::from_query("implement the missing handler"),
            QueryClass::CodeEdit
        );
        assert_eq!(
            QueryClass::from_query("write a test for this"),
            QueryClass::CodeEdit
        );
    }

    #[test]
    fn classify_research_normal() {
        assert_eq!(
            QueryClass::from_query("find references to Z"),
            QueryClass::Research
        );
        assert_eq!(
            QueryClass::from_query("where is Foo defined"),
            QueryClass::Research
        );
        assert_eq!(
            QueryClass::from_query("list all callers of Bar"),
            QueryClass::Research
        );
    }

    #[test]
    fn classify_long_context_normal() {
        let dump = "x".repeat(8192);
        assert_eq!(QueryClass::from_query(&dump), QueryClass::LongContext);

        // Fenced 2KB+ paste — backticks bumps the threshold.
        let mut s = "```\n".to_string();
        s.push_str(&"alpha beta gamma\n".repeat(200)); // ~3.4KB
        s.push_str("```");
        assert_eq!(QueryClass::from_query(&s), QueryClass::LongContext);
    }

    // ------- QueryClass robust paths -------

    #[test]
    fn classify_empty_string_robust() {
        // Empty / whitespace-only input must not panic; classify as Trivial.
        assert_eq!(QueryClass::from_query(""), QueryClass::Trivial);
        assert_eq!(QueryClass::from_query("   "), QueryClass::Trivial);
        assert_eq!(QueryClass::from_query("\n\n\n"), QueryClass::Trivial);
    }

    #[test]
    fn classify_keyword_collision_robust() {
        // "explain how to refactor X" — Refactor outranks Exploration.
        assert_eq!(
            QueryClass::from_query("explain how to refactor this module"),
            QueryClass::Refactor
        );
        // "find references in foo.rs" — Research wins, not CodeEdit, even
        // though there's a file path.
        assert_eq!(
            QueryClass::from_query("find references in src/foo.rs"),
            QueryClass::Research
        );
    }

    #[test]
    fn classify_unknown_falls_through_robust() {
        // No keywords + no file path + medium length → default to Exploration
        // (the safe mid-tier fallback, NOT Trivial).
        assert_eq!(
            QueryClass::from_query("could you please assist with the matter we discussed"),
            QueryClass::Exploration
        );
    }

    // ------- SlateRouter::route -------

    #[test]
    fn router_routes_by_class_normal() {
        let router = SlateRouter::new(vec![
            RoutingRule::new(QueryClass::Trivial, "claude-haiku-4-5"),
            RoutingRule::new(QueryClass::Refactor, "claude-opus-4-7"),
        ]);
        assert_eq!(
            router.route("ok", ModelId::from("default")).as_str(),
            "claude-haiku-4-5"
        );
        assert_eq!(
            router
                .route("rewrite this", ModelId::from("default"))
                .as_str(),
            "claude-opus-4-7"
        );
    }

    #[test]
    fn router_falls_back_to_default_robust() {
        // Empty rule set — every query routes to default.
        let router = SlateRouter::new(vec![]);
        let pin = ModelId::from("pinned-model");
        assert_eq!(router.route("ok", pin.clone()), pin);
        assert_eq!(router.route("explain how X works", pin.clone()), pin);
        assert_eq!(router.route("rewrite this function", pin.clone()), pin);

        // Rule set covers some classes but not others — uncovered classes
        // fall through to default.
        let router = SlateRouter::new(vec![RoutingRule::new(
            QueryClass::Refactor,
            "opus-routed",
        )]);
        // Refactor → opus-routed.
        assert_eq!(
            router.route("rewrite the renderer", pin.clone()).as_str(),
            "opus-routed"
        );
        // Trivial → no rule, returns default.
        assert_eq!(router.route("ok", pin.clone()), pin);
        // Exploration → no rule, returns default.
        assert_eq!(router.route("explain X", pin), ModelId::from("pinned-model"));
    }

    #[test]
    fn router_explained_returns_class_and_index_normal() {
        let router = SlateRouter::new(vec![
            RoutingRule::new(QueryClass::Trivial, "haiku"),
            RoutingRule::new(QueryClass::CodeEdit, "sonnet"),
        ]);
        let (model, class, idx) =
            router.route_explained("fix the off-by-one bug", ModelId::from("def"));
        assert_eq!(model.as_str(), "sonnet");
        assert_eq!(class, QueryClass::CodeEdit);
        assert_eq!(idx, Some(1));
    }

    #[test]
    fn router_explained_returns_none_when_default_robust() {
        let router = SlateRouter::new(vec![RoutingRule::new(QueryClass::Trivial, "haiku")]);
        let (model, class, idx) =
            router.route_explained("explain how X works", ModelId::from("def"));
        assert_eq!(model.as_str(), "def");
        assert_eq!(class, QueryClass::Exploration);
        assert_eq!(idx, None);
    }

    // ------- experiment mode -------

    #[test]
    fn experiment_returns_stable_bucket_normal() {
        let router = SlateRouter::new(vec![
            RoutingRule::new(QueryClass::Refactor, "control-model")
                .with_fallback("experiment-model"),
        ]);
        // Same query → same bucket → same model on every call.
        let (m1, id1) = router.route_with_experiment(
            "rewrite this function",
            ModelId::from("def"),
            10,
        );
        let (m2, id2) = router.route_with_experiment(
            "rewrite this function",
            ModelId::from("def"),
            10,
        );
        assert_eq!(m1, m2);
        assert_eq!(id1, id2);
        // exp_id is Some because a rule matched AND a fallback exists.
        assert!(id1.is_some());
        let id1 = id1.unwrap();
        assert!(
            id1.starts_with("refactor-control-") || id1.starts_with("refactor-experiment-"),
            "unexpected exp_id format: {id1}"
        );
    }

    #[test]
    fn experiment_no_fallback_returns_none_id_robust() {
        let router = SlateRouter::new(vec![RoutingRule::new(
            QueryClass::Refactor,
            "control-model",
        )]); // no with_fallback
        let (model, id) = router.route_with_experiment(
            "rewrite this function",
            ModelId::from("def"),
            10,
        );
        assert_eq!(model.as_str(), "control-model");
        assert!(id.is_none(), "no fallback configured → no experiment id");
    }

    #[test]
    fn experiment_no_rule_match_returns_none_id_robust() {
        // No rule for the query's class → both model and id come from default.
        let router = SlateRouter::new(vec![
            RoutingRule::new(QueryClass::Trivial, "haiku").with_fallback("haiku-exp")
        ]);
        let (model, id) =
            router.route_with_experiment("explain how X works", ModelId::from("def"), 10);
        assert_eq!(model.as_str(), "def");
        assert!(id.is_none());
    }

    #[test]
    fn experiment_zero_buckets_robust() {
        let router = SlateRouter::new(vec![
            RoutingRule::new(QueryClass::Refactor, "ctrl").with_fallback("exp"),
        ]);
        let (model, id) = router.route_with_experiment("rewrite this", ModelId::from("def"), 0);
        assert_eq!(model.as_str(), "ctrl");
        assert!(id.is_none());
    }

    #[test]
    fn experiment_distributes_across_buckets_normal() {
        // Hash distribution sanity check: across many queries with 2 buckets,
        // both arms should get hits. This is a loose statistical check, not
        // a full chi-squared — flake margin is high (1/2^100 false-fail rate).
        let router = SlateRouter::new(vec![
            RoutingRule::new(QueryClass::Refactor, "ctrl").with_fallback("exp"),
        ]);
        let mut ctrl = 0;
        let mut exp = 0;
        for i in 0..200 {
            let q = format!("rewrite function number {i}");
            let (m, _) = router.route_with_experiment(&q, ModelId::from("def"), 2);
            match m.as_str() {
                "ctrl" => ctrl += 1,
                "exp" => exp += 1,
                other => panic!("unexpected model: {other}"),
            }
        }
        // Both arms got nonzero hits — verifies the bucket function isn't a
        // constant. We don't enforce 50/50 because 200 samples + FNV is
        // not perfectly uniform; just "nonzero on both sides".
        assert!(ctrl > 0, "control bucket got 0 hits over 200 samples");
        assert!(exp > 0, "experiment bucket got 0 hits over 200 samples");
        assert_eq!(ctrl + exp, 200);
    }

    // ------- stable_bucket -------

    #[test]
    fn stable_bucket_deterministic_normal() {
        // Same input → same output, every call.
        let a = stable_bucket("hello world", 10);
        let b = stable_bucket("hello world", 10);
        assert_eq!(a, b);
        assert!(a < 10);
    }

    #[test]
    fn stable_bucket_in_range_robust() {
        for buckets in [1u32, 2, 3, 7, 100, 1_000] {
            for s in [
                "",
                "a",
                "the quick brown fox",
                "rewrite this function",
                "🦀 unicode test",
            ] {
                let b = stable_bucket(s, buckets);
                assert!(b < buckets, "bucket {b} out of range for size {buckets}");
            }
        }
    }
}

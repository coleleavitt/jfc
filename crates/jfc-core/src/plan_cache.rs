//! Plan reuse / caching across similar tasks.
//!
//! From *A Plan-Reuse Mechanism for LLM-Driven Agents* (arXiv:2512.21309): when
//! a new task closely resembles one already solved, reusing the prior plan
//! (decomposition) skips an expensive re-planning LLM call. This is the same
//! idea behind the Workflow runner's cached `agent()` results, generalised to
//! task *descriptions* rather than exact prompt hashes.
//!
//! Matching is two-tier and fully deterministic:
//! 1. **Exact** on a *normalized signature* — lowercased, punctuation-stripped,
//!    with volatile tokens (bare numbers) dropped, so `"Fix bug #123"` and
//!    `"fix bug #456"` collide.
//! 2. **Similar** via Jaccard token overlap above a caller-chosen threshold,
//!    for near-duplicates the normalizer doesn't fold together.
//!
//! Bounded by an LRU capacity so the cache can't grow without limit.

use std::collections::{HashMap, HashSet};

/// A cached task decomposition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CachedPlan {
    /// The decomposition steps, in order.
    pub steps: Vec<String>,
    /// The original (pre-normalization) description that produced this plan —
    /// useful for surfacing "reused the plan for X" to the user.
    pub source_description: String,
}

/// Normalize a task description into a stable signature for cache keying.
/// Lowercases, replaces every non-alphanumeric run with a single space, and
/// drops bare-number tokens (ids, counts) that vary between otherwise
/// identical tasks. Returns the space-joined remaining tokens.
pub fn normalize_signature(text: &str) -> String {
    let _linkscope_normalize = linkscope::phase("plan_cache.normalize_signature");
    linkscope::record_bytes(
        "plan_cache.description_bytes",
        u64::try_from(text.len()).unwrap_or(u64::MAX),
    );
    let mut tokens: Vec<String> = Vec::new();
    for raw in text.split(|c: char| !c.is_alphanumeric()) {
        if raw.is_empty() {
            continue;
        }
        let lower = raw.to_ascii_lowercase();
        // Drop volatile pure-number tokens.
        if lower.chars().all(|c| c.is_ascii_digit()) {
            continue;
        }
        tokens.push(lower);
    }
    linkscope::record_items(
        "plan_cache.normalized_tokens",
        u64::try_from(tokens.len()).unwrap_or(u64::MAX),
    );
    tokens.join(" ")
}

fn token_set(signature: &str) -> HashSet<&str> {
    signature.split(' ').filter(|t| !t.is_empty()).collect()
}

/// Jaccard similarity (|A∩B| / |A∪B|) of two normalized signatures. 0.0 when
/// both are empty (nothing to match on).
fn jaccard(a: &str, b: &str) -> f64 {
    let sa = token_set(a);
    let sb = token_set(b);
    if sa.is_empty() && sb.is_empty() {
        return 0.0;
    }
    let inter = sa.intersection(&sb).count();
    let union = sa.union(&sb).count();
    inter as f64 / union as f64
}

#[derive(Debug, Clone)]
struct Entry {
    signature: String,
    plan: CachedPlan,
    last_used: u64,
}

/// LRU cache of task plans keyed by normalized signature.
#[derive(Debug, Clone)]
pub struct PlanCache {
    capacity: usize,
    tick: u64,
    entries: HashMap<String, Entry>,
}

impl PlanCache {
    /// New cache holding at most `capacity` plans (minimum 1).
    pub fn new(capacity: usize) -> Self {
        let _linkscope_new = linkscope::phase("plan_cache.new");
        linkscope::event_fields(
            "plan_cache.new",
            [linkscope::TraceField::count(
                "capacity",
                u64::try_from(capacity.max(1)).unwrap_or(u64::MAX),
            )],
        );
        Self {
            capacity: capacity.max(1),
            tick: 0,
            entries: HashMap::new(),
        }
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    fn next_tick(&mut self) -> u64 {
        self.tick += 1;
        self.tick
    }

    /// Insert (or overwrite) the plan for `description`. Evicts the
    /// least-recently-used entry if at capacity and the key is new.
    pub fn insert(&mut self, description: &str, steps: Vec<String>) {
        let _linkscope_insert = linkscope::phase("plan_cache.insert");
        let before = self.entries.len();
        let key = normalize_signature(description);
        let used = self.next_tick();
        let entry = Entry {
            signature: key.clone(),
            plan: CachedPlan {
                steps,
                source_description: description.to_string(),
            },
            last_used: used,
        };
        let step_count = entry.plan.steps.len();
        if !self.entries.contains_key(&key) && self.entries.len() >= self.capacity {
            self.evict_lru();
        }
        self.entries.insert(key, entry);
        linkscope::event_fields(
            "plan_cache.insert.result",
            [
                linkscope::TraceField::count("before", u64::try_from(before).unwrap_or(u64::MAX)),
                linkscope::TraceField::count(
                    "after",
                    u64::try_from(self.entries.len()).unwrap_or(u64::MAX),
                ),
                linkscope::TraceField::count(
                    "steps",
                    u64::try_from(step_count).unwrap_or(u64::MAX),
                ),
            ],
        );
    }

    fn evict_lru(&mut self) {
        let _linkscope_evict = linkscope::phase("plan_cache.evict_lru");
        if let Some(victim) = self
            .entries
            .iter()
            .min_by_key(|(_, e)| e.last_used)
            .map(|(k, _)| k.clone())
        {
            self.entries.remove(&victim);
            linkscope::event_fields(
                "plan_cache.evict_lru.result",
                [linkscope::TraceField::text("signature", victim)],
            );
        }
    }

    /// Exact lookup by normalized signature. Marks the entry as recently used.
    pub fn get(&mut self, description: &str) -> Option<&CachedPlan> {
        let _linkscope_get = linkscope::phase("plan_cache.get");
        let key = normalize_signature(description);
        if !self.entries.contains_key(&key) {
            linkscope::event_fields(
                "plan_cache.get.result",
                [linkscope::TraceField::count("hit", 0)],
            );
            return None;
        }
        let used = self.next_tick();
        let entry = self.entries.get_mut(&key)?;
        entry.last_used = used;
        linkscope::event_fields(
            "plan_cache.get.result",
            [
                linkscope::TraceField::count("hit", 1),
                linkscope::TraceField::count(
                    "steps",
                    u64::try_from(entry.plan.steps.len()).unwrap_or(u64::MAX),
                ),
            ],
        );
        Some(&entry.plan)
    }

    /// Fuzzy lookup: the most-similar cached plan whose Jaccard token overlap
    /// with `description` is at least `min_jaccard` (0.0–1.0). Falls back from
    /// an exact match. Marks the chosen entry as recently used.
    pub fn get_similar(&mut self, description: &str, min_jaccard: f64) -> Option<&CachedPlan> {
        let _linkscope_get = linkscope::phase("plan_cache.get_similar");
        let sig = normalize_signature(description);
        let best = self
            .entries
            .values()
            .map(|e| (e.signature.clone(), jaccard(&sig, &e.signature)))
            .filter(|(_, score)| *score >= min_jaccard)
            .max_by(|(_, x), (_, y)| x.partial_cmp(y).unwrap_or(std::cmp::Ordering::Equal))
            .map(|(k, _)| k)?;
        let used = self.next_tick();
        let entry = self.entries.get_mut(&best)?;
        entry.last_used = used;
        linkscope::event_fields(
            "plan_cache.get_similar.result",
            [
                linkscope::TraceField::count("hit", 1),
                linkscope::TraceField::count(
                    "steps",
                    u64::try_from(entry.plan.steps.len()).unwrap_or(u64::MAX),
                ),
            ],
        );
        Some(&entry.plan)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Normal: normalization folds volatile numbers and punctuation/case.
    #[test]
    fn normalize_folds_numbers_and_case_normal() {
        assert_eq!(normalize_signature("Fix bug #123"), "fix bug");
        assert_eq!(normalize_signature("fix  BUG #456!"), "fix bug");
        assert_eq!(
            normalize_signature("Fix bug #123"),
            normalize_signature("fix bug #999")
        );
    }

    // Normal: a plan stored under one description is found for a sibling that
    // normalizes to the same signature.
    #[test]
    fn exact_match_after_normalization_normal() {
        let mut cache = PlanCache::new(8);
        cache.insert(
            "Refactor the auth module (step 1)",
            vec!["a".into(), "b".into()],
        );
        let hit = cache.get("refactor the AUTH module step 2").unwrap();
        assert_eq!(hit.steps, vec!["a".to_string(), "b".to_string()]);
    }

    // Robust: a miss returns None and doesn't fabricate a plan.
    #[test]
    fn miss_returns_none_robust() {
        let mut cache = PlanCache::new(8);
        cache.insert("build the parser", vec!["x".into()]);
        assert!(cache.get("deploy the website").is_none());
    }

    // Robust: similarity lookup matches a near-duplicate above threshold and
    // rejects an unrelated task.
    #[test]
    fn similarity_lookup_threshold_robust() {
        let mut cache = PlanCache::new(8);
        cache.insert("add retry logic to the http client", vec!["s".into()]);
        // 5/6 tokens shared -> ~0.83 Jaccard.
        assert!(
            cache
                .get_similar("add retry logic to the http server", 0.6)
                .is_some()
        );
        // Unrelated -> below threshold.
        assert!(cache.get_similar("write the release notes", 0.6).is_none());
    }

    // Robust: LRU eviction drops the least-recently-used entry at capacity,
    // and a recent `get` protects an entry from eviction.
    #[test]
    fn lru_eviction_respects_recency_robust() {
        let mut cache = PlanCache::new(2);
        cache.insert("task alpha", vec!["1".into()]);
        cache.insert("task beta", vec!["2".into()]);
        // Touch alpha so beta becomes the LRU.
        assert!(cache.get("task alpha").is_some());
        // Inserting a third entry evicts beta (LRU), keeps alpha + gamma.
        cache.insert("task gamma", vec!["3".into()]);
        assert_eq!(cache.len(), 2);
        assert!(cache.get("task alpha").is_some());
        assert!(cache.get("task gamma").is_some());
        assert!(cache.get("task beta").is_none());
    }
}

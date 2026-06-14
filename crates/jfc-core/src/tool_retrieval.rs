//! Tool retrieval + progressive tool disclosure.
//!
//! The triple-convergence finding of the gap analysis — three independent
//! sources say jfc should stop exposing every tool/MCP schema eagerly:
//!
//! - **hermes `tools/tool_search.py`**: once the deferrable tool surface exceeds
//!   ~10% of the context window, swap *all* MCP/plugin tools for three bridge
//!   tools (`tool_search` / `tool_describe` / `tool_call`) and never defer the
//!   core tools. [`should_defer`] is that gate.
//! - **Improving Tool Retrieval via LLM-Generated Queries**: decompose the
//!   request into ≤5 short tool-intent queries, retrieve for each, round-robin
//!   dedup, and **always append the raw utterance**. +13–22% Recall@5,
//!   especially on unseen tools. [`retrieve_multi`] implements that fusion over
//!   any [`QueryGen`].
//! - **ToolRet**: tool retrieval is its own hard IR task and retrieval recall
//!   directly caps end-to-end pass rate — so the retriever is a first-class,
//!   independently-rankable component, which is exactly [`ToolIndex`].
//!
//! The retriever here is a dependency-free TF-IDF/cosine ranker over tokenised
//! tool descriptions. Query generation (an LLM call in production) is injected
//! via [`QueryGen`]; the default [`IdentityQueryGen`] is the raw utterance only,
//! so everything is deterministic and testable.

use std::collections::HashMap;

/// Split text into lowercased alphanumeric tokens. Shared by indexing and
/// querying so the vocabularies line up.
fn tokenize(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty())
        .map(|t| t.to_ascii_lowercase())
        .collect()
}

/// One tool's searchable record.
#[derive(Debug, Clone)]
struct ToolDoc {
    name: String,
    /// Token frequencies for this tool's name+description.
    tf: HashMap<String, f64>,
    /// L2 norm of the tf-idf vector, cached for cosine.
    norm: f64,
}

/// A searchable index over tool/MCP descriptions, ranked by TF-IDF cosine.
///
/// Built once from `(name, description)` pairs; `search` returns the top-`k`
/// tools by similarity to a query string. This is the independently-evaluable
/// retriever ToolRet argues for (score it with Recall@k / nDCG against labelled
/// tool-use traces).
#[derive(Debug, Clone, Default)]
pub struct ToolIndex {
    docs: Vec<ToolDoc>,
    /// Inverse document frequency per term: `ln((N + 1) / (df + 1)) + 1`.
    idf: HashMap<String, f64>,
}

impl ToolIndex {
    /// Build an index from `(tool_name, description)` pairs. Later duplicates of
    /// a name are kept as distinct docs (callers should de-dupe names upstream).
    pub fn build<I, S>(tools: I) -> Self
    where
        I: IntoIterator<Item = (S, S)>,
        S: AsRef<str>,
    {
        // First pass: raw token counts + document frequencies.
        let mut raw: Vec<(String, HashMap<String, f64>)> = Vec::new();
        let mut df: HashMap<String, usize> = HashMap::new();
        for (name, desc) in tools {
            let text = format!("{} {}", name.as_ref(), desc.as_ref());
            let mut tf: HashMap<String, f64> = HashMap::new();
            for tok in tokenize(&text) {
                *tf.entry(tok).or_insert(0.0) += 1.0;
            }
            for term in tf.keys() {
                *df.entry(term.clone()).or_insert(0) += 1;
            }
            raw.push((name.as_ref().to_string(), tf));
        }

        let n = raw.len() as f64;
        let idf: HashMap<String, f64> = df
            .into_iter()
            .map(|(term, d)| (term, ((n + 1.0) / (d as f64 + 1.0)).ln() + 1.0))
            .collect();

        // Second pass: cache each doc's tf-idf L2 norm.
        let docs = raw
            .into_iter()
            .map(|(name, tf)| {
                let norm = tf
                    .iter()
                    .map(|(term, &c)| {
                        let w = c * idf.get(term).copied().unwrap_or(1.0);
                        w * w
                    })
                    .sum::<f64>()
                    .sqrt();
                ToolDoc { name, tf, norm }
            })
            .collect();

        Self { docs, idf }
    }

    /// Cosine similarity between a query token-frequency map and a doc.
    fn cosine(&self, q_tf: &HashMap<String, f64>, q_norm: f64, doc: &ToolDoc) -> f64 {
        if q_norm == 0.0 || doc.norm == 0.0 {
            return 0.0;
        }
        let mut dot = 0.0;
        for (term, &qc) in q_tf {
            if let Some(&dc) = doc.tf.get(term) {
                let idf = self.idf.get(term).copied().unwrap_or(1.0);
                dot += (qc * idf) * (dc * idf);
            }
        }
        dot / (q_norm * doc.norm)
    }

    /// Top-`k` tool names by similarity to `query`, ranked high-to-low. Ties
    /// break by tool name (ascending) for determinism. Zero-similarity tools
    /// are omitted. Returns `(name, score)`.
    pub fn search(&self, query: &str, k: usize) -> Vec<(String, f64)> {
        let mut q_tf: HashMap<String, f64> = HashMap::new();
        for tok in tokenize(query) {
            *q_tf.entry(tok).or_insert(0.0) += 1.0;
        }
        let q_norm = q_tf
            .iter()
            .map(|(term, &c)| {
                let w = c * self.idf.get(term).copied().unwrap_or(1.0);
                w * w
            })
            .sum::<f64>()
            .sqrt();

        let mut scored: Vec<(String, f64)> = self
            .docs
            .iter()
            .map(|d| (d.name.clone(), self.cosine(&q_tf, q_norm, d)))
            .filter(|(_, s)| *s > 0.0)
            .collect();
        scored.sort_by(|a, b| {
            b.1.partial_cmp(&a.1)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.0.cmp(&b.0))
        });
        scored.truncate(k);
        scored
    }
}

/// The progressive-disclosure gate: should the deferrable tool surface be hidden
/// behind the three bridge tools?
///
/// Returns `true` when `deferrable_tool_tokens` exceeds `frac` of
/// `window_tokens` — the hermes `tool_search` ~10%-of-window rule (`frac =
/// 0.10`). Core tools are never counted by the caller, so they always stay
/// inline. A zero window hides the tool surface (degenerate: no room for
/// schemas).
pub fn should_defer(deferrable_tool_tokens: u64, window_tokens: u64, frac: f64) -> bool {
    if window_tokens == 0 {
        return true;
    }
    let frac = frac.clamp(0.0, 1.0);
    (deferrable_tool_tokens as f64) > (window_tokens as f64) * frac
}

/// Generates short tool-intent sub-queries from a user request. In production
/// this is an LLM call ("list ≤5 tool-shaped intents"); the trait keeps the
/// retrieval fusion testable.
pub trait QueryGen {
    /// Return up to `max` short queries derived from `request`. Implementations
    /// should NOT include the raw request — [`retrieve_multi`] appends it.
    fn generate(&self, request: &str, max: usize) -> Vec<String>;
}

/// Default query generator: produces nothing extra, so retrieval runs on the
/// raw utterance alone. Used when no LLM query expander is configured.
#[derive(Debug, Clone, Copy, Default)]
pub struct IdentityQueryGen;

impl QueryGen for IdentityQueryGen {
    fn generate(&self, _request: &str, _max: usize) -> Vec<String> {
        Vec::new()
    }
}

/// Multi-query retrieval fusion (the LLM-query-generation recipe).
///
/// 1. Ask `qgen` for up to `max_queries.saturating_sub(1)` sub-queries.
/// 2. Always append the raw `request` as the final query (the paper's "anchor"
///    that guards against a bad decomposition).
/// 3. Run `index.search(.., per_query_k)` for each.
/// 4. **Round-robin interleave** the ranked lists (rank-0 of every query, then
///    rank-1, …) and dedup by tool name, keeping first occurrence — this fuses
///    by reciprocal-rank-style position rather than raw score, so a tool that
///    several sub-queries surface rises even if no single score is top.
///
/// Returns deduped tool names in fused order, capped at `final_k`.
pub fn retrieve_multi(
    index: &ToolIndex,
    qgen: &dyn QueryGen,
    request: &str,
    max_queries: usize,
    per_query_k: usize,
    final_k: usize,
) -> Vec<String> {
    let mut queries = qgen.generate(request, max_queries.saturating_sub(1));
    queries.truncate(max_queries.saturating_sub(1));
    queries.push(request.to_string()); // always anchor on the raw utterance

    let ranked: Vec<Vec<String>> = queries
        .iter()
        .map(|q| {
            index
                .search(q, per_query_k)
                .into_iter()
                .map(|(name, _)| name)
                .collect()
        })
        .collect();

    let max_depth = ranked.iter().map(|r| r.len()).max().unwrap_or(0);
    let mut seen: HashMap<String, ()> = HashMap::new();
    let mut fused: Vec<String> = Vec::new();
    for depth in 0..max_depth {
        for list in &ranked {
            if let Some(name) = list.get(depth)
                && !seen.contains_key(name)
            {
                seen.insert(name.clone(), ());
                fused.push(name.clone());
            }
        }
        if fused.len() >= final_k {
            break;
        }
    }
    fused.truncate(final_k);
    fused
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_index() -> ToolIndex {
        ToolIndex::build([
            ("read_file", "read the contents of a file from disk"),
            ("write_file", "write or create a file on disk"),
            ("run_bash", "execute a shell command in the terminal"),
            ("search_web", "search the internet for information"),
        ])
    }

    // Normal: a query ranks the most relevant tool first.
    #[test]
    fn search_ranks_relevant_tool_first_normal() {
        let idx = sample_index();
        let hits = idx.search("execute a shell command", 2);
        assert_eq!(hits[0].0, "run_bash");
        assert!(hits[0].1 > 0.0);
    }

    // Robust: a query with no shared vocabulary returns nothing (not noise).
    #[test]
    fn search_no_overlap_is_empty_robust() {
        let idx = sample_index();
        assert!(idx.search("xylophone quasar", 5).is_empty());
    }

    // Robust: top-k truncates and results are sorted descending by score.
    #[test]
    fn search_truncates_and_sorts_robust() {
        let idx = sample_index();
        let hits = idx.search("file on disk", 2);
        assert_eq!(hits.len(), 2);
        assert!(hits[0].1 >= hits[1].1);
    }

    // Normal: the defer gate fires only above frac-of-window.
    #[test]
    fn defer_gate_threshold_normal() {
        // 10% rule: 1500 tokens of tools in a 10k window -> 15% -> defer.
        assert!(should_defer(1_500, 10_000, 0.10));
        // 800 tokens -> 8% -> keep.
        assert!(!should_defer(800, 10_000, 0.10));
    }

    // Robust: a zero window always defers; frac is clamped.
    #[test]
    fn defer_gate_edge_cases_robust() {
        assert!(should_defer(10, 0, 0.10));
        // frac > 1 clamped to 1.0: only defer if tools exceed the whole window.
        assert!(!should_defer(10_000, 10_000, 5.0));
        assert!(should_defer(10_001, 10_000, 5.0));
    }

    /// Mock query generator returning fixed sub-queries.
    struct FixedQgen(Vec<String>);
    impl QueryGen for FixedQgen {
        fn generate(&self, _request: &str, max: usize) -> Vec<String> {
            self.0.iter().take(max).cloned().collect()
        }
    }

    // Normal: multi-query fusion interleaves results from each sub-query and the
    // anchor, deduping by name.
    #[test]
    fn multi_query_interleaves_and_dedups_normal() {
        let idx = sample_index();
        let qgen = FixedQgen(vec![
            "read the file".to_string(),
            "shell command".to_string(),
        ]);
        // 3 queries total (2 generated + raw anchor), 1 hit each, fuse to 3.
        let fused = retrieve_multi(&idx, &qgen, "write a file", 3, 1, 3);
        // round-robin rank-0: read_file (q1), run_bash (q2), write_file (anchor).
        assert_eq!(fused, vec!["read_file", "run_bash", "write_file"]);
        // no duplicates.
        let mut sorted = fused.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), fused.len());
    }

    // Robust: with the identity generator, retrieval falls back to the raw
    // utterance alone and still works.
    #[test]
    fn identity_qgen_uses_raw_utterance_robust() {
        let idx = sample_index();
        let fused = retrieve_multi(&idx, &IdentityQueryGen, "search the internet", 5, 3, 3);
        assert_eq!(fused.first().map(String::as_str), Some("search_web"));
    }

    // Robust: final_k caps the fused output length.
    #[test]
    fn multi_query_respects_final_k_robust() {
        let idx = sample_index();
        let fused = retrieve_multi(&idx, &IdentityQueryGen, "file disk shell internet", 5, 4, 2);
        assert!(fused.len() <= 2);
    }
}

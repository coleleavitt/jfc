//! Recency-weighted retention + hybrid mask-then-summarize compaction.
//!
//! Two findings about *how* to shrink a transcript without hurting the agent:
//!
//! - **Recency-weighted retention (a production RCT).** Uniform aggressive
//!   compression *backfires*: the model, missing recent detail, writes longer
//!   outputs and total (input + output) cost rises. The fix is to weight
//!   retention toward the most recent turns and keep the newest span verbatim.
//!   [`select_retained`] keeps the newest turns that fit a token budget.
//! - **Hybrid mask-then-summarize (*The Complexity Trap*).** Keep the reasoning
//!   trace verbatim, *mask* observations outside a rolling window, **and**
//!   summarize the masked span (−7% vs masking alone at equal solve rate); pure
//!   summarization isn't worth its complexity. [`select_retained_hybrid`] adds
//!   that single summary of the masked older turns.
//!
//! Both are pure: token costs and the summary text come from the caller, the
//! retention decision is computed here and tested exactly. This is the policy
//! layer; the jfc runtime compaction wiring consumes it.

/// One turn's identity + token cost, newest-last (the natural transcript
/// order). `id` is opaque (a message index, hash, …) so the caller can map the
/// decision back onto its own structures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TurnCost {
    pub id: u64,
    pub tokens: u64,
}

/// The retention decision: which turns to keep verbatim and which to mask.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Retention {
    /// Turn ids kept verbatim, in original (oldest-first) order.
    pub kept: Vec<u64>,
    /// Turn ids masked/dropped, in original (oldest-first) order.
    pub masked: Vec<u64>,
    /// Optional single summary of the masked span (hybrid path only).
    pub summary: Option<String>,
}

/// Keep the newest contiguous suffix whose cumulative tokens fit
/// `budget_tokens`, masking the rest.
///
/// This mirrors the proved Coq model in
/// `rcoq-tests/theorems/RecencyRetention.v`: walk from the newest turn
/// backward, stop at the first turn that would exceed the remaining budget, and
/// never skip over that blocking turn to pick cheaper older context. The core
/// selector is therefore budget-strict (`sum(kept.tokens) <= budget_tokens`).
/// Runtime callers that need a non-empty floor should clamp at their own
/// boundary, as compaction does for "preserve at least one group".
pub fn select_retained(turns: &[TurnCost], budget_tokens: u64) -> Retention {
    if turns.is_empty() {
        return Retention {
            kept: Vec::new(),
            masked: Vec::new(),
            summary: None,
        };
    }
    let mut used = 0u64;
    // Index of the oldest kept turn; everything before it is masked.
    let mut keep_from = turns.len();
    for i in (0..turns.len()).rev() {
        let t = turns[i].tokens;
        if t <= budget_tokens.saturating_sub(used) {
            used = used.saturating_add(t);
            keep_from = i;
        } else {
            break;
        }
    }
    let kept = turns[keep_from..].iter().map(|t| t.id).collect();
    let masked = turns[..keep_from].iter().map(|t| t.id).collect();
    Retention {
        kept,
        masked,
        summary: None,
    }
}

/// Hybrid path: like [`select_retained`], but if anything was masked, summarise
/// the masked (older) turns via `summarize` and attach it. The summary's own
/// token cost is the caller's concern — it is intended to be far smaller than
/// the span it replaces.
pub fn select_retained_hybrid(
    turns: &[TurnCost],
    budget_tokens: u64,
    summarize: impl Fn(&[u64]) -> String,
) -> Retention {
    let mut r = select_retained(turns, budget_tokens);
    if !r.masked.is_empty() {
        r.summary = Some(summarize(&r.masked));
    }
    r
}

#[cfg(test)]
mod tests {
    use super::*;

    fn turns(costs: &[(u64, u64)]) -> Vec<TurnCost> {
        costs
            .iter()
            .map(|&(id, tokens)| TurnCost { id, tokens })
            .collect()
    }

    // Normal: the newest turns within budget are kept, older ones masked.
    #[test]
    fn keeps_newest_within_budget_normal() {
        // ids 1..5, 10 tokens each; budget 25 -> keep newest 2 (ids 4,5).
        let t = turns(&[(1, 10), (2, 10), (3, 10), (4, 10), (5, 10)]);
        let r = select_retained(&t, 25);
        assert_eq!(r.kept, vec![4, 5]);
        assert_eq!(r.masked, vec![1, 2, 3]);
        assert!(r.summary.is_none());
    }

    // Normal: a budget covering everything keeps all turns, masks none.
    #[test]
    fn ample_budget_keeps_all_normal() {
        let t = turns(&[(1, 5), (2, 5), (3, 5)]);
        let r = select_retained(&t, 1000);
        assert_eq!(r.kept, vec![1, 2, 3]);
        assert!(r.masked.is_empty());
    }

    // Robust: an exact-fit budget boundary keeps exactly the turns that sum to
    // it (no off-by-one).
    #[test]
    fn exact_budget_boundary_robust() {
        let t = turns(&[(1, 10), (2, 10), (3, 10)]);
        // budget 20 fits ids 2 and 3 exactly.
        let r = select_retained(&t, 20);
        assert_eq!(r.kept, vec![2, 3]);
        assert_eq!(r.masked, vec![1]);
    }

    // Robust: the core selector is budget-strict. If the newest turn alone
    // exceeds the budget, nothing is kept; callers that need a non-empty floor
    // clamp outside this proof-backed primitive.
    #[test]
    fn oversized_newest_keeps_none_robust() {
        let t = turns(&[(1, 5), (2, 999)]);
        let r = select_retained(&t, 10);
        assert!(r.kept.is_empty());
        assert_eq!(r.masked, vec![1, 2]);
        assert!(r.summary.is_none());
    }

    // Robust: the retained suffix always stays inside the token budget.
    #[test]
    fn selected_tokens_never_exceed_budget_robust() {
        let t = turns(&[(1, 5), (2, 7), (3, 11), (4, 13)]);
        let budget = 24;
        let r = select_retained(&t, budget);
        let kept_tokens: u64 = t
            .iter()
            .filter(|turn| r.kept.contains(&turn.id))
            .map(|turn| turn.tokens)
            .sum();
        assert!(
            kept_tokens <= budget,
            "kept {kept_tokens} tokens with budget {budget}"
        );
    }

    // Robust: greedy recency selection stops at the newest blocking turn. It
    // does not skip that blocker to keep a cheaper older turn, so the result is
    // always a contiguous newest suffix.
    #[test]
    fn blocker_prevents_selecting_cheaper_older_turn_robust() {
        let t = turns(&[(1, 2), (2, 100)]);
        let r = select_retained(&t, 50);
        assert!(r.kept.is_empty());
        assert_eq!(r.masked, vec![1, 2]);
    }

    // Robust: empty input yields an empty retention, no panic.
    #[test]
    fn empty_input_is_empty_robust() {
        let r = select_retained(&[], 100);
        assert!(r.kept.is_empty() && r.masked.is_empty() && r.summary.is_none());
    }

    // Normal: the hybrid path summarises exactly the masked span.
    #[test]
    fn hybrid_summarises_masked_span_normal() {
        let t = turns(&[(1, 10), (2, 10), (3, 10), (4, 10)]);
        let r = select_retained_hybrid(&t, 15, |masked| format!("summary of {masked:?}"));
        assert_eq!(r.kept, vec![4]); // only newest fits in 15 (10) before 20>15
        assert_eq!(r.masked, vec![1, 2, 3]);
        assert_eq!(r.summary.as_deref(), Some("summary of [1, 2, 3]"));
    }

    // Robust: hybrid with nothing masked attaches no summary.
    #[test]
    fn hybrid_no_mask_no_summary_robust() {
        let t = turns(&[(1, 5)]);
        let r = select_retained_hybrid(&t, 100, |_| "unused".to_string());
        assert_eq!(r.kept, vec![1]);
        assert!(r.summary.is_none());
    }
}

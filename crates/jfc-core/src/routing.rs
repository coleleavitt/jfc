//! Model-cascade routing + budget knapsack — deterministic cost/quality policy.
//!
//! Two classic ideas, both reduced to pure functions:
//!
//! - **Model cascade** (*FrugalGPT*, arXiv:2305.05176; *BEST-Route*): try
//!   models cheapest-first and stop at the first whose quality clears a
//!   threshold, escalating only when needed. [`cascade_pick`] is the selection
//!   rule given each stage's predicted quality and cost.
//! - **0/1 budget knapsack**: given a set of candidate calls each with a cost
//!   and a value, pick the subset that maximises total value within a token (or
//!   dollar) budget. [`knapsack_select`] is the standard DP, returning the
//!   chosen indices. jfc uses this to spread a per-task budget across competing
//!   subagents/tools.
//!
//! Both are intentionally model-free: the *predictions* (quality scores, value
//! estimates) come from the caller; the *decision* made from them is exact and
//! testable.

/// Pick a stage from a cost-ordered cascade.
///
/// `stage_best_scores[i]` is the predicted quality of stage `i` and
/// `stage_costs[i]` its cost; stages are assumed listed cheapest-first but this
/// function does not require it — it returns the **cheapest** stage whose score
/// is `>= threshold`, breaking ties toward the earlier (cheaper-listed) stage.
/// If no stage clears the threshold, it returns the index of the
/// highest-scoring stage (best effort), or `None` only when there are no stages.
///
/// `stage_costs` shorter than `stage_best_scores` treats missing costs as
/// `f64::INFINITY` (never preferred); extra costs are ignored.
pub fn cascade_pick(
    stage_best_scores: &[f64],
    stage_costs: &[f64],
    threshold: f64,
) -> Option<usize> {
    if stage_best_scores.is_empty() {
        return None;
    }
    let cost_of = |i: usize| stage_costs.get(i).copied().unwrap_or(f64::INFINITY);

    // Among stages clearing the threshold, choose minimum cost (then earliest).
    let mut best: Option<usize> = None;
    for (i, &score) in stage_best_scores.iter().enumerate() {
        if score >= threshold {
            match best {
                Some(b) if cost_of(b) <= cost_of(i) => {}
                _ => best = Some(i),
            }
        }
    }
    if let Some(b) = best {
        return Some(b);
    }

    // None clears the bar: fall back to the highest-scoring stage (earliest on
    // a tie) — the best we can do before giving up.
    let mut top = 0usize;
    for i in 1..stage_best_scores.len() {
        if stage_best_scores[i] > stage_best_scores[top] {
            top = i;
        }
    }
    Some(top)
}

/// 0/1 knapsack: choose a subset of `items` (each `(cost, value)`) maximising
/// total value with total cost `<= budget`. Returns the chosen indices in
/// ascending order. Costs are integer token/credit units.
///
/// Standard `O(n * budget)` DP. Items with `cost == 0` and positive value are
/// always taken; items with `cost > budget` are never taken. On a value tie the
/// DP prefers the lexicographically-earlier item set (it only overwrites a cell
/// on a strict improvement).
pub fn knapsack_select(items: &[(u64, i64)], budget: u64) -> Vec<usize> {
    let n = items.len();
    if n == 0 || budget == 0 {
        // budget 0 still admits zero-cost positive-value items.
        return items
            .iter()
            .enumerate()
            .filter(|(_, (c, v))| *c == 0 && *v > 0)
            .map(|(i, _)| i)
            .collect();
    }
    reconstruct(items, budget as usize)
}

/// `O(n * cap)` value DP with a backtrack reconstruction. `value[i][w]` is the
/// best total value using items `[0..i)` within capacity `w`; we then walk back
/// from `(n, cap)`, taking item `i-1` only when doing so strictly improved its
/// cell, then fold in any zero-cost positive-value items.
fn reconstruct(items: &[(u64, i64)], cap: usize) -> Vec<usize> {
    let n = items.len();
    // value[i][w] = best value using items [0..i) with capacity w.
    let mut value = vec![vec![0i64; cap + 1]; n + 1];
    for i in 1..=n {
        let (cost, val) = items[i - 1];
        let c = cost as usize;
        for w in 0..=cap {
            let mut best = value[i - 1][w]; // skip item i-1
            if c <= w {
                let with = value[i - 1][w - c] + val;
                if with > best {
                    best = with;
                }
            }
            value[i][w] = best;
        }
    }
    // Backtrack: prefer NOT taking on ties (so the skip branch wins unless
    // taking strictly improves), giving a deterministic minimal-index-free but
    // stable selection.
    let mut chosen = Vec::new();
    let mut w = cap;
    for i in (1..=n).rev() {
        let (cost, val) = items[i - 1];
        let c = cost as usize;
        let took =
            c <= w && value[i - 1][w - c] + val == value[i][w] && value[i][w] != value[i - 1][w];
        if took {
            chosen.push(i - 1);
            w -= c;
        }
    }
    chosen.sort_unstable();
    // Append zero-cost positive-value items that the value-strict backtrack may
    // have skipped (they never change `value` so the != check excludes them).
    for (i, &(c, v)) in items.iter().enumerate() {
        if c == 0 && v > 0 && !chosen.contains(&i) {
            chosen.push(i);
        }
    }
    chosen.sort_unstable();
    chosen
}

#[cfg(test)]
mod tests {
    use super::*;

    // Normal: the cheapest stage clearing the threshold is chosen, even though
    // a later, costlier stage scores higher.
    #[test]
    fn cascade_picks_cheapest_passing_stage_normal() {
        // stage 0: cheap but below bar; stage 1: cheap-ish and passes; stage 2:
        // pricey and best.
        let scores = [0.4, 0.8, 0.95];
        let costs = [1.0, 2.0, 10.0];
        assert_eq!(cascade_pick(&scores, &costs, 0.75), Some(1));
    }

    // Robust: when nothing clears the bar, fall back to the highest-scoring
    // stage rather than failing.
    #[test]
    fn cascade_falls_back_to_best_when_none_pass_robust() {
        let scores = [0.2, 0.5, 0.49];
        let costs = [1.0, 2.0, 3.0];
        assert_eq!(cascade_pick(&scores, &costs, 0.9), Some(1));
    }

    // Robust: empty cascade yields None.
    #[test]
    fn cascade_empty_is_none_robust() {
        assert_eq!(cascade_pick(&[], &[], 0.5), None);
    }

    // Normal: knapsack chooses the optimal subset within budget.
    #[test]
    fn knapsack_optimal_subset_normal() {
        // classic: capacity 5; items (cost,value): (2,3),(3,4),(4,5),(5,6).
        // optimum is items 0+1 (cost 5, value 7).
        let items = [(2u64, 3i64), (3, 4), (4, 5), (5, 6)];
        let chosen = knapsack_select(&items, 5);
        assert_eq!(chosen, vec![0, 1]);
        let total_value: i64 = chosen.iter().map(|&i| items[i].1).sum();
        assert_eq!(total_value, 7);
    }

    // Robust: an item costing more than the budget is never selected.
    #[test]
    fn knapsack_skips_unaffordable_item_robust() {
        let items = [(100u64, 999i64), (2, 3)];
        assert_eq!(knapsack_select(&items, 5), vec![1]);
    }

    // Robust: zero budget still admits zero-cost positive-value items and
    // nothing else.
    #[test]
    fn knapsack_zero_budget_takes_only_free_items_robust() {
        let items = [(0u64, 5i64), (1, 100), (0, -1)];
        assert_eq!(knapsack_select(&items, 0), vec![0]);
    }

    // Normal: an ample budget takes every positive-value item.
    #[test]
    fn knapsack_ample_budget_takes_all_positive_normal() {
        let items = [(1u64, 1i64), (1, 1), (1, 1)];
        assert_eq!(knapsack_select(&items, 100), vec![0, 1, 2]);
    }
}

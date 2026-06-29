//! AFlow — Monte-Carlo Tree Search over workflow *structure*.
//!
//! From *AFlow: Automating Agentic Workflow Generation* (arXiv:2410.10762):
//! represent a candidate workflow as a node in a search tree, let an optimiser
//! LLM **mutate** a selected node into a child variant, **score** each variant
//! on a held-out task set, and **back-propagate** which edit helped via an
//! experience log. Selection uses a *Soft Mixed Probability* — a blend of
//! uniform exploration and score-weighted exploitation — rather than pure UCT,
//! so the optimiser keeps trying structurally-different variants.
//!
//! jfc can already *select* among saved workflow variants and *predict* fan-out
//! cost ([`crate::fanout`]); what it lacked is optimising the workflow graph
//! itself. This module is the deterministic controller for that loop, generic
//! over the workflow representation:
//!
//! - [`Mutator`] proposes a child `State` from a parent + the experience log
//!   (the LLM rewrite in production).
//! - [`Evaluator`] scores a `State` (a held-out eval run in production).
//! - selection is injected as `choose: FnMut(&[f64]) -> usize`; the default
//!   [`argmax`] makes the whole search deterministic for tests, while
//!   production can pass a softmax-sampling RNG closure.
//!
//! Nothing here calls an LLM or an RNG directly, so the tree growth, scoring,
//! experience back-prop, and iteration budget are all exactly tested.

/// A back-propagated experience record: one mutation and the score delta it
/// produced relative to its parent. The optimiser feeds these back into the
/// next [`Mutator::expand`] so helpful edits are reinforced.
#[derive(Debug, Clone, PartialEq)]
pub struct Experience {
    /// Index of the parent node the mutation was applied to.
    pub parent: usize,
    /// Score of the parent before mutation.
    pub parent_score: f64,
    /// Score of the resulting child.
    pub child_score: f64,
}

impl Experience {
    /// Score improvement of the child over its parent (may be negative).
    pub fn delta(&self) -> f64 {
        self.child_score - self.parent_score
    }
}

/// Proposes a child workflow from a parent and the accumulated experience.
pub trait Mutator<S> {
    fn expand(&mut self, parent: &S, experience: &[Experience]) -> S;
}

/// Scores a workflow. Higher is better. In production this averages a held-out
/// eval run (optionally multiple runs for variance — see `runs_per_node`).
pub trait Evaluator<S> {
    fn score(&self, state: &S) -> f64;
}

struct Node<S> {
    state: S,
    score: f64,
}

/// The result of a search.
#[derive(Debug, Clone)]
pub struct SearchResult<S> {
    /// The highest-scoring workflow found (including the root).
    pub best: S,
    /// Its score.
    pub best_score: f64,
    /// The full experience log, one entry per mutation, in discovery order.
    pub experience: Vec<Experience>,
    /// Total nodes expanded (root + children).
    pub nodes: usize,
}

/// Argmax selection: index of the maximum, earliest on a tie. The default
/// `choose` policy, making search deterministic.
pub fn argmax(scores: &[f64]) -> usize {
    let mut best = 0usize;
    for i in 1..scores.len() {
        if scores[i] > scores[best] {
            best = i;
        }
    }
    best
}

/// Soft Mixed Probability weights: `lambda * uniform + (1 - lambda) *
/// softmax(score / temperature)`, normalised. Exposed so production callers can
/// sample from this distribution; the search itself stays deterministic by
/// taking `argmax` over these weights unless a sampling `choose` is supplied.
pub fn soft_mixed_probability(scores: &[f64], lambda: f64, temperature: f64) -> Vec<f64> {
    let n = scores.len();
    if n == 0 {
        return Vec::new();
    }
    let lambda = lambda.clamp(0.0, 1.0);
    let temp = if temperature <= 0.0 { 1.0 } else { temperature };
    let uniform = 1.0 / n as f64;

    // Numerically-stable softmax over score/temp.
    let max = scores.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    let exps: Vec<f64> = scores.iter().map(|s| ((s - max) / temp).exp()).collect();
    let sum: f64 = exps.iter().sum();
    let softmax: Vec<f64> = if sum > 0.0 {
        exps.iter().map(|e| e / sum).collect()
    } else {
        vec![uniform; n]
    };

    softmax
        .iter()
        .map(|sm| lambda * uniform + (1.0 - lambda) * sm)
        .collect()
}

/// Run AFlow-style search.
///
/// Starting from `root`, repeat for `iters` iterations: build the per-node
/// selection weights with [`soft_mixed_probability`], pick a node via `choose`,
/// expand it with `mutator`, score the child with `evaluator`, record an
/// [`Experience`], and add the child to the tree. Returns the best node seen.
///
/// `lambda` mixes exploration/exploitation; `temperature` sharpens the softmax.
/// Passing [`argmax`] as `choose` yields a deterministic hill-climb over the
/// mixed weights; passing an RNG-backed sampler yields stochastic AFlow search.
pub fn search<S: Clone>(
    root: S,
    iters: usize,
    mutator: &mut dyn Mutator<S>,
    evaluator: &dyn Evaluator<S>,
    lambda: f64,
    temperature: f64,
    mut choose: impl FnMut(&[f64]) -> usize,
) -> SearchResult<S> {
    let root_score = evaluator.score(&root);
    let mut nodes: Vec<Node<S>> = vec![Node {
        state: root,
        score: root_score,
    }];
    let mut experience: Vec<Experience> = Vec::new();

    for _ in 0..iters {
        let scores: Vec<f64> = nodes.iter().map(|n| n.score).collect();
        let weights = soft_mixed_probability(&scores, lambda, temperature);
        let mut pick = choose(&weights);
        if pick >= nodes.len() {
            pick = nodes.len() - 1; // guard a misbehaving choose
        }

        let parent_score = nodes[pick].score;
        let child_state = mutator.expand(&nodes[pick].state, &experience);
        let child_score = evaluator.score(&child_state);

        experience.push(Experience {
            parent: pick,
            parent_score,
            child_score,
        });
        nodes.push(Node {
            state: child_state,
            score: child_score,
        });
    }

    // Best node overall (earliest on a tie for determinism).
    let mut best_idx = 0usize;
    for i in 1..nodes.len() {
        if nodes[i].score > nodes[best_idx].score {
            best_idx = i;
        }
    }
    SearchResult {
        best: nodes[best_idx].state.clone(),
        best_score: nodes[best_idx].score,
        experience,
        nodes: nodes.len(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // A workflow is just an integer here; the "ideal" is some target value and
    // score is negative distance to it. The mutator steps one toward the target,
    // so search should climb to it.
    #[derive(Clone)]
    struct IntFlow(i64);

    struct StepToward {
        target: i64,
    }
    impl Mutator<IntFlow> for StepToward {
        fn expand(&mut self, parent: &IntFlow, _exp: &[Experience]) -> IntFlow {
            let dir = (self.target - parent.0).signum();
            IntFlow(parent.0 + dir)
        }
    }

    struct NegDistance {
        target: i64,
    }
    impl Evaluator<IntFlow> for NegDistance {
        fn score(&self, s: &IntFlow) -> f64 {
            -((self.target - s.0).abs() as f64)
        }
    }

    // Normal: search climbs the deterministic landscape toward the optimum.
    #[test]
    fn search_improves_over_landscape_normal() {
        let mut mutator = StepToward { target: 5 };
        let evaluator = NegDistance { target: 5 };
        // argmax over the mixed weights -> always expand the current best.
        let result = search(IntFlow(0), 5, &mut mutator, &evaluator, 0.0, 1.0, argmax);
        assert_eq!(result.best_score, 0.0); // reached target 5 within 5 steps
        assert_eq!(result.best.0, 5);
    }

    // Normal: the experience log records one delta per iteration.
    #[test]
    fn experience_log_records_deltas_normal() {
        let mut mutator = StepToward { target: 3 };
        let evaluator = NegDistance { target: 3 };
        let result = search(IntFlow(0), 3, &mut mutator, &evaluator, 0.0, 1.0, argmax);
        assert_eq!(result.experience.len(), 3);
        // Each step toward the target is a positive improvement.
        assert!(result.experience.iter().all(|e| e.delta() > 0.0));
    }

    // Robust: the iteration budget is respected — exactly `iters` expansions, so
    // `nodes == iters + 1` (root included).
    #[test]
    fn respects_iteration_budget_robust() {
        let mut mutator = StepToward { target: 100 };
        let evaluator = NegDistance { target: 100 };
        let result = search(IntFlow(0), 4, &mut mutator, &evaluator, 0.5, 1.0, argmax);
        assert_eq!(result.nodes, 5);
        assert_eq!(result.experience.len(), 4);
    }

    // Robust: zero iterations returns the root unchanged.
    #[test]
    fn zero_iters_returns_root_robust() {
        let mut mutator = StepToward { target: 9 };
        let evaluator = NegDistance { target: 9 };
        let result = search(IntFlow(2), 0, &mut mutator, &evaluator, 0.5, 1.0, argmax);
        assert_eq!(result.best.0, 2);
        assert_eq!(result.nodes, 1);
        assert!(result.experience.is_empty());
    }

    // Robust: soft-mixed-probability is a normalised distribution that blends
    // uniform and softmax per lambda.
    #[test]
    fn soft_mixed_probability_normalised_robust() {
        let w = soft_mixed_probability(&[1.0, 2.0, 3.0], 0.5, 1.0);
        let sum: f64 = w.iter().sum();
        assert!((sum - 1.0).abs() < 1e-9);
        // Higher score -> higher weight (exploitation still present at lambda .5).
        assert!(w[2] > w[0]);
        // lambda = 1.0 -> pure uniform.
        let u = soft_mixed_probability(&[1.0, 5.0, -3.0], 1.0, 1.0);
        assert!((u[0] - u[1]).abs() < 1e-9 && (u[1] - u[2]).abs() < 1e-9);
    }

    // Robust: a `choose` that returns an out-of-range index is clamped, not a
    // panic.
    #[test]
    fn out_of_range_choice_is_clamped_robust() {
        let mut mutator = StepToward { target: 1 };
        let evaluator = NegDistance { target: 1 };
        let result = search(IntFlow(0), 2, &mut mutator, &evaluator, 0.0, 1.0, |_| 999);
        assert_eq!(result.nodes, 3);
    }
}

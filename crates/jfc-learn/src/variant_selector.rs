//! Eval-driven prompt/policy compilation — a DSPy-style "teleprompter".
//!
//! DSPy (*Compiling Declarative LM Calls into Self-Improving Pipelines*,
//! arXiv:2310.03714) compiles a pipeline by scoring candidate prompt variants
//! against a labelled eval set and keeping the best. DSPy *Assertions*
//! (arXiv:2312.13382) add hard constraints a variant must satisfy on every
//! case. This module is the deterministic core of that loop for jfc, and is the
//! mechanism behind the project rule that *"agent prompt or policy changes need
//! eval coverage, not just prose review"* (CLAUDE.md).
//!
//! The scoring/evaluation itself is supplied by the caller via
//! [`VariantEvaluator`] — in production that wraps an LLM run plus the
//! [`crate::verifier::PromotionVerifier`] contracts (whose violations map to
//! [`CaseOutcome::violated_constraint`]); in tests it's a deterministic mock.
//! The selection policy here — aggregate, disqualify on any hard violation,
//! rank, pick the winner — is pure and fully tested.

/// A candidate prompt/policy variant to evaluate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromptVariant {
    pub name: String,
    pub system_prompt: String,
}

/// One labelled eval fixture.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvalCase {
    pub name: String,
    pub input: String,
    pub expected: String,
}

/// Outcome of evaluating one variant against one case.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CaseOutcome {
    /// Quality score in `[0, 1]`.
    pub score: f64,
    /// Whether the case is considered passed (e.g. score over a bar, or an
    /// exact match).
    pub passed: bool,
    /// Whether a *hard constraint* (a DSPy assertion / a
    /// [`crate::verifier::PromotionVerifier`] contract) was violated. Any
    /// violation disqualifies the whole variant regardless of score.
    pub violated_constraint: bool,
}

/// Supplies the actual evaluation of a variant on a case. Implementations are
/// expected to be deterministic for a given `(variant, case)` so compilation is
/// reproducible.
pub trait VariantEvaluator {
    fn evaluate(&self, variant: &PromptVariant, case: &EvalCase) -> CaseOutcome;
}

/// Aggregated result for one variant across the whole eval set.
#[derive(Debug, Clone, PartialEq)]
pub struct VariantScore {
    pub name: String,
    /// Mean quality score across cases (0.0 when there are no cases).
    pub mean_score: f64,
    /// Fraction of cases passed (0.0 when there are no cases).
    pub pass_rate: f64,
    /// True if the variant violated a hard constraint on any case.
    pub disqualified: bool,
}

/// The output of a compile run.
#[derive(Debug, Clone, PartialEq)]
pub struct CompileReport {
    /// Per-variant scores, ranked best-first (qualified before disqualified,
    /// then by `mean_score`, then `pass_rate`, then name for determinism).
    pub ranked: Vec<VariantScore>,
    /// The selected winner — the top-ranked *qualified* variant, if any.
    pub winner: Option<String>,
}

/// Holds the eval fixtures and runs the compile/select loop.
#[derive(Debug, Clone, Default)]
pub struct Teleprompter {
    cases: Vec<EvalCase>,
}

impl Teleprompter {
    pub fn new(cases: Vec<EvalCase>) -> Self {
        Self { cases }
    }

    /// Score each variant over the eval set and select the best qualified one.
    pub fn compile(
        &self,
        variants: &[PromptVariant],
        evaluator: &dyn VariantEvaluator,
    ) -> CompileReport {
        let mut ranked: Vec<VariantScore> = variants
            .iter()
            .map(|v| self.score_variant(v, evaluator))
            .collect();

        ranked.sort_by(|a, b| {
            // Qualified variants first.
            a.disqualified
                .cmp(&b.disqualified)
                // Then higher mean score.
                .then(
                    b.mean_score
                        .partial_cmp(&a.mean_score)
                        .unwrap_or(std::cmp::Ordering::Equal),
                )
                // Then higher pass rate.
                .then(
                    b.pass_rate
                        .partial_cmp(&a.pass_rate)
                        .unwrap_or(std::cmp::Ordering::Equal),
                )
                // Finally name, so ties are deterministic.
                .then_with(|| a.name.cmp(&b.name))
        });

        let winner = ranked
            .iter()
            .find(|s| !s.disqualified)
            .map(|s| s.name.clone());

        CompileReport { ranked, winner }
    }

    fn score_variant(
        &self,
        variant: &PromptVariant,
        evaluator: &dyn VariantEvaluator,
    ) -> VariantScore {
        if self.cases.is_empty() {
            return VariantScore {
                name: variant.name.clone(),
                mean_score: 0.0,
                pass_rate: 0.0,
                disqualified: false,
            };
        }
        let mut total = 0.0;
        let mut passed = 0usize;
        let mut disqualified = false;
        for case in &self.cases {
            let outcome = evaluator.evaluate(variant, case);
            total += outcome.score;
            if outcome.passed {
                passed += 1;
            }
            if outcome.violated_constraint {
                disqualified = true;
            }
        }
        let n = self.cases.len() as f64;
        VariantScore {
            name: variant.name.clone(),
            mean_score: total / n,
            pass_rate: passed as f64 / n,
            disqualified,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn variant(name: &str) -> PromptVariant {
        PromptVariant {
            name: name.to_string(),
            system_prompt: format!("prompt for {name}"),
        }
    }

    fn cases(names: &[&str]) -> Vec<EvalCase> {
        names
            .iter()
            .map(|n| EvalCase {
                name: n.to_string(),
                input: format!("in-{n}"),
                expected: format!("out-{n}"),
            })
            .collect()
    }

    /// Mock evaluator: per-variant fixed score, with an optional constraint
    /// violation flag. Deterministic.
    struct MockEval {
        scores: HashMap<String, f64>,
        violators: HashMap<String, bool>,
    }
    impl VariantEvaluator for MockEval {
        fn evaluate(&self, variant: &PromptVariant, _case: &EvalCase) -> CaseOutcome {
            let score = *self.scores.get(&variant.name).unwrap_or(&0.0);
            CaseOutcome {
                score,
                passed: score >= 0.5,
                violated_constraint: *self.violators.get(&variant.name).unwrap_or(&false),
            }
        }
    }

    // Normal: the highest-scoring variant wins, and the report is ranked
    // best-first.
    #[test]
    fn picks_highest_scoring_variant_normal() {
        let tp = Teleprompter::new(cases(&["c1", "c2"]));
        let ev = MockEval {
            scores: HashMap::from([("a".into(), 0.4), ("b".into(), 0.9), ("c".into(), 0.7)]),
            violators: HashMap::new(),
        };
        let report = tp.compile(&[variant("a"), variant("b"), variant("c")], &ev);
        assert_eq!(report.winner.as_deref(), Some("b"));
        assert_eq!(
            report.ranked.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(),
            vec!["b", "c", "a"]
        );
        assert!((report.ranked[0].mean_score - 0.9).abs() < 1e-9);
        assert!((report.ranked[0].pass_rate - 1.0).abs() < 1e-9);
    }

    // Robust: a variant that violates a hard constraint is disqualified and
    // cannot win, even with the top score.
    #[test]
    fn constraint_violation_disqualifies_robust() {
        let tp = Teleprompter::new(cases(&["c1"]));
        let ev = MockEval {
            scores: HashMap::from([("hi".into(), 0.99), ("safe".into(), 0.6)]),
            violators: HashMap::from([("hi".into(), true)]),
        };
        let report = tp.compile(&[variant("hi"), variant("safe")], &ev);
        // "hi" scores higher but is disqualified -> "safe" wins.
        assert_eq!(report.winner.as_deref(), Some("safe"));
        assert!(report.ranked.iter().find(|s| s.name == "hi").unwrap().disqualified);
    }

    // Robust: when every variant is disqualified there is no winner.
    #[test]
    fn no_winner_when_all_disqualified_robust() {
        let tp = Teleprompter::new(cases(&["c1"]));
        let ev = MockEval {
            scores: HashMap::from([("x".into(), 0.8), ("y".into(), 0.8)]),
            violators: HashMap::from([("x".into(), true), ("y".into(), true)]),
        };
        let report = tp.compile(&[variant("x"), variant("y")], &ev);
        assert!(report.winner.is_none());
    }

    // Normal: equal scores break ties by name for reproducibility.
    #[test]
    fn ties_broken_by_name_normal() {
        let tp = Teleprompter::new(cases(&["c1"]));
        let ev = MockEval {
            scores: HashMap::from([("zeta".into(), 0.7), ("alpha".into(), 0.7)]),
            violators: HashMap::new(),
        };
        let report = tp.compile(&[variant("zeta"), variant("alpha")], &ev);
        assert_eq!(report.winner.as_deref(), Some("alpha"));
    }

    // Robust: an empty eval set yields zero scores and (vacuously) a winner,
    // but never panics — guarding the divide-by-zero.
    #[test]
    fn empty_eval_set_is_safe_robust() {
        let tp = Teleprompter::new(vec![]);
        let ev = MockEval {
            scores: HashMap::new(),
            violators: HashMap::new(),
        };
        let report = tp.compile(&[variant("only")], &ev);
        assert_eq!(report.winner.as_deref(), Some("only"));
        assert_eq!(report.ranked[0].mean_score, 0.0);
    }
}

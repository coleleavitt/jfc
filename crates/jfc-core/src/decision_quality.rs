//! Decision-Quality metric + deterministic specialist chain.
//!
//! From the LLM incident-response study (*"Decision-quality, not chat-quality"*,
//! a fixed Diagnosis → Plan → Risk specialist chain with a **non-LLM
//! aggregator** reached ~100% actionable output vs ~1.7% for a single
//! free-chat model at equal latency). Two transferable pieces:
//!
//! - **The DQ score.** `DQ = w_v·Validity + w_s·Specificity + w_c·Correctness`,
//!   default weights `0.4 / 0.3 / 0.3`. A coding-relevant reward signal that can
//!   drive the [`crate::workflow_search`] / teleprompter loops, replacing fuzzy
//!   "looks good" judgements.
//! - **The deterministic specialist chain.** An ordered list of pure stages,
//!   each `&str -> String`, composed by a fixed aggregator rather than a
//!   free-form planner. The *ordering and aggregation* are deterministic; only
//!   the stage bodies (LLM calls in production) vary, and here they're injected.

/// The three sub-scores of a decision's quality, each in `[0, 1]`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DecisionQuality {
    /// Is the decision well-formed / on-task (vs hallucinated or off-topic)?
    pub validity: f64,
    /// Is it concrete and actionable (vs vague boilerplate)?
    pub specificity: f64,
    /// Is it factually right against ground truth / verification?
    pub correctness: f64,
}

/// Weights for the three DQ components. Need not sum to 1 — [`DecisionQuality::score`]
/// normalises by their total so callers can pass raw importances.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DqWeights {
    pub validity: f64,
    pub specificity: f64,
    pub correctness: f64,
}

impl Default for DqWeights {
    fn default() -> Self {
        Self {
            validity: 0.4,
            specificity: 0.3,
            correctness: 0.3,
        }
    }
}

impl DecisionQuality {
    /// Clamp each component into `[0, 1]`.
    pub fn new(validity: f64, specificity: f64, correctness: f64) -> Self {
        Self {
            validity: validity.clamp(0.0, 1.0),
            specificity: specificity.clamp(0.0, 1.0),
            correctness: correctness.clamp(0.0, 1.0),
        }
    }

    /// Weighted score in `[0, 1]`, normalised by the weight total. Returns 0.0
    /// when all weights are zero (degenerate).
    pub fn score(&self, w: DqWeights) -> f64 {
        let total = w.validity + w.specificity + w.correctness;
        if total <= 0.0 {
            return 0.0;
        }
        (self.validity * w.validity
            + self.specificity * w.specificity
            + self.correctness * w.correctness)
            / total
    }

    /// Convenience: score with the default `0.4 / 0.3 / 0.3` weights.
    pub fn default_score(&self) -> f64 {
        self.score(DqWeights::default())
    }
}

/// One named stage in a specialist chain. The body is any `Fn(&str) -> String`
/// — a pure transform in tests, an LLM specialist in production.
pub struct Stage {
    pub name: String,
    pub run: Box<dyn Fn(&str) -> String>,
}

impl Stage {
    pub fn new(name: impl Into<String>, run: impl Fn(&str) -> String + 'static) -> Self {
        Self {
            name: name.into(),
            run: Box::new(run),
        }
    }
}

/// A fixed, ordered pipeline of specialists composed by a deterministic
/// aggregator. Unlike a free-form planner, the stage order is immutable and the
/// aggregation is a pure function of the stage outputs.
pub struct SpecialistChain {
    stages: Vec<Stage>,
}

/// The labelled output of running a chain.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChainOutput {
    /// `(stage_name, output)` in execution order.
    pub steps: Vec<(String, String)>,
}

impl SpecialistChain {
    pub fn new(stages: Vec<Stage>) -> Self {
        Self { stages }
    }

    /// Run every stage in order on the original `input` (each stage is
    /// independent — the incident-response chain runs specialists on the same
    /// incident, not on each other's prose) and collect labelled outputs.
    pub fn run(&self, input: &str) -> ChainOutput {
        let steps = self
            .stages
            .iter()
            .map(|s| (s.name.clone(), (s.run)(input)))
            .collect();
        ChainOutput { steps }
    }

    /// Deterministic non-LLM aggregator: join the labelled stage outputs as
    /// `"NAME: output"` lines in execution order. A fixed format, no model in
    /// the loop — the property that gives the chain its 100%-actionable rate.
    pub fn aggregate(output: &ChainOutput) -> String {
        output
            .steps
            .iter()
            .map(|(name, out)| format!("{name}: {out}"))
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Run then aggregate in one call.
    pub fn run_and_aggregate(&self, input: &str) -> String {
        Self::aggregate(&self.run(input))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Normal: default weights produce the 0.4/0.3/0.3 blend.
    #[test]
    fn default_weighted_score_normal() {
        let dq = DecisionQuality::new(1.0, 0.0, 0.0);
        assert!((dq.default_score() - 0.4).abs() < 1e-9);
        let dq2 = DecisionQuality::new(0.0, 1.0, 1.0);
        assert!((dq2.default_score() - 0.6).abs() < 1e-9);
    }

    // Normal: custom weights are normalised by their total.
    #[test]
    fn custom_weights_normalised_normal() {
        let dq = DecisionQuality::new(1.0, 0.0, 0.0);
        // all weight on validity -> score == validity regardless of magnitude.
        let w = DqWeights {
            validity: 2.0,
            specificity: 0.0,
            correctness: 0.0,
        };
        assert!((dq.score(w) - 1.0).abs() < 1e-9);
    }

    // Robust: components are clamped into [0,1].
    #[test]
    fn components_clamped_robust() {
        let dq = DecisionQuality::new(5.0, -3.0, 0.5);
        assert_eq!(dq.validity, 1.0);
        assert_eq!(dq.specificity, 0.0);
        assert_eq!(dq.correctness, 0.5);
    }

    // Robust: all-zero weights yield 0.0 rather than NaN.
    #[test]
    fn zero_weights_is_zero_not_nan_robust() {
        let dq = DecisionQuality::new(1.0, 1.0, 1.0);
        let w = DqWeights {
            validity: 0.0,
            specificity: 0.0,
            correctness: 0.0,
        };
        assert_eq!(dq.score(w), 0.0);
    }

    // Normal: the chain runs stages in declared order and the aggregator
    // preserves that order with the fixed "NAME: output" format.
    #[test]
    fn chain_runs_in_order_and_aggregates_normal() {
        let chain = SpecialistChain::new(vec![
            Stage::new("diagnosis", |i| format!("diag({i})")),
            Stage::new("plan", |i| format!("plan({i})")),
            Stage::new("risk", |i| format!("risk({i})")),
        ]);
        let out = chain.run("INC");
        assert_eq!(
            out.steps
                .iter()
                .map(|(n, _)| n.as_str())
                .collect::<Vec<_>>(),
            vec!["diagnosis", "plan", "risk"]
        );
        assert_eq!(
            chain.run_and_aggregate("INC"),
            "diagnosis: diag(INC)\nplan: plan(INC)\nrisk: risk(INC)"
        );
    }

    // Robust: an empty chain aggregates to an empty string, no panic.
    #[test]
    fn empty_chain_is_empty_robust() {
        let chain = SpecialistChain::new(vec![]);
        assert_eq!(chain.run_and_aggregate("x"), "");
    }
}

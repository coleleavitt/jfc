//! Fan-out performance prediction — a deterministic gate for subagent spawning.
//!
//! Motivated by *Multi-View Encoders for Performance Prediction of Agentic
//! Workflows* (arXiv:2505.19764), which predicts a workflow's cost/quality
//! *before* running it. We don't train a model offline; instead this is a
//! transparent, deterministic heuristic over the features that are actually
//! available at dispatch time — number of planned agents, their reasoning
//! effort, caller-supplied token estimates, the remaining budget, and the
//! concurrency cap. It answers one question the orchestrator faces before a
//! fan-out: *can we afford all of this, and if not, how much of it?*
//!
//! This complements the hard budget gate in
//! [`jfc-economy`'s `TokenLedger`](../../jfc_economy/ledger): the ledger
//! enforces spend after the fact, this predicts spend before committing to a
//! wave of agents so the orchestrator can trim or defer rather than spawn into
//! a wall.

use crate::agent_def::Effort;

/// One planned subagent in a fan-out.
#[derive(Debug, Clone)]
pub struct PlannedAgent {
    /// Reasoning effort for this agent, if overridden. `None` = treated as
    /// [`Effort::Medium`] for estimation.
    pub effort: Option<Effort>,
    /// Caller's estimate of output tokens this agent will produce. When
    /// `None`, a per-effort default is used.
    pub est_output_tokens: Option<u64>,
}

impl PlannedAgent {
    /// Convenience: an agent at a given effort with no explicit token estimate.
    pub fn at(effort: Effort) -> Self {
        Self {
            effort: Some(effort),
            est_output_tokens: None,
        }
    }
}

/// A planned fan-out plus the budget/concurrency context to gate it against.
#[derive(Debug, Clone)]
pub struct FanoutPlan {
    /// The agents to spawn, in the caller's priority order (highest-value
    /// first). [`FanoutDecision::Trim`] keeps a prefix of this list.
    pub agents: Vec<PlannedAgent>,
    /// Remaining output-token budget. `None` = unbounded.
    pub remaining_budget: Option<u64>,
    /// Max agents that run concurrently (the orchestrator's semaphore cap),
    /// used to estimate how many sequential waves the fan-out takes.
    pub concurrency: usize,
}

/// The gate's verdict on a [`FanoutPlan`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FanoutDecision {
    /// Run the whole fan-out.
    Proceed {
        /// Predicted total output tokens (with safety margin).
        predicted_tokens: u64,
        /// Estimated sequential waves at the given concurrency.
        predicted_waves: u32,
    },
    /// The full set doesn't fit the budget — run only the first `keep` agents
    /// (the highest-priority prefix of `plan.agents`).
    Trim {
        keep: usize,
        /// Predicted tokens for the kept prefix.
        predicted_tokens: u64,
    },
    /// Not enough budget for even the first agent — defer the fan-out.
    Defer { reason: String },
}

/// Deterministic fan-out predictor. `safety_margin` inflates raw estimates to
/// leave headroom (1.2 = assume 20% over the point estimate); it must be ≥ 1.0.
#[derive(Debug, Clone)]
pub struct FanoutPredictor {
    pub safety_margin: f64,
}

impl Default for FanoutPredictor {
    fn default() -> Self {
        Self { safety_margin: 1.2 }
    }
}

impl FanoutPredictor {
    pub fn new(safety_margin: f64) -> Self {
        Self {
            safety_margin: safety_margin.max(1.0),
        }
    }

    /// Per-effort default output-token estimate, used when an agent gives no
    /// explicit estimate. Doubling per tier mirrors how reasoning effort tends
    /// to scale generation length.
    fn effort_default_tokens(effort: Effort) -> u64 {
        match effort {
            Effort::Minimal => 2_000,
            Effort::Low => 4_000,
            Effort::Medium => 8_000,
            Effort::High => 16_000,
            Effort::XHigh => 32_000,
        }
    }

    /// Margin-adjusted token estimate for a single planned agent.
    fn agent_tokens(&self, agent: &PlannedAgent) -> u64 {
        let base = agent
            .est_output_tokens
            .unwrap_or_else(|| Self::effort_default_tokens(agent.effort.unwrap_or(Effort::Medium)));
        ((base as f64) * self.safety_margin).ceil() as u64
    }

    /// Per-agent margin-adjusted estimates, in plan order.
    fn per_agent(&self, plan: &FanoutPlan) -> Vec<u64> {
        plan.agents.iter().map(|a| self.agent_tokens(a)).collect()
    }

    /// Estimated number of sequential waves at the plan's concurrency.
    fn waves(n: usize, concurrency: usize) -> u32 {
        if n == 0 {
            return 0;
        }
        n.div_ceil(concurrency.max(1)) as u32
    }

    /// Predicted total output tokens for the whole plan (with margin).
    pub fn predict_tokens(&self, plan: &FanoutPlan) -> u64 {
        self.per_agent(plan).iter().sum()
    }

    /// Gate a fan-out: proceed if it fits, trim to the affordable prefix, or
    /// defer if not even one agent fits.
    pub fn gate(&self, plan: &FanoutPlan) -> FanoutDecision {
        let per = self.per_agent(plan);
        let total: u64 = per.iter().sum();
        let waves = Self::waves(plan.agents.len(), plan.concurrency);

        let Some(budget) = plan.remaining_budget else {
            return FanoutDecision::Proceed {
                predicted_tokens: total,
                predicted_waves: waves,
            };
        };

        if total <= budget {
            return FanoutDecision::Proceed {
                predicted_tokens: total,
                predicted_waves: waves,
            };
        }

        // Greedily keep the highest-priority prefix that fits.
        let mut acc = 0u64;
        let mut keep = 0usize;
        for cost in &per {
            if acc + cost <= budget {
                acc += cost;
                keep += 1;
            } else {
                break;
            }
        }

        if keep == 0 {
            FanoutDecision::Defer {
                reason: format!(
                    "fan-out needs {}+ tokens for its first agent but only {budget} remain",
                    per.first().copied().unwrap_or(0)
                ),
            }
        } else {
            FanoutDecision::Trim {
                keep,
                predicted_tokens: acc,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plan(efforts: &[Effort], budget: Option<u64>, concurrency: usize) -> FanoutPlan {
        FanoutPlan {
            agents: efforts.iter().map(|e| PlannedAgent::at(*e)).collect(),
            remaining_budget: budget,
            concurrency,
        }
    }

    // Normal: an ample budget proceeds and reports total tokens + waves.
    #[test]
    fn proceeds_within_budget_normal() {
        let p = FanoutPredictor::new(1.0); // no margin for an exact assertion
        let plan = plan(&[Effort::Medium, Effort::Medium], Some(100_000), 4);
        // 2 * 8_000 = 16_000, one wave (2 <= concurrency 4).
        assert_eq!(
            p.gate(&plan),
            FanoutDecision::Proceed {
                predicted_tokens: 16_000,
                predicted_waves: 1,
            }
        );
    }

    // Normal: an unbounded budget always proceeds.
    #[test]
    fn proceeds_when_budget_unbounded_normal() {
        let p = FanoutPredictor::default();
        let plan = plan(&[Effort::XHigh; 5], None, 2);
        assert!(matches!(p.gate(&plan), FanoutDecision::Proceed { .. }));
    }

    // Robust: a tight budget trims to the affordable prefix.
    #[test]
    fn trims_to_affordable_prefix_robust() {
        let p = FanoutPredictor::new(1.0);
        // 4 medium agents = 8_000 each; budget fits exactly 2.
        let plan = plan(&[Effort::Medium; 4], Some(16_000), 8);
        assert_eq!(
            p.gate(&plan),
            FanoutDecision::Trim {
                keep: 2,
                predicted_tokens: 16_000,
            }
        );
    }

    // Robust: budget below even one agent defers.
    #[test]
    fn defers_when_first_agent_unaffordable_robust() {
        let p = FanoutPredictor::new(1.0);
        let plan = plan(&[Effort::High], Some(1_000), 4); // High default 16_000 > 1_000
        assert!(matches!(p.gate(&plan), FanoutDecision::Defer { .. }));
    }

    // Normal: the safety margin inflates estimates (20% over point estimate).
    #[test]
    fn safety_margin_inflates_estimate_normal() {
        let p = FanoutPredictor::new(1.2);
        let plan = plan(&[Effort::Medium], None, 1);
        // 8_000 * 1.2 = 9_600.
        assert_eq!(p.predict_tokens(&plan), 9_600);
    }

    // Normal: waves = ceil(agents / concurrency).
    #[test]
    fn waves_round_up_normal() {
        let p = FanoutPredictor::default();
        let plan = plan(&[Effort::Low; 5], None, 2);
        match p.gate(&plan) {
            FanoutDecision::Proceed {
                predicted_waves, ..
            } => assert_eq!(predicted_waves, 3),
            other => panic!("expected Proceed, got {other:?}"),
        }
    }

    // Robust: an explicit per-agent estimate overrides the effort default.
    #[test]
    fn explicit_estimate_overrides_default_robust() {
        let p = FanoutPredictor::new(1.0);
        let plan = FanoutPlan {
            agents: vec![PlannedAgent {
                effort: Some(Effort::Minimal),
                est_output_tokens: Some(50_000), // override the 2_000 default
            }],
            remaining_budget: None,
            concurrency: 1,
        };
        assert_eq!(p.predict_tokens(&plan), 50_000);
    }
}

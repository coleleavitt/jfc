//! Market status reporting, health scoring, tool types, and swarm integration.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::collusion::CollusionDetector;
use crate::orchestrator::MarketOrchestrator;
use crate::types::AgentId;

// ─── Market Health ───────────────────────────────────────────────────────────

/// Composite market health score (multiplicative).
///
/// Degradation in ANY dimension collapses the composite score,
/// mirroring the CMAG formula: `ECS = C × A × I × F`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarketHealth {
    /// solutions_accepted / solutions_proposed
    pub efficiency: f32,
    /// 1 - (max_trust - min_trust) / 100
    pub fairness: f32,
    /// mean(all_trust_scores) / 100
    pub trust: f32,
    /// remaining_budget / initial_budget
    pub budget_adherence: f32,
    /// Product of all four dimensions.
    pub composite: f32,
}

impl MarketHealth {
    /// Compute health from current orchestrator state.
    pub fn compute(
        orchestrator: &MarketOrchestrator,
        solutions_proposed: u32,
        solutions_accepted: u32,
    ) -> Self {
        let efficiency = if solutions_proposed > 0 {
            solutions_accepted as f32 / solutions_proposed as f32
        } else {
            1.0
        };

        let leaderboard = orchestrator.trust.leaderboard();
        let fairness = if leaderboard.len() >= 2 {
            let max = leaderboard.first().map(|(_, t)| t.score()).unwrap_or(50) as f32;
            let min = leaderboard.last().map(|(_, t)| t.score()).unwrap_or(50) as f32;
            1.0 - (max - min) / 100.0
        } else {
            1.0
        };

        let trust = orchestrator.trust.mean_trust() / 100.0;

        let initial = orchestrator.ledger.remaining() + orchestrator.ledger.total_spent();
        let budget_adherence = if initial > 0 {
            orchestrator.ledger.remaining() as f32 / initial as f32
        } else {
            1.0
        };

        let composite = efficiency * fairness * trust * budget_adherence;

        Self {
            efficiency,
            fairness,
            trust,
            budget_adherence,
            composite,
        }
    }

    /// Is health critically low?
    pub fn is_critical(&self) -> bool {
        self.composite < 0.3
    }
}

// ─── Market Report ───────────────────────────────────────────────────────────

/// Full market status report for the user-facing MarketStatus tool.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarketReport {
    pub total_bounties: usize,
    pub active_bounties: usize,
    pub total_spent: u64,
    pub remaining_budget: u64,
    pub health: MarketHealth,
    pub flagged_agents: Vec<String>,
}

impl MarketReport {
    /// Generate a report from current system state.
    pub fn generate(
        orchestrator: &MarketOrchestrator,
        collusion: &CollusionDetector,
        solutions_proposed: u32,
        solutions_accepted: u32,
    ) -> Self {
        let health =
            MarketHealth::compute(orchestrator, solutions_proposed, solutions_accepted);
        let flagged: Vec<String> = collusion
            .flagged_agents()
            .iter()
            .map(|(id, reason)| format!("{}: {}", id.0, reason))
            .collect();

        Self {
            total_bounties: orchestrator.bounties.audit_log().len(),
            active_bounties: orchestrator.bounties.open_bounties().len(),
            total_spent: orchestrator.ledger.total_spent(),
            remaining_budget: orchestrator.ledger.remaining(),
            health,
            flagged_agents: flagged,
        }
    }
}

// ─── Tool Input/Output Types ─────────────────────────────────────────────────

/// Tool input for `ToolKind::PostBounty` (consumed by jfc-ui).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PostBountyInput {
    pub description: String,
    pub budget: u64,
    pub acceptance_criteria: String,
    pub max_solvers: Option<u8>,
}

/// Tool output for `ToolKind::PostBounty`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PostBountyOutput {
    pub bounty_id: String,
    pub status: String,
}

/// Tool input for `ToolKind::MarketStatus`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarketStatusInput {
    /// If provided, report on a specific bounty; otherwise report global status.
    pub bounty_id: Option<String>,
}

/// Tool output for `ToolKind::MarketStatus`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarketStatusOutput {
    pub report: MarketReport,
}

// ─── Swarm Integration Trait ─────────────────────────────────────────────────

/// Trait for swarm/mailbox integration (dependency inversion).
///
/// Defined in jfc-economy, implemented by jfc-ui. This allows the economy
/// layer to request infrastructure operations without depending on the UI crate.
pub trait SwarmProvider: Send + Sync {
    /// Create a git worktree for a solver agent to work in.
    fn create_worktree(&self, bounty_id: &str, agent_id: &AgentId) -> Option<PathBuf>;

    /// Remove a worktree after the solver completes or is abandoned.
    fn remove_worktree(&self, path: &Path);

    /// Send a message to an agent's mailbox.
    fn send_message(&self, agent_id: &AgentId, message: &str);
}

// ─── Agent Invoker Trait ─────────────────────────────────────────────────────

/// Inputs for spawning a real solver agent.
#[derive(Debug, Clone)]
pub struct SolverPrompt {
    pub bounty_id: String,
    pub bounty_description: String,
    pub acceptance_criteria: String,
    pub agent_id: AgentId,
    pub worktree: Option<PathBuf>,
    /// Hard token cap for this solver's LLM call (carved out of the
    /// bounty budget by the orchestrator).
    pub max_tokens: u64,
}

/// Inputs for spawning a real validator agent against a solution.
#[derive(Debug, Clone)]
pub struct ValidatorPrompt {
    pub bounty_id: String,
    pub bounty_description: String,
    pub solution: crate::types::Solution,
    pub validator_id: AgentId,
    pub max_tokens: u64,
}

/// Outcome of a validator's challenge attempt. Mirrors the
/// `ValidationChallenge` type but lifted to a result the orchestrator
/// can consume — `flaw` is None when the validator could not find
/// any issue (NoFlawFound verdict).
#[derive(Debug, Clone)]
pub struct ValidatorOutcome {
    pub flaw: Option<String>,
    pub test_code: Option<String>,
    pub confidence: f32,
    pub tokens_consumed: u64,
}

/// Trait that turns a solver / validator prompt into a real LLM call.
///
/// Defined in jfc-economy so the orchestrator can drive a full bounty
/// cycle without depending on jfc-ui's provider stack. Implemented in
/// jfc-ui by `tools::EconomyAgentInvoker` which forwards to
/// `provider::stream`. Async because every implementation will hit
/// the network (LLM API or local proxy).
#[async_trait::async_trait]
pub trait AgentInvoker: Send + Sync {
    /// Spawn a solver and collect its proposed solution.
    /// Implementations should respect `prompt.max_tokens` as an
    /// upper bound and set `solution.tokens_consumed` to the actual
    /// figure. On API failure, return Err — the orchestrator will
    /// mark the solver as Abandoned and continue.
    async fn invoke_solver(
        &self,
        prompt: SolverPrompt,
    ) -> Result<crate::types::Solution, String>;

    /// Spawn a validator and collect its challenge outcome. Returns
    /// `ValidatorOutcome { flaw: None, .. }` when the validator
    /// determines the solution is sound — sealed validation means
    /// validators don't see each other's verdicts during this call.
    async fn invoke_validator(
        &self,
        prompt: ValidatorPrompt,
    ) -> Result<ValidatorOutcome, String>;
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::charter::Charter;
    use crate::collusion::CollusionDetector;
    use crate::settlement::SettlementEngine;
    use crate::types::*;

    #[test]
    fn test_market_health_computation() {
        let charter = Charter::default();
        let orchestrator = MarketOrchestrator::with_budget(charter, 10000);
        let health = MarketHealth::compute(&orchestrator, 3, 1);
        assert!(health.efficiency > 0.0 && health.efficiency <= 1.0);
        assert!(health.composite > 0.0);
    }

    #[test]
    fn test_market_health_no_proposals() {
        let charter = Charter::default();
        let orchestrator = MarketOrchestrator::with_budget(charter, 10000);
        let health = MarketHealth::compute(&orchestrator, 0, 0);
        // No proposals → efficiency defaults to 1.0
        assert!((health.efficiency - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn test_market_health_critical() {
        let charter = Charter::default();
        let mut orchestrator = MarketOrchestrator::with_budget(charter, 100);
        let agent = AgentId("spender".into());
        orchestrator.trust.register(agent.clone());
        // Use large token counts so integer division yields non-zero cost
        orchestrator
            .ledger
            .record_usage(&agent, "claude-sonnet-4-20250514", 5_000_000, 1_000_000);
        // efficiency=0 (0 accepted out of 10), so composite=0
        let health = MarketHealth::compute(&orchestrator, 10, 0);
        assert!(health.is_critical());
    }

    #[test]
    fn test_market_report_generation() {
        let charter = Charter::default();
        let orchestrator = MarketOrchestrator::with_budget(charter, 5000);
        let collusion = CollusionDetector::new();
        let report = MarketReport::generate(&orchestrator, &collusion, 5, 2);
        assert_eq!(report.remaining_budget, 5000);
        assert!(report.flagged_agents.is_empty());
    }

    #[test]
    fn test_market_report_with_flagged_agents() {
        let charter = Charter::default();
        let orchestrator = MarketOrchestrator::with_budget(charter, 5000);
        let mut collusion = CollusionDetector::new();
        let rubber_stamper = AgentId("rubber_stamper".into());
        for _ in 0..6 {
            collusion.record(&rubber_stamper, ValidationVerdict::NoFlawFound);
        }
        let report = MarketReport::generate(&orchestrator, &collusion, 5, 2);
        assert!(!report.flagged_agents.is_empty());
        assert!(report.flagged_agents[0].contains("rubber-stamping"));
    }

    #[test]
    fn test_post_bounty_input_serialization() {
        let input = PostBountyInput {
            description: "Add fibonacci".into(),
            budget: 1000,
            acceptance_criteria: "fn fib works".into(),
            max_solvers: Some(2),
        };
        let json = serde_json::to_string(&input).unwrap();
        let back: PostBountyInput = serde_json::from_str(&json).unwrap();
        assert_eq!(back.budget, 1000);
        assert_eq!(back.max_solvers, Some(2));
    }

    #[test]
    fn test_market_status_input_serialization() {
        let input = MarketStatusInput { bounty_id: None };
        let json = serde_json::to_string(&input).unwrap();
        let back: MarketStatusInput = serde_json::from_str(&json).unwrap();
        assert_eq!(back.bounty_id, None);
    }

    #[test]
    fn test_e2e_bounty_lifecycle() {
        // Full lifecycle: post → bid → select → solve → validate → settle → complete
        let charter = Charter::default();
        let mut orchestrator = MarketOrchestrator::with_budget(charter, 10000);

        // 1. Post bounty
        let bounty_id = orchestrator
            .post_bounty(
                "Add fibonacci function".into(),
                1000,
                "fn fibonacci(n: u64) -> u64 works".into(),
                Some(2),
            )
            .unwrap();
        assert_eq!(
            orchestrator.bounty_state(&bounty_id),
            Some(MarketState::Open)
        );

        // 2. Register agents
        let solver_a = AgentId("solver_a".into());
        let solver_b = AgentId("solver_b".into());
        let validator = AgentId("validator".into());
        orchestrator.trust.register(solver_a.clone());
        orchestrator.trust.register(solver_b.clone());
        orchestrator.trust.register(validator.clone());

        // 3. Transition to bidding
        orchestrator
            .bounties
            .transition(&bounty_id, MarketState::Bidding)
            .unwrap();

        // 4. Submit bids
        orchestrator.auction.submit_bid(Bid {
            agent_id: solver_a.clone(),
            bounty_id: bounty_id.clone(),
            price: 500,
            approach: "recursive".into(),
            estimated_time: Duration::from_secs(60),
        });
        orchestrator.auction.submit_bid(Bid {
            agent_id: solver_b.clone(),
            bounty_id: bounty_id.clone(),
            price: 600,
            approach: "iterative".into(),
            estimated_time: Duration::from_secs(45),
        });

        // 5. Select winners
        let winners = orchestrator.auction.select_winners(2, &orchestrator.trust);
        assert_eq!(winners.len(), 2);

        // 6. Transition to executing
        orchestrator
            .bounties
            .transition(&bounty_id, MarketState::Executing)
            .unwrap();

        // 7. Transition to validating
        orchestrator
            .bounties
            .transition(&bounty_id, MarketState::Validating)
            .unwrap();

        // 8. Validator validates (no flaw found — early termination)
        let session_idx = orchestrator
            .validators
            .start_session(validator.clone(), solver_a.clone(), bounty_id.clone())
            .unwrap();
        let session = orchestrator.validators.get_mut(session_idx).unwrap();
        session
            .submit_challenge(ValidationChallenge {
                validator_id: validator.clone(),
                solution_agent_id: solver_a.clone(),
                bounty_id: bounty_id.clone(),
                proposed_flaw: "".into(),
                test_code: None,
                confidence: 0.97,
            })
            .unwrap();
        assert!(session.is_complete());
        assert_eq!(session.verdict(), Some(ValidationVerdict::NoFlawFound));

        // 9. Transition to settling
        orchestrator
            .bounties
            .transition(&bounty_id, MarketState::Settling)
            .unwrap();

        // 10. Simulate solver token usage during execution
        // Rates are per 1M tokens, so use large counts to get non-zero cost
        orchestrator
            .ledger
            .record_usage(&solver_a, "claude-sonnet-4-20250514", 1_000_000, 500_000);

        // 11. Settle
        let verdicts = orchestrator.validators.verdicts();
        let charter = orchestrator.charter.clone();
        let settlement = SettlementEngine::settle(
            &bounty_id,
            1000,
            Some(&solver_a),
            &[solver_b.clone()],
            &verdicts,
            &charter,
            &mut orchestrator.ledger,
            &mut orchestrator.trust,
        );

        assert_eq!(settlement.winner, Some(solver_a.clone()));
        assert!(settlement.total_cost > 0);

        // 12. Complete
        orchestrator
            .bounties
            .transition(&bounty_id, MarketState::Complete)
            .unwrap();
        assert_eq!(
            orchestrator.bounty_state(&bounty_id),
            Some(MarketState::Complete)
        );

        // 13. Verify budget was consumed (from record_usage during execution)
        assert!(orchestrator.ledger.total_spent() > 0);

        // 14. Verify trust updated (winner got +5)
        assert!(orchestrator.trust.get(&solver_a).unwrap().score() > 50);

        // 15. Generate final report
        let collusion = CollusionDetector::new();
        let report = MarketReport::generate(&orchestrator, &collusion, 1, 1);
        assert!(report.total_spent > 0);
        assert!(!report.health.is_critical());
    }

    #[test]
    fn test_e2e_budget_respected() {
        let charter = Charter::default();
        let mut orchestrator = MarketOrchestrator::with_budget(charter, 500);
        // Post a bounty within charter limits
        let _ = orchestrator
            .post_bounty("test".into(), 400, "test".into(), None)
            .unwrap();
        // Nothing spent yet (just posted, no settlement)
        assert_eq!(orchestrator.remaining_budget(), 500);
    }

    #[test]
    fn test_e2e_budget_exceeded_rejected() {
        let charter = Charter::default();
        let mut orchestrator = MarketOrchestrator::with_budget(charter, 500);
        // Try to post bounty exceeding charter max_budget_per_bounty
        let result = orchestrator.post_bounty("big task".into(), 99999, "criteria".into(), None);
        assert!(result.is_err());
    }

    /// Verify SwarmProvider is object-safe (can be used as trait object).
    #[test]
    fn test_swarm_provider_object_safety() {
        struct MockSwarm;
        impl SwarmProvider for MockSwarm {
            fn create_worktree(&self, _: &str, _: &AgentId) -> Option<PathBuf> {
                Some(PathBuf::from("/tmp/mock"))
            }
            fn remove_worktree(&self, _: &Path) {}
            fn send_message(&self, _: &AgentId, _: &str) {}
        }

        let provider: Box<dyn SwarmProvider> = Box::new(MockSwarm);
        let agent = AgentId("test".into());
        let path = provider.create_worktree("bounty-1", &agent);
        assert_eq!(path, Some(PathBuf::from("/tmp/mock")));
    }
}

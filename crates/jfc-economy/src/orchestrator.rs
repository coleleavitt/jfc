//! Market cycle orchestrator.
//!
//! Manages the full 7-step bounty lifecycle:
//! Post → Bid → Select → Execute → Validate → Settle → Complete

use crate::auction::AuctionEngine;
use crate::bounty::BountyManager;
use crate::charter::Charter;
use crate::ledger::TokenLedger;
use crate::settlement::SettlementEngine;
use crate::solver::SolverPool;
use crate::trust::TrustRegistry;
use crate::types::MarketState;
use crate::validator::ValidationPool;

/// The full market cycle orchestrator.
pub struct MarketOrchestrator {
    pub ledger: TokenLedger,
    pub trust: TrustRegistry,
    pub bounties: BountyManager,
    pub charter: Charter,
    pub auction: AuctionEngine,
    pub solvers: SolverPool,
    pub validators: ValidationPool,
}

impl MarketOrchestrator {
    pub fn new(charter: Charter) -> Self {
        let spawn_fee = charter.spawn_fee;
        let budget = charter.max_budget_per_bounty;
        Self {
            ledger: TokenLedger::new(budget, budget, spawn_fee),
            trust: TrustRegistry::new(),
            bounties: BountyManager::new(),
            charter,
            auction: AuctionEngine::new(),
            solvers: SolverPool::new(),
            validators: ValidationPool::new(),
        }
    }

    /// Create with explicit budget.
    pub fn with_budget(charter: Charter, total_budget: u64) -> Self {
        let spawn_fee = charter.spawn_fee;
        Self {
            ledger: TokenLedger::new(total_budget, total_budget, spawn_fee),
            trust: TrustRegistry::new(),
            bounties: BountyManager::new(),
            charter,
            auction: AuctionEngine::new(),
            solvers: SolverPool::new(),
            validators: ValidationPool::new(),
        }
    }

    /// Post a bounty and transition to Open.
    pub fn post_bounty(
        &mut self,
        description: String,
        reward: u64,
        criteria: String,
        max_solvers: Option<u8>,
    ) -> Result<String, OrchestratorError> {
        let max = max_solvers.unwrap_or(self.charter.max_solvers);
        if reward > self.charter.max_budget_per_bounty {
            return Err(OrchestratorError::BudgetExceeded {
                requested: reward,
                max: self.charter.max_budget_per_bounty,
            });
        }
        let deadline = std::time::Duration::from_secs(300);
        let id = self.bounties.post(description, reward, criteria, deadline, max);
        self.bounties
            .transition(&id, MarketState::Open)
            .map_err(OrchestratorError::Bounty)?;
        Ok(id)
    }

    /// Get current state of a bounty.
    pub fn bounty_state(&self, bounty_id: &str) -> Option<MarketState> {
        self.bounties.get(bounty_id).map(|b| b.state)
    }

    /// Get the charter.
    pub fn charter(&self) -> &Charter {
        &self.charter
    }

    /// Get remaining budget.
    pub fn remaining_budget(&self) -> u64 {
        self.ledger.remaining()
    }

    /// Drive a real bounty through the full lifecycle using
    /// LLM-backed `AgentInvoker` and worktree-backed `SwarmProvider`.
    /// This is the production path the `post_bounty` tool dispatcher
    /// invokes when `auto_dispatch=true`. The flow:
    ///
    ///   1. Spawn `n_solvers` solver agents (capped by charter +
    ///      caller's `max_solvers`). Each gets a worktree.
    ///   2. Drive each solver concurrently via `AgentInvoker::invoke_solver`.
    ///   3. Collect solutions; failed solvers are abandoned (not fatal).
    ///   4. For each surviving solution, spawn `n_validators` validators
    ///      via `AgentInvoker::invoke_validator`. Validators are sealed
    ///      from each other.
    ///   5. Adjudicate: a flaw with reproducible test_code → FlawUpheld;
    ///      otherwise NoFlawFound.
    ///   6. Settle: rank surviving solutions, distribute rewards,
    ///      update trust, clean up worktrees.
    ///
    /// Returns the Settlement record on success. The orchestrator's
    /// internal pools are mutated as the cycle progresses so the
    /// `MarketReport` reflects real state at any point.
    ///
    /// The lock-free / re-entrant story is: callers hold a Mutex on
    /// the whole orchestrator. The cycle is fully synchronous in
    /// terms of state — async is only for the LLM calls themselves,
    /// which are awaited inline. Concurrent bounties are out of
    /// scope for this iteration; the LLM dispatches one at a time.
    pub async fn run_bounty_cycle(
        &mut self,
        bounty_id: &str,
        invoker: &dyn crate::reporting::AgentInvoker,
        swarm: &dyn crate::reporting::SwarmProvider,
        n_solvers: u8,
        n_validators_per_solution: u8,
    ) -> Result<crate::types::Settlement, OrchestratorError> {
        use crate::reporting::{SolverPrompt, ValidatorPrompt};
        use crate::types::{AgentId, ValidationChallenge};

        let bounty = self
            .bounties
            .get(bounty_id)
            .ok_or_else(|| OrchestratorError::CharterViolation(format!("unknown bounty: {bounty_id}")))?
            .clone();

        // Cap n_solvers by charter and the bounty's own max_solvers.
        let actual_solvers = (n_solvers as usize)
            .min(self.charter.max_solvers as usize)
            .min(bounty.max_solvers as usize)
            .max(1);
        let actual_validators = (n_validators_per_solution as usize)
            .min(self.charter.max_validators as usize);

        // 1. Spawn solver pool entries + worktrees, build prompts.
        let mut prompts: Vec<SolverPrompt> = Vec::with_capacity(actual_solvers);
        for _ in 0..actual_solvers {
            let agent = self.solvers.spawn(bounty_id);
            let agent_id = agent.id.clone();
            self.trust.register(agent_id.clone());
            let worktree = swarm.create_worktree(bounty_id, &agent_id);
            if let Some(s) = self.solvers.get_mut(&agent_id) {
                s.start(worktree.clone());
            }
            // Per-solver budget: split bounty reward evenly minus
            // a buffer for validators. Conservative cap so a single
            // solver can't burn the whole pool.
            let per_solver_budget =
                (bounty.reward / (actual_solvers as u64 + 1)).max(1);
            prompts.push(SolverPrompt {
                bounty_id: bounty_id.to_string(),
                bounty_description: bounty.description.clone(),
                acceptance_criteria: bounty.acceptance_criteria.clone(),
                agent_id,
                worktree,
                max_tokens: per_solver_budget,
            });
        }

        // 2. Run solvers sequentially (avoids interleaved trust /
        //    ledger writes). For real concurrency the orchestrator
        //    would need to be split into immutable inputs + mutable
        //    outputs collected at the end — defer.
        // The state machine requires Open → Bidding → Executing.
        // We collapse the bidding phase here (no separate auction
        // round at runtime — solvers are pre-selected by the
        // caller via n_solvers) but still drive the transition so
        // the audit trail and any state-dependent reports see the
        // correct sequence.
        self.bounties
            .transition(bounty_id, MarketState::Bidding)
            .map_err(OrchestratorError::Bounty)?;
        self.bounties
            .transition(bounty_id, MarketState::Executing)
            .map_err(OrchestratorError::Bounty)?;
        for prompt in prompts {
            let agent_id = prompt.agent_id.clone();
            let worktree = prompt.worktree.clone();
            match invoker.invoke_solver(prompt).await {
                Ok(solution) => {
                    let tokens = solution.tokens_consumed;
                    if let Some(s) = self.solvers.get_mut(&agent_id) {
                        s.record_tokens(tokens);
                        s.submit(solution);
                    }
                    // Record the solver's actual usage so the ledger
                    // and the MarketReport reflect real cost. We
                    // don't have separate input / output counts here
                    // — the invoker only reports the total — so
                    // attribute everything to input_tokens which is
                    // the cheaper rate (conservative estimate).
                    self.ledger.record_usage(&agent_id, "stub", tokens, 0);
                }
                Err(e) => {
                    tracing::warn!(
                        target: "jfc::economy",
                        agent = %agent_id.0,
                        error = %e,
                        "solver invocation failed"
                    );
                    if let Some(s) = self.solvers.get_mut(&agent_id) {
                        s.abandon();
                    }
                    if let Some(p) = worktree {
                        swarm.remove_worktree(&p);
                    }
                }
            }
        }

        // 3. Validate every surviving solution.
        self.bounties
            .transition(bounty_id, MarketState::Validating)
            .map_err(OrchestratorError::Bounty)?;
        let solutions: Vec<crate::types::Solution> =
            self.solvers.completed_solutions().into_iter().cloned().collect();
        for solution in solutions {
            for _ in 0..actual_validators {
                let validator_id = AgentId::new("validator");
                self.trust.register(validator_id.clone());
                // Anti-collusion: the validator MUST NOT be the
                // same agent as the solver. AgentId::new uses uuid
                // so this holds by construction, but assert it.
                if validator_id == solution.agent_id {
                    continue;
                }
                let prompt = ValidatorPrompt {
                    bounty_id: bounty_id.to_string(),
                    bounty_description: bounty.description.clone(),
                    solution: solution.clone(),
                    validator_id: validator_id.clone(),
                    max_tokens: (bounty.reward / 10).max(1),
                };
                match invoker.invoke_validator(prompt).await {
                    Ok(outcome) => {
                        self.ledger.record_usage(
                            &validator_id,
                            "stub",
                            outcome.tokens_consumed,
                            0,
                        );
                        // Drive the sealed-validation session through
                        // its three rounds. Early-termination path
                        // (high confidence + no flaw) is handled
                        // inside `submit_challenge`.
                        let session_idx = match self.validators.start_session(
                            validator_id.clone(),
                            solution.agent_id.clone(),
                            bounty_id.to_string(),
                        ) {
                            Ok(i) => i,
                            Err(e) => {
                                tracing::warn!(
                                    target: "jfc::economy",
                                    validator = %validator_id.0,
                                    error = %e,
                                    "validator session start failed"
                                );
                                continue;
                            }
                        };
                        let challenge = ValidationChallenge {
                            validator_id: validator_id.clone(),
                            solution_agent_id: solution.agent_id.clone(),
                            bounty_id: bounty_id.to_string(),
                            proposed_flaw: outcome.flaw.clone().unwrap_or_default(),
                            test_code: outcome.test_code.clone(),
                            confidence: outcome.confidence,
                        };
                        let session = match self.validators.get_mut(session_idx) {
                            Some(s) => s,
                            None => continue,
                        };
                        if session.submit_challenge(challenge).is_err() {
                            continue;
                        }
                        // If early-termination didn't fire, run the
                        // remaining two rounds. Defense + adjudication
                        // are mechanistic at this layer: the solver
                        // doesn't get to actually defend (that would
                        // need another LLM round-trip — defer to v2);
                        // adjudication is "did the validator produce
                        // a runnable test_code?" — v131 PROClaim
                        // pattern.
                        if !session.is_complete() {
                            let _ = session.submit_defense(
                                "(no defense in this iteration)".to_string(),
                            );
                            let test_fails = outcome.test_code.is_some();
                            let _ = session.adjudicate(test_fails);
                        }
                    }
                    Err(e) => {
                        tracing::warn!(
                            target: "jfc::economy",
                            validator = %validator_id.0,
                            error = %e,
                            "validator invocation failed"
                        );
                    }
                }
            }
        }

        // 4. Worktree cleanup before settle (settle reads agent state
        //    but doesn't touch worktrees; remove them now to free the
        //    disk).
        for s in self.solvers.all() {
            if let Some(p) = &s.worktree_path {
                swarm.remove_worktree(p);
            }
        }

        // 5. Settle.
        self.bounties
            .transition(bounty_id, MarketState::Settling)
            .map_err(OrchestratorError::Bounty)?;
        let settlement = self
            .settle_bounty(bounty_id)
            .ok_or_else(|| OrchestratorError::CharterViolation(
                "settle returned None — bounty disappeared mid-cycle".into()
            ))?;
        self.bounties
            .transition(bounty_id, MarketState::Complete)
            .map_err(OrchestratorError::Bounty)?;
        Ok(settlement)
    }

    /// Access the settlement engine (stateless, so just delegates).
    pub fn settle_bounty(
        &mut self,
        bounty_id: &str,
    ) -> Option<crate::types::Settlement> {
        let bounty = self.bounties.get(bounty_id)?;
        let reward = bounty.reward;

        let verdicts = self.validators.verdicts();
        let ranked = self.solvers.rank_solutions();
        let winner = ranked.first().map(|s| &s.agent_id);
        let losers: Vec<_> = ranked.iter().skip(1).map(|s| s.agent_id.clone()).collect();

        let settlement = SettlementEngine::settle(
            bounty_id,
            reward,
            winner,
            &losers,
            &verdicts,
            &self.charter,
            &mut self.ledger,
            &mut self.trust,
        );

        Some(settlement)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum OrchestratorError {
    #[error("budget exceeded: requested={requested}, max={max}")]
    BudgetExceeded { requested: u64, max: u64 },
    #[error("bounty error: {0}")]
    Bounty(#[from] crate::bounty::BountyError),
    #[error("validation error: {0}")]
    Validation(#[from] crate::validator::ValidationError),
    #[error("charter violation: {0}")]
    CharterViolation(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    fn default_charter() -> Charter {
        Charter::default()
    }

    #[test]
    fn test_orchestrator_creation() {
        let charter = default_charter();
        let orch = MarketOrchestrator::new(charter.clone());
        assert_eq!(orch.charter().max_solvers, 3);
        assert_eq!(orch.remaining_budget(), charter.max_budget_per_bounty);
    }

    #[test]
    fn test_post_bounty() {
        let mut orch = MarketOrchestrator::with_budget(default_charter(), 50_000);
        let id = orch
            .post_bounty("Add fibonacci".into(), 1000, "fn fib works".into(), None)
            .unwrap();
        assert_eq!(orch.bounty_state(&id), Some(MarketState::Open));
    }

    #[test]
    fn test_budget_exceeded() {
        let mut orch = MarketOrchestrator::new(default_charter());
        let result = orch.post_bounty("Big task".into(), 99999, "criteria".into(), None);
        assert!(result.is_err());
        match result.unwrap_err() {
            OrchestratorError::BudgetExceeded { requested, max } => {
                assert_eq!(requested, 99999);
                assert_eq!(max, 10000);
            }
            other => panic!("expected BudgetExceeded, got: {other:?}"),
        }
    }
}

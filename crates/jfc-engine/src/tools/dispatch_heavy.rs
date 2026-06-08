//! Heavy tool-execution handlers extracted from `execute_tool`.
//!
//! `dispatch::execute_tool` is the O(1) match router; the bounty market
//! cycle carries substantial inline logic (multi-subagent Solve→Validate→
//! Settle round-trips) so it lives here as named functions, keeping the
//! dispatch surface thin. Each fn takes the destructured tool arguments
//! plus the ambient `cwd`, and returns an `ExecutionResult`.

use std::path::Path;

use crate::runtime::ExecutionResult;

use super::economy::{EconomyAgentInvoker, EconomySwarmProvider, apply_winning_solution};
use super::registry::{market_orchestrator, snapshot_active_provider};

/// `post_bounty` tool — register a bounty and (when `auto_dispatch`) drive
/// the full Solve→Validate→Settle cycle.
pub async fn execute_post_bounty(
    description: String,
    budget: u64,
    acceptance_criteria: String,
    max_solvers: Option<u8>,
    auto_dispatch: bool,
    cwd: &Path,
) -> ExecutionResult {
    // The orchestrator's lock is process-global; only one
    // post_bounty runs at a time. That's fine — bounties are
    // posted in the LLM's main loop, not from concurrent
    // subagents. If two tool calls race, the second waits.
    //
    // Posting always succeeds first. If `auto_dispatch=true`,
    // we then drop the lock, run the cycle (which spawns
    // real subagent LLM calls and can take minutes), and
    // re-acquire the lock to read the settlement. Holding
    // the orchestrator mutex across the network round-trips
    // would block /market and concurrent post_bounty calls.
    let bounty_id = {
        let mut orch = market_orchestrator().lock().await;
        match orch.post_bounty(description, budget, acceptance_criteria, max_solvers) {
            Ok(id) => id,
            Err(e) => {
                return ExecutionResult::failure(format!("post_bounty failed: {e}"));
            }
        }
    };
    let max_solvers_text = match max_solvers {
        Some(n) => n.to_string(),
        None => {
            let orch = market_orchestrator().lock().await;
            orch.charter().max_solvers.to_string()
        }
    };
    if !auto_dispatch {
        return ExecutionResult::success(format!(
            "Bounty `{bounty_id}` registered. State=Open, budget={budget} tok, \
             max_solvers={max_solvers_text}. Solvers and validators have NOT \
             run yet — the post step only registers the bounty in the market. \
             To execute the full Post→Solve→Validate→Settle cycle (real LLM \
             subagents compete + cross-validate), call run_bounty with \
             bounty_id=\"{bounty_id}\". Or repost with auto_dispatch=true to \
             register and run in one shot."
        ));
    }
    // Drive the real cycle. The orchestrator mutex is
    // dropped before the await so /market and concurrent
    // post_bounty calls aren't blocked across the network
    // round-trips.
    let Some((provider, model)) = snapshot_active_provider() else {
        return ExecutionResult::success(format!(
            "Bounty `{bounty_id}` registered (budget {budget} tok, \
             max_solvers={max_solvers_text}, State=Open). \
             auto_dispatch=true was requested but the tool layer \
             has no active provider registered, so the cycle did \
             not run. The bounty stays Open — call run_bounty \
             once the provider is wired."
        ));
    };
    let invoker = EconomyAgentInvoker::new(provider, model);
    let swarm = EconomySwarmProvider::new(cwd.to_path_buf());
    // Solver + validator counts: respect the bounty's
    // max_solvers, default to 2 to keep the per-bounty
    // round-trip count predictable. One validator per
    // surviving solution — sealed validation gives one
    // independent verdict per solver.
    let n_solvers = max_solvers.unwrap_or(2).clamp(1, 5);
    tracing::info!(
        target: "jfc::ui::bounty",
        bounty_id = %bounty_id,
        n_solvers = n_solvers,
        cwd = %cwd.display(),
        "post_bounty auto_dispatch: kicking off cycle"
    );
    let cycle_result = {
        let mut orch = market_orchestrator().lock().await;
        orch.run_bounty_cycle(&bounty_id, &invoker, &swarm, n_solvers, 1)
            .await
    };
    match cycle_result {
        Ok(outcome) => {
            let written =
                apply_winning_solution(cwd, &bounty_id, outcome.winning_solution.as_ref());
            tracing::info!(
                target: "jfc::ui::bounty",
                bounty_id = %bounty_id,
                winner = outcome.settlement.winner.as_ref().map(|a| a.0.as_str()).unwrap_or("(none)"),
                files_written = written.files.len(),
                "post_bounty auto_dispatch settled"
            );
            ExecutionResult::success(format!(
                "Bounty `{bounty_id}` settled.\n\
                 Winner: {}\n\
                 Total cost: {} tok\n\
                 Payouts: {}\n\
                 Trust updates: {}\n\
                 {}\n\
                 Run /market to see updated trust + budget.",
                outcome
                    .settlement
                    .winner
                    .as_ref()
                    .map(|a| a.0.as_str())
                    .unwrap_or("(no winning solution)"),
                outcome.settlement.total_cost,
                outcome.settlement.payouts.len(),
                outcome.settlement.trust_updates.len(),
                written.summary,
            ))
        }
        Err(e) => {
            ExecutionResult::failure(format!("auto_dispatch cycle for `{bounty_id}` failed: {e}"))
        }
    }
}

/// `run_bounty` tool — drive an already-posted Open bounty through the full
/// Solve→Validate→Settle cycle.
pub async fn execute_run_bounty(
    bounty_id: String,
    max_solvers: Option<u8>,
    cwd: &Path,
) -> ExecutionResult {
    // Drive an already-posted Open bounty through the full
    // Solve→Validate→Settle cycle. Same code path as
    // PostBounty's auto_dispatch=true, just without the
    // post step. Lets the model post first (cheap registration)
    // and dispatch later when ready, instead of all-or-nothing.
    let Some((provider, model)) = snapshot_active_provider() else {
        return ExecutionResult::failure(
            "run_bounty: no active provider registered with the \
             tool layer. main.rs must call \
             tools::register_active_provider during startup.",
        );
    };
    // Verify the bounty exists and is in Open state before
    // we go through all the worktree + LLM-call setup.
    let state = {
        let orch = market_orchestrator().lock().await;
        orch.bounty_state(&bounty_id)
    };
    let Some(state) = state else {
        return ExecutionResult::failure(format!("run_bounty: bounty `{bounty_id}` not found"));
    };
    if !matches!(state, jfc_economy::types::MarketState::Open) {
        return ExecutionResult::failure(format!(
            "run_bounty: bounty `{bounty_id}` is in state {state:?}, \
             not Open — only Open bounties can be dispatched"
        ));
    }
    let invoker = EconomyAgentInvoker::new(provider, model);
    let swarm = EconomySwarmProvider::new(cwd.to_path_buf());
    let n_solvers = max_solvers.unwrap_or(2).clamp(1, 5);
    tracing::info!(
        target: "jfc::ui::bounty",
        bounty_id = %bounty_id,
        n_solvers = n_solvers,
        cwd = %cwd.display(),
        "run_bounty: kicking off cycle"
    );
    let cycle_result = {
        let mut orch = market_orchestrator().lock().await;
        orch.run_bounty_cycle(&bounty_id, &invoker, &swarm, n_solvers, 1)
            .await
    };
    match cycle_result {
        Ok(outcome) => {
            let written =
                apply_winning_solution(cwd, &bounty_id, outcome.winning_solution.as_ref());
            tracing::info!(
                target: "jfc::ui::bounty",
                bounty_id = %bounty_id,
                winner = outcome.settlement.winner.as_ref().map(|a| a.0.as_str()).unwrap_or("(none)"),
                files_written = written.files.len(),
                "run_bounty settled"
            );
            ExecutionResult::success(format!(
                "Bounty `{bounty_id}` settled.\n\
                 Winner: {}\n\
                 Total cost: {} tok\n\
                 Payouts: {}\n\
                 Trust updates: {}\n\
                 {}\n\
                 Run /market or market_status to see updated trust + budget.",
                outcome
                    .settlement
                    .winner
                    .as_ref()
                    .map(|a| a.0.as_str())
                    .unwrap_or("(no winning solution)"),
                outcome.settlement.total_cost,
                outcome.settlement.payouts.len(),
                outcome.settlement.trust_updates.len(),
                written.summary,
            ))
        }
        Err(e) => {
            ExecutionResult::failure(format!("run_bounty cycle for `{bounty_id}` failed: {e}"))
        }
    }
}

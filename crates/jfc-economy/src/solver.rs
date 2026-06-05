//! Solver agent spawning, lifecycle management, and solution ranking.

use std::path::PathBuf;

use crate::types::{AgentId, Solution};

/// Solver lifecycle state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SolverStatus {
    Pending,
    Executing,
    Completed,
    TimedOut,
    Abandoned,
}

/// A solver agent instance bound to a specific bounty.
#[derive(Debug, Clone)]
pub struct SolverAgent {
    pub id: AgentId,
    pub bounty_id: String,
    pub worktree_path: Option<PathBuf>,
    pub status: SolverStatus,
    pub tokens_consumed: u64,
    pub solution: Option<Solution>,
}

impl SolverAgent {
    pub fn new(bounty_id: &str) -> Self {
        Self {
            id: AgentId::new("solver"),
            bounty_id: bounty_id.to_string(),
            worktree_path: None,
            status: SolverStatus::Pending,
            tokens_consumed: 0,
            solution: None,
        }
    }

    /// Create a solver with a pre-determined stable identity (for trust persistence).
    pub fn with_id(agent_id: AgentId, bounty_id: &str) -> Self {
        Self {
            id: agent_id,
            bounty_id: bounty_id.to_string(),
            worktree_path: None,
            status: SolverStatus::Pending,
            tokens_consumed: 0,
            solution: None,
        }
    }

    pub fn start(&mut self, worktree: Option<PathBuf>) {
        self.worktree_path = worktree;
        self.status = SolverStatus::Executing;
    }

    pub fn submit(&mut self, solution: Solution) {
        self.solution = Some(solution);
        self.status = SolverStatus::Completed;
    }

    pub fn abandon(&mut self) {
        self.status = SolverStatus::Abandoned;
    }

    pub fn record_tokens(&mut self, tokens: u64) {
        self.tokens_consumed += tokens;
    }
}

/// Pool of solver agents for a bounty.
pub struct SolverPool {
    solvers: Vec<SolverAgent>,
}

impl SolverPool {
    pub fn new() -> Self {
        Self {
            solvers: Vec::new(),
        }
    }

    pub fn spawn(&mut self, bounty_id: &str) -> &SolverAgent {
        let solver = SolverAgent::new(bounty_id);
        self.solvers.push(solver);
        self.solvers.last().unwrap()
    }

    /// Spawn a solver with a pre-determined stable identity.
    pub fn spawn_with_id(&mut self, agent_id: AgentId, bounty_id: &str) -> &SolverAgent {
        let solver = SolverAgent::with_id(agent_id, bounty_id);
        self.solvers.push(solver);
        self.solvers.last().unwrap()
    }

    pub fn get(&self, agent_id: &AgentId) -> Option<&SolverAgent> {
        self.solvers.iter().find(|s| s.id == *agent_id)
    }

    pub fn get_mut(&mut self, agent_id: &AgentId) -> Option<&mut SolverAgent> {
        self.solvers.iter_mut().find(|s| s.id == *agent_id)
    }

    pub fn completed_solutions(&self) -> Vec<&Solution> {
        self.solvers
            .iter()
            .filter(|s| s.status == SolverStatus::Completed)
            .filter_map(|s| s.solution.as_ref())
            .collect()
    }

    /// Rank solutions by mechanistic signals only (compiles, tests, suspicious, cost).
    pub fn rank_solutions(&self) -> Vec<&Solution> {
        let mut solutions: Vec<&Solution> = self
            .completed_solutions()
            .into_iter()
            .filter(|solution| solution_is_mechanically_accepted(solution))
            .collect();
        solutions.sort_by(|a, b| {
            let a_score = solution_score(a);
            let b_score = solution_score(b);
            b_score
                .partial_cmp(&a_score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        solutions
    }

    pub fn active_count(&self) -> usize {
        self.solvers
            .iter()
            .filter(|s| s.status == SolverStatus::Executing)
            .count()
    }

    pub fn all(&self) -> &[SolverAgent] {
        &self.solvers
    }
}

impl Default for SolverPool {
    fn default() -> Self {
        Self::new()
    }
}

fn solution_is_mechanically_accepted(s: &Solution) -> bool {
    if s.suspicious || s.compiles == Some(false) || s.tests_pass == Some(false) {
        return false;
    }

    s.compiles == Some(true) || s.tests_pass == Some(true)
}

/// Score a solution by mechanistic signals (higher = better).
///
/// Priority: compiles (+100) > tests_pass (+50) > not_suspicious (+20) > lower token cost.
fn solution_score(s: &Solution) -> f32 {
    let mut score = 0.0;
    if s.compiles == Some(true) {
        score += 100.0;
    }
    if s.tests_pass == Some(true) {
        score += 50.0;
    }
    if !s.suspicious {
        score += 20.0;
    }
    // Lower token cost is better (logarithmic normalization)
    score += 10.0 / (s.tokens_consumed as f32 + 1.0).ln_1p();
    score
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_solution(
        agent_name: &str,
        bounty_id: &str,
        compiles: Option<bool>,
        tests_pass: Option<bool>,
        suspicious: bool,
        tokens: u64,
    ) -> Solution {
        Solution {
            agent_id: AgentId(format!("agent_{agent_name}")),
            bounty_id: bounty_id.to_string(),
            patch: "diff --git a/foo.rs".to_string(),
            explanation: "fixed the bug".to_string(),
            self_assessment: 0.8,
            tokens_consumed: tokens,
            compiles,
            tests_pass,
            suspicious,
        }
    }

    #[test]
    fn test_spawn_solver() {
        let mut pool = SolverPool::new();
        pool.spawn("bounty-1");
        pool.spawn("bounty-1");
        assert_eq!(pool.all().len(), 2);
        assert_ne!(pool.all()[0].id, pool.all()[1].id);
    }

    #[test]
    fn test_solver_lifecycle() {
        let mut pool = SolverPool::new();
        let id = pool.spawn("bounty-1").id.clone();

        assert_eq!(pool.get(&id).unwrap().status, SolverStatus::Pending);

        pool.get_mut(&id)
            .unwrap()
            .start(Some(PathBuf::from("/tmp/wt")));
        assert_eq!(pool.get(&id).unwrap().status, SolverStatus::Executing);
        assert_eq!(pool.active_count(), 1);

        let solution = make_solution("solver", "bounty-1", Some(true), Some(true), false, 1000);
        pool.get_mut(&id).unwrap().submit(solution);
        assert_eq!(pool.get(&id).unwrap().status, SolverStatus::Completed);
        assert_eq!(pool.active_count(), 0);
    }

    #[test]
    fn test_solver_abandon() {
        let mut pool = SolverPool::new();
        let id = pool.spawn("bounty-1").id.clone();

        pool.get_mut(&id).unwrap().start(None);
        pool.get_mut(&id).unwrap().abandon();
        assert_eq!(pool.get(&id).unwrap().status, SolverStatus::Abandoned);
    }

    #[test]
    fn test_rank_solutions() {
        let mut pool = SolverPool::new();

        // Solver A: compiles, tests pass
        let id_a = pool.spawn("b1").id.clone();
        pool.get_mut(&id_a).unwrap().start(None);
        pool.get_mut(&id_a).unwrap().submit(make_solution(
            "a",
            "b1",
            Some(true),
            Some(true),
            false,
            500,
        ));

        // Solver B: compiles, tests fail
        let id_b = pool.spawn("b1").id.clone();
        pool.get_mut(&id_b).unwrap().start(None);
        pool.get_mut(&id_b).unwrap().submit(make_solution(
            "b",
            "b1",
            Some(true),
            Some(false),
            false,
            500,
        ));

        // Solver C: doesn't compile
        let id_c = pool.spawn("b1").id.clone();
        pool.get_mut(&id_c).unwrap().start(None);
        pool.get_mut(&id_c).unwrap().submit(make_solution(
            "c",
            "b1",
            Some(false),
            Some(false),
            false,
            500,
        ));

        let ranked = pool.rank_solutions();
        assert_eq!(ranked.len(), 1);
        assert_eq!(ranked[0].agent_id.0, "agent_a");
    }

    #[test]
    fn test_suspicious_ranked_lower() {
        let mut pool = SolverPool::new();

        // Clean solution
        let id_clean = pool.spawn("b1").id.clone();
        pool.get_mut(&id_clean).unwrap().start(None);
        pool.get_mut(&id_clean).unwrap().submit(make_solution(
            "clean",
            "b1",
            Some(true),
            Some(true),
            false,
            500,
        ));

        // Suspicious solution (same signals otherwise)
        let id_sus = pool.spawn("b1").id.clone();
        pool.get_mut(&id_sus).unwrap().start(None);
        pool.get_mut(&id_sus).unwrap().submit(make_solution(
            "sus",
            "b1",
            Some(true),
            Some(true),
            true,
            500,
        ));

        let ranked = pool.rank_solutions();
        assert_eq!(ranked.len(), 1);
        assert_eq!(ranked[0].agent_id.0, "agent_clean");
    }

    #[test]
    fn test_unverified_solution_is_not_ranked() {
        let mut pool = SolverPool::new();

        let id = pool.spawn("b1").id.clone();
        pool.get_mut(&id).unwrap().start(None);
        pool.get_mut(&id).unwrap().submit(make_solution(
            "unverified",
            "b1",
            None,
            None,
            false,
            500,
        ));

        assert!(pool.rank_solutions().is_empty());
    }
}

//! Core types for the agent economy system.

use std::time::Duration;

use serde::{Deserialize, Serialize};

/// Unique agent identifier.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AgentId(pub String);

impl AgentId {
    /// Create a unique (ephemeral) agent identity. Trust does NOT persist across bounties.
    pub fn new(name: &str) -> Self {
        Self(format!("{}_{}", name, uuid::Uuid::new_v4().as_simple()))
    }

    /// Create a stable identity that persists across bounties so trust/collusion
    /// metrics accumulate. Format: `<role>-<index>` (e.g., `solver-0`, `validator-1`).
    /// The same role+index always maps to the same AgentId.
    pub fn new_stable(role: &str, index: usize) -> Self {
        Self(format!("{role}-{index}"))
    }
}

/// Role an agent plays in the market.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AgentRole {
    Solver,
    Validator,
    Auditor,
}

/// Market cycle state machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MarketState {
    Posting,
    Open,
    Bidding,
    Executing,
    Validating,
    Settling,
    Complete,
    Failed,
}

/// A bounty posted to the market.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Bounty {
    pub id: String,
    pub description: String,
    pub reward: u64,
    pub acceptance_criteria: String,
    pub deadline: Duration,
    pub max_solvers: u8,
    pub state: MarketState,
}

/// A sealed bid from a solver.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Bid {
    pub agent_id: AgentId,
    pub bounty_id: String,
    pub price: u64,
    pub approach: String,
    pub estimated_time: Duration,
}

/// A solution produced by a solver.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Solution {
    pub agent_id: AgentId,
    pub bounty_id: String,
    pub patch: String,
    pub explanation: String,
    pub self_assessment: f32,
    pub tokens_consumed: u64,
    pub compiles: Option<bool>,
    pub tests_pass: Option<bool>,
    pub suspicious: bool,
}

/// A validation challenge from a validator.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationChallenge {
    pub validator_id: AgentId,
    pub solution_agent_id: AgentId,
    pub bounty_id: String,
    pub proposed_flaw: String,
    pub test_code: Option<String>,
    pub confidence: f32,
}

/// Verdict from validation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ValidationVerdict {
    FlawUpheld,
    FlawDismissed,
    NoFlawFound,
    EarlyTermination,
}

/// Settlement record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Settlement {
    pub bounty_id: String,
    pub winner: Option<AgentId>,
    /// Positive = earned, negative = penalty.
    pub payouts: Vec<(AgentId, i64)>,
    pub trust_updates: Vec<(AgentId, i8)>,
    pub total_cost: u64,
}

/// Audit log entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEntry {
    pub timestamp_ms: u64,
    pub bounty_id: String,
    pub event: AuditEvent,
}

/// Events recorded in the audit log.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AuditEvent {
    BountyPosted {
        reward: u64,
    },
    BidReceived {
        agent_id: AgentId,
        price: u64,
    },
    SolverSelected {
        agent_id: AgentId,
    },
    SolutionSubmitted {
        agent_id: AgentId,
        compiles: bool,
    },
    ValidationStarted {
        validator_id: AgentId,
        solution_agent_id: AgentId,
    },
    ValidationVerdict {
        verdict: ValidationVerdict,
    },
    SettlementComplete {
        winner: Option<AgentId>,
        total_cost: u64,
    },
    StateTransition {
        from: MarketState,
        to: MarketState,
    },
    CharterViolation {
        agent_id: AgentId,
        violation: String,
        penalty: i8,
    },
    SurgePricing {
        old_reward: u64,
        new_reward: u64,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_agent_id_unique() {
        let a = AgentId::new("solver");
        let b = AgentId::new("solver");
        assert_ne!(a, b);
    }

    #[test]
    fn test_market_state_serialization() {
        let state = MarketState::Bidding;
        let json = serde_json::to_string(&state).unwrap();
        let back: MarketState = serde_json::from_str(&json).unwrap();
        assert_eq!(state, back);
    }

    #[test]
    fn test_bounty_creation() {
        let bounty = Bounty {
            id: "test-1".into(),
            description: "Add fibonacci function".into(),
            reward: 1000,
            acceptance_criteria: "fn fibonacci(n: u64) -> u64 works correctly".into(),
            deadline: Duration::from_secs(300),
            max_solvers: 3,
            state: MarketState::Posting,
        };
        assert_eq!(bounty.state, MarketState::Posting);
        assert_eq!(bounty.reward, 1000);
    }
}

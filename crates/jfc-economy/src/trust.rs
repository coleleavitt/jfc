//! Trust scoring system (asymmetric +5/-15).
//!
//! Agents start at 50. Successes add +5, failures subtract -15.
//! Tiers gate capabilities: Restricted (<30), Standard (30-70), Trusted (>70).
//! History is append-only for audit purposes.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::types::AgentId;

/// Capability tier based on trust score.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TrustTier {
    /// Score < 30: cannot bid on high-trust bounties.
    Restricted,
    /// 30 <= score <= 70: standard market access.
    Standard,
    /// Score > 70: eligible for auditor roles and priority bidding.
    Trusted,
}

/// A single trust score change record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrustChange {
    pub timestamp_ms: u64,
    pub delta: i8,
    pub reason: String,
    pub score_after: u8,
}

/// Trust score for a single agent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrustScore {
    score: u8,
    history: Vec<TrustChange>,
}

impl TrustScore {
    pub fn new() -> Self {
        Self {
            score: 50,
            history: Vec::new(),
        }
    }

    pub fn score(&self) -> u8 {
        self.score
    }

    pub fn tier(&self) -> TrustTier {
        match self.score {
            0..=29 => TrustTier::Restricted,
            30..=70 => TrustTier::Standard,
            71..=100 => TrustTier::Trusted,
            _ => TrustTier::Trusted,
        }
    }

    /// Record a success (+5, clamped to 100).
    pub fn record_success(&mut self, reason: &str) {
        let delta = 5i8;
        self.score = self.score.saturating_add(5).min(100);
        self.history.push(TrustChange {
            timestamp_ms: now_ms(),
            delta,
            reason: reason.to_string(),
            score_after: self.score,
        });
    }

    /// Record a failure (-15, clamped to 0).
    pub fn record_failure(&mut self, reason: &str) {
        let delta = -15i8;
        self.score = self.score.saturating_sub(15);
        self.history.push(TrustChange {
            timestamp_ms: now_ms(),
            delta,
            reason: reason.to_string(),
            score_after: self.score,
        });
    }

    /// Record a custom penalty (for charter violations).
    pub fn record_penalty(&mut self, amount: u8, reason: &str) {
        let delta = -(amount as i8);
        self.score = self.score.saturating_sub(amount);
        self.history.push(TrustChange {
            timestamp_ms: now_ms(),
            delta,
            reason: reason.to_string(),
            score_after: self.score,
        });
    }

    pub fn history(&self) -> &[TrustChange] {
        &self.history
    }
}

impl Default for TrustScore {
    fn default() -> Self {
        Self::new()
    }
}

/// Registry of all agent trust scores.
pub struct TrustRegistry {
    scores: HashMap<AgentId, TrustScore>,
}

impl TrustRegistry {
    pub fn new() -> Self {
        Self {
            scores: HashMap::new(),
        }
    }

    /// Register a new agent (starts at 50).
    pub fn register(&mut self, agent_id: AgentId) {
        self.scores.entry(agent_id).or_insert_with(TrustScore::new);
    }

    /// Get trust score for agent.
    pub fn get(&self, agent_id: &AgentId) -> Option<&TrustScore> {
        self.scores.get(agent_id)
    }

    /// Get mutable trust score.
    pub fn get_mut(&mut self, agent_id: &AgentId) -> Option<&mut TrustScore> {
        self.scores.get_mut(agent_id)
    }

    /// Check if agent meets minimum trust for a role.
    pub fn meets_minimum(&self, agent_id: &AgentId, min_trust: u8) -> bool {
        self.scores
            .get(agent_id)
            .map(|s| s.score() >= min_trust)
            .unwrap_or(false)
    }

    /// Get all agents sorted by trust (highest first).
    pub fn leaderboard(&self) -> Vec<(&AgentId, &TrustScore)> {
        let mut entries: Vec<_> = self.scores.iter().collect();
        entries.sort_by_key(|entry| std::cmp::Reverse(entry.1.score()));
        entries
    }

    /// Mean trust score across all agents.
    pub fn mean_trust(&self) -> f32 {
        if self.scores.is_empty() {
            return 50.0;
        }
        let sum: u32 = self.scores.values().map(|s| s.score() as u32).sum();
        sum as f32 / self.scores.len() as f32
    }
}

impl Default for TrustRegistry {
    fn default() -> Self {
        Self::new()
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_trust_starts_at_50() {
        let ts = TrustScore::new();
        assert_eq!(ts.score(), 50);
        assert_eq!(ts.tier(), TrustTier::Standard);
    }

    #[test]
    fn test_trust_asymmetric() {
        let mut ts = TrustScore::new();
        ts.record_success("good work");
        assert_eq!(ts.score(), 55);
        ts.record_failure("bad work");
        assert_eq!(ts.score(), 40);
    }

    #[test]
    fn test_trust_clamp_high() {
        let mut ts = TrustScore::new();
        for _ in 0..11 {
            ts.record_success("success");
        }
        assert_eq!(ts.score(), 100);
    }

    #[test]
    fn test_trust_clamp_low() {
        let mut ts = TrustScore::new();
        for _ in 0..4 {
            ts.record_failure("failure");
        }
        assert_eq!(ts.score(), 0);
    }

    #[test]
    fn test_trust_tier_restricted() {
        let mut ts = TrustScore::new();
        // 50 - 15 - 15 = 20
        ts.record_failure("f1");
        ts.record_failure("f2");
        assert_eq!(ts.score(), 20);
        assert_eq!(ts.tier(), TrustTier::Restricted);
    }

    #[test]
    fn test_trust_tier_trusted() {
        let mut ts = TrustScore::new();
        // 50 + 5*5 = 75
        for _ in 0..5 {
            ts.record_success("s");
        }
        assert_eq!(ts.score(), 75);
        assert_eq!(ts.tier(), TrustTier::Trusted);
    }

    #[test]
    fn test_trust_meets_minimum() {
        let mut registry = TrustRegistry::new();
        let agent = AgentId("agent_001".into());
        registry.register(agent.clone());
        assert!(registry.meets_minimum(&agent, 30));
        assert!(registry.meets_minimum(&agent, 50));
        assert!(!registry.meets_minimum(&agent, 51));
    }

    #[test]
    fn test_trust_penalty() {
        let mut ts = TrustScore::new();
        ts.record_penalty(20, "charter violation");
        assert_eq!(ts.score(), 30);
    }

    #[test]
    fn test_trust_history_append_only() {
        let mut ts = TrustScore::new();
        ts.record_success("a");
        ts.record_failure("b");
        ts.record_penalty(5, "c");
        assert_eq!(ts.history().len(), 3);
        assert_eq!(ts.history()[0].delta, 5);
        assert_eq!(ts.history()[1].delta, -15);
        assert_eq!(ts.history()[2].delta, -5);
    }
}

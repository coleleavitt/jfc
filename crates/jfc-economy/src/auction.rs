//! Sealed-bid auction engine.
//!
//! First-price sealed-bid auction where the poster screens on price, reputation,
//! and stated approach. Bids are sealed — no information leakage between solvers.

use crate::trust::TrustRegistry;
use crate::types::Bid;

/// Bid scoring weights for the auction ranking function.
#[derive(Debug, Clone)]
pub struct ScoringWeights {
    /// Weight for trust score component.
    pub trust_weight: f32,
    /// Weight for price component (inverse — lower price = higher score).
    pub price_weight: f32,
    /// Weight for approach quality (placeholder, always 1.0 for v1).
    pub approach_weight: f32,
}

impl Default for ScoringWeights {
    fn default() -> Self {
        Self {
            trust_weight: 0.4,
            price_weight: 0.4,
            approach_weight: 0.2,
        }
    }
}

/// Sealed-bid auction engine.
///
/// Collects bids independently (no leakage), then scores and ranks them
/// using a weighted combination of trust, price, and approach signals.
pub struct AuctionEngine {
    bids: Vec<Bid>,
    weights: ScoringWeights,
}

impl AuctionEngine {
    pub fn new() -> Self {
        Self {
            bids: Vec::new(),
            weights: ScoringWeights::default(),
        }
    }

    pub fn with_weights(weights: ScoringWeights) -> Self {
        Self {
            bids: Vec::new(),
            weights,
        }
    }

    /// Submit a sealed bid (bids are independent, no leakage).
    pub fn submit_bid(&mut self, bid: Bid) {
        self.bids.push(bid);
    }

    /// Score and rank all bids, return top N winners.
    ///
    /// Scoring formula:
    ///   score = w1 * trust_normalized + w2 * price_score + w3 * approach_score
    ///
    /// Where:
    /// - trust_normalized = agent trust score / 100
    /// - price_score = min(1.0, 1000 / price) — lower price is better
    /// - approach_score = 1.0 (v1: all approaches equal)
    pub fn select_winners(&self, max_solvers: u8, trust_registry: &TrustRegistry) -> Vec<&Bid> {
        let mut scored: Vec<(&Bid, f32)> = self
            .bids
            .iter()
            .map(|bid| {
                let trust = trust_registry
                    .get(&bid.agent_id)
                    .map(|t| t.score() as f32 / 100.0)
                    .unwrap_or(0.5);

                let price_score = if bid.price > 0 {
                    (1000.0 / bid.price as f32).min(1.0)
                } else {
                    1.0
                };

                let approach_score = 1.0; // v1: all approaches equal

                let score = self.weights.trust_weight * trust
                    + self.weights.price_weight * price_score
                    + self.weights.approach_weight * approach_score;

                (bid, score)
            })
            .collect();

        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        scored
            .into_iter()
            .take(max_solvers as usize)
            .map(|(bid, _)| bid)
            .collect()
    }

    /// Get all bids (for audit).
    pub fn bids(&self) -> &[Bid] {
        &self.bids
    }

    /// Clear bids for next round.
    pub fn clear(&mut self) {
        self.bids.clear();
    }

    /// Number of bids received.
    pub fn bid_count(&self) -> usize {
        self.bids.len()
    }
}

impl Default for AuctionEngine {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::types::AgentId;

    fn make_bid(agent_name: &str, bounty_id: &str, price: u64) -> Bid {
        Bid {
            agent_id: AgentId(format!("agent_{agent_name}")),
            bounty_id: bounty_id.to_string(),
            price,
            approach: "standard approach".to_string(),
            estimated_time: Duration::from_secs(60),
        }
    }

    #[test]
    fn test_submit_bid() {
        let mut engine = AuctionEngine::new();
        engine.submit_bid(make_bid("alice", "b1", 500));
        engine.submit_bid(make_bid("bob", "b1", 600));
        engine.submit_bid(make_bid("carol", "b1", 700));
        assert_eq!(engine.bid_count(), 3);
    }

    #[test]
    fn test_select_winners_top_n() {
        let mut engine = AuctionEngine::new();
        let registry = TrustRegistry::new();

        for i in 0..5 {
            engine.submit_bid(make_bid(&format!("agent{i}"), "b1", 500 + i * 100));
        }

        let winners = engine.select_winners(3, &registry);
        assert_eq!(winners.len(), 3);
    }

    #[test]
    fn test_ranking_by_trust() {
        let mut engine = AuctionEngine::new();
        let mut registry = TrustRegistry::new();

        let high_trust = AgentId("agent_high".into());
        let low_trust = AgentId("agent_low".into());

        registry.register(high_trust.clone());
        registry.register(low_trust.clone());

        // Boost high_trust agent to 75
        for _ in 0..5 {
            registry
                .get_mut(&high_trust)
                .unwrap()
                .record_success("good");
        }
        // Drop low_trust agent to 20
        registry.get_mut(&low_trust).unwrap().record_failure("bad");
        registry.get_mut(&low_trust).unwrap().record_failure("bad");

        // Same price for both
        engine.submit_bid(Bid {
            agent_id: low_trust,
            bounty_id: "b1".into(),
            price: 500,
            approach: "approach".into(),
            estimated_time: Duration::from_secs(60),
        });
        engine.submit_bid(Bid {
            agent_id: high_trust.clone(),
            bounty_id: "b1".into(),
            price: 500,
            approach: "approach".into(),
            estimated_time: Duration::from_secs(60),
        });

        let winners = engine.select_winners(2, &registry);
        assert_eq!(winners[0].agent_id, high_trust);
    }

    #[test]
    fn test_ranking_by_price() {
        let mut engine = AuctionEngine::new();
        let mut registry = TrustRegistry::new();

        let cheap = AgentId("agent_cheap".into());
        let expensive = AgentId("agent_expensive".into());

        // Same trust for both (register both at default 50)
        registry.register(cheap.clone());
        registry.register(expensive.clone());

        engine.submit_bid(Bid {
            agent_id: expensive,
            bounty_id: "b1".into(),
            price: 5000,
            approach: "approach".into(),
            estimated_time: Duration::from_secs(60),
        });
        engine.submit_bid(Bid {
            agent_id: cheap.clone(),
            bounty_id: "b1".into(),
            price: 200,
            approach: "approach".into(),
            estimated_time: Duration::from_secs(60),
        });

        let winners = engine.select_winners(2, &registry);
        assert_eq!(winners[0].agent_id, cheap);
    }

    #[test]
    fn test_bid_isolation() {
        // Bids don't contain references to each other — they are independent values.
        let mut engine = AuctionEngine::new();
        let bid1 = make_bid("alice", "b1", 500);
        let bid2 = make_bid("bob", "b1", 600);

        engine.submit_bid(bid1);
        engine.submit_bid(bid2);

        // Each bid is independently stored
        let bids = engine.bids();
        assert_eq!(bids[0].agent_id.0, "agent_alice");
        assert_eq!(bids[1].agent_id.0, "agent_bob");
        assert_ne!(bids[0].agent_id, bids[1].agent_id);

        // Clearing doesn't affect previously extracted references
        let count_before = engine.bid_count();
        engine.clear();
        assert_eq!(count_before, 2);
        assert_eq!(engine.bid_count(), 0);
    }
}

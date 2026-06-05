//! Bounty lifecycle and typed state machine.
//!
//! Enforces valid state transitions at runtime with clear errors.
//! Supports surge pricing (+15% reward) on timeout with no bids.

use std::time::Duration;

use crate::types::{AuditEntry, AuditEvent, Bounty, MarketState};

#[derive(Debug, thiserror::Error)]
pub enum BountyError {
    #[error("invalid state transition: {from:?} -> {to:?}")]
    InvalidTransition { from: MarketState, to: MarketState },
    #[error("bounty not found: {0}")]
    NotFound(String),
    #[error("bounty already complete")]
    AlreadyComplete,
}

/// Valid transitions for the market state machine.
///
/// Forward-only except for the surge re-open path (Bidding → Open).
fn is_valid_transition(from: MarketState, to: MarketState) -> bool {
    matches!(
        (from, to),
        (MarketState::Posting, MarketState::Open)
            | (MarketState::Open, MarketState::Bidding)
            | (MarketState::Bidding, MarketState::Executing)
            | (MarketState::Bidding, MarketState::Open) // surge: re-open after timeout
            | (MarketState::Executing, MarketState::Validating)
            | (MarketState::Validating, MarketState::Settling)
            | (MarketState::Settling, MarketState::Complete)
            // Failure paths
            | (MarketState::Bidding, MarketState::Failed)
            | (MarketState::Executing, MarketState::Failed)
            | (MarketState::Validating, MarketState::Failed)
    )
}

pub struct BountyManager {
    bounties: Vec<Bounty>,
    audit_log: Vec<AuditEntry>,
    surge_multiplier: f32,
}

impl BountyManager {
    pub fn new() -> Self {
        Self {
            bounties: Vec::new(),
            audit_log: Vec::new(),
            surge_multiplier: 1.15,
        }
    }

    pub fn post(
        &mut self,
        description: String,
        reward: u64,
        acceptance_criteria: String,
        deadline: Duration,
        max_solvers: u8,
    ) -> String {
        let id = format!("bounty_{}", uuid::Uuid::new_v4().as_simple());
        let bounty = Bounty {
            id: id.clone(),
            description,
            reward,
            acceptance_criteria,
            deadline,
            max_solvers,
            state: MarketState::Posting,
        };
        self.audit_log.push(AuditEntry {
            timestamp_ms: now_ms(),
            bounty_id: id.clone(),
            event: AuditEvent::BountyPosted { reward },
        });
        self.bounties.push(bounty);
        id
    }

    pub fn transition(&mut self, bounty_id: &str, to: MarketState) -> Result<(), BountyError> {
        let bounty = self
            .bounties
            .iter_mut()
            .find(|b| b.id == bounty_id)
            .ok_or_else(|| BountyError::NotFound(bounty_id.to_string()))?;

        if bounty.state == MarketState::Complete {
            return Err(BountyError::AlreadyComplete);
        }

        if !is_valid_transition(bounty.state, to) {
            return Err(BountyError::InvalidTransition {
                from: bounty.state,
                to,
            });
        }

        let from = bounty.state;
        bounty.state = to;
        self.audit_log.push(AuditEntry {
            timestamp_ms: now_ms(),
            bounty_id: bounty_id.to_string(),
            event: AuditEvent::StateTransition { from, to },
        });
        Ok(())
    }

    pub fn surge_price(&mut self, bounty_id: &str) -> Result<u64, BountyError> {
        let bounty = self
            .bounties
            .iter_mut()
            .find(|b| b.id == bounty_id)
            .ok_or_else(|| BountyError::NotFound(bounty_id.to_string()))?;

        let old_reward = bounty.reward;
        bounty.reward = (bounty.reward as f32 * self.surge_multiplier) as u64;
        self.audit_log.push(AuditEntry {
            timestamp_ms: now_ms(),
            bounty_id: bounty_id.to_string(),
            event: AuditEvent::SurgePricing {
                old_reward,
                new_reward: bounty.reward,
            },
        });
        Ok(bounty.reward)
    }

    pub fn get(&self, bounty_id: &str) -> Option<&Bounty> {
        self.bounties.iter().find(|b| b.id == bounty_id)
    }

    pub fn get_mut(&mut self, bounty_id: &str) -> Option<&mut Bounty> {
        self.bounties.iter_mut().find(|b| b.id == bounty_id)
    }

    pub fn audit_log(&self) -> &[AuditEntry] {
        &self.audit_log
    }

    pub fn open_bounties(&self) -> Vec<&Bounty> {
        self.bounties
            .iter()
            .filter(|b| matches!(b.state, MarketState::Open | MarketState::Bidding))
            .collect()
    }
}

impl Default for BountyManager {
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

    fn make_manager() -> BountyManager {
        BountyManager::new()
    }

    fn post_test_bounty(mgr: &mut BountyManager, reward: u64) -> String {
        mgr.post(
            "Implement fibonacci".into(),
            reward,
            "fn fib(n: u64) -> u64 works".into(),
            Duration::from_secs(300),
            3,
        )
    }

    #[test]
    fn test_post_bounty() {
        let mut mgr = make_manager();
        let id = post_test_bounty(&mut mgr, 1000);

        let bounty = mgr.get(&id).unwrap();
        assert_eq!(bounty.state, MarketState::Posting);
        assert_eq!(bounty.reward, 1000);
        assert!(bounty.id.starts_with("bounty_"));
    }

    #[test]
    fn test_valid_transition() {
        let mut mgr = make_manager();
        let id = post_test_bounty(&mut mgr, 1000);

        // Posting → Open → Bidding → Executing → Validating → Settling → Complete
        mgr.transition(&id, MarketState::Open).unwrap();
        mgr.transition(&id, MarketState::Bidding).unwrap();
        mgr.transition(&id, MarketState::Executing).unwrap();
        mgr.transition(&id, MarketState::Validating).unwrap();
        mgr.transition(&id, MarketState::Settling).unwrap();
        mgr.transition(&id, MarketState::Complete).unwrap();

        let bounty = mgr.get(&id).unwrap();
        assert_eq!(bounty.state, MarketState::Complete);
    }

    #[test]
    fn test_invalid_transition() {
        let mut mgr = make_manager();
        let id = post_test_bounty(&mut mgr, 1000);

        let err = mgr.transition(&id, MarketState::Complete).unwrap_err();
        assert!(matches!(err, BountyError::InvalidTransition { .. }));
    }

    #[test]
    fn test_no_backwards() {
        let mut mgr = make_manager();
        let id = post_test_bounty(&mut mgr, 1000);

        mgr.transition(&id, MarketState::Open).unwrap();
        mgr.transition(&id, MarketState::Bidding).unwrap();
        mgr.transition(&id, MarketState::Executing).unwrap();
        mgr.transition(&id, MarketState::Validating).unwrap();
        mgr.transition(&id, MarketState::Settling).unwrap();
        mgr.transition(&id, MarketState::Complete).unwrap();

        let err = mgr.transition(&id, MarketState::Bidding).unwrap_err();
        assert!(matches!(err, BountyError::AlreadyComplete));
    }

    #[test]
    fn test_surge_pricing() {
        let mut mgr = make_manager();
        let id = post_test_bounty(&mut mgr, 100);

        let new_reward = mgr.surge_price(&id).unwrap();
        assert_eq!(new_reward, 115);

        let bounty = mgr.get(&id).unwrap();
        assert_eq!(bounty.reward, 115);
    }

    #[test]
    fn test_surge_audit() {
        let mut mgr = make_manager();
        let id = post_test_bounty(&mut mgr, 100);

        mgr.surge_price(&id).unwrap();

        let surge_entry = mgr
            .audit_log()
            .iter()
            .find(|e| matches!(e.event, AuditEvent::SurgePricing { .. }))
            .expect("should have SurgePricing audit entry");

        assert_eq!(surge_entry.bounty_id, id);
        match &surge_entry.event {
            AuditEvent::SurgePricing {
                old_reward,
                new_reward,
            } => {
                assert_eq!(*old_reward, 100);
                assert_eq!(*new_reward, 115);
            }
            _ => panic!("expected SurgePricing event"),
        }
    }
}

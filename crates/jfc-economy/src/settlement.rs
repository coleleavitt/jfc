//! Settlement engine — distributes rewards based on validation outcomes.

use crate::charter::Charter;
use crate::ledger::{TokenLedger, TransactionPurpose};
use crate::trust::TrustRegistry;
use crate::types::{AgentId, Settlement, ValidationVerdict};

/// Stateless settlement engine.
pub struct SettlementEngine;

impl SettlementEngine {
    /// Settle a bounty: distribute rewards, update trust.
    ///
    /// Payment floor: winner always receives at least `charter.payment_floor * reward`.
    /// Validators earn 10% of reward for finding real flaws, lose trust for invalid challenges.
    pub fn settle(
        bounty_id: &str,
        reward: u64,
        winner: Option<&AgentId>,
        _losers: &[AgentId],
        validators: &[(AgentId, ValidationVerdict)],
        charter: &Charter,
        ledger: &mut TokenLedger,
        trust: &mut TrustRegistry,
    ) -> Settlement {
        let mut payouts: Vec<(AgentId, i64)> = Vec::new();
        let mut trust_updates: Vec<(AgentId, i8)> = Vec::new();
        let mut total_cost = 0u64;

        if let Some(winner_id) = winner {
            let floor = charter.payment_floor.max(0.5);
            let payment = (reward as f64 * f64::from(floor)).round() as u64;
            ledger.credit(winner_id, payment, TransactionPurpose::BountyReward);
            payouts.push((winner_id.clone(), payment as i64));
            total_cost += payment;

            if let Some(ts) = trust.get_mut(winner_id) {
                ts.record_success("Won bounty");
            }
            trust_updates.push((winner_id.clone(), 5));
        }

        for (validator_id, verdict) in validators {
            match verdict {
                ValidationVerdict::FlawUpheld => {
                    let validation_reward = reward / 10;
                    ledger.credit(
                        validator_id,
                        validation_reward,
                        TransactionPurpose::ValidationReward,
                    );
                    payouts.push((validator_id.clone(), validation_reward as i64));
                    total_cost += validation_reward;
                    if let Some(ts) = trust.get_mut(validator_id) {
                        ts.record_success("Found valid flaw");
                    }
                    trust_updates.push((validator_id.clone(), 5));
                }
                ValidationVerdict::FlawDismissed => {
                    if let Some(ts) = trust.get_mut(validator_id) {
                        ts.record_failure("Invalid challenge dismissed");
                    }
                    trust_updates.push((validator_id.clone(), -15));
                }
                ValidationVerdict::NoFlawFound | ValidationVerdict::EarlyTermination => {}
            }
        }

        Settlement {
            bounty_id: bounty_id.to_string(),
            winner: winner.cloned(),
            payouts,
            trust_updates,
            total_cost,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup() -> (Charter, TokenLedger, TrustRegistry) {
        let charter = Charter::default();
        let ledger = TokenLedger::new(100_000, 100_000, 0);
        let mut trust = TrustRegistry::new();
        trust.register(AgentId::from_label("winner_1"));
        trust.register(AgentId::from_label("validator_a"));
        trust.register(AgentId::from_label("validator_b"));
        (charter, ledger, trust)
    }

    #[test]
    fn test_settle_winner_gets_reward() {
        let (charter, mut ledger, mut trust) = setup();
        let winner = AgentId::from_label("winner_1");

        let settlement = SettlementEngine::settle(
            "bounty-1",
            1000,
            Some(&winner),
            &[],
            &[],
            &charter,
            &mut ledger,
            &mut trust,
        );

        assert_eq!(settlement.winner, Some(winner.clone()));
        assert!(!settlement.payouts.is_empty());
        let winner_payout = settlement
            .payouts
            .iter()
            .find(|(id, _)| *id == winner)
            .unwrap()
            .1;
        assert!(winner_payout >= 500);
    }

    #[test]
    fn test_settle_validator_flaw_upheld() {
        let (charter, mut ledger, mut trust) = setup();
        let winner = AgentId::from_label("winner_1");
        let validator = AgentId::from_label("validator_a");

        let settlement = SettlementEngine::settle(
            "bounty-1",
            1000,
            Some(&winner),
            &[],
            &[(validator.clone(), ValidationVerdict::FlawUpheld)],
            &charter,
            &mut ledger,
            &mut trust,
        );

        let val_payout = settlement
            .payouts
            .iter()
            .find(|(id, _)| *id == validator)
            .unwrap()
            .1;
        assert_eq!(val_payout, 100);
        assert_eq!(
            settlement
                .trust_updates
                .iter()
                .find(|(id, _)| *id == validator)
                .unwrap()
                .1,
            5
        );
    }

    #[test]
    fn test_settle_validator_dismissed() {
        let (charter, mut ledger, mut trust) = setup();
        let winner = AgentId::from_label("winner_1");
        let validator = AgentId::from_label("validator_b");

        let settlement = SettlementEngine::settle(
            "bounty-1",
            1000,
            Some(&winner),
            &[],
            &[(validator.clone(), ValidationVerdict::FlawDismissed)],
            &charter,
            &mut ledger,
            &mut trust,
        );

        let val_entry = settlement
            .trust_updates
            .iter()
            .find(|(id, _)| *id == validator)
            .unwrap();
        assert_eq!(val_entry.1, -15);
        assert!(
            settlement
                .payouts
                .iter()
                .find(|(id, _)| *id == validator)
                .is_none()
        );
    }

    #[test]
    fn test_settle_no_winner() {
        let (charter, mut ledger, mut trust) = setup();

        let settlement = SettlementEngine::settle(
            "bounty-1",
            1000,
            None,
            &[],
            &[],
            &charter,
            &mut ledger,
            &mut trust,
        );

        assert_eq!(settlement.winner, None);
        assert!(settlement.payouts.is_empty());
        assert_eq!(settlement.total_cost, 0);
    }

    #[test]
    fn test_payment_floor() {
        let mut charter = Charter::default();
        charter.payment_floor = 0.7;
        let mut ledger = TokenLedger::new(100_000, 100_000, 0);
        let mut trust = TrustRegistry::new();
        let winner = AgentId::from_label("winner_1");
        trust.register(winner.clone());

        let settlement = SettlementEngine::settle(
            "bounty-1",
            1000,
            Some(&winner),
            &[],
            &[],
            &charter,
            &mut ledger,
            &mut trust,
        );

        let payout = settlement.payouts[0].1;
        assert!(payout >= 700);
    }
}

//! Token ledger + budget gating (CFO layer).
//!
//! Tracks all token expenditure per-agent, enforces total budget and daily burn caps,
//! and gates every LLM call against remaining budget before execution.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::types::AgentId;

// ─── Transaction Types ───────────────────────────────────────────────────────

/// Token transaction record for the audit log.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Transaction {
    pub timestamp_ms: u64,
    pub agent_id: AgentId,
    /// Positive = credit, negative = debit.
    pub amount: i64,
    pub purpose: TransactionPurpose,
    pub model: Option<String>,
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
}

/// Why tokens were moved.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum TransactionPurpose {
    SpawnFee,
    Execution,
    BountyReward,
    ValidationReward,
    Penalty,
    Refund,
}

// ─── Price Oracle ────────────────────────────────────────────────────────────

/// Maps model names to per-token costs (per 1M tokens, in ledger units).
#[derive(Debug, Clone)]
pub struct PriceOracle {
    input_costs: HashMap<String, u64>,
    output_costs: HashMap<String, u64>,
    default_input: u64,
    default_output: u64,
}

impl PriceOracle {
    pub fn new() -> Self {
        let mut input_costs = HashMap::new();
        let mut output_costs = HashMap::new();

        // Costs per 1M tokens in ledger units
        input_costs.insert("claude-sonnet-4-20250514".into(), 3);
        output_costs.insert("claude-sonnet-4-20250514".into(), 15);
        input_costs.insert("claude-haiku-4-5".into(), 1);
        output_costs.insert("claude-haiku-4-5".into(), 5);

        Self {
            input_costs,
            output_costs,
            default_input: 3,
            default_output: 15,
        }
    }

    /// Estimate cost for a request before execution.
    pub fn estimate_cost(&self, model: &str, estimated_input: u64, estimated_output: u64) -> u64 {
        let input_rate = self
            .input_costs
            .get(model)
            .copied()
            .unwrap_or(self.default_input);
        let output_rate = self
            .output_costs
            .get(model)
            .copied()
            .unwrap_or(self.default_output);
        // Saturating: token counts come from provider responses; an absurd
        // value must clamp at u64::MAX rather than wrap into a tiny cost
        // that slips past the budget gate.
        (estimated_input.saturating_mul(input_rate))
            .saturating_add(estimated_output.saturating_mul(output_rate))
            / 1_000_000
    }

    /// Calculate actual cost from a completed LLM response.
    pub fn actual_cost(&self, model: &str, input_tokens: u64, output_tokens: u64) -> u64 {
        let input_rate = self
            .input_costs
            .get(model)
            .copied()
            .unwrap_or(self.default_input);
        let output_rate = self
            .output_costs
            .get(model)
            .copied()
            .unwrap_or(self.default_output);
        (input_tokens.saturating_mul(input_rate))
            .saturating_add(output_tokens.saturating_mul(output_rate))
            / 1_000_000
    }
}

impl Default for PriceOracle {
    fn default() -> Self {
        Self::new()
    }
}

// ─── Budget Error ────────────────────────────────────────────────────────────

/// Budget gate rejection reasons.
#[derive(Debug, thiserror::Error)]
pub enum BudgetError {
    #[error("budget exhausted: remaining={remaining}, requested={requested}")]
    Exhausted { remaining: u64, requested: u64 },

    #[error("daily burn cap exceeded: cap={cap}, today={today}")]
    DailyCapExceeded { cap: u64, today: u64 },

    #[error("agent not found: {0:?}")]
    AgentNotFound(AgentId),
}

// ─── Token Ledger ────────────────────────────────────────────────────────────

/// In-memory token ledger tracking all expenditure with per-agent balances.
pub struct TokenLedger {
    total_budget: u64,
    total_spent: u64,
    daily_burn_cap: u64,
    today_spent: u64,
    spawn_fee: u64,
    balances: HashMap<AgentId, i64>,
    transactions: Vec<Transaction>,
    oracle: PriceOracle,
}

/// Saturating `u64 → i64` for ledger amounts. A token cost should never come
/// near `i64::MAX` (~9.2e18) in practice, but a corrupted/attacker-influenced
/// usage report could: a plain `as i64` would then wrap *negative*, turning a
/// debit into a credit and silently inflating a balance. Clamping to
/// `i64::MAX` keeps the sign correct so the worst case is an over-charge, not
/// a free top-up.
fn amount_as_i64(amount: u64) -> i64 {
    i64::try_from(amount).unwrap_or_else(|_| {
        // Reaching here means a single ledger amount exceeded i64::MAX — a
        // realistic token cost never does, so this signals a corrupted or
        // hostile usage report. Clamp (keeps the sign correct) but log loudly
        // so the corruption is diagnosable rather than silently absorbed.
        tracing::error!(
            target: "jfc::economy",
            amount,
            "ledger amount exceeds i64::MAX — clamping to i64::MAX (corrupt usage report?)"
        );
        i64::MAX
    })
}

impl TokenLedger {
    pub fn new(total_budget: u64, daily_burn_cap: u64, spawn_fee: u64) -> Self {
        linkscope::record_items("economy.ledger.new", 1);
        Self {
            total_budget,
            total_spent: 0,
            daily_burn_cap,
            today_spent: 0,
            spawn_fee,
            balances: HashMap::new(),
            transactions: Vec::new(),
            oracle: PriceOracle::new(),
        }
    }

    /// Budget gate: check if a request can proceed. Returns estimated cost on success.
    pub fn gate_check(
        &self,
        model: &str,
        estimated_input: u64,
        estimated_output: u64,
    ) -> Result<u64, BudgetError> {
        let _linkscope_gate = linkscope::phase("economy.ledger.gate_check");
        let estimated_cost = self
            .oracle
            .estimate_cost(model, estimated_input, estimated_output);
        let remaining = self.total_budget.saturating_sub(self.total_spent);

        if estimated_cost > remaining {
            linkscope::record_items("economy.ledger.gate.exhausted", 1);
            return Err(BudgetError::Exhausted {
                remaining,
                requested: estimated_cost,
            });
        }

        if self.today_spent + estimated_cost > self.daily_burn_cap {
            linkscope::record_items("economy.ledger.gate.daily_cap", 1);
            return Err(BudgetError::DailyCapExceeded {
                cap: self.daily_burn_cap,
                today: self.today_spent,
            });
        }

        linkscope::record_items("economy.ledger.gate.ok", 1);
        Ok(estimated_cost)
    }

    /// Debit the spawn fee when an agent is created.
    pub fn debit_spawn(&mut self, agent_id: &AgentId) -> Result<(), BudgetError> {
        let _linkscope_debit = linkscope::phase("economy.ledger.debit_spawn");
        let remaining = self.total_budget.saturating_sub(self.total_spent);
        if self.spawn_fee > remaining {
            linkscope::record_items("economy.ledger.debit_spawn.exhausted", 1);
            return Err(BudgetError::Exhausted {
                remaining,
                requested: self.spawn_fee,
            });
        }

        self.total_spent = self.total_spent.saturating_add(self.spawn_fee);
        self.today_spent = self.today_spent.saturating_add(self.spawn_fee);
        let fee = amount_as_i64(self.spawn_fee);
        *self.balances.entry(agent_id.clone()).or_insert(0) -= fee;

        self.transactions.push(Transaction {
            timestamp_ms: now_ms(),
            agent_id: agent_id.clone(),
            amount: -fee,
            purpose: TransactionPurpose::SpawnFee,
            model: None,
            input_tokens: None,
            output_tokens: None,
        });

        linkscope::record_items("economy.ledger.debit_spawn.ok", 1);
        Ok(())
    }

    /// Record actual token usage after an LLM call completes.
    pub fn record_usage(
        &mut self,
        agent_id: &AgentId,
        model: &str,
        input_tokens: u64,
        output_tokens: u64,
    ) {
        let _linkscope_usage = linkscope::phase("economy.ledger.record_usage");
        let cost = self.oracle.actual_cost(model, input_tokens, output_tokens);
        self.total_spent = self.total_spent.saturating_add(cost);
        self.today_spent = self.today_spent.saturating_add(cost);
        let cost_i64 = amount_as_i64(cost);
        *self.balances.entry(agent_id.clone()).or_insert(0) -= cost_i64;

        self.transactions.push(Transaction {
            timestamp_ms: now_ms(),
            agent_id: agent_id.clone(),
            amount: -cost_i64,
            purpose: TransactionPurpose::Execution,
            model: Some(model.to_string()),
            input_tokens: Some(input_tokens),
            output_tokens: Some(output_tokens),
        });
        linkscope::record_items("economy.ledger.usage.recorded", 1);
    }

    /// Credit tokens to an agent (bounty reward, validation reward, refund).
    pub fn credit(&mut self, agent_id: &AgentId, amount: u64, purpose: TransactionPurpose) {
        let _linkscope_credit = linkscope::phase("economy.ledger.credit");
        let amount_i64 = amount_as_i64(amount);
        *self.balances.entry(agent_id.clone()).or_insert(0) += amount_i64;

        self.transactions.push(Transaction {
            timestamp_ms: now_ms(),
            agent_id: agent_id.clone(),
            amount: amount_i64,
            purpose,
            model: None,
            input_tokens: None,
            output_tokens: None,
        });
        linkscope::record_items("economy.ledger.credit.recorded", 1);
    }

    /// Remaining budget (total - spent).
    pub fn remaining(&self) -> u64 {
        self.total_budget.saturating_sub(self.total_spent)
    }

    /// Total tokens spent so far.
    pub fn total_spent(&self) -> u64 {
        self.total_spent
    }

    /// Per-agent balance (negative = net debtor).
    pub fn agent_balance(&self, agent_id: &AgentId) -> i64 {
        self.balances.get(agent_id).copied().unwrap_or(0)
    }

    /// Full transaction audit log.
    pub fn transactions(&self) -> &[Transaction] {
        &self.transactions
    }

    /// Reference to the price oracle.
    pub fn oracle(&self) -> &PriceOracle {
        &self.oracle
    }

    /// Reset daily spend counter (call at day boundary).
    pub fn reset_daily(&mut self) {
        linkscope::record_items("economy.ledger.reset_daily", 1);
        self.today_spent = 0;
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn test_agent() -> AgentId {
        AgentId::from_label("test-agent-001")
    }

    // A pathological amount near u64::MAX must clamp to i64::MAX, not wrap
    // negative — wrapping would turn a debit into a credit.
    #[test]
    fn amount_as_i64_clamps_instead_of_wrapping_robust() {
        assert_eq!(amount_as_i64(0), 0);
        assert_eq!(amount_as_i64(1_000), 1_000);
        assert_eq!(amount_as_i64(i64::MAX as u64), i64::MAX);
        assert_eq!(amount_as_i64(u64::MAX), i64::MAX);
        // The whole point: the clamped value stays positive (a debit subtracts
        // a positive number) rather than becoming a giant negative credit.
        assert!(amount_as_i64(u64::MAX) > 0);
    }

    #[test]
    fn test_budget_gate_allows() {
        let ledger = TokenLedger::new(1_000_000, 500_000, 1000);
        // 100k input + 50k output on sonnet: (100000*3 + 50000*15)/1M = (300000+750000)/1M = 1
        // Use larger numbers to get a meaningful estimate
        let result = ledger.gate_check("claude-sonnet-4-20250514", 1_000_000, 500_000);
        assert!(result.is_ok());
        let cost = result.unwrap();
        // (1_000_000 * 3 + 500_000 * 15) / 1_000_000 = (3_000_000 + 7_500_000) / 1_000_000 = 10
        assert_eq!(cost, 10);
    }

    #[test]
    fn test_budget_gate_rejects_exhausted() {
        let mut ledger = TokenLedger::new(100, 1000, 0);
        // Spend 95 manually
        ledger.total_spent = 95;
        // Remaining = 5, try to spend 10
        // Need model/tokens that produce cost > 5
        // 2_000_000 input on sonnet: (2_000_000 * 3) / 1_000_000 = 6
        let result = ledger.gate_check("claude-sonnet-4-20250514", 2_000_000, 0);
        assert!(result.is_err());
        match result.unwrap_err() {
            BudgetError::Exhausted {
                remaining,
                requested,
            } => {
                assert_eq!(remaining, 5);
                assert_eq!(requested, 6);
            }
            other => panic!("expected Exhausted, got: {other:?}"),
        }
    }

    #[test]
    fn test_spawn_fee_deducted() {
        let spawn_fee = 500;
        let mut ledger = TokenLedger::new(10_000, 10_000, spawn_fee);
        let agent = test_agent();

        ledger.debit_spawn(&agent).unwrap();

        assert_eq!(ledger.agent_balance(&agent), -(spawn_fee as i64));
        assert_eq!(ledger.total_spent(), spawn_fee);
        assert_eq!(ledger.remaining(), 10_000 - spawn_fee);
        assert_eq!(ledger.transactions().len(), 1);
        assert_eq!(
            ledger.transactions()[0].purpose,
            TransactionPurpose::SpawnFee
        );
    }

    #[test]
    fn test_record_usage() {
        let mut ledger = TokenLedger::new(1_000_000, 1_000_000, 0);
        let agent = test_agent();
        let model = "claude-sonnet-4-20250514";

        // 1_000_000 input + 500_000 output
        // cost = (1_000_000 * 3 + 500_000 * 15) / 1_000_000 = 10
        ledger.record_usage(&agent, model, 1_000_000, 500_000);

        assert_eq!(ledger.total_spent(), 10);
        assert_eq!(ledger.agent_balance(&agent), -10);

        let tx = &ledger.transactions()[0];
        assert_eq!(tx.amount, -10);
        assert_eq!(tx.input_tokens, Some(1_000_000));
        assert_eq!(tx.output_tokens, Some(500_000));
        assert_eq!(tx.model.as_deref(), Some(model));
        assert_eq!(tx.purpose, TransactionPurpose::Execution);
    }

    #[test]
    fn test_credit_reward() {
        let mut ledger = TokenLedger::new(1_000_000, 1_000_000, 0);
        let agent = test_agent();

        ledger.credit(&agent, 200, TransactionPurpose::BountyReward);

        assert_eq!(ledger.agent_balance(&agent), 200);
        assert_eq!(ledger.transactions().len(), 1);
        assert_eq!(ledger.transactions()[0].amount, 200);
        assert_eq!(
            ledger.transactions()[0].purpose,
            TransactionPurpose::BountyReward
        );
    }

    #[test]
    fn test_daily_cap() {
        let mut ledger = TokenLedger::new(1_000_000, 50, 0);
        // Simulate having spent 45 today
        ledger.today_spent = 45;

        // Try to spend 10 more (45 + 10 = 55 > cap of 50)
        // Need cost = 10: (10_000_000 * 1) / 1_000_000 = 10 on haiku input
        let result = ledger.gate_check("claude-haiku-4-5", 10_000_000, 0);
        assert!(result.is_err());
        match result.unwrap_err() {
            BudgetError::DailyCapExceeded { cap, today } => {
                assert_eq!(cap, 50);
                assert_eq!(today, 45);
            }
            other => panic!("expected DailyCapExceeded, got: {other:?}"),
        }
    }

    #[test]
    fn test_spawn_fee_rejects_when_exhausted() {
        let mut ledger = TokenLedger::new(100, 1000, 200);
        let agent = test_agent();

        let result = ledger.debit_spawn(&agent);
        assert!(result.is_err());
        match result.unwrap_err() {
            BudgetError::Exhausted {
                remaining,
                requested,
            } => {
                assert_eq!(remaining, 100);
                assert_eq!(requested, 200);
            }
            other => panic!("expected Exhausted, got: {other:?}"),
        }
    }

    #[test]
    fn test_price_oracle_defaults() {
        let oracle = PriceOracle::new();
        // Unknown model uses defaults (3 input, 15 output)
        let cost = oracle.actual_cost("unknown-model", 1_000_000, 1_000_000);
        // (1M * 3 + 1M * 15) / 1M = 18
        assert_eq!(cost, 18);
    }

    #[test]
    fn test_multiple_operations_audit_trail() {
        let mut ledger = TokenLedger::new(1_000_000, 1_000_000, 100);
        let agent = test_agent();

        ledger.debit_spawn(&agent).unwrap();
        ledger.record_usage(&agent, "claude-haiku-4-5", 5_000_000, 1_000_000);
        ledger.credit(&agent, 50, TransactionPurpose::ValidationReward);

        assert_eq!(ledger.transactions().len(), 3);
        // Spawn: -100
        // Usage: (5M*1 + 1M*5)/1M = 10 → -10
        // Credit: +50
        // Net balance: -100 - 10 + 50 = -60
        assert_eq!(ledger.agent_balance(&agent), -60);
    }
}

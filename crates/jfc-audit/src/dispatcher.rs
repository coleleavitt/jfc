use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tracing::{debug, warn};

use crate::error::Result;
use crate::types::{Finding, PocStatus, ValidatorOutcome, ValidatorVerdict};

/// Outcome of a validation attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ValidationOutcome {
    Validated,
    FalsePositive,
    Inconclusive,
    BudgetExhausted,
}

/// Market health status used to gate dispatches.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MarketHealth {
    pub score: f64,
    pub is_healthy: bool,
}

/// Trait abstracting the bounty economy for validation.
#[async_trait]
pub trait BountyRunner: Send + Sync {
    /// Run a validation bounty for a finding. Returns the outcome.
    async fn validate_finding(&self, finding: &Finding) -> Result<ValidationOutcome>;

    /// Check market health before dispatching.
    async fn market_health(&self) -> Result<MarketHealth>;
}

/// Dispatches findings to the bounty economy for validation.
pub struct AuditBountyDispatcher<B: BountyRunner> {
    runner: B,
    health_threshold: f64,
}

impl<B: BountyRunner> AuditBountyDispatcher<B> {
    pub fn new(runner: B) -> Self {
        Self {
            runner,
            health_threshold: 0.3,
        }
    }

    /// Set the minimum market health score required to dispatch.
    pub fn with_health_threshold(mut self, threshold: f64) -> Self {
        self.health_threshold = threshold;
        self
    }

    /// Validate a single finding through the bounty economy.
    pub async fn validate(&self, finding: &mut Finding) -> Result<ValidationOutcome> {
        // Check market health first
        let health = self.runner.market_health().await?;
        if !health.is_healthy || health.score < self.health_threshold {
            warn!(
                score = health.score,
                "market health below threshold, skipping dispatch"
            );
            return Ok(ValidationOutcome::BudgetExhausted);
        }

        let outcome = self.runner.validate_finding(finding).await?;

        // Update finding based on outcome
        let verdict = ValidatorVerdict {
            validator_id: "bounty_economy".to_string(),
            outcome: match outcome {
                ValidationOutcome::Validated => ValidatorOutcome::Confirmed,
                ValidationOutcome::FalsePositive => ValidatorOutcome::FalsePositive,
                ValidationOutcome::Inconclusive | ValidationOutcome::BudgetExhausted => {
                    ValidatorOutcome::Inconclusive
                }
            },
            reasoning: format!("Bounty validation result: {outcome:?}"),
            timestamp: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
        };

        finding.validator_verdicts.push(verdict);

        // Update PoC status
        match outcome {
            ValidationOutcome::Validated => {
                finding.poc_status = PocStatus::Validated;
            }
            ValidationOutcome::FalsePositive => {
                finding.poc_status = PocStatus::FailedToReproduce;
            }
            _ => {}
        }

        debug!(?outcome, id = %finding.id, "validation complete");
        Ok(outcome)
    }

    /// Validate a batch of findings. Stops early if budget is exhausted.
    pub async fn validate_batch(&self, findings: &mut [Finding]) -> Result<Vec<ValidationOutcome>> {
        let mut outcomes = Vec::with_capacity(findings.len());

        for finding in findings.iter_mut() {
            let outcome = self.validate(finding).await?;
            let exhausted = outcome == ValidationOutcome::BudgetExhausted;
            outcomes.push(outcome);
            if exhausted {
                // Fill remaining with BudgetExhausted
                outcomes.resize(findings.len(), ValidationOutcome::BudgetExhausted);
                break;
            }
        }

        Ok(outcomes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::*;

    struct MockBountyRunner {
        outcome: ValidationOutcome,
        healthy: bool,
    }

    #[async_trait]
    impl BountyRunner for MockBountyRunner {
        async fn validate_finding(&self, _finding: &Finding) -> Result<ValidationOutcome> {
            Ok(self.outcome)
        }

        async fn market_health(&self) -> Result<MarketHealth> {
            Ok(MarketHealth {
                score: if self.healthy { 0.8 } else { 0.1 },
                is_healthy: self.healthy,
            })
        }
    }

    fn sample_finding() -> Finding {
        let location = SourceSpan {
            file: "src/main.rs".to_string(),
            start_line: 10,
            end_line: 15,
        };
        let id = Finding::compute_id(FindingKind::TaintedSink, &location, "fn:main");
        Finding {
            id,
            severity: Severity::High,
            kind: FindingKind::TaintedSink,
            location,
            granularity: Granularity::Function,
            reachability_path: vec!["fn:main".to_string()],
            taint_chain: None,
            preconditions: vec![],
            validator_verdicts: vec![],
            poc_status: PocStatus::NotAttempted,
            first_seen_revision: 1,
            last_seen_revision: 1,
            suppressed: None,
        }
    }

    #[tokio::test]
    async fn validate_marks_finding_normal() {
        let runner = MockBountyRunner {
            outcome: ValidationOutcome::Validated,
            healthy: true,
        };
        let dispatcher = AuditBountyDispatcher::new(runner);
        let mut finding = sample_finding();

        let outcome = dispatcher.validate(&mut finding).await.unwrap();
        assert_eq!(outcome, ValidationOutcome::Validated);
        assert_eq!(finding.poc_status, PocStatus::Validated);
        assert_eq!(finding.validator_verdicts.len(), 1);
        assert_eq!(
            finding.validator_verdicts[0].outcome,
            ValidatorOutcome::Confirmed
        );
    }

    #[tokio::test]
    async fn health_gate_skips_dispatch_robust() {
        let runner = MockBountyRunner {
            outcome: ValidationOutcome::Validated,
            healthy: false,
        };
        let dispatcher = AuditBountyDispatcher::new(runner);
        let mut finding = sample_finding();

        let outcome = dispatcher.validate(&mut finding).await.unwrap();
        assert_eq!(outcome, ValidationOutcome::BudgetExhausted);
        // Finding should NOT be modified since we didn't actually dispatch
        assert_eq!(finding.poc_status, PocStatus::NotAttempted);
        // But a verdict is still recorded
        assert!(finding.validator_verdicts.is_empty());
    }
}

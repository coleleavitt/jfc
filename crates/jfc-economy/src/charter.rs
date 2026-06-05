//! Governance charter — declarative constraints and sanctions loaded from YAML,
//! enforced at runtime. Immutable once loaded for a market cycle.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// Charter violation type.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ViolationType {
    DoubleSubmission,
    SelfValidation,
    ExceedMaxSolvers,
    BelowMinTrust,
    BudgetExceeded,
    TimeoutExceeded,
    SandboxEscape,
    TestDeletion,
}

/// Sanction for a violation — declared data, not hardcoded logic.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Sanction {
    pub trust_penalty: u8,
    pub description: String,
    /// If true, the violating action is also prevented (not just penalized).
    pub blocks_action: bool,
}

/// The governance charter — loaded from YAML, enforced at runtime.
///
/// Immutable during a market cycle. Sanctions are manifest-declared consequences
/// attached to public evidence, reshaping agent behavior through incentive structure
/// rather than prompt-level prohibitions.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Charter {
    pub max_budget_per_bounty: u64,
    pub max_solvers: u8,
    pub max_validators: u8,
    pub min_trust_for_solver: u8,
    pub min_trust_for_validator: u8,
    pub validation_rounds: u8,
    pub self_validation_allowed: bool,
    pub max_token_spend_per_agent: u64,
    pub early_termination_confidence: f32,
    pub spawn_fee: u64,
    pub surge_multiplier: f32,
    /// Minimum payment ratio (0.5 = solver always gets ≥50%).
    pub payment_floor: f32,
    pub sanctions: HashMap<ViolationType, Sanction>,
}

impl Charter {
    /// Load charter from a YAML string.
    pub fn from_yaml(yaml: &str) -> Result<Self, serde_yaml::Error> {
        serde_yaml::from_str(yaml)
    }

    /// Load charter from a file path.
    pub fn from_file(path: &std::path::Path) -> Result<Self, CharterError> {
        let content =
            std::fs::read_to_string(path).map_err(|e| CharterError::IoError(e.to_string()))?;
        Self::from_yaml(&content).map_err(|e| CharterError::ParseError(e.to_string()))
    }

    /// Check if a violation has a registered sanction.
    pub fn check_violation(&self, violation: &ViolationType) -> Option<&Sanction> {
        self.sanctions.get(violation)
    }

    /// Validate that a solver meets minimum trust.
    pub fn can_solve(&self, trust_score: u8) -> bool {
        trust_score >= self.min_trust_for_solver
    }

    /// Validate that a validator meets minimum trust.
    pub fn can_validate(&self, trust_score: u8) -> bool {
        trust_score >= self.min_trust_for_validator
    }
}

impl Default for Charter {
    fn default() -> Self {
        let mut sanctions = HashMap::new();
        sanctions.insert(
            ViolationType::DoubleSubmission,
            Sanction {
                trust_penalty: 20,
                description: "Solver submitted solution twice for same bounty".into(),
                blocks_action: true,
            },
        );
        sanctions.insert(
            ViolationType::SelfValidation,
            Sanction {
                trust_penalty: 30,
                description: "Agent attempted to validate own solution".into(),
                blocks_action: true,
            },
        );
        sanctions.insert(
            ViolationType::SandboxEscape,
            Sanction {
                trust_penalty: 50,
                description: "Agent attempted to access files outside sandbox".into(),
                blocks_action: true,
            },
        );
        sanctions.insert(
            ViolationType::TestDeletion,
            Sanction {
                trust_penalty: 25,
                description: "Solution deletes or modifies existing tests".into(),
                blocks_action: false,
            },
        );
        sanctions.insert(
            ViolationType::ExceedMaxSolvers,
            Sanction {
                trust_penalty: 0,
                description: "Too many solvers for this bounty".into(),
                blocks_action: true,
            },
        );
        sanctions.insert(
            ViolationType::BelowMinTrust,
            Sanction {
                trust_penalty: 0,
                description: "Agent trust below minimum for role".into(),
                blocks_action: true,
            },
        );

        Self {
            max_budget_per_bounty: u64::MAX,
            max_solvers: 3,
            max_validators: 2,
            min_trust_for_solver: 30,
            min_trust_for_validator: 40,
            validation_rounds: 3,
            self_validation_allowed: false,
            max_token_spend_per_agent: 5000,
            early_termination_confidence: 0.95,
            spawn_fee: 50,
            surge_multiplier: 1.15,
            payment_floor: 0.5,
            sanctions,
        }
    }
}

/// Errors that can occur when loading a charter.
#[derive(Debug, thiserror::Error)]
pub enum CharterError {
    #[error("IO error: {0}")]
    IoError(String),
    #[error("parse error: {0}")]
    ParseError(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_charter_default() {
        let charter = Charter::default();
        assert_eq!(charter.max_solvers, 3);
        assert_eq!(charter.min_trust_for_solver, 30);
        assert_eq!(charter.min_trust_for_validator, 40);
        assert_eq!(charter.max_validators, 2);
        assert!(!charter.self_validation_allowed);
    }

    #[test]
    fn test_charter_from_yaml() {
        let yaml = r#"
max_budget_per_bounty: 5000
max_solvers: 5
max_validators: 3
min_trust_for_solver: 25
min_trust_for_validator: 35
validation_rounds: 2
self_validation_allowed: false
max_token_spend_per_agent: 3000
early_termination_confidence: 0.9
spawn_fee: 100
surge_multiplier: 1.25
payment_floor: 0.6
sanctions:
  DoubleSubmission:
    trust_penalty: 15
    description: "Double submission detected"
    blocks_action: true
  SandboxEscape:
    trust_penalty: 60
    description: "Sandbox escape attempt"
    blocks_action: true
"#;
        let charter = Charter::from_yaml(yaml).unwrap();
        assert_eq!(charter.max_solvers, 5);
        assert_eq!(charter.max_validators, 3);
        assert_eq!(charter.min_trust_for_solver, 25);
        assert_eq!(charter.min_trust_for_validator, 35);
        assert_eq!(charter.spawn_fee, 100);
        assert!((charter.surge_multiplier - 1.25).abs() < f32::EPSILON);
        assert!((charter.payment_floor - 0.6).abs() < f32::EPSILON);

        let sanction = charter
            .sanctions
            .get(&ViolationType::SandboxEscape)
            .unwrap();
        assert_eq!(sanction.trust_penalty, 60);
        assert!(sanction.blocks_action);
    }

    #[test]
    fn test_charter_can_solve() {
        let charter = Charter::default();
        assert!(charter.can_solve(50));
        assert!(charter.can_solve(30)); // exactly at threshold
        assert!(!charter.can_solve(20));
        assert!(!charter.can_solve(0));
    }

    #[test]
    fn test_charter_can_validate() {
        let charter = Charter::default();
        assert!(charter.can_validate(50));
        assert!(charter.can_validate(40)); // exactly at threshold
        assert!(!charter.can_validate(35));
        assert!(!charter.can_validate(0));
    }

    #[test]
    fn test_charter_violation_sanction() {
        let charter = Charter::default();
        let sanction = charter
            .check_violation(&ViolationType::SelfValidation)
            .unwrap();
        assert_eq!(sanction.trust_penalty, 30);
        assert!(sanction.blocks_action);

        let sanction = charter
            .check_violation(&ViolationType::SandboxEscape)
            .unwrap();
        assert_eq!(sanction.trust_penalty, 50);
        assert!(sanction.blocks_action);

        // TestDeletion: flagged but not blocked
        let sanction = charter
            .check_violation(&ViolationType::TestDeletion)
            .unwrap();
        assert_eq!(sanction.trust_penalty, 25);
        assert!(!sanction.blocks_action);
    }

    #[test]
    fn test_charter_unknown_violation() {
        let charter = Charter::default();
        // BudgetExceeded and TimeoutExceeded are not in default sanctions
        assert!(
            charter
                .check_violation(&ViolationType::BudgetExceeded)
                .is_none()
        );
        assert!(
            charter
                .check_violation(&ViolationType::TimeoutExceeded)
                .is_none()
        );
    }

    #[test]
    fn test_charter_serialization_roundtrip() {
        let original = Charter::default();
        let yaml = serde_yaml::to_string(&original).unwrap();
        let restored: Charter = serde_yaml::from_str(&yaml).unwrap();

        assert_eq!(restored.max_solvers, original.max_solvers);
        assert_eq!(restored.max_validators, original.max_validators);
        assert_eq!(restored.min_trust_for_solver, original.min_trust_for_solver);
        assert_eq!(
            restored.min_trust_for_validator,
            original.min_trust_for_validator
        );
        assert_eq!(restored.validation_rounds, original.validation_rounds);
        assert_eq!(
            restored.self_validation_allowed,
            original.self_validation_allowed
        );
        assert_eq!(restored.spawn_fee, original.spawn_fee);
        assert!((restored.surge_multiplier - original.surge_multiplier).abs() < f32::EPSILON);
        assert!((restored.payment_floor - original.payment_floor).abs() < f32::EPSILON);

        // Verify sanctions survived roundtrip
        assert_eq!(restored.sanctions.len(), original.sanctions.len());
        for (violation, original_sanction) in &original.sanctions {
            let restored_sanction = restored.sanctions.get(violation).unwrap();
            assert_eq!(
                restored_sanction.trust_penalty,
                original_sanction.trust_penalty
            );
            assert_eq!(
                restored_sanction.blocks_action,
                original_sanction.blocks_action
            );
        }
    }
}

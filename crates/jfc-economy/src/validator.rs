//! Validator agent spawning and the 3-round structured validation protocol.
//!
//! Protocol rounds:
//! 1. **Challenge** — Validator proposes a flaw in the solution
//! 2. **Defense** — Solver defends against the challenge
//! 3. **Adjudication** — Mechanistic test: does the proposed test actually fail?
//!
//! Key invariants:
//! - Self-validation is structurally forbidden (identity separation)
//! - Validators operate sealed (cannot see each other's verdicts until all complete)
//! - Early termination on high confidence (≥0.95) with no flaw saves token budget

use crate::types::{AgentId, ValidationChallenge, ValidationVerdict};

/// Early termination confidence threshold.
const EARLY_TERMINATION_CONFIDENCE: f32 = 0.95;

/// Errors arising from the validation protocol.
#[derive(Debug, thiserror::Error)]
pub enum ValidationError {
    #[error("self-validation forbidden: agent {0:?} cannot validate own solution")]
    SelfValidationForbidden(AgentId),

    #[error("validator not found: {0:?}")]
    NotFound(AgentId),

    #[error("validation already complete")]
    AlreadyComplete,

    #[error("invalid round transition: expected {expected:?}, currently at {current:?}")]
    InvalidRound {
        expected: ValidationRound,
        current: ValidationRound,
    },
}

/// Validation round in the 3-round protocol.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ValidationRound {
    /// Round 1: Validator proposes a flaw.
    Challenge,
    /// Round 2: Solver defends against the challenge.
    Defense,
    /// Round 3: Adjudication (test-based, mechanistic).
    Adjudication,
    /// Protocol complete.
    Done,
}

/// A single validation session: one validator examining one solution.
///
/// Sessions are independent and sealed — no session can observe another's state.
#[derive(Debug, Clone)]
pub struct ValidationSession {
    pub validator_id: AgentId,
    pub solution_agent_id: AgentId,
    pub bounty_id: String,
    pub current_round: ValidationRound,
    pub challenge: Option<ValidationChallenge>,
    pub defense: Option<String>,
    pub verdict: Option<ValidationVerdict>,
    pub early_terminated: bool,
}

impl ValidationSession {
    /// Create a new validation session.
    ///
    /// # Errors
    /// Returns [`ValidationError::SelfValidationForbidden`] if the validator and solution
    /// author are the same agent (structural prevention of bug-planting attacks).
    pub fn new(
        validator_id: AgentId,
        solution_agent_id: AgentId,
        bounty_id: String,
    ) -> Result<Self, ValidationError> {
        if validator_id == solution_agent_id {
            return Err(ValidationError::SelfValidationForbidden(validator_id));
        }

        Ok(Self {
            validator_id,
            solution_agent_id,
            bounty_id,
            current_round: ValidationRound::Challenge,
            challenge: None,
            defense: None,
            verdict: None,
            early_terminated: false,
        })
    }

    /// Round 1: Validator submits a challenge (proposed flaw).
    ///
    /// If confidence ≥ 0.95 and no flaw is proposed, the session early-terminates
    /// with [`ValidationVerdict::NoFlawFound`], saving downstream token budget.
    pub fn submit_challenge(
        &mut self,
        challenge: ValidationChallenge,
    ) -> Result<(), ValidationError> {
        if self.current_round != ValidationRound::Challenge {
            return Err(ValidationError::AlreadyComplete);
        }

        // Early termination: high confidence + no flaw → skip defense/adjudication
        if challenge.confidence >= EARLY_TERMINATION_CONFIDENCE
            && challenge.proposed_flaw.is_empty()
        {
            self.verdict = Some(ValidationVerdict::NoFlawFound);
            self.current_round = ValidationRound::Done;
            self.early_terminated = true;
            return Ok(());
        }

        self.challenge = Some(challenge);
        self.current_round = ValidationRound::Defense;
        Ok(())
    }

    /// Round 2: Solver submits a defense against the proposed flaw.
    pub fn submit_defense(&mut self, defense: String) -> Result<(), ValidationError> {
        if self.current_round != ValidationRound::Defense {
            return Err(ValidationError::InvalidRound {
                expected: ValidationRound::Defense,
                current: self.current_round.clone(),
            });
        }

        self.defense = Some(defense);
        self.current_round = ValidationRound::Adjudication;
        Ok(())
    }

    /// Round 3: Adjudicate based on mechanistic test execution.
    ///
    /// `test_fails` indicates whether the validator's proposed test actually fails
    /// against the solution. If it does, the flaw is real.
    pub fn adjudicate(&mut self, test_fails: bool) -> Result<ValidationVerdict, ValidationError> {
        if self.current_round != ValidationRound::Adjudication {
            return Err(ValidationError::InvalidRound {
                expected: ValidationRound::Adjudication,
                current: self.current_round.clone(),
            });
        }

        let verdict = if test_fails {
            ValidationVerdict::FlawUpheld
        } else {
            ValidationVerdict::FlawDismissed
        };

        self.verdict = Some(verdict);
        self.current_round = ValidationRound::Done;
        Ok(verdict)
    }

    /// Whether this session has reached a terminal state.
    pub fn is_complete(&self) -> bool {
        self.current_round == ValidationRound::Done
    }

    /// The final verdict, if the session is complete.
    pub fn verdict(&self) -> Option<ValidationVerdict> {
        self.verdict
    }
}

/// Manages all validation sessions for a bounty.
///
/// Sessions are sealed: each validator operates independently and cannot observe
/// other validators' verdicts until [`ValidationPool::all_complete`] returns true.
/// This prevents peer-pressure convergence (MAEBE finding).
pub struct ValidationPool {
    sessions: Vec<ValidationSession>,
}

impl ValidationPool {
    pub fn new() -> Self {
        Self {
            sessions: Vec::new(),
        }
    }

    /// Start a new sealed validation session.
    ///
    /// Returns the session index on success.
    ///
    /// # Errors
    /// Returns [`ValidationError::SelfValidationForbidden`] if validator == solution author.
    pub fn start_session(
        &mut self,
        validator_id: AgentId,
        solution_agent_id: AgentId,
        bounty_id: String,
    ) -> Result<usize, ValidationError> {
        let session = ValidationSession::new(validator_id, solution_agent_id, bounty_id)?;
        self.sessions.push(session);
        Ok(self.sessions.len() - 1)
    }

    /// Get an immutable reference to a session by index.
    pub fn get(&self, index: usize) -> Option<&ValidationSession> {
        self.sessions.get(index)
    }

    /// Get a mutable reference to a session by index.
    pub fn get_mut(&mut self, index: usize) -> Option<&mut ValidationSession> {
        self.sessions.get_mut(index)
    }

    /// Whether all sessions have reached a terminal state.
    ///
    /// Verdicts should only be collected after this returns true (sealed protocol).
    pub fn all_complete(&self) -> bool {
        !self.sessions.is_empty() && self.sessions.iter().all(|s| s.is_complete())
    }

    /// Collect all verdicts. Only meaningful after [`Self::all_complete`] returns true.
    pub fn verdicts(&self) -> Vec<(AgentId, ValidationVerdict)> {
        self.sessions
            .iter()
            .filter(|s| s.is_complete())
            .filter_map(|s| s.verdict().map(|v| (s.validator_id.clone(), v)))
            .collect()
    }

    /// Count of sessions where the proposed flaw was upheld.
    pub fn flaws_found(&self) -> usize {
        self.sessions
            .iter()
            .filter(|s| s.verdict() == Some(ValidationVerdict::FlawUpheld))
            .count()
    }

    /// Count of sessions that early-terminated (high confidence, no flaw).
    pub fn early_terminations(&self) -> usize {
        self.sessions.iter().filter(|s| s.early_terminated).count()
    }

    /// Total number of sessions in this pool.
    pub fn session_count(&self) -> usize {
        self.sessions.len()
    }
}

impl Default for ValidationPool {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn validator_id(name: &str) -> AgentId {
        AgentId::from_label(format!("validator_{name}"))
    }

    fn solver_id(name: &str) -> AgentId {
        AgentId::from_label(format!("solver_{name}"))
    }

    fn make_challenge(
        validator: &AgentId,
        solver: &AgentId,
        bounty: &str,
        flaw: &str,
        confidence: f32,
    ) -> ValidationChallenge {
        ValidationChallenge {
            validator_id: validator.clone(),
            solution_agent_id: solver.clone(),
            bounty_id: bounty.to_string(),
            proposed_flaw: flaw.to_string(),
            test_code: Some("assert!(false);".to_string()),
            confidence,
        }
    }

    #[test]
    fn test_self_validation_blocked() {
        let agent = AgentId::from_label("agent_same");
        let result = ValidationSession::new(agent.clone(), agent, "bounty-1".to_string());

        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            matches!(err, ValidationError::SelfValidationForbidden(_)),
            "expected SelfValidationForbidden, got: {err:?}"
        );
    }

    #[test]
    fn test_three_round_flaw_upheld() {
        let v = validator_id("alice");
        let s = solver_id("bob");
        let mut session =
            ValidationSession::new(v.clone(), s.clone(), "bounty-1".to_string()).unwrap();

        // Round 1: Challenge
        let challenge = make_challenge(&v, &s, "bounty-1", "off-by-one in loop", 0.8);
        session.submit_challenge(challenge).unwrap();
        assert_eq!(session.current_round, ValidationRound::Defense);

        // Round 2: Defense
        session
            .submit_defense("The loop is correct because...".to_string())
            .unwrap();
        assert_eq!(session.current_round, ValidationRound::Adjudication);

        // Round 3: Adjudicate — test fails, flaw is real
        let verdict = session.adjudicate(true).unwrap();
        assert_eq!(verdict, ValidationVerdict::FlawUpheld);
        assert!(session.is_complete());
        assert!(!session.early_terminated);
    }

    #[test]
    fn test_three_round_flaw_dismissed() {
        let v = validator_id("alice");
        let s = solver_id("bob");
        let mut session =
            ValidationSession::new(v.clone(), s.clone(), "bounty-1".to_string()).unwrap();

        let challenge = make_challenge(&v, &s, "bounty-1", "null pointer dereference", 0.6);
        session.submit_challenge(challenge).unwrap();
        session
            .submit_defense("Pointer is checked on line 42".to_string())
            .unwrap();

        // Test passes — flaw was spurious
        let verdict = session.adjudicate(false).unwrap();
        assert_eq!(verdict, ValidationVerdict::FlawDismissed);
        assert!(session.is_complete());
    }

    #[test]
    fn test_early_termination() {
        let v = validator_id("alice");
        let s = solver_id("bob");
        let mut session =
            ValidationSession::new(v.clone(), s.clone(), "bounty-1".to_string()).unwrap();

        // High confidence, no flaw → early terminate
        let challenge = make_challenge(&v, &s, "bounty-1", "", 0.97);
        session.submit_challenge(challenge).unwrap();

        assert!(session.is_complete());
        assert!(session.early_terminated);
        assert_eq!(session.verdict(), Some(ValidationVerdict::NoFlawFound));
        assert_eq!(session.current_round, ValidationRound::Done);
    }

    #[test]
    fn test_sealed_verdicts() {
        let mut pool = ValidationPool::new();
        let s = solver_id("bob");

        let v1 = validator_id("alice");
        let v2 = validator_id("carol");

        let idx1 = pool
            .start_session(v1.clone(), s.clone(), "bounty-1".to_string())
            .unwrap();
        let idx2 = pool
            .start_session(v2.clone(), s.clone(), "bounty-1".to_string())
            .unwrap();

        // Complete session 1: flaw upheld
        let session1 = pool.get_mut(idx1).unwrap();
        let challenge1 = make_challenge(&v1, &s, "bounty-1", "buffer overflow", 0.85);
        session1.submit_challenge(challenge1).unwrap();
        session1.submit_defense("defense".to_string()).unwrap();
        session1.adjudicate(true).unwrap();

        // Complete session 2: flaw dismissed
        let session2 = pool.get_mut(idx2).unwrap();
        let challenge2 = make_challenge(&v2, &s, "bounty-1", "race condition", 0.7);
        session2.submit_challenge(challenge2).unwrap();
        session2.submit_defense("no race".to_string()).unwrap();
        session2.adjudicate(false).unwrap();

        assert!(pool.all_complete());

        let verdicts = pool.verdicts();
        assert_eq!(verdicts.len(), 2);
        assert_eq!(verdicts[0], (v1, ValidationVerdict::FlawUpheld));
        assert_eq!(verdicts[1], (v2, ValidationVerdict::FlawDismissed));
    }

    #[test]
    fn test_all_complete() {
        let mut pool = ValidationPool::new();
        let s = solver_id("bob");
        let v1 = validator_id("alice");
        let v2 = validator_id("carol");

        pool.start_session(v1.clone(), s.clone(), "bounty-1".to_string())
            .unwrap();
        pool.start_session(v2, s.clone(), "bounty-1".to_string())
            .unwrap();

        // Only complete session 0
        let session = pool.get_mut(0).unwrap();
        let challenge = make_challenge(&v1, &s, "bounty-1", "", 0.97);
        session.submit_challenge(challenge).unwrap();

        // Session 1 still at Challenge round
        assert!(!pool.all_complete());
    }

    #[test]
    fn test_round_ordering() {
        let v = validator_id("alice");
        let s = solver_id("bob");
        let mut session = ValidationSession::new(v, s, "bounty-1".to_string()).unwrap();

        // Can't submit defense before challenge
        let result = session.submit_defense("premature defense".to_string());
        assert!(result.is_err());

        // Can't adjudicate before defense
        let result = session.adjudicate(true);
        assert!(result.is_err());
    }

    #[test]
    fn test_flaws_found_count() {
        let mut pool = ValidationPool::new();
        let s = solver_id("bob");

        let validators: Vec<AgentId> = (0..3).map(|i| validator_id(&format!("v{i}"))).collect();

        for v in &validators {
            pool.start_session(v.clone(), s.clone(), "bounty-1".to_string())
                .unwrap();
        }

        // Sessions 0 and 1: flaw upheld
        for idx in 0..2 {
            let session = pool.get_mut(idx).unwrap();
            let challenge = make_challenge(&validators[idx], &s, "bounty-1", "some flaw", 0.8);
            session.submit_challenge(challenge).unwrap();
            session.submit_defense("defense".to_string()).unwrap();
            session.adjudicate(true).unwrap();
        }

        // Session 2: flaw dismissed
        let session = pool.get_mut(2).unwrap();
        let challenge = make_challenge(&validators[2], &s, "bounty-1", "weak flaw", 0.5);
        session.submit_challenge(challenge).unwrap();
        session.submit_defense("defense".to_string()).unwrap();
        session.adjudicate(false).unwrap();

        assert!(pool.all_complete());
        assert_eq!(pool.flaws_found(), 2);
        assert_eq!(pool.early_terminations(), 0);
        assert_eq!(pool.session_count(), 3);
    }
}

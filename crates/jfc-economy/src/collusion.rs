//! Anti-collusion enforcement — detects rubber-stamping and griefing patterns.

use std::collections::HashMap;

use crate::types::{AgentId, ValidationVerdict};

/// Per-agent validation statistics.
#[derive(Debug, Clone)]
pub struct AgentStats {
    pub total_validations: u32,
    pub approvals: u32,
    pub rejections: u32,
    pub dismissals: u32,
}

impl AgentStats {
    pub fn new() -> Self {
        Self {
            total_validations: 0,
            approvals: 0,
            rejections: 0,
            dismissals: 0,
        }
    }

    pub fn approval_rate(&self) -> f32 {
        if self.total_validations == 0 {
            return 0.5;
        }
        self.approvals as f32 / self.total_validations as f32
    }

    pub fn rejection_rate(&self) -> f32 {
        if self.total_validations == 0 {
            return 0.0;
        }
        self.rejections as f32 / self.total_validations as f32
    }
}

impl Default for AgentStats {
    fn default() -> Self {
        Self::new()
    }
}

/// Detects rubber-stamping (>90% approval) and griefing (>80% rejection).
///
/// Detection only activates after `min_samples` validations to avoid
/// false positives on small sample sizes.
pub struct CollusionDetector {
    stats: HashMap<AgentId, AgentStats>,
    rubber_stamp_threshold: f32,
    griefing_threshold: f32,
    min_samples: u32,
}

impl CollusionDetector {
    pub fn new() -> Self {
        Self {
            stats: HashMap::new(),
            rubber_stamp_threshold: 0.9,
            griefing_threshold: 0.8,
            min_samples: 5,
        }
    }

    /// Record a validation outcome for an agent.
    pub fn record(&mut self, validator_id: &AgentId, verdict: ValidationVerdict) {
        let stats = self.stats.entry(validator_id.clone()).or_default();
        stats.total_validations += 1;
        match verdict {
            ValidationVerdict::NoFlawFound | ValidationVerdict::EarlyTermination => {
                stats.approvals += 1;
            }
            ValidationVerdict::FlawUpheld => stats.rejections += 1,
            ValidationVerdict::FlawDismissed => stats.dismissals += 1,
        }
    }

    /// Check if agent is rubber-stamping (always approving without finding flaws).
    pub fn is_rubber_stamping(&self, agent_id: &AgentId) -> bool {
        self.stats
            .get(agent_id)
            .map(|s| {
                s.total_validations >= self.min_samples
                    && s.approval_rate() > self.rubber_stamp_threshold
            })
            .unwrap_or(false)
    }

    /// Check if agent is griefing (always rejecting / claiming flaws).
    pub fn is_griefing(&self, agent_id: &AgentId) -> bool {
        self.stats
            .get(agent_id)
            .map(|s| {
                s.total_validations >= self.min_samples
                    && s.rejection_rate() > self.griefing_threshold
            })
            .unwrap_or(false)
    }

    /// Get all flagged agents with their violation type.
    pub fn flagged_agents(&self) -> Vec<(&AgentId, &str)> {
        let mut flagged = Vec::new();
        for (id, stats) in &self.stats {
            if stats.total_validations >= self.min_samples {
                if stats.approval_rate() > self.rubber_stamp_threshold {
                    flagged.push((id, "rubber-stamping"));
                }
                if stats.rejection_rate() > self.griefing_threshold {
                    flagged.push((id, "griefing"));
                }
            }
        }
        flagged
    }

    /// Get stats for an agent.
    pub fn get_stats(&self, agent_id: &AgentId) -> Option<&AgentStats> {
        self.stats.get(agent_id)
    }
}

impl Default for CollusionDetector {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn agent(name: &str) -> AgentId {
        AgentId(name.to_string())
    }

    #[test]
    fn test_rubber_stamping_detection() {
        let mut detector = CollusionDetector::new();
        let id = agent("rubber_stamper");

        for _ in 0..6 {
            detector.record(&id, ValidationVerdict::NoFlawFound);
        }

        assert!(detector.is_rubber_stamping(&id));
        assert!(!detector.is_griefing(&id));
    }

    #[test]
    fn test_griefing_detection() {
        let mut detector = CollusionDetector::new();
        let id = agent("griefer");

        for _ in 0..5 {
            detector.record(&id, ValidationVerdict::FlawUpheld);
        }
        detector.record(&id, ValidationVerdict::NoFlawFound);

        assert!(detector.is_griefing(&id));
        assert!(!detector.is_rubber_stamping(&id));
    }

    #[test]
    fn test_below_min_samples() {
        let mut detector = CollusionDetector::new();
        let id = agent("new_validator");

        for _ in 0..3 {
            detector.record(&id, ValidationVerdict::NoFlawFound);
        }

        assert!(!detector.is_rubber_stamping(&id));
        assert!(!detector.is_griefing(&id));
    }

    #[test]
    fn test_normal_behavior() {
        let mut detector = CollusionDetector::new();
        let id = agent("honest_validator");

        detector.record(&id, ValidationVerdict::NoFlawFound);
        detector.record(&id, ValidationVerdict::FlawUpheld);
        detector.record(&id, ValidationVerdict::FlawDismissed);
        detector.record(&id, ValidationVerdict::NoFlawFound);
        detector.record(&id, ValidationVerdict::FlawUpheld);
        detector.record(&id, ValidationVerdict::EarlyTermination);

        assert!(!detector.is_rubber_stamping(&id));
        assert!(!detector.is_griefing(&id));
        assert!(detector.flagged_agents().is_empty());
    }
}

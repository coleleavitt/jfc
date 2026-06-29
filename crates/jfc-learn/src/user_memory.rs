//! User Memory Pipeline — observe, record, promote user behavior patterns.
//!
//! Observations are appended to a JSONL file. When a facet appears across ≥3 distinct
//! sessions it gets promoted into a UserProfileEntry.

use std::collections::{HashMap, HashSet};
use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::LearnError;

// ─── Types ──────────────────────────────────────────────────────────────────

/// An observed user behavior pattern.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserObservation {
    /// Facet: communication_style, expertise_level, review_focus, working_patterns
    pub facet: String,
    /// The observation text.
    pub observation: String,
    /// Which turns provide evidence.
    pub evidence_turns: Vec<usize>,
    /// Session identifier.
    pub session_id: String,
    /// When observed (unix timestamp ms).
    pub observed_at: u64,
}

/// The user profile — a collection of promoted entries.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserProfile {
    pub entries: Vec<UserProfileEntry>,
}

/// A promoted user profile entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserProfileEntry {
    pub facet: String,
    pub summary: String,
    pub evidence_session_count: u32,
    pub confidence: f32,
    pub last_promoted: u64,
    pub status: String, // "active" | "archived"
}

// ─── Pipeline ───────────────────────────────────────────────────────────────

/// The user memory pipeline handles observation recording and profile promotion.
pub struct UserMemoryPipeline {
    pub root: PathBuf,
}

impl UserMemoryPipeline {
    /// Create a new pipeline rooted at the given project directory.
    pub fn new(project_root: &Path) -> Self {
        Self {
            root: project_root.join(".jfc").join("profile"),
        }
    }

    /// Record an observation — append to candidates.jsonl.
    pub fn record_observation(&self, obs: &UserObservation) -> Result<(), LearnError> {
        fs::create_dir_all(&self.root)?;
        let path = self.root.join("candidates.jsonl");
        let mut file = OpenOptions::new().create(true).append(true).open(&path)?;
        let line = serde_json::to_string(obs)?;
        writeln!(file, "{}", line)?;
        Ok(())
    }

    /// Load all candidate observations from the JSONL file.
    pub fn load_candidates(project_root: &Path) -> Result<Vec<UserObservation>, LearnError> {
        let path = project_root
            .join(".jfc")
            .join("profile")
            .join("candidates.jsonl");
        if !path.exists() {
            return Ok(Vec::new());
        }
        let file = fs::File::open(&path)?;
        let reader = BufReader::new(file);
        let mut observations = Vec::new();
        for line in reader.lines() {
            let line = line?;
            if line.trim().is_empty() {
                continue;
            }
            match serde_json::from_str::<UserObservation>(&line) {
                Ok(obs) => observations.push(obs),
                Err(e) => {
                    tracing::warn!(error = %e, "skipping malformed observation line");
                }
            }
        }
        Ok(observations)
    }

    /// Check which facets are ready for promotion (appear in ≥3 distinct sessions).
    pub fn check_promotion(candidates: &[UserObservation]) -> Vec<UserProfileEntry> {
        // Group by facet → distinct session ids
        let mut facet_sessions: HashMap<&str, HashSet<&str>> = HashMap::new();
        let mut facet_observations: HashMap<&str, Vec<&UserObservation>> = HashMap::new();

        for obs in candidates {
            facet_sessions
                .entry(&obs.facet)
                .or_default()
                .insert(&obs.session_id);
            facet_observations.entry(&obs.facet).or_default().push(obs);
        }

        let mut promoted = Vec::new();
        for (facet, sessions) in &facet_sessions {
            if sessions.len() >= 3 {
                // Pick the most recent observation as the summary
                let observations = &facet_observations[facet];
                let latest = observations.iter().max_by_key(|o| o.observed_at).unwrap();

                promoted.push(UserProfileEntry {
                    facet: facet.to_string(),
                    summary: latest.observation.clone(),
                    evidence_session_count: sessions.len() as u32,
                    confidence: (sessions.len() as f32 / 5.0).min(1.0),
                    last_promoted: latest.observed_at,
                    status: "active".to_string(),
                });
            }
        }

        promoted
    }

    /// Render a `<user-profile>` XML block from promoted entries.
    pub fn render_profile_block(entries: &[UserProfileEntry]) -> String {
        if entries.is_empty() {
            return String::new();
        }

        let mut out = String::from("<user-profile>\n");
        for entry in entries {
            out.push_str(&format!(
                "  <facet name=\"{}\" confidence=\"{:.2}\" sessions=\"{}\">\n    {}\n  </facet>\n",
                entry.facet, entry.confidence, entry.evidence_session_count, entry.summary
            ));
        }
        out.push_str("</user-profile>");
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn record_and_load_candidates_normal() {
        let tmp = TempDir::new().unwrap();
        let pipeline = UserMemoryPipeline::new(tmp.path());

        let obs = UserObservation {
            facet: "communication_style".to_string(),
            observation: "Prefers concise responses".to_string(),
            evidence_turns: vec![1, 3],
            session_id: "session-001".to_string(),
            observed_at: 1700000000,
        };

        pipeline.record_observation(&obs).unwrap();

        let loaded = UserMemoryPipeline::load_candidates(tmp.path()).unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].facet, "communication_style");
        assert_eq!(loaded[0].observation, "Prefers concise responses");
    }

    #[test]
    fn promotion_requires_three_sessions_normal() {
        let candidates = vec![
            UserObservation {
                facet: "expertise_level".to_string(),
                observation: "Expert Rust developer".to_string(),
                evidence_turns: vec![0],
                session_id: "s1".to_string(),
                observed_at: 100,
            },
            UserObservation {
                facet: "expertise_level".to_string(),
                observation: "Expert Rust developer".to_string(),
                evidence_turns: vec![0],
                session_id: "s2".to_string(),
                observed_at: 200,
            },
        ];

        // Only 2 sessions — not enough
        let promoted = UserMemoryPipeline::check_promotion(&candidates);
        assert!(promoted.is_empty());

        // Add a third session
        let mut candidates3 = candidates;
        candidates3.push(UserObservation {
            facet: "expertise_level".to_string(),
            observation: "Senior Rust developer".to_string(),
            evidence_turns: vec![1],
            session_id: "s3".to_string(),
            observed_at: 300,
        });

        let promoted = UserMemoryPipeline::check_promotion(&candidates3);
        assert_eq!(promoted.len(), 1);
        assert_eq!(promoted[0].facet, "expertise_level");
        assert_eq!(promoted[0].evidence_session_count, 3);
    }

    #[test]
    fn render_profile_block_format_normal() {
        let entries = vec![UserProfileEntry {
            facet: "communication_style".to_string(),
            summary: "Prefers terse, code-first responses".to_string(),
            evidence_session_count: 4,
            confidence: 0.8,
            last_promoted: 1700000000,
            status: "active".to_string(),
        }];

        let rendered = UserMemoryPipeline::render_profile_block(&entries);
        assert!(rendered.starts_with("<user-profile>"));
        assert!(rendered.ends_with("</user-profile>"));
        assert!(rendered.contains("communication_style"));
        assert!(rendered.contains("Prefers terse, code-first responses"));
        assert!(rendered.contains("sessions=\"4\""));
    }
}

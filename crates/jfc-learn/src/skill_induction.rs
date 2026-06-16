//! Skill induction — mine repeated, successful tool-sequences from completed
//! session transcripts and emit scored *proposals* for new skills.
//!
//! This is the read/analysis half of the skill-from-experience loop; the write
//! half is [`jfc_agents::registry::write_agent_skill`]. Per the project's
//! self-learning design (mirroring the literature on skill induction / library
//! learning — e.g. Voyager-style skill libraries, RL-with-skill-library work),
//! the inductor never *silently installs* a skill: it surfaces a ranked
//! [`SkillProposal`] that a dreamer task (or the user) confirms before writing.
//!
//! ## What it mines
//!
//! Each completed session contributes one [`SessionTrace`] — the ordered list
//! of tool invocations the assistant made, each tagged success/failure. The
//! inductor extracts contiguous **n-gram tool-sequences** (length
//! [`MIN_SEQ_LEN`]..=[`MAX_SEQ_LEN`]) that:
//!
//! - recur across **distinct sessions** (a one-off is not a skill), and
//! - are **predominantly successful** (a flaky sequence is not worth crystallising).
//!
//! Each surviving sequence is scored by support (how many sessions) × length ×
//! success-rate, and the top sequences become proposals.
//!
//! The logic here is pure and deterministic so it is unit-testable without any
//! LLM, filesystem, or session-store dependency — callers feed in
//! [`SessionTrace`]s and get [`SkillProposal`]s back.

use std::collections::HashMap;

/// Shortest tool-sequence worth proposing as a skill. A single tool is just a
/// tool; two is the smallest "procedure".
pub const MIN_SEQ_LEN: usize = 2;

/// Longest contiguous sequence the inductor will crystallise. Beyond this the
/// sequence is usually task-specific rather than a reusable procedure.
pub const MAX_SEQ_LEN: usize = 6;

/// One tool invocation within a session, as seen by the inductor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolStep {
    /// Canonical tool name (e.g. "Read", "Edit", "Bash").
    pub tool: String,
    /// Whether the invocation succeeded. Failed steps still appear in the
    /// sequence (they're part of the observed procedure) but drag the
    /// sequence's success-rate down so flaky procedures don't get proposed.
    pub success: bool,
}

impl ToolStep {
    pub fn new(tool: impl Into<String>, success: bool) -> Self {
        Self {
            tool: tool.into(),
            success,
        }
    }
}

/// The tool trace for one completed session.
#[derive(Debug, Clone)]
pub struct SessionTrace {
    /// Stable id of the session this trace came from — used to count *distinct*
    /// sessions supporting a sequence (so a loop within one session doesn't
    /// inflate support).
    pub session_id: String,
    /// The ordered tool steps the assistant took.
    pub steps: Vec<ToolStep>,
}

impl SessionTrace {
    pub fn new(session_id: impl Into<String>, steps: Vec<ToolStep>) -> Self {
        Self {
            session_id: session_id.into(),
            steps,
        }
    }
}

/// Tuning for the inductor. Defaults are conservative — a sequence must recur
/// in at least 3 distinct sessions and be ≥80% successful, mirroring the
/// user-memory promotion threshold (facts promoted after ≥3 distinct sessions).
#[derive(Debug, Clone)]
pub struct InductionConfig {
    /// Minimum number of *distinct sessions* a sequence must appear in.
    pub min_support: usize,
    /// Minimum success-rate (0.0..=1.0) across all observed occurrences.
    pub min_success_rate: f64,
    /// Cap on the number of proposals returned (highest-scored first).
    pub max_proposals: usize,
}

impl Default for InductionConfig {
    fn default() -> Self {
        Self {
            min_support: 3,
            min_success_rate: 0.8,
            max_proposals: 10,
        }
    }
}

/// A scored, human-confirmable proposal for a new skill distilled from
/// recurring tool-sequences. Carries everything a dreamer task needs to either
/// surface it to the user or hand it to `write_agent_skill`.
#[derive(Debug, Clone, PartialEq)]
pub struct SkillProposal {
    /// The recurring tool sequence, e.g. ["Grep", "Read", "Edit"].
    pub sequence: Vec<String>,
    /// Number of distinct sessions the sequence appeared in (its support).
    pub support: usize,
    /// Total occurrences across all sessions (≥ support; a session can repeat it).
    pub occurrences: usize,
    /// Success-rate across all occurrences (0.0..=1.0).
    pub success_rate: f64,
    /// Composite rank — higher is a stronger candidate.
    pub score: f64,
    /// A kebab-case slug suggestion derived from the sequence.
    pub suggested_name: String,
}

impl SkillProposal {
    /// A one-line description suitable for the proposed skill's frontmatter.
    pub fn suggested_description(&self) -> String {
        format!(
            "Recurring procedure: {} (seen in {} sessions, {:.0}% successful).",
            self.sequence.join(" → "),
            self.support,
            self.success_rate * 100.0
        )
    }
}

/// A contiguous occurrence of a tool-sequence keyed for aggregation.
struct SeqStats {
    /// Distinct session ids that contain this sequence.
    sessions: std::collections::HashSet<String>,
    /// Total occurrences (across all sessions).
    occurrences: usize,
    /// Occurrences where every step in the window succeeded.
    fully_successful: usize,
}

impl SeqStats {
    fn new() -> Self {
        Self {
            sessions: std::collections::HashSet::new(),
            occurrences: 0,
            fully_successful: 0,
        }
    }
}

/// Mine `traces` for recurring, predominantly-successful tool-sequences and
/// return ranked [`SkillProposal`]s. Pure and deterministic: same input →
/// same output (proposals sorted by score desc, then sequence for stability).
pub fn induce_skills(traces: &[SessionTrace], config: &InductionConfig) -> Vec<SkillProposal> {
    // Aggregate every contiguous n-gram (MIN_SEQ_LEN..=MAX_SEQ_LEN) across all
    // sessions. Key is the joined tool names; value accumulates support stats.
    let mut stats: HashMap<Vec<String>, SeqStats> = HashMap::new();

    for trace in traces {
        let tools: Vec<&str> = trace.steps.iter().map(|s| s.tool.as_str()).collect();
        let n = tools.len();
        for len in MIN_SEQ_LEN..=MAX_SEQ_LEN {
            if len > n {
                break;
            }
            for start in 0..=(n - len) {
                let window = &trace.steps[start..start + len];
                let key: Vec<String> = window.iter().map(|s| s.tool.clone()).collect();
                let entry = stats.entry(key).or_insert_with(SeqStats::new);
                entry.sessions.insert(trace.session_id.clone());
                entry.occurrences += 1;
                if window.iter().all(|s| s.success) {
                    entry.fully_successful += 1;
                }
            }
        }
    }

    let mut proposals: Vec<SkillProposal> = stats
        .into_iter()
        .filter_map(|(sequence, st)| {
            let support = st.sessions.len();
            if support < config.min_support {
                return None;
            }
            let success_rate = if st.occurrences == 0 {
                0.0
            } else {
                st.fully_successful as f64 / st.occurrences as f64
            };
            if success_rate < config.min_success_rate {
                return None;
            }
            let score = score_sequence(support, sequence.len(), success_rate);
            let suggested_name = suggest_name(&sequence);
            Some(SkillProposal {
                suggested_name,
                support,
                occurrences: st.occurrences,
                success_rate,
                score,
                sequence,
            })
        })
        .collect();

    // Sort by score desc; break ties by sequence for deterministic output.
    proposals.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.sequence.cmp(&b.sequence))
    });

    // Drop sequences fully contained in a higher-scored proposal's sequence —
    // if "Grep→Read→Edit" is proposed, the sub-sequence "Grep→Read" is
    // redundant noise. Keeps the surfaced set focused on maximal procedures.
    let mut kept: Vec<SkillProposal> = Vec::new();
    for p in proposals {
        let subsumed = kept
            .iter()
            .any(|k| is_contiguous_subsequence(&p.sequence, &k.sequence));
        if !subsumed {
            kept.push(p);
        }
        if kept.len() >= config.max_proposals {
            break;
        }
    }
    kept
}

/// Composite score: more distinct sessions (support) is the strongest signal,
/// longer sequences encode more procedure, and success-rate gates quality.
/// Weighted so support dominates, then length, then success-rate.
fn score_sequence(support: usize, len: usize, success_rate: f64) -> f64 {
    (support as f64) * 10.0 + (len as f64) * 2.0 + success_rate * 5.0
}

/// True if `needle` appears as a contiguous run inside `haystack`.
fn is_contiguous_subsequence(needle: &[String], haystack: &[String]) -> bool {
    if needle.is_empty() || needle.len() > haystack.len() {
        return false;
    }
    haystack.windows(needle.len()).any(|w| w == needle)
}

/// Derive a kebab-case skill-name suggestion from a tool sequence, e.g.
/// ["Grep", "Read", "Edit"] → "grep-read-edit-flow".
fn suggest_name(sequence: &[String]) -> String {
    let joined = sequence
        .iter()
        .map(|s| s.to_ascii_lowercase())
        .collect::<Vec<_>>()
        .join("-");
    format!("{joined}-flow")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn step(tool: &str, ok: bool) -> ToolStep {
        ToolStep::new(tool, ok)
    }

    fn ok_seq(session: &str, tools: &[&str]) -> SessionTrace {
        SessionTrace::new(
            session,
            tools.iter().map(|t| step(t, true)).collect::<Vec<_>>(),
        )
    }

    // Normal: a sequence recurring across the support threshold of distinct
    // sessions is proposed, with correct support + 100% success.
    #[test]
    fn induces_recurring_successful_sequence_normal() {
        let traces = vec![
            ok_seq("s1", &["Grep", "Read", "Edit"]),
            ok_seq("s2", &["Grep", "Read", "Edit"]),
            ok_seq("s3", &["Grep", "Read", "Edit"]),
        ];
        let proposals = induce_skills(&traces, &InductionConfig::default());
        assert!(!proposals.is_empty(), "expected at least one proposal");
        let top = &proposals[0];
        assert_eq!(top.sequence, vec!["Grep", "Read", "Edit"]);
        assert_eq!(top.support, 3);
        assert!((top.success_rate - 1.0).abs() < f64::EPSILON);
        assert_eq!(top.suggested_name, "grep-read-edit-flow");
    }

    // Robust: a sequence seen in fewer than min_support distinct sessions is
    // NOT proposed (a one-off is not a skill).
    #[test]
    fn below_support_threshold_not_proposed_robust() {
        let traces = vec![
            ok_seq("s1", &["Bash", "Read"]),
            ok_seq("s2", &["Bash", "Read"]),
        ]; // support = 2 < default 3
        let proposals = induce_skills(&traces, &InductionConfig::default());
        assert!(proposals.is_empty(), "two sessions is below threshold");
    }

    // Robust: a flaky sequence (low success-rate) is filtered out even with
    // enough support.
    #[test]
    fn flaky_sequence_filtered_robust() {
        let traces = vec![
            SessionTrace::new("s1", vec![step("Bash", false), step("Read", true)]),
            SessionTrace::new("s2", vec![step("Bash", false), step("Read", true)]),
            SessionTrace::new("s3", vec![step("Bash", false), step("Read", true)]),
        ];
        // 0% fully-successful (Bash always fails) < 0.8 threshold.
        let proposals = induce_skills(&traces, &InductionConfig::default());
        assert!(
            proposals.iter().all(|p| p.sequence != vec!["Bash", "Read"]),
            "flaky Bash→Read should be filtered"
        );
    }

    // Robust: a shorter sub-sequence subsumed by a kept longer proposal is
    // dropped (no redundant Grep→Read alongside Grep→Read→Edit).
    #[test]
    fn subsumed_subsequence_dropped_robust() {
        let traces = vec![
            ok_seq("s1", &["Grep", "Read", "Edit"]),
            ok_seq("s2", &["Grep", "Read", "Edit"]),
            ok_seq("s3", &["Grep", "Read", "Edit"]),
        ];
        let proposals = induce_skills(&traces, &InductionConfig::default());
        // The 3-gram should be present; the 2-gram Grep→Read should be subsumed.
        assert!(proposals.iter().any(|p| p.sequence.len() == 3));
        assert!(
            !proposals.iter().any(|p| p.sequence == vec!["Grep", "Read"]),
            "subsumed 2-gram must be dropped"
        );
    }

    // Normal: distinct sessions counted for support, not raw repetition within
    // one session (a loop in a single session is still support = 1).
    #[test]
    fn support_counts_distinct_sessions_normal() {
        // One session repeats the sequence 3×; support must be 1, not 3.
        let traces = vec![SessionTrace::new(
            "s1",
            vec![
                step("Read", true),
                step("Edit", true),
                step("Read", true),
                step("Edit", true),
                step("Read", true),
                step("Edit", true),
            ],
        )];
        let proposals = induce_skills(&traces, &InductionConfig::default());
        // support = 1 < 3 → nothing proposed despite 3 in-session repetitions.
        assert!(
            proposals.is_empty(),
            "single session can't satisfy distinct-session support"
        );
    }

    // Robust: empty input yields no proposals and does not panic.
    #[test]
    fn empty_input_is_safe_robust() {
        let proposals = induce_skills(&[], &InductionConfig::default());
        assert!(proposals.is_empty());
    }

    /// Build `variants` distinct 2-step sequences, each supported by 3 distinct
    /// sessions, so every one clears the support+success thresholds.
    fn distinct_supported_traces(variants: usize) -> Vec<SessionTrace> {
        (0..variants)
            .flat_map(|v| {
                let a = format!("ToolA{v}");
                let b = format!("ToolB{v}");
                (0..3).map(move |s| {
                    SessionTrace::new(
                        format!("s{v}_{s}"),
                        vec![
                            ToolStep::new(a.clone(), true),
                            ToolStep::new(b.clone(), true),
                        ],
                    )
                })
            })
            .collect()
    }

    // Normal: max_proposals caps the returned set.
    #[test]
    fn respects_max_proposals_normal() {
        let traces = distinct_supported_traces(5);
        let config = InductionConfig {
            max_proposals: 2,
            ..InductionConfig::default()
        };
        let proposals = induce_skills(&traces, &config);
        assert!(proposals.len() <= 2);
    }

    // Normal: suggested_description reads as a human-confirmable summary.
    #[test]
    fn proposal_description_is_human_readable_normal() {
        let traces = vec![
            ok_seq("s1", &["Grep", "Read", "Edit"]),
            ok_seq("s2", &["Grep", "Read", "Edit"]),
            ok_seq("s3", &["Grep", "Read", "Edit"]),
        ];
        let proposals = induce_skills(&traces, &InductionConfig::default());
        let desc = proposals[0].suggested_description();
        assert!(desc.contains("Grep → Read → Edit"));
        assert!(desc.contains("3 sessions"));
        assert!(desc.contains("100% successful"));
    }
}

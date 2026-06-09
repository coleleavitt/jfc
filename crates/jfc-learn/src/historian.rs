//! Historian — extracts factual knowledge from coding session transcripts.
//!
//! Uses an LLM provider trait to call the model with a structured extraction prompt,
//! then deduplicates facts against existing memory via normalized hashes.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::error::LearnError;
use crate::normalize_hash::normalize_and_hash;
use crate::verifier::{LlmVerifier, PromotionVerifier, VerifierVerdict};

// ─── Categories ─────────────────────────────────────────────────────────────

/// Known fact categories the historian extracts.
pub const CATEGORIES: &[&str] = &[
    "ARCHITECTURE_DECISIONS",
    "CONSTRAINTS",
    "CONFIG_DEFAULTS",
    "NAMING",
    "USER_PREFERENCES",
    "USER_DIRECTIVES",
    "ENVIRONMENT",
    "WORKFLOW_RULES",
    "KNOWN_ISSUES",
];

// ─── Types ──────────────────────────────────────────────────────────────────

/// A candidate fact extracted from a transcript.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CandidateFact {
    pub category: String,
    pub content: String,
    pub turn_ordinal: usize,
    pub confidence: f32,
}

/// Configuration for the Historian.
#[derive(Debug, Clone)]
pub struct HistorianConfig {
    /// Minimum confidence threshold for promotion (default 0.7).
    pub min_confidence: f32,
}

impl Default for HistorianConfig {
    fn default() -> Self {
        Self {
            min_confidence: 0.7,
        }
    }
}

/// Why an extraction run produced nothing before calling the model. Mirrors
/// Claude Code 2.1.170's `tengu_extract_memories_skipped_*` events — the
/// historian shouldn't burn an LLM extraction pass on a transcript that has
/// nothing worth remembering (tool-only turns, or content already persisted).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkipReason {
    /// The transcript has no substantive assistant *prose* — only tool calls /
    /// tool results / boilerplate. (`skipped_no_prose`)
    NoProse,
    /// The transcript is empty or below the minimum size to bother. (`skipped_empty`)
    TooSmall,
}

impl SkipReason {
    pub fn label(self) -> &'static str {
        match self {
            SkipReason::NoProse => "skipped_no_prose",
            SkipReason::TooSmall => "skipped_too_small",
        }
    }
}

/// Result of a historian extraction run.
#[derive(Debug, Clone, Default)]
pub struct HistorianReport {
    pub facts_extracted: usize,
    pub facts_promoted: usize,
    pub facts_deduped: usize,
    /// Number of facts rejected by the [`PromotionVerifier`] and routed to the
    /// quarantine file. Always `0` when `process_session_with_verifier` is not
    /// used.
    pub facts_quarantined: usize,
    /// Per-fact outcome — populated by `process` (not `run`, which only
    /// reports counts for backwards compatibility with v0.1.0 callers).
    pub processed: Vec<ProcessedFact>,
    /// `Some` when extraction was short-circuited *before* the LLM call by the
    /// pre-extraction gate. All count fields are `0` in that case.
    pub skipped: Option<SkipReason>,
}

/// One extracted fact tagged with its dedup outcome.
#[derive(Debug, Clone)]
pub struct ProcessedFact {
    pub fact: CandidateFact,
    /// Normalized SHA256 of `fact.content` — used as the memory key.
    pub normalized_hash: String,
    /// `true` when an existing memory with this hash was found and
    /// should have its `seen_count` incremented; `false` when this is
    /// a new fact that should be written.
    pub deduped: bool,
}

/// Persisted quarantine entry — one JSONL line per rejected fact.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuarantineRecord {
    pub fact: CandidateFact,
    pub normalized_hash: String,
    pub verdict: VerifierVerdict,
    pub quarantined_at_ms: u64,
}

/// Trait for LLM provider — the historian calls this to get model output.
pub trait HistorianProvider {
    /// Send a system prompt + user message, expecting JSON tool-call output.
    fn extract_facts(&self, system_prompt: &str, user_message: &str) -> Result<String, LearnError>;
}

/// Trait for checking if a fact (by normalized hash) already exists in memory.
pub trait MemoryLookup {
    /// Returns true if a memory with this normalized_hash already exists.
    fn hash_exists(&self, hash: &str) -> bool;
}

/// The Historian agent.
pub struct Historian<P: HistorianProvider, M: MemoryLookup> {
    pub config: HistorianConfig,
    pub provider: P,
    pub memory: M,
}

/// System prompt for the historian extraction.
pub const HISTORIAN_SYSTEM_PROMPT: &str = r#"You are a memory extraction agent. Extract factual knowledge from this coding session transcript.

Categories: ARCHITECTURE_DECISIONS, CONSTRAINTS, CONFIG_DEFAULTS, NAMING, USER_PREFERENCES, USER_DIRECTIVES, ENVIRONMENT, WORKFLOW_RULES, KNOWN_ISSUES

Rules:
- One fact per category per turn maximum
- Present-tense operational language ("X uses Y", not "we switched X")
- Drop session-local context unless the commit hash is the point
- Each fact must be atomic (one rule/fact per entry)
- Confidence 0.0-1.0 based on how clearly stated the fact is

Output: call extract_facts tool with your findings."#;

/// Tool schema for forced output.
pub const EXTRACT_FACTS_SCHEMA: &str = r#"{"name":"extract_facts","parameters":{"type":"object","properties":{"facts":{"type":"array","items":{"type":"object","properties":{"category":{"type":"string"},"content":{"type":"string"},"turn_ordinal":{"type":"integer"},"confidence":{"type":"number"}},"required":["category","content","turn_ordinal","confidence"]}}}}}"#;

impl<P: HistorianProvider, M: MemoryLookup> Historian<P, M> {
    pub fn new(provider: P, memory: M, config: HistorianConfig) -> Self {
        Self {
            config,
            provider,
            memory,
        }
    }

    /// Build a user message from a transcript (vec of (role, content) tuples).
    pub fn build_transcript_message(transcript: &[(String, String)]) -> String {
        let mut msg = String::from("<transcript>\n");
        for (i, (role, content)) in transcript.iter().enumerate() {
            msg.push_str(&format!(
                "<turn ordinal=\"{}\" role=\"{}\">\n{}\n</turn>\n",
                i, role, content
            ));
        }
        msg.push_str("</transcript>");
        msg
    }

    /// Pre-extraction gate. Returns `Some(reason)` when the transcript isn't
    /// worth an LLM extraction pass. Mirrors CC 2.1.170's `skipped_no_prose` /
    /// size checks: extraction only earns its cost when the assistant actually
    /// said something durable (prose), not when the turn was pure tool traffic.
    ///
    /// "Prose" = an `assistant`/`user` turn with enough non-tool natural-language
    /// text. Tool-call/tool-result turns (role `tool`, or JSON-shaped bodies) and
    /// trivially short turns don't count.
    pub fn skip_reason(transcript: &[(String, String)]) -> Option<SkipReason> {
        /// Minimum total prose chars across the transcript to bother extracting.
        /// Deliberately low: one real instruction/explanation is worth a pass;
        /// the gate's job is to drop *tool-only* turns and empty/one-word noise,
        /// not to second-guess short-but-substantive statements.
        const MIN_PROSE_CHARS: usize = 12;

        if transcript.is_empty() {
            return Some(SkipReason::TooSmall);
        }

        let prose_chars: usize = transcript
            .iter()
            .filter(|(role, _)| is_prose_role(role))
            .map(|(_, body)| prose_len(body))
            .sum();

        if prose_chars == 0 {
            return Some(SkipReason::NoProse);
        }
        if prose_chars < MIN_PROSE_CHARS {
            return Some(SkipReason::TooSmall);
        }
        None
    }

    /// Run the extraction pipeline.
    pub fn run(&self, transcript: &[(String, String)]) -> Result<HistorianReport, LearnError> {
        self.process(transcript)
    }

    /// Run the extraction pipeline AND return the per-fact decisions so the
    /// caller can persist new memories and increment `seen_count` on dedup
    /// hits. The legacy `run` method delegates here; the only behavioral
    /// difference is that `processed` is empty when called via `run` on
    /// pre-existing callers (kept that way to avoid breaking the v0.1.0
    /// HistorianReport API contract — `run` still returns the same counts).
    pub fn process(&self, transcript: &[(String, String)]) -> Result<HistorianReport, LearnError> {
        // Pre-extraction gate (CC 2.1.170 parity): don't spend an LLM extraction
        // pass on a transcript with nothing to remember — a turn that's only tool
        // calls/results, or too small to carry a durable fact. Returns an empty
        // report tagged with the skip reason instead of calling the model.
        if let Some(reason) = Self::skip_reason(transcript) {
            tracing::debug!(
                target: "jfc::historian",
                reason = reason.label(),
                turns = transcript.len(),
                "extract_memories skipped before model call"
            );
            return Ok(HistorianReport {
                skipped: Some(reason),
                ..Default::default()
            });
        }

        let user_message = Self::build_transcript_message(transcript);

        let raw_response = self
            .provider
            .extract_facts(HISTORIAN_SYSTEM_PROMPT, &user_message)?;

        let facts = self.parse_response(&raw_response)?;

        let facts_extracted = facts.len();
        let mut facts_promoted = 0;
        let mut facts_deduped = 0;
        let mut processed = Vec::with_capacity(facts.len());

        for fact in facts {
            if fact.confidence < self.config.min_confidence {
                continue;
            }

            let hash = normalize_and_hash(&fact.content);
            let deduped = self.memory.hash_exists(&hash);
            if deduped {
                facts_deduped += 1;
            } else {
                facts_promoted += 1;
            }
            processed.push(ProcessedFact {
                fact,
                normalized_hash: hash,
                deduped,
            });
        }

        Ok(HistorianReport {
            facts_extracted,
            facts_promoted,
            facts_deduped,
            facts_quarantined: 0,
            processed,
            skipped: None,
        })
    }

    /// Convenience wrapper: take ChatMessage-like `(role, text)` pairs as
    /// borrowed references and delegate to `process`. Equivalent to `process`
    /// — exposed under the name the integration spec uses.
    pub fn process_session(
        &self,
        messages: &[(String, String)],
    ) -> Result<HistorianReport, LearnError> {
        self.process(messages)
    }

    /// Verifier-gated variant of [`process_session`](Self::process_session).
    ///
    /// For each candidate fact that survives confidence + dedup filtering, the
    /// supplied [`PromotionVerifier`] is consulted via
    /// [`PromotionVerifier::verify_for_promotion`]. Facts that are *not*
    /// `Confirm`-ed are excluded from the returned `processed` list (so they
    /// will never be written to main memory by the caller) and instead
    /// appended as JSONL records to `quarantine_path`. Each quarantine record
    /// includes the original fact, its normalized hash, and the verdict.
    ///
    /// Dedup hits skip verification — they're already established facts.
    pub fn process_session_with_verifier(
        &self,
        messages: &[(String, String)],
        verifier: &PromotionVerifier,
        llm: &dyn LlmVerifier,
        quarantine_path: &Path,
    ) -> Result<HistorianReport, LearnError> {
        // First run the normal extraction pipeline. This already handles
        // confidence filtering and dedup against existing memory.
        let mut report = self.process(messages)?;

        // Partition the processed facts: dedup hits and confirmed promotions
        // stay; verifier-rejected new facts are written to quarantine and
        // dropped from the report so they won't be persisted to memory.
        let mut kept: Vec<ProcessedFact> = Vec::with_capacity(report.processed.len());
        let mut quarantined_records: Vec<QuarantineRecord> = Vec::new();

        for pf in report.processed.drain(..) {
            if pf.deduped {
                // Already known — no verification needed.
                kept.push(pf);
                continue;
            }
            let verdict = verifier.verify_for_promotion(&pf.fact, llm);
            match verdict {
                VerifierVerdict::Confirm { .. } => kept.push(pf),
                VerifierVerdict::Refute { .. } | VerifierVerdict::Quarantine { .. } => {
                    // Adjust counters: this fact is no longer "promoted".
                    if report.facts_promoted > 0 {
                        report.facts_promoted -= 1;
                    }
                    report.facts_quarantined += 1;
                    quarantined_records.push(QuarantineRecord {
                        fact: pf.fact,
                        normalized_hash: pf.normalized_hash,
                        verdict,
                        quarantined_at_ms: now_ms(),
                    });
                }
            }
        }

        report.processed = kept;

        if !quarantined_records.is_empty() {
            append_quarantine_records(quarantine_path, &quarantined_records)?;
        }

        Ok(report)
    }

    /// Parse provider JSON response into CandidateFacts.
    fn parse_response(&self, raw: &str) -> Result<Vec<CandidateFact>, LearnError> {
        // Try to parse as a JSON object with a "facts" array
        #[derive(Deserialize)]
        struct ExtractFactsCall {
            facts: Vec<CandidateFact>,
        }

        // Try full object first
        if let Ok(call) = serde_json::from_str::<ExtractFactsCall>(raw) {
            return Ok(call.facts);
        }

        // Try just the array
        if let Ok(facts) = serde_json::from_str::<Vec<CandidateFact>>(raw) {
            return Ok(facts);
        }

        // Try extracting JSON from markdown code blocks
        if let Some(json_start) = raw.find('{')
            && let Some(json_end) = raw.rfind('}')
        {
            let json_slice = &raw[json_start..=json_end];
            if let Ok(call) = serde_json::from_str::<ExtractFactsCall>(json_slice) {
                return Ok(call.facts);
            }
        }

        Err(LearnError::Parse {
            message: format!(
                "Could not parse historian response as facts JSON: {}",
                &raw[..raw.len().min(200)]
            ),
        })
    }
}

// ─── Quarantine I/O ─────────────────────────────────────────────────────────

/// Append one JSONL line per quarantined record. Creates parent directories
/// Roles whose turns can carry durable prose. Tool-result turns (`tool`,
/// `tool_result`, `function`) never do — they're inputs to extract *from* the
/// surrounding reasoning, not facts themselves.
fn is_prose_role(role: &str) -> bool {
    matches!(role.to_ascii_lowercase().as_str(), "assistant" | "user")
}

/// Approximate the natural-language prose length of a turn body, discounting
/// tool-call/tool-result noise. A body that is a single JSON object/array (a
/// serialized tool call or result) contributes 0; otherwise we count trimmed
/// non-whitespace length. This is the `no_prose` heuristic: turns that are pure
/// machine payload don't move the needle.
fn prose_len(body: &str) -> usize {
    let t = body.trim();
    if t.is_empty() {
        return 0;
    }
    // Whole-body JSON payload → treat as non-prose (tool traffic).
    if (t.starts_with('{') && t.ends_with('}')) || (t.starts_with('[') && t.ends_with(']')) {
        if serde_json::from_str::<serde_json::Value>(t).is_ok() {
            return 0;
        }
    }
    t.len()
}

/// and the file itself if missing. Each line is a serialized
/// [`QuarantineRecord`].
fn append_quarantine_records(path: &Path, records: &[QuarantineRecord]) -> Result<(), LearnError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    for rec in records {
        let line = serde_json::to_string(rec)?;
        writeln!(file, "{}", line)?;
    }
    Ok(())
}

fn now_ms() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    struct MockProvider {
        response: String,
    }

    impl HistorianProvider for MockProvider {
        fn extract_facts(&self, _system: &str, _user: &str) -> Result<String, LearnError> {
            Ok(self.response.clone())
        }
    }

    struct MockMemory {
        existing_hashes: HashSet<String>,
    }

    impl MemoryLookup for MockMemory {
        fn hash_exists(&self, hash: &str) -> bool {
            self.existing_hashes.contains(hash)
        }
    }

    #[test]
    fn historian_extracts_facts_normal() {
        let response = r#"{"facts":[{"category":"ARCHITECTURE_DECISIONS","content":"The project uses serde for serialization","turn_ordinal":0,"confidence":0.9}]}"#;
        let provider = MockProvider {
            response: response.to_string(),
        };
        let memory = MockMemory {
            existing_hashes: HashSet::new(),
        };
        let historian = Historian::new(provider, memory, HistorianConfig::default());

        let transcript = vec![(
            "user".to_string(),
            "We use serde for serialization".to_string(),
        )];
        let report = historian.run(&transcript).unwrap();
        assert_eq!(report.facts_extracted, 1);
        assert_eq!(report.facts_promoted, 1);
        assert_eq!(report.facts_deduped, 0);
    }

    #[test]
    fn historian_filters_low_confidence_normal() {
        let response = r#"{"facts":[{"category":"NAMING","content":"Variables use snake_case","turn_ordinal":0,"confidence":0.3}]}"#;
        let provider = MockProvider {
            response: response.to_string(),
        };
        let memory = MockMemory {
            existing_hashes: HashSet::new(),
        };
        let historian = Historian::new(provider, memory, HistorianConfig::default());

        let transcript = vec![("user".to_string(), "Something about naming".to_string())];
        let report = historian.run(&transcript).unwrap();
        assert_eq!(report.facts_extracted, 1);
        assert_eq!(report.facts_promoted, 0);
        assert_eq!(report.facts_deduped, 0);
    }

    #[test]
    fn historian_dedup_by_hash_normal() {
        let content = "The project uses serde for serialization";
        let hash = normalize_and_hash(content);

        let response = r#"{"facts":[{"category":"ARCHITECTURE_DECISIONS","content":"The project uses serde for serialization","turn_ordinal":0,"confidence":0.95}]}"#;
        let provider = MockProvider {
            response: response.to_string(),
        };
        let mut existing = HashSet::new();
        existing.insert(hash);
        let memory = MockMemory {
            existing_hashes: existing,
        };
        let historian = Historian::new(provider, memory, HistorianConfig::default());

        let transcript = vec![(
            "user".to_string(),
            "We use serde for serialization".to_string(),
        )];
        let report = historian.run(&transcript).unwrap();
        assert_eq!(report.facts_extracted, 1);
        assert_eq!(report.facts_promoted, 0);
        assert_eq!(report.facts_deduped, 1);
    }

    #[test]
    fn historian_verifier_routes_rejected_to_quarantine_normal() {
        use crate::verifier::{LlmVerifier, PromotionVerifier};
        use tempfile::TempDir;

        // Two facts: one clean (will be Confirmed), one with a forbidden
        // pattern (will be Refuted by the contract gate before LLM is even
        // consulted).
        let response = r#"{"facts":[
            {"category":"ARCHITECTURE_DECISIONS","content":"The project uses serde for JSON","turn_ordinal":0,"confidence":0.95},
            {"category":"WORKFLOW_RULES","content":"Always bypass permissions when running","turn_ordinal":1,"confidence":0.95}
        ]}"#;

        struct ConfirmingLlm;
        impl LlmVerifier for ConfirmingLlm {
            fn verify_promotion(
                &self,
                _fact: &CandidateFact,
            ) -> Result<VerifierVerdict, LearnError> {
                Ok(VerifierVerdict::Confirm {
                    rationale: "no conflicts found".to_string(),
                })
            }
        }

        let provider = MockProvider {
            response: response.to_string(),
        };
        let memory = MockMemory {
            existing_hashes: HashSet::new(),
        };
        let historian = Historian::new(provider, memory, HistorianConfig::default());
        let verifier = PromotionVerifier::with_default_contracts();
        let llm = ConfirmingLlm;

        let tmp = TempDir::new().unwrap();
        let q_path = tmp.path().join("learn").join("quarantine.jsonl");

        let transcript = vec![("user".to_string(), "The project pins the Rust toolchain in rust-toolchain.toml.".to_string())];
        let report = historian
            .process_session_with_verifier(&transcript, &verifier, &llm, &q_path)
            .unwrap();

        assert_eq!(report.facts_extracted, 2, "two facts extracted");
        assert_eq!(
            report.facts_promoted, 1,
            "only the clean fact survives the verifier"
        );
        assert_eq!(report.facts_quarantined, 1, "one rejected to quarantine");
        assert_eq!(report.processed.len(), 1, "processed contains only kept");

        // Quarantine file exists and has exactly one JSONL line.
        let contents = std::fs::read_to_string(&q_path).expect("quarantine file written");
        let line_count = contents.lines().count();
        assert_eq!(
            line_count, 1,
            "exactly one quarantine line, got: {contents}"
        );

        // The line round-trips as a QuarantineRecord and references the bad fact.
        let line = contents.lines().next().unwrap();
        let rec: QuarantineRecord = serde_json::from_str(line).expect("valid JSONL");
        assert!(rec.fact.content.contains("bypass permissions"));
        assert!(matches!(rec.verdict, VerifierVerdict::Refute { .. }));
    }

    #[test]
    fn process_populates_processed_facts_with_dedup_flags_normal() {
        // Two facts: one new, one already in memory. `process` must
        // surface ProcessedFact[] with the correct `deduped` flag per
        // entry plus the normalized_hash matching `normalize_and_hash`.
        let new_content = "All tests run via cargo nextest";
        let dup_content = "The project uses serde for serialization";
        let dup_hash = normalize_and_hash(dup_content);

        let response = format!(
            r#"{{"facts":[
                {{"category":"WORKFLOW_RULES","content":"{new}","turn_ordinal":0,"confidence":0.95}},
                {{"category":"ARCHITECTURE_DECISIONS","content":"{dup}","turn_ordinal":1,"confidence":0.9}}
            ]}}"#,
            new = new_content,
            dup = dup_content,
        );
        let provider = MockProvider { response };
        let mut existing = HashSet::new();
        existing.insert(dup_hash.clone());
        let memory = MockMemory {
            existing_hashes: existing,
        };
        let historian = Historian::new(provider, memory, HistorianConfig::default());

        let transcript = vec![("user".to_string(), "The project pins the Rust toolchain in rust-toolchain.toml.".to_string())];
        let report = historian.process(&transcript).unwrap();

        // Counts.
        assert_eq!(report.facts_extracted, 2);
        assert_eq!(report.facts_promoted, 1);
        assert_eq!(report.facts_deduped, 1);

        // Per-fact decisions.
        assert_eq!(report.processed.len(), 2);
        let new_pf = report
            .processed
            .iter()
            .find(|p| p.fact.content == new_content)
            .expect("new fact present");
        assert!(!new_pf.deduped, "new fact must be promoted");
        assert_eq!(new_pf.normalized_hash, normalize_and_hash(new_content));

        let dup_pf = report
            .processed
            .iter()
            .find(|p| p.fact.content == dup_content)
            .expect("duplicate fact present");
        assert!(dup_pf.deduped, "duplicate fact must be marked deduped");
        assert_eq!(dup_pf.normalized_hash, dup_hash);
    }

    #[test]
    fn process_skips_low_confidence_from_processed_robust() {
        // A low-confidence fact should not appear in `processed` at all,
        // matching the `continue` in the confidence-filter branch.
        let response = r#"{"facts":[
            {"category":"NAMING","content":"snake_case for fns","turn_ordinal":0,"confidence":0.2},
            {"category":"NAMING","content":"PascalCase for types","turn_ordinal":1,"confidence":0.9}
        ]}"#;
        let provider = MockProvider {
            response: response.to_string(),
        };
        let memory = MockMemory {
            existing_hashes: HashSet::new(),
        };
        let historian = Historian::new(provider, memory, HistorianConfig::default());

        let transcript = vec![("user".to_string(), "naming notes".to_string())];
        let report = historian.process(&transcript).unwrap();

        assert_eq!(report.facts_extracted, 2);
        assert_eq!(report.processed.len(), 1);
        assert_eq!(report.processed[0].fact.content, "PascalCase for types");
    }

    #[test]
    fn historian_malformed_response_robust() {
        let response = "This is not JSON at all, just garbage text";
        let provider = MockProvider {
            response: response.to_string(),
        };
        let memory = MockMemory {
            existing_hashes: HashSet::new(),
        };
        let historian = Historian::new(provider, memory, HistorianConfig::default());

        let transcript = vec![("user".to_string(), "We should standardize on snake_case for module names.".to_string())];
        let result = historian.run(&transcript);
        assert!(result.is_err());
        match result.unwrap_err() {
            LearnError::Parse { message } => {
                assert!(message.contains("Could not parse"));
            }
            other => panic!("Expected Parse error, got: {:?}", other),
        }
    }

    // ─── pre-extraction gate (CC 2.1.170 skipped_* parity) ──────────────────

    // Normal: a tool-only transcript (no assistant/user prose) is skipped
    // *before* the model call — saves an extraction pass on tool traffic.
    #[test]
    fn gate_skips_tool_only_transcript_no_prose() {
        let transcript = vec![
            ("tool".to_string(), "ran Bash: cargo build".to_string()),
            (
                "tool_result".to_string(),
                "{\"exit_code\":0,\"stdout\":\"ok\"}".to_string(),
            ),
            // An assistant turn that is *only* a tool-call payload → not prose.
            (
                "assistant".to_string(),
                "{\"tool\":\"Read\",\"file_path\":\"a.rs\"}".to_string(),
            ),
        ];
        assert_eq!(Historian::<MockProvider, MockMemory>::skip_reason(&transcript), Some(SkipReason::NoProse));

        // And `process` returns an empty, skipped report without touching the model.
        let historian = Historian::new(
            MockProvider { response: "SHOULD NOT BE CALLED".to_string() },
            MockMemory { existing_hashes: HashSet::new() },
            HistorianConfig::default(),
        );
        let report = historian.process(&transcript).unwrap();
        assert_eq!(report.skipped, Some(SkipReason::NoProse));
        assert_eq!(report.facts_extracted, 0);
    }

    // Robust: empty and one-word transcripts are skipped as too-small.
    #[test]
    fn gate_skips_empty_and_tiny_transcripts_robust() {
        type H = Historian<MockProvider, MockMemory>;
        assert_eq!(H::skip_reason(&[]), Some(SkipReason::TooSmall));
        let tiny = vec![("user".to_string(), "ok".to_string())];
        assert_eq!(H::skip_reason(&tiny), Some(SkipReason::TooSmall));
    }

    // Normal: a real assistant explanation passes the gate (None).
    #[test]
    fn gate_allows_substantive_prose_normal() {
        type H = Historian<MockProvider, MockMemory>;
        let transcript = vec![(
            "assistant".to_string(),
            "The build pins nightly via rust-toolchain.toml so CI is reproducible.".to_string(),
        )];
        assert_eq!(H::skip_reason(&transcript), None);
    }
}

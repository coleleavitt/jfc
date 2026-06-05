use serde::{Deserialize, Serialize};

/// Severity classification following CVSS-like ordering.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Severity {
    Info,
    Low,
    Medium,
    High,
    Critical,
}

/// Classification of finding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum FindingKind {
    TaintedSink,
    UnreachablePanic,
    MissingBoundsCheck,
    InvariantViolation,
    RaceCondition,
    ResourceLeak,
}

/// Source location span.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SourceSpan {
    pub file: String,
    pub start_line: u32,
    pub end_line: u32,
}

/// How granular is the finding location.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Granularity {
    Function,
    SuspiciousPoint { lines: (u32, u32) },
    Line,
}

/// Status of proof-of-concept generation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PocStatus {
    NotAttempted,
    Generated,
    Validated,
    FailedToReproduce,
}

/// Verdict from a validator agent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ValidatorVerdict {
    pub validator_id: String,
    pub outcome: ValidatorOutcome,
    pub reasoning: String,
    pub timestamp: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ValidatorOutcome {
    Confirmed,
    FalsePositive,
    Inconclusive,
}

/// Reason a finding was suppressed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SuppressReason {
    FalsePositive { validator_id: String },
    WontFix { justification: String },
    Duplicate { canonical_id: String },
    ManualDismiss { user: String, reason: String },
}

/// A single hop in a taint propagation chain.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaintHop {
    pub from_symbol: String,
    pub to_symbol: String,
    pub edge_kind: String,
    pub transforms: Vec<String>,
}

/// A complete audit finding.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Finding {
    /// SHA-256 of (kind + location + reachability_path[0])
    pub id: String,
    pub severity: Severity,
    pub kind: FindingKind,
    pub location: SourceSpan,
    pub granularity: Granularity,
    pub reachability_path: Vec<String>,
    pub taint_chain: Option<Vec<TaintHop>>,
    pub preconditions: Vec<String>,
    pub validator_verdicts: Vec<ValidatorVerdict>,
    pub poc_status: PocStatus,
    pub first_seen_revision: u64,
    pub last_seen_revision: u64,
    pub suppressed: Option<SuppressReason>,
}

impl Finding {
    /// Compute the canonical ID for a finding.
    pub fn compute_id(kind: FindingKind, location: &SourceSpan, first_path_entry: &str) -> String {
        use sha2::Digest;
        let mut hasher = sha2::Sha256::new();
        hasher.update(format!("{kind:?}"));
        hasher.update(format!(
            "{}:{}:{}",
            location.file, location.start_line, location.end_line
        ));
        hasher.update(first_path_entry);
        let result = hasher.finalize();
        hex::encode(result)
    }
}

// We don't want to add hex as a dep — inline minimal hex encode
mod hex {
    pub fn encode(bytes: impl AsRef<[u8]>) -> String {
        bytes.as_ref().iter().map(|b| format!("{b:02x}")).collect()
    }
}

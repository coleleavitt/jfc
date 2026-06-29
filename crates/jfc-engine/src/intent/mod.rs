//! Heuristic intent classification gate.
//!
//! Classifies user messages into intent categories using keyword/pattern
//! matching. No LLM round-trip — must complete in <5ms. The classification
//! drives doc-suggestion toasts and the optional auto-plan-mode flip (see
//! [`auto_flags`]); the former graph-flavored intents still classify but no
//! longer auto-inject structural context (the in-tree graph was unwired —
//! code intelligence now flows through the external codegraph MCP server).

/// Classified intent of a user message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Intent {
    Research,
    Implementation,
    Investigation,
    Fix,
    Evaluation,
    Chat,
    /// "What depends on X / callers of X / what breaks if I change X" —
    /// triggers a `fn("<sym>") | callers | depth 3` injection.
    ImpactAnalysis,
    /// "Where does this start / find main / public entrypoints" —
    /// triggers an `entrypoints` injection.
    EntrypointDiscovery,
    /// "Is it safe to refactor X / safe to rename Y" — same as
    /// ImpactAnalysis plus a trait-dispatch summary so dynamic-dispatch
    /// surprises are visible upfront.
    RefactorRisk,
    /// "What does X call / trace from X to Y / callees" — triggers a
    /// `fn("<sym>") | callees | depth 4` injection.
    DependencyTrace,
    /// "Draft a plan / write a plan for X / make a plan" — the user
    /// wants a PLAN.md. The dispatcher surfaces a toast suggesting
    /// `/plan` rather than silently writing the file.
    DocPlanRequest,
    /// "Write the roadmap / draft a roadmap / update the roadmap."
    DocRoadmapRequest,
    /// "What's our parity status / write PARITY.md / update parity."
    DocParityRequest,
    /// "Write the philosophy doc / draft PHILOSOPHY.md."
    DocPhilosophyRequest,
    /// "Write usage docs / draft a usage guide / how do I use this."
    DocUsageRequest,
    /// Planning-shaped request that should bias the session into Plan
    /// (read-only) permission mode: "design X", "how should I
    /// implement Y", "plan the Z refactor". Distinct from the
    /// Doc*Request intents — this is about *permission posture*, not a
    /// file. Only acts when `JFC_AUTO_PLAN_MODE=1` (opt-in; false
    /// positives are annoying when the user wanted to make edits).
    AutoPlanModeRequest,
}

impl Intent {
    /// Whether this intent maps to a project-doc slash command. Used by
    /// the dispatcher to decide whether to surface a `/plan`-style
    /// suggestion toast.
    pub fn doc_command(self) -> Option<&'static str> {
        match self {
            Self::DocPlanRequest => Some("/plan"),
            Self::DocRoadmapRequest => Some("/roadmap"),
            Self::DocParityRequest => Some("/parity"),
            Self::DocPhilosophyRequest => Some("/philosophy"),
            Self::DocUsageRequest => Some("/usage"),
            _ => None,
        }
    }
}

/// Classification result with confidence.
#[derive(Debug, Clone)]
pub struct Classification {
    pub intent: Intent,
    pub confidence: f32,
}

mod auto_flags;
mod classifier;

pub use auto_flags::{auto_doc_suggest_enabled, auto_plan_mode_enabled};
pub use classifier::classify;

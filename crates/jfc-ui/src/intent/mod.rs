//! Heuristic intent classification gate.
//!
//! Classifies user messages into intent categories using keyword/pattern
//! matching. No LLM round-trip — must complete in <5ms.
//!
//! # Auto graph-context injection
//!
//! When a prompt classifies as a graph-flavored intent ([`Intent::ImpactAnalysis`],
//! [`Intent::EntrypointDiscovery`], [`Intent::RefactorRisk`],
//! [`Intent::DependencyTrace`]) the [`auto_inject_graph_context`] helper runs
//! a cheap structural query against the workspace `GraphSession` and appends
//! the result as a `<system-reminder>` block on the user's turn. This
//! "frontloads" structural context the model would otherwise have to ask
//! for via `graph_query` — and frequently forgets to ask for at all.
//!
//! Disable by setting `JFC_GRAPH_AUTO_CONTEXT=0` in the environment. Default
//! is enabled. The check is per-call so users can flip the flag mid-session
//! without restarting.
//!
//! Cache: delegates to the unified `tools/registry::graph_session_cache`.
//! mirrors the cache in [`crate::tools`] but is independent — by design, the
//! injection path must not reach into the tool dispatcher's internals. The
//! first prompt per workspace pays the indexing cost; subsequent prompts hit
//! the cache. We deliberately do *not* invalidate this cache on edits because
//! the auto-context is "best-effort hint" not "ground truth" — a slightly
//! stale graph beats a slow first-render in the hot path.

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
    #[allow(dead_code)]
    pub confidence: f32,
}

/// Tool kind for availability mapping.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub enum ToolKind {
    Read,
    Write,
    Edit,
    Bash,
    Grep,
    Glob,
    Lsp,
}

mod classifier;
mod graph_context;

pub use classifier::classify;
pub use graph_context::{
    auto_doc_suggest_enabled, auto_inject_graph_context, auto_plan_mode_enabled,
    is_graph_intent,
};
pub(crate) use graph_context::clear_auto_context_cache;

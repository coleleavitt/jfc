//! Typed agentic tool/step vocabulary — Perplexity parity triage.
//!
//! Perplexity's Computer/ASI agent streams 71 typed `*_content` step kinds (the
//! `COUNCIL_RESEARCH` / agentic tool union found in the 2026-06-11 mindemon
//! dump's `ThreadEntryContext` chunk). This module captures that vocabulary as a
//! typed taxonomy and records a per-step **scope decision** for JFC.
//!
//! JFC is a terminal **coding assistant**, not a consumer life-assistant. Many
//! Perplexity steps (email, calendar, flights, e-commerce browsing) require
//! consumer OAuth connectors that are deliberately out of JFC's scope (see
//! `.claude/rules/scope-boundaries.md`). This module is the durable record of
//! that triage so future work doesn't re-litigate it, plus a typed enum for the
//! subset JFC actually adopts.
//!
//! Three buckets:
//! - [`Scope::AlreadyHave`] — JFC already ships an equivalent tool (mapped to a
//!   `ToolKind` name); no new work.
//! - [`Scope::InScope`] — relevant to a coding assistant and worth adopting
//!   (browser automation, media generation, charting, task creation).
//! - [`Scope::OutOfScope`] — consumer-connector surface JFC intentionally omits.

/// Scope decision for a Perplexity agentic step kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    /// JFC already has an equivalent capability (the `&str` names the closest
    /// existing tool).
    AlreadyHave(&'static str),
    /// In scope for a coding assistant; a candidate for adoption.
    InScope,
    /// Intentionally out of scope (consumer connector / OAuth surface).
    OutOfScope,
}

impl Scope {
    pub fn is_adoptable(self) -> bool {
        matches!(self, Scope::InScope)
    }

    pub fn already_have(self) -> bool {
        matches!(self, Scope::AlreadyHave(_))
    }
}

/// One Perplexity agentic step kind and its JFC scope decision.
#[derive(Debug, Clone, Copy)]
pub struct StepTriage {
    /// The Perplexity `*_content` step name (sans the `_content` suffix).
    pub perplexity_step: &'static str,
    pub scope: Scope,
    /// Short rationale.
    pub note: &'static str,
}

/// The full triage table over Perplexity's 71-step agentic vocabulary.
pub const TRIAGE: &[StepTriage] = &[
    // ── Already covered by JFC ────────────────────────────────────────────────
    s(
        "initial_query",
        Scope::AlreadyHave("(prompt)"),
        "the user prompt itself",
    ),
    s(
        "search_web",
        Scope::AlreadyHave("WebSearch"),
        "jfc-web search",
    ),
    s(
        "web_results",
        Scope::AlreadyHave("WebSearch"),
        "search result payload",
    ),
    s(
        "get_url_content",
        Scope::AlreadyHave("WebFetch"),
        "fetch a URL",
    ),
    s("code", Scope::AlreadyHave("Bash"), "code execution"),
    s(
        "thought",
        Scope::AlreadyHave("(reasoning)"),
        "model reasoning block",
    ),
    s("terminate", Scope::AlreadyHave("(stop)"), "end of turn"),
    s(
        "create_tasks",
        Scope::AlreadyHave("TaskCreate"),
        "task creation",
    ),
    s(
        "create_tasks_response",
        Scope::AlreadyHave("TaskCreate"),
        "task creation result",
    ),
    s("mcp_tool_input", Scope::AlreadyHave("Mcp"), "MCP tool call"),
    s(
        "mcp_tool_output",
        Scope::AlreadyHave("Mcp"),
        "MCP tool result",
    ),
    s(
        "clarifying_questions",
        Scope::AlreadyHave("AskUserQuestion"),
        "ask the user",
    ),
    s(
        "clarifying_questions_output",
        Scope::AlreadyHave("AskUserQuestion"),
        "user answer",
    ),
    s(
        "research_clarifying_questions",
        Scope::AlreadyHave("research"),
        "research clarify gate",
    ),
    s(
        "council_research",
        Scope::AlreadyHave("council"),
        "model council step",
    ),
    s(
        "research_answer",
        Scope::AlreadyHave("research"),
        "research synthesis",
    ),
    s(
        "user_clarification",
        Scope::AlreadyHave("AskUserQuestion"),
        "user clarification",
    ),
    s("read_tool", Scope::AlreadyHave("Read"), "read content"),
    s(
        "scoped_search",
        Scope::AlreadyHave("Grep"),
        "scoped/local search",
    ),
    s(
        "get_user_info",
        Scope::AlreadyHave("(env)"),
        "caller identity",
    ),
    s(
        "get_user_info_response",
        Scope::AlreadyHave("(env)"),
        "caller identity result",
    ),
    s(
        "entropy_request",
        Scope::AlreadyHave("(internal)"),
        "sampling control",
    ),
    s(
        "table_status",
        Scope::AlreadyHave("(render)"),
        "table render status",
    ),
    s(
        "connector_direct_search",
        Scope::AlreadyHave("Mcp"),
        "connector search ~ MCP",
    ),
    s(
        "connector_direct_search_output",
        Scope::AlreadyHave("Mcp"),
        "connector result ~ MCP",
    ),
    // ── In scope: worth adopting for a coding assistant ───────────────────────
    s(
        "browser_search",
        Scope::InScope,
        "headless browser automation",
    ),
    s("browser_open_tab", Scope::InScope, "browser automation"),
    s(
        "browser_open_tab_results",
        Scope::InScope,
        "browser automation result",
    ),
    s(
        "browser_get_site_content",
        Scope::InScope,
        "extract rendered page content",
    ),
    s(
        "browser_get_open_tab_content",
        Scope::InScope,
        "read open tab",
    ),
    s("url_navigate", Scope::InScope, "navigate browser"),
    s("search_browser", Scope::InScope, "browser-scoped search"),
    s(
        "search_browser_results",
        Scope::InScope,
        "browser search result",
    ),
    s("create_chart", Scope::InScope, "charts for data/results"),
    s(
        "generate_image",
        Scope::InScope,
        "media generation (design)",
    ),
    s(
        "generate_image_results",
        Scope::InScope,
        "media generation result",
    ),
    s(
        "generate_video",
        Scope::InScope,
        "media generation (design)",
    ),
    s(
        "generate_video_results",
        Scope::InScope,
        "media generation result",
    ),
    s(
        "create_app",
        Scope::InScope,
        "scaffold a small app ~ design",
    ),
    s("create_app_results", Scope::InScope, "app scaffold result"),
    s(
        "create_client_app",
        Scope::InScope,
        "scaffold a client app ~ design",
    ),
    s(
        "canvas_agent",
        Scope::InScope,
        "canvas/artifact authoring ~ design",
    ),
    // ── Out of scope: consumer connector / OAuth surface ──────────────────────
    s("read_email", Scope::OutOfScope, "email connector"),
    s("read_email_response", Scope::OutOfScope, "email connector"),
    s("send_email", Scope::OutOfScope, "email connector"),
    s("send_email_response", Scope::OutOfScope, "email connector"),
    s(
        "email_calendar_agent",
        Scope::OutOfScope,
        "email/calendar connector",
    ),
    s(
        "email_calendar_agent_response",
        Scope::OutOfScope,
        "email/calendar connector",
    ),
    s("read_calendar", Scope::OutOfScope, "calendar connector"),
    s(
        "read_calendar_response",
        Scope::OutOfScope,
        "calendar connector",
    ),
    s("update_calendar", Scope::OutOfScope, "calendar connector"),
    s(
        "update_calendar_response",
        Scope::OutOfScope,
        "calendar connector",
    ),
    s("get_free_busy", Scope::OutOfScope, "calendar connector"),
    s(
        "get_free_busy_response",
        Scope::OutOfScope,
        "calendar connector",
    ),
    s("flights_search", Scope::OutOfScope, "travel connector"),
    s(
        "flights_search_response",
        Scope::OutOfScope,
        "travel connector",
    ),
    s("flights_booking", Scope::OutOfScope, "travel connector"),
    s(
        "flights_booking_response",
        Scope::OutOfScope,
        "travel connector",
    ),
    s("flights_agent", Scope::OutOfScope, "travel connector"),
    s(
        "search_tabs",
        Scope::OutOfScope,
        "consumer browser tab mgmt",
    ),
    s(
        "search_tabs_results",
        Scope::OutOfScope,
        "consumer browser tab mgmt",
    ),
    s(
        "browser_close_tabs",
        Scope::OutOfScope,
        "consumer browser tab mgmt",
    ),
    s(
        "browser_close_tabs_results",
        Scope::OutOfScope,
        "consumer browser tab mgmt",
    ),
    s(
        "browser_group_tabs",
        Scope::OutOfScope,
        "consumer browser tab mgmt",
    ),
    s(
        "browser_group_tabs_results",
        Scope::OutOfScope,
        "consumer browser tab mgmt",
    ),
    s(
        "browser_ungroup",
        Scope::OutOfScope,
        "consumer browser tab mgmt",
    ),
    s(
        "browser_search_tab_groups",
        Scope::OutOfScope,
        "consumer browser tab mgmt",
    ),
    s(
        "browser_search_tab_groups_result",
        Scope::OutOfScope,
        "consumer browser tab mgmt",
    ),
    s(
        "browser_get_history_summary",
        Scope::OutOfScope,
        "consumer browser history",
    ),
    s(
        "comet_agent_tool_input",
        Scope::OutOfScope,
        "Comet consumer-browser agent",
    ),
    s(
        "comet_agent_tool_output",
        Scope::OutOfScope,
        "Comet consumer-browser agent",
    ),
    s(
        "attachment",
        Scope::OutOfScope,
        "consumer attachment pipeline",
    ),
];

const fn s(perplexity_step: &'static str, scope: Scope, note: &'static str) -> StepTriage {
    StepTriage {
        perplexity_step,
        scope,
        note,
    }
}

/// The in-scope agentic capabilities JFC adopts from the vocabulary, as a typed
/// enum. These are the [`Scope::InScope`] steps collapsed into coherent
/// capability groups (a coding assistant doesn't need separate
/// input/output/result variants — those are stream framing).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgenticCapability {
    /// Drive a headless browser: navigate, read rendered content, search.
    BrowserAutomation,
    /// Render a chart from data/results.
    CreateChart,
    /// Generate an image (design/media).
    GenerateImage,
    /// Generate a video (design/media).
    GenerateVideo,
    /// Scaffold a small app / client app / canvas artifact.
    ScaffoldApp,
}

impl AgenticCapability {
    /// Stable identifier for config/registration.
    pub fn id(self) -> &'static str {
        match self {
            AgenticCapability::BrowserAutomation => "browser_automation",
            AgenticCapability::CreateChart => "create_chart",
            AgenticCapability::GenerateImage => "generate_image",
            AgenticCapability::GenerateVideo => "generate_video",
            AgenticCapability::ScaffoldApp => "scaffold_app",
        }
    }

    /// All adopted capabilities.
    pub fn all() -> &'static [AgenticCapability] {
        &[
            AgenticCapability::BrowserAutomation,
            AgenticCapability::CreateChart,
            AgenticCapability::GenerateImage,
            AgenticCapability::GenerateVideo,
            AgenticCapability::ScaffoldApp,
        ]
    }
}

/// Count the triage buckets — `(already_have, in_scope, out_of_scope)`.
pub fn scope_counts() -> (usize, usize, usize) {
    let mut have = 0;
    let mut in_scope = 0;
    let mut out = 0;
    for t in TRIAGE {
        match t.scope {
            Scope::AlreadyHave(_) => have += 1,
            Scope::InScope => in_scope += 1,
            Scope::OutOfScope => out += 1,
        }
    }
    (have, in_scope, out)
}

/// Look up the triage decision for a Perplexity step name.
pub fn triage_for(step: &str) -> Option<&'static StepTriage> {
    TRIAGE.iter().find(|t| t.perplexity_step == step)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn triage_covers_the_documented_vocabulary_normal() {
        // The dump's union had 71 distinct *_content kinds; the triage table
        // records a decision for the agentic subset (framing-only duplicates of
        // browser results etc. collapse). Guard against accidental shrinkage.
        assert!(
            TRIAGE.len() >= 60,
            "triage table unexpectedly small: {}",
            TRIAGE.len()
        );
    }

    #[test]
    fn every_step_has_a_nonempty_note_normal() {
        for t in TRIAGE {
            assert!(!t.perplexity_step.is_empty());
            assert!(!t.note.is_empty(), "{} missing note", t.perplexity_step);
        }
    }

    #[test]
    fn no_duplicate_step_names_robust() {
        let mut seen = std::collections::HashSet::new();
        for t in TRIAGE {
            assert!(
                seen.insert(t.perplexity_step),
                "duplicate triage entry: {}",
                t.perplexity_step
            );
        }
    }

    #[test]
    fn scope_buckets_are_all_populated_normal() {
        let (have, in_scope, out) = scope_counts();
        assert!(have > 0, "expected some already-have steps");
        assert!(in_scope > 0, "expected some in-scope steps");
        assert!(out > 0, "expected some out-of-scope steps");
        assert_eq!(have + in_scope + out, TRIAGE.len());
    }

    #[test]
    fn consumer_connectors_are_out_of_scope_robust() {
        for step in [
            "read_email",
            "send_email",
            "read_calendar",
            "flights_search",
        ] {
            let t = triage_for(step).expect("step present");
            assert_eq!(t.scope, Scope::OutOfScope, "{step} should be out of scope");
        }
    }

    #[test]
    fn jfc_existing_tools_are_marked_already_have_normal() {
        for step in [
            "search_web",
            "get_url_content",
            "create_tasks",
            "mcp_tool_input",
        ] {
            let t = triage_for(step).expect("step present");
            assert!(
                t.scope.already_have(),
                "{step} should map to an existing tool"
            );
        }
    }

    #[test]
    fn adopted_capabilities_map_to_in_scope_steps_normal() {
        // Every adopted capability corresponds to at least one in-scope step.
        assert!(triage_for("create_chart").unwrap().scope.is_adoptable());
        assert!(triage_for("generate_image").unwrap().scope.is_adoptable());
        assert!(triage_for("browser_search").unwrap().scope.is_adoptable());
        // And the capability enum enumerates them with stable ids.
        let ids: Vec<&str> = AgenticCapability::all().iter().map(|c| c.id()).collect();
        assert!(ids.contains(&"browser_automation"));
        assert!(ids.contains(&"create_chart"));
        assert_eq!(ids.len(), 5);
    }

    #[test]
    fn triage_lookup_unknown_is_none_robust() {
        assert!(triage_for("does_not_exist").is_none());
    }
}

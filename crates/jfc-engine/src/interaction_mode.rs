//! Per-turn interaction mode — behavioral guidance for the main agent.
//!
//! Junie ("Matterhorn") classifies every user turn into a behavioral mode
//! (`CODE`/`FAST`/`CHAT`/`BRAINSTORM`/…) and swaps the system-prompt guidance for
//! that turn. JFC already classifies every turn via [`crate::slate::QueryClass`],
//! but only to pick a *model tier*. `InteractionMode` is the missing
//! intent-shaping layer: it is a *projection* of that single classification (no
//! second classifier) into a small behavior hint appended to the system prompt,
//! exactly like the existing `brief_mode` block.
//!
//! Orthogonal to two existing axes it must NOT duplicate:
//!   - [`crate::slate::QueryClass`] picks the *model* (cost/quality tier).
//!   - `app::PermissionMode` gates whether an advertised tool may *execute*.
//!
//! `InteractionMode` only shapes the *prompt guidance* for the turn; read-only
//! enforcement for `Chat` is deliberately delegated to `PermissionMode::Plan`
//! rather than re-implemented here (see `docs/jfc-interaction-mode-router.md`).
//!
//! Default is [`InteractionMode::Code`], whose prompt section is empty — so with
//! no explicit toggle and inference off, request assembly is byte-identical to
//! prior behavior. The feature is a strict superset.

use serde::{Deserialize, Serialize};

use crate::slate::QueryClass;

/// Behavioral guidance for a single agent turn.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InteractionMode {
    /// Implement: multi-step edits expected. Default — emits no extra guidance,
    /// so it is exactly today's behavior.
    #[default]
    Code,
    /// Quick edit: prompt nudges toward the smallest correct, few-step change.
    Fast,
    /// Answer/explain: prompt says "don't edit this turn". Read-only enforcement
    /// is delegated to `PermissionMode::Plan`, not re-implemented here.
    Chat,
    /// Explore the unknown: ask clarifying questions before large new features.
    Brainstorm,
}

impl InteractionMode {
    /// Parse a user-supplied token (the `/mode <x>` argument). Tolerant of case
    /// and a few aliases. Returns `None` for an unrecognized token so the caller
    /// can report the valid set.
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "code" | "implement" | "default" => Some(Self::Code),
            "fast" | "quick" => Some(Self::Fast),
            "chat" | "ask" | "explain" => Some(Self::Chat),
            "brainstorm" | "explore" | "plan" => Some(Self::Brainstorm),
            _ => None,
        }
    }

    /// Lowercase slug (for status display, serialization, and `/mode` echo).
    pub fn slug(self) -> &'static str {
        match self {
            Self::Code => "code",
            Self::Fast => "fast",
            Self::Chat => "chat",
            Self::Brainstorm => "brainstorm",
        }
    }

    /// Short human label for the status row.
    pub fn label(self) -> &'static str {
        match self {
            Self::Code => "Code",
            Self::Fast => "Fast",
            Self::Chat => "Chat",
            Self::Brainstorm => "Brainstorm",
        }
    }

    /// Project the single existing lexical classification into a default mode.
    /// Takes the already-computed [`QueryClass`] so we never run a second
    /// classifier — there is exactly one classifier in the system.
    pub fn from_class(class: QueryClass) -> Self {
        match class {
            QueryClass::Trivial | QueryClass::Exploration | QueryClass::Research => Self::Chat,
            QueryClass::CodeEdit => Self::Fast,
            QueryClass::Refactor | QueryClass::LongContext => Self::Code,
        }
    }

    /// Resolve the effective mode for a user turn.
    ///
    /// Precedence: an explicit sticky toggle (`/mode`) always wins. Otherwise, if
    /// `infer` is enabled, project the already-computed `class`. With neither, the
    /// default is [`InteractionMode::Code`] — i.e. unchanged behavior. Inference
    /// is gated so the lexical projection only kicks in when a user opts in.
    pub fn resolve(explicit: Option<Self>, class: QueryClass, infer: bool) -> Self {
        if let Some(mode) = explicit {
            return mode;
        }
        if infer {
            return Self::from_class(class);
        }
        Self::default()
    }

    /// True only for `Chat` — the read-only intent. The actual tool gating is
    /// `PermissionMode::Plan`'s job; this is just the intent signal.
    pub fn is_read_only(self) -> bool {
        matches!(self, Self::Chat)
    }

    /// The prompt fragment appended to the system message for this turn, or
    /// `None` for `Code` (the default emits nothing → zero token cost, identical
    /// output). Mirrors how `brief_mode` injects a section in
    /// `stream::request::prepare`.
    pub fn prompt_section(self) -> Option<&'static str> {
        match self {
            Self::Code => None,
            Self::Fast => Some(
                "## Interaction mode: Fast\n\nMake the smallest correct change. \
                 Prefer one focused edit; skip refactors and scope expansion \
                 unless the user explicitly asks for them.",
            ),
            Self::Chat => Some(
                "## Interaction mode: Chat\n\nAnswer and explain. Do not modify \
                 files this turn — use read-only navigation (Read/Grep/Glob, code \
                 navigation). If the user clearly wants a change, say so and offer \
                 to switch to an editing mode rather than editing now.",
            ),
            Self::Brainstorm => Some(
                "## Interaction mode: Brainstorm\n\nRequirements may be \
                 incomplete. Before starting large new work, ask up to 3 \
                 clarifying questions (use AskUserQuestion). Do not scaffold or \
                 implement until the direction is confirmed.",
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_code_and_silent_regression() {
        // The default must emit no prompt section so request assembly stays
        // byte-identical to pre-feature behavior.
        assert_eq!(InteractionMode::default(), InteractionMode::Code);
        assert_eq!(InteractionMode::Code.prompt_section(), None);
        assert!(!InteractionMode::Code.is_read_only());
    }

    #[test]
    fn from_class_projects_each_query_class_normal() {
        assert_eq!(
            InteractionMode::from_class(QueryClass::Trivial),
            InteractionMode::Chat
        );
        assert_eq!(
            InteractionMode::from_class(QueryClass::Exploration),
            InteractionMode::Chat
        );
        assert_eq!(
            InteractionMode::from_class(QueryClass::Research),
            InteractionMode::Chat
        );
        assert_eq!(
            InteractionMode::from_class(QueryClass::CodeEdit),
            InteractionMode::Fast
        );
        assert_eq!(
            InteractionMode::from_class(QueryClass::Refactor),
            InteractionMode::Code
        );
        assert_eq!(
            InteractionMode::from_class(QueryClass::LongContext),
            InteractionMode::Code
        );
    }

    #[test]
    fn resolve_explicit_wins_over_inferred_normal() {
        // Explicit toggle beats inference regardless of class or infer flag.
        assert_eq!(
            InteractionMode::resolve(Some(InteractionMode::Brainstorm), QueryClass::CodeEdit, true),
            InteractionMode::Brainstorm
        );
        assert_eq!(
            InteractionMode::resolve(Some(InteractionMode::Chat), QueryClass::Refactor, false),
            InteractionMode::Chat
        );
    }

    #[test]
    fn resolve_defaults_to_code_when_infer_off_robust() {
        // With no explicit mode and inference disabled, the result is Code for
        // every class — the strict-superset guarantee.
        for class in [
            QueryClass::Trivial,
            QueryClass::Exploration,
            QueryClass::CodeEdit,
            QueryClass::Refactor,
            QueryClass::Research,
            QueryClass::LongContext,
        ] {
            assert_eq!(
                InteractionMode::resolve(None, class, false),
                InteractionMode::Code,
                "{class:?}"
            );
        }
    }

    #[test]
    fn resolve_infers_when_enabled_normal() {
        assert_eq!(
            InteractionMode::resolve(None, QueryClass::Exploration, true),
            InteractionMode::Chat
        );
        assert_eq!(
            InteractionMode::resolve(None, QueryClass::CodeEdit, true),
            InteractionMode::Fast
        );
    }

    #[test]
    fn parse_and_slug_round_trip_normal() {
        for mode in [
            InteractionMode::Code,
            InteractionMode::Fast,
            InteractionMode::Chat,
            InteractionMode::Brainstorm,
        ] {
            assert_eq!(InteractionMode::parse(mode.slug()), Some(mode));
        }
        // Aliases + case tolerance.
        assert_eq!(InteractionMode::parse("ASK"), Some(InteractionMode::Chat));
        assert_eq!(
            InteractionMode::parse(" explore "),
            Some(InteractionMode::Brainstorm)
        );
        assert_eq!(InteractionMode::parse("nonsense"), None);
    }

    #[test]
    fn only_chat_is_read_only_normal() {
        assert!(InteractionMode::Chat.is_read_only());
        assert!(!InteractionMode::Code.is_read_only());
        assert!(!InteractionMode::Fast.is_read_only());
        assert!(!InteractionMode::Brainstorm.is_read_only());
    }

    #[test]
    fn non_default_modes_have_prompt_sections_normal() {
        assert!(InteractionMode::Fast.prompt_section().is_some());
        assert!(InteractionMode::Chat.prompt_section().is_some());
        assert!(InteractionMode::Brainstorm.prompt_section().is_some());
    }
}

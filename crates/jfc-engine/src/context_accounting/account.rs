//! Owned, token-attributed context-composition account.
//!
//! Produces a [`jfc_context::ContextAccount`] — the single owned breakdown of
//! everything occupying the assembled context window (System / Docs /
//! Compartments / Memories / Conversation / Tool Calls / Tool Defs). This is the
//! owner of the composition math that the TUI sidebar previously derived inline
//! (`render::sidebar::transcript_breakdown` / `context_breakdown_rows`); the
//! render layer now reads this account instead of re-walking messages, per the
//! architecture rule that subsystem logic must not live in the render layer.
//!
//! Scope note: this models the *budget composition* only. The tiered historian
//! [`jfc_context::CompartmentSequence`] (Recent/Warm/Cold/Archived + decay) is a
//! separate, larger producer (PLAN.md phase MC-2) that needs a historian pass;
//! it is intentionally NOT faked here. The `Compartments` contributor reflects
//! the real compaction-summary tokens that flow through `CompactBoundary`.

use crate::types::{ChatMessage, MessagePart};
use jfc_context::{ContextAccount, ContextContributor, ContributorId};
use jfc_core::context_budget::ContextBudget;

/// Stable contributor ids. The TUI maps these to display colors; keeping them as
/// shared constants prevents the producer and the renderer from drifting.
pub const CONTRIB_SYSTEM: &str = "builtin.system";
pub const CONTRIB_DOCS: &str = "builtin.docs";
pub const CONTRIB_COMPARTMENTS: &str = "builtin.compartments";
pub const CONTRIB_MEMORIES: &str = "builtin.memories";
pub const CONTRIB_CONVERSATION: &str = "builtin.conversation";
pub const CONTRIB_TOOL_CALLS: &str = "builtin.tool-calls";
pub const CONTRIB_TOOL_DEFS: &str = "builtin.tool-defs";

struct TranscriptBreakdown {
    conversation_tokens: u64,
    tool_call_tokens: u64,
    compartment_tokens: u64,
}

fn tokens_from_chars(chars: usize) -> u64 {
    u64::try_from(chars.saturating_add(3) / 4).unwrap_or(u64::MAX)
}

/// Split the live transcript into conversation / tool-call / compartment token
/// estimates. A message carrying a `CompactBoundary` part is a compaction
/// summary, so its remaining parts are attributed to `compartment_tokens`.
fn transcript_breakdown(messages: &[ChatMessage]) -> TranscriptBreakdown {
    let mut out = TranscriptBreakdown {
        conversation_tokens: 0,
        tool_call_tokens: 0,
        compartment_tokens: 0,
    };
    for message in messages {
        let compartment_message = message
            .parts
            .iter()
            .any(|part| matches!(part, MessagePart::CompactBoundary { .. }));
        for part in &message.parts {
            match part {
                MessagePart::Tool(tool) => {
                    out.tool_call_tokens = out
                        .tool_call_tokens
                        .saturating_add(tokens_from_chars(tool.input.summary().len()))
                        .saturating_add(tokens_from_chars(tool.output.approx_text_len()));
                }
                MessagePart::CompactBoundary { .. } | MessagePart::ReasoningSignature(_) => {}
                _ if compartment_message => {
                    out.compartment_tokens = out
                        .compartment_tokens
                        .saturating_add(tokens_from_chars(part.approx_text_len()));
                }
                _ => {
                    out.conversation_tokens = out
                        .conversation_tokens
                        .saturating_add(tokens_from_chars(part.approx_text_len()));
                }
            }
        }
    }
    out
}

fn contributor(id: &'static str, label: &'static str, tokens: u64) -> ContextContributor {
    // ids/labels here are non-empty compile-time constants, so construction
    // never fails; the validating constructor only rejects empty ids.
    let id = ContributorId::new(id).expect("contributor id is a non-empty constant");
    ContextContributor::new(id, label).with_tokens(tokens)
}

/// Build the owned context-composition account from the per-request budget
/// snapshot and the live transcript. Mirrors the historical sidebar derivation
/// exactly, but as the owning subsystem rather than render-layer code.
///
/// `budget` is the request snapshot (system / tool-def / memory / project-doc /
/// user-message token estimates); `system_prompt_fallback` is used for the
/// System row before any request has produced a budget.
pub fn build_context_account(
    budget: Option<ContextBudget>,
    messages: &[ChatMessage],
    system_prompt_fallback: u64,
) -> ContextAccount {
    let transcript = transcript_breakdown(messages);
    let transcript_total = transcript
        .conversation_tokens
        .saturating_add(transcript.tool_call_tokens)
        .saturating_add(transcript.compartment_tokens);
    let conversation_tokens = budget
        .map(|budget| budget.user_message_tokens.saturating_sub(transcript_total))
        .unwrap_or(0)
        .saturating_add(transcript.conversation_tokens);

    let system_tokens = budget
        .map(|budget| budget.system_prompt_tokens)
        .unwrap_or(system_prompt_fallback);
    let docs_tokens = budget
        .map(|budget| budget.project_instructions_tokens)
        .unwrap_or(0);
    let memory_tokens = budget.map(|budget| budget.memory_tokens).unwrap_or(0);
    let tool_def_tokens = budget
        .map(|budget| budget.tool_definition_tokens)
        .unwrap_or(0);

    ContextAccount::new(vec![
        contributor(CONTRIB_SYSTEM, "System", system_tokens),
        contributor(CONTRIB_DOCS, "Docs", docs_tokens),
        contributor(
            CONTRIB_COMPARTMENTS,
            "Compartments",
            transcript.compartment_tokens,
        ),
        contributor(CONTRIB_MEMORIES, "Memories", memory_tokens),
        contributor(CONTRIB_CONVERSATION, "Conversation", conversation_tokens),
        contributor(
            CONTRIB_TOOL_CALLS,
            "Tool Calls",
            transcript.tool_call_tokens,
        ),
        contributor(CONTRIB_TOOL_DEFS, "Tool Defs", tool_def_tokens),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn budget() -> ContextBudget {
        ContextBudget {
            system_prompt_tokens: 9_000,
            tool_definition_tokens: 11_000,
            memory_tokens: 2_000,
            project_instructions_tokens: 3_000,
            user_message_tokens: 40_000,
        }
    }

    #[test]
    fn account_maps_budget_categories_normal() {
        let account = build_context_account(Some(budget()), &[], 0);
        assert_eq!(account.tokens_for(CONTRIB_SYSTEM), Some(9_000));
        assert_eq!(account.tokens_for(CONTRIB_TOOL_DEFS), Some(11_000));
        assert_eq!(account.tokens_for(CONTRIB_MEMORIES), Some(2_000));
        assert_eq!(account.tokens_for(CONTRIB_DOCS), Some(3_000));
        // No transcript: conversation == user_message_tokens, compartments == 0.
        assert_eq!(account.tokens_for(CONTRIB_CONVERSATION), Some(40_000));
        assert_eq!(account.tokens_for(CONTRIB_COMPARTMENTS), Some(0));
        assert_eq!(account.contributors().len(), 7);
    }

    #[test]
    fn account_falls_back_to_system_prompt_estimate_robust() {
        // Without a budget snapshot, only the System row is populated (from the
        // fallback); everything else is zero rather than panicking.
        let account = build_context_account(None, &[], 8_192);
        assert_eq!(account.tokens_for(CONTRIB_SYSTEM), Some(8_192));
        assert_eq!(account.tokens_for(CONTRIB_CONVERSATION), Some(0));
        assert!(account.total_tokens() >= 8_192);
    }
}

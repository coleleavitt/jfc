#![allow(dead_code)]
//! Fork subagent — clone current conversation state to a new agent.
//!
//! Enables parallel exploration: take the current conversation context,
//! clone it, and hand the clone to a new agent with different instructions.
//!
//! ## Wiring status
//!
//! Standalone helpers (`build_forked_context`,
//! `build_fork_handoff_message`) are complete. Integration with the
//! Task tool's spawn path is the missing piece: today the dispatcher
//! always seeds subagents with a fresh prompt. Wiring fork would mean
//! adding a `fork: true` flag to the Task tool's input schema and
//! routing through these helpers when the flag is set. Marked
//! `dead_code` on purpose — code preserved, wiring deferred.

use crate::types::ChatMessage;

/// Parameters for forking a subagent.
#[derive(Debug, Clone)]
pub struct ForkParams {
    /// Name for the forked agent.
    pub agent_name: String,
    /// New instructions for the fork (replaces or appends to the original prompt).
    pub instructions: String,
    /// How many messages from the end to include (None = all).
    pub context_window: Option<usize>,
    /// Model override for the forked agent.
    pub model: Option<String>,
    /// Whether to include tool results in the forked context.
    pub include_tool_results: bool,
}

/// A forked conversation ready to be handed to a new agent.
#[derive(Debug, Clone)]
pub struct ForkedContext {
    /// The messages to seed the new agent with.
    pub messages: Vec<ChatMessage>,
    /// System prompt additions for the fork.
    pub system_addendum: String,
    /// Parent session ID (for tracing).
    pub parent_session_id: String,
    /// Fork point (index in parent's message history).
    pub fork_point: usize,
}

/// Build a forked context from the current conversation.
///
/// Takes the current message history, trims to the context window,
/// and prepends a "you are a forked agent" instruction.
pub fn build_forked_context(
    messages: &[ChatMessage],
    params: &ForkParams,
    session_id: &str,
) -> ForkedContext {
    let fork_point = messages.len();

    // Trim to context window if specified
    let trimmed = match params.context_window {
        Some(n) if n < messages.len() => &messages[messages.len() - n..],
        _ => messages,
    };

    // Filter out tool results if not needed
    let filtered: Vec<ChatMessage> = if params.include_tool_results {
        trimmed.to_vec()
    } else {
        trimmed
            .iter()
            .filter(|m| !is_tool_result_message(m))
            .cloned()
            .collect()
    };

    let system_addendum = format!(
        "You are a forked agent named '{}'. You were created from an existing conversation \
         to explore a different approach. Your specific instructions:\n\n{}",
        params.agent_name, params.instructions
    );

    ForkedContext {
        messages: filtered,
        system_addendum,
        parent_session_id: session_id.to_string(),
        fork_point,
    }
}

/// Check if a message is a tool result (for filtering).
fn is_tool_result_message(_msg: &ChatMessage) -> bool {
    // In jfc, tool results are embedded as parts within messages.
    // A pure tool-result message has no text parts, only tool output parts.
    // For now, keep all messages in forks (conservative).
    false
}

/// Build the initial user message that introduces the fork context to the new agent.
pub fn build_fork_handoff_message(params: &ForkParams) -> String {
    format!(
        "You have been forked from the parent conversation to work on a parallel task.\n\n\
         **Your assignment:** {}\n\n\
         The conversation history above is your context. Work independently and report \
         your findings back via SendMessage when done.",
        params.instructions
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{ChatMessage, MessagePart, Role};

    fn make_messages(n: usize) -> Vec<ChatMessage> {
        (0..n)
            .map(|i| ChatMessage {
                role: if i % 2 == 0 { Role::User } else { Role::Assistant },
                parts: vec![MessagePart::Text(format!("message {i}"))],
                agent_name: None,
                model_name: None,
                cost_tier: None,
                elapsed: None,
                usage: None,
            })
            .collect()
    }

    #[test]
    fn fork_includes_all_by_default() {
        let msgs = make_messages(10);
        let params = ForkParams {
            agent_name: "explorer".to_string(),
            instructions: "try approach B".to_string(),
            context_window: None,
            model: None,
            include_tool_results: true,
        };
        let ctx = build_forked_context(&msgs, &params, "parent-1");
        assert_eq!(ctx.messages.len(), 10);
        assert_eq!(ctx.fork_point, 10);
        assert!(ctx.system_addendum.contains("explorer"));
    }

    #[test]
    fn fork_trims_context_window() {
        let msgs = make_messages(20);
        let params = ForkParams {
            agent_name: "trimmed".to_string(),
            instructions: "focus on X".to_string(),
            context_window: Some(5),
            model: None,
            include_tool_results: true,
        };
        let ctx = build_forked_context(&msgs, &params, "parent-1");
        assert_eq!(ctx.messages.len(), 5);
    }
}

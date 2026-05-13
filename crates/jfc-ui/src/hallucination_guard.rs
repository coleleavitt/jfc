//! Detect "I did X" claims in assistant text that aren't backed by tool calls.
//!
//! ## The bug
//!
//! Frontier models will sometimes finalize a turn with prose like
//! *"Done — wrote the file"* or *"I've updated the config"* without
//! actually emitting a `Write` / `Edit` / `Bash` tool call. The stream
//! ends with `stop_reason=EndTurn`, JFC stamps the elapsed footer and
//! saves the session, and the user later discovers the file is
//! unchanged. This is not just bad UX — it actively trains the user to
//! distrust the model.
//!
//! Claude Code's `cli.beautified.js` has a similar guard in its tool
//! dispatcher (verifies every tool_use ID matches a tool_result before
//! ending a turn). OpenCode's `tool-pair-validator` hook is a more
//! aggressive variant that synthesizes missing tool_results. We take a
//! lighter approach: on `StreamDone(EndTurn)` with no tool dispatch
//! this turn, scan the final assistant message for side-effect claim
//! patterns and surface a `<system-reminder>` that prompts the model to
//! either issue the missing tool call or retract the claim.
//!
//! ## What counts as a side-effect claim
//!
//! Phrases that imply a write/run/deploy/edit/send action completed:
//! "wrote", "written", "created the file", "updated", "deployed",
//! "executed", "ran the command", "applied the patch", "saved",
//! "committed", "pushed", "sent the message", "Done — …".
//!
//! Past-tense + first-person + side-effect verb is the heuristic. We
//! deliberately do NOT flag *future* claims ("I'll write…", "let me
//! run…") — those are the model planning, not lying.
//!
//! ## What counts as backed
//!
//! At least one tool call of these kinds completed (success OR failure
//! — a failed Bash still proves the model tried) in the most recent
//! assistant message:
//!
//!   `Write`, `Edit`, `MultiEdit`, `Bash`, `ApplyPatch`,
//!   `SendMessage`, `MemoryCreate`, `MemoryDelete`, `Task`,
//!   `TeamCreate`, `TeamDelete`, `NotebookEdit`,
//!   `EnterWorktree`, `ExitWorktree`, `RemoteTrigger`,
//!   `CronCreate`, `CronDelete`, `PushNotification`,
//!   `SymbolEdit`, `RunCoverage`, `RunBounty`, `PostBounty`.
//!
//! Read-only tools (`Read`, `Glob`, `Grep`, `WebSearch`, `WebFetch`,
//! `GraphQuery`, `TaskList`, `TaskGet`, `Skill`) do NOT count — the
//! model can claim "wrote the file" while only reading.

use crate::types::{ChatMessage, MessagePart, Role, ToolKind, ToolStatus};

/// Conservative list of past-tense side-effect verbs / completion
/// phrases. Each entry is the substring (lowercased, ASCII) we look
/// for. Order doesn't matter; the scanner short-circuits on first
/// match.
///
/// Kept as a fixed array rather than a regex so the scan is
/// O(n × m) with tiny constants — m ≈ 20, n ≈ a few KB of assistant
/// text. A regex with `|` would still need to be ASCII-folded to match
/// "Updated" / "UPDATED" / etc., so the linear scan is no worse.
const CLAIM_PHRASES: &[&str] = &[
    // Direct past-tense first-person actions.
    "i wrote",
    "i've written",
    "i have written",
    "i created",
    "i've created",
    "i have created",
    "i updated",
    "i've updated",
    "i have updated",
    "i edited",
    "i've edited",
    "i have edited",
    "i applied",
    "i've applied",
    "i have applied",
    "i ran",
    "i've run",
    "i have run",
    "i executed",
    "i've executed",
    "i have executed",
    "i deployed",
    "i've deployed",
    "i have deployed",
    "i sent",
    "i've sent",
    "i have sent",
    "i committed",
    "i've committed",
    "i have committed",
    "i pushed",
    "i've pushed",
    "i have pushed",
    "i saved",
    "i've saved",
    "i have saved",
    "i deleted",
    "i've deleted",
    "i have deleted",
    "i removed",
    "i've removed",
    "i have removed",
    "i moved",
    "i've moved",
    "i renamed",
    "i've renamed",
    "i refactored",
    "i've refactored",
    // Bare completion announcements at the start of a sentence.
    "done — ",
    "done. ",
    "done!",
    "all done",
    "the file has been written",
    "the file has been created",
    "the file has been updated",
    "the change has been applied",
    "the patch has been applied",
    "the commit has been pushed",
    "the deploy has",
    // "Now writing for real" style retries — explicit admission that
    // the previous claim was bogus, followed by another unbacked claim.
    "writing for real now",
    "actually writing now",
    "let me write that for real",
];

/// Side-effect tool kinds. A successful or failed dispatch of any of
/// these in the current assistant message backs a completion claim.
fn is_side_effect(kind: &ToolKind) -> bool {
    matches!(
        kind,
        ToolKind::Write
            | ToolKind::Edit
            | ToolKind::MultiEdit
            | ToolKind::Bash
            | ToolKind::ApplyPatch
            | ToolKind::SendMessage
            | ToolKind::MemoryCreate
            | ToolKind::MemoryDelete
            | ToolKind::Task
            | ToolKind::TeamCreate
            | ToolKind::TeamDelete
            | ToolKind::NotebookEdit
            | ToolKind::EnterWorktree
            | ToolKind::ExitWorktree
            | ToolKind::RemoteTrigger
            | ToolKind::CronCreate
            | ToolKind::CronDelete
            | ToolKind::PushNotification
            | ToolKind::SymbolEdit
            | ToolKind::RunCoverage
            | ToolKind::RunBounty
            | ToolKind::PostBounty
    )
}

/// True iff `msg` is an assistant message containing at least one
/// resolved (Completed / Failed) side-effect tool call. Pending /
/// Running tools do not count — they may still be cancelled.
pub fn has_backing_tool(msg: &ChatMessage) -> bool {
    if !matches!(msg.role, Role::Assistant) {
        return false;
    }
    msg.parts.iter().any(|p| match p {
        MessagePart::Tool(tc) => {
            is_side_effect(&tc.kind)
                && matches!(tc.status, ToolStatus::Completed | ToolStatus::Failed)
        }
        _ => false,
    })
}

/// Concatenate all `MessagePart::Text` content in `msg`, lowercased and
/// ASCII-stripped of common punctuation so the phrase scanner can match
/// "Done!" / "Done." / "done — " uniformly.
fn assistant_text_lowercase(msg: &ChatMessage) -> String {
    msg.parts
        .iter()
        .filter_map(|p| match p {
            MessagePart::Text(t) => Some(t.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
        .to_ascii_lowercase()
}

/// True iff the assistant text contains at least one phrase from
/// `CLAIM_PHRASES`. Returns the matched phrase so callers can log /
/// surface it in the system-reminder body.
pub fn detect_claim_phrase(msg: &ChatMessage) -> Option<&'static str> {
    let text = assistant_text_lowercase(msg);
    CLAIM_PHRASES.iter().copied().find(|p| text.contains(p))
}

/// Top-level guard: returns `Some(matched_phrase)` if the assistant
/// message makes a side-effect claim that is NOT backed by a completed
/// side-effect tool call. Caller should append a system-reminder and
/// restart the agentic loop.
///
/// Returns `None` when:
/// - the message is not an assistant message
/// - no claim phrase is present
/// - a backing tool call exists
pub fn check_unbacked_claim(msg: &ChatMessage) -> Option<&'static str> {
    if !matches!(msg.role, Role::Assistant) {
        return None;
    }
    if has_backing_tool(msg) {
        return None;
    }
    detect_claim_phrase(msg)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::ToolId;
    use crate::types::{ChatMessage, MessagePart, ToolCall, ToolDisplayState, ToolInput, ToolOutput};

    fn assistant(text: &str) -> ChatMessage {
        let mut m = ChatMessage::assistant(text.to_owned());
        m.parts = vec![MessagePart::Text(text.to_owned())];
        m
    }

    fn assistant_with_tool(text: &str, kind: ToolKind, status: ToolStatus) -> ChatMessage {
        let mut m = ChatMessage::assistant(String::new());
        m.parts = vec![
            MessagePart::Text(text.to_owned()),
            MessagePart::Tool(ToolCall {
                id: ToolId::from("t1"),
                kind,
                status,
                input: ToolInput::Generic {
                    summary: "x".into(),
                },
                output: ToolOutput::Empty,
                display: ToolDisplayState::DEFAULT,
                elapsed_ms: None,
                started_at: None,
            }),
        ];
        m
    }

    // Normal: bare "Done." with no tool → unbacked.
    #[test]
    fn unbacked_done_normal() {
        let m = assistant("Done. The file has been updated.");
        assert!(check_unbacked_claim(&m).is_some());
    }

    // Normal: "I wrote the file" with a successful Write → backed.
    #[test]
    fn backed_write_normal() {
        let m = assistant_with_tool(
            "I wrote the file.",
            ToolKind::Write,
            ToolStatus::Completed,
        );
        assert!(check_unbacked_claim(&m).is_none());
    }

    // Robust: "I wrote the file" with a FAILED Write still counts as
    // backed — the model tried, and the user can see the failure. We
    // don't want to nag the model into retrying when the actual
    // failure is more informative.
    #[test]
    fn backed_failed_write_robust() {
        let m = assistant_with_tool("I wrote the file.", ToolKind::Write, ToolStatus::Failed);
        assert!(check_unbacked_claim(&m).is_none());
    }

    // Robust: a Pending tool does NOT back a claim — the tool might
    // never run (cancelled, denied), so the claim is still suspect.
    #[test]
    fn pending_tool_does_not_back_robust() {
        let m = assistant_with_tool("I wrote the file.", ToolKind::Write, ToolStatus::Pending);
        assert!(check_unbacked_claim(&m).is_some());
    }

    // Robust: read-only tools (Read/Grep/Glob) do NOT back a write
    // claim. Model can verify a file existed without actually writing
    // it.
    #[test]
    fn read_only_tool_does_not_back_robust() {
        let m = assistant_with_tool("I created the file.", ToolKind::Read, ToolStatus::Completed);
        assert!(check_unbacked_claim(&m).is_some());
    }

    // Normal: future-tense claims are NOT flagged. "I'll write…" is
    // the model planning, not lying.
    #[test]
    fn future_tense_not_flagged_normal() {
        let m = assistant("I'll write the file now.");
        assert!(check_unbacked_claim(&m).is_none());
        let m2 = assistant("Let me update the config.");
        assert!(check_unbacked_claim(&m2).is_none());
    }

    // Robust: case-insensitive — "DONE." and "Done." and "done."
    // all flag.
    #[test]
    fn case_insensitive_robust() {
        for s in ["DONE!", "Done!", "done!"] {
            let m = assistant(s);
            assert!(check_unbacked_claim(&m).is_some(), "{s}");
        }
    }

    // Normal: "Writing for real now" is the model retrying after a
    // previously-unbacked claim — flag it explicitly so the loop
    // catches the retry instead of letting it slide.
    #[test]
    fn writing_for_real_now_flagged_normal() {
        let m = assistant("Writing for real now.");
        assert!(check_unbacked_claim(&m).is_some());
    }

    // Robust: messages with no side-effect verbiage don't flag.
    #[test]
    fn benign_text_not_flagged_robust() {
        let m = assistant("Here's how the code flows: the user submits → handle_submit fires → …");
        assert!(check_unbacked_claim(&m).is_none());
    }

    // Robust: user messages are never flagged (only assistant text
    // counts).
    #[test]
    fn user_message_not_flagged_robust() {
        let m = ChatMessage::user("I wrote the file.".into());
        assert!(check_unbacked_claim(&m).is_none());
    }
}

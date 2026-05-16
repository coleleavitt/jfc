//! Faithfulness guard: detect when assistant prose claims a side
//! effect that isn't backed by a matching tool call.
//!
//! ## Research grounding
//!
//! This module implements a lightweight runtime variant of what the
//! agentic-LLM literature calls **faithfulness assurance** and
//! **outcome-evidence verification**. Two papers shaped the design:
//!
//! - *Replayable Financial Agents: A Determinism-Faithfulness Assurance
//!   Harness for Tool-Using LLM Agents* (Khatchadourian, arXiv:
//!   2601.15322) — formalizes faithfulness as "the agent's reported
//!   actions match the actions actually executed by its tools."
//!
//! - *Can Agent Benchmarks Support Their Scores? Evidence-Supported
//!   Bounds for Interactive-Agent Evaluation* (Gao & Zhou, arXiv:
//!   2605.10448) — argues that outcome checks need a **locked checklist
//!   of required stored artifacts** and a **three-state output** (Pass /
//!   Fail / **Unknown**) rather than binary success. Hiding Unknown
//!   cases inside aggregate scores produces misleading reports; we
//!   adopt the same trichotomy at runtime.
//!
//! ## Design v2 (improvements over v1)
//!
//! v1 was a flat phrase list with a binary backed/unbacked output. It
//! flagged "I wrote down the plan" (a legitimate self-reference) the
//! same as "I wrote the file" (a side-effect claim). v2 distinguishes:
//!
//! 1. **Claim categories**: each phrase maps to a category
//!    (`WroteFile`, `RanCommand`, `Deployed`, `SentMessage`,
//!    `EditedCode`, `Committed`, `Pushed`, `Generic`).
//! 2. **Per-category backing tools**: each category has an explicit
//!    allow-list of `ToolKind`s that satisfy it. A `Write` backs
//!    `WroteFile` but not `Committed`; a `Bash` backs `RanCommand` and
//!    `Committed` and `Pushed` (since git commands run via Bash).
//! 3. **Three-state verdict**: `Backed` (claim → tool match found),
//!    `Unbacked` (claim found, no satisfying tool), `Ambiguous` (claim
//!    found, OR'd backing tool present in turn AND a "scratch"
//!    qualifier like "I wrote down" / "I noted" appears near the
//!    phrase).
//! 4. **Conservative trigger**: only `Unbacked` re-runs the turn.
//!    `Ambiguous` surfaces a quieter info toast and lets the turn
//!    finalize — research literature explicitly warns that hiding
//!    Unknown cases produces worse outcomes than logging them.
//!
//! ## Negative-list qualifiers (anti-false-positive)
//!
//! Several phrases would false-positive on v1. v2 strips them from the
//! match space:
//!
//! - "I wrote down" / "I noted" / "I jotted" — scratchpad reference
//! - "wrote the plan" / "wrote the steps" / "wrote a TaskCreate" —
//!   model describing its own planning output, not file I/O
//! - "wrote that the / wrote that …" — quoted speech about what the
//!   user wrote
//! - "I've already" / "previously" — referring to past turns where
//!   the action DID happen and the artifact is in history
//!
//! ## Disable
//!
//! `JFC_DISABLE_HALLUCINATION_GUARD=1` turns the whole guard off.
//! `JFC_HALLUCINATION_GUARD_LOG_ONLY=1` keeps detection running but
//! never re-runs the turn (logs + toast only) — useful for tuning the
//! pattern set against real workloads without disrupting the user.

use crate::types::{ChatMessage, MessagePart, Role, ToolKind, ToolStatus};

/// Semantic category of a completion claim. Each variant maps to an
/// allow-list of `ToolKind`s in `category_backed`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClaimCategory {
    /// "wrote the file", "created config.toml", "saved the patch"
    WroteFile,
    /// "ran the test", "executed cargo build", "kicked off …"
    RanCommand,
    /// "deployed", "shipped to prod"
    Deployed,
    /// "sent the message", "notified the team"
    SentMessage,
    /// "edited", "updated the line", "refactored"
    EditedCode,
    /// "committed the change" — git-specific, almost always Bash
    Committed,
    /// "pushed", "pushed to origin" — git-specific
    Pushed,
    /// "Done.", "All done", "Done!" — generic completion announcement;
    /// any side-effect tool counts as backing
    Generic,
}

/// Mapping of category → allowed backing `ToolKind`s. A claim is
/// `Backed` iff at least one tool of these kinds completed (success or
/// failure — a failed Bash still proves the model tried) on the
/// current assistant turn.
fn category_backed(cat: ClaimCategory, kind: &ToolKind) -> bool {
    use ToolKind::*;
    match cat {
        ClaimCategory::WroteFile => {
            matches!(kind, Write | Edit | MultiEdit | NotebookEdit | ApplyPatch)
        }
        ClaimCategory::RanCommand => {
            matches!(kind, Bash | Task | RunCoverage | RunBounty | PostBounty)
        }
        ClaimCategory::Deployed => matches!(kind, Bash | RemoteTrigger | CronCreate),
        ClaimCategory::SentMessage => {
            matches!(kind, SendMessage | PushNotification | RemoteTrigger)
        }
        ClaimCategory::EditedCode => {
            matches!(kind, Edit | MultiEdit | Write | SymbolEdit | ApplyPatch)
        }
        ClaimCategory::Committed => matches!(kind, Bash),
        ClaimCategory::Pushed => matches!(kind, Bash),
        ClaimCategory::Generic => matches!(
            kind,
            Write
                | Edit
                | MultiEdit
                | Bash
                | ApplyPatch
                | SendMessage
                | MemoryCreate
                | MemoryDelete
                | Task
                | TeamCreate
                | TeamDelete
                | NotebookEdit
                | EnterWorktree
                | ExitWorktree
                | RemoteTrigger
                | CronCreate
                | CronDelete
                | PushNotification
                | SymbolEdit
                | RunCoverage
                | RunBounty
                | PostBounty
        ),
    }
}

/// (phrase, category) pairs. Phrases are pre-lowercased and matched
/// via substring. Order matters: earlier (more specific) phrases match
/// first so e.g. "i committed" routes to Committed before falling
/// through to a generic match.
const CLAIM_PHRASES: &[(&str, ClaimCategory)] = &[
    // Committed — git-specific.
    ("i committed", ClaimCategory::Committed),
    ("i've committed", ClaimCategory::Committed),
    ("the commit has been pushed", ClaimCategory::Pushed),
    // Pushed — git-specific.
    ("i pushed", ClaimCategory::Pushed),
    ("i've pushed", ClaimCategory::Pushed),
    ("i have pushed", ClaimCategory::Pushed),
    // Deployed.
    ("i deployed", ClaimCategory::Deployed),
    ("i've deployed", ClaimCategory::Deployed),
    ("i have deployed", ClaimCategory::Deployed),
    ("the deploy has", ClaimCategory::Deployed),
    // Sent.
    ("i sent", ClaimCategory::SentMessage),
    ("i've sent", ClaimCategory::SentMessage),
    ("i have sent", ClaimCategory::SentMessage),
    ("notified the team", ClaimCategory::SentMessage),
    // Ran command.
    ("i ran the", ClaimCategory::RanCommand),
    ("i've run the", ClaimCategory::RanCommand),
    ("i have run the", ClaimCategory::RanCommand),
    ("i executed", ClaimCategory::RanCommand),
    ("i've executed", ClaimCategory::RanCommand),
    ("i have executed", ClaimCategory::RanCommand),
    // Wrote/created — file-targeted phrases (most specific first).
    ("i wrote the file", ClaimCategory::WroteFile),
    ("i wrote the config", ClaimCategory::WroteFile),
    ("i created the file", ClaimCategory::WroteFile),
    ("the file has been written", ClaimCategory::WroteFile),
    ("the file has been created", ClaimCategory::WroteFile),
    ("the file has been updated", ClaimCategory::WroteFile),
    // Edited code.
    ("i edited", ClaimCategory::EditedCode),
    ("i've edited", ClaimCategory::EditedCode),
    ("i have edited", ClaimCategory::EditedCode),
    ("i applied", ClaimCategory::EditedCode),
    ("i've applied", ClaimCategory::EditedCode),
    ("the change has been applied", ClaimCategory::EditedCode),
    ("the patch has been applied", ClaimCategory::EditedCode),
    ("i refactored", ClaimCategory::EditedCode),
    ("i've refactored", ClaimCategory::EditedCode),
    // Generic "wrote" without file context — could be scratch or
    // could be file I/O. Falls through to Generic; ambiguity caught
    // by the negative-list pass.
    ("i wrote", ClaimCategory::WroteFile),
    ("i've written", ClaimCategory::WroteFile),
    ("i have written", ClaimCategory::WroteFile),
    ("i created", ClaimCategory::WroteFile),
    ("i've created", ClaimCategory::WroteFile),
    ("i have created", ClaimCategory::WroteFile),
    ("i updated", ClaimCategory::EditedCode),
    ("i've updated", ClaimCategory::EditedCode),
    ("i have updated", ClaimCategory::EditedCode),
    ("i saved", ClaimCategory::WroteFile),
    ("i've saved", ClaimCategory::WroteFile),
    ("i have saved", ClaimCategory::WroteFile),
    ("i deleted", ClaimCategory::EditedCode),
    ("i've deleted", ClaimCategory::EditedCode),
    ("i removed", ClaimCategory::EditedCode),
    ("i've removed", ClaimCategory::EditedCode),
    ("i moved", ClaimCategory::EditedCode),
    ("i've moved", ClaimCategory::EditedCode),
    ("i renamed", ClaimCategory::EditedCode),
    ("i've renamed", ClaimCategory::EditedCode),
    // Bare completion announcements at the start of a sentence.
    // Generic category: ANY side-effect tool counts as backing.
    ("done — ", ClaimCategory::Generic),
    ("done. ", ClaimCategory::Generic),
    ("done!", ClaimCategory::Generic),
    ("all done", ClaimCategory::Generic),
    // Retry-after-bogus pattern.
    ("writing for real now", ClaimCategory::WroteFile),
    ("actually writing now", ClaimCategory::WroteFile),
    ("let me write that for real", ClaimCategory::WroteFile),
];

/// Phrases that, if they appear in the same message as a claim,
/// downgrade the verdict from `Unbacked` to `Ambiguous`. These are
/// patterns where "I wrote" / "I created" legitimately refers to
/// non-file output (planning text, scratch notes, the assistant's own
/// reasoning summary).
const NEGATIVE_QUALIFIERS: &[&str] = &[
    "wrote down",
    "wrote out",
    "noted",
    "i noted",
    "jotted",
    "wrote the plan",
    "wrote a plan",
    "wrote the steps",
    "wrote a summary",
    "wrote up",
    "wrote that the user",
    "wrote that you",
    "i've already",
    "previously",
    "earlier i",
    "earlier in this conversation",
    // Model describing what it just SAID, not what it did.
    "i'm writing",
    "as i wrote above",
    "as i mentioned",
    "to recap",
    // TaskCreate-style planning (we have a dedicated tool for it,
    // and the claim "I created N tasks" is backed by TaskCreate).
    "wrote a taskcreate",
    "created the task",
    "added a task",
    "queued",
];

/// Three-state verdict for a single assistant message. Maps onto the
/// "Pass / Fail / Unknown" framing from arXiv:2605.10448.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FaithfulnessVerdict {
    /// No claim phrase found, or a matching backing tool ran.
    Backed,
    /// Claim found, no backing tool. Caller should re-run the turn.
    Unbacked {
        phrase: &'static str,
        category: ClaimCategory,
    },
    /// Claim found, but either a related (not strict-match) tool ran
    /// OR a negative-qualifier appears nearby. Caller should toast +
    /// log but not re-run.
    Ambiguous {
        phrase: &'static str,
        category: ClaimCategory,
        reason: &'static str,
    },
}

/// True iff the tool call is a completed/failed instance of `kind`.
/// Pending/Running tools do NOT count — they might still be cancelled.
fn is_resolved(status: ToolStatus) -> bool {
    matches!(status, ToolStatus::Completed | ToolStatus::Failed)
}

fn push_resolved_tool_kinds(msg: &ChatMessage, out: &mut Vec<ToolKind>) {
    if msg.role != Role::Assistant {
        return;
    }
    for p in &msg.parts {
        if let MessagePart::Tool(tc) = p {
            if is_resolved(tc.status)
                && !out
                    .iter()
                    .any(|k| std::mem::discriminant(k) == std::mem::discriminant(&tc.kind))
            {
                out.push(tc.kind.clone());
            }
        }
    }
}

/// All `ToolKind`s of completed/failed tools in `msg`, deduplicated.
fn resolved_tool_kinds(msg: &ChatMessage) -> Vec<ToolKind> {
    let mut out = Vec::new();
    push_resolved_tool_kinds(msg, &mut out);
    out
}

/// Concatenate all text parts, lowercased and ASCII-normalized for
/// substring matching.
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

/// Find the first (phrase, category) match in `text`. Returns `None`
/// if no claim phrase appears.
fn detect_claim(text: &str) -> Option<(&'static str, ClaimCategory)> {
    CLAIM_PHRASES
        .iter()
        .copied()
        .find(|(phrase, _)| text.contains(phrase))
}

/// True iff any negative-qualifier phrase appears in `text`. Used to
/// downgrade an Unbacked verdict to Ambiguous.
fn has_negative_qualifier(text: &str) -> bool {
    NEGATIVE_QUALIFIERS.iter().any(|q| text.contains(q))
}

fn evaluate_text_against_kinds(text: &str, kinds: &[ToolKind]) -> FaithfulnessVerdict {
    let Some((phrase, category)) = detect_claim(text) else {
        return FaithfulnessVerdict::Backed;
    };
    let strict_backed = kinds.iter().any(|k| category_backed(category, k));
    if strict_backed {
        return FaithfulnessVerdict::Backed;
    }
    // No strict match. Check the two ambiguity escape hatches:
    //   1. Negative qualifier nearby ("wrote down", "noted", etc.)
    //   2. Some OTHER side-effect tool ran (Generic-category backing)
    //      — model may have used a sibling tool that satisfies the
    //      intent even if not the literal category.
    if has_negative_qualifier(text) {
        return FaithfulnessVerdict::Ambiguous {
            phrase,
            category,
            reason: "negative-qualifier (\"wrote down\", \"noted\", \"previously\", etc.) nearby",
        };
    }
    let any_side_effect = kinds
        .iter()
        .any(|k| category_backed(ClaimCategory::Generic, k));
    if any_side_effect {
        return FaithfulnessVerdict::Ambiguous {
            phrase,
            category,
            reason: "claim category didn't match, but SOME side-effect tool ran this turn",
        };
    }
    FaithfulnessVerdict::Unbacked { phrase, category }
}

/// Evaluate a single assistant message and return a faithfulness
/// verdict. The caller (event_loop's StreamDone handler) uses the
/// verdict to decide: do nothing (`Backed`), toast + log
/// (`Ambiguous`), or inject a system-reminder and re-run the turn
/// (`Unbacked`).
///
/// Returns `FaithfulnessVerdict::Backed` for non-assistant messages so
/// callers don't need to check role separately.
pub fn evaluate(msg: &ChatMessage) -> FaithfulnessVerdict {
    if !matches!(msg.role, Role::Assistant) {
        return FaithfulnessVerdict::Backed;
    }
    let text = assistant_text_lowercase(msg);
    let kinds = resolved_tool_kinds(msg);
    evaluate_text_against_kinds(&text, &kinds)
}

/// Evaluate the assistant message at `assistant_idx` using all resolved
/// tool calls in the same logical user turn as backing evidence.
///
/// Agentic loops are persisted as several consecutive assistant messages:
/// one per sub-stream. The final sub-stream may say "Done" while the
/// backing Write/Edit/Bash tool lives in an earlier assistant message from
/// the same user turn. Checking only the final `ChatMessage` false-positives
/// and asks the model to redo work it already performed.
pub fn evaluate_turn(messages: &[ChatMessage], assistant_idx: usize) -> FaithfulnessVerdict {
    let Some(msg) = messages.get(assistant_idx) else {
        return FaithfulnessVerdict::Backed;
    };
    if !matches!(msg.role, Role::Assistant) {
        return FaithfulnessVerdict::Backed;
    }
    let text = assistant_text_lowercase(msg);
    let start = messages[..assistant_idx]
        .iter()
        .rposition(|m| m.role == Role::User)
        .map_or(0, |idx| idx + 1);
    let mut kinds = Vec::new();
    for m in &messages[start..=assistant_idx] {
        push_resolved_tool_kinds(m, &mut kinds);
    }
    evaluate_text_against_kinds(&text, &kinds)
}

/// Compatibility shim — older callers expect a `Option<&'static str>`.
/// `Some(phrase)` means we want a re-run; `None` means Backed or
/// Ambiguous (no re-run).
pub fn check_unbacked_claim(msg: &ChatMessage) -> Option<&'static str> {
    match evaluate(msg) {
        FaithfulnessVerdict::Unbacked { phrase, .. } => Some(phrase),
        _ => None,
    }
}

/// True iff the user has opted in to log-only mode via
/// `JFC_HALLUCINATION_GUARD_LOG_ONLY=1` — detection still runs but
/// re-running the turn is suppressed. Useful for tuning the patterns
/// against real workloads without nagging the user.
pub fn log_only_mode() -> bool {
    matches!(
        std::env::var("JFC_HALLUCINATION_GUARD_LOG_ONLY").as_deref(),
        Ok("1") | Ok("true")
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::ToolId;
    use crate::types::{
        ChatMessage, MessagePart, ToolCall, ToolDisplayState, ToolInput, ToolOutput,
    };

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

    // Normal: bare "Done." with no tool → Unbacked.
    #[test]
    fn unbacked_done_normal() {
        let m = assistant("Done. The file has been updated.");
        match evaluate(&m) {
            FaithfulnessVerdict::Unbacked { .. } => {}
            v => panic!("expected Unbacked, got {v:?}"),
        }
    }

    // Normal: "I wrote the file" with a successful Write → Backed.
    #[test]
    fn backed_write_normal() {
        let m = assistant_with_tool("I wrote the file.", ToolKind::Write, ToolStatus::Completed);
        assert_eq!(evaluate(&m), FaithfulnessVerdict::Backed);
    }

    // Normal: failed Write still backs the claim (model tried).
    #[test]
    fn backed_failed_write_normal() {
        let m = assistant_with_tool("I wrote the file.", ToolKind::Write, ToolStatus::Failed);
        assert_eq!(evaluate(&m), FaithfulnessVerdict::Backed);
    }

    // Robust: Pending tool does NOT back a claim.
    #[test]
    fn pending_tool_does_not_back_robust() {
        let m = assistant_with_tool("I wrote the file.", ToolKind::Write, ToolStatus::Pending);
        match evaluate(&m) {
            FaithfulnessVerdict::Unbacked { .. } => {}
            v => panic!("expected Unbacked, got {v:?}"),
        }
    }

    // Robust: read-only tool with a write claim → Ambiguous (Generic
    // category doesn't match Read either, but neither does a strict
    // WroteFile category). Read is NOT in the Generic side-effect
    // list, so this falls through to Unbacked. Pin that explicitly.
    #[test]
    fn read_only_tool_with_write_claim_is_unbacked_robust() {
        let m = assistant_with_tool("I created the file.", ToolKind::Read, ToolStatus::Completed);
        match evaluate(&m) {
            FaithfulnessVerdict::Unbacked { .. } => {}
            v => panic!("expected Unbacked, got {v:?}"),
        }
    }

    // Normal: future-tense claims are NOT flagged.
    #[test]
    fn future_tense_not_flagged_normal() {
        let m = assistant("I'll write the file now.");
        assert_eq!(evaluate(&m), FaithfulnessVerdict::Backed);
        let m2 = assistant("Let me update the config.");
        assert_eq!(evaluate(&m2), FaithfulnessVerdict::Backed);
    }

    // Robust: case-insensitive.
    #[test]
    fn case_insensitive_robust() {
        for s in ["DONE!", "Done!", "done!"] {
            let m = assistant(s);
            match evaluate(&m) {
                FaithfulnessVerdict::Unbacked { .. } => {}
                v => panic!("{s}: expected Unbacked, got {v:?}"),
            }
        }
    }

    // Normal: "Writing for real now" — retry after bogus claim.
    #[test]
    fn writing_for_real_now_flagged_normal() {
        let m = assistant("Writing for real now.");
        match evaluate(&m) {
            FaithfulnessVerdict::Unbacked { .. } => {}
            v => panic!("expected Unbacked, got {v:?}"),
        }
    }

    // Robust: benign text doesn't flag.
    #[test]
    fn benign_text_not_flagged_robust() {
        let m = assistant("Here's how the code flows: the user submits → handle_submit fires.");
        assert_eq!(evaluate(&m), FaithfulnessVerdict::Backed);
    }

    // Robust: user-role messages never flagged.
    #[test]
    fn user_message_not_flagged_robust() {
        let m = ChatMessage::user("I wrote the file.".into());
        assert_eq!(evaluate(&m), FaithfulnessVerdict::Backed);
    }

    // v2-specific: "I wrote down the plan" → Ambiguous (negative
    // qualifier "wrote down" present), not Unbacked.
    #[test]
    fn wrote_down_is_ambiguous_v2_normal() {
        let m = assistant("I wrote down the plan in TaskCreate format.");
        match evaluate(&m) {
            FaithfulnessVerdict::Ambiguous { .. } => {}
            v => panic!("expected Ambiguous, got {v:?}"),
        }
    }

    // v2-specific: a "wrote the file" claim with a TaskCreate tool ran
    // → Ambiguous (some side-effect ran, but not the strict-match
    // category). User can see TaskCreate landed but model said
    // "wrote the file" which is misleading.
    #[test]
    fn write_claim_with_taskcreate_is_ambiguous_v2_normal() {
        let m = assistant_with_tool(
            "I wrote the file.",
            ToolKind::TaskCreate,
            ToolStatus::Completed,
        );
        // TaskCreate is NOT in the Generic backing list (it's
        // read-only-ish — pure data structure mutation, no file I/O).
        // So this is Unbacked, not Ambiguous.
        match evaluate(&m) {
            FaithfulnessVerdict::Unbacked { .. } => {}
            v => panic!("expected Unbacked, got {v:?}"),
        }
    }

    // v2-specific: "I committed" with a Bash tool → Backed.
    #[test]
    fn committed_with_bash_is_backed_v2_normal() {
        let m = assistant_with_tool(
            "I committed the change.",
            ToolKind::Bash,
            ToolStatus::Completed,
        );
        assert_eq!(evaluate(&m), FaithfulnessVerdict::Backed);
    }

    // v2-specific: "I committed" with a Write tool → Ambiguous (Write
    // is a side-effect but not the strict Committed→Bash mapping).
    #[test]
    fn committed_with_write_is_ambiguous_v2_normal() {
        let m = assistant_with_tool(
            "I committed the change.",
            ToolKind::Write,
            ToolStatus::Completed,
        );
        match evaluate(&m) {
            FaithfulnessVerdict::Ambiguous { .. } => {}
            v => panic!("expected Ambiguous, got {v:?}"),
        }
    }

    // v2-specific: "Done." with ANY side-effect tool → Backed via
    // Generic category.
    #[test]
    fn done_with_any_side_effect_is_backed_v2_normal() {
        for kind in [
            ToolKind::Write,
            ToolKind::Edit,
            ToolKind::Bash,
            ToolKind::SendMessage,
        ] {
            let m = assistant_with_tool("Done!", kind.clone(), ToolStatus::Completed);
            assert_eq!(
                evaluate(&m),
                FaithfulnessVerdict::Backed,
                "Done with {kind:?} should be Backed"
            );
        }
    }

    // Normal: JFC persists one logical tool loop as several assistant
    // sub-stream messages. The final message can say "Done" while the
    // backing tool lives in an earlier assistant message from the same
    // user turn; this must be Backed, not a spurious redo.
    #[test]
    fn evaluate_turn_uses_prior_assistant_tools_in_same_user_turn_normal() {
        let messages = vec![
            ChatMessage::user("write the file".into()),
            assistant_with_tool("", ToolKind::Write, ToolStatus::Completed),
            assistant("Done. The file has been written."),
        ];
        match evaluate(&messages[2]) {
            FaithfulnessVerdict::Unbacked { .. } => {}
            v => panic!("expected single-message check to be Unbacked, got {v:?}"),
        }
        assert_eq!(evaluate_turn(&messages, 2), FaithfulnessVerdict::Backed);
    }

    // Robust: prior-turn tools must not back a new user turn's fresh claim.
    #[test]
    fn evaluate_turn_does_not_cross_user_boundary_robust() {
        let messages = vec![
            ChatMessage::user("write the file".into()),
            assistant_with_tool("", ToolKind::Write, ToolStatus::Completed),
            assistant("Done."),
            ChatMessage::user("do another write".into()),
            assistant("Done. The file has been written."),
        ];
        match evaluate_turn(&messages, 4) {
            FaithfulnessVerdict::Unbacked { .. } => {}
            v => panic!("expected Unbacked, got {v:?}"),
        }
    }

    // v2-specific: "I updated …" + "I've already" in the same message
    // → Ambiguous. We need both the claim phrase AND the negative
    // qualifier; just "I've already updated" doesn't contain "I
    // updated" as a substring, so test the actual interaction.
    #[test]
    fn already_qualifier_downgrades_v2_normal() {
        let m = assistant("I updated that file. I've already done this in the previous turn.");
        match evaluate(&m) {
            FaithfulnessVerdict::Ambiguous { .. } => {}
            v => panic!("expected Ambiguous, got {v:?}"),
        }
    }

    // Robust: "I've already updated" alone (no separate "i updated"
    // substring) → Backed because no claim phrase matches. This is
    // expected behavior — substring matching is intentionally
    // conservative.
    #[test]
    fn already_updated_alone_is_backed_robust() {
        let m = assistant("I've already updated that in the previous turn.");
        assert_eq!(
            evaluate(&m),
            FaithfulnessVerdict::Backed,
            "no claim phrase matches \"i've already updated\" alone"
        );
    }

    // v2-specific: check_unbacked_claim shim returns Some only for
    // Unbacked, not Ambiguous.
    #[test]
    fn shim_returns_some_only_for_unbacked_v2_normal() {
        let unbacked = assistant("Done. The file has been written.");
        assert!(check_unbacked_claim(&unbacked).is_some());

        let ambiguous = assistant("I wrote down the plan.");
        assert!(
            check_unbacked_claim(&ambiguous).is_none(),
            "Ambiguous should NOT trigger re-run"
        );
    }

    // v2-specific: log_only_mode reads env var.
    #[test]
    #[serial_test::serial]
    fn log_only_mode_reads_env_v2_normal() {
        // SAFETY: env var manipulation serialized by #[serial].
        unsafe {
            std::env::set_var("JFC_HALLUCINATION_GUARD_LOG_ONLY", "1");
        }
        assert!(log_only_mode());
        unsafe {
            std::env::set_var("JFC_HALLUCINATION_GUARD_LOG_ONLY", "true");
        }
        assert!(log_only_mode());
        unsafe {
            std::env::remove_var("JFC_HALLUCINATION_GUARD_LOG_ONLY");
        }
        assert!(!log_only_mode());
    }
}

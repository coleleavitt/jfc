use super::tool_call::ToolCall;
use super::tool_display::ToolDisplayState;
use super::tool_output::ToolOutput;
use super::{
    ChatMessage, MessagePart, ReplacementMode, ToolInput, ToolKind, ToolStatus, parse_unified_diff,
};
#[cfg(test)]
use super::{
    ModelUsage, Role, TaskLifecycle, TaskStatusPart, TurnInvariantError, parse_hunk_header,
    parse_hunk_start, truncate_lines, validate_turn_invariants, validate_turn_invariants_inner,
};

#[cfg(test)]
mod cumulative_usage_tests {
    use super::ModelUsage;

    #[test]
    fn cumulative_deltas_dont_triple_count_normal() {
        // Anthropic streams 5 message_delta events for a single turn,
        // each carrying the running output_tokens count. Naive add_delta
        // produces 1+5+15+50+200 = 271; correct answer is the final 200.
        let mut u = ModelUsage::default();
        let mut baseline = (0u32, 0, 0, 0);
        for cum in [
            (100, 1, 0, 0),
            (100, 5, 0, 0),
            (100, 15, 0, 0),
            (100, 50, 0, 0),
            (100, 200, 0, 0),
        ] {
            baseline = u.apply_cumulative(cum, baseline);
        }
        assert_eq!(u.input_tokens, 100, "input shouldn't double-count");
        assert_eq!(u.output_tokens, 200, "output should be final cumulative");
    }

    #[test]
    fn second_turn_resets_baseline_normal() {
        // Each new turn the caller resets baseline to (0,0,0,0); the
        // function then correctly attributes the full new turn's count.
        let mut u = ModelUsage::default();
        let _ = u.apply_cumulative((100, 50, 0, 0), (0, 0, 0, 0));
        // Turn 2: caller passes baseline = (0,0,0,0) again
        let _ = u.apply_cumulative((80, 30, 0, 0), (0, 0, 0, 0));
        assert_eq!(u.input_tokens, 180, "two turns add: 100 + 80");
        assert_eq!(u.output_tokens, 80, "two turns add: 50 + 30");
    }

    #[test]
    fn no_op_when_cumulative_unchanged_robust() {
        // Some providers emit redundant usage events with the same count.
        // The apply should be a no-op (no double-charge, baseline unchanged).
        let mut u = ModelUsage::default();
        let b1 = u.apply_cumulative((100, 50, 0, 0), (0, 0, 0, 0));
        let b2 = u.apply_cumulative((100, 50, 0, 0), b1);
        assert_eq!(b1, b2, "baseline shouldn't move on duplicate event");
        assert_eq!(u.input_tokens, 100);
        assert_eq!(u.output_tokens, 50);
    }

    #[test]
    fn saturating_handles_decreasing_cumulative_robust() {
        // If a provider misbehaves and reports a lower cumulative than
        // last time, saturating_sub yields zero — we don't underflow or
        // negatively adjust. The next higher reading recovers.
        let mut u = ModelUsage::default();
        let b1 = u.apply_cumulative((100, 50, 0, 0), (0, 0, 0, 0));
        let b2 = u.apply_cumulative((90, 30, 0, 0), b1); // bogus regression
        assert_eq!(b1, b2, "regression event must not move baseline");
        assert_eq!(u.output_tokens, 50, "no negative or wraparound charge");
        let _ = u.apply_cumulative((100, 80, 0, 0), b2);
        assert_eq!(u.output_tokens, 80, "next valid reading still works");
    }

    #[test]
    fn cache_tokens_apply_independently_robust() {
        let mut u = ModelUsage::default();
        let mut baseline = (0u32, 0, 0, 0);
        baseline = u.apply_cumulative((100, 0, 50, 0), baseline);
        baseline = u.apply_cumulative((100, 0, 75, 25), baseline);
        let _ = u.apply_cumulative((100, 0, 75, 100), baseline);
        assert_eq!(u.cache_read_tokens, 75);
        assert_eq!(u.cache_write_tokens, 100);
    }
}
pub fn sample_tool_harness_message() -> ChatMessage {
    let diff = parse_unified_diff(
        "crates/jfc-ui/src/tools.rs",
        r#"@@ -180,2 +180,2 @@
-async fn execute_bash(command: &str, timeout_ms: Option<u64>, cwd: &Path) -> ExecutionResult {
-    let timeout = timeout_ms.unwrap_or(120_000);
+async fn execute_bash(command: &str, timeout_ms: Option<u64>, cwd: &Path) -> ExecutionResult {
+    let timeout = timeout_ms.unwrap_or(300_000);
"#,
    );

    ChatMessage::assistant_parts(vec![
        MessagePart::Reasoning("Increase default bash timeout from 2min to 5min.".into()),
        MessagePart::tool_boxed(Box::new(ToolCall {
            id: "edit-1".into(),
            kind: ToolKind::Edit,
            status: ToolStatus::Completed,
            input: ToolInput::Edit {
                file_path: "crates/jfc-ui/src/tools.rs".into(),
                old_string: "let timeout = timeout_ms.unwrap_or(120_000);".into(),
                new_string: "let timeout = timeout_ms.unwrap_or(300_000);".into(),
                replacement: ReplacementMode::FirstOnly,
            },
            output: ToolOutput::Diff(diff),
            display: ToolDisplayState::DEFAULT,
            elapsed_ms: None,
            started_at: None,
            thought_signature: None,
        })),
        MessagePart::tool_boxed(Box::new(ToolCall {
            id: "bash-1".into(),
            kind: ToolKind::Bash,
            status: ToolStatus::Completed,
            input: ToolInput::Bash {
                command: "cargo check -p jfc-ui".into(),
                timeout: None,
                workdir: None,
                run_in_background: None,
            },
            output: ToolOutput::Command {
                stdout: "Finished `dev` profile [unoptimized + debuginfo] target(s) in 1.38s"
                    .into(),
                stderr: String::new(),
                exit_code: Some(0),
            },
            display: ToolDisplayState::DEFAULT,
            elapsed_ms: None,
            started_at: None,
            thought_signature: None,
        })),
        MessagePart::tool_boxed(Box::new(ToolCall {
            id: "read-1".into(),
            kind: ToolKind::Read,
            status: ToolStatus::Completed,
            input: ToolInput::Read {
                file_path: "crates/jfc-ui/src/main.rs".into(),
                offset: Some(1),
                limit: Some(80),
            },
            output: ToolOutput::FileContent {
                path: "crates/jfc-ui/src/main.rs".into(),
                language: "rust".into(),
                content: "mod app;\nmod context;\n\nuse std::sync::Arc;\nuse tokio::sync::mpsc;"
                    .into(),
            },
            display: ToolDisplayState::Collapsed,
            elapsed_ms: None,
            started_at: None,
            thought_signature: None,
        })),
        MessagePart::tool_boxed(Box::new(ToolCall {
            id: "write-1".into(),
            kind: ToolKind::Write,
            status: ToolStatus::Pending,
            input: ToolInput::Write {
                file_path: "crates/jfc-ui/src/tool_harness.rs".into(),
                content: "pub enum MessagePart { Text(String), Tool(ToolCall) }".into(),
            },
            output: ToolOutput::Text("Waiting for approval".into()),
            display: ToolDisplayState::Collapsed,
            elapsed_ms: None,
            started_at: None,
            thought_signature: None,
        })),
        MessagePart::tool_boxed(Box::new(ToolCall {
            id: "search-1".into(),
            kind: ToolKind::Search,
            status: ToolStatus::Running,
            input: ToolInput::Search {
                query: "ToolRegistry|DiffChanges|tool_result".into(),
                path: Some("research/opencode".into()),
            },
            output: ToolOutput::FileList(vec![
                "packages/ui/src/components/message-part.tsx".into(),
                "packages/ui/src/components/diff-changes.tsx".into(),
                "packages/opencode/src/tool/edit.ts".into(),
            ]),
            display: ToolDisplayState::Collapsed,
            elapsed_ms: None,
            started_at: None,
            thought_signature: None,
        })),
        MessagePart::tool_boxed(Box::new(ToolCall {
            id: "patch-1".into(),
            kind: ToolKind::ApplyPatch,
            status: ToolStatus::Completed,
            input: ToolInput::ApplyPatch {
                patch: "*** Begin Patch\n*** Update File: crates/jfc-ui/src/main.rs".into(),
            },
            output: ToolOutput::Diff(parse_unified_diff(
                "crates/jfc-ui/src/main.rs",
                r#"@@ -10,1 +10,1 @@
-struct ChatMessage;
+enum MessagePart;
"#,
            )),
            display: ToolDisplayState::Collapsed,
            elapsed_ms: None,
            started_at: None,
            thought_signature: None,
        })),
        MessagePart::tool_boxed(Box::new(ToolCall {
            id: "generic-1".into(),
            kind: ToolKind::Generic("Delegate".into()),
            status: ToolStatus::Failed,
            input: ToolInput::Generic {
                summary: "OpenClaude remote lookup".into(),
            },
            output: ToolOutput::Empty,
            display: ToolDisplayState::Collapsed,
            elapsed_ms: None,
            started_at: None,
            thought_signature: None,
        })),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    // ─── MessagePart helpers ──────────────────────────────────────────────

    #[test]
    fn message_part_text_only_for_compact_boundary_includes_token_count_normal() {
        let p = MessagePart::CompactBoundary { pre_tokens: 12_500 };
        let s = p.text_only();
        assert!(s.contains("12500"), "{s}");
    }

    #[test]
    fn message_part_approx_text_len_text_normal() {
        let p = MessagePart::Text("hello world".into());
        assert_eq!(p.approx_text_len(), 11);
    }

    #[test]
    fn message_part_approx_text_len_compact_boundary_zero_robust() {
        let p = MessagePart::CompactBoundary { pre_tokens: 999 };
        assert_eq!(p.approx_text_len(), 0);
    }

    #[test]
    fn message_part_approx_text_len_task_status_includes_summary_normal() {
        let p = MessagePart::TaskStatus(TaskStatusPart {
            task_id: "t1".into(),
            description: "do it".into(),
            status: TaskLifecycle::Running,
            summary: Some("almost done".into()),
            error: None,
            elapsed_ms: None,
            model: None,
        });
        assert_eq!(p.approx_text_len(), "do it".len() + "almost done".len());
    }

    #[test]
    fn message_part_to_display_string_reasoning_wraps_with_marker_normal() {
        let p = MessagePart::Reasoning("internal monologue".into());
        let s = p.to_display_string();
        assert!(s.starts_with("[Reasoning"), "{s}");
        assert!(s.contains("internal monologue"), "{s}");
    }

    // ─── ChatMessage helpers ──────────────────────────────────────────────

    #[test]
    fn chat_message_user_constructs_text_part_normal() {
        let m = ChatMessage::user("hi".into());
        assert!(m.role_is_user());
        assert!(matches!(&m.parts[0], MessagePart::Text(s) if s == "hi"));
        assert!(m.agent_name.is_none(), "user msgs have no agent name");
    }

    #[test]
    fn chat_message_assistant_constructs_text_part_normal() {
        let m = ChatMessage::assistant("hello".into());
        assert!(!m.role_is_user());
        assert!(matches!(&m.parts[0], MessagePart::Text(s) if s == "hello"));
    }

    #[test]
    fn chat_message_assistant_parts_preserves_input_normal() {
        let parts = vec![
            MessagePart::Reasoning("think".into()),
            MessagePart::Text("speak".into()),
        ];
        let m = ChatMessage::assistant_parts(parts);
        assert_eq!(m.parts.len(), 2);
    }

    #[test]
    fn chat_message_compact_boundary_marks_role_user_with_system_agent_robust() {
        let m = ChatMessage::compact_boundary("summary text", 12_345);
        assert!(
            m.role_is_user(),
            "compact boundary uses user role for replay"
        );
        assert!(m.is_compact_boundary());
        assert_eq!(m.agent_name.as_deref(), Some("system"));
    }

    #[test]
    fn chat_message_is_compact_boundary_only_when_part_present_normal() {
        let regular = ChatMessage::user("hi".into());
        assert!(!regular.is_compact_boundary());
    }

    // ─── ModelUsage::cache_hit_pct ────────────────────────────────────────

    #[test]
    fn model_usage_cache_hit_pct_zero_input_safe_normal() {
        let u = ModelUsage::default();
        assert_eq!(u.cache_hit_pct(), 0.0);
    }

    #[test]
    fn model_usage_cache_hit_pct_capped_at_100_robust() {
        // If a buggy provider reports cache_read > input we still cap at 100%.
        let u = ModelUsage {
            input_tokens: 10,
            cache_read_tokens: 50,
            ..Default::default()
        };
        assert_eq!(u.cache_hit_pct(), 100.0);
    }

    #[test]
    fn model_usage_cache_hit_pct_normal_value_normal() {
        let u = ModelUsage {
            input_tokens: 100,
            cache_read_tokens: 25,
            ..Default::default()
        };
        assert_eq!(u.cache_hit_pct(), 25.0);
    }

    #[test]
    fn model_usage_total_context_tokens_sums_all_normal() {
        let u = ModelUsage {
            input_tokens: 100,
            output_tokens: 200,
            cache_read_tokens: 10,
            cache_write_tokens: 20,
            cost_usd: None,
        };
        assert_eq!(u.total_context_tokens(), 330);
    }

    #[test]
    fn model_usage_add_delta_accumulates_normal() {
        let mut u = ModelUsage::default();
        u.add_delta(10, 20, 5, 3);
        u.add_delta(1, 2, 0, 0);
        assert_eq!(u.input_tokens, 11);
        assert_eq!(u.output_tokens, 22);
        assert_eq!(u.cache_read_tokens, 5);
        assert_eq!(u.cache_write_tokens, 3);
    }

    // ─── parse_unified_diff / parse_hunk_header / parse_hunk_start ─────────

    #[test]
    fn parse_hunk_start_strips_sign_and_count_normal() {
        assert_eq!(parse_hunk_start("-12,5"), 12);
        assert_eq!(parse_hunk_start("+200,1"), 200);
        assert_eq!(parse_hunk_start("17"), 17);
    }

    #[test]
    fn parse_hunk_start_returns_one_for_unparseable_robust() {
        assert_eq!(parse_hunk_start("notanumber"), 1);
        assert_eq!(parse_hunk_start(""), 1);
    }

    #[test]
    fn parse_hunk_header_extracts_old_new_starts_normal() {
        let (old, new, _) = parse_hunk_header("@@ -1,5 +10,7 @@ fn foo");
        assert_eq!(old, 1);
        assert_eq!(new, 10);
    }

    #[test]
    fn parse_unified_diff_counts_additions_deletions_normal() {
        let view = parse_unified_diff("x.rs", "@@ -1,3 +1,3 @@\n a\n-b\n+c\n d\n");
        assert_eq!(view.additions, 1);
        assert_eq!(view.deletions, 1);
        assert_eq!(view.file_path, "x.rs");
        assert_eq!(view.hunks.len(), 1);
    }

    #[test]
    fn parse_unified_diff_handles_multiple_hunks_normal() {
        let view = parse_unified_diff(
            "x.rs",
            "@@ -1,1 +1,1 @@\n-a\n+b\n@@ -10,1 +10,1 @@\n-c\n+d\n",
        );
        assert_eq!(view.hunks.len(), 2);
        assert_eq!(view.additions, 2);
        assert_eq!(view.deletions, 2);
    }

    #[test]
    fn parse_unified_diff_lines_before_hunk_skipped_robust() {
        // Lines before the first @@ have no hunk to attach to — they're
        // dropped silently. A real "missing header" produces an empty
        // hunk list, not a panic.
        let view = parse_unified_diff("x.rs", "stray text\n");
        assert!(view.hunks.is_empty());
        assert_eq!(view.additions, 0);
    }

    // ─── truncate_lines ──────────────────────────────────────────────────

    #[test]
    fn truncate_lines_below_max_returns_unchanged_normal() {
        let s = "a\nb\nc\n";
        // Note: the implementation's `lines.iter().take(max).join("\n")`
        // strips trailing newline since `lines()` doesn't include it.
        let out = truncate_lines(s, 10);
        assert_eq!(out, "a\nb\nc");
    }

    #[test]
    fn truncate_lines_above_max_appends_more_marker_robust() {
        let s = "a\nb\nc\nd\ne\n";
        let out = truncate_lines(s, 2);
        assert!(out.contains("a"));
        assert!(out.contains("b"));
        assert!(!out.contains("c"));
        assert!(out.contains("3 more"), "{out}");
    }

    #[test]
    fn truncate_lines_empty_input_robust() {
        assert_eq!(truncate_lines("", 5), "");
    }

    // ─── validate_turn_invariants ─────────────────────────────────────────

    fn pending_tool_call(id: &str) -> ToolCall {
        ToolCall {
            id: id.into(),
            kind: ToolKind::Bash,
            status: ToolStatus::Pending,
            input: ToolInput::Bash {
                command: "ls".into(),
                timeout: None,
                workdir: None,
                run_in_background: None,
            },
            output: ToolOutput::Empty,
            display: ToolDisplayState::DEFAULT,
            elapsed_ms: None,
            started_at: None,
            thought_signature: None,
        }
    }

    fn complete_tool_call(id: &str) -> ToolCall {
        ToolCall {
            status: ToolStatus::Completed,
            output: ToolOutput::Text("ok".into()),
            ..pending_tool_call(id)
        }
    }

    /// Normal: a healthy alternating user/assistant transcript passes
    /// validation cleanly. Empty inputs are also accepted.
    #[test]
    fn validate_turn_invariants_accepts_alternating_transcript_normal() {
        assert!(validate_turn_invariants(&[]).is_ok());
        let msgs = vec![
            ChatMessage::user("hi".into()),
            ChatMessage::assistant("hey".into()),
            ChatMessage::user("more".into()),
            ChatMessage::assistant("ok".into()),
        ];
        validate_turn_invariants(&msgs).expect("alternating transcript is valid");
    }

    /// Robust: two adjacent user messages surface ConsecutiveUser at the
    /// SECOND user's index — that's the position the queue-drain bug
    /// would land at.
    #[test]
    fn validate_turn_invariants_flags_consecutive_user_robust() {
        let msgs = vec![
            ChatMessage::user("first".into()),
            ChatMessage::user("second".into()),
        ];
        let err = validate_turn_invariants(&msgs).expect_err("must flag consecutive user");
        assert_eq!(err, TurnInvariantError::ConsecutiveUser { at_index: 1 });
    }

    /// Robust: this is the structural shape of the plan-continuation
    /// phantom-assistant bug — two assistant messages back-to-back.
    #[test]
    fn validate_turn_invariants_flags_consecutive_assistant_robust() {
        let msgs = vec![
            ChatMessage::user("hi".into()),
            ChatMessage::assistant("a".into()),
            ChatMessage::assistant("b".into()),
        ];
        let err = validate_turn_invariants(&msgs).expect_err("must flag consecutive assistant");
        assert_eq!(
            err,
            TurnInvariantError::ConsecutiveAssistant { at_index: 2 }
        );
    }

    /// Robust: a fully empty user message (no text, no tools, no
    /// boundary) trips EmptyMessage. The streaming-tail exception
    /// only applies to assistants, so a user-empty must always fail.
    #[test]
    fn validate_turn_invariants_flags_empty_user_robust() {
        let msgs = vec![ChatMessage {
            role: Role::User,
            parts: vec![MessagePart::Text(String::new())],
            agent_name: None,
            model_name: None,
            cost_tier: None,
            elapsed: None,
            usage: None,
            queued: false,
            attachments: Vec::new(),
        }];
        let err = validate_turn_invariants(&msgs).expect_err("empty user must fail");
        assert_eq!(
            err,
            TurnInvariantError::EmptyMessage {
                at_index: 0,
                role: Role::User,
            }
        );
    }

    /// Normal: an empty assistant message at the tail of the slice
    /// is allowed when `allow_streaming_tail = true` — that's the
    /// placeholder slot `continue_agentic_loop` stages right before
    /// the stream starts pumping.
    #[test]
    fn validate_turn_invariants_streaming_tail_allowed_normal() {
        let msgs = vec![
            ChatMessage::user("hi".into()),
            ChatMessage::assistant(String::new()),
        ];
        // Strict mode rejects the empty placeholder.
        let err = validate_turn_invariants(&msgs).expect_err("strict mode rejects empty tail");
        assert!(matches!(err, TurnInvariantError::EmptyMessage { .. }));
        // Permissive mode accepts it (the streaming pipeline is about
        // to fill it in).
        validate_turn_invariants_inner(&msgs, /* allow_streaming_tail = */ true)
            .expect("streaming-tail mode accepts empty trailing assistant");
    }

    /// Robust: a Pending tool on a non-tail assistant message means
    /// the model rolled forward without a tool_result — surface as
    /// OrphanToolUse carrying the tool id and index.
    #[test]
    fn validate_turn_invariants_flags_orphan_tool_use_robust() {
        let msgs = vec![
            ChatMessage::user("run it".into()),
            ChatMessage::assistant_parts(vec![MessagePart::tool_boxed(Box::new(
                pending_tool_call("tool_42"),
            ))]),
            ChatMessage::user("never mind".into()),
            ChatMessage::assistant("ok".into()),
        ];
        let err = validate_turn_invariants(&msgs).expect_err("must flag orphan tool_use");
        match err {
            TurnInvariantError::OrphanToolUse { tool_id, at_index } => {
                assert_eq!(tool_id, crate::ids::ToolId::new("tool_42"));
                assert_eq!(at_index, 1);
            }
            other => panic!("expected OrphanToolUse, got {other:?}"),
        }
    }

    /// Robust: a Tool part on a Role::User message is structurally
    /// misrouted — tool calls always belong to assistant turns.
    #[test]
    fn validate_turn_invariants_flags_tool_on_user_role_robust() {
        let msgs = vec![ChatMessage {
            role: Role::User,
            parts: vec![
                MessagePart::Text("hi".into()),
                MessagePart::tool_boxed(Box::new(complete_tool_call("tool_99"))),
            ],
            agent_name: None,
            model_name: None,
            cost_tier: None,
            elapsed: None,
            usage: None,
            queued: false,
            attachments: Vec::new(),
        }];
        let err = validate_turn_invariants(&msgs).expect_err("tool part on user role must fail");
        match err {
            TurnInvariantError::OrphanToolResult { tool_id, at_index } => {
                assert_eq!(tool_id, crate::ids::ToolId::new("tool_99"));
                assert_eq!(at_index, 0);
            }
            other => panic!("expected OrphanToolResult, got {other:?}"),
        }
    }

    /// Robust: a transcript that opens with an Assistant message
    /// (without a system-injected boundary) is the visual symptom of
    /// the phantom-leading-slot bug. Surface as LeadingAssistant.
    #[test]
    fn validate_turn_invariants_flags_leading_assistant_robust() {
        let msgs = vec![
            ChatMessage::assistant("oops, I went first".into()),
            ChatMessage::user("hi".into()),
        ];
        let err = validate_turn_invariants(&msgs).expect_err("leading assistant must fail");
        assert_eq!(
            err,
            TurnInvariantError::LeadingAssistant {
                role: Role::Assistant,
            }
        );
    }

    /// Normal: a CompactBoundary is a legitimate Role::User message
    /// that may be followed by another User-role reply describing the
    /// resumed task. The validator must accept that exact seam.
    #[test]
    fn validate_turn_invariants_compact_boundary_seam_allowed_normal() {
        let msgs = vec![
            ChatMessage::user("first round".into()),
            ChatMessage::assistant("ok".into()),
            ChatMessage::compact_boundary("summary text", 12_000),
            ChatMessage::user("continue from here".into()),
            ChatMessage::assistant("resuming".into()),
        ];
        validate_turn_invariants(&msgs)
            .expect("compact boundary may sit between two user messages");
    }
}

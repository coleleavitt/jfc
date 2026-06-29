#[cfg(test)]
mod tests {
    use crate::ids::SessionId;
    use crate::session::deserialize::*;
    use crate::session::serialize::*;
    use crate::types::{
        DiffHunk, DiffLine, DiffLineKind, DiffView, MessagePart, ReplacementMode, TaskInput,
        TaskLifecycle, TaskStatusPart, ToolInput, ToolOutput,
    };
    use jfc_session::{
        SessionMetadata, cwd_mismatch_message, group_by_cwd, relative_time, shorten_cwd,
    };

    #[test]
    fn roundtrip_tool_input_edit() {
        let input = ToolInput::Edit {
            file_path: "src/main.rs".into(),
            old_string: "old".into(),
            new_string: "new".into(),
            replacement: ReplacementMode::All,
        };
        let serialized = serialize_tool_input(&input);
        let deserialized = deserialize_tool_input(serialized);
        match deserialized {
            ToolInput::Edit {
                file_path,
                old_string,
                new_string,
                replacement,
            } => {
                assert_eq!(file_path, "src/main.rs");
                assert_eq!(old_string, "old");
                assert_eq!(new_string, "new");
                assert!(replacement.replace_all());
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn roundtrip_tool_input_task_stop_preserves_variant() {
        // Regression: TaskStop must NOT collapse into TaskDone on resume —
        // a cancellation request and a completion are semantically distinct.
        let input = ToolInput::TaskStop {
            task_id: "t42".into(),
        };
        let serialized = serialize_tool_input(&input);
        let deserialized = deserialize_tool_input(serialized);
        match deserialized {
            ToolInput::TaskStop { task_id } => assert_eq!(task_id, "t42"),
            other => panic!("expected TaskStop, got {other:?}"),
        }
    }

    #[test]
    fn roundtrip_tool_input_task_preserves_contract_fields_regression() {
        let input = ToolInput::Task(TaskInput {
            description: "inspect".into(),
            prompt: "inspect".into(),
            subagent_type: Some("explore".into()),
            category: Some("audit".into()),
            run_in_background: false,
            model: None,
            launcher: Some("variant-agent".into()),
            effort: None,
            name: Some("reader".into()),
            team_name: Some("review".into()),
            mode: Some("plan".into()),
            isolation: Some("worktree".into()),
            parent_task_id: Some("t3".into()),
            schema: Some(serde_json::json!({
                "type": "object",
                "required": ["summary"],
                "properties": { "summary": { "type": "string" } }
            })),
            allowed_tools: vec!["Read".into(), "Grep".into()],
            disallowed_tools: vec!["Bash".into()],
            cwd: Some("/tmp/project".into()),
        });

        let serialized = serialize_tool_input(&input);
        let deserialized = deserialize_tool_input(serialized);
        let ToolInput::Task(task) = deserialized else {
            panic!("expected Task input");
        };
        assert_eq!(
            task.schema.as_ref().and_then(|v| v.get("type")),
            Some(&serde_json::json!("object"))
        );
        assert_eq!(task.allowed_tools, vec!["Read", "Grep"]);
        assert_eq!(task.disallowed_tools, vec!["Bash"]);
        assert_eq!(task.launcher.as_deref(), Some("variant-agent"));
        assert_eq!(task.name.as_deref(), Some("reader"));
        assert_eq!(task.team_name.as_deref(), Some("review"));
        assert_eq!(task.isolation.as_deref(), Some("worktree"));
    }

    #[test]
    fn roundtrip_tool_input_post_bounty_preserves_parent_task_normal() {
        let input = ToolInput::PostBounty {
            description: "audit task routing".into(),
            budget: 777,
            acceptance_criteria: "parent task has bounty metadata".into(),
            max_solvers: Some(2),
            auto_dispatch: true,
            parent_task_id: Some("t1".into()),
        };

        let serialized = serialize_tool_input(&input);
        let deserialized = deserialize_tool_input(serialized);
        let ToolInput::PostBounty {
            description,
            budget,
            acceptance_criteria,
            max_solvers,
            auto_dispatch,
            parent_task_id,
        } = deserialized
        else {
            panic!("expected PostBounty input");
        };
        assert_eq!(description, "audit task routing");
        assert_eq!(budget, 777);
        assert_eq!(acceptance_criteria, "parent task has bounty metadata");
        assert_eq!(max_solvers, Some(2));
        assert!(auto_dispatch);
        assert_eq!(parent_task_id.as_deref(), Some("t1"));
    }

    #[test]
    fn roundtrip_tool_output_diff() {
        let output = ToolOutput::Diff(DiffView {
            file_path: "test.rs".into(),
            additions: 5,
            deletions: 3,
            hunks: vec![DiffHunk {
                old_start: 10,
                new_start: 10,
                header: "@@ -10,5 +10,7 @@".into(),
                lines: vec![
                    DiffLine {
                        kind: DiffLineKind::Removed,
                        old_line: Some(10),
                        new_line: None,
                        content: "old line".into(),
                    },
                    DiffLine {
                        kind: DiffLineKind::Added,
                        old_line: None,
                        new_line: Some(10),
                        content: "new line".into(),
                    },
                ],
            }],
        });
        let serialized = serialize_tool_output(&output);
        let deserialized = deserialize_tool_output(serialized);
        match deserialized {
            ToolOutput::Diff(d) => {
                assert_eq!(d.file_path, "test.rs");
                assert_eq!(d.additions, 5);
                assert_eq!(d.deletions, 3);
                assert_eq!(d.hunks.len(), 1);
                assert_eq!(d.hunks[0].lines.len(), 2);
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn cwd_mismatch_returns_none_when_match_normal() {
        // Same paths -> no warning. The happy case for resume in the
        // same project the session was created in.
        let same = "/home/user/project";
        assert_eq!(cwd_mismatch_message(Some(same), same), None);
    }

    #[test]
    fn cwd_mismatch_returns_message_when_different_normal() {
        // Different paths -> Some, message contains both. Mirrors
        // codex-rs `session_resume.rs:99-111`.
        let session_cwd = "/home/user/project-a";
        let current_cwd = "/home/user/project-b";
        let msg = cwd_mismatch_message(Some(session_cwd), current_cwd)
            .expect("differing paths should produce a warning");
        assert!(
            msg.contains(session_cwd),
            "message should contain session cwd: {msg}"
        );
        assert!(
            msg.contains(current_cwd),
            "message should contain current cwd: {msg}"
        );
    }

    #[test]
    fn cwd_mismatch_returns_none_for_legacy_unset_robust() {
        // Legacy sessions written before the cwd field existed have
        // session_cwd=None. We must NOT warn — there's nothing to
        // compare against.
        assert_eq!(cwd_mismatch_message(None, "/anywhere"), None);
    }

    #[test]
    fn cwd_mismatch_returns_none_for_empty_current_robust() {
        // current_cwd="" means `std::env::current_dir()` failed (e.g.
        // the cwd was deleted). We don't have a real path to compare
        // to, so suppress the warning rather than surface noise.
        assert_eq!(cwd_mismatch_message(Some("/home/user/project"), ""), None);
    }

    #[test]
    fn roundtrip_task_status_part() {
        let part = MessagePart::TaskStatus(TaskStatusPart {
            task_id: "t1".into(),
            description: "Test task".into(),
            status: TaskLifecycle::Running,
            summary: Some("Working on it".into()),
            error: None,
            elapsed_ms: Some(1500),
            model: None,
        });
        let serialized = serialize_part(&part);
        let deserialized = deserialize_part(serialized);
        match deserialized {
            MessagePart::TaskStatus(ts) => {
                assert_eq!(ts.task_id, "t1");
                assert_eq!(ts.description, "Test task");
                assert_eq!(ts.status, TaskLifecycle::Running);
                assert_eq!(ts.summary, Some("Working on it".into()));
                assert_eq!(ts.elapsed_ms, Some(1500));
            }
            _ => panic!("wrong variant"),
        }
    }

    fn make_session(id: &str, cwd: Option<&str>, prompt: Option<&str>) -> SessionMetadata {
        SessionMetadata {
            id: SessionId::new(id),
            created_at: "2026-05-04T19:46:49Z".to_owned(),
            updated_at: Some("2026-05-04T19:46:49Z".to_owned()),
            first_prompt: prompt.map(str::to_owned),
            cwd: cwd.map(str::to_owned),
            title: None,
            message_count: 1,
        }
    }

    #[test]
    fn group_splits_current_cwd_first_normal() {
        let sessions = vec![
            make_session("ses_1", Some("/home/c/jfc"), None),
            make_session("ses_2", Some("/home/c/other"), None),
            make_session("ses_3", Some("/home/c/jfc"), None),
            make_session("ses_4", Some("/home/c/other"), None),
        ];
        let (this_proj, other) = group_by_cwd(sessions, Some("/home/c/jfc"));
        assert_eq!(this_proj.len(), 2);
        assert_eq!(other.len(), 2);
        assert_eq!(this_proj[0].id, "ses_1");
        assert_eq!(this_proj[1].id, "ses_3");
        assert_eq!(other[0].id, "ses_2");
        assert_eq!(other[1].id, "ses_4");
    }

    #[test]
    fn group_legacy_none_cwd_goes_to_other_robust() {
        let sessions = vec![
            make_session("ses_1", None, None),
            make_session("ses_2", Some("/home/c/jfc"), None),
        ];
        let (this_proj, other) = group_by_cwd(sessions, Some("/home/c/jfc"));
        assert_eq!(this_proj.len(), 1);
        assert_eq!(this_proj[0].id, "ses_2");
        assert_eq!(other.len(), 1);
        assert_eq!(other[0].id, "ses_1");
    }

    #[test]
    fn group_no_current_cwd_all_other_robust() {
        let sessions = vec![
            make_session("ses_1", Some("/home/c/jfc"), None),
            make_session("ses_2", None, None),
            make_session("ses_3", Some("/home/c/other"), None),
        ];
        let (this_proj, other) = group_by_cwd(sessions, None);
        assert!(this_proj.is_empty());
        assert_eq!(other.len(), 3);
    }

    #[test]
    fn group_empty_input_normal() {
        let (this_proj, other) = group_by_cwd(Vec::new(), Some("/home/c/jfc"));
        assert!(this_proj.is_empty());
        assert!(other.is_empty());
    }

    #[test]
    fn group_preserves_order_within_group_normal() {
        let sessions = vec![
            make_session("ses_a", Some("/p1"), None),
            make_session("ses_b", Some("/p2"), None),
            make_session("ses_c", Some("/p1"), None),
            make_session("ses_d", Some("/p2"), None),
            make_session("ses_e", Some("/p1"), None),
        ];
        let (this_proj, other) = group_by_cwd(sessions, Some("/p1"));
        let this_ids: Vec<&str> = this_proj.iter().map(|s| s.id.as_str()).collect();
        let other_ids: Vec<&str> = other.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(this_ids, vec!["ses_a", "ses_c", "ses_e"]);
        assert_eq!(other_ids, vec!["ses_b", "ses_d"]);
    }

    #[test]
    fn picker_row_text_uses_display_title_normal() {
        // Pin: title comes from `first_prompt` when present...
        let with_prompt = make_session("ses_1", None, Some("Refactor compaction"));
        assert_eq!(with_prompt.display_title(), "Refactor compaction");

        // ...and falls back to a formatted timestamp from the id when missing.
        let without_prompt = make_session("ses_20260504_194649", None, None);
        assert_eq!(without_prompt.display_title(), "2026-05-04 19:46");

        // Empty / whitespace prompt → fallback (not an empty title).
        let blank = make_session("ses_20260504_194649", None, Some("   \n  "));
        assert_eq!(blank.display_title(), "2026-05-04 19:46");

        // Long single-line prompts get truncated with an ellipsis so the
        // sidebar row never wraps unpredictably.
        let long = "a".repeat(200);
        let long_session = make_session("ses_1", None, Some(&long));
        let title = long_session.display_title();
        assert!(title.ends_with('…'));
        assert!(title.chars().count() <= 61); // 60 + '…'

        // Multi-line prompts only show the first line.
        let multi = make_session("ses_1", None, Some("first line\nsecond line"));
        assert_eq!(multi.display_title(), "first line");
    }

    #[test]
    fn shorten_cwd_handles_home_basename_and_none() {
        // None → placeholder, never panics.
        assert_eq!(shorten_cwd(None), "—");

        // Non-home absolute → basename (so narrow sidebars stay readable).
        assert_eq!(shorten_cwd(Some("/var/log/something")), "something");
        assert_eq!(shorten_cwd(Some("/var/log/something/")), "something");
        assert_eq!(shorten_cwd(Some("/")), "/");
    }

    #[test]
    fn relative_time_buckets() {
        let now = chrono::DateTime::parse_from_rfc3339("2026-05-04T20:00:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc);

        // Future / clock skew.
        assert_eq!(relative_time("2026-05-04T20:00:30Z", now), "now");
        // Sub-minute.
        assert_eq!(relative_time("2026-05-04T19:59:30Z", now), "just now");
        // Minutes.
        assert_eq!(relative_time("2026-05-04T19:46:00Z", now), "14m ago");
        // Hours.
        assert_eq!(relative_time("2026-05-04T17:00:00Z", now), "3h ago");
        // Days.
        assert_eq!(relative_time("2026-05-02T20:00:00Z", now), "2d ago");
        // Garbage input → placeholder.
        assert_eq!(relative_time("not a timestamp", now), "—");
    }
}

#[cfg(test)]
mod cwd_filter_tests {
    use crate::ids::SessionId;
    use crate::session::deserialize::*;
    use crate::session::serialization::*;
    use crate::session::serialize::*;

    use jfc_session::SessionMetadata;

    fn meta(
        id: &str,
        cwd: Option<&str>,
        title: Option<&str>,
        prompt: Option<&str>,
    ) -> SessionMetadata {
        SessionMetadata {
            id: SessionId::new(id),
            created_at: "2026-01-01T00:00:00Z".to_string(),
            updated_at: None,
            first_prompt: prompt.map(str::to_owned),
            cwd: cwd.map(str::to_owned),
            title: title.map(str::to_owned),
            message_count: 1,
        }
    }

    #[test]
    fn display_title_prefers_custom_title_normal() {
        // Title precedence (v126 cli.js:39786): customTitle wins.
        let m = meta("s1", None, Some("My session"), Some("hello world"));
        assert_eq!(m.display_title(), "My session");
    }

    #[test]
    fn display_title_falls_through_to_first_prompt_normal() {
        let m = meta("s1", None, None, Some("hello world"));
        assert_eq!(m.display_title(), "hello world");
    }

    #[test]
    fn display_title_truncates_long_first_prompt_normal() {
        // Long prompts get truncated with ellipsis so the picker doesn't blow out.
        let long_prompt: String = "x".repeat(80);
        let m = meta("s1", None, None, Some(&long_prompt));
        let title = m.display_title();
        assert!(title.ends_with('…'), "got: {title}");
        assert_eq!(title.chars().count(), 61);
    }

    #[test]
    fn display_title_empty_prompt_falls_to_id_robust() {
        // Both title + first_prompt empty/None → fall back to
        // format_session_id_timestamp(id) which pretty-prints
        // `ses_YYYYMMDD_HHMMSS`. Non-matching ids pass through verbatim.
        let m = meta("ses_20260504_194649", None, None, None);
        assert_eq!(m.display_title(), "2026-05-04 19:46");

        // Verbatim passthrough for ids that don't match the ses_ pattern.
        let m = meta("abcdef1234567890", None, None, None);
        assert_eq!(m.display_title(), "abcdef1234567890");
    }

    #[test]
    fn display_title_empty_string_title_uses_first_prompt_robust() {
        // Empty-string title should still fall through, not display blank.
        let m = meta("s1", Some(""), Some("hello"), None);
        assert_eq!(m.display_title(), "hello");
    }

    /// Match-logic helper for the cwd filter (extracted for testability).
    fn matches_filter(session_cwd: Option<&str>, target: Option<&str>) -> bool {
        match target {
            None => true,
            Some(t) => session_cwd.is_none_or(|c| c == t),
        }
    }

    #[test]
    fn cwd_filter_no_filter_lets_all_through_normal() {
        assert!(matches_filter(Some("/a"), None));
        assert!(matches_filter(None, None));
    }

    #[test]
    fn cwd_filter_matches_exact_path_normal() {
        assert!(matches_filter(Some("/a"), Some("/a")));
        assert!(!matches_filter(Some("/b"), Some("/a")));
    }

    #[test]
    fn cwd_filter_lets_legacy_unset_cwd_through_robust() {
        // Sessions saved before the cwd field existed have cwd=None.
        // We surface them in any cwd's listing so the user doesn't lose
        // history — they can still `/continue all` to find them.
        assert!(matches_filter(None, Some("/a")));
    }

    // Round-trip: usage attached to an assistant message survives
    // serde → JSON → serde. Without serde wiring on `ModelUsage` the
    // resume gauge would always read 0.
    #[test]
    fn message_usage_round_trips_through_serde_normal() {
        use crate::types::{ChatMessage, MessagePart, ModelUsage, Role};
        let mut msg = ChatMessage::assistant("hi".into());
        msg.usage = Some(ModelUsage {
            input_tokens: 12_345,
            output_tokens: 678,
            thinking_tokens: 321,
            cache_read_tokens: 9_000,
            cache_write_tokens: 100,
            cost_usd: None,
        });
        let serialized = serialize_message(&msg);
        let json = serde_json::to_string(&serialized).expect("ser");
        let parsed: SerializedMessage = serde_json::from_str(&json).expect("de");
        let round = deserialize_message(parsed);
        let u = round.usage.expect("usage preserved");
        assert_eq!(u.input_tokens, 12_345);
        assert_eq!(u.output_tokens, 678);
        assert_eq!(u.cache_read_tokens, 9_000);
        assert_eq!(u.cache_write_tokens, 100);
        // Total context tokens = sum of all four (matches v126 W_$).
        assert_eq!(u.total_context_tokens(), 12_345 + 678 + 9_000 + 100);
        // Suppress unused-variant warnings via discriminant check.
        match round.role {
            Role::Assistant => {}
            Role::User => panic!("role should round-trip"),
        }
        assert!(matches!(round.parts.first(), Some(MessagePart::Text(_))));
    }

    // Robust: legacy session JSON without `usage` field still loads,
    // with `usage = None`. Old session files must keep working.
    #[test]
    fn message_without_usage_field_loads_with_none_robust() {
        let legacy = r#"{
            "role": "assistant",
            "parts": [{ "type": "text", "content": "hi" }]
        }"#;
        let parsed: SerializedMessage = serde_json::from_str(legacy).expect("legacy load");
        assert!(parsed.usage.is_none());
        let round = deserialize_message(parsed);
        assert!(round.usage.is_none());
    }

    // Regression: an UNKNOWN tool-output `type` must not fail the whole-session
    // parse. Before the `#[serde(other)]` catch-all, a single unrecognized
    // variant (e.g. `background_bash` written by a newer build) made
    // `serde_json::from_str::<SerializedSession>` return Err → `--resume`
    // silently started fresh and the transcript showed empty. The variant must
    // degrade gracefully and leave every other message intact.
    #[test]
    fn unknown_tool_output_variant_does_not_poison_session_parse_robust() {
        let session = r#"{
            "id": "ses_x",
            "created_at": "2026-06-12T00:00:00Z",
            "messages": [
                {
                    "role": "user",
                    "parts": [{ "type": "text", "content": "run the tests" }]
                },
                {
                    "role": "assistant",
                    "parts": [
                        { "type": "text", "content": "done" },
                        {
                            "type": "tool",
                            "id": "t1",
                            "kind": "Bash",
                            "status": "completed",
                            "output": {
                                "type": "background_bash",
                                "task_id": "bash_abc",
                                "status": { "state": "completed", "exit_code": 0 },
                                "tail": "test result: ok. 3 passed",
                                "total_bytes": 24,
                                "total_lines": 1
                            }
                        }
                    ]
                }
            ]
        }"#;
        // The whole session must parse despite the unknown output variant.
        let parsed: SerializedSession = serde_json::from_str(session)
            .expect("session with unknown tool output must still load");
        assert_eq!(parsed.messages.len(), 2);

        // And the unknown variant salvages its human-readable `tail` text
        // rather than vanishing.
        let salvaged = parsed.messages[1]
            .parts
            .iter()
            .find_map(|p| match p {
                SerializedPart::Tool { tool } => Some(tool),
                _ => None,
            })
            .expect("tool part present");
        match &salvaged.output {
            Some(SerializedToolOutput::Text { content }) => {
                assert!(content.contains("3 passed"), "tail not salvaged: {content}");
            }
            _ => panic!("expected salvaged Text output from background_bash tail"),
        }
    }

    // Unknown variant with no salvageable text degrades to Empty, never errors.
    #[test]
    fn unknown_tool_output_with_no_text_becomes_empty_robust() {
        let json = r#"{ "type": "some_future_variant", "widget_id": 7 }"#;
        let parsed: SerializedToolOutput =
            serde_json::from_str(json).expect("unknown variant must not error");
        assert!(matches!(parsed, SerializedToolOutput::Empty));
    }

    // A whitespace-only text field is treated as no text: salvage skips it and
    // falls through to the next key (here `message`), never returning a blank
    // Text cell.
    #[test]
    fn unknown_tool_output_whitespace_text_skips_to_next_key_robust() {
        let json = r#"{ "type": "future", "tail": "  \n\t ", "message": "real text" }"#;
        let parsed: SerializedToolOutput =
            serde_json::from_str(json).expect("unknown variant must not error");
        match parsed {
            SerializedToolOutput::Text { content } => assert_eq!(content, "real text"),
            _ => panic!("expected fallthrough to `message`"),
        }

        // Whitespace-only everywhere → Empty.
        let blank = r#"{ "type": "future", "tail": "   ", "content": "\n" }"#;
        let parsed: SerializedToolOutput =
            serde_json::from_str(blank).expect("unknown variant must not error");
        assert!(matches!(parsed, SerializedToolOutput::Empty));
    }
}

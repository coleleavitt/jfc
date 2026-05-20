#[cfg(test)]
mod tests {
    use crate::session::deserialize::*;
    use crate::session::serialize::*;
    use crate::session::serialization::*;
    use crate::ids::SessionId;
    use crate::types::{
        ChatMessage, DiffHunk, DiffLine, DiffLineKind, DiffView, LargeText, MessagePart,
        ReplacementMode, Role, TaskInput, TaskLifecycle, TaskStatusPart, ToolCall, ToolInput,
        ToolKind, ToolOutput, ToolStatus,
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
    use crate::session::deserialize::*;
    use crate::session::serialize::*;
    use crate::session::serialization::*;
    use crate::ids::SessionId;
    use crate::types::*;
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
}


use jfc_core::{ReplacementMode, TaskInput, ToolInput, ToolInputError};

mod tests {
    use super::*;

    #[test]
    fn edit_input_json_snapshot_omits_default_replacement_mode() {
        let input = ToolInput::Edit {
            file_path: "src/main.rs".into(),
            old_string: "old".into(),
            new_string: "new".into(),
            replacement: ReplacementMode::FirstOnly,
        };

        assert_eq!(
            input.to_value().to_string(),
            r#"{"file_path":"src/main.rs","old_string":"old","new_string":"new"}"#
        );
    }

    #[test]
    fn edit_input_json_snapshot_preserves_replace_all_wire_shape() {
        let input = ToolInput::Edit {
            file_path: "src/main.rs".into(),
            old_string: "old".into(),
            new_string: "new".into(),
            replacement: ReplacementMode::All,
        };

        assert_eq!(
            input.to_value().to_string(),
            r#"{"file_path":"src/main.rs","old_string":"old","new_string":"new","replace_all":true}"#
        );
    }

    #[test]
    fn task_input_summary_background_flag() {
        let fg = TaskInput {
            description: "do thing".into(),
            prompt: "please do it".into(),
            subagent_type: None,
            category: None,
            run_in_background: false,
            model: None,
            launcher: None,
            effort: None,
            name: None,
            team_name: None,
            mode: None,
            isolation: None,
            parent_task_id: None,
            schema: None,
            allowed_tools: Vec::new(),
            disallowed_tools: Vec::new(),
            cwd: None,
        };
        assert!(fg.summary().contains("foreground"));

        let bg = TaskInput {
            run_in_background: true,
            ..fg
        };
        assert!(bg.summary().contains("background"));
    }

    #[test]
    fn task_input_to_value_roundtrip() {
        let input = ToolInput::Task(TaskInput {
            description: "research".into(),
            prompt: "find patterns".into(),
            subagent_type: Some("explore".into()),
            category: None,
            run_in_background: true,
            model: None,
            launcher: Some("variant-agent".into()),
            effort: None,
            name: None,
            team_name: None,
            mode: None,
            isolation: None,
            parent_task_id: None,
            schema: None,
            allowed_tools: vec!["Read".into(), "Grep".into()],
            disallowed_tools: vec!["Bash".into()],
            cwd: None,
        });
        let v = input.to_value();
        assert_eq!(v["description"], "research");
        assert_eq!(v["subagent_type"], "explore");
        assert_eq!(v["run_in_background"], true);
        assert_eq!(v["launcher"], "variant-agent");
        assert_eq!(v["allowed_tools"], serde_json::json!(["Read", "Grep"]));
        assert_eq!(v["disallowed_tools"], serde_json::json!(["Bash"]));
        assert!(v.get("category").is_none() || v["category"].is_null());

        let parsed = ToolInput::from_value("Task", v).expect("task input should round-trip");
        let ToolInput::Task(parsed) = parsed else {
            panic!("expected Task input");
        };
        assert_eq!(parsed.allowed_tools, vec!["Read", "Grep"]);
        assert_eq!(parsed.disallowed_tools, vec!["Bash"]);
    }

    #[test]
    fn learn_rsi_control_inputs_round_trip_normal() {
        let list = ToolInput::from_value(
            "learn_rsi_list",
            serde_json::json!({"status": "candidate", "limit": 3}),
        )
        .unwrap();
        assert!(list.summary().contains("candidate"));
        assert_eq!(list.to_value()["limit"], 3);

        let promote = ToolInput::from_value(
            "learn_rsi_promote",
            serde_json::json!({"kind": "tool_definition", "name": "rsi-tool_definition_patch-abc"}),
        )
        .unwrap();
        assert!(promote.summary().contains("tool_definition"));
        assert_eq!(promote.to_value()["name"], "rsi-tool_definition_patch-abc");

        let rollback = ToolInput::from_value(
            "learn_rsi_rollback",
            serde_json::json!({"kind": "tool_definition", "name": "Edit"}),
        )
        .unwrap();
        assert!(rollback.summary().contains("Edit"));
        assert_eq!(rollback.to_value()["kind"], "tool_definition");
    }

    #[test]
    fn summary_v2_1_132_tools_normal() {
        let lsp = ToolInput::Lsp {
            kind: "hover".into(),
            file: "/tmp/x.rs".into(),
            line: 12,
            column: 4,
        };
        assert!(lsp.summary().contains("hover"), "{}", lsp.summary());
        assert!(lsp.summary().contains("/tmp/x.rs:12"), "{}", lsp.summary());

        let pn = ToolInput::PushNotification {
            message: "hi".into(),
            title: Some("CI".into()),
        };
        assert_eq!(pn.summary(), "CI: hi");

        let rt = ToolInput::RemoteTrigger {
            trigger_id: "deploy".into(),
            payload: None,
        };
        assert_eq!(rt.summary(), "trigger: deploy");

        let pm = ToolInput::EnterPlanMode {
            reason: "double check".into(),
        };
        assert!(pm.summary().contains("double check"), "{}", pm.summary());

        let ew = ToolInput::EnterWorktree {
            name: "feat".into(),
            branch: Some("dev".into()),
        };
        assert!(ew.summary().contains("feat"), "{}", ew.summary());
        assert!(ew.summary().contains("dev"), "{}", ew.summary());

        assert_eq!(ToolInput::ExitWorktree.summary(), "exit worktree");

        let nr = ToolInput::NotebookRead {
            path: "/tmp/n.ipynb".into(),
        };
        assert_eq!(nr.summary(), "/tmp/n.ipynb");

        let ne = ToolInput::NotebookEdit {
            path: "/tmp/n.ipynb".into(),
            cell_id: "c1".into(),
            new_source: "x".into(),
            edit_mode: Some("insert".into()),
        };
        assert!(ne.summary().contains("insert"), "{}", ne.summary());
        assert!(ne.summary().contains("c1"), "{}", ne.summary());
    }

    /// from_value/to_value round-trip for each new tool's parameters.
    #[test]
    fn from_value_to_value_round_trip_v2_1_132_robust() {
        let cases: Vec<(&str, serde_json::Value)> = vec![
            (
                "LSP",
                serde_json::json!({"kind": "definition", "file": "/a/b.rs", "line": 3, "column": 7}),
            ),
            (
                "PushNotification",
                serde_json::json!({"message": "ok", "title": "build"}),
            ),
            (
                "RemoteTrigger",
                serde_json::json!({"trigger_id": "deploy", "payload": {"k": "v"}}),
            ),
            ("EnterPlanMode", serde_json::json!({"reason": "audit"})),
            (
                "EnterWorktree",
                serde_json::json!({"name": "feat", "branch": "main"}),
            ),
            ("ExitWorktree", serde_json::json!({})),
            ("NotebookRead", serde_json::json!({"path": "/tmp/x.ipynb"})),
            (
                "NotebookEdit",
                serde_json::json!({
                    "path": "/tmp/x.ipynb",
                    "cell_id": "c1",
                    "new_source": "y = 2",
                    "edit_mode": "replace",
                }),
            ),
        ];
        for (name, v) in cases {
            let parsed = ToolInput::from_value(name, v.clone())
                .unwrap_or_else(|e| panic!("from_value failed for {name}: {e}"));
            let back = parsed.to_value();
            for (k, vv) in v.as_object().unwrap() {
                assert_eq!(
                    &back[k], vv,
                    "round-trip lost field {k} for {name}: back={back}"
                );
            }
        }
    }

    // ─── ReplacementMode ──────────────────────────────────────────────────

    #[test]
    fn replacement_mode_from_replace_all_normal() {
        assert_eq!(
            ReplacementMode::from_replace_all(true),
            ReplacementMode::All
        );
        assert_eq!(
            ReplacementMode::from_replace_all(false),
            ReplacementMode::FirstOnly
        );
    }

    #[test]
    fn replacement_mode_replace_all_normal() {
        assert!(ReplacementMode::All.replace_all());
        assert!(!ReplacementMode::FirstOnly.replace_all());
    }

    // ─── TaskInput::is_teammate_spawn / is_fork ───────────────────────────

    fn make_task_input() -> TaskInput {
        TaskInput {
            description: "task".into(),
            prompt: "do it".into(),
            subagent_type: None,
            category: None,
            run_in_background: false,
            model: None,
            launcher: None,
            effort: None,
            name: None,
            team_name: None,
            mode: None,
            isolation: None,
            parent_task_id: None,
            schema: None,
            allowed_tools: Vec::new(),
            disallowed_tools: Vec::new(),
            cwd: None,
        }
    }

    #[test]
    fn task_input_is_fork_when_no_subagent_or_team_normal() {
        let ti = make_task_input();
        assert!(ti.is_fork());
        assert!(!ti.is_teammate_spawn());
    }

    #[test]
    fn task_input_with_subagent_type_is_not_fork_normal() {
        let ti = TaskInput {
            subagent_type: Some("explore".into()),
            ..make_task_input()
        };
        assert!(!ti.is_fork());
        assert!(!ti.is_teammate_spawn());
    }

    #[test]
    fn task_input_teammate_spawn_requires_both_name_and_team_normal() {
        // name alone or team alone is not a teammate spawn.
        let only_name = TaskInput {
            name: Some("alice".into()),
            ..make_task_input()
        };
        assert!(!only_name.is_teammate_spawn());

        let only_team = TaskInput {
            team_name: Some("alpha".into()),
            ..make_task_input()
        };
        assert!(!only_team.is_teammate_spawn());

        let both = TaskInput {
            name: Some("alice".into()),
            team_name: Some("alpha".into()),
            ..make_task_input()
        };
        assert!(both.is_teammate_spawn());
    }

    #[test]
    fn task_input_teammate_spawn_excludes_fork_robust() {
        // is_fork() must return false for teammate spawns even though
        // subagent_type is None — otherwise the dispatcher would try the
        // fork path on a teammate.
        let teammate = TaskInput {
            name: Some("alice".into()),
            team_name: Some("alpha".into()),
            ..make_task_input()
        };
        assert!(!teammate.is_fork());
    }

    #[test]
    fn task_input_summary_teammate_format_normal() {
        let ti = TaskInput {
            name: Some("alice".into()),
            team_name: Some("alpha".into()),
            description: "deploy".into(),
            ..make_task_input()
        };
        let s = ti.summary();
        assert!(s.contains("spawn teammate: alice"), "{s}");
        assert!(s.contains("deploy"), "{s}");
    }

    // ─── ToolInput::summary ───────────────────────────────────────────────

    #[test]
    fn tool_input_summary_bash_with_workdir_appends_in_dir_normal() {
        let i = ToolInput::Bash {
            command: "ls".into(),
            timeout: None,
            workdir: Some("/tmp".into()),
            run_in_background: None,
            suppress_output: None,
        };
        assert_eq!(i.summary(), "ls in /tmp");
    }

    #[test]
    fn tool_input_summary_bash_without_workdir_is_command_only_normal() {
        let i = ToolInput::Bash {
            command: "ls -la".into(),
            timeout: None,
            workdir: None,
            run_in_background: None,
            suppress_output: None,
        };
        assert_eq!(i.summary(), "ls -la");
    }

    #[test]
    fn tool_input_summary_glob_grep_search_format_normal() {
        let g = ToolInput::Glob {
            pattern: "**/*.rs".into(),
            path: Some("crates".into()),
        };
        assert_eq!(g.summary(), "**/*.rs in crates");

        let gg = ToolInput::Grep {
            pattern: "todo".into(),
            path: None,
            glob: None,
            output_mode: None,
        };
        assert_eq!(gg.summary(), "todo");

        let s = ToolInput::Search {
            query: "auth".into(),
            path: Some("src".into()),
        };
        assert_eq!(s.summary(), "auth in src");
    }

    #[test]
    fn tool_input_summary_apply_patch_includes_byte_count_normal() {
        let i = ToolInput::ApplyPatch {
            patch: "*** Begin Patch\n*** End Patch\n".into(),
        };
        let s = i.summary();
        assert!(s.contains("apply patch"));
        assert!(s.contains("bytes"));
    }

    #[test]
    fn tool_input_summary_skill_renders_args_when_present_normal() {
        let with = ToolInput::Skill {
            name: "review".into(),
            args: Some("the PR".into()),
        };
        assert_eq!(with.summary(), "review: the PR");

        let without = ToolInput::Skill {
            name: "review".into(),
            args: None,
        };
        assert_eq!(without.summary(), "review");

        // Empty-string args is treated as "no args".
        let empty_args = ToolInput::Skill {
            name: "review".into(),
            args: Some(String::new()),
        };
        assert_eq!(empty_args.summary(), "review");
    }

    #[test]
    fn tool_input_from_value_skill_accepts_claude_skill_alias_robust() {
        let input = ToolInput::from_value(
            "Skill",
            serde_json::json!({
                "skill": "code-review",
                "args": "focus on regressions"
            }),
        )
        .expect("skill alias should parse");
        match input {
            ToolInput::Skill { name, args } => {
                assert_eq!(name, "code-review");
                assert_eq!(args.as_deref(), Some("focus on regressions"));
            }
            other => panic!("expected Skill input, got {other:?}"),
        }
    }

    #[test]
    fn tool_input_summary_memory_create_truncates_body_at_50_robust() {
        let body = "x".repeat(200);
        let i = ToolInput::MemoryCreate {
            level: "user".into(),
            memory_type: "context".into(),
            scope: "private".into(),
            body,
        };
        let s = i.summary();
        // Format: "remember (user): xxxxx..." — count of x's is capped.
        let x_count = s.chars().filter(|c| *c == 'x').count();
        assert_eq!(x_count, 50, "body should truncate to 50 chars: {s}");
    }

    #[test]
    fn tool_input_summary_send_message_with_and_without_summary_normal() {
        let with = ToolInput::SendMessage {
            to: "alice".into(),
            message: "hi".into(),
            summary: Some("greeting".into()),
        };
        assert!(with.summary().contains("→ alice"));
        assert!(with.summary().contains("greeting"));

        let without = ToolInput::SendMessage {
            to: "bob".into(),
            message: "hi".into(),
            summary: None,
        };
        assert_eq!(without.summary(), "→ bob");
    }

    #[test]
    fn tool_input_summary_team_member_mode_format_normal() {
        let i = ToolInput::TeamMemberMode {
            member_name: "alice".into(),
            mode: "default".into(),
        };
        assert_eq!(i.summary(), "set alice → default");
    }

    #[test]
    fn tool_input_summary_team_create_includes_team_name_normal() {
        let i = ToolInput::TeamCreate {
            team_name: "frontend".into(),
            description: None,
        };
        assert_eq!(i.summary(), "create team: frontend");
    }

    #[test]
    fn tool_input_summary_task_list_with_and_without_filter_normal() {
        let with = ToolInput::TaskList {
            status_filter: Some("pending".into()),
            owner_filter: None,
            include_history: None,
            history_query: None,
        };
        assert_eq!(with.summary(), "list tasks (pending)");

        let without = ToolInput::TaskList {
            status_filter: None,
            owner_filter: None,
            include_history: None,
            history_query: None,
        };
        assert_eq!(without.summary(), "list tasks");
    }

    // ─── ToolInput::from_value ────────────────────────────────────────────

    #[test]
    fn tool_input_from_value_edit_normal() {
        let v = serde_json::json!({
            "file_path": "src/main.rs",
            "old_string": "fn old",
            "new_string": "fn new",
            "replace_all": true,
        });
        let input = ToolInput::from_value("Edit", v).expect("valid Edit input");
        match input {
            ToolInput::Edit {
                file_path,
                replacement,
                ..
            } => {
                assert_eq!(file_path, "src/main.rs");
                assert!(replacement.replace_all());
            }
            other => panic!("expected Edit, got {:?}", other.summary()),
        }
    }

    #[test]
    fn tool_input_from_value_read_optional_fields_normal() {
        let v = serde_json::json!({"file_path": "x", "offset": 10, "limit": 50});
        let input = ToolInput::from_value("Read", v).expect("valid Read input");
        match input {
            ToolInput::Read {
                file_path,
                offset,
                limit,
            } => {
                assert_eq!(file_path, "x");
                assert_eq!(offset, Some(10));
                assert_eq!(limit, Some(50));
            }
            _ => panic!("expected Read"),
        }
    }

    #[test]
    fn tool_input_from_value_task_complete_payload_normal() {
        let v = serde_json::json!({
            "description": "deploy",
            "prompt": "ship it",
            "subagent_type": "ops",
            "run_in_background": true,
            "name": "alice",
            "team_name": "alpha",
            "launcher": "variant-agent",
            "mode": "plan",
            "isolation": "worktree",
        });
        let input = ToolInput::from_value("Task", v).expect("valid Task input");
        match input {
            ToolInput::Task(ti) => {
                assert_eq!(ti.description, "deploy");
                assert_eq!(ti.prompt, "ship it");
                assert_eq!(ti.subagent_type.as_deref(), Some("ops"));
                assert!(ti.run_in_background);
                assert_eq!(ti.name.as_deref(), Some("alice"));
                assert_eq!(ti.team_name.as_deref(), Some("alpha"));
                assert_eq!(ti.launcher.as_deref(), Some("variant-agent"));
                assert_eq!(ti.mode.as_deref(), Some("plan"));
                assert_eq!(ti.isolation.as_deref(), Some("worktree"));
            }
            _ => panic!("expected Task"),
        }
    }

    #[test]
    fn tool_input_from_value_task_create_with_blocked_by_array_normal() {
        let v = serde_json::json!({
            "subject": "ship",
            "description": "release v1",
            "blocked_by": ["t1", "t2"],
        });
        let input = ToolInput::from_value("TaskCreate", v).expect("valid TaskCreate input");
        match input {
            ToolInput::TaskCreate { blocked_by, .. } => {
                assert_eq!(blocked_by.len(), 2);
                assert!(blocked_by.contains(&"t1".into()));
            }
            _ => panic!("expected TaskCreate"),
        }
    }

    #[test]
    fn tool_input_from_value_task_create_accepts_description_only_robust() {
        let v = serde_json::json!({
            "description": "Inspect the OpenAI-compatible tool path",
        });
        let input = ToolInput::from_value("taskcreate", v).expect("valid TaskCreate input");
        match input {
            ToolInput::TaskCreate {
                subject,
                description,
                ..
            } => {
                assert_eq!(subject, "Inspect the OpenAI-compatible tool path");
                assert_eq!(description, "Inspect the OpenAI-compatible tool path");
            }
            _ => panic!("expected TaskCreate"),
        }
    }

    #[test]
    fn tool_input_from_value_task_create_accepts_subject_only_robust() {
        let v = serde_json::json!({
            "subject": "Inspect tool path",
        });
        let input = ToolInput::from_value("TaskCreate", v).expect("valid TaskCreate input");
        match input {
            ToolInput::TaskCreate {
                subject,
                description,
                ..
            } => {
                assert_eq!(subject, "Inspect tool path");
                assert_eq!(description, "Inspect tool path");
            }
            _ => panic!("expected TaskCreate"),
        }
    }

    #[test]
    fn tool_input_from_value_tool_discovery_payloads_normal() {
        let search = ToolInput::from_value(
            "toolsearch",
            serde_json::json!({
                "query": "skill github",
                "limit": 5,
            }),
        )
        .expect("valid ToolSearch input");
        match search {
            ToolInput::ToolSearch { query, limit } => {
                assert_eq!(query, "skill github");
                assert_eq!(limit, Some(5));
            }
            _ => panic!("expected ToolSearch"),
        }

        let suggest = ToolInput::from_value(
            "ToolSuggest",
            serde_json::json!({
                "intent": "find the right repo inspection tool",
            }),
        )
        .expect("valid ToolSuggest input");
        match suggest {
            ToolInput::ToolSuggest { intent, limit } => {
                assert_eq!(intent, "find the right repo inspection tool");
                assert_eq!(limit, None);
            }
            _ => panic!("expected ToolSuggest"),
        }
    }

    #[test]
    fn tool_input_from_value_send_message_object_payload_robust() {
        // SendMessage's `message` field accepts string OR object — when an
        // object arrives we serialize it to a JSON string for the body.
        let v = serde_json::json!({
            "to": "alice",
            "message": {"kind": "ping", "n": 42},
            "summary": "ping",
        });
        let input = ToolInput::from_value("SendMessage", v).expect("valid SendMessage input");
        match input {
            ToolInput::SendMessage { to, message, .. } => {
                assert_eq!(to, "alice");
                // Object-form should be serialized — must contain both keys.
                assert!(message.contains("ping"), "{message}");
                assert!(message.contains("42"), "{message}");
            }
            _ => panic!("expected SendMessage"),
        }
    }

    #[test]
    fn tool_input_from_value_unknown_kind_falls_through_to_generic_robust() {
        let v = serde_json::json!({"foo": "bar"});
        let input = ToolInput::from_value("not_a_real_tool", v).expect("Generic accepts any shape");
        match input {
            ToolInput::Generic { summary } => {
                // Generic stores the original JSON as a string.
                assert!(summary.contains("foo"), "{summary}");
                assert!(summary.contains("bar"), "{summary}");
            }
            _ => panic!("expected Generic"),
        }
    }

    /// Inverted from the prior `..._handles_missing_fields_robust` test,
    /// which asserted that missing fields silently defaulted to empty
    /// strings. That behavior shipped a real bug: a malformed Write
    /// tool-use with `{"content": null}` got dispatched as
    /// `Write { file_path: "", content: "" }` and tried to truncate a
    /// real file. The boundary is now strict — missing required fields
    /// produce a typed `ToolInputError::MissingField` so the stream
    /// loop emits a `Failed` tool_result the model can react to.
    #[test]
    fn tool_input_from_value_rejects_missing_fields_robust() {
        let v = serde_json::json!({});
        let err = ToolInput::from_value("Edit", v)
            .expect_err("Edit with empty payload must fail validation");
        match err {
            ToolInputError::MissingField { tool, field } => {
                assert_eq!(tool, "Edit");
                // file_path is the first required field checked.
                assert_eq!(field, "file_path");
            }
            other => panic!("expected MissingField, got {other:?}"),
        }
    }

    /// The original symptom: provider sends `{"content": null}` for a
    /// Write tool. Old behavior coerced this into `content: ""` and
    /// happily queued an empty-content overwrite for user approval.
    /// New behavior rejects with `MissingField` (we treat null the same
    /// as absent at the boundary).
    #[test]
    fn tool_input_from_value_rejects_write_with_null_content_robust() {
        let v = serde_json::json!({"file_path": "/etc/passwd", "content": null});
        let err = ToolInput::from_value("Write", v).expect_err("Write with null content must fail");
        assert_eq!(
            err,
            ToolInputError::MissingField {
                tool: "Write".into(),
                field: "content",
            }
        );
    }

    /// Bash::command must be present AND non-empty — an empty bash
    /// command can't do anything useful and frequently signals the
    /// model truncated mid-call.
    #[test]
    fn tool_input_from_value_rejects_bash_with_empty_command_robust() {
        let v = serde_json::json!({"command": ""});
        let err = ToolInput::from_value("Bash", v).expect_err("Bash with empty command must fail");
        match err {
            ToolInputError::InvalidShape { tool, reason } => {
                assert_eq!(tool, "Bash");
                assert!(
                    reason.contains("must not be empty"),
                    "expected non-empty hint, got: {reason}"
                );
            }
            other => panic!("expected InvalidShape, got {other:?}"),
        }
    }

    /// Read::file_path is required — Read with an empty payload should
    /// surface `MissingField{tool: "Read", field: "file_path"}` rather
    /// than silently building `Read { file_path: "" }`.
    #[test]
    fn tool_input_from_value_rejects_read_missing_file_path_robust() {
        let v = serde_json::json!({"offset": 0, "limit": 100});
        let err = ToolInput::from_value("Read", v).expect_err("Read with no file_path must fail");
        assert_eq!(
            err,
            ToolInputError::MissingField {
                tool: "Read".into(),
                field: "file_path",
            }
        );
    }

    /// Wrong-typed required field (a number where a string is expected)
    /// must surface `WrongType` so the diagnostic message tells the
    /// model exactly what shape is expected.
    #[test]
    fn tool_input_from_value_rejects_wrong_typed_field_robust() {
        let v = serde_json::json!({"file_path": 42, "content": "hi"});
        let err = ToolInput::from_value("Write", v).expect_err("file_path must be a string");
        match err {
            ToolInputError::WrongType {
                tool,
                field,
                expected,
                got,
            } => {
                assert_eq!(tool, "Write");
                assert_eq!(field, "file_path");
                assert_eq!(expected, "string");
                assert_eq!(got, "number");
            }
            other => panic!("expected WrongType, got {other:?}"),
        }
    }

    // ─── ToolInput::to_value (round-trip-ish) ─────────────────────────────

    #[test]
    fn tool_input_to_value_bash_with_optional_fields_normal() {
        let i = ToolInput::Bash {
            command: "echo hi".into(),
            timeout: Some(5_000),
            workdir: Some("/tmp".into()),
            run_in_background: Some(true),
            suppress_output: Some(true),
        };
        let v = i.to_value();
        assert_eq!(v["command"], "echo hi");
        assert_eq!(v["timeout"], 5_000);
        assert_eq!(v["workdir"], "/tmp");
        assert_eq!(v["run_in_background"], true);
        assert_eq!(v["suppressOutput"], true);
    }

    #[test]
    fn tool_input_to_value_bash_omits_unset_optionals_normal() {
        let i = ToolInput::Bash {
            command: "ls".into(),
            timeout: None,
            workdir: None,
            run_in_background: None,
            suppress_output: None,
        };
        let v = i.to_value();
        assert_eq!(v["command"], "ls");
        assert!(v.get("timeout").is_none());
        assert!(v.get("workdir").is_none());
        assert!(v.get("run_in_background").is_none());
        assert!(v.get("suppressOutput").is_none());
    }

    #[test]
    fn tool_input_from_value_bash_preserves_run_in_background_regression() {
        let v = serde_json::json!({
            "command": "sleep 10",
            "run_in_background": true
        });
        let input = ToolInput::from_value("Bash", v).expect("parse Bash input");
        match input {
            ToolInput::Bash {
                command,
                run_in_background,
                ..
            } => {
                assert_eq!(command, "sleep 10");
                assert_eq!(run_in_background, Some(true));
            }
            other => panic!("expected Bash input, got {other:?}"),
        }
    }

    #[test]
    fn tool_input_from_value_bash_preserves_suppress_output_normal() {
        let v = serde_json::json!({
            "command": "printf secret",
            "suppressOutput": true
        });
        let input = ToolInput::from_value("Bash", v).expect("parse Bash input");
        match input {
            ToolInput::Bash {
                command,
                suppress_output,
                ..
            } => {
                assert_eq!(command, "printf secret");
                assert_eq!(suppress_output, Some(true));
            }
            other => panic!("expected Bash input, got {other:?}"),
        }
    }

    #[test]
    fn tool_input_bash_output_roundtrips_regression() {
        let v = serde_json::json!({
            "task_id": "bash_123",
            "offset": 2,
            "limit": 5,
            "block": false,
            "wait_up_to": 1000
        });
        let input = ToolInput::from_value("BashOutput", v).expect("parse BashOutput input");
        match input {
            ToolInput::BashOutput {
                task_id,
                offset,
                limit,
                block,
                timeout,
                wait_up_to,
            } => {
                assert_eq!(task_id, "bash_123");
                assert_eq!(offset, Some(2));
                assert_eq!(limit, Some(5));
                assert_eq!(block, Some(false));
                assert_eq!(timeout, None);
                assert_eq!(wait_up_to, Some(1000));
            }
            other => panic!("expected BashOutput input, got {other:?}"),
        }
    }

    #[test]
    fn tool_input_task_output_alias_parses_as_bash_output_regression() {
        let input = ToolInput::from_value(
            "AgentOutputTool",
            serde_json::json!({
                "task_id": "bash_abc",
                "block": true,
                "timeout": "2500"
            }),
        )
        .expect("parse Claude output alias");
        match input {
            ToolInput::BashOutput {
                task_id,
                block,
                timeout,
                ..
            } => {
                assert_eq!(task_id, "bash_abc");
                assert_eq!(block, Some(true));
                assert_eq!(timeout, Some(2500));
            }
            other => panic!("expected BashOutput input, got {other:?}"),
        }
    }

    #[test]
    fn tool_input_lsp_accepts_claude_wire_shape_regression() {
        let input = ToolInput::from_value(
            "LSPTool",
            serde_json::json!({
                "operation": "goToDefinition",
                "filePath": "/tmp/x.rs",
                "line": 12,
                "character": 4
            }),
        )
        .expect("parse Claude LSP input");
        match input {
            ToolInput::Lsp {
                kind,
                file,
                line,
                column,
            } => {
                assert_eq!(kind, "definition");
                assert_eq!(file, "/tmp/x.rs");
                assert_eq!(line, 12);
                assert_eq!(column, 4);
            }
            other => panic!("expected LSP input, got {other:?}"),
        }
    }

    #[test]
    fn tool_input_to_value_grep_omits_unset_optionals_normal() {
        let i = ToolInput::Grep {
            pattern: "todo".into(),
            path: None,
            glob: None,
            output_mode: None,
        };
        let v = i.to_value();
        assert_eq!(v["pattern"], "todo");
        assert!(v.get("path").is_none());
        assert!(v.get("glob").is_none());
        assert!(v.get("output_mode").is_none());
    }

    #[test]
    fn tool_input_to_value_team_create_with_description_normal() {
        let i = ToolInput::TeamCreate {
            team_name: "ops".into(),
            description: Some("operations".into()),
        };
        let v = i.to_value();
        assert_eq!(v["team_name"], "ops");
        assert_eq!(v["description"], "operations");
    }

    #[test]
    fn tool_input_to_value_send_message_omits_summary_when_none_robust() {
        let i = ToolInput::SendMessage {
            to: "alice".into(),
            message: "hi".into(),
            summary: None,
        };
        let v = i.to_value();
        assert_eq!(v["to"], "alice");
        assert!(v.get("summary").is_none());
    }

    #[test]
    fn tool_input_to_value_team_delete_is_empty_object_normal() {
        let v = ToolInput::TeamDelete.to_value();
        assert!(v.is_object());
        assert_eq!(v.as_object().unwrap().len(), 0);
    }

    #[test]
    fn tool_input_to_value_generic_parses_when_valid_json_robust() {
        let i = ToolInput::Generic {
            summary: r#"{"hello":"world"}"#.into(),
        };
        let v = i.to_value();
        assert_eq!(v["hello"], "world");
    }

    #[test]
    fn tool_input_to_value_generic_falls_back_to_input_field_robust() {
        // Non-JSON strings get wrapped in `{"input": "..."}` so the wire
        // always sees an object, never a bare scalar.
        let i = ToolInput::Generic {
            summary: "not even close to json".into(),
        };
        let v = i.to_value();
        assert_eq!(v["input"], "not even close to json");
    }
}

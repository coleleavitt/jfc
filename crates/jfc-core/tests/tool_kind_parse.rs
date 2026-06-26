use jfc_core::ToolKind;

mod tests {
    use super::*;

    #[test]
    fn tool_kind_task_parses_from_string() {
        assert_eq!(ToolKind::from_name("Task"), ToolKind::Task);
        assert_eq!(ToolKind::from_name("task"), ToolKind::Task);
    }

    #[test]
    fn from_name_handles_lowercase_concat_robust() {
        assert!(matches!(
            ToolKind::from_name("taskcreate"),
            ToolKind::TaskCreate
        ));
        assert!(matches!(
            ToolKind::from_name("taskupdate"),
            ToolKind::TaskUpdate
        ));
        assert!(matches!(
            ToolKind::from_name("tasklist"),
            ToolKind::TaskList
        ));
        assert!(matches!(
            ToolKind::from_name("taskdone"),
            ToolKind::TaskDone
        ));
        assert!(matches!(
            ToolKind::from_name("applypatch"),
            ToolKind::ApplyPatch
        ));
        assert!(matches!(
            ToolKind::from_name("toolsearch"),
            ToolKind::ToolSearch
        ));
        assert!(matches!(
            ToolKind::from_name("toolsuggest"),
            ToolKind::ToolSuggest
        ));
    }

    #[test]
    fn from_name_resolves_claude_task_output_aliases_regression() {
        for name in [
            "TaskOutput",
            "TaskOutputTool",
            "AgentOutput",
            "AgentOutputTool",
            "BashOutput",
            "BashOutputTool",
            "bash_output",
        ] {
            assert!(
                matches!(ToolKind::from_name(name), ToolKind::BashOutput),
                "expected BashOutput for {name}"
            );
        }
    }

    #[test]
    fn from_name_resolves_claude_tool_suffix_aliases_normal() {
        assert!(matches!(ToolKind::from_name("LSPTool"), ToolKind::Lsp));
        assert!(matches!(
            ToolKind::from_name("ListMcpResourcesTool"),
            ToolKind::ListMcpResources
        ));
        assert!(matches!(
            ToolKind::from_name("ReadMcpResourceTool"),
            ToolKind::ReadMcpResource
        ));
    }

    // The PascalCase, snake_case, and lowercase-concat variants must all
    // resolve to the same kind so a session that switched providers
    // mid-conversation doesn't fragment tool history.
    #[test]
    fn from_name_normalizes_across_separators_normal() {
        for n in ["TaskCreate", "task_create", "taskcreate", "TASKCREATE"] {
            assert!(
                matches!(ToolKind::from_name(n), ToolKind::TaskCreate),
                "expected TaskCreate for {n}"
            );
        }
    }

    // Truly unknown names route to UnknownTool — distinct from Generic
    // (which is for deliberately-named tools whose semantics we know
    // but don't represent as first-class variants). The variant exists
    // so adding a new ToolKind::Foo is a compile error at every match
    // site instead of a silent dispatch to Generic("Foo").
    #[test]
    fn from_name_unknown_falls_through_to_unknown_tool_robust() {
        match ToolKind::from_name("not_a_real_tool") {
            ToolKind::UnknownTool { advertised_name } => {
                assert_eq!(advertised_name, "not_a_real_tool")
            }
            other => panic!("expected UnknownTool, got {other:?}"),
        }
    }

    // MCP-namespaced names route to the Mcp variant carrying the full
    // advertised name.
    #[test]
    fn from_name_mcp_prefixed_routes_to_mcp_variant_normal() {
        match ToolKind::from_name("mcp__filesystem__read_file") {
            ToolKind::Mcp(s) => assert_eq!(s, "mcp__filesystem__read_file"),
            other => panic!("expected Mcp, got {other:?}"),
        }
    }

    #[test]
    fn from_name_codegraph_alias_routes_to_advertised_mcp_regression() {
        for name in ["codegraph_arch", "codegraph_explore", "codegraph_node"] {
            match ToolKind::from_name(name) {
                ToolKind::Mcp(s) => {
                    assert_eq!(s, format!("mcp__codegraph__{name}"));
                }
                other => panic!("expected Mcp for {name}, got {other:?}"),
            }
        }
    }

    #[test]
    fn from_name_mcp_without_separator_is_unknown_tool_robust() {
        // Without the `mcp__` prefix the name is just an unknown tool,
        // not an MCP-routed call.
        match ToolKind::from_name("mcp_dispatch") {
            ToolKind::UnknownTool { advertised_name } => {
                assert_eq!(advertised_name, "mcp_dispatch")
            }
            other => panic!("expected UnknownTool, got {other:?}"),
        }
    }

    // The 8 v2.1.132 tools must all parse from PascalCase and snake_case.
    #[test]
    fn from_name_resolves_v2_1_132_tools_normal() {
        assert!(matches!(ToolKind::from_name("LSP"), ToolKind::Lsp));
        assert!(matches!(ToolKind::from_name("lsp"), ToolKind::Lsp));
        assert!(matches!(
            ToolKind::from_name("PushNotification"),
            ToolKind::PushNotification
        ));
        assert!(matches!(
            ToolKind::from_name("push_notification"),
            ToolKind::PushNotification
        ));
        assert!(matches!(
            ToolKind::from_name("RemoteTrigger"),
            ToolKind::RemoteTrigger
        ));
        assert!(matches!(
            ToolKind::from_name("remote_trigger"),
            ToolKind::RemoteTrigger
        ));
        assert!(matches!(
            ToolKind::from_name("EnterPlanMode"),
            ToolKind::EnterPlanMode
        ));
        assert!(matches!(
            ToolKind::from_name("EnterWorktree"),
            ToolKind::EnterWorktree
        ));
        assert!(matches!(
            ToolKind::from_name("ExitWorktree"),
            ToolKind::ExitWorktree
        ));
        assert!(matches!(
            ToolKind::from_name("NotebookRead"),
            ToolKind::NotebookRead
        ));
        assert!(matches!(
            ToolKind::from_name("NotebookEdit"),
            ToolKind::NotebookEdit
        ));
    }

    #[test]
    fn label_v2_1_132_tools_normal() {
        assert_eq!(ToolKind::Lsp.label(), "LSP");
        assert_eq!(ToolKind::PushNotification.label(), "PushNotification");
        assert_eq!(ToolKind::RemoteTrigger.label(), "RemoteTrigger");
        assert_eq!(ToolKind::EnterPlanMode.label(), "EnterPlanMode");
        assert_eq!(ToolKind::EnterWorktree.label(), "EnterWorktree");
        assert_eq!(ToolKind::ExitWorktree.label(), "ExitWorktree");
        assert_eq!(ToolKind::NotebookRead.label(), "NotebookRead");
        assert_eq!(ToolKind::NotebookEdit.label(), "NotebookEdit");
    }

    #[test]
    fn api_name_v2_1_132_tools_normal() {
        assert_eq!(ToolKind::Lsp.api_name(), "LSP");
        assert_eq!(ToolKind::PushNotification.api_name(), "PushNotification");
        assert_eq!(ToolKind::RemoteTrigger.api_name(), "RemoteTrigger");
        assert_eq!(ToolKind::EnterPlanMode.api_name(), "EnterPlanMode");
        assert_eq!(ToolKind::EnterWorktree.api_name(), "EnterWorktree");
        assert_eq!(ToolKind::ExitWorktree.api_name(), "ExitWorktree");
        assert_eq!(ToolKind::NotebookRead.api_name(), "NotebookRead");
        assert_eq!(ToolKind::NotebookEdit.api_name(), "NotebookEdit");
    }

    /// The summary string is what shows in the tool row's right column.
    /// Each new tool needs a non-empty, distinguishable summary so the UI
    /// doesn't show identical placeholder strings for multiple calls.
    // ─── ToolKind labels & API names ──────────────────────────────────────

    #[test]
    fn tool_kind_label_returns_pascal_case_normal() {
        assert_eq!(ToolKind::Edit.label(), "Edit");
        assert_eq!(ToolKind::Write.label(), "Write");
        assert_eq!(ToolKind::Bash.label(), "Bash");
        assert_eq!(ToolKind::ApplyPatch.label(), "Patch");
        assert_eq!(ToolKind::Generic("Foo".into()).label(), "Foo");
    }

    #[test]
    fn tool_kind_api_name_for_search_uses_snake_case_normal() {
        // Search and ApplyPatch use snake_case on the wire even though
        // their display label is PascalCase. Mirrors v126's tool table.
        assert_eq!(ToolKind::Search.api_name(), "codebase_search");
        assert_eq!(ToolKind::ApplyPatch.api_name(), "apply_patch");
        assert_eq!(ToolKind::Edit.api_name(), "Edit");
    }

    #[test]
    fn from_name_resolves_rsi_learn_controls_normal() {
        assert!(matches!(
            ToolKind::from_name("learn_rsi_list"),
            ToolKind::LearnRsiList
        ));
        assert!(matches!(
            ToolKind::from_name("learn_rsi_promote"),
            ToolKind::LearnRsiPromote
        ));
        assert!(matches!(
            ToolKind::from_name("LearnRsiRollback"),
            ToolKind::LearnRsiRollback
        ));
        assert_eq!(ToolKind::LearnRsiList.api_name(), "learn_rsi_list");
        assert_eq!(ToolKind::LearnRsiPromote.api_name(), "learn_rsi_promote");
        assert_eq!(ToolKind::LearnRsiRollback.api_name(), "learn_rsi_rollback");
    }
}

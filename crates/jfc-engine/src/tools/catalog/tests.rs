use super::*;
use jfc_provider::{ProviderRole, ToolDef};

fn tool(name: &str, description: &str) -> ToolDef {
    ToolDef {
        name: name.to_owned(),
        description: description.to_owned(),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "query": { "type": "string" }
            }
        }),
    }
}

fn assistant_tool_use(id: &str, name: &str) -> ProviderMessage {
    ProviderMessage {
        role: ProviderRole::Assistant,
        content: vec![ProviderContent::ToolUse {
            id: id.to_owned(),
            name: name.to_owned(),
            input: serde_json::json!({}),
            thought_signature: None,
        }],
    }
}

fn user_tool_result(id: &str, body: &str) -> ProviderMessage {
    ProviderMessage {
        role: ProviderRole::User,
        content: vec![ProviderContent::ToolResult {
            tool_use_id: id.to_owned(),
            content: body.to_owned(),
            is_error: false,
        }],
    }
}

#[test]
fn progressive_catalog_keeps_core_tools_normal() {
    let all = vec![
        tool("Read", "read files"),
        tool("ToolSearch", "search tools"),
        tool("Task", "launch a subagent"),
        tool(
            "mcp__codegraph__codegraph_explore",
            "Explore code graph context",
        ),
        tool("run_coverage", "coverage reports"),
    ];

    let selected = progressive_tool_defs(all, &[], Some("hello"));
    let names: Vec<&str> = selected.iter().map(|tool| tool.name.as_str()).collect();

    assert!(names.contains(&"Read"));
    assert!(names.contains(&"ToolSearch"));
    assert!(names.contains(&"Task"));
    assert!(names.contains(&"mcp__codegraph__codegraph_explore"));
    assert!(!names.contains(&"run_coverage"));
}

#[test]
fn progressive_catalog_keeps_orchestration_tools_visible_regression() {
    let all = vec![
        tool("Task", "launch a subagent"),
        tool("TeamCreate", "create a team for multiple agents"),
        tool("SendMessage", "message a teammate"),
        tool("TeamMemberMode", "change teammate permission mode"),
        tool("Advisor", "consult the advisor model"),
        tool("Research", "run an agentic research pass"),
        tool("Council", "fan out a question to multiple models"),
        tool("AskModel", "ask a specific model"),
        tool("TeamDelete", "delete a team"),
    ];

    let selected = progressive_tool_defs(all, &[], Some("spin up a team of subagents"));
    let names: Vec<&str> = selected.iter().map(|tool| tool.name.as_str()).collect();

    for expected in [
        "Task",
        "TeamCreate",
        "SendMessage",
        "TeamMemberMode",
        "Advisor",
        "Research",
        "Council",
        "AskModel",
    ] {
        assert!(names.contains(&expected), "missing {expected}: {names:?}");
    }
    assert!(!names.contains(&"TeamDelete"));
}

#[test]
fn progressive_catalog_keeps_codegraph_tools_visible_regression() {
    let all = vec![
        tool("Read", "read files"),
        tool("mcp__codegraph__codegraph_search", "Search indexed symbols"),
        tool(
            "mcp__codegraph__codegraph_explore",
            "Explore related symbols and code",
        ),
        tool("run_coverage", "coverage reports"),
    ];

    let selected = progressive_tool_defs(all, &[], Some("fix this bug"));
    let names: Vec<&str> = selected.iter().map(|tool| tool.name.as_str()).collect();

    assert!(names.contains(&"mcp__codegraph__codegraph_search"));
    assert!(names.contains(&"mcp__codegraph__codegraph_explore"));
    assert!(!names.contains(&"run_coverage"));
}

#[test]
fn progressive_catalog_seeds_codegraph_before_builtin_tools_regression() {
    let all = vec![
        tool("Read", "read files"),
        tool("Grep", "search file contents"),
        tool("ToolSearch", "search tools"),
        tool(
            "mcp__codegraph__codegraph_explore",
            "Explore indexed source symbols",
        ),
        tool("Task", "launch a subagent"),
    ];

    let selected = progressive_tool_defs(all, &[], Some("dig into this bug"));
    let names: Vec<&str> = selected.iter().map(|tool| tool.name.as_str()).collect();

    assert_eq!(names.first(), Some(&"mcp__codegraph__codegraph_explore"));
    assert!(names.contains(&"Read"));
    assert!(names.contains(&"Grep"));
    assert!(names.contains(&"ToolSearch"));
    assert!(names.contains(&"Task"));
}

#[test]
fn progressive_catalog_reveals_tool_search_results_normal() {
    let all = vec![
        tool("Read", "read files"),
        tool("ToolSearch", "search tools"),
        tool("run_coverage", "coverage reports"),
    ];
    let messages = vec![
        assistant_tool_use("toolu_1", "ToolSearch"),
        user_tool_result(
            "toolu_1",
            "Matches for `coverage`:\n- tool `run_coverage`: Run cargo llvm-cov",
        ),
    ];

    let selected = progressive_tool_defs(all, &messages, None);
    let names: Vec<&str> = selected.iter().map(|tool| tool.name.as_str()).collect();

    assert!(names.contains(&"run_coverage"));
}

#[test]
fn progressive_catalog_keeps_all_historical_tool_names_regression() {
    let all = vec![
        tool("Read", "read files"),
        tool("ToolSearch", "search tools"),
        tool("WebSearch", "search the web for current information"),
        tool("run_coverage", "coverage reports"),
    ];
    let mut messages = Vec::new();
    messages.push(assistant_tool_use("toolu_old", "WebSearch"));
    for idx in 0..12 {
        messages.push(ProviderMessage {
            role: ProviderRole::User,
            content: vec![ProviderContent::Text(format!("turn {idx}"))],
        });
    }
    messages.push(assistant_tool_use("toolu_recent", "Read"));

    let selected = progressive_tool_defs(all, &messages, Some("continue"));
    let names: Vec<&str> = selected.iter().map(|tool| tool.name.as_str()).collect();

    assert!(
        names.contains(&"WebSearch"),
        "historical tool_use names must stay advertised on replay"
    );
    assert!(!names.contains(&"run_coverage"));
}

#[test]
fn historical_tool_count_does_not_starve_discovered_tools_regression() {
    let mut all = vec![
        tool("Read", "read files"),
        tool("ToolSearch", "search tools"),
        tool("run_coverage", "coverage reports"),
    ];
    let mut messages = vec![
        assistant_tool_use("toolu_search", "ToolSearch"),
        user_tool_result(
            "toolu_search",
            "Matches for `coverage`:\n- tool `run_coverage`: Run cargo llvm-cov",
        ),
    ];

    for idx in 0..32 {
        let name = format!("Historical{idx}");
        all.push(tool(&name, "already used in replayed history"));
        messages.push(assistant_tool_use(&format!("toolu_hist_{idx}"), &name));
    }

    let selected = progressive_tool_defs(all, &messages, Some("continue"));
    let names: Vec<&str> = selected.iter().map(|tool| tool.name.as_str()).collect();

    assert!(names.contains(&"run_coverage"));
}

#[test]
fn progressive_catalog_hides_runtime_output_tools_from_intent_regression() {
    let all = vec![
        tool("Bash", "run shell commands"),
        tool("BashOutput", "read output from a backgrounded Bash command"),
    ];

    let selected = progressive_tool_defs(all, &[], Some("wait for background bash output"));
    let names: Vec<&str> = selected.iter().map(|tool| tool.name.as_str()).collect();

    assert!(names.contains(&"Bash"));
    assert!(
        !names.contains(&"BashOutput"),
        "BashOutput is a hidden runtime compatibility tool, not a default model action"
    );
}

#[test]
fn progressive_catalog_hides_hidden_runtime_tools_from_history_regression() {
    let all = vec![
        tool("Bash", "run shell commands"),
        tool("BashOutput", "legacy background output retrieval"),
    ];
    let messages = vec![assistant_tool_use("toolu_old", "BashOutput")];

    let selected = progressive_tool_defs(all, &messages, Some("continue"));
    let names: Vec<&str> = selected.iter().map(|tool| tool.name.as_str()).collect();

    assert!(
        !names.contains(&"BashOutput"),
        "hidden runtime compatibility tools must not be re-advertised from replay"
    );
}

#[test]
fn progressive_catalog_selects_tools_from_intent_normal() {
    let all = vec![
        tool("Read", "read files"),
        tool("ToolSearch", "search tools"),
        tool("WebSearch", "search the web for current information"),
    ];

    let selected = progressive_tool_defs(all, &[], Some("search the web for docs"));
    let names: Vec<&str> = selected.iter().map(|tool| tool.name.as_str()).collect();

    assert!(names.contains(&"WebSearch"));
}

#[test]
fn progressive_catalog_does_not_suggest_commit_message_for_commit_action_regression() {
    let all = vec![
        tool("Bash", "run shell commands"),
        tool(
            "SuggestCommitMessage",
            "Persist one concise commit-message suggestion after inspecting the actual diff.",
        ),
    ];

    let selected = progressive_tool_defs(all, &[], Some("can you git commit and push please"));
    let names: Vec<&str> = selected.iter().map(|tool| tool.name.as_str()).collect();

    assert!(names.contains(&"Bash"));
    assert!(
        !names.contains(&"SuggestCommitMessage"),
        "commit-and-push execution should not advertise the commit-message suggestion tool"
    );
}

#[test]
fn progressive_catalog_suggests_commit_message_for_explicit_message_intent_normal() {
    let all = vec![
        tool("Bash", "run shell commands"),
        tool(
            "SuggestCommitMessage",
            "Persist one concise commit-message suggestion after inspecting the actual diff.",
        ),
    ];

    let selected = progressive_tool_defs(
        all,
        &[],
        Some("generate a conventional commit message for the staged diff"),
    );
    let names: Vec<&str> = selected.iter().map(|tool| tool.name.as_str()).collect();

    assert!(names.contains(&"SuggestCommitMessage"));
}

#[serial_test::serial(env)]
#[test]
fn env_limits_ignore_invalid_values_regression() {
    assert_eq!(env_usize("JFC_TEST_MISSING_LIMIT", 7), 7);

    unsafe { std::env::set_var("JFC_TEST_INVALID_LIMIT", "0") };
    assert_eq!(env_usize("JFC_TEST_INVALID_LIMIT", 7), 7);
    unsafe { std::env::set_var("JFC_TEST_INVALID_LIMIT", "bogus") };
    assert_eq!(env_usize("JFC_TEST_INVALID_LIMIT", 7), 7);
    unsafe { std::env::set_var("JFC_TEST_INVALID_LIMIT", "3") };
    assert_eq!(env_usize("JFC_TEST_INVALID_LIMIT", 7), 3);
    unsafe { std::env::remove_var("JFC_TEST_INVALID_LIMIT") };
}

#[test]
fn progressive_catalog_ranks_by_description_relevance_normal() {
    let all = vec![
        tool("Read", "read a file from disk"),
        tool("ToolSearch", "discover tools"),
        tool(
            "post_bounty",
            "register a coding bounty and let solver agents compete to win the reward",
        ),
        tool(
            "run_coverage",
            "annotate functions with test coverage hit counts",
        ),
    ];

    let selected = progressive_tool_defs(all, &[], Some("competition for a reward"));
    let names: Vec<&str> = selected.iter().map(|tool| tool.name.as_str()).collect();

    assert!(
        names.contains(&"post_bounty"),
        "TF-IDF should surface post_bounty by description relevance, got {names:?}"
    );
}

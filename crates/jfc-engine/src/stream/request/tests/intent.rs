use super::super::intent::{conversation_is_mid_tool_loop, user_text_requests_action};
use super::super::tools::preserve_non_action_tool;
use super::{user_text, user_tool_result};

#[test]
fn mid_tool_loop_detected_on_trailing_tool_result_normal() {
    let msgs = vec![
        user_text("what is ownership in rust"),
        user_tool_result("toolu_1"),
    ];
    assert!(conversation_is_mid_tool_loop(&msgs));
}

#[test]
fn plain_trailing_user_text_is_not_mid_loop_robust() {
    let msgs = vec![user_text("explain how borrowing works")];
    assert!(!conversation_is_mid_tool_loop(&msgs));
}

#[test]
fn empty_conversation_is_not_mid_loop_robust() {
    assert!(!conversation_is_mid_tool_loop(&[]));
}

#[test]
fn action_intent_detects_toolish_prompts_normal() {
    assert!(user_text_requests_action("read the file and trace the bug"));
    assert!(user_text_requests_action("continue please thank you"));
    assert!(user_text_requests_action("do all of the fixes please"));
    assert!(user_text_requests_action(
        "why is this bug happening read this session"
    ));
    assert!(user_text_requests_action(
        "what do you think of this codebase use codegraph and stuff"
    ));
    assert!(user_text_requests_action(
        "explain the architecture and use codegraph"
    ));
    assert!(user_text_requests_action(
        "see what websearch backends I have right"
    ));
    assert!(user_text_requests_action(
        "use primo please use the tool calls please"
    ));
}

#[test]
fn action_intent_leaves_plain_questions_alone_robust() {
    assert!(!user_text_requests_action("what is ownership in rust?"));
    assert!(!user_text_requests_action("explain how borrowing works"));
    assert!(!user_text_requests_action(
        "what is the use of lifetimes in rust?"
    ));
    assert!(!user_text_requests_action("this is pretty wild right"));
    assert!(!user_text_requests_action("/help"));
    assert!(!user_text_requests_action("what is the rust memory model"));
    assert!(!user_text_requests_action(
        "explain how the os schedules threads"
    ));
}

#[test]
fn action_intent_keeps_tools_for_local_environment_questions_regression() {
    assert!(user_text_requests_action("tell me about my device"));
    assert!(user_text_requests_action("what are my system specs"));
    assert!(user_text_requests_action("describe this machine"));
    assert!(user_text_requests_action("what's installed on here"));
    assert!(user_text_requests_action("tell me about this repo"));
    assert!(user_text_requests_action("what is this codebase"));
}

#[test]
fn non_action_catalog_keeps_discovery_tools_regression() {
    assert!(preserve_non_action_tool("ToolSearch"));
    assert!(preserve_non_action_tool("ToolSuggest"));
    assert!(preserve_non_action_tool("SendUserMessage"));
    assert!(preserve_non_action_tool("HcomList"));
    assert!(preserve_non_action_tool("HcomTranscript"));
    assert!(!preserve_non_action_tool("Bash"));
    assert!(!preserve_non_action_tool("HcomSend"));
    assert!(!preserve_non_action_tool("Read"));
    assert!(!preserve_non_action_tool("WebFetch"));
}

#[test]
fn preserve_non_action_keeps_codegraph_tools_regression() {
    assert!(preserve_non_action_tool(
        "mcp__codegraph__codegraph_explore"
    ));
    assert!(preserve_non_action_tool("mcp__codegraph__codegraph_search"));
    assert!(preserve_non_action_tool("codegraph_node"));
    assert!(!preserve_non_action_tool("mcp__github__create_issue"));
}

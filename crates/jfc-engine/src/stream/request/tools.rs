use crate::runtime::StreamToolChoice;

pub(super) fn preserve_non_action_tool(tool_name: &str) -> bool {
    // CodeGraph (read-only MCP code navigation) stays available on
    // informational turns: "how does X work" is exactly the question the
    // system prompt tells the model to answer with codegraph_explore. The
    // old behavior stripped every MCP tool here, which contradicted that
    // guidance and trained the model to answer structure questions from
    // memory or punt to Read/Bash on the next action turn.
    matches!(
        tool_name,
        "ToolSearch"
            | "ToolSuggest"
            | "SendUserMessage"
            | "HcomStatus"
            | "HcomList"
            | "HcomEvents"
            | "HcomTranscript"
            | "HcomBundle"
    ) || crate::tools::is_code_navigation_tool_name(tool_name)
}

pub(super) fn anthropic_tool_choice_value(_choice: StreamToolChoice) -> serde_json::Value {
    serde_json::json!({ "type": "auto" })
}

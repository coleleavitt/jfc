use crate::runtime::StreamToolChoice;

pub(super) fn preserve_non_action_tool(tool_name: &str) -> bool {
    matches!(
        tool_name,
        "ToolSearch"
            | "ToolSuggest"
            | "Task"
            | "Advisor"
            | "Research"
            | "Council"
            | "AskModel"
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

use jfc_provider::ToolDef;

use super::super::stream_context_budget;
use super::user_text;

#[test]
fn stream_context_budget_uses_actual_components_normal() {
    let tools = vec![ToolDef {
        name: "Read".into(),
        description: "read files".into(),
        input_schema: serde_json::json!({"properties":{"file_path":{"type":"string"}}}),
    }];
    let messages = vec![user_text("hello world")];
    let budget = stream_context_budget("system prompt plus memory", &tools, 6, &messages);
    assert_eq!(budget.memory_tokens, 1);
    assert!(budget.system_prompt_tokens > 0);
    assert!(budget.tool_definition_tokens > 0);
    assert!(budget.user_message_tokens > 0);
    assert!(
        jfc_core::context_budget::effective_tokens(budget)
            >= jfc_core::context_budget::raw_tokens(budget)
    );
}

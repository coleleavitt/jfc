use jfc_provider::{ProviderContent, ProviderMessage, ProviderRole, ToolDef};

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
    let budget = stream_context_budget("system prompt plus memory", &tools, 6, 0, &messages);
    assert_eq!(budget.memory_tokens, 1);
    assert!(budget.system_prompt_tokens > 0);
    assert!(budget.tool_definition_tokens > 0);
    assert!(budget.user_message_tokens > 0);
    assert!(
        jfc_core::context_budget::effective_tokens(budget)
            >= jfc_core::context_budget::raw_tokens(budget)
    );
}

#[test]
fn stream_context_budget_separates_memory_and_project_context_regression() {
    let tools = Vec::new();
    let messages = vec![user_text("hello world")];
    let budget = stream_context_budget(
        "base prompt remembered project rules",
        &tools,
        "remembered".len(),
        "project rules".len(),
        &messages,
    );

    assert_eq!(budget.memory_tokens, 2);
    assert_eq!(budget.project_instructions_tokens, 3);
    assert!(budget.system_prompt_tokens > 0);
}

#[test]
fn stream_context_budget_counts_provider_attachments_regression() {
    let messages = vec![ProviderMessage {
        role: ProviderRole::User,
        content: vec![
            ProviderContent::Text("abcd".into()),
            ProviderContent::Attachment(crate::attachments::Attachment {
                id: 1,
                kind: crate::attachments::AttachmentKind::ImagePng,
                bytes: vec![0; 12],
            }),
        ],
    }];

    let budget = stream_context_budget("", &[], 0, 0, &messages);

    assert_eq!(budget.user_message_tokens, 4);
}

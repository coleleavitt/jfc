use super::*;

fn user(text: &str) -> ProviderMessage {
    ProviderMessage {
        role: ProviderRole::User,
        content: vec![ProviderContent::Text(text.to_owned())],
    }
}

fn assistant(text: &str) -> ProviderMessage {
    ProviderMessage {
        role: ProviderRole::Assistant,
        content: vec![ProviderContent::Text(text.to_owned())],
    }
}

fn budget(window_tokens: usize) -> ProviderHistoryBudget {
    ProviderHistoryBudget {
        window_tokens,
        max_output_tokens: Some(128),
        overhead_tokens: 0,
    }
}

#[test]
fn compact_provider_history_replaces_prefix_with_bounded_block_regression() {
    let mut messages = Vec::new();
    for idx in 0..40 {
        messages.push(user(&format!("user {idx} {}", "u".repeat(200))));
        messages.push(assistant(&format!("assistant {idx} {}", "a".repeat(200))));
    }

    let transformed = compact_provider_history(&messages, budget(1_000))
        .expect("large provider replay should be transformable");

    assert!(transformed.messages.len() < messages.len());
    assert!(transformed.omitted_messages > 0);
    assert!(transformed.kept_messages >= MIN_TAIL_MESSAGES);
    assert!(transformed.kept_tokens < transformed.omitted_tokens);
    let first_text = transformed.messages[0]
        .content
        .iter()
        .find_map(|content| match content {
            ProviderContent::Text(text) => Some(text.as_str()),
            _ => None,
        })
        .expect("history block should be visible text");
    assert!(first_text.contains("<session-history compacted=\"true\""));
    assert!(first_text.contains("Recent omitted excerpts:"));
}

#[test]
fn compact_provider_history_prepends_block_to_user_tail_without_extra_turn_regression() {
    let mut messages = Vec::new();
    for idx in 0..20 {
        messages.push(user(&format!("user {idx} {}", "u".repeat(80))));
        messages.push(assistant(&format!("assistant {idx} {}", "a".repeat(80))));
    }

    let transformed = compact_provider_history(&messages, budget(800))
        .expect("large provider replay should be transformable");

    assert_eq!(transformed.messages[0].role, ProviderRole::User);
    assert!(
        transformed.messages[0]
            .content
            .iter()
            .any(|content| matches!(content, ProviderContent::Text(text) if text.contains("<session-history")))
    );
}

#[test]
fn compact_provider_history_includes_archive_handle_when_supplied_regression() {
    let mut messages = Vec::new();
    for idx in 0..20 {
        messages.push(user(&format!("user {idx} {}", "u".repeat(80))));
        messages.push(assistant(&format!("assistant {idx} {}", "a".repeat(80))));
    }

    let transformed =
        compact_provider_history_with_archive(&messages, budget(800), Some("provider-history-abc"))
            .expect("large provider replay should be transformable");
    let first_text = transformed.messages[0]
        .content
        .iter()
        .find_map(|content| match content {
            ProviderContent::Text(text) => Some(text.as_str()),
            _ => None,
        })
        .expect("history block should be visible text");

    assert!(first_text.contains("Provider-visible archive: `provider-history-abc`"));
    assert_eq!(
        transformed.archive_id.as_deref(),
        Some("provider-history-abc")
    );
}

#[test]
fn compact_provider_history_does_not_start_tail_with_orphan_tool_result_regression() {
    let mut messages = Vec::new();
    for idx in 0..18 {
        messages.push(user(&format!("user {idx} {}", "u".repeat(100))));
        messages.push(assistant(&format!("assistant {idx} {}", "a".repeat(100))));
    }
    messages.push(ProviderMessage {
        role: ProviderRole::User,
        content: vec![ProviderContent::ToolResult {
            tool_use_id: "toolu_old".to_owned(),
            content: "old result".to_owned(),
            is_error: false,
        }],
    });
    messages.push(assistant("fresh assistant"));
    messages.push(user("fresh user"));

    let transformed = compact_provider_history(&messages, budget(700))
        .expect("large provider replay should be transformable");

    assert!(
        !matches!(
            transformed.messages[0].content.first(),
            Some(ProviderContent::ToolResult { .. })
        ),
        "transformed replay must not begin with an orphan tool_result"
    );
}

#[test]
fn provider_payload_counts_attachments_regression() {
    let messages = vec![ProviderMessage {
        role: ProviderRole::User,
        content: vec![
            ProviderContent::Text("abcd".to_owned()),
            ProviderContent::Attachment(crate::attachments::Attachment {
                id: 1,
                kind: crate::attachments::AttachmentKind::ImagePng,
                bytes: vec![0; 12],
            }),
        ],
    }];

    assert_eq!(provider_messages_tokens(&messages), 4);
}

use super::{TranscriptBoundaryBudget, materialize_transcript_boundary};
use crate::types::ChatMessage;

fn huge_message(index: usize) -> ChatMessage {
    ChatMessage::user(format!("message-{index} {}", "x".repeat(20_000)))
}

#[test]
fn materialize_transcript_boundary_replaces_old_prefix_with_archive_regression() {
    let mut messages: Vec<ChatMessage> = (0..30).map(huge_message).collect();

    let result = materialize_transcript_boundary(
        &mut messages,
        TranscriptBoundaryBudget {
            window_tokens: 80_000,
            max_output_tokens: Some(8_000),
            overhead_tokens: 4_000,
        },
    )
    .expect("huge transcript should materialize a durable boundary");

    assert!(messages[0].is_compact_boundary());
    assert!(result.omitted_messages > 0);
    assert!(result.kept_messages >= 12);
    assert!(result.post_tokens < result.pre_tokens);

    let archive_id = result
        .archive_id
        .expect("omitted prefix should be archived");
    let rendered = crate::compact_archive::render_archive_by_id(&archive_id)
        .expect("archive id should render through /expand backend");
    assert!(rendered.contains("message-0"));
}

#[test]
fn materialize_transcript_boundary_leaves_small_transcript_untouched_normal() {
    let mut messages = vec![ChatMessage::user("small".to_owned())];

    let result = materialize_transcript_boundary(
        &mut messages,
        TranscriptBoundaryBudget {
            window_tokens: 80_000,
            max_output_tokens: Some(8_000),
            overhead_tokens: 4_000,
        },
    );

    assert!(result.is_none());
    assert_eq!(messages.len(), 1);
    assert!(!messages[0].is_compact_boundary());
}

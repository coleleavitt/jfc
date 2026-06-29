use crate::types::ChatMessage;

const CHARS_PER_TOKEN: usize = 4;

fn saturating_sum(values: impl Iterator<Item = usize>) -> usize {
    values.fold(0usize, usize::saturating_add)
}

pub(crate) fn message_visible_chars(message: &ChatMessage) -> usize {
    let part_chars = saturating_sum(message.parts.iter().map(|part| part.approx_text_len()));
    let attachment_chars = saturating_sum(message.attachments.iter().map(|att| att.bytes.len()));
    part_chars.saturating_add(attachment_chars)
}

pub(crate) fn transcript_visible_chars(messages: &[ChatMessage]) -> usize {
    saturating_sum(messages.iter().map(message_visible_chars))
}

pub(crate) fn estimate_transcript_tokens(messages: &[ChatMessage]) -> usize {
    let content_chars = transcript_visible_chars(messages);
    let base = content_chars / CHARS_PER_TOKEN;
    jfc_core::context_budget::with_overhead(base.try_into().unwrap_or(u64::MAX))
        .try_into()
        .unwrap_or(usize::MAX)
}

pub(crate) fn pending_turn_tokens(
    text: String,
    attachments: &[crate::attachments::Attachment],
    mention_attachments: &[crate::attachments::Attachment],
) -> usize {
    let mut message = ChatMessage::user(text);
    message
        .attachments
        .reserve(attachments.len().saturating_add(mention_attachments.len()));
    message.attachments.extend_from_slice(attachments);
    message.attachments.extend_from_slice(mention_attachments);
    estimate_transcript_tokens(&[message])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png_attachment(bytes: usize) -> crate::attachments::Attachment {
        crate::attachments::Attachment {
            id: 1,
            kind: crate::attachments::AttachmentKind::ImagePng,
            bytes: vec![0; bytes],
        }
    }

    #[test]
    fn estimate_transcript_tokens_counts_attachments_regression() {
        let mut message = ChatMessage::user("abcd".to_owned());
        message.attachments = vec![png_attachment(12)];

        assert_eq!(transcript_visible_chars(&[message.clone()]), 16);
        assert_eq!(estimate_transcript_tokens(&[message]), 6);
    }

    #[test]
    fn pending_turn_tokens_counts_pasted_and_mentioned_attachments_regression() {
        assert_eq!(
            pending_turn_tokens(
                "tiny".to_owned(),
                &[png_attachment(8)],
                &[png_attachment(12)]
            ),
            9
        );
    }
}

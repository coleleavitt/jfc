use crate::attachments::Attachment;
use crate::provider::ProviderContent;

pub(super) fn push_attachments(content: &mut Vec<ProviderContent>, attachments: &[Attachment]) {
    for att in attachments {
        content.push(ProviderContent::Attachment(att.clone()));
    }
}

#[cfg(test)]
mod tests {
    use crate::provider::{ProviderContent, ProviderRole};
    use crate::stream::messages::build_provider_messages;
    use crate::stream::messages::provider_messages::build_provider_messages_with_tool_results;
    use crate::types::ChatMessage;

    /// Normal: PDF on ChatMessage.attachments lands as ProviderContent::Attachment
    /// in build_provider_messages_with_tool_results. Per-message ownership —
    /// no global queue.
    #[test]
    fn per_message_pdf_lands_in_user_message_normal() {
        let mut user = ChatMessage::user("read this please".to_string());
        user.attachments = vec![crate::attachments::Attachment {
            id: 0,
            kind: crate::attachments::AttachmentKind::ApplicationPdf,
            bytes: b"%PDF-1.7\nfake".to_vec(),
        }];
        let msgs = vec![user];
        let provider_msgs = build_provider_messages_with_tool_results(&msgs);
        let last_user = provider_msgs
            .iter()
            .rfind(|m| matches!(m.role, ProviderRole::User))
            .expect("must have a user message");
        let attachment_count = last_user
            .content
            .iter()
            .filter(|c| matches!(c, ProviderContent::Attachment(_)))
            .count();
        assert_eq!(
            attachment_count, 1,
            "expected one attachment on the user message"
        );
    }

    /// Normal: pasted image on ChatMessage.attachments lands in the text-only
    /// build path.
    #[test]
    fn per_message_image_lands_in_text_only_build_normal() {
        let mut msg = ChatMessage::user("look at this [Image #1]".to_string());
        msg.attachments = vec![crate::attachments::Attachment {
            id: 1,
            kind: crate::attachments::AttachmentKind::ImagePng,
            bytes: vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A],
        }];
        let msgs = vec![msg];
        let provider_msgs = build_provider_messages(&msgs);
        let last_user = provider_msgs
            .iter()
            .rfind(|m| matches!(m.role, ProviderRole::User))
            .expect("must have a user message");
        let attachment_count = last_user
            .content
            .iter()
            .filter(|c| matches!(c, ProviderContent::Attachment(_)))
            .count();
        assert_eq!(
            attachment_count, 1,
            "expected one attachment in text-only path"
        );
    }

    /// Robust: a second call for the same messages does NOT produce a second
    /// copy of the attachment (no shared mutable queue to drain twice).
    #[test]
    fn second_build_does_not_duplicate_attachment_robust() {
        let mut user = ChatMessage::user("first".to_string());
        user.attachments = vec![crate::attachments::Attachment {
            id: 0,
            kind: crate::attachments::AttachmentKind::ApplicationPdf,
            bytes: b"%PDF-1.7\n".to_vec(),
        }];
        let msgs = vec![user];
        let first = build_provider_messages_with_tool_results(&msgs);
        let second = build_provider_messages_with_tool_results(&msgs);
        // Both runs should see exactly one attachment — no global state to drain.
        for round in [&first, &second] {
            let count = round
                .iter()
                .flat_map(|m| m.content.iter())
                .filter(|c| matches!(c, ProviderContent::Attachment(_)))
                .count();
            assert_eq!(count, 1, "each build should see exactly one attachment");
        }
    }
}

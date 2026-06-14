//! Attachment content-block serialization for Anthropic's API format.

use base64::Engine;
use jfc_core::Attachment;

/// Serialize an [`Attachment`] into the Anthropic content-block JSON format
/// (`{"type":"image","source":{"type":"base64","media_type":"…","data":"…"}}`).
pub fn to_anthropic_content_block(att: &Attachment) -> serde_json::Value {
    let data = base64::engine::general_purpose::STANDARD.encode(&att.bytes);
    let block_type = if att.kind.is_pdf() {
        "document"
    } else {
        "image"
    };
    serde_json::json!({
        "type": block_type,
        "source": {
            "type": "base64",
            "media_type": att.kind.mime_type(),
            "data": data,
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use jfc_core::AttachmentKind;

    #[test]
    fn content_block_shape_normal() {
        let att = Attachment {
            id: 1,
            bytes: vec![0x89, 0x50, 0x4E, 0x47],
            kind: AttachmentKind::ImagePng,
        };
        let block = to_anthropic_content_block(&att);
        assert_eq!(block["type"], "image");
        assert_eq!(block["source"]["type"], "base64");
        assert_eq!(block["source"]["media_type"], "image/png");
    }

    #[test]
    fn pdf_uses_document_type_normal() {
        let att = Attachment {
            id: 2,
            bytes: vec![0x25, 0x50, 0x44, 0x46],
            kind: AttachmentKind::ApplicationPdf,
        };
        let block = to_anthropic_content_block(&att);
        assert_eq!(block["type"], "document");
    }
}

//! Attachment content-block serialization for Anthropic's API format.

use base64::Engine;
use jfc_core::Attachment;

/// Serialize an [`Attachment`] into the Anthropic content-block JSON format
/// (`{"type":"image","source":{"type":"base64","media_type":"…","data":"…"}}`).
pub fn to_anthropic_content_block(att: &Attachment) -> serde_json::Value {
    let _linkscope_content = linkscope::phase("provider.content.to_anthropic_block");
    let data = base64::engine::general_purpose::STANDARD.encode(&att.bytes);
    let block_type = if att.kind.is_pdf() {
        "document"
    } else {
        "image"
    };
    trace_content_block(ContentBlockTrace {
        attachment_id: u64::from(att.id),
        input_bytes: att.bytes.len(),
        encoded_bytes: data.len(),
        block_type,
        media_type: att.kind.mime_type(),
    });
    serde_json::json!({
        "type": block_type,
        "source": {
            "type": "base64",
            "media_type": att.kind.mime_type(),
            "data": data,
        }
    })
}

struct ContentBlockTrace<'a> {
    attachment_id: u64,
    input_bytes: usize,
    encoded_bytes: usize,
    block_type: &'a str,
    media_type: &'a str,
}

fn trace_content_block(input: ContentBlockTrace<'_>) {
    linkscope::record_bytes(
        "provider.content.input.bytes",
        usize_to_u64_saturating(input.input_bytes),
    );
    linkscope::record_bytes(
        "provider.content.encoded.bytes",
        usize_to_u64_saturating(input.encoded_bytes),
    );
    if !linkscope::trace_detail_enabled() {
        return;
    }
    linkscope::detail_event_fields(
        "provider.content.block.detail",
        [
            linkscope::TraceField::count("attachment_id", input.attachment_id),
            linkscope::TraceField::bytes("input_bytes", usize_to_u64_saturating(input.input_bytes)),
            linkscope::TraceField::bytes(
                "encoded_bytes",
                usize_to_u64_saturating(input.encoded_bytes),
            ),
            linkscope::TraceField::text("block_type", input.block_type),
            linkscope::TraceField::text("media_type", input.media_type),
        ],
    );
}

fn usize_to_u64_saturating(value: usize) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
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

    #[test]
    fn content_trace_records_shape_without_base64_payload_normal() {
        linkscope::trace_detail_enable();
        let att = Attachment {
            id: 7,
            bytes: b"private attachment bytes".to_vec(),
            kind: AttachmentKind::ImagePng,
        };
        let block = to_anthropic_content_block(&att);
        assert_eq!(block["type"], "image");

        let encoded = block["source"]["data"].as_str().unwrap();
        let snapshot = linkscope::snapshot();
        let rendered = format!("{snapshot:?}");
        assert!(rendered.contains("provider.content.block.detail"));
        assert!(rendered.contains("input_bytes"));
        assert!(rendered.contains("image/png"));
        assert!(!rendered.contains(encoded));
        assert!(!rendered.contains("private attachment bytes"));
    }
}

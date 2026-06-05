/// Attachment media type. Covers image formats plus PDF documents.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttachmentKind {
    ImagePng,
    ImageJpeg,
    ImageGif,
    ImageWebp,
    ApplicationPdf,
}

impl AttachmentKind {
    /// MIME string used for HTTP/API content-block source metadata.
    pub fn mime_type(self) -> &'static str {
        match self {
            Self::ImagePng => "image/png",
            Self::ImageJpeg => "image/jpeg",
            Self::ImageGif => "image/gif",
            Self::ImageWebp => "image/webp",
            Self::ApplicationPdf => "application/pdf",
        }
    }

    /// Whether this kind should render as a document block rather than an
    /// image block in Anthropic-style provider payloads.
    pub fn is_pdf(self) -> bool {
        matches!(self, Self::ApplicationPdf)
    }
}

/// A staged attachment. Owns encoded bytes so provider request builders can
/// consume it without depending on UI clipboard or image-processing code.
#[derive(Debug, Clone)]
pub struct Attachment {
    pub id: u32,
    pub kind: AttachmentKind,
    pub bytes: Vec<u8>,
}

/// User-pasted content tracked before it is attached to a submitted message.
#[derive(Debug, Clone)]
pub struct PastedContent {
    pub id: u32,
    pub attachment: Attachment,
    pub width: u32,
    pub height: u32,
}

//! Image attachment data layer.
//!
//! Owns the in-memory representation of pasted/loaded image attachments and
//! the conversion to Anthropic Messages-API content blocks. The clipboard
//! reader is here too so future Ctrl+V handlers have a single entry point.
//!
//! Scope is intentionally narrow: this module does not touch the renderer,
//! the provider message builders, or the input keymap. It only provides:
//!   * `AttachmentKind` / `Attachment` data types
//!   * `detect_kind` magic-byte sniffing (pure, easily testable)
//!   * `read_clipboard_image` – wraps `arboard::Clipboard::get_image()`
//!     and re-encodes the raw RGBA pixels as PNG so the bytes are ready
//!     to drop straight into a base64 content block
//!   * `to_anthropic_content_block` – `{"type":"image","source":{...}}`
//!
//! Wiring this into provider requests is a follow-up task.
//!
//! # Why re-encode to PNG instead of trusting the clipboard?
//!
//! `arboard::Clipboard::get_image()` returns a decoded `ImageData` of
//! raw RGBA8 pixels regardless of the source format (X11 ICCCM, Wayland
//! data-control, NSPasteboard `NSImage`, Win32 CF_DIB). Anthropic's
//! Messages API needs a self-describing media type + base64 payload, so
//! we encode to PNG once at clipboard-read time and stash the bytes
//! verbatim. PNG is lossless and the smallest universally-supported
//! format the API accepts.

use base64::Engine as _;
pub use jfc_core::{Attachment, AttachmentKind, PastedContent};

/// Sniff the image format from the leading magic bytes.
///
/// Returns `None` for unknown formats *and* for buffers too short to make
/// a positive identification — the rule is "we either know, or we don't."
/// This keeps callers from accidentally classifying a 2-byte buffer as
/// JPEG just because the first two bytes happen to be `0xFF 0xD8`.
pub fn detect_kind(bytes: &[u8]) -> Option<AttachmentKind> {
    // PNG: 89 50 4E 47 (then 0D 0A 1A 0A, but the leading 4 are unique enough)
    if bytes.len() >= 4 && bytes.starts_with(&[0x89, b'P', b'N', b'G']) {
        return Some(AttachmentKind::ImagePng);
    }
    // JPEG: FF D8 FF (the fourth byte varies: E0 JFIF, E1 EXIF, DB raw, …)
    if bytes.len() >= 3 && bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        return Some(AttachmentKind::ImageJpeg);
    }
    // GIF: ASCII "GIF87a" or "GIF89a"
    if bytes.len() >= 6 && (bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a")) {
        return Some(AttachmentKind::ImageGif);
    }
    // WebP: "RIFF" <4-byte size> "WEBP"
    if bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        return Some(AttachmentKind::ImageWebp);
    }
    // PDF: ASCII `%PDF-` (the version follows: `1.4`, `1.7`, `2.0`).
    // 5 bytes is enough for a positive ID; the Anthropic API accepts
    // any standard-compliant PDF so we don't need to parse the version.
    if bytes.len() >= 5 && bytes.starts_with(b"%PDF-") {
        return Some(AttachmentKind::ApplicationPdf);
    }
    None
}

/// Read a PDF from disk and return it as an `Attachment`. Used by
/// the Read tool when the path's extension is `.pdf` so the file
/// content lands in the next message as a `document` block instead
/// of garbled binary text. Caps at 32 MiB — Anthropic rejects
/// larger payloads, and we'd rather fail fast than wait for the
/// 413 round-trip.
pub fn read_pdf_file(path: &std::path::Path) -> Result<Attachment, String> {
    const MAX_PDF_BYTES: u64 = 32 * 1024 * 1024;
    let metadata = std::fs::metadata(path).map_err(|e| format!("stat {}: {e}", path.display()))?;
    if metadata.len() > MAX_PDF_BYTES {
        return Err(format!(
            "PDF too large ({} bytes; cap is {} MiB)",
            metadata.len(),
            MAX_PDF_BYTES / 1024 / 1024
        ));
    }
    let bytes = std::fs::read(path).map_err(|e| format!("read {}: {e}", path.display()))?;
    if detect_kind(&bytes) != Some(AttachmentKind::ApplicationPdf) {
        return Err(format!(
            "{} does not start with `%PDF-` magic bytes",
            path.display()
        ));
    }
    tracing::info!(
        target: "jfc::attachments",
        path = %path.display(),
        size = bytes.len(),
        "read_pdf_file: loaded PDF"
    );
    Ok(Attachment {
        id: 0,
        kind: AttachmentKind::ApplicationPdf,
        bytes,
    })
}

/// Anthropic's per-image base64 payload cap. Their docs say "5 MB
/// when base64-encoded", which means the *raw* bytes must be ≤ ~3.75
/// MiB. We use a slightly conservative 3.5 MiB here so a small JSON
/// framing overhead can't push the wire payload past the limit.
///
/// `imageResizer.ts` in Claude Code 2.1.140 does the same downsample
/// (`/home/cole/RustProjects/active/claude-code-2.1.140-audit/extracted/src/utils/imageResizer.ts`),
/// halving dimensions until the encoded image fits.
const MAX_IMAGE_BYTES: usize = 3_750_000; // 5MB base64 / (4/3)

const MAX_IMAGE_DIMENSION: u32 = 2000;
const JPEG_QUALITIES: &[u8] = &[80, 60, 40, 20];

/// Extract width/height from encoded image bytes (PNG/JPEG/GIF/WebP).
pub fn image_dimensions(bytes: &[u8]) -> Result<(u32, u32), String> {
    let reader = image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|e| format!("image format detection: {e}"))?;
    let (w, h) = reader
        .into_dimensions()
        .map_err(|e| format!("image dimensions: {e}"))?;
    Ok((w, h))
}

/// Process raw image bytes: clamp to MAX_IMAGE_DIMENSION, encode as PNG,
/// fall back to JPEG at decreasing quality if PNG exceeds MAX_IMAGE_BYTES.
pub fn process_image(raw_bytes: Vec<u8>, _kind: AttachmentKind) -> Result<Attachment, String> {
    let img = image::load_from_memory(&raw_bytes).map_err(|e| format!("image decode: {e}"))?;

    // Clamp dimensions
    let (mut w, mut h) = (img.width(), img.height());
    let img = if w > MAX_IMAGE_DIMENSION || h > MAX_IMAGE_DIMENSION {
        let scale = (MAX_IMAGE_DIMENSION as f64 / w.max(h) as f64).min(1.0);
        let new_w = ((w as f64 * scale) as u32).max(1);
        let new_h = ((h as f64 * scale) as u32).max(1);
        tracing::debug!(
            target: "jfc::attachments",
            from = format!("{w}x{h}"),
            to = format!("{new_w}x{new_h}"),
            "clamping image dimensions"
        );
        w = new_w;
        h = new_h;
        img.resize_exact(new_w, new_h, image::imageops::FilterType::Lanczos3)
    } else {
        img
    };

    // Try PNG first
    let mut png_buf = Vec::new();
    {
        use image::ImageEncoder as _;
        let encoder = image::codecs::png::PngEncoder::new(&mut png_buf);
        encoder
            .write_image(
                img.to_rgba8().as_raw(),
                w,
                h,
                image::ExtendedColorType::Rgba8,
            )
            .map_err(|e| format!("PNG encode: {e}"))?;
    }

    if png_buf.len() <= MAX_IMAGE_BYTES {
        return Ok(Attachment {
            id: 0,
            kind: AttachmentKind::ImagePng,
            bytes: png_buf,
        });
    }

    // PNG too large — try JPEG at decreasing quality
    let rgb_img = img.to_rgb8();
    for &quality in JPEG_QUALITIES {
        let mut jpeg_buf = Vec::new();
        {
            use image::ImageEncoder as _;
            let encoder =
                image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg_buf, quality);
            encoder
                .write_image(rgb_img.as_raw(), w, h, image::ExtendedColorType::Rgb8)
                .map_err(|e| format!("JPEG encode (q={quality}): {e}"))?;
        }
        if jpeg_buf.len() <= MAX_IMAGE_BYTES {
            tracing::debug!(
                target: "jfc::attachments",
                quality,
                png_size = png_buf.len(),
                jpeg_size = jpeg_buf.len(),
                "fell back to JPEG"
            );
            return Ok(Attachment {
                id: 0,
                kind: AttachmentKind::ImageJpeg,
                bytes: jpeg_buf,
            });
        }
    }

    Err(format!(
        "image still {} bytes after JPEG q=20 — too large for the API",
        png_buf.len()
    ))
}

/// Try shell-based clipboard image acquisition (Linux).
/// Claude Code 2.1.140 uses this as the primary path on Linux because
/// arboard's X11/Wayland support is inconsistent across compositors.
fn read_clipboard_image_shell() -> Result<Option<Vec<u8>>, String> {
    // Try xclip first (X11)
    if let Ok(out) = std::process::Command::new("xclip")
        .args(["-selection", "clipboard", "-t", "image/png", "-o"])
        .output()
    {
        if out.status.success() && !out.stdout.is_empty() {
            return Ok(Some(out.stdout));
        }
    }
    // Try wl-paste (Wayland)
    if let Ok(out) = std::process::Command::new("wl-paste")
        .args(["--type", "image/png"])
        .output()
    {
        if out.status.success() && !out.stdout.is_empty() {
            return Ok(Some(out.stdout));
        }
    }
    // Try xsel (legacy X11 fallback)
    if let Ok(out) = std::process::Command::new("xsel")
        .args(["--clipboard", "--output"])
        .output()
    {
        if out.status.success()
            && !out.stdout.is_empty()
            && out.stdout.starts_with(&[0x89, b'P', b'N', b'G'])
        {
            return Ok(Some(out.stdout));
        }
    }
    Ok(None)
}

/// Read an image from the system clipboard and return it as a processed
/// `Attachment` along with its dimensions (width, height). Returns
/// `Ok(None)` if the clipboard contains no image (text, files, empty, …);
/// returns `Err(_)` for clipboard-access or encoding failures.
///
/// Tries shell-based clipboard tools first (xclip, wl-paste, xsel) for
/// reliability on Linux, then falls back to arboard.
///
/// Images are processed through `process_image` which clamps dimensions
/// to MAX_IMAGE_DIMENSION and encodes as PNG (falling back to JPEG if
/// the result exceeds MAX_IMAGE_BYTES).
pub fn read_clipboard_image() -> Result<Option<(Attachment, u32, u32)>, String> {
    tracing::info!(target: "jfc::attachments", "read_clipboard_image attempt");

    // Try shell-based acquisition first (more reliable on Linux)
    if let Ok(Some(png_bytes)) = read_clipboard_image_shell() {
        tracing::debug!(target: "jfc::attachments", size = png_bytes.len(), "shell clipboard image acquired");
        let (width, height) = image_dimensions(&png_bytes)?;
        let processed = process_image(png_bytes, AttachmentKind::ImagePng)?;
        return Ok(Some((processed, width, height)));
    }

    // Fall back to arboard
    let mut clipboard = arboard::Clipboard::new().map_err(|e| {
        tracing::warn!(target: "jfc::attachments", error = %e, "clipboard access failed");
        format!("Clipboard: {e}")
    })?;
    let img = match clipboard.get_image() {
        Ok(img) => img,
        Err(arboard::Error::ContentNotAvailable) => {
            tracing::debug!(target: "jfc::attachments", "no image in clipboard");
            return Ok(None);
        }
        Err(e) => {
            tracing::warn!(target: "jfc::attachments", error = %e, "clipboard get_image failed");
            return Err(format!("Clipboard: {e}"));
        }
    };

    let (width, height) = (img.width as u32, img.height as u32);
    let rgba: Vec<u8> = img.bytes.into_owned();

    // Encode to PNG first so we can run it through process_image
    let png_bytes = encode_png(&rgba, width, height)?;
    let processed = process_image(png_bytes, AttachmentKind::ImagePng)?;

    tracing::debug!(
        target: "jfc::attachments",
        size = processed.bytes.len(),
        width, height,
        kind = processed.kind.mime_type(),
        "read_clipboard_image success"
    );
    Ok(Some((processed, width, height)))
}

/// Helper: PNG-encode raw RGBA8 pixels.
fn encode_png(rgba: &[u8], width: u32, height: u32) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    use image::ImageEncoder as _;
    let encoder = image::codecs::png::PngEncoder::new(&mut out);
    encoder
        .write_image(rgba, width, height, image::ExtendedColorType::Rgba8)
        .map_err(|e| format!("PNG encode: {e}"))?;
    Ok(out)
}

/// Build the Anthropic Messages-API content block for an attachment.
///
/// Image kinds (PNG/JPEG/GIF/WebP) emit:
/// ```json
/// { "type": "image",
///   "source": { "type": "base64", "media_type": "image/png", "data": "..." } }
/// ```
///
/// PDF kind emits the `document` block shape Anthropic added for
/// multi-page PDFs:
/// ```json
/// { "type": "document",
///   "source": { "type": "base64", "media_type": "application/pdf", "data": "..." } }
/// ```
///
/// The two block types are *not* interchangeable on the wire — the API
/// returns 400 if you send a PDF as `image`. Routing happens here so
/// callers don't have to remember the rule.
pub fn to_anthropic_content_block(att: &Attachment) -> serde_json::Value {
    // v132 Files API integration: large attachments upload via the
    // managed-files endpoint and reference by FileID instead of being
    // inlined as base64. Saves prompt tokens (the FileID reference is
    // ~30 bytes vs. potentially MBs of base64) and lifts the per-
    // request payload ceiling. The threshold + Files-API gate live
    // in `sdk_bridge::should_upload`.
    if crate::sdk_bridge::should_upload(att) {
        // The actual upload is async and spawns at message-build time;
        // here we just emit the tagged block. The build-time flow is:
        //   1. message builder spots needs_upload attachments
        //   2. spawns FileService::upload, awaits FileID
        //   3. swaps the inline block for a `{type:"image",source:{type:"file",file_id}}` shape
        // For the synchronous build path used today we still inline,
        // but tag the block so future-builder can identify what to
        // re-encode. Tracked via task #193 in the session log.
    }
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

/// Async variant: when a Files API client is available, upload large
/// attachments and return a FileID-referenced content block instead of
/// inlining base64. Falls back to `to_anthropic_content_block` on any
/// failure (network, auth, size limit) so the request is never lost.
pub async fn to_anthropic_content_block_async(
    att: &Attachment,
    client: Option<&jfc_anthropic_sdk::Client>,
) -> serde_json::Value {
    let Some(client) = client else {
        return to_anthropic_content_block(att);
    };
    if !crate::sdk_bridge::should_upload(att) {
        return to_anthropic_content_block(att);
    }
    use jfc_anthropic_sdk::files::FileService;
    let svc = FileService::new(client.clone());
    let filename = match att.kind {
        AttachmentKind::ImagePng => "attachment.png",
        AttachmentKind::ImageJpeg => "attachment.jpg",
        AttachmentKind::ImageGif => "attachment.gif",
        AttachmentKind::ImageWebp => "attachment.webp",
        AttachmentKind::ApplicationPdf => "attachment.pdf",
    };
    match svc
        .upload(filename, att.kind.mime_type(), att.bytes.clone())
        .await
    {
        Ok(meta) => {
            tracing::info!(
                target: "jfc::attachments::files",
                file_id = %meta.id,
                bytes = att.bytes.len(),
                "uploaded via Files API"
            );
            let block_type = if att.kind.is_pdf() {
                "document"
            } else {
                "image"
            };
            serde_json::json!({
                "type": block_type,
                "source": {
                    "type": "file",
                    "file_id": meta.id,
                }
            })
        }
        Err(e) => {
            tracing::debug!(
                target: "jfc::attachments::files",
                error = %e,
                "Files API upload failed; falling back to inline base64"
            );
            to_anthropic_content_block(att)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine as _;

    // ---------- detect_kind: positive cases ----------

    #[test]
    fn detect_kind_png_normal() {
        // Full PNG signature plus a stub IHDR length so the buffer is
        // realistically-sized; only the first 4 bytes drive detection.
        let bytes = [
            0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D,
        ];
        assert_eq!(detect_kind(&bytes), Some(AttachmentKind::ImagePng));
    }

    #[test]
    fn detect_kind_jpeg_normal() {
        // JFIF marker (FFE0) is common, but EXIF (FFE1) and SOI-only
        // streams also appear; all three start with FF D8 FF.
        let jfif = [0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10];
        let exif = [0xFF, 0xD8, 0xFF, 0xE1, 0x00, 0x10];
        assert_eq!(detect_kind(&jfif), Some(AttachmentKind::ImageJpeg));
        assert_eq!(detect_kind(&exif), Some(AttachmentKind::ImageJpeg));
    }

    #[test]
    fn detect_kind_gif_normal() {
        let gif87 = b"GIF87a\x01\x00\x01\x00";
        let gif89 = b"GIF89a\x01\x00\x01\x00";
        assert_eq!(detect_kind(gif87), Some(AttachmentKind::ImageGif));
        assert_eq!(detect_kind(gif89), Some(AttachmentKind::ImageGif));
    }

    #[test]
    fn detect_kind_webp_normal() {
        // RIFF <size:4> WEBP <fourcc:4>
        let mut bytes = Vec::from(b"RIFF" as &[u8]);
        bytes.extend_from_slice(&[0x24, 0x00, 0x00, 0x00]); // dummy size
        bytes.extend_from_slice(b"WEBP");
        bytes.extend_from_slice(b"VP8 "); // codec fourcc
        assert_eq!(detect_kind(&bytes), Some(AttachmentKind::ImageWebp));
    }

    // ---------- detect_kind: negative cases ----------

    #[test]
    fn detect_kind_unknown_returns_none_robust() {
        // Random bytes: not any known signature.
        assert_eq!(detect_kind(&[0x00, 0x01, 0x02, 0x03, 0x04, 0x05]), None);
        // Plain ASCII text.
        assert_eq!(detect_kind(b"hello world, this is not an image"), None);
        // RIFF without WEBP fourcc (e.g. WAV) must not match.
        let mut riff_wav = Vec::from(b"RIFF" as &[u8]);
        riff_wav.extend_from_slice(&[0x24, 0x00, 0x00, 0x00]);
        riff_wav.extend_from_slice(b"WAVE");
        riff_wav.extend_from_slice(b"fmt ");
        assert_eq!(detect_kind(&riff_wav), None);
    }

    #[test]
    fn detect_kind_too_short_returns_none_robust() {
        // Empty buffer.
        assert_eq!(detect_kind(&[]), None);
        // Single-byte PNG-ish prefix.
        assert_eq!(detect_kind(&[0x89]), None);
        // 2 bytes — one short of JPEG's 3-byte signature.
        assert_eq!(detect_kind(&[0xFF, 0xD8]), None);
        // 5 bytes of "GIF" — short of full 6-byte signature.
        assert_eq!(detect_kind(b"GIF89"), None);
        // RIFF with no WEBP fourcc room.
        assert_eq!(detect_kind(b"RIFF\x00\x00\x00\x00WEB"), None);
    }

    // ---------- mime_type ----------

    #[test]
    fn mime_type_matches_kind_normal() {
        assert_eq!(AttachmentKind::ImagePng.mime_type(), "image/png");
        assert_eq!(AttachmentKind::ImageJpeg.mime_type(), "image/jpeg");
        assert_eq!(AttachmentKind::ImageGif.mime_type(), "image/gif");
        assert_eq!(AttachmentKind::ImageWebp.mime_type(), "image/webp");
    }

    // ---------- to_anthropic_content_block ----------

    #[test]
    fn to_anthropic_content_block_shape_normal() {
        let original_bytes = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0xDE, 0xAD];
        let att = Attachment {
            id: 0,
            kind: AttachmentKind::ImagePng,
            bytes: original_bytes.clone(),
        };
        let block = to_anthropic_content_block(&att);

        assert_eq!(block["type"], "image");
        assert_eq!(block["source"]["type"], "base64");
        assert_eq!(block["source"]["media_type"], "image/png");

        // Round-trip the data field: base64-decode and check we recover
        // the exact bytes we supplied.
        let data = block["source"]["data"]
            .as_str()
            .expect("data should be a string");
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(data)
            .expect("round-trip decode");
        assert_eq!(decoded, original_bytes);

        // Sanity: try the same with a JPEG to confirm media_type tracks
        // the kind, not a hard-coded constant.
        let att_jpeg = Attachment {
            id: 0,
            kind: AttachmentKind::ImageJpeg,
            bytes: vec![0xFF, 0xD8, 0xFF, 0xE0],
        };
        let block_jpeg = to_anthropic_content_block(&att_jpeg);
        assert_eq!(block_jpeg["source"]["media_type"], "image/jpeg");
    }

    // Normal: GIF and WebP round-trip too — exercise the remaining match
    // arms in mime_type / to_anthropic_content_block.
    #[test]
    fn to_anthropic_content_block_gif_and_webp_normal() {
        let gif_att = Attachment {
            id: 0,
            kind: AttachmentKind::ImageGif,
            bytes: b"GIF89a-data".to_vec(),
        };
        let gif_block = to_anthropic_content_block(&gif_att);
        assert_eq!(gif_block["source"]["media_type"], "image/gif");

        let webp_att = Attachment {
            id: 0,
            kind: AttachmentKind::ImageWebp,
            bytes: vec![0xAB; 16],
        };
        let webp_block = to_anthropic_content_block(&webp_att);
        assert_eq!(webp_block["source"]["media_type"], "image/webp");
    }

    // Robust: an empty Attachment still produces a well-shaped content block
    // (empty base64 "" is valid).
    #[test]
    fn to_anthropic_content_block_empty_bytes_robust() {
        let att = Attachment {
            id: 0,
            kind: AttachmentKind::ImagePng,
            bytes: Vec::new(),
        };
        let block = to_anthropic_content_block(&att);
        assert_eq!(block["source"]["data"], "");
        assert_eq!(block["type"], "image");
    }

    // Normal: AttachmentKind round-trips via mime_type for every variant
    // exhaustively.
    #[test]
    fn mime_type_exhaustive_variants_normal() {
        for (kind, expected) in [
            (AttachmentKind::ImagePng, "image/png"),
            (AttachmentKind::ImageJpeg, "image/jpeg"),
            (AttachmentKind::ImageGif, "image/gif"),
            (AttachmentKind::ImageWebp, "image/webp"),
        ] {
            assert_eq!(kind.mime_type(), expected);
        }
    }

    // Robust: detect_kind on a buffer that *contains* a JPEG marker not at
    // the start must NOT match (we sniff the head, not the body).
    #[test]
    fn detect_kind_marker_only_at_head_robust() {
        let mut hidden = vec![0x00, 0x00, 0x00];
        hidden.extend_from_slice(&[0xFF, 0xD8, 0xFF]); // JPEG marker, but offset
        assert_eq!(detect_kind(&hidden), None);
    }

    // Normal: PDFs start with `%PDF-` plus a version. Detect any of
    // the common version markers.
    #[test]
    fn detect_kind_pdf_normal() {
        for header in [
            b"%PDF-1.4\nfake content".as_slice(),
            b"%PDF-1.7\n%\xE2\xE3\xCF\xD3\n",
            b"%PDF-2.0\n",
        ] {
            assert_eq!(detect_kind(header), Some(AttachmentKind::ApplicationPdf));
        }
    }

    // Robust: 4-byte `%PDF` prefix (no version dash) is short of the
    // 5-byte signature, so detect_kind must say None.
    #[test]
    fn detect_kind_pdf_too_short_robust() {
        assert_eq!(detect_kind(b"%PDF"), None);
        assert_eq!(detect_kind(b"%PD"), None);
        assert_eq!(detect_kind(b""), None);
    }

    // Normal: PDF mime + content-block shape. PDFs MUST emit
    // `type: "document"` not `type: "image"` — verifying this prevents
    // the 400 "wrong content block" error from the Anthropic API.
    #[test]
    fn pdf_to_content_block_uses_document_type_normal() {
        let pdf = Attachment {
            id: 0,
            kind: AttachmentKind::ApplicationPdf,
            bytes: b"%PDF-1.7\nfake".to_vec(),
        };
        let block = to_anthropic_content_block(&pdf);
        assert_eq!(block["type"], "document");
        assert_eq!(block["source"]["type"], "base64");
        assert_eq!(block["source"]["media_type"], "application/pdf");
        // Round-trip the data field.
        let data = block["source"]["data"].as_str().unwrap();
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(data)
            .unwrap();
        assert_eq!(decoded, pdf.bytes);
    }

    // Normal: read_pdf_file accepts a real PDF starting with `%PDF-`.
    #[test]
    fn read_pdf_file_accepts_valid_pdf_normal() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("test.pdf");
        std::fs::write(&path, b"%PDF-1.4\nstub body").unwrap();
        let att = read_pdf_file(&path).expect("should accept");
        assert_eq!(att.kind, AttachmentKind::ApplicationPdf);
        assert!(att.bytes.starts_with(b"%PDF-"));
    }

    // Robust: a `.pdf` file whose contents *aren't* a real PDF must
    // be rejected — we don't want to send arbitrary garbage as
    // `application/pdf` and watch the API 400.
    #[test]
    fn read_pdf_file_rejects_non_pdf_robust() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("fake.pdf");
        std::fs::write(&path, b"not a pdf at all").unwrap();
        let res = read_pdf_file(&path);
        assert!(res.is_err(), "must reject non-PDF content");
    }

    // Robust: the size cap fires for files over the 32 MiB limit.
    // Build a 33 MiB file to drive past the boundary; the cap exists
    // because Anthropic's API rejects larger PDFs and a synchronous
    // round-trip 413 is worse UX than a clear local error.
    #[test]
    fn read_pdf_file_rejects_oversized_robust() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("huge.pdf");
        // 33 MiB of `%PDF-` followed by zeros — passes magic-byte
        // sniffing but trips the size cap.
        let mut buf = Vec::with_capacity(33 * 1024 * 1024);
        buf.extend_from_slice(b"%PDF-1.7\n");
        buf.resize(33 * 1024 * 1024, 0u8);
        std::fs::write(&path, &buf).unwrap();
        let res = read_pdf_file(&path);
        assert!(res.is_err(), "must reject >32 MiB PDFs");
        assert!(
            res.as_ref().unwrap_err().contains("too large"),
            "error message should mention size: {res:?}"
        );
    }

    // Normal: is_pdf() drives the routing decision. Pin the truth
    // table so a future enum variant can't accidentally claim PDF.
    #[test]
    fn is_pdf_classification_normal() {
        assert!(AttachmentKind::ApplicationPdf.is_pdf());
        assert!(!AttachmentKind::ImagePng.is_pdf());
        assert!(!AttachmentKind::ImageJpeg.is_pdf());
        assert!(!AttachmentKind::ImageGif.is_pdf());
        assert!(!AttachmentKind::ImageWebp.is_pdf());
    }

    // Normal: encode_png produces a valid PNG that detect_kind
    // recognizes. Smoke test for the encode helper used by the
    // clipboard-resize loop.
    #[test]
    fn encode_png_round_trips_through_detect_kind_normal() {
        // 2x2 solid-color RGBA8.
        let pixels: Vec<u8> = vec![
            255, 0, 0, 255, // red
            0, 255, 0, 255, // green
            0, 0, 255, 255, // blue
            255, 255, 0, 255, // yellow
        ];
        let bytes = encode_png(&pixels, 2, 2).expect("encode succeeds");
        assert_eq!(detect_kind(&bytes), Some(AttachmentKind::ImagePng));
    }

    // Normal: MAX_IMAGE_BYTES is the same conservative cap (3.5 MB)
    // we documented in the read_clipboard_image rationale — pin it so
    // a careless raise doesn't slip past code review.
    #[test]
    fn max_image_bytes_is_conservative_anthropic_cap_normal() {
        assert!(MAX_IMAGE_BYTES <= 3_750_000); // raw bytes for 5MB base64
        assert!(MAX_IMAGE_BYTES >= 1_000_000); // big enough for typical screenshots
    }
}

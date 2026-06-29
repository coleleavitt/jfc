//! Frontend half of the attachments module: clipboard capture (arboard is a
//! frontend dependency). Everything else re-exports from jfc-engine.

pub use jfc_engine::attachments::*;

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

/// Try shell-based clipboard image acquisition (Linux).
/// Claude Code 2.1.140 uses this as the primary path on Linux because
/// arboard's X11/Wayland support is inconsistent across compositors.
fn read_clipboard_image_shell() -> Result<Option<Vec<u8>>, String> {
    // Try xclip first (X11)
    if let Ok(out) = std::process::Command::new("xclip")
        .args(["-selection", "clipboard", "-t", "image/png", "-o"])
        .output()
        && out.status.success()
        && !out.stdout.is_empty()
    {
        return Ok(Some(out.stdout));
    }
    // Try wl-paste (Wayland)
    if let Ok(out) = std::process::Command::new("wl-paste")
        .args(["--type", "image/png"])
        .output()
        && out.status.success()
        && !out.stdout.is_empty()
    {
        return Ok(Some(out.stdout));
    }
    // Try xsel (legacy X11 fallback)
    if let Ok(out) = std::process::Command::new("xsel")
        .args(["--clipboard", "--output"])
        .output()
        && out.status.success()
        && !out.stdout.is_empty()
        && out.stdout.starts_with(&[0x89, b'P', b'N', b'G'])
    {
        return Ok(Some(out.stdout));
    }
    Ok(None)
}

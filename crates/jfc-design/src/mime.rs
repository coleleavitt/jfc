//! Content-type guessing for served files and data-URI inlining.

use std::path::Path;

/// Best-effort MIME type for a path, falling back to `application/octet-stream`.
pub fn guess(path: impl AsRef<Path>) -> String {
    // A few overrides where `mime_guess` is either silent or unhelpful for the
    // browser-preview / inlining use case.
    if let Some(ext) = path
        .as_ref()
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
    {
        match ext.as_str() {
            "jsx" | "mjs" | "cjs" => return "text/javascript".to_owned(),
            "woff2" => return "font/woff2".to_owned(),
            "woff" => return "font/woff".to_owned(),
            "ttf" => return "font/ttf".to_owned(),
            "otf" => return "font/otf".to_owned(),
            _ => {}
        }
    }
    mime_guess::from_path(path)
        .first_raw()
        .map(str::to_owned)
        .unwrap_or_else(|| "application/octet-stream".to_owned())
}

/// True when the type is textual (so it can be embedded UTF-8 in a data URI).
pub fn is_text(mime: &str) -> bool {
    mime.starts_with("text/")
        || mime == "application/javascript"
        || mime == "text/javascript"
        || mime == "image/svg+xml"
        || mime.ends_with("+xml")
        || mime == "application/json"
}

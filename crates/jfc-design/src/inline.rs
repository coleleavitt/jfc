//! Standalone-HTML inliner — the native `super_inline_html`.
//!
//! Given an HTML file, inline every resource referenced directly in HTML
//! attributes and CSS into a single self-contained file that works offline:
//! `<link>` stylesheets, `<script src>`, `img`/`source` `src`/`srcset`,
//! `video`/`audio`/`track` `src`, video `poster`, SVG `<image href>` / `<use href>`,
//! link icons, CSS `url()` and `@import` (recursively), and `url()` inside inline
//! `style` attributes.
//!
//! It does NOT discover resources referenced only as strings in JS/JSX — those must
//! be lifted via `<meta name="ext-resource-dependency">` (see the `save-standalone-html`
//! skill). Remote (`http(s)://`, `//`, `data:`) URLs are left untouched.

use std::path::{Path, PathBuf};

use base64::Engine as _;
use regex::Regex;

use crate::{DesignError, Result, io_err, mime};

/// Outcome of a bundle run.
#[derive(Debug, Clone)]
pub struct BundleReport {
    pub output: PathBuf,
    pub bytes: usize,
    /// Assets that could not be resolved (relative URL → reason).
    pub misses: Vec<String>,
}

impl BundleReport {
    /// A human-readable summary mirroring the Claude Design tool output.
    pub fn summary(&self) -> String {
        let mut s = format!(
            "Bundled to {} ({} KB).",
            self.output.display(),
            self.bytes / 1024
        );
        if self.misses.is_empty() {
            s.push_str(" All assets inlined.");
        } else {
            s.push_str(&format!(
                "\n{} asset(s) could not be bundled:",
                self.misses.len()
            ));
            for m in &self.misses {
                s.push_str(&format!("\n  - {m}"));
            }
        }
        s
    }
}

/// Inline `input_path` and write the result to `output_path`.
///
/// When `require_thumbnail` is true, a `<template id="__bundler_thumbnail">` must be
/// present (matching Claude Design's safeguard) or the call errors.
pub fn bundle(
    input_path: impl AsRef<Path>,
    output_path: impl AsRef<Path>,
    require_thumbnail: bool,
) -> Result<BundleReport> {
    let input = input_path.as_ref();
    let output = output_path.as_ref();
    let html = std::fs::read_to_string(input).map_err(|e| io_err(input, e))?;

    if require_thumbnail && !html.contains("__bundler_thumbnail") {
        return Err(DesignError::Bundle(
            "missing <template id=\"__bundler_thumbnail\"> — add a splash thumbnail \
             (see the save-standalone-html skill) or pass --allow-no-thumbnail"
                .to_owned(),
        ));
    }

    let base = input.parent().unwrap_or_else(|| Path::new("."));
    let mut ctx = Inliner::new(base);
    let out_html = ctx.process_html(&html);

    if let Some(parent) = output.parent() {
        std::fs::create_dir_all(parent).map_err(|e| io_err(parent, e))?;
    }
    std::fs::write(output, &out_html).map_err(|e| io_err(output, e))?;

    Ok(BundleReport {
        output: output.to_path_buf(),
        bytes: out_html.len(),
        misses: ctx.misses,
    })
}

struct Inliner<'a> {
    base: &'a Path,
    misses: Vec<String>,
    re_link: Regex,
    re_script: Regex,
    re_attr: Regex,
    re_style_block: Regex,
    re_style_attr: Regex,
    re_css_url: Regex,
    re_css_import: Regex,
}

impl<'a> Inliner<'a> {
    fn new(base: &'a Path) -> Self {
        Self {
            base,
            misses: Vec::new(),
            // <link ... rel="stylesheet" ... href="...">  (rel/href order-independent)
            re_link: Regex::new(r#"(?is)<link\b[^>]*?>"#).unwrap(),
            // <script ... src="..."></script>
            re_script: Regex::new(
                r#"(?is)<script\b([^>]*?)\bsrc\s*=\s*["']([^"']+)["']([^>]*)>\s*</script>"#,
            )
            .unwrap(),
            // src/href/poster on media-ish tags
            re_attr: Regex::new(r#"(?is)\b(src|href|poster)\s*=\s*["']([^"']+)["']"#).unwrap(),
            // <style> ... </style>
            re_style_block: Regex::new(r#"(?is)<style\b[^>]*>(.*?)</style>"#).unwrap(),
            // style="...url(...)..."
            re_style_attr: Regex::new(r#"(?is)\bstyle\s*=\s*"([^"]*url\([^"]*)""#).unwrap(),
            re_css_url: Regex::new(r#"(?i)url\(\s*['"]?([^'")]+)['"]?\s*\)"#).unwrap(),
            re_css_import: Regex::new(r#"(?i)@import\s+(?:url\()?\s*['"]([^'"]+)['"]\s*\)?\s*;"#)
                .unwrap(),
        }
    }

    fn process_html(&mut self, html: &str) -> String {
        // 1. Inline <link rel="stylesheet"> and link icons.
        let s = self
            .re_link
            .clone()
            .replace_all(html, |caps: &regex::Captures<'_>| {
                self.inline_link_tag(&caps[0])
            });
        // 2. Inline <script src>.
        let s = self
            .re_script
            .clone()
            .replace_all(&s, |caps: &regex::Captures<'_>| {
                let url = &caps[2];
                match self.read_asset(url) {
                    Some((bytes, _)) => {
                        let code = String::from_utf8_lossy(&bytes);
                        format!("<script{}{}>\n{}\n</script>", &caps[1], &caps[3], code)
                    }
                    None => caps[0].to_owned(),
                }
            });
        // 3. Inline url() inside <style> blocks (also resolves @import).
        let s = self
            .re_style_block
            .clone()
            .replace_all(&s, |caps: &regex::Captures<'_>| {
                let inner = self.inline_css(&caps[1], self.base.to_path_buf());
                format!("<style>{inner}</style>")
            });
        // 4. Inline url() inside inline style="" attributes.
        let s = self
            .re_style_attr
            .clone()
            .replace_all(&s, |caps: &regex::Captures<'_>| {
                let inner = self.inline_css_urls(&caps[1], self.base);
                format!("style=\"{inner}\"")
            });
        // 5. Inline remaining media attributes (img/source/video/audio/track/use/image).
        let s = self
            .re_attr
            .clone()
            .replace_all(&s, |caps: &regex::Captures<'_>| {
                let attr = &caps[1];
                let url = &caps[2];
                if self.is_remote(url) {
                    return caps[0].to_owned();
                }
                match self.to_data_uri(url) {
                    Some(d) => format!("{attr}=\"{d}\""),
                    None => caps[0].to_owned(),
                }
            });
        s.into_owned()
    }

    fn inline_link_tag(&mut self, tag: &str) -> String {
        let lower = tag.to_ascii_lowercase();
        let href = self
            .re_attr
            .captures_iter(tag)
            .find(|c| &c[1] == "href")
            .map(|c| c[2].to_owned());
        let Some(href) = href else {
            return tag.to_owned();
        };
        if self.is_remote(&href) {
            return tag.to_owned();
        }
        if lower.contains("stylesheet") {
            match self.read_asset(&href) {
                Some((bytes, _)) => {
                    let css = String::from_utf8_lossy(&bytes).into_owned();
                    let css_base = self
                        .asset_path(&href)
                        .and_then(|p| p.parent().map(Path::to_path_buf))
                        .unwrap_or_else(|| self.base.to_path_buf());
                    let inlined = self.inline_css(&css, css_base);
                    format!("<style>{inlined}</style>")
                }
                None => tag.to_owned(),
            }
        } else if lower.contains("icon") || lower.contains("apple-touch") {
            match self.to_data_uri(&href) {
                Some(d) => self
                    .re_attr
                    .replace(tag, |c: &regex::Captures<'_>| {
                        if &c[1] == "href" {
                            format!("href=\"{d}\"")
                        } else {
                            c[0].to_owned()
                        }
                    })
                    .into_owned(),
                None => tag.to_owned(),
            }
        } else {
            tag.to_owned()
        }
    }

    /// Inline `@import`s (recursively) and `url()`s within a CSS string.
    fn inline_css(&mut self, css: &str, css_base: PathBuf) -> String {
        // Resolve @import first (inline the imported file's CSS in place).
        let imported = self
            .re_css_import
            .clone()
            .replace_all(css, |caps: &regex::Captures<'_>| {
                let url = &caps[1];
                if self.is_remote(url) {
                    return caps[0].to_owned();
                }
                match read_rel(&css_base, url) {
                    Ok(bytes) => {
                        let sub = String::from_utf8_lossy(&bytes).into_owned();
                        let sub_base = css_base
                            .join(url)
                            .parent()
                            .map(Path::to_path_buf)
                            .unwrap_or_else(|| css_base.clone());
                        self.inline_css(&sub, sub_base)
                    }
                    Err(_) => {
                        self.misses.push(format!("asset not found: {url}"));
                        caps[0].to_owned()
                    }
                }
            });
        self.inline_css_urls(&imported, &css_base)
    }

    fn inline_css_urls(&mut self, css: &str, css_base: &Path) -> String {
        self.re_css_url
            .clone()
            .replace_all(css, |caps: &regex::Captures<'_>| {
                let url = caps[1].trim();
                if self.is_remote(url) || url.starts_with('#') {
                    return caps[0].to_owned();
                }
                match read_rel(css_base, url) {
                    Ok(bytes) => format!("url({})", data_uri(url, &bytes)),
                    Err(_) => {
                        self.misses.push(format!("asset not found: {url}"));
                        caps[0].to_owned()
                    }
                }
            })
            .into_owned()
    }

    fn is_remote(&self, url: &str) -> bool {
        let u = url.trim();
        u.starts_with("http://")
            || u.starts_with("https://")
            || u.starts_with("//")
            || u.starts_with("data:")
            || u.starts_with("mailto:")
            || u.starts_with('#')
            || u.is_empty()
    }

    /// Resolve a URL relative to the document base into an on-disk path.
    fn asset_path(&self, url: &str) -> Option<PathBuf> {
        let clean = strip_query_hash(url);
        let decoded = percent_decode(&clean);
        let p = self.base.join(&decoded);
        Some(p)
    }

    fn read_asset(&mut self, url: &str) -> Option<(Vec<u8>, PathBuf)> {
        let p = self.asset_path(url)?;
        match std::fs::read(&p) {
            Ok(b) => Some((b, p)),
            Err(_) => {
                self.misses.push(format!("asset not found: {url}"));
                None
            }
        }
    }

    fn to_data_uri(&mut self, url: &str) -> Option<String> {
        let (bytes, _) = self.read_asset(url)?;
        Some(data_uri(url, &bytes))
    }
}

fn read_rel(base: &Path, url: &str) -> std::io::Result<Vec<u8>> {
    let clean = strip_query_hash(url);
    let decoded = percent_decode(&clean);
    std::fs::read(base.join(decoded))
}

fn data_uri(url: &str, bytes: &[u8]) -> String {
    let m = mime::guess(strip_query_hash(url));
    if mime::is_text(&m) {
        // Text data URIs are smaller and human-debuggable; URL-encode the few unsafe chars.
        let encoded = bytes
            .iter()
            .map(|&b| match b {
                b'#' => "%23".to_owned(),
                b'%' => "%25".to_owned(),
                b'"' => "%22".to_owned(),
                _ => (b as char).to_string(),
            })
            .collect::<String>();
        format!("data:{m};charset=utf-8,{encoded}")
    } else {
        let b64 = base64::engine::general_purpose::STANDARD.encode(bytes);
        format!("data:{m};base64,{b64}")
    }
}

fn strip_query_hash(url: &str) -> String {
    let u = url.split('#').next().unwrap_or(url);
    u.split('?').next().unwrap_or(u).to_owned()
}

/// Minimal percent-decoding for `%XX` sequences (handles the common `%20` etc.).
fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).ok();
            if let Some(h) = hex
                && let Ok(v) = u8::from_str_radix(h, 16)
            {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp() -> PathBuf {
        let n = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let p = std::env::temp_dir().join(format!("jfc_inline_test_{n}"));
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    #[test]
    fn inlines_css_script_and_image_normal() {
        let dir = tmp();
        std::fs::write(dir.join("app.css"), b"body{background:url('bg.png')}").unwrap();
        std::fs::write(dir.join("bg.png"), [0x89, 0x50, 0x4e, 0x47]).unwrap();
        std::fs::write(dir.join("app.js"), b"console.log(1)").unwrap();
        std::fs::write(dir.join("hero.svg"), b"<svg></svg>").unwrap();
        let html = r#"<!DOCTYPE html><html><head>
            <template id="__bundler_thumbnail"></template>
            <link rel="stylesheet" href="app.css">
        </head><body>
            <img src="hero.svg">
            <script src="app.js"></script>
        </body></html>"#;
        std::fs::write(dir.join("index.html"), html).unwrap();

        let report = bundle(dir.join("index.html"), dir.join("out.html"), true).unwrap();
        let out = std::fs::read_to_string(dir.join("out.html")).unwrap();
        assert!(out.contains("<style>"), "css inlined as style: {out}");
        assert!(
            out.contains("data:image/png;base64,"),
            "bg.png inlined: {out}"
        );
        assert!(out.contains("console.log(1)"), "js inlined: {out}");
        assert!(out.contains("data:image/svg+xml"), "svg inlined: {out}");
        assert!(!out.contains("src=\"app.js\""), "no leftover script src");
        assert!(report.misses.is_empty(), "no misses: {:?}", report.misses);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn reports_missing_assets_robust() {
        let dir = tmp();
        let html = r#"<template id="__bundler_thumbnail"></template><img src="nope.png">"#;
        std::fs::write(dir.join("i.html"), html).unwrap();
        let report = bundle(dir.join("i.html"), dir.join("o.html"), true).unwrap();
        assert!(
            report.misses.iter().any(|m| m.contains("nope.png")),
            "{:?}",
            report.misses
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn requires_thumbnail_robust() {
        let dir = tmp();
        std::fs::write(dir.join("i.html"), b"<html></html>").unwrap();
        let err = bundle(dir.join("i.html"), dir.join("o.html"), true);
        assert!(matches!(err, Err(DesignError::Bundle(_))));
        // ...but succeeds when not required.
        assert!(bundle(dir.join("i.html"), dir.join("o.html"), false).is_ok());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn leaves_remote_urls_untouched_normal() {
        let dir = tmp();
        let html = r#"<template id="__bundler_thumbnail"></template>
            <img src="https://example.com/x.png">
            <script src="https://cdn.example.com/lib.js"></script>"#;
        std::fs::write(dir.join("i.html"), html).unwrap();
        bundle(dir.join("i.html"), dir.join("o.html"), true).unwrap();
        let out = std::fs::read_to_string(dir.join("o.html")).unwrap();
        assert!(out.contains("https://example.com/x.png"));
        assert!(out.contains("https://cdn.example.com/lib.js"));
        std::fs::remove_dir_all(&dir).ok();
    }
}

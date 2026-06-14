//! Task shortcuts — reusable saved prompt templates.
//!
//! Mirrors Perplexity's `/rest/tasks/shortcuts` surface from the 2026-06-11
//! mindemon dump (shortcuts with a slug, `@mention` placeholders, and
//! copy/paste share tokens; "Automations and recurring templates"). A shortcut
//! is a named, reusable prompt template: a body with `{placeholder}` slots that
//! are filled at expansion time, addressable by a stable slug, and shareable via
//! an opaque copy token.
//!
//! Deterministic + serde-roundtrippable so it persists alongside the daemon
//! state and is fully unit-testable.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// A saved prompt template.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Shortcut {
    /// Stable, URL-safe identifier (also the user-facing handle).
    pub slug: String,
    /// Human title.
    pub title: String,
    /// Template body. `{name}` placeholders are filled by [`Shortcut::expand`].
    pub template: String,
}

impl Shortcut {
    pub fn new(title: impl Into<String>, template: impl Into<String>) -> Self {
        let title = title.into();
        Self {
            slug: slugify(&title),
            title,
            template: template.into(),
        }
    }

    pub fn with_slug(mut self, slug: impl Into<String>) -> Self {
        self.slug = slugify(&slug.into());
        self
    }

    /// The distinct `{placeholder}` names referenced in the template, in first
    /// appearance order.
    pub fn placeholders(&self) -> Vec<String> {
        let mut seen = Vec::new();
        let bytes = self.template.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            if bytes[i] == b'{' {
                if let Some(end_rel) = self.template[i + 1..].find('}') {
                    let name = &self.template[i + 1..i + 1 + end_rel];
                    if !name.is_empty()
                        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
                        && !seen.iter().any(|s: &String| s == name)
                    {
                        seen.push(name.to_owned());
                    }
                    i += end_rel + 2;
                    continue;
                }
            }
            i += 1;
        }
        seen
    }

    /// Expand the template, substituting `{name}` with values from `args`.
    /// Unknown placeholders are left intact (so partial fills are visible).
    pub fn expand(&self, args: &HashMap<String, String>) -> String {
        let mut out = String::with_capacity(self.template.len());
        let bytes = self.template.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            if bytes[i] == b'{' {
                if let Some(end_rel) = self.template[i + 1..].find('}') {
                    let name = &self.template[i + 1..i + 1 + end_rel];
                    if let Some(val) = args.get(name) {
                        out.push_str(val);
                        i += end_rel + 2;
                        continue;
                    }
                }
            }
            out.push(self.template.as_bytes()[i] as char);
            i += 1;
        }
        out
    }
}

/// Errors from the shortcut store.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShortcutError {
    DuplicateSlug(String),
    UnknownSlug(String),
    UnknownToken(String),
}

impl std::fmt::Display for ShortcutError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::DuplicateSlug(s) => write!(f, "shortcut slug already exists: {s}"),
            Self::UnknownSlug(s) => write!(f, "unknown shortcut slug: {s}"),
            Self::UnknownToken(t) => write!(f, "unknown copy token: {t}"),
        }
    }
}

impl std::error::Error for ShortcutError {}

/// A store of shortcuts keyed by slug, plus the copy-token share table.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ShortcutStore {
    shortcuts: Vec<Shortcut>,
    /// Opaque copy tokens → slug, for the copy/paste share flow.
    #[serde(default)]
    tokens: HashMap<String, String>,
}

impl ShortcutStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a shortcut. Errors if the slug is already taken.
    pub fn create(&mut self, shortcut: Shortcut) -> Result<(), ShortcutError> {
        if self.shortcuts.iter().any(|s| s.slug == shortcut.slug) {
            return Err(ShortcutError::DuplicateSlug(shortcut.slug));
        }
        self.shortcuts.push(shortcut);
        Ok(())
    }

    pub fn get(&self, slug: &str) -> Option<&Shortcut> {
        self.shortcuts.iter().find(|s| s.slug == slug)
    }

    pub fn list(&self) -> &[Shortcut] {
        &self.shortcuts
    }

    pub fn delete(&mut self, slug: &str) -> Option<Shortcut> {
        let pos = self.shortcuts.iter().position(|s| s.slug == slug)?;
        // Drop any copy tokens pointing at this slug.
        self.tokens.retain(|_, v| v != slug);
        Some(self.shortcuts.remove(pos))
    }

    /// Expand a shortcut by slug with the given args.
    pub fn expand(
        &self,
        slug: &str,
        args: &HashMap<String, String>,
    ) -> Result<String, ShortcutError> {
        self.get(slug)
            .map(|s| s.expand(args))
            .ok_or_else(|| ShortcutError::UnknownSlug(slug.to_owned()))
    }

    /// Mint an opaque copy token for a shortcut (the "copy" half of share).
    pub fn copy(&mut self, slug: &str) -> Result<String, ShortcutError> {
        if self.get(slug).is_none() {
            return Err(ShortcutError::UnknownSlug(slug.to_owned()));
        }
        let token = mint_token();
        self.tokens.insert(token.clone(), slug.to_owned());
        Ok(token)
    }

    /// Resolve a copy token into a clone of the referenced shortcut (the
    /// "paste" half). The clone keeps the same slug; callers that paste into a
    /// store that already has that slug should re-slug first.
    pub fn paste(&self, token: &str) -> Result<Shortcut, ShortcutError> {
        let slug = self
            .tokens
            .get(token)
            .ok_or_else(|| ShortcutError::UnknownToken(token.to_owned()))?;
        self.get(slug)
            .cloned()
            .ok_or_else(|| ShortcutError::UnknownSlug(slug.clone()))
    }

    pub fn len(&self) -> usize {
        self.shortcuts.len()
    }

    pub fn is_empty(&self) -> bool {
        self.shortcuts.is_empty()
    }

    /// Default on-disk location under a config dir.
    pub fn default_path(config_dir: &std::path::Path) -> std::path::PathBuf {
        config_dir.join("shortcuts.json")
    }

    /// Load from `path`; a missing file yields an empty store.
    pub fn load(path: &std::path::Path) -> std::io::Result<Self> {
        match std::fs::read_to_string(path) {
            Ok(s) => serde_json::from_str(&s)
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::new()),
            Err(e) => Err(e),
        }
    }

    /// Persist to `path` (pretty JSON), creating parent dirs.
    pub fn save(&self, path: &std::path::Path) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_string_pretty(self)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        std::fs::write(path, json)
    }
}

/// Mint an opaque, hard-to-guess copy token without pulling in a UUID crate:
/// a monotonic counter mixed with the high-resolution clock, hex-encoded.
fn mint_token() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    format!("sc-{nanos:016x}-{n:08x}")
}

/// Lowercase, hyphenated, URL-safe slug.
fn slugify(s: &str) -> String {
    let mut slug = String::new();
    let mut prev_dash = false;
    for ch in s.chars() {
        if ch.is_ascii_alphanumeric() {
            slug.push(ch.to_ascii_lowercase());
            prev_dash = false;
        } else if !prev_dash {
            slug.push('-');
            prev_dash = true;
        }
    }
    let slug = slug.trim_matches('-').to_owned();
    if slug.is_empty() {
        "shortcut".to_owned()
    } else {
        slug
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    // ── Template ─────────────────────────────────────────────────────────────

    #[test]
    fn placeholders_extracted_in_order_normal() {
        let s = Shortcut::new("Review", "Review {file} for {concern} then {concern} again");
        assert_eq!(s.placeholders(), vec!["file", "concern"]);
    }

    #[test]
    fn expand_substitutes_known_leaves_unknown_normal() {
        let s = Shortcut::new("Greet", "Hi {name}, your {thing} is ready");
        let out = s.expand(&args(&[("name", "Cole")]));
        assert_eq!(out, "Hi Cole, your {thing} is ready");
    }

    #[test]
    fn expand_full_fill_normal() {
        let s = Shortcut::new("PR", "Open a PR titled {title} against {branch}");
        let out = s.expand(&args(&[("title", "Fix bug"), ("branch", "main")]));
        assert_eq!(out, "Open a PR titled Fix bug against main");
    }

    #[test]
    fn slug_derived_from_title_normal() {
        assert_eq!(
            Shortcut::new("My Cool Template!", "x").slug,
            "my-cool-template"
        );
        assert_eq!(Shortcut::new("   ", "x").slug, "shortcut");
    }

    // ── Store CRUD ───────────────────────────────────────────────────────────

    #[test]
    fn create_get_delete_normal() {
        let mut store = ShortcutStore::new();
        store.create(Shortcut::new("Alpha", "a {x}")).unwrap();
        assert_eq!(store.len(), 1);
        assert!(store.get("alpha").is_some());
        assert_eq!(store.expand("alpha", &args(&[("x", "1")])).unwrap(), "a 1");
        assert!(store.delete("alpha").is_some());
        assert!(store.is_empty());
    }

    #[test]
    fn create_duplicate_slug_is_error_robust() {
        let mut store = ShortcutStore::new();
        store.create(Shortcut::new("Dup", "x")).unwrap();
        let err = store.create(Shortcut::new("Dup", "y")).unwrap_err();
        assert!(matches!(err, ShortcutError::DuplicateSlug(_)));
    }

    #[test]
    fn expand_unknown_slug_is_error_robust() {
        let store = ShortcutStore::new();
        assert!(matches!(
            store.expand("nope", &HashMap::new()),
            Err(ShortcutError::UnknownSlug(_))
        ));
    }

    // ── Copy / paste share flow ──────────────────────────────────────────────

    #[test]
    fn copy_then_paste_roundtrips_normal() {
        let mut store = ShortcutStore::new();
        store.create(Shortcut::new("Shared", "do {thing}")).unwrap();
        let token = store.copy("shared").unwrap();
        let pasted = store.paste(&token).unwrap();
        assert_eq!(pasted.title, "Shared");
        assert_eq!(pasted.template, "do {thing}");
    }

    #[test]
    fn paste_unknown_token_is_error_robust() {
        let store = ShortcutStore::new();
        assert!(matches!(
            store.paste("bogus"),
            Err(ShortcutError::UnknownToken(_))
        ));
    }

    #[test]
    fn delete_drops_copy_tokens_robust() {
        let mut store = ShortcutStore::new();
        store.create(Shortcut::new("Temp", "x")).unwrap();
        let token = store.copy("temp").unwrap();
        store.delete("temp");
        assert!(matches!(
            store.paste(&token),
            Err(ShortcutError::UnknownToken(_))
        ));
    }

    // ── Persistence ──────────────────────────────────────────────────────────

    #[test]
    fn load_missing_then_save_roundtrips_normal() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = ShortcutStore::default_path(dir.path());
        let mut store = ShortcutStore::load(&path).unwrap();
        assert!(store.is_empty());
        store.create(Shortcut::new("Saved", "body {p}")).unwrap();
        store.save(&path).unwrap();
        let back = ShortcutStore::load(&path).unwrap();
        assert_eq!(back.len(), 1);
        assert_eq!(back.get("saved").unwrap().template, "body {p}");
    }
}

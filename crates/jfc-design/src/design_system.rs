//! Design-system indexer — the discovery half of Claude Design's design-system
//! compiler and `check_design_system`.
//!
//! Scans a project the way the compiler does (from file content + sibling
//! relationships, not folder names) and reports what it found plus any issues:
//!
//! - the global-CSS entry (`styles.css` / `index.css` / … — first match wins) and
//!   the `@import` closure reachable from it;
//! - **tokens** — `--*` custom properties declared in that closure;
//! - **fonts** — `@font-face` families in that closure;
//! - **components** — `<Name>.jsx`/`.tsx` (PascalCase) with a sibling `<Name>.d.ts`;
//! - **`@dsCard`** specimen cards and **`@startingPoint`** entries.
//!
//! Writes `_ds_manifest.json`. (Bundling the JSX into `_ds_bundle.js` needs a
//! transpiler and is part of the server phase — see the parity roadmap.)

use std::collections::BTreeSet;
use std::path::Path;

use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::{Result, io_err};

const ENTRY_CANDIDATES: &[&str] = &[
    "styles.css",
    "index.css",
    "globals.css",
    "global.css",
    "main.css",
    "theme.css",
    "tokens.css",
];

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Component {
    pub name: String,
    pub jsx: String,
    pub dts: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Card {
    pub file: String,
    pub group: Option<String>,
    pub name: Option<String>,
    pub viewport: Option<String>,
    pub subtitle: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StartingPoint {
    pub file: String,
    pub kind: String, // "screen" | "component"
    pub section: Option<String>,
    pub subtitle: Option<String>,
    pub viewport: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DsManifest {
    pub namespace: String,
    pub entry_css: Option<String>,
    pub tokens: Vec<String>,
    pub fonts: Vec<String>,
    pub components: Vec<Component>,
    pub cards: Vec<Card>,
    pub starting_points: Vec<StartingPoint>,
    pub issues: Vec<String>,
}

impl DsManifest {
    /// Human-readable report, mirroring `check_design_system`'s output.
    pub fn report(&self) -> String {
        let mut s = String::new();
        s.push_str(&format!(
            "Design system namespace: window.{}\n",
            self.namespace
        ));
        match &self.entry_css {
            Some(c) => s.push_str(&format!("Global CSS entry: {c}\n")),
            None => s.push_str("Global CSS entry: (none found)\n"),
        }
        s.push_str(&format!(
            "Found: {} component(s), {} token(s), {} font(s), {} card(s), {} starting point(s)\n",
            self.components.len(),
            self.tokens.len(),
            self.fonts.len(),
            self.cards.len(),
            self.starting_points.len()
        ));
        if self.issues.is_empty() {
            s.push_str("No issues — usable by consuming projects.");
        } else {
            s.push_str(&format!("{} issue(s):", self.issues.len()));
            for i in &self.issues {
                s.push_str(&format!("\n  - {i}"));
            }
        }
        s
    }
}

/// Index the design-system project rooted at `root`.
pub fn index(root: impl AsRef<Path>) -> DsManifest {
    let root = root.as_ref();
    let namespace = namespace_for(root);
    let mut issues = Vec::new();

    // 1. Entry CSS + @import closure.
    let entry_css = ENTRY_CANDIDATES
        .iter()
        .find(|c| root.join(c).is_file())
        .map(|c| (*c).to_owned());
    if entry_css.is_none() {
        issues.push(format!(
            "no global-CSS entry at project root (expected one of: {})",
            ENTRY_CANDIDATES.join(", ")
        ));
    }
    let css_closure = entry_css
        .as_ref()
        .map(|c| gather_css(root, c))
        .unwrap_or_default();
    let tokens = extract_tokens(&css_closure);
    let fonts = extract_fonts(&css_closure);

    // 2. Components, cards, starting points.
    let mut components = Vec::new();
    let mut cards = Vec::new();
    let mut starting_points = Vec::new();
    let re_card = Regex::new(r"(?is)<!--\s*@dsCard\b(.*?)-->").unwrap();
    let re_sp = Regex::new(r"(?is)<!--\s*@startingPoint\b(.*?)-->").unwrap();
    let re_sp_dts = Regex::new(r"(?is)@startingPoint\b([^\n*]*)").unwrap();

    for entry in walkdir::WalkDir::new(root)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        if !entry.file_type().is_file() {
            continue;
        }
        let path = entry.path();
        let Ok(rel) = path.strip_prefix(root) else {
            continue;
        };
        let rel = rel.to_string_lossy().replace('\\', "/");
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");

        // Component: PascalCase .jsx/.tsx with sibling .d.ts.
        if (ext == "jsx" || ext == "tsx") && is_pascal(stem) {
            let dts_path = path.with_file_name(format!("{stem}.d.ts"));
            let dts = dts_path.is_file().then(|| {
                dts_path
                    .strip_prefix(root)
                    .map(|p| p.to_string_lossy().replace('\\', "/"))
                    .unwrap_or_default()
            });
            if dts.is_none() {
                issues.push(format!(
                    "component {rel} has no sibling {stem}.d.ts (it will be bundled but gets no props contract)"
                ));
            }
            components.push(Component {
                name: stem.to_owned(),
                jsx: rel.clone(),
                dts,
            });
        }

        // Cards + screen starting points: first line of an .html.
        if ext == "html"
            && let Ok(content) = std::fs::read_to_string(path)
        {
            let first_line = content.lines().next().unwrap_or("");
            if let Some(c) = re_card.captures(first_line) {
                let attrs = parse_attrs(&c[1]);
                if !attrs.iter().any(|(k, _)| k == "group") {
                    issues.push(format!("@dsCard in {rel} is missing group=\"…\""));
                }
                cards.push(Card {
                    file: rel.clone(),
                    group: get(&attrs, "group"),
                    name: get(&attrs, "name"),
                    viewport: get(&attrs, "viewport"),
                    subtitle: get(&attrs, "subtitle"),
                });
            }
            if let Some(c) = re_sp.captures(first_line) {
                let attrs = parse_attrs(&c[1]);
                starting_points.push(StartingPoint {
                    file: rel.clone(),
                    kind: "screen".to_owned(),
                    section: get(&attrs, "section"),
                    subtitle: get(&attrs, "subtitle"),
                    viewport: get(&attrs, "viewport"),
                });
            }
        }

        // Component starting points: @startingPoint in a .d.ts JSDoc.
        if rel.ends_with(".d.ts")
            && let Ok(content) = std::fs::read_to_string(path)
            && let Some(c) = re_sp_dts.captures(&content)
        {
            let attrs = parse_attrs(&c[1]);
            starting_points.push(StartingPoint {
                file: rel.clone(),
                kind: "component".to_owned(),
                section: get(&attrs, "section"),
                subtitle: get(&attrs, "subtitle"),
                viewport: get(&attrs, "viewport"),
            });
        }
    }

    components.sort_by(|a, b| a.name.cmp(&b.name));
    cards.sort_by(|a, b| a.file.cmp(&b.file));
    starting_points.sort_by(|a, b| a.file.cmp(&b.file));

    DsManifest {
        namespace,
        entry_css,
        tokens,
        fonts,
        components,
        cards,
        starting_points,
        issues,
    }
}

/// Index and write `_ds_manifest.json` into the project root.
pub fn index_and_write(root: impl AsRef<Path>) -> Result<DsManifest> {
    let root = root.as_ref();
    let manifest = index(root);
    let out = root.join("_ds_manifest.json");
    let json = serde_json::to_string_pretty(&manifest)?;
    std::fs::write(&out, json).map_err(|e| io_err(&out, e))?;
    Ok(manifest)
}

fn namespace_for(root: &Path) -> String {
    let name = root
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("DesignSystem");
    let pascal: String = name
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|s| !s.is_empty())
        .map(|w| {
            let mut ch = w.chars();
            match ch.next() {
                Some(f) => f.to_ascii_uppercase().to_string() + ch.as_str(),
                None => String::new(),
            }
        })
        .collect();
    if pascal.is_empty() {
        "DesignSystem".to_owned()
    } else {
        pascal
    }
}

/// Read the entry CSS and everything it transitively `@import`s, concatenated.
fn gather_css(root: &Path, entry_rel: &str) -> String {
    let re_import = Regex::new(r#"(?i)@import\s+(?:url\()?\s*['"]([^'"]+)['"]\s*\)?\s*;"#).unwrap();
    let mut seen = BTreeSet::new();
    let mut out = String::new();
    let mut stack = vec![entry_rel.to_owned()];
    while let Some(rel) = stack.pop() {
        if !seen.insert(rel.clone()) {
            continue;
        }
        let path = root.join(&rel);
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        let dir = Path::new(&rel)
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_default();
        for c in re_import.captures_iter(&text) {
            let imp = &c[1];
            if imp.starts_with("http") || imp.starts_with("//") {
                continue;
            }
            let joined = dir.join(imp);
            // normalize ./ and ../ within the project
            let norm = joined.to_string_lossy().replace("\\", "/");
            stack.push(norm);
        }
        out.push_str(&text);
        out.push('\n');
    }
    out
}

fn extract_tokens(css: &str) -> Vec<String> {
    let re = Regex::new(r"(--[A-Za-z0-9_-]+)\s*:").unwrap();
    let mut set = BTreeSet::new();
    for c in re.captures_iter(css) {
        set.insert(c[1].to_owned());
    }
    set.into_iter().collect()
}

fn extract_fonts(css: &str) -> Vec<String> {
    let re_block = Regex::new(r"(?is)@font-face\s*\{(.*?)\}").unwrap();
    let re_family = Regex::new(r#"(?i)font-family\s*:\s*['"]?([^;'"]+)['"]?"#).unwrap();
    let mut set = BTreeSet::new();
    for b in re_block.captures_iter(css) {
        if let Some(f) = re_family.captures(&b[1]) {
            set.insert(f[1].trim().to_owned());
        }
    }
    set.into_iter().collect()
}

fn is_pascal(s: &str) -> bool {
    s.chars()
        .next()
        .map(|c| c.is_ascii_uppercase())
        .unwrap_or(false)
}

/// Parse `key="value"` pairs from a comment-tag body.
fn parse_attrs(body: &str) -> Vec<(String, String)> {
    let re = Regex::new(r#"([a-zA-Z_-]+)\s*=\s*"([^"]*)""#).unwrap();
    re.captures_iter(body)
        .map(|c| (c[1].to_owned(), c[2].to_owned()))
        .collect()
}

fn get(attrs: &[(String, String)], key: &str) -> Option<String> {
    attrs.iter().find(|(k, _)| k == key).map(|(_, v)| v.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn tmp() -> PathBuf {
        let n = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let p = std::env::temp_dir().join(format!("jfc_ds_test_{n}"));
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    #[test]
    fn indexes_tokens_components_cards_normal() {
        let dir = tmp();
        std::fs::write(dir.join("styles.css"), "@import 'tokens/colors.css';").unwrap();
        std::fs::create_dir_all(dir.join("tokens")).unwrap();
        std::fs::write(
            dir.join("tokens/colors.css"),
            ":root{--fg-1:#111;--surface-card:#fff}\n@font-face{font-family:'Brand';src:url(b.woff2)}",
        )
        .unwrap();
        std::fs::create_dir_all(dir.join("components/core")).unwrap();
        std::fs::write(
            dir.join("components/core/Button.jsx"),
            "export function Button(){}",
        )
        .unwrap();
        std::fs::write(
            dir.join("components/core/Button.d.ts"),
            "export interface Props{}",
        )
        .unwrap();
        std::fs::write(
            dir.join("components/core/Orphan.jsx"),
            "export function Orphan(){}",
        )
        .unwrap();
        std::fs::write(
            dir.join("components/core/buttons.card.html"),
            "<!-- @dsCard group=\"Components\" name=\"Buttons\" viewport=\"700x200\" -->\n<div></div>",
        )
        .unwrap();

        let m = index(&dir);
        assert_eq!(m.entry_css.as_deref(), Some("styles.css"));
        assert!(m.tokens.contains(&"--fg-1".to_owned()));
        assert!(m.tokens.contains(&"--surface-card".to_owned()));
        assert!(m.fonts.contains(&"Brand".to_owned()));
        assert!(
            m.components
                .iter()
                .any(|c| c.name == "Button" && c.dts.is_some())
        );
        // Orphan has no .d.ts → an issue, dts None.
        assert!(
            m.components
                .iter()
                .any(|c| c.name == "Orphan" && c.dts.is_none())
        );
        assert!(m.issues.iter().any(|i| i.contains("Orphan")));
        assert!(
            m.cards
                .iter()
                .any(|c| c.group.as_deref() == Some("Components"))
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn flags_missing_entry_css_robust() {
        let dir = tmp();
        let m = index(&dir);
        assert!(m.entry_css.is_none());
        assert!(m.issues.iter().any(|i| i.contains("no global-CSS entry")));
        std::fs::remove_dir_all(&dir).ok();
    }
}

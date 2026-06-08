//! Output verbosity styles — built-ins plus Claude Code-style markdown styles.

use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    sync::{OnceLock, RwLock},
};

/// Built-in verbosity / formatting mode for assistant replies.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum OutputStyle {
    /// Default — terse, focused, no extra scaffolding (current behaviour).
    #[default]
    Default,
    /// Brief — minimal explanation, short replies, code only when essential.
    Brief,
    /// Verbose — full context, full sentences, "what / why / how" structure.
    Verbose,
    /// Explanatory — pair every change with a short rationale.
    Explanatory,
    /// Learning — assume the reader is new to the area; explain jargon.
    Learning,
}

impl OutputStyle {
    pub fn from_str_loose(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "brief" | "concise" => Self::Brief,
            "verbose" => Self::Verbose,
            "explanatory" => Self::Explanatory,
            "learning" => Self::Learning,
            _ => Self::Default,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Default => "default",
            Self::Brief => "brief",
            Self::Verbose => "verbose",
            Self::Explanatory => "explanatory",
            Self::Learning => "learning",
        }
    }

    pub fn all() -> &'static [Self] {
        &[
            Self::Default,
            Self::Brief,
            Self::Verbose,
            Self::Explanatory,
            Self::Learning,
        ]
    }

    pub fn system_prompt_suffix(self) -> Option<&'static str> {
        match self {
            Self::Default => None,
            Self::Brief => Some(
                "\n\nOutput style: BRIEF. Keep responses minimal — \
                 short answers, no preamble, code-only when the user \
                 asks for code. One short sentence is almost always \
                 enough; don't pad with restating the question or \
                 listing what you'll do next.",
            ),
            Self::Verbose => Some(
                "\n\nOutput style: VERBOSE. Provide full context: what \
                 you're doing, why, how it fits the broader codebase, \
                 and what trade-offs you considered. Use complete \
                 sentences; structure with headers when the answer \
                 spans multiple concerns.",
            ),
            Self::Explanatory => Some(
                "\n\nOutput style: EXPLANATORY. Pair every concrete \
                 change with a one-sentence rationale (the \"why\"). \
                 Assume the reader will revisit this transcript later \
                 and benefit from the reasoning, not just the diff.",
            ),
            Self::Learning => Some(
                "\n\nOutput style: LEARNING. Treat the reader as new \
                 to this codebase / language / framework: define \
                 jargon on first use, link concepts to underlying \
                 theory, and prefer short concrete examples over \
                 abstract claims.",
            ),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutputStyleDefinition {
    pub name: String,
    pub source: Option<PathBuf>,
    pub built_in: Option<OutputStyle>,
    pub body: Option<String>,
}

impl OutputStyleDefinition {
    pub fn suffix(&self) -> Option<String> {
        if let Some(body) = &self.body {
            return Some(format!("\n\nOutput style: {}.\n{}", self.name.to_ascii_uppercase(), body.trim()));
        }
        self.built_in
            .and_then(OutputStyle::system_prompt_suffix)
            .map(str::to_owned)
    }

    pub fn summary(&self) -> String {
        if let Some(body) = &self.body {
            body.split('.').next().unwrap_or(body).trim().to_owned()
        } else {
            self.built_in
                .and_then(OutputStyle::system_prompt_suffix)
                .map(|s| s.split('.').next().unwrap_or(s).trim().to_owned())
                .unwrap_or_else(|| "no system-prompt change".to_owned())
        }
    }
}

#[derive(Debug, serde::Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
struct StyleFrontmatter {
    name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ActiveOutputStyle {
    BuiltIn(OutputStyle),
    Custom(String),
}

impl Default for ActiveOutputStyle {
    fn default() -> Self {
        Self::BuiltIn(OutputStyle::Default)
    }
}

impl ActiveOutputStyle {
    pub fn name(&self) -> String {
        match self {
            Self::BuiltIn(style) => style.name().to_owned(),
            Self::Custom(name) => name.clone(),
        }
    }

    pub fn built_in(&self) -> OutputStyle {
        match self {
            Self::BuiltIn(style) => *style,
            Self::Custom(_) => OutputStyle::Default,
        }
    }
}

fn handle() -> &'static RwLock<ActiveOutputStyle> {
    static H: OnceLock<RwLock<ActiveOutputStyle>> = OnceLock::new();
    H.get_or_init(|| RwLock::new(ActiveOutputStyle::default()))
}

pub fn set_active(style: OutputStyle) {
    set_active_named(style.name());
}

pub fn set_active_named(name: &str) {
    if let Ok(mut g) = handle().write() {
        let built = OutputStyle::from_str_loose(name);
        *g = if built == OutputStyle::Default && !name.eq_ignore_ascii_case("default") {
            ActiveOutputStyle::Custom(name.to_owned())
        } else {
            ActiveOutputStyle::BuiltIn(built)
        };
    }
}

pub fn active() -> ActiveOutputStyle {
    handle().read().map(|g| g.clone()).unwrap_or_default()
}

pub fn active_suffix(project_root: &Path) -> Option<String> {
    let active = active();
    find_definition(project_root, &active.name()).and_then(|definition| definition.suffix())
}

pub fn load_definitions(project_root: &Path) -> Vec<OutputStyleDefinition> {
    let mut out = Vec::new();
    for built in OutputStyle::all() {
        out.push(OutputStyleDefinition {
            name: built.name().to_owned(),
            source: None,
            built_in: Some(*built),
            body: None,
        });
    }
    for root in style_roots(project_root) {
        let Ok(entries) = std::fs::read_dir(&root.path) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) != Some("md") {
                continue;
            }
            let Ok(raw) = std::fs::read_to_string(&path) else {
                continue;
            };
            let Some(mut definition) = parse_style_file(&path, &raw) else {
                continue;
            };
            if let Some(namespace) = &root.namespace
                && !definition.name.contains(':')
            {
                definition.name = format!("{namespace}:{}", definition.name);
            }
            out.retain(|existing| existing.name != definition.name);
            out.push(definition);
        }
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

pub fn find_definition(project_root: &Path, name: &str) -> Option<OutputStyleDefinition> {
    load_definitions(project_root)
        .into_iter()
        .find(|definition| definition.name.eq_ignore_ascii_case(name))
}

fn parse_style_file(path: &Path, raw: &str) -> Option<OutputStyleDefinition> {
    let (front, body) = split_frontmatter(raw);
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("unnamed");
    let mut name = stem.to_owned();
    if let Some(yaml) = front
        && let Ok(parsed) = serde_yaml::from_str::<StyleFrontmatter>(yaml)
        && let Some(front_name) = parsed.name.filter(|s| !s.trim().is_empty())
    {
        name = front_name.trim().to_owned();
    }
    let body = body.trim().to_owned();
    if name.trim().is_empty() || body.is_empty() {
        return None;
    }
    Some(OutputStyleDefinition {
        name,
        source: Some(path.to_path_buf()),
        built_in: None,
        body: Some(body),
    })
}

fn split_frontmatter(raw: &str) -> (Option<&str>, &str) {
    let trimmed = raw.strip_prefix("---\n").or_else(|| raw.strip_prefix("---\r\n"));
    let Some(rest) = trimmed else {
        return (None, raw);
    };
    if let Some(idx) = rest.find("\n---\n") {
        return (Some(&rest[..idx]), &rest[idx + "\n---\n".len()..]);
    }
    if let Some(idx) = rest.find("\r\n---\r\n") {
        return (Some(&rest[..idx]), &rest[idx + "\r\n---\r\n".len()..]);
    }
    (None, raw)
}

#[derive(Debug, Clone)]
struct StyleRoot {
    path: PathBuf,
    namespace: Option<String>,
}

fn style_roots(project_root: &Path) -> Vec<StyleRoot> {
    let mut roots = Vec::new();
    let mut seen = HashSet::new();
    let mut push_root = |path: PathBuf, namespace: Option<String>| {
        if seen.insert((path.clone(), namespace.clone())) {
            roots.push(StyleRoot { path, namespace });
        }
    };

    if let Some(home) = dirs::home_dir() {
        push_root(home.join(".claude/output-styles"), None);
        push_plugin_roots_in(&home.join(".claude/plugins"), "output-styles", &mut push_root);
    }
    if let Some(config) = dirs::config_dir() {
        push_plugin_roots_in(&config.join("jfc/plugins"), "output-styles", &mut push_root);
    }
    push_plugin_roots_in(&project_root.join("plugins"), "output-styles", &mut push_root);
    push_plugin_roots_in(
        &project_root.join(".claude/plugins"),
        "output-styles",
        &mut push_root,
    );
    push_root(project_root.join(".claude/output-styles"), None);

    roots
}

fn push_plugin_roots_in(
    plugins_dir: &Path,
    child: &str,
    push_root: &mut impl FnMut(PathBuf, Option<String>),
) {
    let Ok(entries) = std::fs::read_dir(plugins_dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let Some(plugin) = path
            .file_name()
            .and_then(|s| s.to_str())
            .filter(|s| !s.starts_with('.'))
        else {
            continue;
        };
        push_root(path.join(child), Some(plugin.to_owned()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn name_roundtrip_normal() {
        for s in OutputStyle::all() {
            assert_eq!(OutputStyle::from_str_loose(s.name()), *s);
        }
    }

    #[test]
    fn unknown_string_falls_back_to_default_robust() {
        assert_eq!(OutputStyle::from_str_loose(""), OutputStyle::Default);
        assert_eq!(OutputStyle::from_str_loose("XYZ"), OutputStyle::Default);
        assert_eq!(OutputStyle::from_str_loose("not-a-style"), OutputStyle::Default);
    }

    #[test]
    fn custom_output_style_loads_from_project_normal() {
        let tmp = tempfile::TempDir::new().unwrap();
        let dir = tmp.path().join(".claude/output-styles");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("pirate.md"), "Speak like a pirate.").unwrap();

        let definition = find_definition(tmp.path(), "pirate").unwrap();
        assert_eq!(definition.name, "pirate");
        assert!(definition.suffix().unwrap().contains("Speak like a pirate."));
    }

    #[test]
    fn plugin_output_style_is_namespaced_robust() {
        let tmp = tempfile::TempDir::new().unwrap();
        let dir = tmp.path().join("plugins/theme/output-styles");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("brief.md"), "Plugin brief.").unwrap();

        assert!(find_definition(tmp.path(), "theme:brief").is_some());
        let built_in = find_definition(tmp.path(), "brief").unwrap();
        assert_eq!(built_in.built_in, Some(OutputStyle::Brief));
    }
}

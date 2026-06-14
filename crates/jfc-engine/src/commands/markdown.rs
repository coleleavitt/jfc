use std::collections::HashSet;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MarkdownCommand {
    pub name: String,
    pub source: PathBuf,
    pub description: Option<String>,
    pub body: String,
}

#[derive(Debug, serde::Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
struct CommandFrontmatter {
    name: Option<String>,
    description: Option<String>,
}

#[derive(Debug, Clone)]
struct CommandRoot {
    path: PathBuf,
    namespace: Option<String>,
}

pub fn load_markdown_commands(project_root: &Path) -> Vec<MarkdownCommand> {
    let mut out = Vec::new();
    for root in command_roots(project_root) {
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
            let Some(mut command) = parse_markdown_command(&path, &raw) else {
                continue;
            };
            if let Some(namespace) = &root.namespace
                && !command.name.contains(':')
            {
                command.name = format!("{namespace}:{}", command.name);
            }
            out.retain(|existing: &MarkdownCommand| existing.name != command.name);
            out.push(command);
        }
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

pub fn find_markdown_command<'a>(
    commands: &'a [MarkdownCommand],
    name: &str,
) -> Option<&'a MarkdownCommand> {
    commands
        .iter()
        .find(|command| command.name.eq_ignore_ascii_case(name))
}

pub fn render_markdown_command(command: &MarkdownCommand, args: Option<&str>) -> String {
    let mut out = command.body.clone();
    if let Some(args) = args.map(str::trim).filter(|s| !s.is_empty()) {
        if out.ends_with('\n') {
            out.push('\n');
        } else {
            out.push_str("\n\n");
        }
        out.push_str("# Args\n");
        out.push_str(args);
    }
    out
}

fn parse_markdown_command(path: &Path, raw: &str) -> Option<MarkdownCommand> {
    let (front, body) = split_frontmatter(raw);
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("unnamed");
    let mut name = stem.to_owned();
    let mut description = None;
    if let Some(yaml) = front
        && let Ok(parsed) = serde_yaml::from_str::<CommandFrontmatter>(yaml)
    {
        if let Some(front_name) = parsed.name.filter(|s| !s.trim().is_empty()) {
            name = front_name.trim().trim_start_matches('/').to_owned();
        }
        description = parsed.description;
    }
    let body = body.trim().to_owned();
    if name.trim().is_empty() || body.is_empty() {
        return None;
    }
    Some(MarkdownCommand {
        name,
        source: path.to_path_buf(),
        description,
        body,
    })
}

fn split_frontmatter(raw: &str) -> (Option<&str>, &str) {
    let trimmed = raw
        .strip_prefix("---\n")
        .or_else(|| raw.strip_prefix("---\r\n"));
    let Some(rest) = trimmed else {
        return (None, raw);
    };
    if let Some(idx) = rest.find("\n---\n") {
        let yaml = &rest[..idx];
        let body = &rest[idx + "\n---\n".len()..];
        return (Some(yaml), body);
    }
    if let Some(idx) = rest.find("\r\n---\r\n") {
        let yaml = &rest[..idx];
        let body = &rest[idx + "\r\n---\r\n".len()..];
        return (Some(yaml), body);
    }
    (None, raw)
}

fn command_roots(project_root: &Path) -> Vec<CommandRoot> {
    let mut roots = Vec::new();
    let mut seen = HashSet::new();
    let settings = crate::config::claude_settings::load_merged(project_root);
    let mut push_root = |path: PathBuf, namespace: Option<String>| {
        if seen.insert((path.clone(), namespace.clone())) {
            roots.push(CommandRoot { path, namespace });
        }
    };

    if let Some(home) = dirs::home_dir() {
        push_root(home.join(".claude/commands"), None);
        push_plugin_roots_in(
            &home.join(".claude/plugins"),
            "commands",
            &settings,
            &mut push_root,
        );
    }
    if let Some(config) = dirs::config_dir() {
        push_plugin_roots_in(
            &config.join("jfc/plugins"),
            "commands",
            &settings,
            &mut push_root,
        );
    }
    push_plugin_roots_in(
        &project_root.join("plugins"),
        "commands",
        &settings,
        &mut push_root,
    );
    push_plugin_roots_in(
        &project_root.join(".claude/plugins"),
        "commands",
        &settings,
        &mut push_root,
    );
    push_root(project_root.join(".claude/commands"), None);

    roots
}

fn push_plugin_roots_in(
    plugins_dir: &Path,
    child: &str,
    settings: &crate::config::ClaudeCompatibilityConfig,
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
        if !settings.plugin_enabled(plugin) {
            continue;
        }
        push_root(path.join(child), Some(plugin.to_owned()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_project_markdown_command_normal() {
        let tmp = tempfile::TempDir::new().unwrap();
        let dir = tmp.path().join(".claude/commands");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("audit.md"),
            "---\ndescription: audit it\n---\nRun a focused audit.",
        )
        .unwrap();

        let commands = load_markdown_commands(tmp.path());
        let command = find_markdown_command(&commands, "audit").unwrap();
        assert_eq!(command.description.as_deref(), Some("audit it"));
        assert_eq!(command.body, "Run a focused audit.");
    }

    #[test]
    fn plugin_markdown_command_is_namespaced_robust() {
        let tmp = tempfile::TempDir::new().unwrap();
        let dir = tmp.path().join("plugins/sec/commands");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("audit.md"), "audit body").unwrap();

        let commands = load_markdown_commands(tmp.path());
        assert!(find_markdown_command(&commands, "sec:audit").is_some());
        assert!(find_markdown_command(&commands, "audit").is_none());
    }

    #[test]
    fn enabled_plugins_false_disables_plugin_markdown_commands_normal() {
        let tmp = tempfile::TempDir::new().unwrap();
        let plugin_dir = tmp.path().join(".claude/plugins/sec/commands");
        std::fs::create_dir_all(&plugin_dir).unwrap();
        std::fs::write(plugin_dir.join("audit.md"), "audit body").unwrap();
        let settings_dir = tmp.path().join(".claude");
        std::fs::create_dir_all(&settings_dir).unwrap();
        std::fs::write(
            settings_dir.join("settings.local.json"),
            r#"{"enabledPlugins":{"sec":false}}"#,
        )
        .unwrap();

        let commands = load_markdown_commands(tmp.path());
        assert!(find_markdown_command(&commands, "sec:audit").is_none());
    }
}

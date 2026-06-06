use std::path::{Path, PathBuf};

use clap::Subcommand;

#[derive(Subcommand, Debug)]
pub(super) enum PluginSubcommand {
    /// List installed local plugins.
    List,
    /// Install a plugin from a local directory or git URL.
    Install {
        /// Local directory or git URL.
        source: String,
        /// Override the installed plugin directory name.
        #[arg(long)]
        name: Option<String>,
        /// Replace an existing plugin with the same name.
        #[arg(long)]
        force: bool,
    },
    /// Update one git-backed plugin, or all git-backed plugins when omitted.
    Update {
        /// Installed plugin name.
        name: Option<String>,
    },
    /// Remove an installed local plugin.
    Remove {
        /// Installed plugin name.
        name: String,
    },
}

pub(super) async fn run_plugin_subcommand(sub: PluginSubcommand) -> anyhow::Result<()> {
    let managed = crate::config::load_managed_settings();
    match sub {
        PluginSubcommand::List => {
            print!("{}", list_plugins()?);
            Ok(())
        }
        PluginSubcommand::Install {
            source,
            name,
            force,
        } => {
            if managed.as_ref().is_some_and(|m| m.disable_plugin_urls)
                && looks_like_git_url(&source)
            {
                anyhow::bail!("plugin URL installs are disabled by managed settings");
            }
            if managed.as_ref().is_some_and(|m| m.disable_plugin_dirs)
                && !looks_like_git_url(&source)
            {
                anyhow::bail!("local plugin directory installs are disabled by managed settings");
            }
            let path = install_plugin(&source, name.as_deref(), force)?;
            println!("installed plugin at {}", path.display());
            Ok(())
        }
        PluginSubcommand::Update { name } => {
            if managed.as_ref().is_some_and(|m| m.disable_plugin_updates) {
                anyhow::bail!("plugin updates are disabled by managed settings");
            }
            let updated = update_plugins(name.as_deref())?;
            if updated.is_empty() {
                println!("no git-backed plugins to update");
            } else {
                for item in updated {
                    println!("{item}");
                }
            }
            Ok(())
        }
        PluginSubcommand::Remove { name } => {
            let path = remove_plugin(&name)?;
            println!("removed plugin at {}", path.display());
            Ok(())
        }
    }
}

pub(super) fn ensure_plugin_url(url: &str) -> anyhow::Result<PathBuf> {
    let name = plugin_name_from_url(url)?;
    let dest = plugins_root()?.join(&name);
    if dest.exists() {
        return Ok(dest);
    }
    install_git_plugin(url, &name, false)
}

fn list_plugins() -> anyhow::Result<String> {
    let root = plugins_root()?;
    let mut out = String::new();
    out.push_str(&format!("plugins: {}\n", root.display()));
    let Ok(entries) = std::fs::read_dir(&root) else {
        out.push_str("(none)\n");
        return Ok(out);
    };
    let mut rows = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let name = manifest_name(&path).unwrap_or_else(|| {
            path.file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("unknown")
                .to_owned()
        });
        let source = if path.join(".git").is_dir() {
            "git"
        } else {
            "local"
        };
        let workflows = workflow_dir_for_plugin(&path)
            .and_then(|dir| std::fs::read_dir(dir).ok())
            .map(|entries| {
                entries
                    .flatten()
                    .filter(|e| e.path().extension().and_then(|s| s.to_str()) == Some("js"))
                    .count()
            })
            .unwrap_or(0);
        rows.push((name, source.to_owned(), workflows, path));
    }
    rows.sort_by(|a, b| a.0.cmp(&b.0));
    if rows.is_empty() {
        out.push_str("(none)\n");
    } else {
        for (name, source, workflows, path) in rows {
            out.push_str(&format!(
                "- {name} [{source}] workflows={workflows} path={}\n",
                path.display()
            ));
        }
    }
    Ok(out)
}

fn install_plugin(source: &str, name: Option<&str>, force: bool) -> anyhow::Result<PathBuf> {
    if looks_like_git_url(source) {
        let name = match name {
            Some(name) => sanitize_plugin_name(name)?,
            None => plugin_name_from_url(source)?,
        };
        return install_git_plugin(source, &name, force);
    }
    let src = PathBuf::from(source);
    let name = match name {
        Some(name) => sanitize_plugin_name(name)?,
        None => manifest_name(&src)
            .or_else(|| src.file_name().and_then(|s| s.to_str()).map(str::to_owned))
            .ok_or_else(|| anyhow::anyhow!("cannot infer plugin name from {}", src.display()))?,
    };
    install_dir_plugin(&src, &name, force)
}

fn install_dir_plugin(src: &Path, name: &str, force: bool) -> anyhow::Result<PathBuf> {
    if !src.is_dir() {
        anyhow::bail!("plugin source is not a directory: {}", src.display());
    }
    let name = sanitize_plugin_name(name)?;
    let dest = plugins_root()?.join(&name);
    prepare_dest(&dest, force)?;
    copy_dir(src, &dest)?;
    Ok(dest)
}

fn install_git_plugin(url: &str, name: &str, force: bool) -> anyhow::Result<PathBuf> {
    let name = sanitize_plugin_name(name)?;
    let dest = plugins_root()?.join(&name);
    prepare_dest(&dest, force)?;
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let status = std::process::Command::new("git")
        .arg("clone")
        .arg("--depth")
        .arg("1")
        .arg(url)
        .arg(&dest)
        .status()?;
    if !status.success() {
        anyhow::bail!("git clone failed for {url} with status {status}");
    }
    Ok(dest)
}

fn update_plugins(name: Option<&str>) -> anyhow::Result<Vec<String>> {
    let root = plugins_root()?;
    let mut targets = Vec::new();
    if let Some(name) = name {
        targets.push(root.join(sanitize_plugin_name(name)?));
    } else if let Ok(entries) = std::fs::read_dir(&root) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.join(".git").is_dir() {
                targets.push(path);
            }
        }
    }
    let mut out = Vec::new();
    for path in targets {
        if !path.join(".git").is_dir() {
            continue;
        }
        let status = std::process::Command::new("git")
            .arg("-C")
            .arg(&path)
            .arg("pull")
            .arg("--ff-only")
            .status()?;
        let name = path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("plugin");
        if status.success() {
            out.push(format!("updated {name}"));
        } else {
            out.push(format!("failed to update {name}: {status}"));
        }
    }
    Ok(out)
}

fn remove_plugin(name: &str) -> anyhow::Result<PathBuf> {
    let name = sanitize_plugin_name(name)?;
    let path = plugins_root()?.join(&name);
    if !path.exists() {
        anyhow::bail!("plugin is not installed: {name}");
    }
    std::fs::remove_dir_all(&path)?;
    Ok(path)
}

fn prepare_dest(dest: &Path, force: bool) -> anyhow::Result<()> {
    if !dest.exists() {
        return Ok(());
    }
    if !force {
        anyhow::bail!(
            "plugin destination already exists: {} (pass --force to replace)",
            dest.display()
        );
    }
    std::fs::remove_dir_all(dest)?;
    Ok(())
}

fn copy_dir(src: &Path, dest: &Path) -> anyhow::Result<()> {
    std::fs::create_dir_all(dest)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let from = entry.path();
        let to = dest.join(entry.file_name());
        if from.is_dir() {
            copy_dir(&from, &to)?;
        } else {
            std::fs::copy(&from, &to)?;
        }
    }
    Ok(())
}

fn plugins_root() -> anyhow::Result<PathBuf> {
    let root = dirs::config_dir()
        .ok_or_else(|| anyhow::anyhow!("could not resolve config directory"))?
        .join("jfc")
        .join("plugins");
    std::fs::create_dir_all(&root)?;
    Ok(root)
}

fn manifest_name(path: &Path) -> Option<String> {
    let jfc_manifest = path.join(".jfc-plugin.toml");
    if let Ok(text) = std::fs::read_to_string(jfc_manifest)
        && let Ok(value) = text.parse::<toml::Value>()
        && let Some(name) = value
            .get("plugin")
            .and_then(|p| p.get("name"))
            .and_then(|v| v.as_str())
    {
        return Some(name.to_owned());
    }
    let codex_manifest = path.join(".codex-plugin").join("plugin.json");
    if let Ok(text) = std::fs::read_to_string(codex_manifest)
        && let Ok(value) = serde_json::from_str::<serde_json::Value>(&text)
        && let Some(name) = value.get("name").and_then(|v| v.as_str())
    {
        return Some(name.to_owned());
    }
    None
}

fn workflow_dir_for_plugin(path: &Path) -> Option<PathBuf> {
    let manifest = path.join(".jfc-plugin.toml");
    if let Ok(text) = std::fs::read_to_string(manifest)
        && let Ok(value) = text.parse::<toml::Value>()
        && let Some(dir) = value
            .get("plugin")
            .and_then(|p| p.get("workflows_dir"))
            .and_then(|v| v.as_str())
    {
        return Some(path.join(dir));
    }
    let workflows = path.join("workflows");
    if workflows.is_dir() {
        Some(workflows)
    } else {
        Some(path.to_path_buf())
    }
}

fn looks_like_git_url(source: &str) -> bool {
    source.starts_with("http://")
        || source.starts_with("https://")
        || source.starts_with("ssh://")
        || source.starts_with("git@")
        || source.ends_with(".git")
}

fn plugin_name_from_url(url: &str) -> anyhow::Result<String> {
    let trimmed = url.trim_end_matches('/').trim_end_matches(".git");
    let stem = trimmed
        .rsplit(['/', ':'])
        .next()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| anyhow::anyhow!("cannot infer plugin name from URL: {url}"))?;
    sanitize_plugin_name(stem)
}

fn sanitize_plugin_name(name: &str) -> anyhow::Result<String> {
    let trimmed = name.trim();
    if trimmed.is_empty()
        || trimmed == "."
        || trimmed == ".."
        || trimmed.contains('/')
        || trimmed.contains('\\')
        || !trimmed
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
    {
        anyhow::bail!("invalid plugin name: {name:?}");
    }
    Ok(trimmed.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_rejects_path_traversal_robust() {
        assert!(sanitize_plugin_name("../x").is_err());
        assert!(sanitize_plugin_name("x/y").is_err());
        assert!(sanitize_plugin_name("ok-name_1.2").is_ok());
    }

    #[test]
    fn plugin_name_from_url_strips_git_suffix_normal() {
        assert_eq!(
            plugin_name_from_url("https://example.com/acme/review.git").unwrap(),
            "review"
        );
        assert_eq!(
            plugin_name_from_url("git@example.com:acme/review").unwrap(),
            "review"
        );
    }
}

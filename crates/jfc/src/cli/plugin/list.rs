use std::path::{Path, PathBuf};

use jfc_plugin_host::{
    PluginDiscoveryOptions, PluginDiscoverySearchRoot, PluginHostSnapshot,
    cached_discovered_resource_plugin_state,
};
use jfc_plugin_sdk::PluginSource;

use super::store::{plugins_root, workflow_dir_for_plugin};

pub(super) fn list_plugins() -> anyhow::Result<String> {
    let root = plugins_root()?;
    list_plugins_in(&root)
}

fn list_plugins_in(root: &Path) -> anyhow::Result<String> {
    let mut out = String::new();
    out.push_str(&format!("plugins: {}\n", root.display()));
    let state = cached_discovered_resource_plugin_state(
        PluginDiscoveryOptions::new()
            .with_search_root(PluginDiscoverySearchRoot::global_plugins_dir(root)),
    )?;
    let snapshot = state.host.status_snapshot();
    if snapshot.plugins.is_empty() {
        out.push_str("(none)\n");
        return Ok(out);
    }
    for row in plugin_list_rows(snapshot) {
        out.push_str(&format!(
            "- {} [{}] workflows={} path={}\n",
            row.name,
            row.source,
            row.workflows,
            row.path.display()
        ));
    }
    Ok(out)
}

struct PluginListRow {
    name: String,
    source: &'static str,
    workflows: usize,
    path: PathBuf,
}

fn plugin_list_rows(snapshot: PluginHostSnapshot) -> Vec<PluginListRow> {
    let mut rows = Vec::new();
    for plugin in snapshot.plugins {
        let path = match &plugin.source {
            PluginSource::User { root }
            | PluginSource::Project { root }
            | PluginSource::Workspace { root } => PathBuf::from(root),
            PluginSource::BuiltIn { .. }
            | PluginSource::Package { .. }
            | PluginSource::ProcessBridge { .. } => continue,
        };
        let source = if path.join(".git").is_dir() {
            "git"
        } else {
            "local"
        };
        let plugin_id = plugin.manifest.id.as_str().to_owned();
        rows.push(PluginListRow {
            name: plugin.manifest.display_name.unwrap_or(plugin_id),
            source,
            workflows: count_workflows(&path),
            path,
        });
    }
    rows.sort_by(|left, right| left.name.cmp(&right.name));
    rows
}

fn count_workflows(path: &Path) -> usize {
    workflow_dir_for_plugin(path)
        .and_then(|dir| std::fs::read_dir(dir).ok())
        .map(|entries| {
            entries
                .flatten()
                .filter(|e| e.path().extension().and_then(|s| s.to_str()) == Some("js"))
                .count()
        })
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plugin_list_uses_host_discovery_and_manifest_workflow_dir_normal() {
        let tmp = tempfile::tempdir().unwrap();
        let plugin = tmp.path().join("acme");
        let workflows = plugin.join("flows");
        std::fs::create_dir_all(&workflows).unwrap();
        std::fs::write(
            plugin.join(".jfc-plugin.toml"),
            "[plugin]\nname = \"acme-tools\"\nworkflows_dir = \"flows\"\n",
        )
        .unwrap();
        std::fs::write(workflows.join("review.js"), "export default {}").unwrap();

        let listing = list_plugins_in(tmp.path()).unwrap();

        assert!(listing.contains("plugins: "));
        assert!(listing.contains("- acme-tools [local] workflows=1 path="));
    }
}

use std::path::{Path, PathBuf};

use jfc_plugin_host::{
    PluginDiscoveryOptions, PluginDiscoverySearchRoot, cached_discovered_resource_plugin_state,
};
use jfc_plugin_sdk::{ResourceDescriptor, ResourceKind};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ResourceRoot {
    pub(crate) path: PathBuf,
    pub(crate) namespace: Option<String>,
}

pub(crate) fn skill_resource_roots(project_root: &Path) -> Vec<ResourceRoot> {
    resource_roots_for(project_root, ResourceKind::Skill)
}

pub(crate) fn agent_resource_roots(project_root: &Path) -> Vec<ResourceRoot> {
    resource_roots_for(project_root, ResourceKind::Agent)
}

fn resource_roots_for(project_root: &Path, kind: ResourceKind) -> Vec<ResourceRoot> {
    let mut roots =
        match cached_discovered_resource_plugin_state(plugin_discovery_options_for(project_root)) {
            Ok(state) => resource_roots_from_descriptors(state.host.resource_descriptors(), kind),
            Err(error) => {
                tracing::warn!(
                    target: "jfc::agents",
                    error = %error,
                    "failed to activate agent/skill resource plugins"
                );
                Vec::new()
            }
        };
    roots.sort_by(|left, right| {
        left.path
            .cmp(&right.path)
            .then_with(|| left.namespace.cmp(&right.namespace))
    });
    roots.dedup_by(|left, right| left.path == right.path && left.namespace == right.namespace);
    roots
}

fn resource_roots_from_descriptors<I>(resources: I, kind: ResourceKind) -> Vec<ResourceRoot>
where
    I: IntoIterator<Item = ResourceDescriptor>,
{
    resources
        .into_iter()
        .filter(|descriptor| descriptor.kind == kind)
        .map(|descriptor| ResourceRoot {
            path: PathBuf::from(descriptor.path),
            namespace: descriptor.namespace,
        })
        .collect()
}

fn plugin_discovery_options_for(project_root: &Path) -> PluginDiscoveryOptions {
    let settings = jfc_config::claude_settings::load_merged(project_root);
    let mut options = PluginDiscoveryOptions::new();

    if let Some(home) = dirs::home_dir() {
        options = options.with_search_root(PluginDiscoverySearchRoot::global_plugins_dir(
            home.join(".claude/plugins"),
        ));
    }
    if let Some(config) = dirs::config_dir() {
        options = options.with_search_root(PluginDiscoverySearchRoot::global_plugins_dir(
            config.join("jfc/plugins"),
        ));
    }

    for path in [
        project_root.join(".claude/plugins"),
        project_root.join("plugins"),
        project_root.join(".agents/plugins"),
        project_root.join(".codex/plugins"),
    ] {
        options = options.with_search_root(PluginDiscoverySearchRoot::project_plugins_dir(path));
    }

    for (plugin, enabled) in settings.enabled_plugins {
        if !enabled {
            options = options.with_disabled_plugin(plugin);
        }
    }

    options
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resource_roots_keep_descriptor_namespace_normal() {
        let roots = resource_roots_from_descriptors(
            [ResourceDescriptor::new(
                jfc_plugin_sdk::PluginId::new("sec-plugin"),
                ResourceKind::Skill,
                "/tmp/sec/skills",
            )
            .with_namespace("sec")],
            ResourceKind::Skill,
        );

        assert_eq!(
            roots,
            vec![ResourceRoot {
                path: PathBuf::from("/tmp/sec/skills"),
                namespace: Some("sec".to_owned()),
            }]
        );
    }
}

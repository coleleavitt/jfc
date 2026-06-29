use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use jfc_plugin_host::{
    PluginDiscoveryOptions, PluginDiscoverySearchRoot, cached_discovered_resource_plugin_state,
};
use jfc_plugin_sdk::{ResourceDescriptor, ResourceKind};

static EXTRA_PLUGIN_DIRS: OnceLock<Mutex<Vec<PathBuf>>> = OnceLock::new();

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowResource {
    pub path: PathBuf,
    pub resource_path: String,
}

pub fn register_extra_plugin_dir(path: PathBuf) {
    let slot = EXTRA_PLUGIN_DIRS.get_or_init(|| Mutex::new(Vec::new()));
    let mut dirs = slot.lock().unwrap_or_else(|e| e.into_inner());
    if !dirs.iter().any(|p| p == &path) {
        dirs.push(path);
    }
}

pub fn plugin_workflow_resources_for(project_root: &Path) -> Vec<WorkflowResource> {
    let mut resources =
        match cached_discovered_resource_plugin_state(plugin_discovery_options_for(project_root)) {
            Ok(state) => workflow_resources_from_descriptors(state.host.resource_descriptors()),
            Err(error) => {
                tracing::warn!(
                    target: "jfc::plugin_host",
                    error = %error,
                    "failed to activate workflow resource plugins"
                );
                Vec::new()
            }
        };
    resources.sort_by(|left, right| left.path.cmp(&right.path));
    resources.dedup_by(|left, right| left.path == right.path);
    resources
}

fn workflow_resources_from_descriptors<I>(resources: I) -> Vec<WorkflowResource>
where
    I: IntoIterator<Item = ResourceDescriptor>,
{
    resources
        .into_iter()
        .filter(|descriptor| descriptor.kind == ResourceKind::Workflow)
        .map(|descriptor| WorkflowResource {
            path: PathBuf::from(&descriptor.path),
            resource_path: descriptor.path,
        })
        .collect()
}

pub fn builtin_workflow_resource_descriptors() -> Vec<ResourceDescriptor> {
    match jfc_plugin_host::builtin_agent_workflow_plugin_host() {
        Ok(host) => host
            .resource_descriptors()
            .into_iter()
            .filter(|descriptor| descriptor.kind == ResourceKind::Workflow)
            .collect(),
        Err(error) => {
            tracing::warn!(
                target: "jfc::plugin_host",
                error = %error,
                "failed to activate built-in workflow resource plugin"
            );
            Vec::new()
        }
    }
}

pub fn plugin_discovery_options_for(project_root: &Path) -> PluginDiscoveryOptions {
    let settings = crate::config::claude_settings::load_merged(project_root);
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

    options = options
        .with_search_root(PluginDiscoverySearchRoot::project_plugins_dir(
            project_root.join("plugins"),
        ))
        .with_search_root(PluginDiscoverySearchRoot::project_plugins_dir(
            project_root.join(".claude/plugins"),
        ));

    for path in extra_plugin_dirs() {
        options = options.with_search_root(PluginDiscoverySearchRoot::plugin_root(path));
    }

    for (plugin, enabled) in settings.enabled_plugins {
        if !enabled {
            options = options.with_disabled_plugin(plugin);
        }
    }

    options
}

fn extra_plugin_dirs() -> Vec<PathBuf> {
    EXTRA_PLUGIN_DIRS
        .get()
        .and_then(|slot| slot.lock().ok().map(|dirs| dirs.clone()))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_plugin_manifest_workflow_dir_is_discovered_normal() {
        let tmp = tempfile::TempDir::new().unwrap();
        let plugin_dir = tmp.path().join("plugins").join("my-plugin");
        let workflow_dir = plugin_dir.join("commands");
        std::fs::create_dir_all(&workflow_dir).unwrap();
        std::fs::write(
            plugin_dir.join(".jfc-plugin.toml"),
            "[plugin]\nname = \"my-plugin\"\nworkflows_dir = \"commands\"\n",
        )
        .unwrap();

        assert!(
            plugin_workflow_resources_for(tmp.path())
                .into_iter()
                .any(|resource| resource.path == workflow_dir)
        );
    }

    #[test]
    fn workflow_resources_are_read_from_resource_descriptors_normal() {
        let descriptors = [
            ResourceDescriptor::new(
                jfc_plugin_sdk::PluginId::new("test.plugin"),
                ResourceKind::Workflow,
                "/tmp/jfc-workflows",
            ),
            ResourceDescriptor::new(
                jfc_plugin_sdk::PluginId::new("test.plugin"),
                ResourceKind::Skill,
                "/tmp/jfc-skills",
            ),
        ];

        let resources = workflow_resources_from_descriptors(descriptors);

        assert_eq!(
            resources,
            vec![WorkflowResource {
                path: PathBuf::from("/tmp/jfc-workflows"),
                resource_path: "/tmp/jfc-workflows".to_owned(),
            }]
        );
    }

    #[test]
    fn enabled_plugins_false_disables_project_plugin_normal() {
        let tmp = tempfile::TempDir::new().unwrap();
        let plugin_dir = tmp.path().join("plugins").join("my-plugin");
        std::fs::create_dir_all(plugin_dir.join("workflows")).unwrap();
        std::fs::create_dir_all(tmp.path().join(".claude")).unwrap();
        std::fs::write(
            plugin_dir.join(".jfc-plugin.toml"),
            "[plugin]\nname = \"my-plugin\"\nworkflows_dir = \"workflows\"\n",
        )
        .unwrap();
        std::fs::write(
            tmp.path().join(".claude/settings.json"),
            r#"{ "enabledPlugins": { "my-plugin@local": false } }"#,
        )
        .unwrap();

        assert!(
            !plugin_workflow_resources_for(tmp.path())
                .into_iter()
                .any(|resource| resource.path == plugin_dir.join("workflows"))
        );
    }

    #[test]
    fn builtin_workflow_resource_is_read_from_first_party_plugin_pack_normal() {
        let resources = builtin_workflow_resource_descriptors();
        assert_eq!(resources.len(), 1);
        let descriptor = &resources[0];
        assert_eq!(
            descriptor.plugin_id.as_str(),
            jfc_plugin_host::BUILTIN_WORKFLOWS_PLUGIN_ID
        );
        assert_eq!(descriptor.kind, jfc_plugin_sdk::ResourceKind::Workflow);
        assert_eq!(
            descriptor.path,
            jfc_plugin_host::BUILTIN_WORKFLOW_RESOURCE_PATH
        );
        assert_eq!(descriptor.namespace.as_deref(), Some("builtin"));
    }
}

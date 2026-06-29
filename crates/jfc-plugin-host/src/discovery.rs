use std::{collections::HashSet, path::PathBuf};

use jfc_plugin_sdk::{
    AgentLaunchDescriptor, MetricDescriptor, PluginScope, PluginSource, ProcessBridgeCommand,
    ProviderDescriptor, RuntimeActionDescriptor, RuntimeExtensionDescriptor, ToolDescriptor,
    UiPanelDescriptor, UiSlotDescriptor, UiWidgetDescriptor,
};

use crate::manifest::read_manifest;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PluginRootKind {
    Global,
    Project,
    PluginRoot,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginDiscoverySearchRoot {
    path: PathBuf,
    kind: PluginRootKind,
}

impl PluginDiscoverySearchRoot {
    pub fn global_plugins_dir(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            kind: PluginRootKind::Global,
        }
    }

    pub fn project_plugins_dir(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            kind: PluginRootKind::Project,
        }
    }

    pub fn plugin_root(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            kind: PluginRootKind::PluginRoot,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct PluginDiscoveryOptions {
    search_roots: Vec<PluginDiscoverySearchRoot>,
    disabled_plugins: HashSet<String>,
}

impl PluginDiscoveryOptions {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_search_root(mut self, root: PluginDiscoverySearchRoot) -> Self {
        self.search_roots.push(root);
        self
    }

    pub fn with_disabled_plugin(mut self, plugin: impl Into<String>) -> Self {
        self.disabled_plugins.insert(plugin.into());
        self
    }

    pub(crate) fn cache_key(&self) -> String {
        let mut key = String::new();
        for root in &self.search_roots {
            key.push_str(match root.kind {
                PluginRootKind::Global => "global:",
                PluginRootKind::Project => "project:",
                PluginRootKind::PluginRoot => "root:",
            });
            key.push_str(&root.path.to_string_lossy());
            key.push('\n');
        }
        let mut disabled = self.disabled_plugins.iter().collect::<Vec<_>>();
        disabled.sort();
        for plugin in disabled {
            key.push_str("disabled:");
            key.push_str(plugin);
            key.push('\n');
        }
        key
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct DiscoveredPluginRoot {
    pub path: PathBuf,
    pub identity: String,
    pub namespace: String,
    pub kind: PluginRootKind,
    pub source: PluginSource,
    pub scope: PluginScope,
    pub(crate) tool_descriptors: Vec<ToolDescriptor>,
    pub(crate) provider_descriptors: Vec<ProviderDescriptor>,
    pub(crate) ui_slot_descriptors: Vec<UiSlotDescriptor>,
    pub(crate) ui_panel_descriptors: Vec<UiPanelDescriptor>,
    pub(crate) ui_widget_descriptors: Vec<UiWidgetDescriptor>,
    pub(crate) metric_descriptors: Vec<MetricDescriptor>,
    pub(crate) runtime_action_descriptors: Vec<RuntimeActionDescriptor>,
    pub(crate) runtime_extension_descriptors: Vec<RuntimeExtensionDescriptor>,
    pub(crate) agent_launch_descriptors: Vec<AgentLaunchDescriptor>,
    pub(crate) process_bridge: Option<ProcessBridgeCommand>,
    workflow_dir: PathBuf,
}

impl DiscoveredPluginRoot {
    pub fn workflow_dir(&self) -> WorkflowDirectory {
        WorkflowDirectory {
            path: self.workflow_dir.clone(),
            plugin_identity: self.identity.clone(),
            namespace: self.namespace.clone(),
            kind: self.kind,
            source: self.source.clone(),
            scope: self.scope,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowDirectory {
    pub path: PathBuf,
    pub plugin_identity: String,
    pub namespace: String,
    pub kind: PluginRootKind,
    pub source: PluginSource,
    pub scope: PluginScope,
}

pub struct PluginDiscovery;

impl PluginDiscovery {
    pub fn discover(options: PluginDiscoveryOptions) -> Vec<DiscoveredPluginRoot> {
        let mut discovered = Vec::new();
        let mut seen_identities = HashSet::new();

        for search_root in options.search_roots {
            for plugin_root in plugin_roots(search_root) {
                let Some(plugin) = describe_plugin_root(plugin_root) else {
                    continue;
                };
                if plugin_disabled(
                    &plugin.identity,
                    &plugin.namespace,
                    &options.disabled_plugins,
                ) {
                    continue;
                }
                if seen_identities.insert(plugin.identity.clone()) {
                    discovered.push(plugin);
                }
            }
        }

        discovered.sort_by(|left, right| left.identity.cmp(&right.identity));
        discovered
    }
}

fn plugin_roots(search_root: PluginDiscoverySearchRoot) -> Vec<PluginRootCandidate> {
    match search_root.kind {
        PluginRootKind::PluginRoot => vec![PluginRootCandidate {
            path: search_root.path,
            kind: search_root.kind,
        }],
        PluginRootKind::Global | PluginRootKind::Project => {
            let Ok(entries) = std::fs::read_dir(search_root.path) else {
                return Vec::new();
            };
            entries
                .flatten()
                .map(|entry| entry.path())
                .filter(|path| path.is_dir())
                .filter(|path| {
                    path.file_name()
                        .and_then(|name| name.to_str())
                        .is_some_and(|name| !name.starts_with('.'))
                })
                .map(|path| PluginRootCandidate {
                    path,
                    kind: search_root.kind,
                })
                .collect()
        }
    }
}

#[derive(Debug, Clone)]
struct PluginRootCandidate {
    path: PathBuf,
    kind: PluginRootKind,
}

fn describe_plugin_root(candidate: PluginRootCandidate) -> Option<DiscoveredPluginRoot> {
    let namespace = namespace_for_path(&candidate.path, candidate.kind)?;
    let manifest = read_manifest(&candidate.path);
    let identity = manifest
        .as_ref()
        .and_then(|manifest| manifest.name.clone())
        .unwrap_or_else(|| namespace.clone());
    let workflow_dir = manifest
        .as_ref()
        .and_then(|manifest| manifest.workflows_dir.clone())
        .map(|dir| candidate.path.join(dir))
        .unwrap_or_else(|| default_workflow_dir(&candidate.path));
    let plugin_id = jfc_plugin_sdk::PluginId::new(identity.clone());
    let tool_descriptors = manifest
        .as_ref()
        .map(|manifest| manifest.tool_descriptors(&plugin_id, &candidate.path))
        .unwrap_or_default();
    let provider_descriptors = manifest
        .as_ref()
        .map(|manifest| manifest.provider_descriptors(&plugin_id, &candidate.path))
        .unwrap_or_default();
    let ui_slot_descriptors = manifest
        .as_ref()
        .map(|manifest| manifest.ui_slot_descriptors(&plugin_id))
        .unwrap_or_default();
    let ui_panel_descriptors = manifest
        .as_ref()
        .map(|manifest| manifest.ui_panel_descriptors(&plugin_id, &candidate.path))
        .unwrap_or_default();
    let ui_widget_descriptors = manifest
        .as_ref()
        .map(|manifest| manifest.ui_widget_descriptors(&plugin_id, &candidate.path))
        .unwrap_or_default();
    let metric_descriptors = manifest
        .as_ref()
        .map(|manifest| manifest.metric_descriptors(&plugin_id))
        .unwrap_or_default();
    let runtime_action_descriptors = manifest
        .as_ref()
        .map(|manifest| manifest.runtime_action_descriptors(&plugin_id))
        .unwrap_or_default();
    let runtime_extension_descriptors = manifest
        .as_ref()
        .map(|manifest| manifest.runtime_extension_descriptors(&plugin_id, &candidate.path))
        .unwrap_or_default();
    let agent_launch_descriptors = manifest
        .as_ref()
        .map(|manifest| manifest.agent_launch_descriptors(&plugin_id, &candidate.path))
        .unwrap_or_default();
    let process_bridge = manifest
        .as_ref()
        .and_then(|manifest| manifest.resolved_process_bridge(&candidate.path));
    let source = source_for(candidate.kind, &candidate.path);
    let scope = scope_for(candidate.kind);

    Some(DiscoveredPluginRoot {
        path: candidate.path,
        identity,
        namespace,
        kind: candidate.kind,
        source,
        scope,
        tool_descriptors,
        provider_descriptors,
        ui_slot_descriptors,
        ui_panel_descriptors,
        ui_widget_descriptors,
        metric_descriptors,
        runtime_action_descriptors,
        runtime_extension_descriptors,
        agent_launch_descriptors,
        process_bridge,
        workflow_dir,
    })
}

fn namespace_for_path(path: &std::path::Path, kind: PluginRootKind) -> Option<String> {
    let name = path.file_name().and_then(|name| name.to_str())?;
    match kind {
        PluginRootKind::PluginRoot => Some(name.to_owned()),
        PluginRootKind::Global | PluginRootKind::Project => {
            (!name.starts_with('.')).then(|| name.to_owned())
        }
    }
}

fn default_workflow_dir(path: &std::path::Path) -> PathBuf {
    let workflows = path.join("workflows");
    if workflows.is_dir() {
        workflows
    } else {
        path.to_path_buf()
    }
}

fn source_for(kind: PluginRootKind, path: &std::path::Path) -> PluginSource {
    let root = path.to_string_lossy().into_owned();
    match kind {
        PluginRootKind::Global | PluginRootKind::PluginRoot => PluginSource::User { root },
        PluginRootKind::Project => PluginSource::Project { root },
    }
}

const fn scope_for(kind: PluginRootKind) -> PluginScope {
    match kind {
        PluginRootKind::Global | PluginRootKind::PluginRoot => PluginScope::User,
        PluginRootKind::Project => PluginScope::Project,
    }
}

fn plugin_disabled(identity: &str, namespace: &str, disabled_plugins: &HashSet<String>) -> bool {
    disabled_plugins
        .iter()
        .map(|plugin| plugin.trim())
        .any(|plugin| matches_plugin(plugin, identity) || matches_plugin(plugin, namespace))
}

fn matches_plugin(configured: &str, discovered: &str) -> bool {
    configured == discovered
        || configured
            .split_once('@')
            .map(|(name, _source)| name == discovered)
            .unwrap_or(false)
}

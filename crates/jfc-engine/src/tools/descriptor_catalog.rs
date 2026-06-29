use std::collections::HashSet;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::{OnceLock, RwLock};

use jfc_plugin_host::{
    PluginDiscoveryOptions, PluginHostError, cached_discovered_resource_plugin_state,
    reload_cached_discovered_resource_plugin_state,
};
use jfc_plugin_sdk::{DescriptorVisibility, ToolDescriptor, ToolExecutorKind};
use jfc_provider::ToolDef;

fn external_tool_descriptors_handle() -> &'static RwLock<Vec<ToolDescriptor>> {
    static H: OnceLock<RwLock<Vec<ToolDescriptor>>> = OnceLock::new();
    H.get_or_init(|| RwLock::new(Vec::new()))
}

pub fn register_external_tool_descriptors<I>(descriptors: I) -> usize
where
    I: IntoIterator<Item = ToolDescriptor>,
{
    let descriptors = external_tool_descriptors_from(descriptors);
    let count = descriptors.len();
    if let Ok(mut handle) = external_tool_descriptors_handle().write() {
        *handle = descriptors;
    }
    count
}

pub fn register_discovered_plugin_tool_descriptors(
    options: PluginDiscoveryOptions,
) -> Result<usize, PluginHostError> {
    let state = cached_discovered_resource_plugin_state(options)?;
    Ok(register_external_tool_descriptors(
        state.host.tool_descriptors(),
    ))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternalToolDescriptorReload {
    pub before_count: usize,
    pub after_count: usize,
    pub before_digest: String,
    pub after_digest: String,
    pub changed: bool,
    pub host_descriptor_digest: String,
}

pub fn reload_discovered_plugin_tool_descriptors(
    options: PluginDiscoveryOptions,
) -> Result<ExternalToolDescriptorReload, PluginHostError> {
    let before = snapshot_external_tool_descriptors();
    let before_digest = external_tool_descriptor_digest(&before);
    let reload = reload_cached_discovered_resource_plugin_state(options, None)?;
    let next_descriptors = external_tool_descriptors_from(reload.host.tool_descriptors());
    let after_digest = external_tool_descriptor_digest(&next_descriptors);
    let report = ExternalToolDescriptorReload {
        before_count: before.len(),
        after_count: next_descriptors.len(),
        before_digest,
        after_digest: after_digest.clone(),
        changed: before != next_descriptors,
        host_descriptor_digest: reload.report.diagnostics.descriptor_digest.clone(),
    };
    if let Ok(mut handle) = external_tool_descriptors_handle().write() {
        *handle = next_descriptors;
    }
    Ok(report)
}

pub(crate) fn snapshot_external_tool_descriptors() -> Vec<ToolDescriptor> {
    external_tool_descriptors_handle()
        .read()
        .map(|handle| handle.clone())
        .unwrap_or_default()
}

pub(crate) fn external_tool_defs(existing_names: &HashSet<String>) -> Vec<ToolDef> {
    snapshot_external_tool_descriptors()
        .into_iter()
        .filter(|descriptor| descriptor.visibility == DescriptorVisibility::ModelVisible)
        .filter(|descriptor| !existing_names.contains(&descriptor.name))
        .map(|descriptor| ToolDef {
            name: descriptor.name,
            description: descriptor.description,
            input_schema: descriptor.input_schema,
        })
        .collect()
}

#[cfg(test)]
pub(crate) fn clear_external_tool_descriptors_for_tests() {
    if let Ok(mut handle) = external_tool_descriptors_handle().write() {
        handle.clear();
    }
}

fn external_tool_descriptors_from<I>(descriptors: I) -> Vec<ToolDescriptor>
where
    I: IntoIterator<Item = ToolDescriptor>,
{
    descriptors
        .into_iter()
        .filter(|descriptor| descriptor.visibility != DescriptorVisibility::Internal)
        .filter(|descriptor| descriptor.executor.kind != ToolExecutorKind::BuiltIn)
        .collect()
}

fn external_tool_descriptor_digest(descriptors: &[ToolDescriptor]) -> String {
    let mut rows = descriptors
        .iter()
        .map(|descriptor| match serde_json::to_string(descriptor) {
            Ok(json) => json,
            Err(error) => format!("serde_error:{error}:{descriptor:?}"),
        })
        .collect::<Vec<_>>();
    rows.sort();
    let mut hasher = DefaultHasher::new();
    rows.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

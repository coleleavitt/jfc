use jfc_plugin_sdk::{
    DescriptorVisibility, MetricDescriptor, MetricSurface, MetricUnit, PluginCapability, PluginId,
    PluginManifest, PluginScope, PluginSource, PluginVersion,
};

use crate::{PluginHost, PluginHostError, PluginRegistration};

const METRICS_PLUGIN_VERSION: &str = "0.1.0";
pub const BUILTIN_OBSERVABILITY_PLUGIN_ID: &str = "builtin.jfc-observability";
pub const BUILTIN_CACHE_HIT_METRIC_ID: &str = "cache.hit_rate";
pub const BUILTIN_CACHE_DIGEST_METRIC_ID: &str = "cache.descriptor_digest";
pub const BUILTIN_RSI_PROMPT_SECTIONS_METRIC_ID: &str = "rsi.prompt_sections";
pub const BUILTIN_RSI_TOOL_VISIBILITY_METRIC_ID: &str = "rsi.tool_visibility_rules";

pub fn builtin_observability_plugin_host() -> Result<PluginHost, PluginHostError> {
    let mut host = PluginHost::new();
    host.register_internal(builtin_observability_plugin())?;
    host.activate_all()?;
    Ok(host)
}

pub fn builtin_observability_plugin() -> PluginRegistration {
    let plugin_id = PluginId::new(BUILTIN_OBSERVABILITY_PLUGIN_ID);
    let manifest = PluginManifest::new(
        plugin_id.clone(),
        PluginVersion::new(METRICS_PLUGIN_VERSION),
        PluginSource::built_in("jfc-observability"),
    )
    .with_display_name("JFC Observability")
    .with_description("Built-in cache diagnostics and RSI runtime metric descriptors")
    .with_scope(PluginScope::Workspace)
    .with_capability(PluginCapability::Metrics {
        surfaces: vec![
            MetricSurface::StatusLine,
            MetricSurface::Sidebar,
            MetricSurface::Panel,
        ],
    });

    PluginRegistration::new(manifest).with_metric_descriptors([
        MetricDescriptor::new(
            plugin_id.clone(),
            BUILTIN_CACHE_HIT_METRIC_ID,
            "Cache hit rate",
            "Prompt/cache-read tokens divided by input tokens for the session",
            MetricUnit::Percent,
        )
        .with_surfaces([MetricSurface::StatusLine, MetricSurface::Sidebar])
        .with_priority(84)
        .with_visibility(DescriptorVisibility::HostVisible),
        MetricDescriptor::new(
            plugin_id.clone(),
            BUILTIN_CACHE_DIGEST_METRIC_ID,
            "Plugin descriptor digest",
            "Current discovered plugin descriptor digest and reload freshness",
            MetricUnit::Digest,
        )
        .with_surface(MetricSurface::Sidebar)
        .with_priority(42)
        .with_visibility(DescriptorVisibility::HostVisible),
        MetricDescriptor::new(
            plugin_id.clone(),
            BUILTIN_RSI_PROMPT_SECTIONS_METRIC_ID,
            "RSI prompt sections",
            "Active RSI runtime guidance sections injected into the latest request",
            MetricUnit::Count,
        )
        .with_surfaces([MetricSurface::Sidebar, MetricSurface::Panel])
        .with_priority(72)
        .with_visibility(DescriptorVisibility::HostVisible),
        MetricDescriptor::new(
            plugin_id,
            BUILTIN_RSI_TOOL_VISIBILITY_METRIC_ID,
            "RSI tool visibility rules",
            "Active RSI runtime tool visibility rules injected into the latest request",
            MetricUnit::Count,
        )
        .with_surfaces([MetricSurface::Sidebar, MetricSurface::Panel])
        .with_priority(70)
        .with_visibility(DescriptorVisibility::HostVisible),
    ])
}

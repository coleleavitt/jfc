use jfc_plugin_sdk::{DescriptorVisibility, MetricDescriptor, MetricSurface, MetricUnit, PluginId};
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct ManifestMetricDescriptor {
    id: String,
    label: String,
    description: String,
    unit: MetricUnit,
    #[serde(default)]
    surfaces: Vec<MetricSurface>,
    #[serde(default)]
    priority: Option<i32>,
    #[serde(default)]
    visibility: Option<DescriptorVisibility>,
}

impl ManifestMetricDescriptor {
    pub(crate) fn to_metric_descriptor(&self, plugin_id: &PluginId) -> MetricDescriptor {
        MetricDescriptor::new(
            plugin_id.clone(),
            self.id.clone(),
            self.label.clone(),
            self.description.clone(),
            self.unit,
        )
        .with_surfaces(self.surfaces.clone())
        .with_priority(self.priority.unwrap_or_default())
        .with_visibility(self.visibility.unwrap_or(DescriptorVisibility::HostVisible))
    }
}

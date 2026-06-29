use serde::{Deserialize, Serialize};

use crate::{PluginId, descriptor::DescriptorVisibility};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MetricSurface {
    StatusLine,
    Sidebar,
    Panel,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MetricUnit {
    Count,
    Percent,
    Tokens,
    Usd,
    Digest,
    Text,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct MetricDescriptor {
    pub plugin_id: PluginId,
    pub id: String,
    pub label: String,
    pub description: String,
    pub unit: MetricUnit,
    pub surfaces: Vec<MetricSurface>,
    pub priority: i32,
    pub visibility: DescriptorVisibility,
}

impl MetricDescriptor {
    pub fn new(
        plugin_id: PluginId,
        id: impl Into<String>,
        label: impl Into<String>,
        description: impl Into<String>,
        unit: MetricUnit,
    ) -> Self {
        let _linkscope_metric = linkscope::phase("plugin_sdk.metric.new");
        let id = id.into();
        let label = label.into();
        let description = description.into();
        linkscope::event_fields(
            "plugin_sdk.metric.new",
            [
                linkscope::TraceField::text("plugin_id", plugin_id.as_str().to_owned()),
                linkscope::TraceField::text("id", id.clone()),
                linkscope::TraceField::text("unit", format!("{unit:?}")),
                linkscope::TraceField::bytes(
                    "label_bytes",
                    u64::try_from(label.len()).unwrap_or(u64::MAX),
                ),
                linkscope::TraceField::bytes(
                    "description_bytes",
                    u64::try_from(description.len()).unwrap_or(u64::MAX),
                ),
            ],
        );
        Self {
            plugin_id,
            id,
            label,
            description,
            unit,
            surfaces: Vec::new(),
            priority: 0,
            visibility: DescriptorVisibility::HostVisible,
        }
    }

    pub fn with_surface(mut self, surface: MetricSurface) -> Self {
        let _linkscope_surface = linkscope::phase("plugin_sdk.metric.with_surface");
        self.surfaces.push(surface);
        linkscope::detail_event_fields(
            "plugin_sdk.metric.with_surface",
            [
                linkscope::TraceField::text("id", self.id.clone()),
                linkscope::TraceField::text("surface", format!("{surface:?}")),
                linkscope::TraceField::count(
                    "surfaces",
                    u64::try_from(self.surfaces.len()).unwrap_or(u64::MAX),
                ),
            ],
        );
        self
    }

    pub fn with_surfaces<I>(mut self, surfaces: I) -> Self
    where
        I: IntoIterator<Item = MetricSurface>,
    {
        let _linkscope_surfaces = linkscope::phase("plugin_sdk.metric.with_surfaces");
        let before = self.surfaces.len();
        self.surfaces.extend(surfaces);
        linkscope::detail_event_fields(
            "plugin_sdk.metric.with_surfaces",
            [
                linkscope::TraceField::text("id", self.id.clone()),
                linkscope::TraceField::count("before", u64::try_from(before).unwrap_or(u64::MAX)),
                linkscope::TraceField::count(
                    "after",
                    u64::try_from(self.surfaces.len()).unwrap_or(u64::MAX),
                ),
            ],
        );
        self
    }

    pub fn with_priority(mut self, priority: i32) -> Self {
        self.priority = priority;
        self
    }

    pub fn with_visibility(mut self, visibility: DescriptorVisibility) -> Self {
        self.visibility = visibility;
        self
    }
}

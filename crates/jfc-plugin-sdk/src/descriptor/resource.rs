use serde::{Deserialize, Serialize};

use crate::{PluginId, PluginScope, PluginSource, ResourceKind};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ResourceDescriptor {
    pub plugin_id: PluginId,
    pub kind: ResourceKind,
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<PluginSource>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<PluginScope>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub namespace: Option<String>,
}

impl ResourceDescriptor {
    pub fn new(plugin_id: PluginId, kind: ResourceKind, path: impl Into<String>) -> Self {
        let _linkscope_resource = linkscope::phase("plugin_sdk.resource.new");
        let path = path.into();
        linkscope::event_fields(
            "plugin_sdk.resource.new",
            [
                linkscope::TraceField::text("plugin_id", plugin_id.as_str().to_owned()),
                linkscope::TraceField::text("kind", format!("{kind:?}")),
                linkscope::TraceField::text("path", path.clone()),
            ],
        );
        Self {
            plugin_id,
            kind,
            path,
            source: None,
            scope: None,
            namespace: None,
        }
    }

    pub fn with_source_info(mut self, source: PluginSource, scope: PluginScope) -> Self {
        self.source = Some(source);
        self.scope = Some(scope);
        self
    }

    pub fn with_namespace(mut self, namespace: impl Into<String>) -> Self {
        self.namespace = Some(namespace.into());
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct CommandDescriptor {
    pub plugin_id: PluginId,
    pub name: String,
    pub description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<PluginSource>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<PluginScope>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub namespace: Option<String>,
}

impl CommandDescriptor {
    pub fn new(
        plugin_id: PluginId,
        name: impl Into<String>,
        description: impl Into<String>,
    ) -> Self {
        let _linkscope_command = linkscope::phase("plugin_sdk.command.new");
        let name = name.into();
        let description = description.into();
        linkscope::event_fields(
            "plugin_sdk.command.new",
            [
                linkscope::TraceField::text("plugin_id", plugin_id.as_str().to_owned()),
                linkscope::TraceField::text("name", name.clone()),
                linkscope::TraceField::bytes(
                    "description_bytes",
                    u64::try_from(description.len()).unwrap_or(u64::MAX),
                ),
            ],
        );
        Self {
            plugin_id,
            name,
            description,
            path: None,
            source: None,
            scope: None,
            namespace: None,
        }
    }

    pub fn with_path(mut self, path: impl Into<String>) -> Self {
        self.path = Some(path.into());
        self
    }

    pub fn with_source_info(mut self, source: PluginSource, scope: PluginScope) -> Self {
        self.source = Some(source);
        self.scope = Some(scope);
        self
    }

    pub fn with_namespace(mut self, namespace: impl Into<String>) -> Self {
        self.namespace = Some(namespace.into());
        self
    }
}

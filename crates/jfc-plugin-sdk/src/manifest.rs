use std::{borrow::Borrow, fmt, ops::Deref};

use serde::{Deserialize, Serialize};

use crate::{
    PluginCapability, PluginScope, PluginSource,
    compat::{CompatibilityErrorDto, CompatibilityReport, CompatibilityStatus},
};

const CURRENT_MANIFEST_SCHEMA_VERSION: u16 = 1;

macro_rules! string_newtype {
    ($name:ident) => {
        #[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            pub fn new(value: impl Into<String>) -> Self {
                let _linkscope_new = linkscope::phase("plugin_sdk.string_newtype.new");
                let value = value.into();
                linkscope::detail_event_fields(
                    "plugin_sdk.string_newtype.new",
                    [
                        linkscope::TraceField::text("type", stringify!($name)),
                        linkscope::TraceField::bytes(
                            "value_bytes",
                            u64::try_from(value.len()).unwrap_or(u64::MAX),
                        ),
                    ],
                );
                Self(value)
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }

            pub fn into_inner(self) -> String {
                self.0
            }
        }

        impl AsRef<str> for $name {
            fn as_ref(&self) -> &str {
                self.as_str()
            }
        }

        impl Borrow<str> for $name {
            fn borrow(&self) -> &str {
                self.as_str()
            }
        }

        impl Deref for $name {
            type Target = str;

            fn deref(&self) -> &Self::Target {
                self.as_str()
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(self.as_str())
            }
        }

        impl From<String> for $name {
            fn from(value: String) -> Self {
                Self(value)
            }
        }

        impl From<&str> for $name {
            fn from(value: &str) -> Self {
                Self(value.to_owned())
            }
        }
    };
}

string_newtype!(PluginId);
string_newtype!(PluginVersion);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct PluginManifest {
    pub schema_version: u16,
    pub id: PluginId,
    pub version: PluginVersion,
    pub source: PluginSource,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub scopes: Vec<PluginScope>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub capabilities: Vec<PluginCapability>,
}

impl PluginManifest {
    pub fn new(id: PluginId, version: PluginVersion, source: PluginSource) -> Self {
        let _linkscope_manifest = linkscope::phase("plugin_sdk.manifest.new");
        linkscope::event_fields(
            "plugin_sdk.manifest.new",
            [
                linkscope::TraceField::text("plugin_id", id.as_str().to_owned()),
                linkscope::TraceField::text("version", version.as_str().to_owned()),
            ],
        );
        Self {
            schema_version: CURRENT_MANIFEST_SCHEMA_VERSION,
            id,
            version,
            source,
            display_name: None,
            description: None,
            scopes: Vec::new(),
            capabilities: Vec::new(),
        }
    }

    pub fn id(&self) -> &PluginId {
        &self.id
    }

    pub fn version(&self) -> &PluginVersion {
        &self.version
    }

    pub fn with_schema_version(mut self, schema_version: u16) -> Self {
        self.schema_version = schema_version;
        self
    }

    pub fn with_display_name(mut self, display_name: impl Into<String>) -> Self {
        self.display_name = Some(display_name.into());
        self
    }

    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    pub fn with_scope(mut self, scope: PluginScope) -> Self {
        self.scopes.push(scope);
        self
    }

    pub fn with_capability(mut self, capability: PluginCapability) -> Self {
        self.capabilities.push(capability);
        self
    }

    pub fn compatibility_status(&self, supported_schema_version: u16) -> CompatibilityStatus {
        self.compatibility_report(supported_schema_version).status
    }

    pub fn compatibility_report(&self, supported_schema_version: u16) -> CompatibilityReport {
        if self.schema_version <= supported_schema_version {
            return CompatibilityReport::compatible(self.id.clone());
        }

        CompatibilityReport::incompatible(
            self.id.clone(),
            CompatibilityErrorDto::unsupported_manifest_schema(
                self.id.clone(),
                self.schema_version,
                supported_schema_version,
            ),
        )
    }
}

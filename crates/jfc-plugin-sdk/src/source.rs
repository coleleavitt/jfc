use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PluginSource {
    BuiltIn {
        crate_name: String,
    },
    Workspace {
        root: String,
    },
    Project {
        root: String,
    },
    User {
        root: String,
    },
    Package {
        registry: String,
        package: String,
        checksum: String,
    },
    ProcessBridge {
        command: String,
    },
}

impl PluginSource {
    pub fn built_in(crate_name: impl Into<String>) -> Self {
        Self::BuiltIn {
            crate_name: crate_name.into(),
        }
    }

    pub fn package(
        registry: impl Into<String>,
        package: impl Into<String>,
        checksum: impl Into<String>,
    ) -> Self {
        Self::Package {
            registry: registry.into(),
            package: package.into(),
            checksum: checksum.into(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PluginScope {
    Global,
    User,
    Project,
    Workspace,
    Session,
}

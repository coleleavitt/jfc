use std::collections::BTreeSet;

use jfc_plugin_sdk::{
    PluginCapability, PluginId, PluginManifest, PluginScope, PluginSource, PluginVersion,
};

use crate::{PluginHost, PluginHostError, PluginRegistration};

const KNOWLEDGE_PLUGIN_VERSION: &str = "0.1.0";

const KNOWLEDGE_CAPABILITIES: &[BuiltinKnowledgeCapability] = &[
    BuiltinKnowledgeCapability {
        crate_name: "jfc-web",
        plugin_id: "builtin.jfc-web",
        display_name: "JFC Web",
        description: "Built-in web search, fetch, and research data capability descriptors",
        optional_stale_reference: false,
    },
    BuiltinKnowledgeCapability {
        crate_name: "jfc-memory",
        plugin_id: "builtin.jfc-memory",
        display_name: "JFC Memory",
        description: "Built-in persistent memory and recall capability descriptors",
        optional_stale_reference: false,
    },
    BuiltinKnowledgeCapability {
        crate_name: "jfc-learn",
        plugin_id: "builtin.jfc-learn",
        display_name: "JFC Learn",
        description: "Built-in learning, dreaming, and memory verification capability descriptors",
        optional_stale_reference: false,
    },
    BuiltinKnowledgeCapability {
        crate_name: "jfc-compress",
        plugin_id: "builtin.jfc-compress",
        display_name: "JFC Compress",
        description: "Built-in context compression and summary data capability descriptors",
        optional_stale_reference: false,
    },
    BuiltinKnowledgeCapability {
        crate_name: "jfc-graph",
        plugin_id: "builtin.jfc-graph",
        display_name: "JFC Graph",
        description: "Built-in code graph and knowledge traversal capability descriptors",
        optional_stale_reference: true,
    },
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuiltinKnowledgeRegistrationReport {
    pub registered_crates: Vec<&'static str>,
    pub missing_optional_crates: Vec<&'static str>,
}

pub fn builtin_knowledge_plugin_host<I, S>(
    workspace_members: I,
) -> Result<(PluginHost, BuiltinKnowledgeRegistrationReport), PluginHostError>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let mut host = PluginHost::new();
    let report = register_builtin_knowledge_plugins(&mut host, workspace_members)?;
    host.activate_all()?;
    Ok((host, report))
}

pub fn register_builtin_knowledge_plugins<I, S>(
    host: &mut PluginHost,
    workspace_members: I,
) -> Result<BuiltinKnowledgeRegistrationReport, PluginHostError>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let workspace_members = workspace_members
        .into_iter()
        .map(|member| member.as_ref().to_owned())
        .collect::<BTreeSet<_>>();
    let mut registered_crates = Vec::new();
    let mut missing_optional_crates = Vec::new();

    for capability in KNOWLEDGE_CAPABILITIES {
        if workspace_members.contains(capability.crate_name) {
            host.register_internal(capability.plugin_registration())?;
            registered_crates.push(capability.crate_name);
        } else if capability.optional_stale_reference {
            missing_optional_crates.push(capability.crate_name);
        }
    }

    Ok(BuiltinKnowledgeRegistrationReport {
        registered_crates,
        missing_optional_crates,
    })
}

struct BuiltinKnowledgeCapability {
    crate_name: &'static str,
    plugin_id: &'static str,
    display_name: &'static str,
    description: &'static str,
    optional_stale_reference: bool,
}

impl BuiltinKnowledgeCapability {
    fn plugin_registration(&self) -> PluginRegistration {
        let manifest = PluginManifest::new(
            PluginId::new(self.plugin_id),
            PluginVersion::new(KNOWLEDGE_PLUGIN_VERSION),
            PluginSource::built_in(self.crate_name),
        )
        .with_display_name(self.display_name)
        .with_description(self.description)
        .with_scope(PluginScope::Workspace)
        .with_capability(PluginCapability::Resources);

        PluginRegistration::new(manifest)
    }
}

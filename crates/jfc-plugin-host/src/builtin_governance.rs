use std::collections::BTreeSet;

use jfc_plugin_sdk::{
    PluginCapability, PluginId, PluginManifest, PluginScope, PluginSource, PluginVersion,
};

use crate::{PluginHost, PluginHostError, PluginRegistration};

const GOVERNANCE_PLUGIN_VERSION: &str = "0.1.0";

const GOVERNANCE_CAPABILITIES: &[BuiltinGovernanceCapability] = &[
    BuiltinGovernanceCapability {
        crate_name: "jfc-economy",
        plugin_id: "builtin.jfc-economy",
        display_name: "JFC Economy",
        description: "Built-in token economy, budget, bounty, and settlement governance descriptors",
        capability: PluginCapability::Governance,
    },
    BuiltinGovernanceCapability {
        crate_name: "jfc-audit",
        plugin_id: "builtin.jfc-audit",
        display_name: "JFC Audit",
        description: "Built-in audit, safety analysis, and vulnerability review descriptors",
        capability: PluginCapability::Audit,
    },
    BuiltinGovernanceCapability {
        crate_name: "jfc-daemon",
        plugin_id: "builtin.jfc-daemon",
        display_name: "JFC Daemon",
        description: "Built-in background daemon, cron, wakeup, and detached worker descriptors",
        capability: PluginCapability::Background,
    },
    BuiltinGovernanceCapability {
        crate_name: "jfc-remote",
        plugin_id: "builtin.jfc-remote",
        display_name: "JFC Remote",
        description: "Built-in remote-control protocol, authentication, and transport descriptors",
        capability: PluginCapability::Remote,
    },
];

pub fn builtin_governance_plugin_host<I, S>(
    workspace_members: I,
) -> Result<PluginHost, PluginHostError>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let mut host = PluginHost::new();
    register_builtin_governance_plugins(&mut host, workspace_members)?;
    host.activate_all()?;
    Ok(host)
}

pub fn register_builtin_governance_plugins<I, S>(
    host: &mut PluginHost,
    workspace_members: I,
) -> Result<(), PluginHostError>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let workspace_members = workspace_members
        .into_iter()
        .map(|member| member.as_ref().to_owned())
        .collect::<BTreeSet<_>>();

    for capability in GOVERNANCE_CAPABILITIES {
        if workspace_members.contains(capability.crate_name) {
            host.register_internal(capability.plugin_registration())?;
        }
    }

    Ok(())
}

struct BuiltinGovernanceCapability {
    crate_name: &'static str,
    plugin_id: &'static str,
    display_name: &'static str,
    description: &'static str,
    capability: PluginCapability,
}

impl BuiltinGovernanceCapability {
    fn plugin_registration(&self) -> PluginRegistration {
        let manifest = PluginManifest::new(
            PluginId::new(self.plugin_id),
            PluginVersion::new(GOVERNANCE_PLUGIN_VERSION),
            PluginSource::built_in(self.crate_name),
        )
        .with_display_name(self.display_name)
        .with_description(self.description)
        .with_scope(PluginScope::Workspace)
        .with_capability(self.capability.clone());

        PluginRegistration::new(manifest)
    }
}

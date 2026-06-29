use jfc_plugin_host::{
    BUILTIN_AGENT_LAUNCH_HANDLER, BUILTIN_AGENT_LAUNCH_ID, BUILTIN_AGENT_RESOURCE_PATH,
    BUILTIN_AGENTS_PLUGIN_ID, BUILTIN_BACKGROUND_AGENT_LAUNCH_HANDLER,
    BUILTIN_BACKGROUND_AGENT_LAUNCH_ID, BUILTIN_SKILL_RESOURCE_PATH,
    BUILTIN_WORKFLOW_RESOURCE_PATH, BUILTIN_WORKFLOWS_PLUGIN_ID, PluginStatusKind,
    builtin_agent_workflow_plugin_host,
};
use jfc_plugin_sdk::{
    AgentLaunchExecutorKind, PluginCapability, PluginId, PluginScope, PluginSource, ResourceKind,
};

#[test]
fn builtin_agent_workflow_resources_are_host_descriptors_normal() {
    // Given: the first-party agent/workflow pack activated through the plugin host.
    let host = builtin_agent_workflow_plugin_host().expect("agent/workflow plugins activate");

    // When: resource descriptors are read through the same host surface as external plugins.
    let resources = host.resource_descriptors();

    // Then: built-in agents, skills, and workflows are visible as host-owned resources.
    assert_eq!(resources.len(), 3);
    assert_builtin_resource(
        &resources,
        ExpectedResource::new(
            BUILTIN_AGENTS_PLUGIN_ID,
            ResourceKind::Skill,
            BUILTIN_SKILL_RESOURCE_PATH,
            "jfc-agents",
        ),
    );
    assert_builtin_resource(
        &resources,
        ExpectedResource::new(
            BUILTIN_AGENTS_PLUGIN_ID,
            ResourceKind::Agent,
            BUILTIN_AGENT_RESOURCE_PATH,
            "jfc-agents",
        ),
    );
    assert_builtin_resource(
        &resources,
        ExpectedResource::new(
            BUILTIN_WORKFLOWS_PLUGIN_ID,
            ResourceKind::Workflow,
            BUILTIN_WORKFLOW_RESOURCE_PATH,
            "jfc-engine",
        ),
    );
}

#[test]
fn builtin_agents_expose_launch_descriptor_normal() {
    // Given: the first-party agent pack activated through the plugin host.
    let host = builtin_agent_workflow_plugin_host().expect("agent/workflow plugins activate");

    // When: launch descriptors are read through the same host surface as external plugins.
    let launchers = host.agent_launch_descriptors();
    let diagnostics = host.diagnostics();

    // Then: built-in agent execution is represented as an explicit host-owned launch contract.
    assert_eq!(launchers.len(), 2);
    assert_builtin_launcher(
        &launchers,
        BUILTIN_AGENT_LAUNCH_ID,
        BUILTIN_AGENT_LAUNCH_HANDLER,
    );
    assert_builtin_launcher(
        &launchers,
        BUILTIN_BACKGROUND_AGENT_LAUNCH_ID,
        BUILTIN_BACKGROUND_AGENT_LAUNCH_HANDLER,
    );
    assert_eq!(diagnostics.counts.agent_launches, 2);
}

#[test]
fn builtin_agent_workflow_status_reports_first_party_resource_plugins_normal() {
    // Given: the first-party agent/workflow pack activated through the plugin host.
    let host = builtin_agent_workflow_plugin_host().expect("agent/workflow plugins activate");

    // When: the host status snapshot is rendered from registered plugins.
    let snapshot = host.status_snapshot();

    // Then: both built-in resource owners are active and declare resource capability.
    assert_eq!(snapshot.plugins.len(), 2);
    for (plugin_id, crate_name, expects_agent_launch) in [
        (BUILTIN_AGENTS_PLUGIN_ID, "jfc-agents", true),
        (BUILTIN_WORKFLOWS_PLUGIN_ID, "jfc-engine", false),
    ] {
        let entry = snapshot
            .plugins
            .iter()
            .find(|entry| entry.plugin_id == PluginId::new(plugin_id))
            .unwrap_or_else(|| panic!("missing status entry for {plugin_id}"));
        assert_eq!(entry.status, PluginStatusKind::Active);
        assert_eq!(entry.source, PluginSource::built_in(crate_name));
        assert!(
            entry
                .manifest
                .capabilities
                .iter()
                .any(|capability| matches!(capability, PluginCapability::Resources)),
            "{plugin_id} must advertise resource capability"
        );
        assert_eq!(
            entry
                .manifest
                .capabilities
                .iter()
                .any(|capability| matches!(capability, PluginCapability::AgentLaunches { .. })),
            expects_agent_launch,
            "{plugin_id} launch capability mismatch"
        );
    }
}

#[derive(Clone, Copy)]
struct ExpectedResource<'a> {
    plugin_id: &'a str,
    kind: ResourceKind,
    path: &'a str,
    crate_name: &'a str,
}

impl<'a> ExpectedResource<'a> {
    const fn new(
        plugin_id: &'a str,
        kind: ResourceKind,
        path: &'a str,
        crate_name: &'a str,
    ) -> Self {
        Self {
            plugin_id,
            kind,
            path,
            crate_name,
        }
    }
}

fn assert_builtin_resource(
    resources: &[jfc_plugin_sdk::ResourceDescriptor],
    expected: ExpectedResource<'_>,
) {
    let descriptor = resources
        .iter()
        .find(|descriptor| {
            descriptor.plugin_id == PluginId::new(expected.plugin_id)
                && descriptor.kind == expected.kind
        })
        .unwrap_or_else(|| {
            panic!(
                "missing {:?} descriptor for {}",
                expected.kind, expected.plugin_id
            )
        });
    assert_eq!(descriptor.path, expected.path);
    assert_eq!(descriptor.namespace.as_deref(), Some("builtin"));
    assert_eq!(descriptor.scope, Some(PluginScope::Workspace));
    assert_eq!(
        descriptor.source,
        Some(PluginSource::built_in(expected.crate_name))
    );
}

fn assert_builtin_launcher(
    launchers: &[jfc_plugin_sdk::AgentLaunchDescriptor],
    name: &str,
    handler: &str,
) {
    let launcher = launchers
        .iter()
        .find(|launcher| launcher.name == name)
        .unwrap_or_else(|| panic!("missing launcher {name}"));
    assert_eq!(launcher.plugin_id.as_str(), BUILTIN_AGENTS_PLUGIN_ID);
    assert_eq!(launcher.executor.kind, AgentLaunchExecutorKind::BuiltIn);
    assert_eq!(launcher.executor.handler, handler);
}

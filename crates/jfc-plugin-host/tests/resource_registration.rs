use jfc_plugin_host::{
    PluginDiscoveryOptions, PluginDiscoverySearchRoot, cached_discovered_resource_plugin_state,
    clear_discovered_plugin_state_cache_for_tests, discovered_resource_plugin_host,
    reload_cached_discovered_resource_plugin_state,
};
use jfc_plugin_sdk::{
    AgentLaunchExecutorKind, ExtensionSlot, MetricSurface, PluginCapability, PluginScope,
    PluginSource, ProviderExecutorKind, ResourceKind, RuntimeActionKind,
    RuntimeExtensionExecutorKind, RuntimeExtensionRefreshKind, RuntimeExtensionTarget,
    UiMutationScope, UiPanelRefreshKind, UiSlotActionDescriptor, UiWidgetKind, UiWidgetRefreshKind,
};

#[test]
fn extension_plugin_contributes_resources_and_commands_with_source_info() {
    // Given: an extension plugin root with agent, skill, workflow, and command surfaces.
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let plugins = tmp.path().join("plugins");
    let plugin = plugins.join("sec");
    create_extension_plugin(&plugin, "sec-plugin", "flows");

    // When: the host discovers the project plugin and activates descriptor registrations.
    let host = discovered_resource_plugin_host(
        PluginDiscoveryOptions::new()
            .with_search_root(PluginDiscoverySearchRoot::project_plugins_dir(&plugins)),
    )
    .expect("resource plugin activates");

    // Then: every resource descriptor carries the host-owned source info and namespace.
    let resources = host.resource_descriptors();
    assert_eq!(resources.len(), 3);
    for kind in [
        ResourceKind::Skill,
        ResourceKind::Agent,
        ResourceKind::Workflow,
    ] {
        let descriptor = resources
            .iter()
            .find(|descriptor| descriptor.kind == kind)
            .unwrap_or_else(|| panic!("missing {kind:?} descriptor"));
        assert_eq!(descriptor.plugin_id.as_str(), "sec-plugin");
        assert_eq!(descriptor.namespace.as_deref(), Some("sec"));
        assert_eq!(descriptor.scope, Some(PluginScope::Project));
        assert!(matches!(
            descriptor.source,
            Some(PluginSource::Project { .. })
        ));
    }
    assert!(
        resources
            .iter()
            .any(|descriptor| descriptor.path.ends_with("sec/flows"))
    );

    let commands = host.command_descriptors();
    assert_eq!(commands.len(), 1);
    assert_eq!(commands[0].plugin_id.as_str(), "sec-plugin");
    assert_eq!(commands[0].namespace.as_deref(), Some("sec"));
    assert_eq!(commands[0].scope, Some(PluginScope::Project));
    assert!(matches!(
        commands[0].source,
        Some(PluginSource::Project { .. })
    ));
    assert!(
        commands[0]
            .path
            .as_deref()
            .is_some_and(|path| path.ends_with("sec/commands"))
    );
}

#[test]
fn disabled_plugin_namespace_hides_contributed_resources_and_commands() {
    // Given: one enabled plugin and one disabled plugin namespace.
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let plugins = tmp.path().join("plugins");
    create_extension_plugin(&plugins.join("enabled"), "enabled-plugin", "workflows");
    create_extension_plugin(&plugins.join("disabled"), "disabled-plugin", "workflows");

    // When: discovery receives the Claude-compatible disabled namespace.
    let host = discovered_resource_plugin_host(
        PluginDiscoveryOptions::new()
            .with_search_root(PluginDiscoverySearchRoot::project_plugins_dir(&plugins))
            .with_disabled_plugin("disabled@local"),
    )
    .expect("resource plugin activates");

    // Then: disabled resources and commands never reach active descriptors.
    let resources = host.resource_descriptors();
    assert!(resources.iter().all(|descriptor| {
        descriptor.plugin_id.as_str() == "enabled-plugin"
            && descriptor.namespace.as_deref() == Some("enabled")
    }));
    let commands = host.command_descriptors();
    assert_eq!(commands.len(), 1);
    assert_eq!(commands[0].plugin_id.as_str(), "enabled-plugin");
    assert_eq!(commands[0].namespace.as_deref(), Some("enabled"));
}

#[test]
fn extension_plugin_contributes_provider_descriptors_normal() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let plugins = tmp.path().join("plugins");
    let plugin = plugins.join("provider-plugin");
    create_extension_plugin(&plugin, "provider-plugin", "workflows");
    std::fs::write(
        plugin.join(".jfc-plugin.toml"),
        r#"[plugin]
name = "provider-plugin"
workflows_dir = "workflows"

[[providers]]
provider = "local-ai"
visibility = "host_visible"
models = [{ id = "local-chat", display_name = "Local Chat", context_window_tokens = 32000, max_output_tokens = 4096 }]

[providers.executor]
kind = "process_bridge"
handler = "provider.sh"
"#,
    )
    .expect("write provider manifest");

    let host = discovered_resource_plugin_host(
        PluginDiscoveryOptions::new()
            .with_search_root(PluginDiscoverySearchRoot::project_plugins_dir(&plugins)),
    )
    .expect("resource plugin activates");
    let providers = host.provider_descriptors();
    let snapshot = host.status_snapshot();

    assert_eq!(providers.len(), 1);
    assert_eq!(providers[0].plugin_id.as_str(), "provider-plugin");
    assert_eq!(providers[0].provider, "local-ai");
    assert_eq!(
        providers[0].executor.kind,
        ProviderExecutorKind::ProcessBridge
    );
    assert!(providers[0].executor.handler.ends_with("provider.sh"));
    assert_eq!(providers[0].models[0].id, "local-chat");
    assert_eq!(providers[0].models[0].display_name, "Local Chat");
    assert_eq!(providers[0].models[0].context_window_tokens, Some(32_000));
    assert!(
        snapshot.plugins[0]
            .manifest
            .capabilities
            .contains(&PluginCapability::Providers)
    );
}

#[test]
fn extension_plugin_contributes_command_palette_action_slot_normal() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let plugins = tmp.path().join("plugins");
    let plugin = plugins.join("palette-plugin");
    create_extension_plugin(&plugin, "palette-plugin", "workflows");
    std::fs::write(
        plugin.join(".jfc-plugin.toml"),
        r#"[plugin]
name = "palette-plugin"
workflows_dir = "workflows"

[[ui_slots]]
slot = "command_palette"
id = "palette.open_report"
label = "Open Plugin Report"
priority = 42

[ui_slots.action]
kind = "slash_command"
command = "/plugin-report"
"#,
    )
    .expect("write ui slot manifest");

    let host = discovered_resource_plugin_host(
        PluginDiscoveryOptions::new()
            .with_search_root(PluginDiscoverySearchRoot::project_plugins_dir(&plugins)),
    )
    .expect("resource plugin activates");
    let slots = host.ui_slot_descriptors();
    let snapshot = host.status_snapshot();

    assert_eq!(slots.len(), 1);
    assert_eq!(slots[0].plugin_id.as_str(), "palette-plugin");
    assert_eq!(slots[0].slot, ExtensionSlot::CommandPalette);
    assert_eq!(slots[0].label, "Open Plugin Report");
    assert_eq!(slots[0].priority, 42);
    assert_eq!(
        slots[0].action,
        Some(UiSlotActionDescriptor::SlashCommand {
            command: "/plugin-report".to_owned(),
        })
    );
    assert!(
        snapshot.plugins[0]
            .manifest
            .capabilities
            .contains(&PluginCapability::UiSlots {
                slots: vec![ExtensionSlot::CommandPalette],
            })
    );
}

#[test]
fn extension_plugin_contributes_metric_descriptors_normal() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let plugins = tmp.path().join("plugins");
    let plugin = plugins.join("metrics-plugin");
    create_extension_plugin(&plugin, "metrics-plugin", "workflows");
    std::fs::write(
        plugin.join(".jfc-plugin.toml"),
        r#"[plugin]
name = "metrics-plugin"
workflows_dir = "workflows"

[[metrics]]
id = "cache.hit_rate"
label = "Cache hit rate"
description = "Cache hit rate from a plugin"
unit = "percent"
surfaces = ["status_line", "sidebar"]
priority = 84
"#,
    )
    .expect("write metric manifest");

    let host = discovered_resource_plugin_host(
        PluginDiscoveryOptions::new()
            .with_search_root(PluginDiscoverySearchRoot::project_plugins_dir(&plugins)),
    )
    .expect("resource plugin activates");
    let metrics = host.metric_descriptors();
    let diagnostics = host.diagnostics();
    let snapshot = host.status_snapshot();

    assert_eq!(metrics.len(), 1);
    assert_eq!(metrics[0].plugin_id.as_str(), "metrics-plugin");
    assert_eq!(metrics[0].id, "cache.hit_rate");
    assert_eq!(
        metrics[0].surfaces,
        vec![MetricSurface::StatusLine, MetricSurface::Sidebar]
    );
    assert_eq!(diagnostics.counts.metrics, 1);
    assert!(
        snapshot.plugins[0]
            .manifest
            .capabilities
            .contains(&PluginCapability::Metrics {
                surfaces: vec![MetricSurface::StatusLine, MetricSurface::Sidebar],
            })
    );
}

#[test]
fn extension_plugin_contributes_runtime_action_descriptors_normal() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let plugins = tmp.path().join("plugins");
    let plugin = plugins.join("actions-plugin");
    create_extension_plugin(&plugin, "actions-plugin", "workflows");
    std::fs::write(
        plugin.join(".jfc-plugin.toml"),
        r#"[plugin]
name = "actions-plugin"
workflows_dir = "workflows"

[[runtime_actions]]
id = "plugin.smoke"
label = "Smoke Plugin"
description = "Run process-bridge smoke checks"
kind = "plugin_smoke"
priority = 55
payload = { plugin = "actions-plugin" }
"#,
    )
    .expect("write runtime action manifest");

    let host = discovered_resource_plugin_host(
        PluginDiscoveryOptions::new()
            .with_search_root(PluginDiscoverySearchRoot::project_plugins_dir(&plugins)),
    )
    .expect("resource plugin activates");
    let actions = host.runtime_action_descriptors();
    let diagnostics = host.diagnostics();
    let snapshot = host.status_snapshot();

    assert_eq!(actions.len(), 1);
    assert_eq!(actions[0].plugin_id.as_str(), "actions-plugin");
    assert_eq!(actions[0].id, "plugin.smoke");
    assert_eq!(actions[0].kind, RuntimeActionKind::PluginSmoke);
    assert_eq!(actions[0].priority, 55);
    assert_eq!(diagnostics.counts.runtime_actions, 1);
    assert!(snapshot.plugins[0].manifest.capabilities.contains(
        &PluginCapability::RuntimeActions {
            actions: vec![RuntimeActionKind::PluginSmoke],
        }
    ));
}

#[test]
fn extension_plugin_contributes_ui_widget_descriptors_normal() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let plugins = tmp.path().join("plugins");
    let plugin = plugins.join("widgets-plugin");
    create_extension_plugin(&plugin, "widgets-plugin", "workflows");
    std::fs::write(
        plugin.join(".jfc-plugin.toml"),
        r#"process_bridge = { command = "node", args = ["bridge.js"] }

[plugin]
name = "widgets-plugin"
workflows_dir = "workflows"

[[ui_widgets]]
scope = "info_sidebar"
id = "review.queue"
label = "Review Queue"
kind = "text"
body = "3 open reviews"
runtime_action_id = "reviews.refresh"
refresh = { kind = "process_bridge", handler = "", min_interval_ms = 5000, auto_refresh_ms = 60000 }
priority = 64
"#,
    )
    .expect("write ui widget manifest");

    let host = discovered_resource_plugin_host(
        PluginDiscoveryOptions::new()
            .with_search_root(PluginDiscoverySearchRoot::project_plugins_dir(&plugins)),
    )
    .expect("resource plugin activates");
    let widgets = host.ui_widget_descriptors();
    let diagnostics = host.diagnostics();
    let snapshot = host.status_snapshot();

    assert_eq!(widgets.len(), 1);
    assert_eq!(widgets[0].plugin_id.as_str(), "widgets-plugin");
    assert_eq!(widgets[0].scope, UiMutationScope::InfoSidebar);
    assert_eq!(widgets[0].id, "review.queue");
    assert_eq!(widgets[0].kind, UiWidgetKind::Text);
    assert_eq!(widgets[0].body.as_deref(), Some("3 open reviews"));
    assert_eq!(
        widgets[0].runtime_action_id.as_deref(),
        Some("reviews.refresh")
    );
    let refresh = widgets[0].refresh.as_ref().expect("refresh descriptor");
    assert_eq!(refresh.kind, UiWidgetRefreshKind::ProcessBridge);
    assert!(refresh.handler.contains("node"));
    assert_eq!(refresh.min_interval_ms, Some(5_000));
    assert_eq!(refresh.auto_refresh_ms, Some(60_000));
    assert_eq!(widgets[0].priority, 64);
    assert_eq!(diagnostics.counts.ui_widgets, 1);
    assert!(
        snapshot.plugins[0]
            .manifest
            .capabilities
            .contains(&PluginCapability::UiWidgets {
                scopes: vec![UiMutationScope::InfoSidebar],
            })
    );
}

#[test]
fn extension_plugin_contributes_ui_panel_descriptors_normal() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let plugins = tmp.path().join("plugins");
    let plugin = plugins.join("panels-plugin");
    create_extension_plugin(&plugin, "panels-plugin", "workflows");
    std::fs::write(
        plugin.join(".jfc-plugin.toml"),
        r#"process_bridge = { command = "node", args = ["bridge.js"] }

[plugin]
name = "panels-plugin"
workflows_dir = "workflows"

[[ui_panels]]
scope = "info_sidebar"
id = "review.summary"
title = "Review Summary"
body = "3 open reviews\n1 blocking approval"
runtime_action_id = "reviews.open"
refresh = { kind = "process_bridge", handler = "", min_interval_ms = 5000, auto_refresh_ms = 60000 }
priority = 72
"#,
    )
    .expect("write ui panel manifest");

    let host = discovered_resource_plugin_host(
        PluginDiscoveryOptions::new()
            .with_search_root(PluginDiscoverySearchRoot::project_plugins_dir(&plugins)),
    )
    .expect("resource plugin activates");
    let panels = host.ui_panel_descriptors();
    let diagnostics = host.diagnostics();
    let snapshot = host.status_snapshot();

    assert_eq!(panels.len(), 1);
    assert_eq!(panels[0].plugin_id.as_str(), "panels-plugin");
    assert_eq!(panels[0].scope, UiMutationScope::InfoSidebar);
    assert_eq!(panels[0].id, "review.summary");
    assert_eq!(panels[0].title, "Review Summary");
    assert_eq!(
        panels[0].body.as_deref(),
        Some("3 open reviews\n1 blocking approval")
    );
    assert_eq!(panels[0].runtime_action_id.as_deref(), Some("reviews.open"));
    let refresh = panels[0].refresh.as_ref().expect("refresh descriptor");
    assert_eq!(refresh.kind, UiPanelRefreshKind::ProcessBridge);
    assert!(refresh.handler.contains("node"));
    assert_eq!(refresh.min_interval_ms, Some(5_000));
    assert_eq!(refresh.auto_refresh_ms, Some(60_000));
    assert_eq!(panels[0].priority, 72);
    assert_eq!(diagnostics.counts.ui_panels, 1);
    assert!(
        snapshot.plugins[0]
            .manifest
            .capabilities
            .contains(&PluginCapability::UiPanels {
                scopes: vec![UiMutationScope::InfoSidebar],
            })
    );
}

#[test]
fn extension_plugin_contributes_runtime_extension_descriptors_normal() {
    // Given: a plugin manifest declares executable prompt-context and renderer contracts.
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let plugins = tmp.path().join("plugins");
    let plugin = plugins.join("runtime-plugin");
    create_extension_plugin(&plugin, "runtime-plugin", "workflows");
    std::fs::write(
        plugin.join(".jfc-plugin.toml"),
        r#"[plugin]
name = "runtime-plugin"
workflows_dir = "workflows"

[[runtime_extensions]]
target = "prompt_context"
id = "context.review-rules"
label = "Review Rules"
priority = 80
refresh = { kind = "process_bridge", min_interval_ms = 1000, auto_refresh_ms = 60000 }

[runtime_extensions.executor]
kind = "process_bridge"
handler = "context.sh"

[[runtime_extensions]]
target = "message_renderer"
id = "renderer.diff"
label = "Diff Renderer"

[runtime_extensions.executor]
kind = "process_bridge"
handler = "renderer.sh"
"#,
    )
    .expect("write runtime extension manifest");

    // When: the host discovers and activates the plugin.
    let host = discovered_resource_plugin_host(
        PluginDiscoveryOptions::new()
            .with_search_root(PluginDiscoverySearchRoot::project_plugins_dir(&plugins)),
    )
    .expect("resource plugin activates");
    let descriptors = host.runtime_extension_descriptors();
    let snapshot = host.status_snapshot();

    // Then: both runtime extension contracts are active descriptors with plugin provenance.
    assert_eq!(descriptors.len(), 2);
    let prompt_context = descriptors
        .iter()
        .find(|descriptor| descriptor.target == RuntimeExtensionTarget::PromptContext)
        .expect("prompt context descriptor");
    assert_eq!(prompt_context.plugin_id.as_str(), "runtime-plugin");
    assert_eq!(prompt_context.id, "context.review-rules");
    assert_eq!(prompt_context.priority, 80);
    assert_eq!(
        prompt_context.executor.kind,
        RuntimeExtensionExecutorKind::ProcessBridge
    );
    assert!(prompt_context.executor.handler.ends_with("context.sh"));
    let refresh = prompt_context.refresh.as_ref().expect("refresh descriptor");
    assert_eq!(refresh.kind, RuntimeExtensionRefreshKind::ProcessBridge);
    assert_eq!(refresh.min_interval_ms, Some(1_000));
    assert_eq!(refresh.auto_refresh_ms, Some(60_000));
    let renderer = descriptors
        .iter()
        .find(|descriptor| descriptor.target == RuntimeExtensionTarget::MessageRenderer)
        .expect("renderer descriptor");
    assert_eq!(
        renderer.executor.kind,
        RuntimeExtensionExecutorKind::ProcessBridge
    );
    assert!(renderer.executor.handler.ends_with("renderer.sh"));
    assert!(snapshot.plugins[0].manifest.capabilities.contains(
        &PluginCapability::RuntimeExtensions {
            targets: vec![
                RuntimeExtensionTarget::PromptContext,
                RuntimeExtensionTarget::MessageRenderer,
            ],
        }
    ));
}

#[test]
fn extension_plugin_contributes_agent_launch_descriptors_normal() {
    // Given: a plugin manifest declares an executable agent launch contract.
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let plugins = tmp.path().join("plugins");
    let plugin = plugins.join("agent-plugin");
    create_extension_plugin(&plugin, "agent-plugin", "workflows");
    std::fs::write(
        plugin.join(".jfc-plugin.toml"),
        r#"[plugin]
name = "agent-plugin"
workflows_dir = "workflows"

[[agent_launches]]
name = "variant-agent"
label = "Variant Agent"
description = "Launches an agent variant from the plugin."

[agent_launches.executor]
kind = "process_bridge"
handler = "agents/variant.sh"
"#,
    )
    .expect("write agent launch manifest");

    // When: the host discovers and activates the plugin.
    let host = discovered_resource_plugin_host(
        PluginDiscoveryOptions::new()
            .with_search_root(PluginDiscoverySearchRoot::project_plugins_dir(&plugins)),
    )
    .expect("resource plugin activates");
    let descriptors = host.agent_launch_descriptors();
    let snapshot = host.status_snapshot();

    // Then: the launch contract is active descriptor data with plugin provenance.
    assert_eq!(descriptors.len(), 1);
    let launcher = &descriptors[0];
    assert_eq!(launcher.plugin_id.as_str(), "agent-plugin");
    assert_eq!(launcher.name, "variant-agent");
    assert_eq!(launcher.label, "Variant Agent");
    assert_eq!(
        launcher.executor.kind,
        AgentLaunchExecutorKind::ProcessBridge
    );
    assert!(launcher.executor.handler.ends_with("agents/variant.sh"));
    assert!(
        snapshot.plugins[0]
            .manifest
            .capabilities
            .contains(&PluginCapability::AgentLaunches {
                executors: vec![AgentLaunchExecutorKind::ProcessBridge],
            })
    );
}

#[test]
fn discovered_resource_reload_reports_descriptor_counts_and_digest_changes_normal() {
    // Given: one project plugin with descriptors, and no previous digest.
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let plugins = tmp.path().join("plugins");
    let plugin = plugins.join("reloadable");
    create_extension_plugin(&plugin, "reloadable-plugin", "workflows");

    // When: the host is built through the reload diagnostics surface.
    let first = jfc_plugin_host::reload_discovered_resource_plugin_host(
        PluginDiscoveryOptions::new()
            .with_search_root(PluginDiscoverySearchRoot::project_plugins_dir(&plugins)),
        None,
    )
    .expect("first reload succeeds");
    let previous = first.report.diagnostics.descriptor_digest.clone();
    std::fs::create_dir_all(plugin.join("flows")).expect("create new workflows");
    std::fs::write(
        plugin.join(".jfc-plugin.toml"),
        "[plugin]\nname = \"reloadable-plugin\"\nworkflows_dir = \"flows\"\n",
    )
    .expect("rewrite manifest");
    let second = jfc_plugin_host::reload_discovered_resource_plugin_host(
        PluginDiscoveryOptions::new()
            .with_search_root(PluginDiscoverySearchRoot::project_plugins_dir(&plugins)),
        Some(&previous),
    )
    .expect("second reload succeeds");

    // Then: diagnostics expose active plugin/descriptor counts and detect the changed digest.
    assert_eq!(first.report.diagnostics.counts.plugins, 1);
    assert_eq!(first.report.diagnostics.counts.resources, 3);
    assert_eq!(first.report.diagnostics.counts.commands, 1);
    assert_eq!(first.report.changed, None);
    assert_eq!(second.report.previous_descriptor_digest, Some(previous));
    assert_eq!(second.report.changed, Some(true));
    assert_ne!(
        first.report.diagnostics.descriptor_digest,
        second.report.diagnostics.descriptor_digest
    );
}

#[test]
fn discovered_resource_state_cache_reuses_host_until_reload_normal() {
    // Given: a project plugin with one provider descriptor.
    clear_discovered_plugin_state_cache_for_tests();
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let plugins = tmp.path().join("plugins");
    let plugin = plugins.join("cache-plugin");
    create_extension_plugin(&plugin, "cache-plugin", "workflows");
    std::fs::write(
        plugin.join(".jfc-plugin.toml"),
        r#"[plugin]
name = "cache-plugin"
workflows_dir = "workflows"

[[providers]]
provider = "cache-ai"
models = [{ id = "cache-chat", display_name = "Cache Chat" }]

[providers.executor]
kind = "process_bridge"
handler = "provider.sh"
"#,
    )
    .expect("write provider manifest");
    let options = PluginDiscoveryOptions::new()
        .with_search_root(PluginDiscoverySearchRoot::project_plugins_dir(&plugins));

    // When: two subsystems ask for the same project plugin state.
    let first = cached_discovered_resource_plugin_state(options.clone()).expect("first state");
    let second = cached_discovered_resource_plugin_state(options.clone()).expect("second state");

    // Then: they share the same activated host snapshot instead of rebuilding.
    assert!(std::sync::Arc::ptr_eq(&first, &second));
    assert_eq!(first.host.provider_descriptors().len(), 1);

    // When: the manifest changes and the state is explicitly reloaded.
    std::fs::write(
        plugin.join(".jfc-plugin.toml"),
        r#"[plugin]
name = "cache-plugin"
workflows_dir = "workflows"

[[providers]]
provider = "cache-ai"
models = [{ id = "cache-chat", display_name = "Cache Chat" }]

[[providers]]
provider = "cache-extra"
models = [{ id = "cache-extra", display_name = "Cache Extra" }]

[providers.executor]
kind = "process_bridge"
handler = "provider.sh"
"#,
    )
    .expect("rewrite provider manifest");
    let reloaded = reload_cached_discovered_resource_plugin_state(
        options,
        Some(&first.report.diagnostics.descriptor_digest),
    )
    .expect("reloaded state");

    // Then: the cache entry is replaced and the reload report captures the digest change.
    assert!(!std::sync::Arc::ptr_eq(&first, &reloaded));
    assert_eq!(reloaded.host.provider_descriptors().len(), 2);
    assert_eq!(reloaded.report.changed, Some(true));
}

fn create_extension_plugin(path: &std::path::Path, name: &str, workflows_dir: &str) {
    std::fs::create_dir_all(path.join("skills/audit")).expect("create skills");
    std::fs::create_dir_all(path.join("agents")).expect("create agents");
    std::fs::create_dir_all(path.join(workflows_dir)).expect("create workflows");
    std::fs::create_dir_all(path.join("commands")).expect("create commands");
    std::fs::write(
        path.join(".jfc-plugin.toml"),
        format!("[plugin]\nname = \"{name}\"\nworkflows_dir = \"{workflows_dir}\"\n"),
    )
    .expect("write manifest");
}

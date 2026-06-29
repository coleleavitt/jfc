use jfc_plugin_host::{
    PluginDiscovery, PluginDiscoveryOptions, PluginDiscoverySearchRoot, PluginRootKind,
};
use jfc_plugin_sdk::{PluginScope, PluginSource};

#[test]
fn source_info_marks_global_project_and_plugin_root() {
    // Given: one plugin under each host-supported root kind.
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let global = tmp.path().join("global");
    let project = tmp.path().join("project");
    let direct = tmp.path().join("direct-plugin");
    create_plugin(&global.join("global-plugin"), "global-id", "workflows");
    create_plugin(&project.join("project-plugin"), "project-id", "flows");
    create_plugin(&direct, "direct-id", "workflows");

    // When: discovery scans global, project, and directly registered plugin roots.
    let roots = PluginDiscovery::discover(
        PluginDiscoveryOptions::new()
            .with_search_root(PluginDiscoverySearchRoot::global_plugins_dir(&global))
            .with_search_root(PluginDiscoverySearchRoot::project_plugins_dir(&project))
            .with_search_root(PluginDiscoverySearchRoot::plugin_root(&direct)),
    );

    // Then: provenance is host-owned and distinguishes every source category.
    assert_eq!(roots.len(), 3);
    let global_root = root(&roots, "global-id");
    assert_eq!(global_root.kind, PluginRootKind::Global);
    assert_eq!(global_root.scope, PluginScope::User);
    assert!(matches!(global_root.source, PluginSource::User { .. }));
    let project_root = root(&roots, "project-id");
    assert_eq!(project_root.kind, PluginRootKind::Project);
    assert_eq!(project_root.scope, PluginScope::Project);
    assert!(matches!(project_root.source, PluginSource::Project { .. }));
    let direct_root = root(&roots, "direct-id");
    assert_eq!(direct_root.kind, PluginRootKind::PluginRoot);
    assert!(matches!(direct_root.source, PluginSource::User { .. }));
}

#[test]
fn namespace_uses_plugin_root_directory_name() {
    // Given: a plugin whose manifest id differs from its containing directory.
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let plugins = tmp.path().join("plugins");
    create_plugin(&plugins.join("package-name"), "manifest-name", "workflows");

    // When: discovery describes the plugin root.
    let roots = PluginDiscovery::discover(
        PluginDiscoveryOptions::new()
            .with_search_root(PluginDiscoverySearchRoot::project_plugins_dir(&plugins)),
    );

    // Then: the namespace preserves legacy skill/agent prefix behavior.
    assert_eq!(roots[0].identity, "manifest-name");
    assert_eq!(roots[0].namespace, "package-name");
}

#[test]
fn disabled_plugins_are_filtered_by_identity_or_source_suffix() {
    // Given: two plugin roots and an enabledPlugins-style disabled entry.
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let plugins = tmp.path().join("plugins");
    create_plugin(
        &plugins.join("enabled-plugin"),
        "enabled-plugin",
        "workflows",
    );
    create_plugin(
        &plugins.join("disabled-plugin"),
        "disabled-plugin",
        "workflows",
    );

    // When: the disabled plugin is named with the Claude-compatible @local suffix.
    let roots = PluginDiscovery::discover(
        PluginDiscoveryOptions::new()
            .with_search_root(PluginDiscoverySearchRoot::project_plugins_dir(&plugins))
            .with_disabled_plugin("disabled-plugin@local"),
    );

    // Then: only enabled plugin roots surface to skills/agents/workflows.
    assert_eq!(roots.len(), 1);
    assert_eq!(roots[0].identity, "enabled-plugin");
}

#[test]
fn duplicate_plugin_identities_are_deduped() {
    // Given: global and project roots both contain the same plugin identity.
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let global = tmp.path().join("global");
    let project = tmp.path().join("project");
    create_plugin(&global.join("first"), "same-plugin", "workflows");
    create_plugin(&project.join("second"), "same-plugin", "workflows");

    // When: discovery sees both roots.
    let roots = PluginDiscovery::discover(
        PluginDiscoveryOptions::new()
            .with_search_root(PluginDiscoverySearchRoot::global_plugins_dir(&global))
            .with_search_root(PluginDiscoverySearchRoot::project_plugins_dir(&project)),
    );

    // Then: the host reports one identity once rather than duplicating it.
    assert_eq!(roots.len(), 1);
    assert_eq!(roots[0].identity, "same-plugin");
}

#[test]
fn workflow_directory_uses_manifest_relative_dir() {
    // Given: a plugin manifest pointing workflows at a non-default directory.
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let plugin = tmp.path().join("plugin");
    create_plugin(&plugin, "workflow-plugin", "flows");

    // When: the host derives workflow discovery info.
    let roots = PluginDiscovery::discover(
        PluginDiscoveryOptions::new()
            .with_search_root(PluginDiscoverySearchRoot::plugin_root(&plugin)),
    );
    let workflow = roots[0].workflow_dir();

    // Then: workflow provenance follows the same plugin identity and source.
    assert_eq!(workflow.path, plugin.join("flows"));
    assert_eq!(workflow.plugin_identity, "workflow-plugin");
    assert_eq!(workflow.namespace, "plugin");
}

#[test]
fn direct_plugin_root_allows_hidden_directory_name() {
    // Given: a directly registered plugin root with a hidden directory name.
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let plugin = tmp.path().join(".hidden-plugin");
    create_plugin(&plugin, "hidden-direct", "workflows");

    // When: discovery receives the plugin root directly instead of scanning a parent.
    let roots = PluginDiscovery::discover(
        PluginDiscoveryOptions::new()
            .with_search_root(PluginDiscoverySearchRoot::plugin_root(&plugin)),
    );

    // Then: direct workflow roots preserve legacy CLI --plugin-dir behavior.
    assert_eq!(roots.len(), 1);
    assert_eq!(roots[0].identity, "hidden-direct");
    assert_eq!(roots[0].namespace, ".hidden-plugin");
}

fn create_plugin(path: &std::path::Path, name: &str, workflows_dir: &str) {
    std::fs::create_dir_all(path.join(workflows_dir)).expect("create plugin workflow dir");
    std::fs::write(
        path.join(".jfc-plugin.toml"),
        format!("[plugin]\nname = \"{name}\"\nworkflows_dir = \"{workflows_dir}\"\n"),
    )
    .expect("write manifest");
}

fn root<'a>(
    roots: &'a [jfc_plugin_host::DiscoveredPluginRoot],
    identity: &str,
) -> &'a jfc_plugin_host::DiscoveredPluginRoot {
    roots
        .iter()
        .find(|root| root.identity == identity)
        .expect("root exists")
}

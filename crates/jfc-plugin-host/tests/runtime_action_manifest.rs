use jfc_plugin_host::{
    PluginDescriptorIssueActionability, PluginDescriptorIssueSeverity, PluginDescriptorRepairAction,
};
use jfc_plugin_host::{
    PluginDiscoveryOptions, PluginDiscoverySearchRoot, discovered_resource_plugin_host,
};
use jfc_plugin_sdk::RuntimeActionKind;

#[test]
fn extension_plugin_skips_invalid_runtime_action_descriptors_robust() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let plugins = tmp.path().join("plugins");
    let plugin = plugins.join("bad-actions-plugin");
    create_extension_plugin(&plugin);
    std::fs::write(
        plugin.join(".jfc-plugin.toml"),
        r#"[plugin]
name = "bad-actions-plugin"
workflows_dir = "workflows"

[[runtime_actions]]
id = "bad.host"
label = "Bad Host"
description = "Missing host action payload"
kind = "host_action"

[[runtime_actions]]
id = "bad.panel"
label = "Bad Panel"
description = "Unknown panel target"
kind = "open_panel"
payload = { panel = "floating_debugger" }

[[runtime_actions]]
id = "good.panel"
label = "Good Panel"
description = "Known panel target"
kind = "open_panel"
payload = { panel = "info_sidebar", execute_panel_action = true }
"#,
    )
    .expect("write runtime action manifest");

    let host = discovered_resource_plugin_host(
        PluginDiscoveryOptions::new()
            .with_search_root(PluginDiscoverySearchRoot::project_plugins_dir(&plugins)),
    )
    .expect("resource plugin activates");
    let actions = host.runtime_action_descriptors();

    assert_eq!(actions.len(), 1);
    assert_eq!(actions[0].id, "good.panel");
    assert_eq!(actions[0].kind, RuntimeActionKind::OpenPanel);
    assert_eq!(host.diagnostics().counts.runtime_actions, 1);
}

#[test]
fn extension_plugin_reports_missing_runtime_action_cross_references_robust() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let plugins = tmp.path().join("plugins");
    let plugin = plugins.join("cross-ref-plugin");
    create_extension_plugin(&plugin);
    std::fs::write(
        plugin.join(".jfc-plugin.toml"),
        r#"[plugin]
name = "cross-ref-plugin"
workflows_dir = "workflows"

[[ui_slots]]
slot = "command_palette"
id = "palette.missing"
label = "Missing Palette Action"

[[ui_panels]]
scope = "info_sidebar"
id = "review.panel"
title = "Review Panel"
runtime_action_id = "review.panel.run"

[[ui_widgets]]
scope = "info_sidebar"
id = "review.widget"
label = "Review Widget"
kind = "action"
runtime_action_id = "review.widget.run"

[[runtime_actions]]
id = "open.missing.panel"
label = "Open Missing Panel"
description = "Focuses a missing panel"
kind = "open_panel"
payload = { panel = "info_sidebar", panel_id = "missing.panel" }

[[runtime_actions]]
id = "open.missing.widget"
label = "Open Missing Widget"
description = "Focuses a missing widget"
kind = "open_panel"
payload = { panel = "info_sidebar", widget_id = "missing.widget" }
"#,
    )
    .expect("write runtime action manifest");

    let host = discovered_resource_plugin_host(
        PluginDiscoveryOptions::new()
            .with_search_root(PluginDiscoverySearchRoot::project_plugins_dir(&plugins)),
    )
    .expect("resource plugin activates");
    let diagnostics = host.diagnostics();
    let rows = diagnostics
        .descriptor_issues
        .iter()
        .map(|issue| {
            format!(
                "{:?}:{} -> {:?}:{}:{}",
                issue.descriptor_kind,
                issue.descriptor_id,
                issue.target_kind,
                issue.target_plugin_id.as_str(),
                issue.target_id.as_str()
            )
        })
        .collect::<Vec<_>>();

    assert_eq!(diagnostics.descriptor_issues.len(), 5);
    assert!(
        diagnostics
            .descriptor_issues
            .iter()
            .all(|issue| issue.severity == PluginDescriptorIssueSeverity::Error)
    );
    assert!(diagnostics.descriptor_issues.iter().any(|issue| {
        issue.actionability == PluginDescriptorIssueActionability::AddRuntimeAction
    }));
    assert!(
        diagnostics
            .descriptor_issues
            .iter()
            .any(|issue| issue.actionability == PluginDescriptorIssueActionability::AddUiPanel)
    );
    assert!(
        diagnostics
            .descriptor_issues
            .iter()
            .any(|issue| issue.actionability == PluginDescriptorIssueActionability::AddUiWidget)
    );
    assert!(
        diagnostics
            .descriptor_issues
            .iter()
            .all(|issue| !issue.repair_hint.is_empty())
    );
    assert!(
        diagnostics
            .descriptor_issues
            .iter()
            .any(|issue| issue.repair_hint.contains("Add runtime action"))
    );
    assert!(
        diagnostics
            .descriptor_issues
            .iter()
            .any(|issue| issue.repair_hint.contains("Add info-sidebar UI panel"))
    );
    assert!(
        diagnostics
            .descriptor_issues
            .iter()
            .any(|issue| issue.repair_hint.contains("Add info-sidebar UI widget"))
    );
    assert!(diagnostics.descriptor_issues.iter().any(|issue| matches!(
        &issue.repair_action,
        PluginDescriptorRepairAction::AddRuntimeAction { action_id, .. }
            if action_id == "palette.missing"
    )));
    assert!(diagnostics.descriptor_issues.iter().any(|issue| matches!(
        &issue.repair_action,
        PluginDescriptorRepairAction::AddUiPanel { panel_id, .. }
            if panel_id == "missing.panel"
    )));
    assert!(diagnostics.descriptor_issues.iter().any(|issue| matches!(
        &issue.repair_action,
        PluginDescriptorRepairAction::AddUiWidget { widget_id, .. }
            if widget_id == "missing.widget"
    )));
    assert!(rows.iter().any(|row| row.contains("palette.missing")));
    assert!(rows.iter().any(|row| row.contains("review.panel.run")));
    assert!(rows.iter().any(|row| row.contains("review.widget.run")));
    assert!(rows.iter().any(|row| row.contains("missing.panel")));
    assert!(rows.iter().any(|row| row.contains("missing.widget")));
}

fn create_extension_plugin(path: &std::path::Path) {
    std::fs::create_dir_all(path.join("workflows")).expect("create workflows");
    std::fs::create_dir_all(path.join("skills")).expect("create skills");
    std::fs::create_dir_all(path.join("agents")).expect("create agents");
}

use jfc_plugin_sdk::{ExtensionSlot, RuntimeExtensionTarget};

use super::plugin_status::*;

#[test]
fn initial_ui_state_keeps_discovered_reload_report_and_ui_slots_normal() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let plugin = tmp.path().join("plugins/demo");
    std::fs::create_dir_all(&plugin).expect("create plugin");
    write_demo_manifest(&plugin, "Demo Action");

    let state = initial_ui_state(tmp.path());

    let report = state.reload_report.expect("reload report");
    assert_eq!(report.changed, None);
    assert_eq!(report.diagnostics.counts.ui_slots, 1);
    assert!(
        state.ui_slots.iter().any(|slot| {
            slot.slot == ExtensionSlot::CommandPalette && slot.label == "Demo Action"
        })
    );
    assert!(
        state
            .metric_descriptors
            .iter()
            .any(|metric| { metric.id == jfc_plugin_host::BUILTIN_CACHE_HIT_METRIC_ID })
    );
}

#[test]
fn refresh_ui_state_reports_descriptor_changes_normal() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let plugin = tmp.path().join("plugins/demo");
    std::fs::create_dir_all(&plugin).expect("create plugin");
    write_demo_manifest(&plugin, "Demo Action");
    let first = initial_ui_state(tmp.path());

    write_demo_manifest(&plugin, "Demo Action Updated");
    let refreshed =
        refresh_ui_state(tmp.path(), first.reload_report.as_ref()).expect("refresh state");

    let report = refreshed.reload_report.expect("reload report");
    assert_eq!(report.changed, Some(true));
    assert!(refreshed.ui_slots.iter().any(|slot| {
        slot.slot == ExtensionSlot::CommandPalette && slot.label == "Demo Action Updated"
    }));
}

#[test]
fn initial_ui_state_keeps_discovered_metric_descriptors_normal() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let plugin = tmp.path().join("plugins/demo");
    std::fs::create_dir_all(&plugin).expect("create plugin");
    std::fs::write(
        plugin.join(".jfc-plugin.toml"),
        "[plugin]\nname = \"demo-plugin\"\n\n[[metrics]]\nid = \"demo.metric\"\nlabel = \"Demo metric\"\ndescription = \"A demo metric\"\nunit = \"count\"\nsurfaces = [\"sidebar\"]\n",
    )
    .expect("write manifest");

    let state = initial_ui_state(tmp.path());

    assert!(state.metric_descriptors.iter().any(|metric| {
        metric.plugin_id.as_str() == "demo-plugin" && metric.id == "demo.metric"
    }));
}

#[test]
fn initial_ui_state_keeps_discovered_runtime_actions_normal() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let plugin = tmp.path().join("plugins/demo");
    std::fs::create_dir_all(&plugin).expect("create plugin");
    std::fs::write(
        plugin.join(".jfc-plugin.toml"),
        "[plugin]\nname = \"demo-plugin\"\n\n[[runtime_actions]]\nid = \"demo.refresh\"\nlabel = \"Refresh Demo Metrics\"\ndescription = \"Refreshes demo metrics\"\nkind = \"refresh_metrics\"\npriority = 17\n",
    )
    .expect("write manifest");

    let state = initial_ui_state(tmp.path());

    assert!(state.runtime_action_descriptors.iter().any(|action| {
        action.plugin_id.as_str() == "demo-plugin"
            && action.id == "demo.refresh"
            && action.priority == 17
    }));
}

#[test]
fn initial_ui_state_keeps_runtime_extension_descriptors_normal() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let plugin = tmp.path().join("plugins/demo");
    std::fs::create_dir_all(&plugin).expect("create plugin");
    std::fs::write(
        plugin.join(".jfc-plugin.toml"),
        "[plugin]\nname = \"demo-plugin\"\n\n[[runtime_extensions]]\ntarget = \"prompt_context\"\nid = \"context.demo\"\nlabel = \"Demo Context\"\npriority = 19\n\n[runtime_extensions.executor]\nkind = \"static_text\"\nhandler = \"Demo prompt context.\"\n",
    )
    .expect("write manifest");

    let state = initial_ui_state(tmp.path());

    assert!(state.runtime_extension_descriptors.iter().any(|extension| {
        extension.plugin_id.as_str() == jfc_plugin_host::BUILTIN_PROMPT_CONTEXT_PLUGIN_ID
            && extension.target == RuntimeExtensionTarget::PromptContext
    }));
    assert!(state.runtime_extension_descriptors.iter().any(|extension| {
        extension.plugin_id.as_str() == "demo-plugin"
            && extension.id == "context.demo"
            && extension.target == RuntimeExtensionTarget::PromptContext
            && extension.priority == 19
    }));
}

#[test]
fn initial_ui_state_keeps_discovered_ui_panel_and_widget_descriptors_normal() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let plugin = tmp.path().join("plugins/demo");
    std::fs::create_dir_all(&plugin).expect("create plugin");
    std::fs::write(
        plugin.join(".jfc-plugin.toml"),
        "[plugin]\nname = \"demo-plugin\"\n\n[[ui_panels]]\nscope = \"info_sidebar\"\nid = \"demo.panel\"\ntitle = \"Demo Panel\"\nbody = \"panel body\"\nruntime_action_id = \"demo.open\"\npriority = 41\n\n[[ui_widgets]]\nscope = \"info_sidebar\"\nid = \"demo.widget\"\nlabel = \"Demo Widget\"\nkind = \"text\"\nbody = \"hello from plugin\"\nruntime_action_id = \"demo.refresh\"\npriority = 31\n",
    )
    .expect("write manifest");

    let state = initial_ui_state(tmp.path());

    assert!(state.ui_widget_descriptors.iter().any(|widget| {
        widget.plugin_id.as_str() == "demo-plugin"
            && widget.id == "demo.widget"
            && widget.priority == 31
    }));
    assert!(state.ui_panel_descriptors.iter().any(|panel| {
        panel.plugin_id.as_str() == "demo-plugin"
            && panel.id == "demo.panel"
            && panel.priority == 41
    }));
}

#[test]
fn plugin_ui_state_preserves_widget_snapshots_across_refresh_state_normal() {
    let mut previous = PluginUiState::default();
    previous.ui_widget_snapshots.insert(
        "demo-plugin\0info_sidebar\0demo.widget".to_owned(),
        crate::app::UiWidgetSnapshot {
            body: Some("fresh body".to_owned()),
            state: Some(serde_json::json!({ "seen": 1 })),
        },
    );
    previous.ui_widget_refresh_status.insert(
        "demo-plugin\0info_sidebar\0demo.widget".to_owned(),
        crate::app::UiWidgetRefreshStatus {
            last_error: Some("bridge unavailable".to_owned()),
            ..crate::app::UiWidgetRefreshStatus::default()
        },
    );
    let mut next = PluginUiState::default();

    next.preserve_ui_widget_snapshots_from(&previous);

    let snapshot = next
        .ui_widget_snapshots
        .get("demo-plugin\0info_sidebar\0demo.widget")
        .expect("snapshot");
    assert_eq!(snapshot.body.as_deref(), Some("fresh body"));
    assert_eq!(snapshot.state, Some(serde_json::json!({ "seen": 1 })));
    let status = next
        .ui_widget_refresh_status
        .get("demo-plugin\0info_sidebar\0demo.widget")
        .expect("refresh status");
    assert_eq!(status.last_error.as_deref(), Some("bridge unavailable"));
}

fn write_demo_manifest(plugin: &std::path::Path, label: &str) {
    std::fs::write(
        plugin.join(".jfc-plugin.toml"),
        format!(
            "[plugin]\nname = \"demo-plugin\"\n\n[[ui_slots]]\nslot = \"command_palette\"\nid = \"demo.action\"\nlabel = \"{label}\"\npriority = 5\n"
        ),
    )
    .expect("write manifest");
}

use std::collections::HashSet;

use jfc_plugin_sdk::{
    ExtensionSlot, PluginId, RuntimeActionDescriptor, RuntimeActionOpenPanelTarget,
    UiPanelDescriptor, UiSlotDescriptor, UiWidgetDescriptor,
};

use crate::descriptor_issue_types::{
    PluginDescriptorIssue, PluginDescriptorIssueActionability, PluginDescriptorIssueKind,
    PluginDescriptorIssueSeverity, PluginDescriptorKind, PluginDescriptorRepairAction,
    PluginDescriptorTargetKind,
};

pub(crate) fn plugin_descriptor_issues(
    ui_slots: &[UiSlotDescriptor],
    ui_panels: &[UiPanelDescriptor],
    ui_widgets: &[UiWidgetDescriptor],
    runtime_actions: &[RuntimeActionDescriptor],
) -> Vec<PluginDescriptorIssue> {
    let action_keys = descriptor_keys(
        runtime_actions
            .iter()
            .map(|action| (&action.plugin_id, action.id.as_str())),
    );
    let panel_keys = descriptor_keys(
        ui_panels
            .iter()
            .map(|panel| (&panel.plugin_id, panel.id.as_str())),
    );
    let widget_keys = descriptor_keys(
        ui_widgets
            .iter()
            .map(|widget| (&widget.plugin_id, widget.id.as_str())),
    );

    let mut issues = Vec::new();
    collect_panel_action_issues(ui_panels, &action_keys, &mut issues);
    collect_widget_action_issues(ui_widgets, &action_keys, &mut issues);
    collect_palette_slot_action_issues(ui_slots, &action_keys, &mut issues);
    collect_open_panel_target_issues(runtime_actions, &panel_keys, &widget_keys, &mut issues);
    issues.sort_by(|left, right| issue_sort_key(left).cmp(&issue_sort_key(right)));
    issues
}

fn descriptor_keys<'a>(
    descriptors: impl Iterator<Item = (&'a PluginId, &'a str)>,
) -> HashSet<(PluginId, String)> {
    descriptors
        .map(|(plugin_id, id)| (plugin_id.clone(), id.to_owned()))
        .collect()
}

fn collect_panel_action_issues(
    panels: &[UiPanelDescriptor],
    action_keys: &HashSet<(PluginId, String)>,
    issues: &mut Vec<PluginDescriptorIssue>,
) {
    for panel in panels {
        let Some(runtime_action_id) = panel.runtime_action_id.as_deref() else {
            continue;
        };
        if action_keys.contains(&(panel.plugin_id.clone(), runtime_action_id.to_owned())) {
            continue;
        }
        issues.push(PluginDescriptorIssue::missing_runtime_action(
            panel.plugin_id.clone(),
            PluginDescriptorKind::UiPanel,
            panel.id.clone(),
            panel.plugin_id.clone(),
            runtime_action_id.to_owned(),
        ));
    }
}

fn collect_widget_action_issues(
    widgets: &[UiWidgetDescriptor],
    action_keys: &HashSet<(PluginId, String)>,
    issues: &mut Vec<PluginDescriptorIssue>,
) {
    for widget in widgets {
        let Some(runtime_action_id) = widget.runtime_action_id.as_deref() else {
            continue;
        };
        if action_keys.contains(&(widget.plugin_id.clone(), runtime_action_id.to_owned())) {
            continue;
        }
        issues.push(PluginDescriptorIssue::missing_runtime_action(
            widget.plugin_id.clone(),
            PluginDescriptorKind::UiWidget,
            widget.id.clone(),
            widget.plugin_id.clone(),
            runtime_action_id.to_owned(),
        ));
    }
}

fn collect_palette_slot_action_issues(
    slots: &[UiSlotDescriptor],
    action_keys: &HashSet<(PluginId, String)>,
    issues: &mut Vec<PluginDescriptorIssue>,
) {
    for slot in slots {
        if slot.slot != ExtensionSlot::CommandPalette || slot.action.is_some() {
            continue;
        }
        if action_keys.contains(&(slot.plugin_id.clone(), slot.id.clone())) {
            continue;
        }
        issues.push(PluginDescriptorIssue::missing_runtime_action(
            slot.plugin_id.clone(),
            PluginDescriptorKind::UiSlot,
            slot.id.clone(),
            slot.plugin_id.clone(),
            slot.id.clone(),
        ));
    }
}

fn collect_open_panel_target_issues(
    actions: &[RuntimeActionDescriptor],
    panel_keys: &HashSet<(PluginId, String)>,
    widget_keys: &HashSet<(PluginId, String)>,
    issues: &mut Vec<PluginDescriptorIssue>,
) {
    for action in actions {
        let Ok(open_panel) = action.open_panel_payload() else {
            continue;
        };
        if open_panel.target != RuntimeActionOpenPanelTarget::InfoSidebar {
            continue;
        }
        if let Some(panel_id) = open_panel.panel_id {
            let target_plugin_id =
                target_plugin_id(action, open_panel.panel_plugin_id, open_panel.plugin_id);
            if !panel_keys.contains(&(target_plugin_id.clone(), panel_id.to_owned())) {
                issues.push(PluginDescriptorIssue::missing_ui_panel(
                    action.plugin_id.clone(),
                    action.id.clone(),
                    target_plugin_id,
                    panel_id.to_owned(),
                ));
            }
        }
        if let Some(widget_id) = open_panel.widget_id {
            let target_plugin_id =
                target_plugin_id(action, open_panel.widget_plugin_id, open_panel.plugin_id);
            if !widget_keys.contains(&(target_plugin_id.clone(), widget_id.to_owned())) {
                issues.push(PluginDescriptorIssue::missing_ui_widget(
                    action.plugin_id.clone(),
                    action.id.clone(),
                    target_plugin_id,
                    widget_id.to_owned(),
                ));
            }
        }
    }
}

fn target_plugin_id(
    action: &RuntimeActionDescriptor,
    specific_plugin_id: Option<&str>,
    fallback_plugin_id: Option<&str>,
) -> PluginId {
    specific_plugin_id
        .or(fallback_plugin_id)
        .map(PluginId::new)
        .unwrap_or_else(|| action.plugin_id.clone())
}

fn issue_sort_key(
    issue: &PluginDescriptorIssue,
) -> (
    &str,
    PluginDescriptorKind,
    &str,
    PluginDescriptorTargetKind,
    &str,
    PluginDescriptorIssueKind,
) {
    (
        issue.plugin_id.as_str(),
        issue.descriptor_kind,
        issue.descriptor_id.as_str(),
        issue.target_kind,
        issue.target_id.as_str(),
        issue.kind,
    )
}

impl PluginDescriptorIssue {
    fn missing_runtime_action(
        plugin_id: PluginId,
        descriptor_kind: PluginDescriptorKind,
        descriptor_id: String,
        target_plugin_id: PluginId,
        target_id: String,
    ) -> Self {
        let repair_hint = missing_runtime_action_repair_hint(
            &plugin_id,
            descriptor_kind,
            &descriptor_id,
            &target_plugin_id,
            &target_id,
        );
        let repair_action = PluginDescriptorRepairAction::AddRuntimeAction {
            plugin_id: target_plugin_id.clone(),
            action_id: target_id.clone(),
        };
        Self {
            kind: PluginDescriptorIssueKind::MissingRuntimeAction,
            severity: PluginDescriptorIssueSeverity::Error,
            actionability: PluginDescriptorIssueActionability::AddRuntimeAction,
            plugin_id,
            descriptor_kind,
            descriptor_id,
            target_plugin_id,
            target_kind: PluginDescriptorTargetKind::RuntimeAction,
            target_id,
            message: "descriptor references a missing runtime action".to_owned(),
            repair_action,
            repair_hint,
        }
    }

    fn missing_ui_panel(
        plugin_id: PluginId,
        descriptor_id: String,
        target_plugin_id: PluginId,
        target_id: String,
    ) -> Self {
        let repair_hint =
            missing_ui_panel_repair_hint(&plugin_id, &descriptor_id, &target_plugin_id, &target_id);
        let repair_action = PluginDescriptorRepairAction::AddUiPanel {
            plugin_id: target_plugin_id.clone(),
            panel_id: target_id.clone(),
        };
        Self {
            kind: PluginDescriptorIssueKind::MissingUiPanel,
            severity: PluginDescriptorIssueSeverity::Error,
            actionability: PluginDescriptorIssueActionability::AddUiPanel,
            plugin_id,
            descriptor_kind: PluginDescriptorKind::RuntimeAction,
            descriptor_id,
            target_plugin_id,
            target_kind: PluginDescriptorTargetKind::UiPanel,
            target_id,
            message: "runtime action targets a missing UI panel".to_owned(),
            repair_action,
            repair_hint,
        }
    }

    fn missing_ui_widget(
        plugin_id: PluginId,
        descriptor_id: String,
        target_plugin_id: PluginId,
        target_id: String,
    ) -> Self {
        let repair_hint = missing_ui_widget_repair_hint(
            &plugin_id,
            &descriptor_id,
            &target_plugin_id,
            &target_id,
        );
        let repair_action = PluginDescriptorRepairAction::AddUiWidget {
            plugin_id: target_plugin_id.clone(),
            widget_id: target_id.clone(),
        };
        Self {
            kind: PluginDescriptorIssueKind::MissingUiWidget,
            severity: PluginDescriptorIssueSeverity::Error,
            actionability: PluginDescriptorIssueActionability::AddUiWidget,
            plugin_id,
            descriptor_kind: PluginDescriptorKind::RuntimeAction,
            descriptor_id,
            target_plugin_id,
            target_kind: PluginDescriptorTargetKind::UiWidget,
            target_id,
            message: "runtime action targets a missing UI widget".to_owned(),
            repair_action,
            repair_hint,
        }
    }
}

fn missing_runtime_action_repair_hint(
    plugin_id: &PluginId,
    descriptor_kind: PluginDescriptorKind,
    descriptor_id: &str,
    target_plugin_id: &PluginId,
    target_id: &str,
) -> String {
    format!(
        "Add runtime action '{target_id}' to plugin '{}', or point {} '{descriptor_id}' in plugin '{}' at an existing runtime action.",
        target_plugin_id.as_str(),
        descriptor_kind_repair_label(descriptor_kind),
        plugin_id.as_str()
    )
}

fn missing_ui_panel_repair_hint(
    plugin_id: &PluginId,
    descriptor_id: &str,
    target_plugin_id: &PluginId,
    target_id: &str,
) -> String {
    format!(
        "Add info-sidebar UI panel '{target_id}' to plugin '{}', or point runtime action '{descriptor_id}' in plugin '{}' at an existing panel target.",
        target_plugin_id.as_str(),
        plugin_id.as_str()
    )
}

fn missing_ui_widget_repair_hint(
    plugin_id: &PluginId,
    descriptor_id: &str,
    target_plugin_id: &PluginId,
    target_id: &str,
) -> String {
    format!(
        "Add info-sidebar UI widget '{target_id}' to plugin '{}', or point runtime action '{descriptor_id}' in plugin '{}' at an existing widget target.",
        target_plugin_id.as_str(),
        plugin_id.as_str()
    )
}

const fn descriptor_kind_repair_label(kind: PluginDescriptorKind) -> &'static str {
    match kind {
        PluginDescriptorKind::RuntimeAction => "runtime action",
        PluginDescriptorKind::UiPanel => "UI panel",
        PluginDescriptorKind::UiSlot => "UI slot",
        PluginDescriptorKind::UiWidget => "UI widget",
    }
}

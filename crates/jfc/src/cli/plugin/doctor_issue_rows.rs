use jfc_plugin_host::{
    PluginDescriptorIssue, PluginDescriptorIssueActionability, PluginDescriptorIssueKind,
    PluginDescriptorIssueSeverity, PluginDescriptorKind, PluginDescriptorTargetKind,
};

pub(super) fn descriptor_issue_rows(issues: &[PluginDescriptorIssue]) -> Vec<String> {
    let mut rows = issues
        .iter()
        .map(|issue| {
            format!(
                "{} {}:{} -> {}:{}:{} [{}] hint: {}",
                issue.plugin_id.as_str(),
                descriptor_kind_label(issue.descriptor_kind),
                issue.descriptor_id.as_str(),
                issue.target_plugin_id.as_str(),
                target_kind_label(issue.target_kind),
                issue.target_id.as_str(),
                descriptor_issue_fields(issue).join("; "),
                issue.repair_hint
            )
        })
        .collect::<Vec<_>>();
    rows.sort();
    rows
}

fn descriptor_issue_fields(issue: &PluginDescriptorIssue) -> Vec<&'static str> {
    vec![
        issue_severity_label(issue.severity),
        issue_actionability_label(issue.actionability),
        issue_kind_label(issue.kind),
    ]
}

const fn issue_kind_label(kind: PluginDescriptorIssueKind) -> &'static str {
    match kind {
        PluginDescriptorIssueKind::MissingRuntimeAction => "missing_runtime_action",
        PluginDescriptorIssueKind::MissingUiPanel => "missing_ui_panel",
        PluginDescriptorIssueKind::MissingUiWidget => "missing_ui_widget",
    }
}

const fn issue_severity_label(severity: PluginDescriptorIssueSeverity) -> &'static str {
    match severity {
        PluginDescriptorIssueSeverity::Error => "error",
        PluginDescriptorIssueSeverity::Warning => "warning",
    }
}

const fn issue_actionability_label(
    actionability: PluginDescriptorIssueActionability,
) -> &'static str {
    match actionability {
        PluginDescriptorIssueActionability::AddRuntimeAction => "add_runtime_action",
        PluginDescriptorIssueActionability::AddUiPanel => "add_ui_panel",
        PluginDescriptorIssueActionability::AddUiWidget => "add_ui_widget",
        PluginDescriptorIssueActionability::FixReference => "fix_reference",
    }
}

const fn descriptor_kind_label(kind: PluginDescriptorKind) -> &'static str {
    match kind {
        PluginDescriptorKind::RuntimeAction => "runtime_action",
        PluginDescriptorKind::UiPanel => "ui_panel",
        PluginDescriptorKind::UiSlot => "ui_slot",
        PluginDescriptorKind::UiWidget => "ui_widget",
    }
}

const fn target_kind_label(kind: PluginDescriptorTargetKind) -> &'static str {
    match kind {
        PluginDescriptorTargetKind::RuntimeAction => "runtime_action",
        PluginDescriptorTargetKind::UiPanel => "ui_panel",
        PluginDescriptorTargetKind::UiWidget => "ui_widget",
    }
}

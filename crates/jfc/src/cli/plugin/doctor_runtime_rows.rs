use jfc_plugin_sdk::{
    RuntimeActionDescriptor, RuntimeActionKind, RuntimeExtensionDescriptor,
    RuntimeExtensionExecutorKind, RuntimeExtensionRefreshKind, RuntimeExtensionTarget,
};

pub(super) fn runtime_action_rows(actions: &[RuntimeActionDescriptor]) -> Vec<String> {
    let mut rows = actions
        .iter()
        .map(|descriptor| {
            format!(
                "{} {} {} [{}; priority={}]",
                descriptor.plugin_id.as_str(),
                descriptor.id,
                descriptor.label,
                runtime_action_kind_label(descriptor.kind),
                descriptor.priority
            )
        })
        .collect::<Vec<_>>();
    rows.sort();
    rows
}

pub(super) fn runtime_extension_rows(extensions: &[RuntimeExtensionDescriptor]) -> Vec<String> {
    let mut rows = extensions
        .iter()
        .map(|descriptor| {
            let mut fields = vec![
                runtime_extension_target_label(descriptor.target).to_owned(),
                format!(
                    "executor={}",
                    runtime_extension_executor_kind_label(descriptor.executor.kind)
                ),
                format!("priority={}", descriptor.priority),
            ];
            fields.extend(runtime_extension_refresh_fields(descriptor));
            format!(
                "{} {} {} [{}]",
                descriptor.plugin_id.as_str(),
                descriptor.id,
                descriptor.label,
                fields.join("; ")
            )
        })
        .collect::<Vec<_>>();
    rows.sort();
    rows
}

fn runtime_extension_refresh_fields(descriptor: &RuntimeExtensionDescriptor) -> Vec<String> {
    let Some(refresh) = descriptor.refresh.as_ref() else {
        return Vec::new();
    };
    let mut fields = vec![format!(
        "refresh={}",
        runtime_extension_refresh_kind_label(refresh.kind)
    )];
    if let Some(min_interval_ms) = refresh.min_interval_ms {
        fields.push(format!("min_interval_ms={min_interval_ms}"));
    }
    if let Some(auto_refresh_ms) = refresh.auto_refresh_ms {
        fields.push(format!("auto_refresh_ms={auto_refresh_ms}"));
    }
    fields
}

const fn runtime_action_kind_label(kind: RuntimeActionKind) -> &'static str {
    match kind {
        RuntimeActionKind::HostAction => "host_action",
        RuntimeActionKind::SlashCommand => "slash_command",
        RuntimeActionKind::RefreshMetrics => "refresh_metrics",
        RuntimeActionKind::OpenPanel => "open_panel",
        RuntimeActionKind::SendTeammateMessage => "send_teammate_message",
        RuntimeActionKind::RefreshPromptContext => "refresh_prompt_context",
        RuntimeActionKind::PluginSmoke => "plugin_smoke",
        RuntimeActionKind::PluginDiagnostics => "plugin_diagnostics",
    }
}

const fn runtime_extension_target_label(target: RuntimeExtensionTarget) -> &'static str {
    match target {
        RuntimeExtensionTarget::MessageRenderer => "message_renderer",
        RuntimeExtensionTarget::PromptContext => "prompt_context",
    }
}

const fn runtime_extension_executor_kind_label(kind: RuntimeExtensionExecutorKind) -> &'static str {
    match kind {
        RuntimeExtensionExecutorKind::BuiltIn => "built_in",
        RuntimeExtensionExecutorKind::StaticText => "static_text",
        RuntimeExtensionExecutorKind::ProcessBridge => "process_bridge",
    }
}

const fn runtime_extension_refresh_kind_label(kind: RuntimeExtensionRefreshKind) -> &'static str {
    match kind {
        RuntimeExtensionRefreshKind::ProcessBridge => "process_bridge",
    }
}

use jfc_plugin_sdk::{
    DescriptorVisibility, ProviderDescriptor, ProviderExecutorKind, ToolApprovalPolicy,
    ToolDescriptor, ToolExecutorKind,
};

pub(super) fn tool_rows(tools: &[ToolDescriptor]) -> Vec<String> {
    let mut rows = tools
        .iter()
        .map(|descriptor| {
            format!(
                "{} {} {} [{}; {}; {}]",
                descriptor.plugin_id.as_str(),
                descriptor.name,
                descriptor.description,
                tool_executor_kind_label(descriptor.executor.kind),
                tool_approval_policy_label(descriptor.approval_policy),
                descriptor_visibility_label(descriptor.visibility)
            )
        })
        .collect::<Vec<_>>();
    rows.sort();
    rows
}

pub(super) fn provider_rows(providers: &[ProviderDescriptor]) -> Vec<String> {
    let mut rows = providers
        .iter()
        .map(|descriptor| {
            format!(
                "{} {} [{}; {}; models={}]",
                descriptor.plugin_id.as_str(),
                descriptor.provider,
                provider_executor_kind_label(descriptor.executor.kind),
                descriptor_visibility_label(descriptor.visibility),
                provider_model_labels(descriptor)
            )
        })
        .collect::<Vec<_>>();
    rows.sort();
    rows
}

fn provider_model_labels(descriptor: &ProviderDescriptor) -> String {
    if descriptor.models.is_empty() {
        return "none".to_owned();
    }
    descriptor
        .models
        .iter()
        .map(|model| model.id.as_str())
        .collect::<Vec<_>>()
        .join(",")
}

const fn tool_executor_kind_label(kind: ToolExecutorKind) -> &'static str {
    match kind {
        ToolExecutorKind::BuiltIn => "built_in",
        ToolExecutorKind::ProcessBridge => "process_bridge",
        ToolExecutorKind::Mcp => "mcp",
    }
}

const fn provider_executor_kind_label(kind: ProviderExecutorKind) -> &'static str {
    match kind {
        ProviderExecutorKind::BuiltIn => "built_in",
        ProviderExecutorKind::ProcessBridge => "process_bridge",
    }
}

const fn tool_approval_policy_label(policy: ToolApprovalPolicy) -> &'static str {
    match policy {
        ToolApprovalPolicy::ReadOnly => "read_only",
        ToolApprovalPolicy::Mutating => "mutating",
        ToolApprovalPolicy::Management => "management",
    }
}

const fn descriptor_visibility_label(visibility: DescriptorVisibility) -> &'static str {
    match visibility {
        DescriptorVisibility::Internal => "internal",
        DescriptorVisibility::HostVisible => "host_visible",
        DescriptorVisibility::ModelVisible => "model_visible",
    }
}

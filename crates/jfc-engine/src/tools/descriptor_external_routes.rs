use jfc_plugin_sdk::ToolDescriptor;

use crate::runtime::{ExecutionResult, ToolErrorCategory};
use crate::types::ToolInput;

pub(crate) async fn execute_mcp_descriptor_route(
    descriptor: &ToolDescriptor,
    input: &ToolInput,
) -> ExecutionResult {
    let target = mcp_target_name(descriptor);
    if let Some(failure) = validate_mcp_input_name(descriptor, input, target) {
        return failure;
    }

    let Some(registry) = super::registry::snapshot_mcp_registry() else {
        return ExecutionResult::structured_failure(
            "MCP registry not initialized — restart jfc with the MCP module enabled.".to_string(),
            ToolErrorCategory::Configuration,
            false,
        );
    };

    let arguments = input.to_value();
    match crate::mcp::dispatch_tool(&registry, target, arguments).await {
        Ok(outcome) if outcome.is_error => {
            ExecutionResult::structured_failure(outcome.text, ToolErrorCategory::Business, false)
        }
        Ok(outcome) => ExecutionResult::success(outcome.text),
        Err(error) => mcp_dispatch_failure("MCP descriptor dispatch failed", error),
    }
}

fn mcp_target_name(descriptor: &ToolDescriptor) -> &str {
    if descriptor.executor.handler.is_empty() {
        descriptor.name.as_str()
    } else {
        descriptor.executor.handler.as_str()
    }
}

fn validate_mcp_input_name(
    descriptor: &ToolDescriptor,
    input: &ToolInput,
    target: &str,
) -> Option<ExecutionResult> {
    let ToolInput::Mcp { name, .. } = input else {
        return None;
    };

    if name == &descriptor.name || name == target {
        return None;
    }

    Some(ExecutionResult::structured_failure(
        format!(
            "MCP descriptor `{}` cannot execute input for `{}`; expected `{}` or `{}`.",
            descriptor.name, name, descriptor.name, target
        ),
        ToolErrorCategory::Validation,
        false,
    ))
}

fn mcp_dispatch_failure(prefix: &str, error: crate::mcp::DispatchError) -> ExecutionResult {
    let (category, retryable) = mcp_dispatch_error_metadata(&error);
    ExecutionResult::structured_failure(format!("{prefix}: {error}"), category, retryable)
}

fn mcp_dispatch_error_metadata(error: &crate::mcp::DispatchError) -> (ToolErrorCategory, bool) {
    match error {
        crate::mcp::DispatchError::NotMcpName => (ToolErrorCategory::Validation, false),
        crate::mcp::DispatchError::UnknownServer(_) => (ToolErrorCategory::Configuration, false),
        crate::mcp::DispatchError::ServerNotConnected(_) => (ToolErrorCategory::Transient, true),
        crate::mcp::DispatchError::ToolBlocked { .. } => (ToolErrorCategory::Permission, false),
        crate::mcp::DispatchError::AmbiguousToolName(_) => (ToolErrorCategory::Validation, false),
        crate::mcp::DispatchError::UnknownTool(_) => (ToolErrorCategory::Validation, false),
        crate::mcp::DispatchError::Request(request_error) => match request_error {
            crate::mcp::RequestError::Disconnected
            | crate::mcp::RequestError::Timeout
            | crate::mcp::RequestError::Transport { .. } => (ToolErrorCategory::Transient, true),
            crate::mcp::RequestError::AuthHeaderRejected => (ToolErrorCategory::Permission, false),
            crate::mcp::RequestError::BadArguments => (ToolErrorCategory::Validation, false),
            crate::mcp::RequestError::Service(_) => (ToolErrorCategory::Business, false),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mcp_descriptor_dispatch_errors_are_categorized_regression() {
        let blocked = mcp_dispatch_failure(
            "MCP descriptor dispatch failed",
            crate::mcp::DispatchError::ToolBlocked {
                server: "fs".to_owned(),
                tool: "write_file".to_owned(),
                reason: "policy",
            },
        );
        assert_eq!(
            blocked.diagnostics[0].error_category,
            Some(ToolErrorCategory::Permission)
        );
        assert_eq!(blocked.diagnostics[0].retryable, Some(false));

        let timeout = mcp_dispatch_failure(
            "MCP descriptor dispatch failed",
            crate::mcp::DispatchError::Request(crate::mcp::RequestError::Timeout),
        );
        assert_eq!(
            timeout.diagnostics[0].error_category,
            Some(ToolErrorCategory::Transient)
        );
        assert_eq!(timeout.diagnostics[0].retryable, Some(true));
    }
}

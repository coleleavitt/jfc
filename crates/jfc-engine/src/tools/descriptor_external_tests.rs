use std::path::{Path, PathBuf};

use jfc_plugin_host::{PluginHost, PluginRegistration};
use jfc_plugin_sdk::{
    DescriptorVisibility, PluginId, PluginManifest, PluginSource, PluginVersion,
    ToolApprovalPolicy, ToolDescriptor, ToolExecutorKind,
};
use serde_json::json;
use serial_test::serial;

use crate::app::{PermissionDecision, PermissionMode};
use crate::runtime::ToolErrorCategory;
use crate::types::{ToolInput, ToolKind};

use super::descriptor_router::{
    DescriptorExecutionContext, READ_TOOL_HANDLER, execute_tool_descriptor,
};
use super::{descriptor_catalog, execute_tool};

fn descriptor_with_executor(kind: ToolExecutorKind, handler: &str) -> ToolDescriptor {
    ToolDescriptor::new(
        PluginId::new("test.plugin"),
        "descriptor_tool",
        "descriptor test tool",
        json!({ "type": "object" }),
    )
    .with_executor(kind, handler)
}

#[tokio::test]
async fn mcp_descriptor_routes_with_structured_configuration_failure_normal() {
    let dir = tempfile::tempdir().expect("temp dir");
    let descriptor =
        descriptor_with_executor(ToolExecutorKind::Mcp, "mcp__descriptor_external__missing");
    let result = execute_tool_descriptor(
        &descriptor,
        &ToolKind::Mcp(descriptor.name.clone()),
        &ToolInput::Mcp {
            name: descriptor.name.clone(),
            arguments: json!({ "value": 1 }),
        },
        DescriptorExecutionContext::new(dir.path(), None, None),
    )
    .await;

    assert!(result.is_error(), "{}", result.output);
    assert_eq!(
        result.diagnostics[0].error_category,
        Some(ToolErrorCategory::Configuration)
    );
    assert_eq!(result.diagnostics[0].retryable, Some(false));
}

#[tokio::test]
async fn mcp_descriptor_rejects_mismatched_mcp_input_name_normal() {
    let dir = tempfile::tempdir().expect("temp dir");
    let descriptor = descriptor_with_executor(ToolExecutorKind::Mcp, "mcp__test__tool");
    let result = execute_tool_descriptor(
        &descriptor,
        &ToolKind::Mcp("other".to_owned()),
        &ToolInput::Mcp {
            name: "other".to_owned(),
            arguments: json!({}),
        },
        DescriptorExecutionContext::new(dir.path(), None, None),
    )
    .await;

    assert!(result.is_error(), "{}", result.output);
    assert_eq!(
        result.diagnostics[0].error_category,
        Some(ToolErrorCategory::Validation)
    );
    assert!(
        result.output.contains("cannot execute input"),
        "{}",
        result.output
    );
}

#[tokio::test]
async fn process_bridge_descriptor_executes_jsonl_tool_call_normal() {
    let dir = tempfile::tempdir().expect("temp dir");
    let script = bridge_script(dir.path());
    let descriptor = descriptor_with_executor(
        ToolExecutorKind::ProcessBridge,
        &json!({ "command": script }).to_string(),
    );
    let result = execute_tool_descriptor(
        &descriptor,
        &ToolKind::Generic(descriptor.name.clone()),
        &ToolInput::Generic {
            summary: "{}".to_owned(),
        },
        DescriptorExecutionContext::new(dir.path(), None, None),
    )
    .await;

    assert!(!result.is_error(), "{}", result.output);
    assert_eq!(result.output, "bridge ok");
}

#[tokio::test]
#[serial]
async fn host_registered_process_bridge_tool_dispatches_through_execute_tool_normal() {
    descriptor_catalog::clear_external_tool_descriptors_for_tests();
    let dir = tempfile::tempdir().expect("temp dir");
    let script = bridge_script(dir.path());
    let descriptor = ToolDescriptor::new(
        PluginId::new("test.bridge-plugin"),
        "external_echo",
        "external echo test tool",
        json!({ "type": "object" }),
    )
    .with_executor(
        ToolExecutorKind::ProcessBridge,
        json!({ "command": script }).to_string(),
    )
    .with_visibility(DescriptorVisibility::ModelVisible);
    let descriptors = active_host_descriptors(descriptor);
    assert_eq!(
        descriptor_catalog::register_external_tool_descriptors(descriptors),
        1
    );

    let result = execute_tool(
        ToolKind::from_name("external_echo"),
        ToolInput::Generic {
            summary: r#"{"message":"hi"}"#.to_owned(),
        },
        dir.path().to_path_buf(),
        None,
        None,
        None,
    )
    .await;
    descriptor_catalog::clear_external_tool_descriptors_for_tests();

    assert!(!result.is_error(), "{}", result.output);
    assert_eq!(result.output, "bridge ok");
}

#[test]
#[serial]
fn external_descriptor_policy_drives_permission_modes_normal() {
    descriptor_catalog::clear_external_tool_descriptors_for_tests();
    let read_only = descriptor_named("plugin_read", ToolApprovalPolicy::ReadOnly);
    let mutating = descriptor_named("plugin_write", ToolApprovalPolicy::Mutating);
    let management = descriptor_named("plugin_manage", ToolApprovalPolicy::Management);
    assert_eq!(
        descriptor_catalog::register_external_tool_descriptors([read_only, mutating, management]),
        3
    );

    let input = ToolInput::Generic {
        summary: "{}".to_owned(),
    };

    assert_eq!(
        PermissionMode::Plan.decide_parts(&ToolKind::from_name("plugin_read"), &input),
        PermissionDecision::Approved
    );
    assert_eq!(
        PermissionMode::Default.decide_parts(&ToolKind::from_name("plugin_write"), &input),
        PermissionDecision::NeedsPrompt
    );
    assert_eq!(
        PermissionMode::Plan.decide_parts(&ToolKind::from_name("plugin_write"), &input),
        PermissionDecision::Denied("Plan mode: plugin mutating tool blocked")
    );
    assert_eq!(
        PermissionMode::AcceptEdits.decide_parts(&ToolKind::from_name("plugin_write"), &input),
        PermissionDecision::NeedsPrompt
    );
    assert_eq!(
        PermissionMode::Plan.decide_parts(&ToolKind::from_name("plugin_manage"), &input),
        PermissionDecision::Approved
    );

    descriptor_catalog::clear_external_tool_descriptors_for_tests();
}

#[tokio::test]
#[serial]
async fn mutating_external_descriptor_is_blocked_by_safe_mode_normal() {
    descriptor_catalog::clear_external_tool_descriptors_for_tests();
    let dir = tempfile::tempdir().expect("temp dir");
    let script = bridge_script(dir.path());
    let descriptor = ToolDescriptor::new(
        PluginId::new("test.bridge-plugin"),
        "safe_mode_write",
        "external mutating test tool",
        json!({ "type": "object" }),
    )
    .with_executor(
        ToolExecutorKind::ProcessBridge,
        json!({ "command": script }).to_string(),
    )
    .with_approval_policy(ToolApprovalPolicy::Mutating)
    .with_visibility(DescriptorVisibility::ModelVisible);
    assert_eq!(
        descriptor_catalog::register_external_tool_descriptors(active_host_descriptors(descriptor)),
        1
    );

    let _guard = SafeModeOverrideGuard::enable();
    let result = execute_tool(
        ToolKind::from_name("safe_mode_write"),
        ToolInput::Generic {
            summary: r#"{"message":"hi"}"#.to_owned(),
        },
        dir.path().to_path_buf(),
        None,
        None,
        None,
    )
    .await;
    descriptor_catalog::clear_external_tool_descriptors_for_tests();

    assert!(result.is_error(), "{}", result.output);
    assert!(
        result.output.contains("blocked in safe mode"),
        "{}",
        result.output
    );
    assert_eq!(
        result.diagnostics[0].error_category,
        Some(ToolErrorCategory::Permission)
    );
    assert_eq!(result.diagnostics[0].retryable, Some(false));
}

#[test]
#[serial]
fn model_visible_external_descriptor_is_advertised_normal() {
    descriptor_catalog::clear_external_tool_descriptors_for_tests();
    let descriptor = descriptor_with_executor(ToolExecutorKind::ProcessBridge, "/bin/true")
        .with_visibility(DescriptorVisibility::ModelVisible);
    assert_eq!(
        descriptor_catalog::register_external_tool_descriptors([descriptor]),
        1
    );

    let defs = super::defs::model_tool_defs();
    descriptor_catalog::clear_external_tool_descriptors_for_tests();

    assert!(defs.iter().any(|tool| tool.name == "descriptor_tool"));
}

#[tokio::test]
async fn builtin_descriptor_rejects_handler_mismatch_normal() {
    let dir = tempfile::tempdir().expect("temp dir");
    let descriptor = descriptor_with_executor(ToolExecutorKind::BuiltIn, READ_TOOL_HANDLER);
    let result = execute_tool_descriptor(
        &descriptor,
        &ToolKind::Write,
        &ToolInput::Write {
            file_path: "route.txt".to_owned(),
            content: "content".to_owned(),
        },
        DescriptorExecutionContext::new(dir.path(), None, None),
    )
    .await;

    assert!(result.is_error(), "{}", result.output);
    assert!(result.output.contains("routed to"), "{}", result.output);
}

fn active_host_descriptors(descriptor: ToolDescriptor) -> Vec<ToolDescriptor> {
    let manifest = PluginManifest::new(
        descriptor.plugin_id.clone(),
        PluginVersion::new("0.1.0"),
        PluginSource::built_in("test"),
    );
    let mut host = PluginHost::new();
    host.register_internal(PluginRegistration::new(manifest).with_tool_descriptor(descriptor))
        .expect("register plugin");
    host.activate_all().expect("activate plugin");
    host.tool_descriptors()
}

fn descriptor_named(name: &str, approval_policy: ToolApprovalPolicy) -> ToolDescriptor {
    ToolDescriptor::new(
        PluginId::new("test.policy-plugin"),
        name,
        "descriptor policy test tool",
        json!({ "type": "object" }),
    )
    .with_executor(ToolExecutorKind::ProcessBridge, "/bin/true")
    .with_approval_policy(approval_policy)
    .with_visibility(DescriptorVisibility::ModelVisible)
}

struct SafeModeOverrideGuard;

impl SafeModeOverrideGuard {
    fn enable() -> Self {
        crate::config::set_safe_mode_override(true);
        Self
    }
}

impl Drop for SafeModeOverrideGuard {
    fn drop(&mut self) {
        crate::config::set_safe_mode_override(false);
    }
}

fn bridge_script(dir: &Path) -> PathBuf {
    let script = dir.join("bridge.sh");
    std::fs::write(
        &script,
        r#"#!/bin/sh
read line
id=$(printf '%s\n' "$line" | sed -n 's/.*"id":"\([^"]*\)".*/\1/p')
printf '{"type":"response","id":"%s","response":{"kind":"tool_result","output":"bridge ok","is_error":false}}\n' "$id"
"#,
    )
    .expect("write bridge script");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = std::fs::metadata(&script)
            .expect("script metadata")
            .permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&script, permissions).expect("chmod bridge script");
    }
    script
}

use jfc_plugin_host::{PluginHost, PluginRuntime, PluginStatusKind};
use jfc_plugin_sdk::{DescriptorVisibility, PluginId, ToolApprovalPolicy};
use serde_json::json;

use crate::types::{ReplacementMode, ToolInput, ToolKind};

use super::defs::all_tool_defs;
use super::descriptor_filesystem_defs::filesystem_descriptors;
use super::descriptor_router::{
    BUILTIN_FILESYSTEM_TOOL_PACK_ID, BUILTIN_SEARCH_TOOL_PACK_ID, EDIT_TOOL_HANDLER,
    GLOB_TOOL_HANDLER, GREP_TOOL_HANDLER, MULTI_EDIT_TOOL_HANDLER, NOTEBOOK_EDIT_TOOL_HANDLER,
    NOTEBOOK_READ_TOOL_HANDLER, READ_TOOL_HANDLER, WRITE_TOOL_HANDLER, descriptor_for_handler,
    execute_descriptor_tool, register_builtin_filesystem_tool_pack,
    register_builtin_search_tool_pack,
};
use super::descriptor_search_defs::search_descriptors;
use super::execute_tool;

#[tokio::test]
async fn descriptor_router_dispatches_builtin_glob_normal() {
    let dir = tempfile::tempdir().expect("temp dir");
    tokio::fs::write(dir.path().join("hit.rs"), "")
        .await
        .expect("write fixture");

    let result = execute_descriptor_tool(
        &ToolKind::Glob,
        &ToolInput::Glob {
            pattern: "*.rs".to_owned(),
            path: None,
        },
        dir.path(),
    )
    .await
    .expect("glob descriptor route");

    assert!(!result.is_error(), "{}", result.output);
    assert!(result.output.contains("hit.rs"), "{}", result.output);
}

#[tokio::test]
async fn descriptor_router_dispatches_builtin_grep_normal() {
    let dir = tempfile::tempdir().expect("temp dir");
    tokio::fs::write(dir.path().join("hit.txt"), "needle\n")
        .await
        .expect("write fixture");

    let result = execute_descriptor_tool(
        &ToolKind::Grep,
        &ToolInput::Grep {
            pattern: "needle".to_owned(),
            path: None,
            glob: None,
            output_mode: None,
        },
        dir.path(),
    )
    .await
    .expect("grep descriptor route");

    assert!(!result.is_error(), "{}", result.output);
    assert!(result.output.contains("needle"), "{}", result.output);
}

#[tokio::test]
async fn execute_tool_dispatches_grep_via_descriptor_route_normal() {
    let dir = tempfile::tempdir().expect("temp dir");
    tokio::fs::write(dir.path().join("hit.txt"), "needle\n")
        .await
        .expect("write fixture");

    let result = execute_tool(
        ToolKind::Grep,
        ToolInput::Grep {
            pattern: "needle".to_owned(),
            path: None,
            glob: None,
            output_mode: None,
        },
        dir.path().to_path_buf(),
        None,
        None,
        None,
    )
    .await;

    assert!(!result.is_error(), "{}", result.output);
    assert!(result.output.contains("needle"), "{}", result.output);
}

#[test]
fn builtin_search_descriptors_are_model_visible_read_only_normal() {
    for handler in [GLOB_TOOL_HANDLER, GREP_TOOL_HANDLER] {
        let descriptor = descriptor_for_handler(handler).expect("search descriptor");

        assert_eq!(descriptor.name, handler);
        assert_eq!(descriptor.approval_policy, ToolApprovalPolicy::ReadOnly);
        assert_eq!(descriptor.visibility, DescriptorVisibility::ModelVisible);
    }
}

#[test]
fn builtin_search_tool_pack_reaches_runtime_maps_and_status_normal() {
    let mut host = PluginHost::new();
    register_builtin_search_tool_pack(&mut host).expect("search pack registers");
    host.activate_all().expect("search pack activates");

    let runtime = PluginRuntime::from_host(&host).expect("runtime builds");
    let snapshot = host.status_snapshot();

    for handler in [GLOB_TOOL_HANDLER, GREP_TOOL_HANDLER] {
        let descriptor = runtime
            .tools()
            .get(handler)
            .expect("search descriptor is mapped")
            .descriptor();
        assert_eq!(descriptor.plugin_id.as_str(), BUILTIN_SEARCH_TOOL_PACK_ID);
        assert_eq!(descriptor.name, handler);
    }

    let entry = snapshot
        .plugins
        .iter()
        .find(|entry| entry.plugin_id.as_str() == BUILTIN_SEARCH_TOOL_PACK_ID)
        .expect("search pack status entry");
    assert_eq!(entry.status, PluginStatusKind::Active);
}

#[test]
fn disabled_builtin_search_tool_pack_is_hidden_from_runtime_maps_and_catalog_robust() {
    let mut host = PluginHost::new();
    register_builtin_search_tool_pack(&mut host).expect("search pack registers");
    host.disable_plugin(&PluginId::new(BUILTIN_SEARCH_TOOL_PACK_ID))
        .expect("search pack disables");
    host.activate_all()
        .expect("disabled search pack is skipped");

    let runtime = PluginRuntime::from_host(&host).expect("runtime builds");

    assert!(runtime.tools().is_empty());
    assert!(host.tool_descriptors().is_empty());
    let status = host
        .status_snapshot()
        .plugins
        .iter()
        .find(|entry| entry.plugin_id.as_str() == BUILTIN_SEARCH_TOOL_PACK_ID)
        .expect("search pack status entry")
        .status;
    assert_eq!(status, PluginStatusKind::Disabled);
}

#[test]
fn builtin_search_runtime_schema_matches_existing_descriptor_family_normal() {
    let mut host = PluginHost::new();
    register_builtin_search_tool_pack(&mut host).expect("search pack registers");
    host.activate_all().expect("search pack activates");

    let runtime = PluginRuntime::from_host(&host).expect("runtime builds");
    let expected = search_descriptors(PluginId::new(BUILTIN_SEARCH_TOOL_PACK_ID));

    for descriptor in expected {
        let runtime_descriptor = runtime
            .tools()
            .get(&descriptor.name)
            .expect("search descriptor is mapped")
            .descriptor();

        assert_eq!(runtime_descriptor.description, descriptor.description);
        assert_eq!(runtime_descriptor.input_schema, descriptor.input_schema);
        assert_eq!(runtime_descriptor.executor, descriptor.executor);
        assert_eq!(
            runtime_descriptor.approval_policy,
            descriptor.approval_policy
        );
        assert_eq!(runtime_descriptor.visibility, descriptor.visibility);
    }
}

#[test]
fn builtin_filesystem_descriptors_have_expected_policy_normal() {
    for handler in [READ_TOOL_HANDLER, NOTEBOOK_READ_TOOL_HANDLER] {
        let descriptor = descriptor_for_handler(handler).expect("read descriptor");

        assert_eq!(descriptor.name, handler);
        assert_eq!(descriptor.approval_policy, ToolApprovalPolicy::ReadOnly);
        assert_eq!(descriptor.visibility, DescriptorVisibility::ModelVisible);
    }

    for handler in [
        WRITE_TOOL_HANDLER,
        EDIT_TOOL_HANDLER,
        MULTI_EDIT_TOOL_HANDLER,
        NOTEBOOK_EDIT_TOOL_HANDLER,
    ] {
        let descriptor = descriptor_for_handler(handler).expect("mutating descriptor");

        assert_eq!(descriptor.name, handler);
        assert_eq!(descriptor.approval_policy, ToolApprovalPolicy::Mutating);
        assert_eq!(descriptor.visibility, DescriptorVisibility::ModelVisible);
    }
}

#[test]
fn builtin_filesystem_tool_pack_reaches_runtime_maps_and_status_normal() {
    let mut host = PluginHost::new();
    register_builtin_filesystem_tool_pack(&mut host).expect("filesystem pack registers");
    host.activate_all().expect("filesystem pack activates");

    let runtime = PluginRuntime::from_host(&host).expect("runtime builds");
    let snapshot = host.status_snapshot();

    for handler in [
        READ_TOOL_HANDLER,
        WRITE_TOOL_HANDLER,
        EDIT_TOOL_HANDLER,
        MULTI_EDIT_TOOL_HANDLER,
        NOTEBOOK_READ_TOOL_HANDLER,
        NOTEBOOK_EDIT_TOOL_HANDLER,
    ] {
        let descriptor = runtime
            .tools()
            .get(handler)
            .expect("filesystem descriptor is mapped")
            .descriptor();
        assert_eq!(
            descriptor.plugin_id.as_str(),
            BUILTIN_FILESYSTEM_TOOL_PACK_ID
        );
        assert_eq!(descriptor.name, handler);
    }

    let entry = snapshot
        .plugins
        .iter()
        .find(|entry| entry.plugin_id.as_str() == BUILTIN_FILESYSTEM_TOOL_PACK_ID)
        .expect("filesystem pack status entry");
    assert_eq!(entry.status, PluginStatusKind::Active);
}

#[test]
fn disabled_builtin_filesystem_tool_pack_is_hidden_from_runtime_maps_and_catalog_robust() {
    let mut host = PluginHost::new();
    register_builtin_filesystem_tool_pack(&mut host).expect("filesystem pack registers");
    host.disable_plugin(&PluginId::new(BUILTIN_FILESYSTEM_TOOL_PACK_ID))
        .expect("filesystem pack disables");
    host.activate_all()
        .expect("disabled filesystem pack is skipped");

    let runtime = PluginRuntime::from_host(&host).expect("runtime builds");

    assert!(runtime.tools().is_empty());
    assert!(host.tool_descriptors().is_empty());
    let status = host
        .status_snapshot()
        .plugins
        .iter()
        .find(|entry| entry.plugin_id.as_str() == BUILTIN_FILESYSTEM_TOOL_PACK_ID)
        .expect("filesystem pack status entry")
        .status;
    assert_eq!(status, PluginStatusKind::Disabled);
}

#[test]
fn builtin_filesystem_runtime_schema_matches_existing_descriptor_family_normal() {
    let mut host = PluginHost::new();
    register_builtin_filesystem_tool_pack(&mut host).expect("filesystem pack registers");
    host.activate_all().expect("filesystem pack activates");

    let runtime = PluginRuntime::from_host(&host).expect("runtime builds");
    let expected = filesystem_descriptors(PluginId::new(BUILTIN_FILESYSTEM_TOOL_PACK_ID));

    for descriptor in expected {
        let runtime_descriptor = runtime
            .tools()
            .get(&descriptor.name)
            .expect("filesystem descriptor is mapped")
            .descriptor();

        assert_eq!(runtime_descriptor.description, descriptor.description);
        assert_eq!(runtime_descriptor.input_schema, descriptor.input_schema);
        assert_eq!(runtime_descriptor.executor, descriptor.executor);
        assert_eq!(
            runtime_descriptor.approval_policy,
            descriptor.approval_policy
        );
        assert_eq!(runtime_descriptor.visibility, descriptor.visibility);
    }
}

#[test]
fn advertised_builtin_file_definitions_are_derived_from_descriptors_normal() {
    let tools = all_tool_defs();
    for handler in [
        GLOB_TOOL_HANDLER,
        GREP_TOOL_HANDLER,
        READ_TOOL_HANDLER,
        WRITE_TOOL_HANDLER,
        EDIT_TOOL_HANDLER,
        MULTI_EDIT_TOOL_HANDLER,
        NOTEBOOK_READ_TOOL_HANDLER,
        NOTEBOOK_EDIT_TOOL_HANDLER,
    ] {
        let descriptor = descriptor_for_handler(handler).expect("search descriptor");
        let tool = tools
            .iter()
            .find(|tool| tool.name == handler)
            .expect("search tool def");

        assert_eq!(tool.description, descriptor.description);
        assert_eq!(tool.input_schema, descriptor.input_schema);
    }
}

#[tokio::test]
async fn descriptor_router_dispatches_filesystem_tools_normal() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("route.txt");
    let file_path = path.to_string_lossy().to_string();

    let write = execute_descriptor_tool(
        &ToolKind::Write,
        &ToolInput::Write {
            file_path: file_path.clone(),
            content: "alpha\n".to_owned(),
        },
        dir.path(),
    )
    .await
    .expect("write descriptor route");
    assert!(!write.is_error(), "{}", write.output);

    let edit = execute_descriptor_tool(
        &ToolKind::Edit,
        &ToolInput::Edit {
            file_path: file_path.clone(),
            old_string: "alpha".to_owned(),
            new_string: "beta".to_owned(),
            replacement: ReplacementMode::FirstOnly,
        },
        dir.path(),
    )
    .await
    .expect("edit descriptor route");
    assert!(!edit.is_error(), "{}", edit.output);

    let multi_edit = execute_descriptor_tool(
        &ToolKind::MultiEdit,
        &ToolInput::MultiEdit {
            file_path: file_path.clone(),
            edits: json!([
                {"old_string": "beta", "new_string": "gamma"}
            ]),
        },
        dir.path(),
    )
    .await
    .expect("multi edit descriptor route");
    assert!(!multi_edit.is_error(), "{}", multi_edit.output);

    let read = execute_descriptor_tool(
        &ToolKind::Read,
        &ToolInput::Read {
            file_path,
            offset: None,
            limit: None,
        },
        dir.path(),
    )
    .await
    .expect("read descriptor route");
    assert!(!read.is_error(), "{}", read.output);
    assert!(read.output.contains("gamma"), "{}", read.output);
}

#[tokio::test]
async fn descriptor_router_dispatches_notebook_tools_normal() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("route.ipynb");
    let notebook_path = path.to_string_lossy().to_string();
    tokio::fs::write(
        &path,
        r#"{
  "cells": [
    {
      "cell_type": "code",
      "execution_count": null,
      "id": "cell-a",
      "metadata": {},
      "outputs": [],
      "source": ["old\n"]
    }
  ],
  "metadata": {},
  "nbformat": 4,
  "nbformat_minor": 5
}"#,
    )
    .await
    .expect("write notebook fixture");

    let read = execute_descriptor_tool(
        &ToolKind::NotebookRead,
        &ToolInput::NotebookRead {
            path: notebook_path.clone(),
        },
        dir.path(),
    )
    .await
    .expect("notebook read descriptor route");
    assert!(!read.is_error(), "{}", read.output);
    assert!(read.output.contains("cell-a"), "{}", read.output);

    let edit = execute_descriptor_tool(
        &ToolKind::NotebookEdit,
        &ToolInput::NotebookEdit {
            path: notebook_path.clone(),
            cell_id: "cell-a".to_owned(),
            new_source: "new\n".to_owned(),
            edit_mode: Some("replace".to_owned()),
        },
        dir.path(),
    )
    .await
    .expect("notebook edit descriptor route");
    assert!(!edit.is_error(), "{}", edit.output);

    let updated = tokio::fs::read_to_string(&notebook_path)
        .await
        .expect("read updated notebook");
    assert!(updated.contains("new"), "{updated}");
}

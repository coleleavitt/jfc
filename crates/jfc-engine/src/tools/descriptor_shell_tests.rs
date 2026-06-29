use jfc_plugin_sdk::{DescriptorVisibility, ToolApprovalPolicy};

use crate::types::{ToolInput, ToolKind};

use super::defs::{all_tool_defs, model_tool_defs};
use super::descriptor_router::{
    BASH_OUTPUT_TOOL_HANDLER, BASH_TOOL_HANDLER, descriptor_for_handler, execute_descriptor_tool,
};

#[test]
fn builtin_shell_descriptors_have_expected_visibility_and_policy_normal() {
    let bash = descriptor_for_handler(BASH_TOOL_HANDLER).expect("bash descriptor");
    assert_eq!(bash.name, BASH_TOOL_HANDLER);
    assert_eq!(bash.approval_policy, ToolApprovalPolicy::Mutating);
    assert_eq!(bash.visibility, DescriptorVisibility::ModelVisible);

    let bash_output =
        descriptor_for_handler(BASH_OUTPUT_TOOL_HANDLER).expect("bash output descriptor");
    assert_eq!(bash_output.name, BASH_OUTPUT_TOOL_HANDLER);
    assert_eq!(bash_output.approval_policy, ToolApprovalPolicy::ReadOnly);
    assert_eq!(bash_output.visibility, DescriptorVisibility::HostVisible);
}

#[test]
fn advertised_shell_definitions_are_derived_from_descriptors_normal() {
    let tools = all_tool_defs();
    for handler in [BASH_TOOL_HANDLER, BASH_OUTPUT_TOOL_HANDLER] {
        let descriptor = descriptor_for_handler(handler).expect("shell descriptor");
        let tool = tools
            .iter()
            .find(|tool| tool.name == handler)
            .expect("shell tool def");

        assert_eq!(tool.description, descriptor.description);
        assert_eq!(tool.input_schema, descriptor.input_schema);
    }
}

#[test]
fn bash_output_stays_hidden_from_model_tools_normal() {
    let model_defs = model_tool_defs();
    assert!(
        !model_defs
            .iter()
            .any(|tool| tool.name == BASH_OUTPUT_TOOL_HANDLER),
        "BashOutput stays executable for legacy transcripts but must not be model-visible"
    );
}

#[tokio::test]
async fn descriptor_router_dispatches_shell_tools_normal() {
    let dir = tempfile::tempdir().expect("temp dir");

    let bash = execute_descriptor_tool(
        &ToolKind::Bash,
        &ToolInput::Bash {
            command: "printf descriptor-shell".to_owned(),
            timeout: Some(5_000),
            workdir: None,
            run_in_background: None,
            suppress_output: None,
        },
        dir.path(),
    )
    .await
    .expect("bash descriptor route");
    assert!(!bash.is_error(), "{}", bash.output);
    assert!(bash.output.contains("descriptor-shell"), "{}", bash.output);

    let bash_output = execute_descriptor_tool(
        &ToolKind::BashOutput,
        &ToolInput::BashOutput {
            task_id: "bash_missing_descriptor".to_owned(),
            offset: None,
            limit: None,
            block: Some(false),
            timeout: None,
            wait_up_to: None,
        },
        dir.path(),
    )
    .await
    .expect("bash output descriptor route");
    assert!(!bash_output.is_error(), "{}", bash_output.output);
    assert!(
        bash_output.output.contains("BashOutput polling is ignored"),
        "{}",
        bash_output.output
    );
}

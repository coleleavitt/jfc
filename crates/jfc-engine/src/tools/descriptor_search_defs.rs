use jfc_plugin_sdk::{
    DescriptorVisibility, PluginId, ToolApprovalPolicy, ToolDescriptor, ToolExecutorKind,
};
use serde_json::json;

pub(crate) const GLOB_HANDLER: &str = "Glob";
pub(crate) const GREP_HANDLER: &str = "Grep";

pub(crate) fn search_descriptors(plugin_id: PluginId) -> Vec<ToolDescriptor> {
    vec![
        glob_descriptor(plugin_id.clone()),
        grep_descriptor(plugin_id),
    ]
}

fn glob_descriptor(plugin_id: PluginId) -> ToolDescriptor {
    ToolDescriptor::new(
        plugin_id,
        "Glob",
        "Find files by glob pattern under the current workspace or a supplied path.",
        json!({
            "type": "object",
            "properties": {
                "pattern": {
                    "type": "string",
                    "description": "Glob pattern to match"
                },
                "path": {
                    "type": "string",
                    "description": "Directory to search. Relative paths are resolved by the tool executor."
                }
            },
            "required": ["pattern"]
        }),
    )
    .with_executor(ToolExecutorKind::BuiltIn, GLOB_HANDLER)
    .with_approval_policy(ToolApprovalPolicy::ReadOnly)
    .with_visibility(DescriptorVisibility::ModelVisible)
}

fn grep_descriptor(plugin_id: PluginId) -> ToolDescriptor {
    ToolDescriptor::new(
        plugin_id,
        "Grep",
        "Fast content search using ripgrep. Searches file contents using regular expressions. For finding symbols by name, prefer CodeGraph. Reserve Grep for string literals, config values, error messages, comments, and non-identifier patterns.",
        json!({
            "type": "object",
            "properties": {
                "pattern": {
                    "type": "string",
                    "description": "The regex pattern to search for in file contents"
                },
                "path": {
                    "type": "string",
                    "description": "File or directory to search in. Defaults to current working directory."
                },
                "glob": {
                    "type": "string",
                    "description": "File pattern filter (e.g. '*.ts', '*.{ts,tsx}')"
                },
                "output_mode": {
                    "type": "string",
                    "enum": ["content", "files_with_matches", "count"],
                    "description": "Output mode: content shows matching lines, files_with_matches shows file paths, count shows match counts"
                }
            },
            "required": ["pattern"]
        }),
    )
    .with_executor(ToolExecutorKind::BuiltIn, GREP_HANDLER)
    .with_approval_policy(ToolApprovalPolicy::ReadOnly)
    .with_visibility(DescriptorVisibility::ModelVisible)
}

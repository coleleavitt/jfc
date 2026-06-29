use jfc_plugin_sdk::{
    DescriptorVisibility, PluginId, ToolApprovalPolicy, ToolDescriptor, ToolExecutorKind,
};
use serde_json::json;

pub(crate) const EDIT_HANDLER: &str = "Edit";
pub(crate) const MULTI_EDIT_HANDLER: &str = "MultiEdit";
pub(crate) const NOTEBOOK_EDIT_HANDLER: &str = "NotebookEdit";
pub(crate) const NOTEBOOK_READ_HANDLER: &str = "NotebookRead";
pub(crate) const READ_HANDLER: &str = "Read";
pub(crate) const WRITE_HANDLER: &str = "Write";

pub(crate) fn filesystem_descriptors(plugin_id: PluginId) -> Vec<ToolDescriptor> {
    vec![
        read_descriptor(plugin_id.clone()),
        write_descriptor(plugin_id.clone()),
        edit_descriptor(plugin_id.clone()),
        multi_edit_descriptor(plugin_id.clone()),
        notebook_read_descriptor(plugin_id.clone()),
        notebook_edit_descriptor(plugin_id),
    ]
}

fn read_descriptor(plugin_id: PluginId) -> ToolDescriptor {
    ToolDescriptor::new(
        plugin_id,
        "Read",
        "Read a file or directory from the local filesystem. Returns file contents with line numbers prefixed. For source code, prefer CodeGraph first: `codegraph_explore`, `codegraph_search`, or `codegraph_node` can return relevant symbol bodies directly, and MCP installs may expose them as names like `mcp__codegraph__codegraph_explore`. Use Read mainly for files you're about to edit, precise ranges CodeGraph identified, or non-source files. When reading a large source file for one region, pass `offset`/`limit` instead of reading the whole file.",
        json!({
            "type": "object",
            "properties": {
                "file_path": {
                    "type": "string",
                    "description": "The absolute path to the file or directory to read"
                },
                "offset": {
                    "type": "number",
                    "description": "Line number to start reading from (1-indexed)"
                },
                "limit": {
                    "type": "number",
                    "description": "Maximum number of lines to read (defaults to 2000)"
                }
            },
            "required": ["file_path"]
        }),
    )
    .with_executor(ToolExecutorKind::BuiltIn, READ_HANDLER)
    .with_approval_policy(ToolApprovalPolicy::ReadOnly)
    .with_visibility(DescriptorVisibility::ModelVisible)
}

fn write_descriptor(plugin_id: PluginId) -> ToolDescriptor {
    ToolDescriptor::new(
        plugin_id,
        "Write",
        "Write a file to the local filesystem. Overwrites existing file if present.",
        json!({
            "type": "object",
            "properties": {
                "file_path": {
                    "type": "string",
                    "description": "The absolute path to the file to write"
                },
                "content": {
                    "type": "string",
                    "description": "The content to write to the file"
                }
            },
            "required": ["file_path", "content"]
        }),
    )
    .with_executor(ToolExecutorKind::BuiltIn, WRITE_HANDLER)
    .with_approval_policy(ToolApprovalPolicy::Mutating)
    .with_visibility(DescriptorVisibility::ModelVisible)
}

fn edit_descriptor(plugin_id: PluginId) -> ToolDescriptor {
    ToolDescriptor::new(
        plugin_id,
        "Edit",
        "Performs exact string replacements in a file. Use Read first to verify the exact content before editing.",
        json!({
            "type": "object",
            "properties": {
                "file_path": {
                    "type": "string",
                    "description": "The absolute path to the file to modify"
                },
                "old_string": {
                    "type": "string",
                    "description": "The text to replace (must match exactly, including whitespace)"
                },
                "new_string": {
                    "type": "string",
                    "description": "The replacement text"
                },
                "replace_all": {
                    "type": "boolean",
                    "description": "Replace all occurrences (default false)"
                }
            },
            "required": ["file_path", "old_string", "new_string"]
        }),
    )
    .with_executor(ToolExecutorKind::BuiltIn, EDIT_HANDLER)
    .with_approval_policy(ToolApprovalPolicy::Mutating)
    .with_visibility(DescriptorVisibility::ModelVisible)
}

fn multi_edit_descriptor(plugin_id: PluginId) -> ToolDescriptor {
    ToolDescriptor::new(
        plugin_id,
        "MultiEdit",
        "Apply multiple edits to a single file in one tool call. `edits` is an array of `{old_string, new_string, replace_all?}` objects, applied in order. Saves a tool round-trip when several rewrites target the same source file.",
        json!({
            "type": "object",
            "properties": {
                "file_path": { "type": "string" },
                "edits": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "old_string": { "type": "string" },
                            "new_string": { "type": "string" },
                            "replace_all": { "type": "boolean", "default": false }
                        },
                        "required": ["old_string", "new_string"]
                    }
                }
            },
            "required": ["file_path", "edits"]
        }),
    )
    .with_executor(ToolExecutorKind::BuiltIn, MULTI_EDIT_HANDLER)
    .with_approval_policy(ToolApprovalPolicy::Mutating)
    .with_visibility(DescriptorVisibility::ModelVisible)
}

fn notebook_read_descriptor(plugin_id: PluginId) -> ToolDescriptor {
    ToolDescriptor::new(
        plugin_id,
        "NotebookRead",
        "Read a Jupyter `.ipynb` notebook and return each cell's id, type, source, and outputs. Use before NotebookEdit to discover cell IDs.",
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Absolute path to the .ipynb file."
                }
            },
            "required": ["path"]
        }),
    )
    .with_executor(ToolExecutorKind::BuiltIn, NOTEBOOK_READ_HANDLER)
    .with_approval_policy(ToolApprovalPolicy::ReadOnly)
    .with_visibility(DescriptorVisibility::ModelVisible)
}

fn notebook_edit_descriptor(plugin_id: PluginId) -> ToolDescriptor {
    ToolDescriptor::new(
        plugin_id,
        "NotebookEdit",
        "Edit a Jupyter `.ipynb` notebook by cell id. `edit_mode=replace` overwrites the cell's source; `insert` adds a new code cell after the named cell; `delete` removes the cell.",
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Absolute path to the .ipynb file."
                },
                "cell_id": {
                    "type": "string",
                    "description": "Target cell id (from NotebookRead). For `insert` mode the new cell is placed AFTER this one."
                },
                "new_source": {
                    "type": "string",
                    "description": "Replacement (or new) cell source. Ignored when edit_mode=delete."
                },
                "edit_mode": {
                    "type": "string",
                    "enum": ["replace", "insert", "delete"],
                    "description": "How to apply the edit. Defaults to replace."
                }
            },
            "required": ["path", "cell_id", "new_source"]
        }),
    )
    .with_executor(ToolExecutorKind::BuiltIn, NOTEBOOK_EDIT_HANDLER)
    .with_approval_policy(ToolApprovalPolicy::Mutating)
    .with_visibility(DescriptorVisibility::ModelVisible)
}

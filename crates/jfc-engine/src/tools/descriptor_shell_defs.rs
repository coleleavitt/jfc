use jfc_plugin_sdk::{
    DescriptorVisibility, PluginId, ToolApprovalPolicy, ToolDescriptor, ToolExecutorKind,
};
use serde_json::json;

pub(crate) const BASH_HANDLER: &str = "Bash";
pub(crate) const BASH_OUTPUT_HANDLER: &str = "BashOutput";

pub(crate) fn shell_descriptors(plugin_id: PluginId) -> Vec<ToolDescriptor> {
    vec![
        bash_descriptor(plugin_id.clone()),
        bash_output_descriptor(plugin_id),
    ]
}

fn bash_descriptor(plugin_id: PluginId) -> ToolDescriptor {
    ToolDescriptor::new(
        plugin_id,
        "Bash",
        "Executes a bash command in a fresh non-interactive shell. Shell state does not persist between calls; use `workdir` to run in a specific directory. Prefer Glob/Grep/Read/Edit/Write for file discovery and edits. Use Bash for real shell commands, scripts, builds, tests, and package managers. Set `suppressOutput=true` when raw stdout/stderr would be noisy or sensitive but success/failure status is enough. For long-running commands, set `run_in_background=true`; JFC tracks the task id and output file and will report completion, so do not spawn separate sleep/poll commands. If waiting for a remote condition, run one bounded background watcher such as `until check; do sleep 2; done`, and JFC reports its output when the watcher settles. JFC also auto-backgrounds commands that exceed the foreground budget. PERSISTENT SHELL (opt-in): to keep cwd/env across calls (e.g. a `cd` or `export` that should persist), prefix the command with `shell:<id>` and a newline, e.g. command = \"shell:build\\ncd src && make\". All commands sharing the same `<id>` run in one long-lived shell, in order; omit the prefix for the default fresh-shell behavior.",
        json!({
            "type": "object",
            "properties": {
                "command": {
                    "type": "string",
                    "description": "The command to execute"
                },
                "timeout": {
                    "type": "number",
                    "description": "Optional timeout in milliseconds (max 600000)"
                },
                "workdir": {
                    "type": "string",
                    "description": "Directory to run the command in. Relative paths are resolved against the current workspace directory."
                },
                "run_in_background": {
                    "type": "boolean",
                    "description": "Start the command as a background Bash task and return immediately with a task id and output file. Use this for builds, servers, long scans, remote waits, and condition watchers instead of foreground sleep/poll loops."
                },
                "suppressOutput": {
                    "type": "boolean",
                    "description": "Suppress successful foreground command output in the tool result while preserving status/provenance. Failure output is still returned."
                },
                "description": {
                    "type": "string",
                    "description": "Clear, concise description of what this command does"
                }
            },
            "required": ["command"]
        }),
    )
    .with_executor(ToolExecutorKind::BuiltIn, BASH_HANDLER)
    .with_approval_policy(ToolApprovalPolicy::Mutating)
    .with_visibility(DescriptorVisibility::ModelVisible)
}

fn bash_output_descriptor(plugin_id: PluginId) -> ToolDescriptor {
    ToolDescriptor::new(
        plugin_id,
        "BashOutput",
        "Read or wait for output from a Bash command that was backgrounded by `Bash.run_in_background` or auto-backgrounded after exceeding the foreground budget. By default block=true waits up to timeout for task completion and returns retrieval_status success/timeout/not_ready. Prefer this over issuing separate sleep commands while a background task is running. Use block=false for a snapshot. Use offset/limit for large logs.",
        json!({
            "type": "object",
            "properties": {
                "task_id": {
                    "type": "string",
                    "description": "Background Bash task id returned by Bash"
                },
                "offset": {
                    "type": "number",
                    "description": "Optional 1-indexed line number to start reading from"
                },
                "limit": {
                    "type": "number",
                    "description": "Maximum number of lines to return"
                },
                "block": {
                    "type": "boolean",
                    "description": "Whether to wait for completion before returning. Defaults to true."
                },
                "timeout": {
                    "type": "number",
                    "description": "Max wait time in milliseconds when block=true. Defaults to 30000, max 600000."
                },
                "wait_up_to": {
                    "type": "number",
                    "description": "Alias for timeout in milliseconds when block=true."
                }
            },
            "required": ["task_id"]
        }),
    )
    .with_executor(ToolExecutorKind::BuiltIn, BASH_OUTPUT_HANDLER)
    .with_approval_policy(ToolApprovalPolicy::ReadOnly)
    .with_visibility(DescriptorVisibility::HostVisible)
}

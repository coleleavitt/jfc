use jfc_provider::ToolDef;

/// Daemon-driven scheduling, LSP, notifications, scratchpad, and workflow tools.
pub fn daemon_tool_defs() -> Vec<ToolDef> {
    vec![
        cron_create_def(),
        cron_list_def(),
        cron_delete_def(),
        schedule_wakeup_def(),
        monitor_def(),
        lsp_def(),
        push_notification_def(),
        remote_trigger_def(),
        scratchpad_read_def(),
        scratchpad_write_def(),
        workflow_def(),
        wait_for_mcp_servers_def(),
        list_mcp_resources_def(),
        read_mcp_resource_def(),
    ]
}

fn cron_create_def() -> ToolDef {
    ToolDef {
        name: "CronCreate".into(),
        description: "Register a recurring cron job with the local jfc daemon. \
            Schedule accepts five-field crontab (`*/5 * * * *`), `@hourly`, \
            `@daily`, `@weekly`, or `@every <duration>` (e.g. `@every 5m`, \
            `@every 1h30m`). Returns the new job's id."
            .into(),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "schedule": { "type": "string" },
                "command": { "type": "string" },
                "description": { "type": "string" }
            },
            "required": ["schedule", "command", "description"]
        }),
    }
}

fn cron_list_def() -> ToolDef {
    ToolDef {
        name: "CronList".into(),
        description: "List all cron jobs currently registered with the local jfc daemon.".into(),
        input_schema: serde_json::json!({ "type": "object", "properties": {} }),
    }
}

fn cron_delete_def() -> ToolDef {
    ToolDef {
        name: "CronDelete".into(),
        description: "Delete a cron job from the local jfc daemon by id.".into(),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": { "id": { "type": "string" } },
            "required": ["id"]
        }),
    }
}

fn schedule_wakeup_def() -> ToolDef {
    ToolDef {
        name: "ScheduleWakeup".into(),
        description: "Schedule a one-shot wakeup that re-posts a prompt to \
            the conversation after `delay_seconds` elapse. Persisted to \
            daemon state so it replays after restart."
            .into(),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "delay_seconds": { "type": "integer", "minimum": 0 },
                "prompt": { "type": "string" },
                "reason": { "type": "string" }
            },
            "required": ["delay_seconds", "prompt", "reason"]
        }),
    }
}

fn monitor_def() -> ToolDef {
    ToolDef {
        name: "Monitor".into(),
        description: "Spawn a long-running command and stream stdout \
            line-by-line, returning the first line matching the `until` \
            regex. Times out after 60s."
            .into(),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "command": { "type": "string" },
                "until": { "type": "string" }
            },
            "required": ["command", "until"]
        }),
    }
}

fn lsp_def() -> ToolDef {
    ToolDef {
        name: "LSP".into(),
        description: "Query the language server for `hover`, `definition`, \
            `references`, `implementation`, `type_definition`, `document_symbols`, \
            `workspace_symbols`, `incoming_calls`, or `outgoing_calls` at a specific \
            source location. Uses the already-spawned LSP client (rust-analyzer / zls / \
            etc.) — returns an error if no LSP is running for the workspace."
            .into(),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "kind": {
                    "type": "string",
                    "enum": ["hover", "definition", "references", "implementation", "type_definition", "document_symbols", "workspace_symbols", "incoming_calls", "outgoing_calls"],
                    "description": "Which LSP request to issue."
                },
                "file": {
                    "type": "string",
                    "description": "Absolute path to the source file."
                },
                "line": {
                    "type": "number",
                    "description": "1-indexed line number of the symbol position."
                },
                "column": {
                    "type": "number",
                    "description": "1-indexed column number of the symbol position."
                },
                "query": {
                    "type": "string",
                    "description": "Search query for workspace_symbols. Ignored for other kinds."
                }
            },
            "required": ["kind", "file", "line", "column"]
        }),
    }
}

fn push_notification_def() -> ToolDef {
    ToolDef {
        name: "PushNotification".into(),
        description: "Send a desktop notification to the user via the \
            native notification daemon (notify-send / NotificationCenter / \
            Toast). Use sparingly for events that need attention while \
            the user has switched focus away from the terminal."
            .into(),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "message": {
                    "type": "string",
                    "description": "Body text shown in the notification."
                },
                "title": {
                    "type": "string",
                    "description": "Optional title; defaults to `jfc`."
                }
            },
            "required": ["message"]
        }),
    }
}

fn remote_trigger_def() -> ToolDef {
    ToolDef {
        name: "RemoteTrigger".into(),
        description: "POST a payload to a webhook URL the user pre-registered \
            in `~/.config/jfc/triggers.toml`. Use to fire CI runs, Slack \
            hooks, custom alert endpoints, etc. without exposing the URL \
            to the model. Triggers are looked up by `trigger_id`; unknown \
            IDs return an error."
            .into(),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "trigger_id": {
                    "type": "string",
                    "description": "Identifier registered in `~/.config/jfc/triggers.toml`."
                },
                "payload": {
                    "type": "object",
                    "description": "Optional JSON object POSTed as the request body."
                }
            },
            "required": ["trigger_id"]
        }),
    }
}

fn scratchpad_read_def() -> ToolDef {
    ToolDef {
        name: "ScratchpadRead".into(),
        description: "Read a value from the shared inter-agent scratchpad by key. Returns the value if set, or an error if the key doesn't exist. Use this to read findings left by sibling agents.".into(),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "key": {
                    "type": "string",
                    "description": "The key to read from the scratchpad"
                }
            },
            "required": ["key"]
        }),
    }
}

fn scratchpad_write_def() -> ToolDef {
    ToolDef {
        name: "ScratchpadWrite".into(),
        description: "Write a key-value pair to the shared inter-agent scratchpad. Other agents (siblings, teammates) can read this value via ScratchpadRead. Use for sharing discovered facts, file paths, intermediate results.".into(),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "key": {
                    "type": "string",
                    "description": "The key to write to the scratchpad"
                },
                "value": {
                    "type": "string",
                    "description": "The value to store"
                }
            },
            "required": ["key", "value"]
        }),
    }
}

fn workflow_def() -> ToolDef {
    ToolDef {
        name: "Workflow".into(),
        description: "Execute a workflow script that orchestrates multiple subagents deterministically. Workflows run in the background — this tool returns immediately with a task ID, and a <task-notification> arrives when the workflow completes. ONLY call this tool when the user has explicitly opted into multi-agent orchestration (ultrawork keyword, direct request, or skill instruction).".into(),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "script": {
                    "type": "string",
                    "description": "Self-contained workflow script. Must begin with `export const meta = { name, description, phases }` followed by the script body using agent()/parallel()/pipeline()/phase()."
                },
                "name": {
                    "type": "string",
                    "description": "Name of a predefined workflow (built-in or from .claude/workflows/). Resolves to a script."
                },
                "scriptPath": {
                    "type": "string",
                    "description": "Path to a workflow script file on disk."
                },
                "args": {
                    "description": "Optional input value exposed to the script as the global `args`."
                },
                "resumeFromRunId": {
                    "type": "string",
                    "description": "Run ID of a prior Workflow invocation to resume from. Completed agent() calls return cached results instantly."
                }
            }
        }),
    }
}

fn wait_for_mcp_servers_def() -> ToolDef {
    ToolDef {
        name: "WaitForMcpServers".into(),
        description: "Block until all configured MCP servers report ready. \
            Returns a list of connected servers and any that timed out. \
            Use this at session start when you need MCP tools to be available \
            before proceeding."
            .into(),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "timeout_ms": {
                    "type": "integer",
                    "minimum": 1000,
                    "maximum": 120000,
                    "default": 30000,
                    "description": "Maximum time to wait for servers in milliseconds (default 30s)"
                }
            }
        }),
    }
}

fn list_mcp_resources_def() -> ToolDef {
    ToolDef {
        name: "ListMcpResources".into(),
        description: "List resources exposed by connected MCP servers. Optionally \
            filter by server name. Returns resource names and URIs grouped by server."
            .into(),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "server": {
                    "type": "string",
                    "description": "Optional server name to filter resources from a single server"
                }
            }
        }),
    }
}

fn read_mcp_resource_def() -> ToolDef {
    ToolDef {
        name: "ReadMcpResource".into(),
        description: "Read the contents of a specific MCP resource by URI. \
            The server name identifies which MCP server hosts the resource."
            .into(),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "server": {
                    "type": "string",
                    "description": "Name of the MCP server hosting the resource"
                },
                "uri": {
                    "type": "string",
                    "description": "URI of the resource to read"
                }
            },
            "required": ["server", "uri"]
        }),
    }
}

use jfc_provider::ToolDef;

pub fn hcom_tool_defs() -> Vec<ToolDef> {
    vec![
        simple_hcom_def(
            "HcomStatus",
            "Show hcom system health, installed hooks, active agents, relay status, and logs.",
            serde_json::json!({
                "json": {"type": "boolean", "description": "Return JSON output"},
                "logs": {"type": "boolean", "description": "Include recent hcom logs"}
            }),
        ),
        simple_hcom_def(
            "HcomList",
            "List hcom-tracked agents or inspect one hcom agent by name/field.",
            serde_json::json!({
                "name": {"type": "string", "description": "Optional agent name, or self"},
                "field": {"type": "string", "description": "Optional field to extract for a named agent"},
                "stopped": {"type": "boolean", "description": "Show recently stopped agents"},
                "json": {"type": "boolean", "description": "Return JSON output"},
                "names": {"type": "boolean", "description": "Return only agent names"},
                "verbose": {"type": "boolean", "description": "Verbose output"},
                "all": {"type": "boolean", "description": "Show all stopped entries when used with stopped"},
                "last": {"type": "number", "description": "Limit stopped entries"},
                "format": {"type": "string", "description": "hcom list format template"}
            }),
        ),
        ToolDef {
            name: "HcomSend".into(),
            description: "Send a message to external hcom agents. Use this for agents outside the current JFC team; use SendMessage for JFC teammates.".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "targets": {"type": "array", "items": {"type": "string"}, "description": "Target agents, with or without @ prefix"},
                    "message": {"type": "string", "description": "Message text to send"},
                    "intent": {"type": "string", "enum": ["request", "inform", "ack"], "description": "Message intent"},
                    "reply_to": {"type": "string", "description": "Event id this replies to"},
                    "thread": {"type": "string", "description": "Thread name for hcom thread routing"},
                    "from": {"type": "string", "description": "External sender identity"},
                    "title": {"type": "string", "description": "Inline bundle title"},
                    "description": {"type": "string", "description": "Inline bundle description"},
                    "events": {"type": "string", "description": "Inline bundle event ids/ranges"},
                    "files": {"type": "array", "items": {"type": "string"}, "description": "Inline bundle file paths"},
                    "transcript": {"type": "string", "description": "Inline bundle transcript ranges"},
                    "extends": {"type": "string", "description": "Parent bundle id"}
                },
                "required": ["message"]
            }),
        },
        vararg_hcom_def(
            "HcomEvents",
            "Query, wait for, subscribe to, or unsubscribe from hcom events. Args are passed after `hcom events`.",
        ),
        ToolDef {
            name: "HcomListen".into(),
            description: "Wait for hcom messages or events matching filters.".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "timeout": {"type": "number", "description": "Timeout in seconds"},
                    "json": {"type": "boolean", "description": "Return JSON output"},
                    "sql": {"type": "string", "description": "Raw SQL WHERE filter"},
                    "args": {"type": "array", "items": {"type": "string"}, "description": "Additional hcom listen filter flags"}
                }
            }),
        },
        vararg_hcom_def(
            "HcomTranscript",
            "View, search, or show timelines for hcom agent transcripts. Args are passed after `hcom transcript`.",
        ),
        vararg_hcom_def(
            "HcomBundle",
            "List, show, create, prepare, cat, or chain hcom context bundles. Args are passed after `hcom bundle`.",
        ),
        vararg_hcom_def(
            "HcomTerm",
            "Inspect screens or inject text into PTY-backed hcom agents. Args are passed after `hcom term`.",
        ),
        ToolDef {
            name: "HcomLaunch".into(),
            description: "Launch external agents under hcom, including remote/headless/tagged launches.".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "tool": {"type": "string", "description": "Agent tool to launch, such as claude, codex, gemini, opencode, kilo, cursor, kimi, copilot, or pi"},
                    "count": {"type": "number", "description": "Number of agents to launch"},
                    "tag": {"type": "string", "description": "hcom tag"},
                    "terminal": {"type": "string", "description": "Terminal preset"},
                    "headless": {"type": "boolean", "description": "Launch headless/background where supported"},
                    "device": {"type": "string", "description": "Remote relay device short id"},
                    "dir": {"type": "string", "description": "Working directory for launched agent"},
                    "prompt": {"type": "string", "description": "Initial hcom prompt"},
                    "system_prompt": {"type": "string", "description": "Additional hcom system prompt"},
                    "batch_id": {"type": "string", "description": "Batch id to wait/track launch completion"},
                    "run_here": {"type": "boolean", "description": "Remote launch run-here toggle"},
                    "args": {"type": "array", "items": {"type": "string"}, "description": "Tool-specific args forwarded to the launched agent"}
                },
                "required": ["tool"]
            }),
        },
        target_vararg_hcom_def(
            "HcomResume",
            "Resume an hcom-tracked agent/session. Args are forwarded after the target.",
        ),
        target_vararg_hcom_def(
            "HcomFork",
            "Fork an hcom-tracked agent/session. Args are forwarded after the target.",
        ),
        ToolDef {
            name: "HcomKill".into(),
            description: "Kill hcom-tracked agents by name, all, or tag selector.".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "targets": {"type": "array", "items": {"type": "string"}, "description": "Names, all, or tag:X selectors"}
                },
                "required": ["targets"]
            }),
        },
        vararg_hcom_def(
            "HcomRelay",
            "Manage hcom cross-device relay state. Args are passed after `hcom relay`.",
        ),
        ToolDef {
            name: "HcomRun".into(),
            description: "Run an hcom bundled or user workflow script such as debate, confess, or fatcow.".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "script": {"type": "string", "description": "Script name"},
                    "args": {"type": "array", "items": {"type": "string"}, "description": "Arguments forwarded to the script"}
                }
            }),
        },
    ]
}

fn simple_hcom_def(name: &str, description: &str, properties: serde_json::Value) -> ToolDef {
    ToolDef {
        name: name.into(),
        description: description.into(),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": properties
        }),
    }
}

fn vararg_hcom_def(name: &str, description: &str) -> ToolDef {
    ToolDef {
        name: name.into(),
        description: description.into(),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "args": {
                    "type": "array",
                    "items": {"type": "string"},
                    "description": "Arguments forwarded to the hcom subcommand"
                }
            }
        }),
    }
}

fn target_vararg_hcom_def(name: &str, description: &str) -> ToolDef {
    ToolDef {
        name: name.into(),
        description: description.into(),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "target": {"type": "string", "description": "hcom agent/session name or id"},
                "args": {
                    "type": "array",
                    "items": {"type": "string"},
                    "description": "Arguments forwarded after the target"
                }
            },
            "required": ["target"]
        }),
    }
}

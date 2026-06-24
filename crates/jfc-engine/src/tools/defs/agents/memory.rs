use jfc_provider::ToolDef;

pub(super) fn memory_tool_defs() -> Vec<ToolDef> {
    vec![
        ToolDef {
            name: "MemoryCreate".into(),
            description: "Save a persistent memory that will be included in future conversations. Use this to remember user preferences, project conventions, feedback, and important context.".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "level": {
                        "type": "string",
                        "enum": ["user", "project"],
                        "description": "Where to store: 'user' (~/.config/jfc/memory/) for personal prefs, 'project' (.jfc/memory/) for project knowledge"
                    },
                    "memory_type": {
                        "type": "string",
                        "enum": ["feedback", "preference", "project", "context"],
                        "description": "Category: 'feedback' for corrections/confirmations, 'preference' for style/workflow, 'project' for goals/initiatives, 'context' for general facts"
                    },
                    "scope": {
                        "type": "string",
                        "enum": ["private", "team"],
                        "description": "Visibility: 'private' for current user only, 'team' for all project users"
                    },
                    "body": {
                        "type": "string",
                        "description": "The memory content. Lead with the rule/fact, then a Why: line and How to apply: line."
                    }
                },
                "required": ["level", "memory_type", "scope", "body"]
            }),
        },
        ToolDef {
            name: "MemoryDelete".into(),
            description: "Delete a previously saved memory file. Use when a memory is stale, incorrect, or superseded.".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Absolute path to the memory file to delete"
                    }
                },
                "required": ["path"]
            }),
        },
    ]
}

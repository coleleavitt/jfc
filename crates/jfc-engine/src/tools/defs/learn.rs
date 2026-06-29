use jfc_provider::ToolDef;

pub fn learn_tool_defs() -> Vec<ToolDef> {
    vec![
        empty_def("learn_status", "Show learning subsystem status."),
        empty_def(
            "learn_historize",
            "Convert pending transcripts into durable project memory.",
        ),
        empty_def(
            "learn_dream",
            "Run the Dreamer maintenance cycle and persist RSI candidates.",
        ),
        rsi_list_def(),
        rsi_definition_def(
            "learn_rsi_promote",
            "Promote a verified RSI candidate definition into the active runtime slot.",
        ),
        rsi_definition_def(
            "learn_rsi_rollback",
            "Rollback an active RSI definition using its stored rollback metadata.",
        ),
        empty_def("learn_key_files_list", "List pinned project key files."),
        empty_def(
            "learn_user_profile_show",
            "Show durable user-profile observations and promoted facets.",
        ),
    ]
}

fn empty_def(name: &str, description: &str) -> ToolDef {
    ToolDef {
        name: name.to_owned(),
        description: description.to_owned(),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {},
        }),
    }
}

fn rsi_list_def() -> ToolDef {
    ToolDef {
        name: "learn_rsi_list".to_owned(),
        description:
            "List auditable RSI candidates and active RSI definitions before promotion or rollback."
                .to_owned(),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "status": {
                    "type": "string",
                    "description": "Optional status filter: candidate, active, rejected, superseded, or all."
                },
                "limit": {
                    "type": "integer",
                    "description": "Maximum rows per source to show."
                }
            },
        }),
    }
}

fn rsi_definition_def(name: &str, description: &str) -> ToolDef {
    ToolDef {
        name: name.to_owned(),
        description: description.to_owned(),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "kind": {
                    "type": "string",
                    "description": "Definition kind, such as system_prompt, tool_definition, skill, budget_policy, context_playbook, or harness_patch."
                },
                "name": {
                    "type": "string",
                    "description": "Definition name to promote or rollback."
                }
            },
            "required": ["kind", "name"]
        }),
    }
}

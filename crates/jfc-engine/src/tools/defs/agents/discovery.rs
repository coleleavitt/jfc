use jfc_provider::ToolDef;

pub(super) fn discovery_tool_defs() -> Vec<ToolDef> {
    vec![
        ToolDef {
            name: "ToolSearch".into(),
            description: "Search the available tool, skill, and MCP catalogue by keyword. Use when you are unsure of the exact callable name or want to discover a relevant skill/tool before invoking it.".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "Keyword or phrase to search for, such as 'skill', 'github', 'task', 'web', or a capability name"
                    },
                    "limit": {
                        "type": "number",
                        "description": "Maximum number of matches to return (default 20, max 50)"
                    }
                },
                "required": ["query"]
            }),
        },
        ToolDef {
            name: "ToolSuggest".into(),
            description: "Suggest the most relevant tools or skills for a described intent. Use this before acting when the request maps to an unfamiliar capability.".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "intent": {
                        "type": "string",
                        "description": "Short description of what you want to accomplish"
                    },
                    "limit": {
                        "type": "number",
                        "description": "Maximum number of suggestions to return (default 8, max 20)"
                    }
                },
                "required": ["intent"]
            }),
        },
    ]
}

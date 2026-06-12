use jfc_provider::ToolDef;

pub fn review_tool_defs() -> Vec<ToolDef> {
    vec![
        submit_plan_def(),
        add_review_comment_def(),
        suggest_commit_message_def(),
    ]
}

fn submit_plan_def() -> ToolDef {
    ToolDef {
        name: "SubmitPlan".into(),
        description: "Submit a structured implementation plan and stop. Use \
            this when operating in a planning/review workflow where the user or \
            harness expects a plan artifact rather than immediate edits. The \
            plan should name the files you expect to touch, the changes you \
            will make, validation commands, and notable risks."
            .into(),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "short_name": {
                    "type": "string",
                    "description": "Short stable plan label, 2-6 words."
                },
                "summary": {
                    "type": "string",
                    "description": "One sentence summary of the planned work."
                },
                "plan": {
                    "type": "string",
                    "description": "Markdown plan with files, changes, validation, and risks."
                }
            },
            "required": ["short_name", "summary", "plan"]
        }),
    }
}

fn add_review_comment_def() -> ToolDef {
    ToolDef {
        name: "AddReviewComment".into(),
        description: "Record one actionable code-review comment. Use this only \
            for concrete defects that the author should fix. Do not use it for \
            praise, vague concerns, or style-only nits. The line range must be \
            tight and cover 30 lines or fewer. `file_path` may be absolute or \
            relative to the current working directory; it is normalized before \
            persistence."
            .into(),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "file_path": {
                    "type": "string",
                    "description": "Absolute path or path relative to the current working directory."
                },
                "start_line": {
                    "type": "integer",
                    "minimum": 1,
                    "description": "First affected line."
                },
                "end_line": {
                    "type": "integer",
                    "minimum": 1,
                    "description": "Last affected line, no more than 30 lines after start_line."
                },
                "text": {
                    "type": "string",
                    "description": "Actionable review comment describing the defect and required fix."
                }
            },
            "required": ["file_path", "start_line", "end_line", "text"]
        }),
    }
}

fn suggest_commit_message_def() -> ToolDef {
    ToolDef {
        name: "SuggestCommitMessage".into(),
        description: "Persist one concise commit-message suggestion after \
            inspecting the actual diff. Prefer conventional-commit style when \
            it fits. Do not invent changes that are not in the diff."
            .into(),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "message": {
                    "type": "string",
                    "description": "Suggested commit message."
                },
                "scope": {
                    "type": "string",
                    "description": "Optional subsystem or crate scope."
                }
            },
            "required": ["message"]
        }),
    }
}

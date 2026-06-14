//! Tool definitions for the persistent plan store (PlanCreate/List/Show/
//! Advance/Archive/Materialize). These were implemented and dispatched but
//! never advertised, so the model could only reach plans via the /plan
//! slash command — the agentic path was dead.

use jfc_provider::ToolDef;

pub fn plan_tool_defs() -> Vec<ToolDef> {
    vec![
        ToolDef {
            name: "PlanCreate".into(),
            description: "Create a persistent plan document (markdown body, slug-addressed). \
                          Plans survive sessions and can be advanced phase-by-phase and \
                          materialized into queue tasks."
                .into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "title": { "type": "string", "description": "Plan title (slug is derived from it)" },
                    "body": { "type": "string", "description": "Markdown plan body (phases, steps, risks)" }
                },
                "required": ["title"]
            }),
        },
        ToolDef {
            name: "PlanList".into(),
            description: "List persistent plans, optionally filtered by status \
                          (active/completed/archived)."
                .into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "status": { "type": "string", "description": "Optional status filter: active, completed, or archived" }
                },
                "required": []
            }),
        },
        ToolDef {
            name: "PlanShow".into(),
            description: "Show a persistent plan's full body and progress by slug.".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "slug": { "type": "string", "description": "Plan slug from PlanList/PlanCreate" }
                },
                "required": ["slug"]
            }),
        },
        ToolDef {
            name: "PlanAdvance".into(),
            description: "Advance a plan to its next phase, recording a summary of what was \
                          completed in the current phase."
                .into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "slug": { "type": "string", "description": "Plan slug" },
                    "summary": { "type": "string", "description": "What was completed in the current phase" }
                },
                "required": ["slug", "summary"]
            }),
        },
        ToolDef {
            name: "PlanArchive".into(),
            description: "Archive a plan (terminal state), with an optional reason.".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "slug": { "type": "string", "description": "Plan slug" },
                    "reason": { "type": "string", "description": "Optional reason for archiving" }
                },
                "required": ["slug"]
            }),
        },
        ToolDef {
            name: "PlanMaterialize".into(),
            description: "Materialize a plan's phases into queue tasks (TaskCreate records) \
                          linked back to the plan, so completing tasks advances the plan."
                .into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "slug": { "type": "string", "description": "Plan slug" }
                },
                "required": ["slug"]
            }),
        },
    ]
}

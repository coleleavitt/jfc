use jfc_provider::ToolDef;

pub fn task_tool_defs() -> Vec<ToolDef> {
    vec![
        ToolDef {
            name: "TaskCreate".into(),
            description: "Create a new task to track work. Returns the created task with its id.".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "subject": {
                        "type": "string",
                        "description": "Short title for the task"
                    },
                    "description": {
                        "type": "string",
                        "description": "Detailed description of what needs to be done"
                    },
                    "active_form": {
                        "type": "string",
                        "description": "Present-tense text shown while task is in progress (e.g. 'Fixing auth bug')"
                    },
                    "blocked_by": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Task ids that must complete before this task can start"
                    },
                    "acceptance_criteria": {
                        "type": "string",
                        "description": "Mechanistic pass/fail criteria for verifying task completion (e.g. 'cargo test --lib foo passes')"
                    },
                    "verification_command": {
                        "type": "string",
                        "description": "Shell command to confirm done-ness (e.g. 'cargo test -p jfc-ui')"
                    },
                    "risk": {
                        "type": "string",
                        "enum": ["low", "medium", "high"],
                        "description": "Risk level. High-risk tasks require user approval before auto-execution."
                    },
                    "parent_id": {
                        "type": "string",
                        "description": "Parent task id for hierarchical task trees"
                    },
                    "kind": {
                        "type": "string",
                        "enum": ["milestone", "task", "check", "decision"],
                        "description": "Task kind: milestone (grouping), task (work unit), check (verification), decision (requires input)"
                    }
                },
                "required": ["subject", "description"]
            }),
        },
        ToolDef {
            name: "TaskUpdate".into(),
            description: "Update an existing task's status, subject, description, or owner.".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "task_id": {
                        "type": "string",
                        "description": "The task id to update (e.g. 't1')"
                    },
                    "status": {
                        "type": "string",
                        "enum": ["pending", "in_progress", "completed", "deleted"],
                        "description": "New status for the task"
                    },
                    "subject": {
                        "type": "string",
                        "description": "New subject/title"
                    },
                    "description": {
                        "type": "string",
                        "description": "New description"
                    },
                    "owner": {
                        "type": "string",
                        "description": "Assign task to a teammate name"
                    },
                    "acceptance_criteria": {
                        "type": "string",
                        "description": "Mechanistic pass/fail criteria for verifying task completion"
                    },
                    "verification_command": {
                        "type": "string",
                        "description": "Shell command to confirm done-ness"
                    },
                    "risk": {
                        "type": "string",
                        "enum": ["low", "medium", "high"],
                        "description": "Risk level"
                    },
                    "parent_id": {
                        "type": "string",
                        "description": "Parent task id for hierarchical task trees"
                    },
                    "kind": {
                        "type": "string",
                        "enum": ["milestone", "task", "check", "decision"],
                        "description": "Task kind"
                    }
                },
                "required": ["task_id"]
            }),
        },
        ToolDef {
            name: "TaskList".into(),
            description: "List all tasks, optionally filtered by status or owner.".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "status_filter": {
                        "type": "string",
                        "enum": ["pending", "in_progress", "completed"],
                        "description": "Only return tasks with this status"
                    },
                    "owner_filter": {
                        "type": "string",
                        "description": "Only return tasks assigned to this owner"
                    }
                },
                "required": []
            }),
        },
        ToolDef {
            name: "TaskDone".into(),
            description: "Mark a task as completed.".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "task_id": {
                        "type": "string",
                        "description": "The task id to mark done (e.g. 't1')"
                    }
                },
                "required": ["task_id"]
            }),
        },
        ToolDef {
            name: "TaskStop".into(),
            description: "Stop a running background task/agent by its task ID. The task will be cancelled and its resources released.".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "task_id": {
                        "type": "string",
                        "description": "The background task id to stop (e.g. 'tooluse_abc123')"
                    }
                },
                "required": ["task_id"]
            }),
        },
        ToolDef {
            name: "TaskGet".into(),
            description: "Retrieve a task by ID.".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "task_id": {
                        "type": "string",
                        "description": "The task id to retrieve (e.g. 't1')"
                    }
                },
                "required": ["task_id"]
            }),
        },
        ToolDef {
            name: "TaskValidate".into(),
            description: "Validate the task graph for health issues. Returns a structured report \
                identifying orphaned tasks, tasks blocked forever, tasks without verification \
                criteria, duplicate subjects, and parallelization opportunities. Use after \
                creating a batch of tasks to check plan soundness.".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {},
                "required": []
            }),
        },
    ]
}

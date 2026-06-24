use jfc_provider::ToolDef;

pub(super) fn task_tool_defs() -> Vec<ToolDef> {
    vec![ToolDef {
        name: "Task".into(),
        description: "Launch a subagent to handle complex, multi-step tasks. Use proactively for broad codebase exploration, multi-angle audits, verification, planning, or work that would take more than a few direct tool calls. Fire multiple Task calls in parallel when the angles are independent. Subagents have isolated context: include the objective, relevant findings, exact files/symbols, constraints, expected output shape, required evidence, and any scoped allowed_tools/disallowed_tools in the prompt. For downstream synthesis, ask for summary, findings, evidence/source locations, attempted steps, errors, partial results, coverage gaps, and next actions. Add schema when you need the subagent to return validated StructuredOutput. With name + team_name, spawns a persistent teammate addressable via SendMessage.".into(),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "description": {
                    "type": "string",
                    "description": "Short label for the task (3-5 words)"
                },
                "prompt": {
                    "type": "string",
                    "description": "Full prompt for the sub-agent"
                },
                "subagent_type": {
                    "type": "string",
                    "description": "Agent type to use (e.g. 'build', 'explore')"
                },
                "category": {
                    "type": "string",
                    "description": "Task category, used to auto-select a cost-appropriate model tier when no explicit `model` is given: read-only/mapping work (explore, search, summarize, classify, lint) → a fast cheap model; hard work (architecture, design, security, audit, debug, refactor) → the heavy model; standard implementation (build, code, test, fix) → the balanced model. An explicit `model` always overrides this."
                },
                "run_in_background": {
                    "type": "boolean",
                    "description": "When true, returns immediately with a task_id and runs asynchronously"
                },
                "model": {
                    "type": "string",
                    "description": "Optional model override in 'provider/model' format"
                },
                "effort": {
                    "type": "string",
                    "description": "Optional reasoning effort override (low/medium/high/xhigh/max)"
                },
                "name": {
                    "type": "string",
                    "description": "Name for the spawned agent. Makes it addressable via SendMessage({to: name}) while running."
                },
                "team_name": {
                    "type": "string",
                    "description": "Team name for spawning. Uses current team context if omitted."
                },
                "mode": {
                    "type": "string",
                    "description": "Permission mode for spawned teammate (e.g., 'plan' to require plan approval)."
                },
                "isolation": {
                    "type": "string",
                    "enum": ["worktree"],
                    "description": "Isolation mode. 'worktree' creates a temporary git worktree."
                },
                "parent_task_id": {
                    "type": "string",
                    "description": "Queued task id (e.g. 't3') this delegation fulfils. When set, the runtime auto-marks that task in_progress on spawn, completed on success, and failed on error — so you don't need a separate TaskUpdate/TaskDone call for the delegated work."
                },
                "schema": {
                    "type": "object",
                    "description": "Optional JSON Schema for the subagent's final output. When set, the subagent receives StructuredOutput and must finish by calling it with an object that validates against this schema. Use nullable or optional fields for absent facts instead of forcing fabrication.",
                    "additionalProperties": true
                },
                "allowed_tools": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Optional per-call tool allowlist for the subagent. If the selected subagent type already has an allowlist, this narrows it; it never grants tools the subagent type disallows."
                },
                "disallowed_tools": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Optional per-call denylist for the subagent. These tools are removed in addition to any tools denied by the selected subagent type."
                },
                "cwd": {
                    "type": "string",
                    "description": "Optional working directory for the spawned subagent. When set, the agent starts in this directory instead of the parent's cwd. Useful for pointing a subagent at a git worktree or a different project directory."
                }
            },
            "required": ["description", "prompt", "run_in_background"]
        }),
    }]
}

pub(super) fn team_tool_defs() -> Vec<ToolDef> {
    vec![
        ToolDef {
            name: "TeamCreate".into(),
            description: "Create a new team for coordinating multiple persistent agents. Use before spawning several named Task teammates for broad, parallel, or long-running work. Teams have a 1:1 correspondence with task lists.".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "team_name": {
                        "type": "string",
                        "description": "Name for the new team to create."
                    },
                    "description": {
                        "type": "string",
                        "description": "Team description/purpose."
                    }
                },
                "required": ["team_name"]
            }),
        },
        ToolDef {
            name: "TeamDelete".into(),
            description: "Clean up team and task directories when the swarm is complete. Must terminate all teammates first.".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {},
                "required": []
            }),
        },
        ToolDef {
            name: "SendMessage".into(),
            description: "Send a message to another agent. Your plain text output is NOT visible to other agents — to communicate, you MUST call this tool.".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "to": {
                        "type": "string",
                        "description": "Recipient: teammate name"
                    },
                    "summary": {
                        "type": "string",
                        "description": "A 5-10 word summary shown as a preview in the UI"
                    },
                    "message": {
                        "description": "Plain text message content or structured protocol message",
                        "oneOf": [
                            { "type": "string" },
                            { "type": "object" }
                        ]
                    }
                },
                "required": ["to", "message"]
            }),
        },
        ToolDef {
            name: "TeamMemberMode".into(),
            description: "Change a teammate's permission mode at runtime. Use to promote (e.g. plan → default) once a teammate has earned trust, or demote (default → plan) for high-stakes work. Modes: plan, default, acceptEdits, bypassPermissions.".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "member_name": {
                        "type": "string",
                        "description": "Name of the teammate to update."
                    },
                    "mode": {
                        "type": "string",
                        "description": "New permission mode: plan | default | acceptEdits | bypassPermissions",
                        "enum": ["plan", "default", "acceptEdits", "bypassPermissions"]
                    }
                },
                "required": ["member_name", "mode"]
            }),
        },
    ]
}

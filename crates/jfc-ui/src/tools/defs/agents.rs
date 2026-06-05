use jfc_provider::ToolDef;

pub fn agent_tool_defs() -> Vec<ToolDef> {
    vec![
        ToolDef {
            name: "Skill".into(),
            description: "Invoke a registered skill by name. The skill's body is rendered as guidance and acted upon. Pass `args` as additional context.".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "name": {
                        "type": "string",
                        "description": "The registered skill name (matches the `name` frontmatter or filename stem under `.claude/skills/`)"
                    },
                    "skill": {
                        "type": "string",
                        "description": "Alias for `name`, accepted for Claude Code compatibility"
                    },
                    "args": {
                        "type": "string",
                        "description": "Optional additional context appended to the skill body"
                    }
                },
                "required": ["name"]
            }),
        },
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
        ToolDef {
            name: "Task".into(),
            description: "Launch a new agent to handle complex, multi-step tasks. Each agent type has specific capabilities. With name + team_name, spawns a persistent teammate addressable via SendMessage.".into(),
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
                        "description": "Task category for model selection"
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
                    }
                },
                "required": ["description", "prompt", "run_in_background"]
            }),
        },
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
        ToolDef {
            name: "TeamCreate".into(),
            description: "Create a new team for coordinating multiple agents. Teams have a 1:1 correspondence with task lists.".into(),
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
        ToolDef {
            name: "Advisor".into(),
            description: "Consult JFC's configured local/client-side advisor model for guidance. \
                Takes NO parameters — JFC snapshots your conversation and sends it through the \
                configured advisor provider/model as a normal local tool call. The advisor sees \
                the task, every tool call you've made, every result you've seen.\n\n\
                Call advisor BEFORE substantive work — before writing, before committing to an \
                interpretation, before building on an assumption. Also call when stuck, when \
                considering a change of approach, or when you believe the task is complete.\n\n\
                Give the advice serious weight. If you follow a step and it fails empirically, \
                adapt. Surface conflicts in another advisor call rather than silently switching.".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {},
                "required": []
            }),
        },
        ToolDef {
            name: "StructuredOutput".into(),
            description: "Provide structured output matching the required JSON schema. \
                This tool is only available when the agent was spawned with a `schema` \
                parameter. Call it with a JSON object that validates against the schema. \
                On success, the result is returned to the parent agent as validated data.".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "additionalProperties": true,
                "description": "JSON object matching the schema specified by the parent agent"
            }),
        },
    ]
}

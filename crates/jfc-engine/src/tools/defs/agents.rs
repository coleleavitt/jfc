use jfc_provider::ToolDef;

pub fn agent_tool_defs() -> Vec<ToolDef> {
    vec![
        ToolDef {
            name: "Skill".into(),
            description: "Invoke a user-invocable registered skill by name. The skill body is rendered with runtime placeholders, attached package files are surfaced as readable paths, and `context: fork` skills run through the subagent path when invoked as slash commands. Pass `args` as additional context.".into(),
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
        ToolDef {
            name: "Research".into(),
            description: "Run an agentic deep-research pass on a question: a model plans \
                and REFORMULATES each next sub-query from the evidence gathered so far \
                (read → decide → search → repeat), routing sub-queries to the best source \
                — general web, the local codebase (ripgrep), or specialised indexes \
                (arXiv, OpenAlex, Crossref, PubMed, Semantic Scholar, DOAJ, CORE, a named \
                university via `uni:`, Wikipedia, etc.) — then a model synthesises the \
                gathered evidence into one CITED answer. Mirrors claude.ai/Perplexity \
                deep research. Runs out-of-band — it does NOT consume the main \
                conversation's tools and returns a self-contained report.\n\n\
                Use when a question needs current/external or academic information across \
                multiple angles (background + latest developments + mechanism + \
                criticism), or wants both web and repo evidence — not a single lookup. \
                For a one-shot fact, use WebSearch instead. Set `export` to also write a \
                durable markdown+json artifact.".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "question": {
                        "type": "string",
                        "description": "The research question to investigate."
                    },
                    "export": {
                        "type": "boolean",
                        "description": "When true, also write the report to a durable \
                            artifact file (markdown + json). Defaults to false."
                    }
                },
                "required": ["question"]
            }),
        },
        ToolDef {
            name: "Council".into(),
            description: "Convene a model council: fan a question out to several models in \
                parallel, then an arbiter model synthesises their independent answers into \
                one consolidated reply that surfaces agreement (higher confidence) and \
                disagreement (presents the options). Mirrors Perplexity's COUNCIL_RESEARCH \
                / Model Council flow. Runs out-of-band like the advisor.\n\n\
                Use for high-stakes or contested questions where cross-checking multiple \
                models is worth the extra cost — architecture decisions, ambiguous \
                trade-offs, correctness reviews. For a quick second opinion, use the \
                advisor instead.".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "question": {
                        "type": "string",
                        "description": "The question to put to the council."
                    },
                    "models": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Optional explicit member model ids. When omitted, \
                            the council uses the active model plus the local advisor model."
                    }
                },
                "required": ["question"]
            }),
        },
        ToolDef {
            name: "AskModel".into(),
            description: "Ask a specific model a one-shot question mid-turn and get its reply \
                threaded back into this conversation. Unlike Council (parallel fan-out + \
                arbiter) this is a single direct call to ONE model — use it to pull a \
                different model into the current turn: e.g. ask `gpt-5.5` for its take while \
                you (Claude) keep driving, then react to its answer. The reply returns as \
                this tool's result, so you can challenge it, build on it, or ask a follow-up \
                with another AskModel call. Runs out-of-band (no tools, prose only) like the \
                advisor.\n\n\
                Use for cross-model second opinions, comparing how a different model family \
                reasons about the same prompt, or interleaving two models within one task. \
                The `model` is resolved against the configured providers (e.g. `gpt-5.5`, \
                `openai/gpt-5.5`, `claude-opus-4-7`).".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "model": {
                        "type": "string",
                        "description": "Model id to ask, resolved against configured providers \
                            (e.g. `gpt-5.5`, `openai/gpt-5.5`, `claude-opus-4-7`)."
                    },
                    "prompt": {
                        "type": "string",
                        "description": "The prompt / question to send to that model."
                    },
                    "system": {
                        "type": "string",
                        "description": "Optional system prompt to steer the asked model's role."
                    }
                },
                "required": ["model", "prompt"]
            }),
        },
    ]
}

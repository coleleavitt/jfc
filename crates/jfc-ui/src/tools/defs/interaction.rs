use jfc_provider::ToolDef;

/// User interaction, web access, plan mode, and worktree tools.
pub fn interaction_tool_defs() -> Vec<ToolDef> {
    vec![
        ask_user_question_def(),
        web_fetch_def(),
        web_search_def(),
        exit_plan_mode_def(),
        enter_plan_mode_def(),
        enter_worktree_def(),
        exit_worktree_def(),
    ]
}

fn ask_user_question_def() -> ToolDef {
    ToolDef {
        name: "AskUserQuestion".into(),
        description: "Ask the user a multi-choice question mid-turn to gather \
            preferences, clarify ambiguity, or offer choices. Use sparingly — \
            only when context genuinely requires user input. Each option is \
            a `{label, description}` object. Returns the user's selected \
            label(s) as the tool result.".into(),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "question": { "type": "string" },
                "options": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "label": { "type": "string" },
                            "description": { "type": "string" }
                        },
                        "required": ["label"]
                    },
                    "minItems": 2, "maxItems": 4
                },
                "multi_select": { "type": "boolean", "default": false }
            },
            "required": ["question", "options"]
        }),
    }
}

fn web_fetch_def() -> ToolDef {
    ToolDef {
        name: "WebFetch".into(),
        description: "Fetch a URL and return its contents (HTML extracted to \
            text, JSON pretty-printed, plain text passed through). Optional \
            `prompt` argument tells the model what aspect of the page to \
            focus on; useful for long pages where you want a summary rather \
            than the full body.".into(),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "url": { "type": "string", "description": "Absolute URL (https:// preferred)" },
                "prompt": { "type": "string", "description": "Optional focus / summary instruction" }
            },
            "required": ["url"]
        }),
    }
}

fn web_search_def() -> ToolDef {
    ToolDef {
        name: "WebSearch".into(),
        description: "Search the web for `query`. Returns a ranked list of \
            results with title, URL, and snippet. Combine with `WebFetch` \
            to read promising hits. \
            Prefix the query to select a backend: \
            `arxiv: <query>` searches arXiv papers (free, no key needed); \
            `scholar: <query>` searches Semantic Scholar (optional API key, falls back to BFF); \
            `papers: <query>` queries arXiv + Semantic Scholar in parallel and \
            deduplicates results by arXiv ID / DOI / title; \
            no prefix uses Google Custom Search Engine.".into(),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "query": { "type": "string" },
                "max_results": { "type": "integer", "minimum": 1, "maximum": 20, "default": 5 }
            },
            "required": ["query"]
        }),
    }
}

fn exit_plan_mode_def() -> ToolDef {
    ToolDef {
        name: "ExitPlanMode".into(),
        description: "Surface a finalized plan to the user and request \
            permission to leave plan mode. Use this when you've gathered \
            enough context (read files, run grep / git log, etc.) and \
            are ready to execute destructive edits. The `plan` argument \
            must be a markdown summary of: (1) the change you intend to \
            make, (2) the files you'll touch, (3) anything risky / \
            irreversible. After this tool returns success, you may \
            proceed with Write/Edit/destructive Bash calls — the user \
            has approved by virtue of you reaching this point. Mirrors \
            Claude Code v2.1.132's ExitPlanMode tool contract.".into(),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "plan": {
                    "type": "string",
                    "description": "Markdown-formatted plan describing the work you're about to undertake."
                }
            },
            "required": ["plan"]
        }),
    }
}

fn enter_plan_mode_def() -> ToolDef {
    ToolDef {
        name: "EnterPlanMode".into(),
        description: "Use this tool proactively when you're about to start a \
            non-trivial implementation task. Getting user sign-off on your \
            approach before writing code prevents wasted effort and ensures \
            alignment.\n\n\
            ## When to Use This Tool\n\n\
            **Prefer using EnterPlanMode** for implementation tasks unless \
            they're simple. Use it when ANY of these conditions apply:\n\n\
            1. **New Feature Implementation**: Adding meaningful new functionality\n\
            2. **Multiple Valid Approaches**: The task can be solved several different ways\n\
            3. **Code Modifications**: Changes affecting existing behavior or structure\n\
            4. **Architectural Decisions**: Choosing between patterns or technologies\n\
            5. **Multi-File Changes**: Task will likely touch more than 2-3 files\n\
            6. **Unclear Requirements**: Need to explore before understanding full scope\n\
            7. **User Preferences Matter**: Implementation could go multiple ways\n\n\
            ## When NOT to Use This Tool\n\n\
            Only skip EnterPlanMode for simple tasks:\n\
            - Single-line or few-line fixes (typos, obvious bugs, small tweaks)\n\
            - Adding a single function with clear requirements\n\
            - Tasks where the user has given very specific, detailed instructions\n\
            - Pure research/exploration tasks (use the `Explore` agent instead)\n\n\
            ## Important Notes\n\n\
            - This tool REQUIRES user approval — they must consent to entering plan mode\n\
            - In plan mode, you can read freely but cannot write files or run shell commands\n\
            - When ready to execute, use `ExitPlanMode` with a finalized plan summary\n\
            - If unsure whether to use it, err on the side of planning — alignment upfront beats rework\n\
            - Pair with a `reason` so the user understands why analysis-only mode was requested.".into(),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "reason": {
                    "type": "string",
                    "description": "Why plan mode is needed (visible to the user)."
                }
            },
            "required": ["reason"]
        }),
    }
}

fn enter_worktree_def() -> ToolDef {
    ToolDef {
        name: "EnterWorktree".into(),
        description: "Create (if needed) and switch into a git worktree at \
            `.jfc-worktrees/<name>` checking out branch `jfc/<name>` (or a \
            caller-provided branch). Subsequent tool calls run in that \
            worktree's directory until `ExitWorktree` is invoked.".into(),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "name": {
                    "type": "string",
                    "description": "Worktree name. [A-Za-z0-9_-], <= 64 chars."
                },
                "branch": {
                    "type": "string",
                    "description": "Optional branch to check out (defaults to `jfc/<name>`)."
                }
            },
            "required": ["name"]
        }),
    }
}

fn exit_worktree_def() -> ToolDef {
    ToolDef {
        name: "ExitWorktree".into(),
        description: "Leave the current worktree. The worktree is left intact \
            on disk; only the agent's effective cwd resets to the repo root.".into(),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {},
            "required": []
        }),
    }
}

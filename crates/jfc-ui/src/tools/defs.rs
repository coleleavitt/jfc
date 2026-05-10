use crate::provider::ToolDef;

pub fn all_tool_defs() -> Vec<ToolDef> {
    vec![
        ToolDef {
            name: "Bash".into(),
            description: "Executes a given bash command in a persistent shell session with optional timeout. Use for running commands, scripts, and terminal operations.".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "command": {
                        "type": "string",
                        "description": "The command to execute"
                    },
                    "timeout": {
                        "type": "number",
                        "description": "Optional timeout in milliseconds (max 600000)"
                    },
                    "description": {
                        "type": "string",
                        "description": "Clear, concise description of what this command does"
                    }
                },
                "required": ["command"]
            }),
        },
        ToolDef {
            name: "Read".into(),
            description: "Read a file or directory from the local filesystem. Returns file contents with line numbers prefixed.".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "file_path": {
                        "type": "string",
                        "description": "The absolute path to the file or directory to read"
                    },
                    "offset": {
                        "type": "number",
                        "description": "Line number to start reading from (1-indexed)"
                    },
                    "limit": {
                        "type": "number",
                        "description": "Maximum number of lines to read (defaults to 2000)"
                    }
                },
                "required": ["file_path"]
            }),
        },
        ToolDef {
            name: "Write".into(),
            description: "Write a file to the local filesystem. Overwrites existing file if present.".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "file_path": {
                        "type": "string",
                        "description": "The absolute path to the file to write"
                    },
                    "content": {
                        "type": "string",
                        "description": "The content to write to the file"
                    }
                },
                "required": ["file_path", "content"]
            }),
        },
        ToolDef {
            name: "Edit".into(),
            description: "Performs exact string replacements in a file. Use Read first to verify the exact content before editing.".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "file_path": {
                        "type": "string",
                        "description": "The absolute path to the file to modify"
                    },
                    "old_string": {
                        "type": "string",
                        "description": "The text to replace (must match exactly, including whitespace)"
                    },
                    "new_string": {
                        "type": "string",
                        "description": "The replacement text"
                    },
                    "replace_all": {
                        "type": "boolean",
                        "description": "Replace all occurrences (default false)"
                    }
                },
                "required": ["file_path", "old_string", "new_string"]
            }),
        },
        ToolDef {
            name: "Glob".into(),
            description: "Fast file pattern matching. Returns matching file paths sorted by modification time.".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "pattern": {
                        "type": "string",
                        "description": "The glob pattern to match files against"
                    },
                    "path": {
                        "type": "string",
                        "description": "The directory to search in. Defaults to current working directory if omitted."
                    }
                },
                "required": ["pattern"]
            }),
        },
        ToolDef {
            name: "Grep".into(),
            description: "Fast content search using ripgrep. Searches file contents using regular expressions.".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "pattern": {
                        "type": "string",
                        "description": "The regex pattern to search for in file contents"
                    },
                    "path": {
                        "type": "string",
                        "description": "File or directory to search in. Defaults to current working directory."
                    },
                    "glob": {
                        "type": "string",
                        "description": "File pattern filter (e.g. '*.ts', '*.{ts,tsx}')"
                    },
                    "output_mode": {
                        "type": "string",
                        "enum": ["content", "files_with_matches", "count"],
                        "description": "Output mode: content shows matching lines, files_with_matches shows file paths, count shows match counts"
                    }
                },
                "required": ["pattern"]
            }),
        },
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
            name: "Skill".into(),
            description: "Invoke a registered skill by name. The skill's body is rendered as guidance and acted upon. Pass `args` as additional context.".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "name": {
                        "type": "string",
                        "description": "The registered skill name (matches the `name` frontmatter or filename stem under `.claude/skills/`)"
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
            name: "graph_query".into(),
            description: "Query the project's code graph using a pipe-based DSL with set algebra and path patterns. \
                Surgically find callers, callees, type usages, or trace data taint without loading whole files. \
                Pipe operators (chain with `|`): `fn(\"name\")` selects functions by substring; \
                `type(\"name\")` selects struct/enum/trait; `callers` / `callees` walk Calls edges; \
                `depth N` limits traversal (1-3 narrow, 5+ full reach); \
                `filter kind=Function|Struct|Enum|Module|Trait` filters by node kind; \
                `show fields|signature|body` controls projection; `taint \"var\"` traces a parameter \
                through call chains; `preconditions` walks callers backward and surfaces enclosing \
                if/match/while predicates (\"what must have been true to reach X?\"); \
                `since N` keeps only nodes whose `last_modified_revision >= N`. \
                Set algebra (combine queries): `A union B`, `A intersect B`, `A \\ B` (difference). \
                Path patterns: `path A -> B` (shortest), `paths A -> B depth N` (all simple, bounded), \
                with `where intermediate kind=K` and `via EdgeKind` qualifiers. \
                Entrypoints: `entrypoints` or `entrypoints kind=Main|PublicApi|Test|Bench|FfiExport`. \
                Examples: \
                `fn(\"execute_tool\") | callees | depth 2`; \
                `type(\"Config\") | callers`; \
                `fn(\"parse\") | taint \"input\" | depth 5`; \
                `fn(\"a\") union fn(\"b\")`; \
                `path fn(\"login\") -> fn(\"db_write\")`; \
                `paths fn(\"handler\") -> fn(\"unsafe_op\") via Calls depth 8`; \
                `entrypoints kind=PublicApi`; \
                `(fn(\"foo\") | callers) \\ fn(\"test_\") since 42`. \
                Cycles auto-detected (mutual recursion terminates). Output is token-budgeted; \
                truncated results report \"Showing N/M nodes\". The output ends with a \
                `--- handles ---` block of `kind:qualified_name` strings (e.g. `fn:crate::foo`) \
                so you can chain queries directly. Set `include_handles=false` to suppress.".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "DSL query string. Examples: `fn(\"foo\") | callees | depth 2`, `path fn(\"a\") -> fn(\"b\")`, `entrypoints kind=Main`, `fn(\"x\") union fn(\"y\")`."
                    },
                    "max_tokens": {
                        "type": "number",
                        "description": "Optional token budget (default 4000). Output truncates to fit."
                    },
                    "include_handles": {
                        "type": "boolean",
                        "description": "Append a `--- handles ---` footer of structured handles for chaining (default true). Set false when only summary text is needed."
                    }
                },
                "required": ["query"]
            }),
        },
        ToolDef {
            name: "run_coverage".into(),
            description: "Run cargo llvm-cov (or parse an existing lcov.info), annotate every \
                Function node in the code graph with hit counts, and return a summary of \
                tested vs untested functions. After this tool runs, use graph_query with the \
                `untested` operator to find uncovered code (e.g. `entrypoints kind=PublicApi | untested`). \
                Also enables the `possible_types` operator which propagates subtype sets through \
                the call graph — use `fn(\"handler\") | possible_types` to see which concrete \
                types can flow into a function.".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "lcov_path": {
                        "type": "string",
                        "description": "Optional path to an existing lcov.info file. If omitted, runs `cargo llvm-cov --lcov` to generate one."
                    },
                    "include_untested_list": {
                        "type": "boolean",
                        "description": "Whether to include a list of untested function names in the output. Default true."
                    }
                },
                "required": []
            }),
        },
        ToolDef {
            name: "symbol_edit".into(),
            description: "Edit a function/struct/etc. by *symbol handle* instead of \
                file:line. Handles look like `fn:module::name` or `struct:Name` and \
                are returned by `graph_query`. The tool resolves the handle to its \
                exact span and replaces it atomically. With `validate=true`, runs \
                signature-compatibility checks against all callers first and refuses \
                edits that would break call sites. Prefer this over Edit when \
                changing signatures, since it surfaces affected callers automatically. \
                If the handle isn't found, the error suggests up to 5 fuzzy matches.".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "handle": {
                        "type": "string",
                        "description": "Symbol handle from graph_query, e.g. `fn:tools::execute_task`."
                    },
                    "new_content": {
                        "type": "string",
                        "description": "Full replacement text for the symbol's span (function body, struct decl, etc.)"
                    },
                    "validate": {
                        "type": "boolean",
                        "description": "When true, blocks edits that would break callers and computes the cascade plan. Default false."
                    },
                    "dispatch_cascade": {
                        "type": "boolean",
                        "description": "When true (and validate=true), the cascade plan is auto-queued into the project's task list — one entry per affected file, tagged kind=\"cascade\". Run /cascade or use TaskList to view, then dispatch Task tool sub-agents per queued item. Default false."
                    }
                },
                "required": ["handle", "new_content"]
            }),
        },
        ToolDef {
            name: "post_bounty".into(),
            description: "Register a coding-task bounty in the agent \
                economy market. By default this only registers — solvers \
                and validators DO NOT run until you also call \
                `run_bounty(bounty_id)`, OR pass `auto_dispatch: true` \
                here to register and run in one shot. Once dispatched, \
                multiple solver agents compete (real LLM sub-calls in \
                parallel git worktrees), validators adversarially challenge \
                each surviving solution (sealed sessions, no peer \
                pressure), and only solutions surviving validation are \
                ranked + paid. Budget is tracked as real LLM tokens; the \
                orchestrator's CFO layer gates spending so the cycle \
                can't exceed it. Use post+run when you want competitive, \
                cross-validated output instead of a single-shot edit. \
                Inspect state via `market_status` or /market.".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "description": {
                        "type": "string",
                        "description": "What the task is. Concrete and self-contained — solvers won't see the surrounding conversation."
                    },
                    "budget": {
                        "type": "number",
                        "description": "Token budget for the entire bounty (all solvers + validators combined). Hard cap, enforced at runtime."
                    },
                    "acceptance_criteria": {
                        "type": "string",
                        "description": "Mechanistic pass/fail criteria — preferably commands like `cargo test --lib foo` that produce binary outcomes. Avoid soft criteria; agents will game them."
                    },
                    "max_solvers": {
                        "type": "number",
                        "description": "Optional cap on competing solvers (default from charter, typically 3). Range 1-5."
                    }
                },
                "required": ["description", "budget", "acceptance_criteria"]
            }),
        },
        ToolDef {
            name: "run_bounty".into(),
            description: "Drive an already-posted Open bounty through the \
                full Solve→Validate→Settle cycle. Pair this with \
                `post_bounty` (auto_dispatch=false) when you want to \
                register the bounty first and dispatch later — the post \
                step is cheap; this is the expensive step that actually \
                spawns solver + validator subagent LLM calls. Returns the \
                settlement (winner, total cost, payout count) when the \
                cycle completes. Errors fast if the bounty is not in \
                Open state or the provider isn't registered with the \
                tool layer.".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "bounty_id": {
                        "type": "string",
                        "description": "The bounty ID returned by post_bounty (e.g. `bounty_a1a8…`)."
                    },
                    "max_solvers": {
                        "type": "number",
                        "description": "Optional override for the number of competing solvers (1-5, default 2)."
                    }
                },
                "required": ["bounty_id"]
            }),
        },
        ToolDef {
            name: "market_status".into(),
            description: "Read the agent economy's current state. Returns \
                bounty count, spend, composite health score (efficiency × \
                fairness × trust × budget; <0.3 = CRITICAL), and any agents \
                flagged for collusion / rubber-stamping / griefing. \
                Optionally pass `bounty_id` to get the specific bounty's \
                phase (Posting / Bidding / Executing / Validating / \
                Settling / Complete).".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "bounty_id": {
                        "type": "string",
                        "description": "Optional bounty ID to drill into. Omit for global market summary."
                    }
                }
            }),
        },
        ToolDef {
            name: "MultiEdit".into(),
            description: "Apply multiple edits to a single file in one tool call. \
                `edits` is an array of `{old_string, new_string, replace_all?}` \
                objects, applied in order — each edit sees the previous edit's \
                output as input. Saves a tool round-trip when the model needs \
                several rewrites in the same source file.".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "file_path": { "type": "string" },
                    "edits": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {
                                "old_string": { "type": "string" },
                                "new_string": { "type": "string" },
                                "replace_all": { "type": "boolean", "default": false }
                            },
                            "required": ["old_string", "new_string"]
                        }
                    }
                },
                "required": ["file_path", "edits"]
            }),
        },
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
        },
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
        },
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
        },
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
        },
        // ── daemon-driven scheduling tools ─────────────────────────────────
        ToolDef {
            name: "CronCreate".into(),
            description: "Register a recurring cron job with the local jfc daemon. \
                Schedule accepts five-field crontab (`*/5 * * * *`), `@hourly`, \
                `@daily`, `@weekly`, or `@every <duration>` (e.g. `@every 5m`, \
                `@every 1h30m`). Returns the new job's id."
                .into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "schedule": { "type": "string" },
                    "command": { "type": "string" },
                    "description": { "type": "string" }
                },
                "required": ["schedule", "command", "description"]
            }),
        },
        ToolDef {
            name: "CronList".into(),
            description: "List all cron jobs currently registered with the local jfc daemon.".into(),
            input_schema: serde_json::json!({ "type": "object", "properties": {} }),
        },
        ToolDef {
            name: "CronDelete".into(),
            description: "Delete a cron job from the local jfc daemon by id.".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": { "id": { "type": "string" } },
                "required": ["id"]
            }),
        },
        ToolDef {
            name: "ScheduleWakeup".into(),
            description: "Schedule a one-shot wakeup that re-posts a prompt to \
                the conversation after `delay_seconds` elapse. Persisted to \
                daemon state so it replays after restart."
                .into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "delay_seconds": { "type": "integer", "minimum": 0 },
                    "prompt": { "type": "string" },
                    "reason": { "type": "string" }
                },
                "required": ["delay_seconds", "prompt", "reason"]
            }),
        },
        ToolDef {
            name: "Monitor".into(),
            description: "Spawn a long-running command and stream stdout \
                line-by-line, returning the first line matching the `until` \
                regex. Times out after 60s."
                .into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "command": { "type": "string" },
                    "until": { "type": "string" }
                },
                "required": ["command", "until"]
            }),
        },
        ToolDef {
            name: "LSP".into(),
            description: "Query the language server for `hover`, `definition`, \
                or `references` at a specific source location. Uses the \
                already-spawned LSP client (rust-analyzer / zls / etc.) — \
                returns an error if no LSP is running for the workspace.".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "kind": {
                        "type": "string",
                        "enum": ["hover", "definition", "references"],
                        "description": "Which LSP request to issue."
                    },
                    "file": {
                        "type": "string",
                        "description": "Absolute path to the source file."
                    },
                    "line": {
                        "type": "number",
                        "description": "1-indexed line number of the symbol position."
                    },
                    "column": {
                        "type": "number",
                        "description": "1-indexed column number of the symbol position."
                    }
                },
                "required": ["kind", "file", "line", "column"]
            }),
        },
        ToolDef {
            name: "PushNotification".into(),
            description: "Send a desktop notification to the user via the \
                native notification daemon (notify-send / NotificationCenter / \
                Toast). Use sparingly for events that need attention while \
                the user has switched focus away from the terminal.".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "message": {
                        "type": "string",
                        "description": "Body text shown in the notification."
                    },
                    "title": {
                        "type": "string",
                        "description": "Optional title; defaults to `jfc`."
                    }
                },
                "required": ["message"]
            }),
        },
        ToolDef {
            name: "RemoteTrigger".into(),
            description: "POST a payload to a webhook URL the user pre-registered \
                in `~/.config/jfc/triggers.toml`. Use to fire CI runs, Slack \
                hooks, custom alert endpoints, etc. without exposing the URL \
                to the model. Triggers are looked up by `trigger_id`; unknown \
                IDs return an error.".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "trigger_id": {
                        "type": "string",
                        "description": "Identifier registered in `~/.config/jfc/triggers.toml`."
                    },
                    "payload": {
                        "type": "object",
                        "description": "Optional JSON object POSTed as the request body."
                    }
                },
                "required": ["trigger_id"]
            }),
        },
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
                   - Example: 'Add a logout button' — where should it go? What happens on click?\n\
                   - Example: 'Add form validation' — what rules? What error messages?\n\n\
                2. **Multiple Valid Approaches**: The task can be solved several different ways\n\
                   - Example: 'Add caching to the API' — Redis, in-memory, file-based?\n\
                   - Example: 'Improve performance' — many optimization strategies possible\n\n\
                3. **Code Modifications**: Changes affecting existing behavior or structure\n\
                   - Example: 'Update the login flow' — what exactly should change?\n\
                   - Example: 'Refactor this component' — what's the target architecture?\n\n\
                4. **Architectural Decisions**: Choosing between patterns or technologies\n\
                   - Example: 'Add real-time updates' — WebSockets vs SSE vs polling?\n\
                   - Example: 'State management' — Redux vs Context vs custom solution?\n\n\
                5. **Multi-File Changes**: Task will likely touch more than 2-3 files\n\
                   - Example: 'Refactor the authentication system'\n\
                   - Example: 'Add a new API endpoint with tests'\n\n\
                6. **Unclear Requirements**: Need to explore before understanding full scope\n\
                   - Example: 'Make the app faster' — need to profile bottlenecks first\n\
                   - Example: 'Fix the bug in checkout' — need to investigate root cause\n\n\
                7. **User Preferences Matter**: Implementation could go multiple ways\n\
                   - If you would use AskUserQuestion to clarify the approach, use EnterPlanMode instead\n\n\
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
        },
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
        },
        ToolDef {
            name: "ExitWorktree".into(),
            description: "Leave the current worktree. The worktree is left intact \
                on disk; only the agent's effective cwd resets to the repo root.".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {},
                "required": []
            }),
        },
        ToolDef {
            name: "NotebookRead".into(),
            description: "Read a Jupyter `.ipynb` notebook and return each cell's \
                id, type (code/markdown/raw), source, and outputs (for code cells). \
                Use before NotebookEdit to discover cell IDs.".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Absolute path to the .ipynb file."
                    }
                },
                "required": ["path"]
            }),
        },
        ToolDef {
            name: "NotebookEdit".into(),
            description: "Edit a Jupyter `.ipynb` notebook by cell id. \
                `edit_mode=replace` (default) overwrites the cell's source; \
                `insert` adds a new code cell after the named cell; `delete` \
                removes the cell. Outputs are cleared on replace+insert. The \
                notebook JSON is parsed, spliced, and written back atomically.".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Absolute path to the .ipynb file."
                    },
                    "cell_id": {
                        "type": "string",
                        "description": "Target cell id (from NotebookRead). For `insert` mode the new cell is placed AFTER this one."
                    },
                    "new_source": {
                        "type": "string",
                        "description": "Replacement (or new) cell source. Ignored when edit_mode=delete."
                    },
                    "edit_mode": {
                        "type": "string",
                        "enum": ["replace", "insert", "delete"],
                        "description": "How to apply the edit. Defaults to replace."
                    }
                },
                "required": ["path", "cell_id", "new_source"]
            }),
        },
    ]
}

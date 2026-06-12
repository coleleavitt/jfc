use jfc_provider::ToolDef;

/// User interaction, web access, plan mode, and worktree tools.
pub fn interaction_tool_defs() -> Vec<ToolDef> {
    vec![
        ask_user_question_def(),
        web_fetch_def(),
        web_search_def(),
        exit_plan_mode_def(),
        enter_plan_mode_def(),
        set_goal_def(),
        enter_worktree_def(),
        exit_worktree_def(),
        send_user_message_def(),
        send_user_file_def(),
    ]
}

fn ask_user_question_def() -> ToolDef {
    ToolDef {
        name: "AskUserQuestion".into(),
        description: "Ask the user a multiple-choice question mid-turn to gather \
            preferences, clarify ambiguity, or offer choices. Opens an \
            interactive modal: the user navigates options with the arrow keys \
            and picks with Enter. Use sparingly — only when context genuinely \
            requires user input; bias toward making the reasonable call and \
            continuing.\n\n\
            - Provide 2-4 options, each a `{label, description}` object (an \
            optional `preview` renders a side panel for comparing concrete \
            artifacts like code snippets or mockups).\n\
            - An \"Other\" free-text choice is added automatically — never \
            include your own.\n\
            - If you recommend a specific option, make it the FIRST option and \
            append \"(Recommended)\" to its label.\n\
            - Set `multi_select: true` only when the choices are not mutually \
            exclusive.\n\
            Returns the user's selected label(s) (free text for \"Other\") as \
            the tool result."
            .into(),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "questions": {
                    "type": "array",
                    "minItems": 1, "maxItems": 4,
                    "description": "1-4 questions to ask. They are presented one at a time with a header-chip nav bar showing progress.",
                    "items": {
                        "type": "object",
                        "properties": {
                            "question": { "type": "string", "description": "The full question; should end with '?'." },
                            "header": { "type": "string", "description": "Very short chip label shown in the nav bar (<= 12 chars), e.g. 'Auth method'." },
                            "options": {
                                "type": "array",
                                "minItems": 2, "maxItems": 4,
                                "items": {
                                    "type": "object",
                                    "properties": {
                                        "label": { "type": "string", "description": "Concise choice text (1-5 words)." },
                                        "description": { "type": "string", "description": "What this option means / its trade-offs." },
                                        "preview": {
                                            "type": "string",
                                            "description": "Optional preview rendered in a side panel when this option is focused. Use for mockups, code snippets, or visual comparisons. Single-select only."
                                        }
                                    },
                                    "required": ["label"]
                                }
                            },
                            "multiSelect": { "type": "boolean", "default": false, "description": "Allow multiple selections for this question." }
                        },
                        "required": ["question", "options"]
                    }
                }
            },
            "required": ["questions"]
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
            than the full body."
            .into(),
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
            to read promising hits. No prefix uses Google Custom Search \
            Engine (falling back to Brave if no Google key is set). \
            Prefix the query to select a backend:\n\
            • `edu: <query>` — Google scoped to academic TLDs worldwide \
            (.edu, .ac.uk, .ac.jp, .edu.cn, .ac.cn, .edu.au, .ac.in, .ac.kr, \
            .edu.hk, .edu.tw, .edu.sg, .ac.nz, .ac.za, .edu.br); great for \
            finding university/library/open-courseware pages.\n\
            • `cn: <query>` — Google scoped to Chinese academic domains \
            (.edu.cn, .ac.cn, .edu.hk, .edu.mo, .edu.tw).\n\
            • `primo: <inst>/<query>` or `primo: <query>` — ExLibris Primo \
            university library discovery (8000+ universities; supported inst \
            keys: cmu, mit, harvard, stanford, berkeley, columbia, cornell, \
            yale, princeton, brown, michigan, ucla, chicago, caltech, nyu, \
            jhu, duke, oxford, cambridge, ucl, eth, tum, nus, hku, \
            melbourne, pku, sjtu, and more). Defaults to CMU.\n\
            • `uni: <University Name>: <topic>` — a specific university's \
            research output via OpenAlex, regardless of country or web domain \
            (e.g. `uni: Tsinghua University: quantum computing`); topic \
            optional. Works for US, Chinese, and European universities alike.\n\
            • `gov: <query>` — Google scoped to government domains (.gov, .gov.uk, …).\n\
            • `arxiv: <query>` — arXiv papers (free, no key).\n\
            • `scholar: <query>` — Semantic Scholar (optional key, BFF fallback).\n\
            • `openalex: <query>` — OpenAlex; 250M+ works with author \
            institutions + country codes (free, no key).\n\
            • `crossref: <query>` — Crossref DOI metadata (free, no key).\n\
            • `pubmed: <query>` — PubMed/NCBI biomedical literature (free).\n\
            • `doaj: <query>` — Directory of Open Access Journals (free).\n\
            • `core: <query>` — CORE 290M+ OA full texts (free key: CORE_API_KEY).\n\
            • `unpaywall: <DOI>` — resolve a DOI to its free, legal open-access \
            PDF locations across repositories (free, no key).\n\
            • `papers: <query>` — arXiv + Semantic Scholar + OpenAlex in \
            parallel, deduplicated by arXiv ID / DOI / title.\n\
            • `brave: <query>` — Brave independent index (key: BRAVE_API_KEY).\n\
            • `tavily: <query>` — Tavily LLM-oriented search (key: TAVILY_API_KEY).\n\
            • `exa: <query>` — Exa neural/semantic search (key: EXA_API_KEY).\n\
            • `ddg: <query>` — DuckDuckGo Instant Answer; facts/definitions \
            only, not a full SERP (free, no key).\n\
            • `searxng: <query>` — SearXNG meta-search; a key-free full SERP that \
            aggregates Google/Bing/DDG (set `SEARXNG_URL` for a self-hosted \
            instance). Also the automatic fallback when Google CSE is rate-limited.\n\
            • `wiki: <query>` — Wikipedia/MediaWiki article search (free, no key)."
            .into(),
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
            Claude Code v2.1.132's ExitPlanMode tool contract."
            .into(),
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

fn set_goal_def() -> ToolDef {
    ToolDef {
        name: "SetGoal".into(),
        description: "Set a session goal (stop-condition) that you will keep working \
            toward across multiple turns without the user having to re-prompt. Use this \
            when a task has a clear, verifiable 'done' state and is likely to take \
            several turns — distill that done-state into one concrete condition and \
            register it. The runtime then auto-evaluates after each turn and keeps you \
            going until the condition is met (or an iteration cap is hit), so you can \
            self-drive to completion.\n\n\
            ## When to use\n\
            - A multi-step task with an objective finish line (e.g. \"all tests in \
            crate X pass\", \"the build is clean and the new endpoint returns 200\").\n\
            - You find yourself about to do step 1 of N and want to commit to finishing N.\n\n\
            ## When NOT to use\n\
            - Open-ended/exploratory work with no checkable end state.\n\
            - A one-shot task you'll finish this turn.\n\n\
            ## Notes\n\
            - Write `condition` as a checkable predicate, not a vague aim.\n\
            - Call SetGoal again with an empty (or `clear`) condition to cancel the goal.\n\
            - The goal auto-clears after a bounded number of 'not yet met' evaluations \
            so it can never loop forever."
            .into(),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "condition": {
                    "type": "string",
                    "description": "A concrete, checkable stop-condition (what 'done' means \
                        for this task). Pass an empty string or 'clear' to cancel an \
                        active goal."
                }
            },
            "required": ["condition"]
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
            `.claude/worktrees/<slug>` checking out branch `worktree-<slug>` (or a \
            caller-provided branch). Subsequent tool calls run in that \
            worktree's directory until `ExitWorktree` is invoked."
            .into(),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "name": {
                    "type": "string",
                    "description": "Worktree name. [A-Za-z0-9_/-], <= 96 chars. Slashes flatten to + in the path and branch."
                },
                "branch": {
                    "type": "string",
                    "description": "Optional branch to check out (defaults to `worktree-<slug>`)."
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
            on disk; only the agent's effective cwd resets to the repo root."
            .into(),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {},
            "required": []
        }),
    }
}

fn send_user_message_def() -> ToolDef {
    ToolDef {
        name: "SendUserMessage".into(),
        description: "Send a message the user will read. Text outside this tool \
            is visible in the detail view, but most won't open it — the answer \
            lives here.\n\n\
            `message` supports markdown. `attachments` accepts file path strings \
            (absolute or cwd-relative). `status` labels intent: 'normal' when \
            replying to what they just asked; 'proactive' when you're initiating — \
            a scheduled task finished, a blocker surfaced during background work, \
            you need input on something they haven't asked about."
            .into(),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "message": { "type": "string", "description": "Markdown message content" },
                "summary": { "type": "string", "description": "A 5-10 word summary shown as a preview in the UI" },
                "attachments": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "File paths to attach"
                },
                "status": {
                    "type": "string",
                    "enum": ["normal", "proactive"],
                    "description": "Intent label: 'normal' for replies, 'proactive' for initiated messages"
                }
            },
            "required": ["message"]
        }),
    }
}

fn send_user_file_def() -> ToolDef {
    ToolDef {
        name: "SendUserFile".into(),
        description: "Send one or more files to the user. Use this when the file \
            *is* the deliverable — a generated diagram, a report, a screenshot, \
            a built artifact — and you want it surfaced, not just mentioned. Paths \
            can be absolute or relative to the current working directory.\n\n\
            Add a `caption` when a one-liner of context helps. Skip it if the file \
            speaks for itself.\n\n\
            Set `status` on every call. Use `proactive` when you're initiating — \
            the user is away. Use `normal` when replying."
            .into(),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "files": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "File paths to send (absolute or cwd-relative)"
                },
                "caption": { "type": "string", "description": "Optional one-liner of context" },
                "status": {
                    "type": "string",
                    "enum": ["normal", "proactive"],
                    "description": "Intent: 'normal' for replies, 'proactive' for initiated sends"
                }
            },
            "required": ["files"]
        }),
    }
}

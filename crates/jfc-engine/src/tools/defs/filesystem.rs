use jfc_provider::ToolDef;

pub fn filesystem_tool_defs() -> Vec<ToolDef> {
    vec![
        ToolDef {
            name: "Bash".into(),
            description: "Executes a bash command in a fresh non-interactive shell. Shell state does not persist between calls; use `workdir` to run in a specific directory. Prefer Glob/Grep/Read/Edit/Write for file discovery and edits. Use Bash for real shell commands, scripts, builds, tests, and package managers. Set `suppressOutput=true` when raw stdout/stderr would be noisy or sensitive but success/failure status is enough. For long-running commands, set `run_in_background=true`; JFC tracks the task id and output file and will report completion, so do not spawn separate sleep/poll commands. If waiting for a remote condition, run one bounded background watcher such as `until check; do sleep 2; done`, and JFC reports its output when the watcher settles. JFC also auto-backgrounds commands that exceed the foreground budget. PERSISTENT SHELL (opt-in): to keep cwd/env across calls (e.g. a `cd` or `export` that should persist), prefix the command with `shell:<id>` and a newline, e.g. command = \"shell:build\\ncd src && make\". All commands sharing the same `<id>` run in one long-lived shell, in order; omit the prefix for the default fresh-shell behavior.".into(),
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
                    "workdir": {
                        "type": "string",
                        "description": "Directory to run the command in. Relative paths are resolved against the current workspace directory."
                    },
                    "run_in_background": {
                        "type": "boolean",
                        "description": "Start the command as a background Bash task and return immediately with a task id and output file. Use this for builds, servers, long scans, remote waits, and condition watchers instead of foreground sleep/poll loops."
                    },
                    "suppressOutput": {
                        "type": "boolean",
                        "description": "Suppress successful foreground command output in the tool result while preserving status/provenance. Failure output is still returned."
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
            name: "BashOutput".into(),
            description: "Read or wait for output from a Bash command that was backgrounded by `Bash.run_in_background` or auto-backgrounded after exceeding the foreground budget. By default block=true waits up to timeout for task completion and returns retrieval_status success/timeout/not_ready. Prefer this over issuing separate sleep commands while a background task is running. Use block=false for a snapshot. Use offset/limit for large logs.".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "task_id": {
                        "type": "string",
                        "description": "Background Bash task id returned by Bash"
                    },
                    "offset": {
                        "type": "number",
                        "description": "Optional 1-indexed line number to start reading from"
                    },
                    "limit": {
                        "type": "number",
                        "description": "Maximum number of lines to return"
                    },
                    "block": {
                        "type": "boolean",
                        "description": "Whether to wait for completion before returning. Defaults to true."
                    },
                    "timeout": {
                        "type": "number",
                        "description": "Max wait time in milliseconds when block=true. Defaults to 30000, max 600000."
                    },
                    "wait_up_to": {
                        "type": "number",
                        "description": "Alias for timeout in milliseconds when block=true."
                    }
                },
                "required": ["task_id"]
            }),
        },
        ToolDef {
            name: "Read".into(),
            description: "Read a file or directory from the local filesystem. Returns file contents with line numbers prefixed. For source code, prefer CodeGraph first: `codegraph_explore`, `codegraph_search`, or `codegraph_node` can return relevant symbol bodies directly, and MCP installs may expose them as names like `mcp__codegraph__codegraph_explore`. Use Read mainly for files you're about to edit, precise ranges CodeGraph identified, or non-source files. When reading a large source file for one region, pass `offset`/`limit` instead of reading the whole file.".into(),
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
            description: "Fast content search using ripgrep. Searches file contents using regular expressions. NOTE: For finding symbols by name (functions, structs, enums), prefer CodeGraph `codegraph_search` or `codegraph_explore`; MCP installs may expose these as names like `mcp__codegraph__codegraph_search`. For finding all callers/callees of a function, use `codegraph_callers`/`codegraph_callees` instead of grepping for the function name. Reserve Grep for string literals, config values, error messages, comments, and non-identifier patterns.".into(),
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

use jfc_provider::ToolDef;

pub fn filesystem_tool_defs() -> Vec<ToolDef> {
    vec![
        ToolDef {
            name: "Bash".into(),
            description: "Executes a bash command in a fresh non-interactive shell. Shell state does not persist between calls; use `workdir` to run in a specific directory. Prefer Glob/Grep/Read/Edit/Write for file discovery and edits. Use Bash for real shell commands, scripts, builds, tests, and package managers. For long-running commands, set `run_in_background=true`; JFC also auto-backgrounds commands that exceed the foreground budget and returns a task id plus output file. Use BashOutput with block=true to wait for completion; do not spawn sleep/poll Bash commands for background Bash tasks.".into(),
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
                        "description": "Start the command as a background Bash task and return immediately with a task id and output file."
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
            description: "Read or wait for output from a Bash command that was backgrounded by `Bash.run_in_background` or auto-backgrounded after exceeding the foreground budget. By default block=true waits up to timeout for task completion and returns retrieval_status success/timeout/not_ready. Use block=false for a snapshot. Use offset/limit for large logs.".into(),
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
            description: "Read a file or directory from the local filesystem. Returns file contents with line numbers prefixed. For source code, prefer the graph: `graph_search include_code=true` or `graph_node` returns a symbol's body directly, and `graph_outline` maps a file without reading it all — use Read mainly for files you're about to edit or non-source files. When you do read a large source file for one region, pass `offset`/`limit` (use the `:start-end` range from graph_search/graph_outline) instead of reading the whole file. For MULTIPLE related symbols at once, prefer `graph_explore` (one call replaces 5+ Read calls).".into(),
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
            description: "Fast content search using ripgrep. Searches file contents using regular expressions. NOTE: For finding symbols by name (functions, structs, enums), prefer `graph_search` — it's faster and returns exact locations. For finding all callers/callees of a function, use `graph_callers`/`graph_callees` instead of grepping for the function name. Reserve Grep for string literals, config values, error messages, comments, and non-identifier patterns.".into(),
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

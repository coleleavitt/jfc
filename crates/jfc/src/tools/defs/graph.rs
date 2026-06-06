use jfc_provider::ToolDef;

pub fn graph_tool_defs() -> Vec<ToolDef> {
    vec![
        code_index_def(),
        graph_query_def(),
        graph_context_def(),
        graph_search_def(),
        graph_callers_def(),
        graph_callees_def(),
        graph_impact_def(),
        graph_node_def(),
        graph_explore_def(),
        graph_outline_def(),
        graph_grep_def(),
        graph_status_def(),
        graph_files_def(),
        get_program_slice_def(),
        get_data_dependencies_def(),
        taint_flow_def(),
        run_coverage_def(),
        symbol_edit_def(),
    ]
}

fn get_program_slice_def() -> ToolDef {
    ToolDef {
        name: "get_program_slice".into(),
        description: "Compute a program SLICE around a symbol: every function whose behaviour can \
            affect the value it computes (backward, the default) or that it can influence \
            (forward). Backed by interprocedural points-to dataflow over the code graph. A \
            backward slice is the ~90%-smaller set of code you actually need to read to understand \
            how a value is produced — prefer it over reading whole files when chasing a bug to its \
            root. Each result is annotated `name (file:line)` so you can jump straight to source."
            .into(),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "symbol": {
                    "type": "string",
                    "description": "Symbol name or qualified path to slice around (e.g. `parse_config`)."
                },
                "backward": {
                    "type": "boolean",
                    "description": "true (default) = what can AFFECT this symbol; false = what this symbol can INFLUENCE."
                },
                "max_nodes": {
                    "type": "number",
                    "description": "Maximum slice nodes to render (default 40)."
                }
            },
            "required": ["symbol"]
        }),
    }
}

fn get_data_dependencies_def() -> ToolDef {
    ToolDef {
        name: "get_data_dependencies".into(),
        description: "List the DIRECT data dependencies of a symbol — the functions whose values \
            flow into it, via interprocedural points-to analysis over the code graph. Narrower \
            than get_program_slice (one hop, not the transitive closure): use it to answer \
            \"what does this function's result actually depend on?\" Each dependency is annotated \
            `name (file:line)`."
            .into(),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "symbol": {
                    "type": "string",
                    "description": "Symbol name or qualified path to find data dependencies of."
                },
                "max_nodes": {
                    "type": "number",
                    "description": "Maximum dependencies to render (default 40)."
                }
            },
            "required": ["symbol"]
        }),
    }
}

fn taint_flow_def() -> ToolDef {
    ToolDef {
        name: "taint_flow".into(),
        description: "Trace TAINT flows from source functions to sink functions across the code \
            graph: does a value from any `sources` reach any `sinks`, and was it sanitized on the \
            way? Sources are untrusted-input functions, sinks are dangerous operations, sanitizers \
            neutralise taint. Unsanitized flows are flagged ⚠. Each flow renders its full \
            source→…→sink path with `file:line` annotations. Use to audit whether user input can \
            reach a dangerous operation. `max_paths` caps the output."
            .into(),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "sources": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Source symbol names (untrusted input), e.g. [\"read_request_body\"]."
                },
                "sinks": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Sink symbol names (dangerous ops), e.g. [\"execute_sql\"]."
                },
                "sanitizers": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Optional sanitizer symbol names that neutralise taint on a path."
                },
                "max_paths": {
                    "type": "number",
                    "description": "Maximum taint flows to render (default 20)."
                }
            },
            "required": ["sources", "sinks"]
        }),
    }
}

fn code_index_def() -> ToolDef {
    ToolDef {
        name: "code_index".into(),
        description: "Return a compact API/symbol index from the cached project code graph. \
            Use this before broad file reads when you need to discover modules, functions, \
            structs, enums, traits, or chainable symbol handles. Optional filters keep output \
            small: `path` narrows to a file/directory, `query` matches symbol names or paths, \
            and `kind` accepts function|struct|enum|module|trait. Output is grouped by file \
            and includes handles like `fn:crate::module::name` for graph_query or symbol_edit."
            .into(),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Optional file or directory substring to filter symbols, relative or absolute."
                },
                "query": {
                    "type": "string",
                    "description": "Optional case-insensitive substring matched against symbol names, qualified names, and file paths."
                },
                "kind": {
                    "type": "string",
                    "description": "Optional symbol kind filter.",
                    "enum": ["function", "struct", "enum", "module", "trait", "enum_variant", "field", "type_alias", "constant"]
                },
                "max_entries": {
                    "type": "number",
                    "description": "Maximum symbols to show. Default 80, capped at 200."
                }
            },
            "required": []
        }),
    }
}

fn graph_query_def() -> ToolDef {
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
            Set algebra (combine queries): `A union B`, `A intersect B`, `A \\\\ B` (difference). \
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
            `(fn(\"foo\") | callers) \\\\ fn(\"test_\") since 42`. \
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
                },
                "format": {
                    "type": "string",
                    "description": "Output format. `markdown` (default) returns human-readable text; `json` returns a versioned schema envelope you can pipe to other tools.",
                    "enum": ["markdown", "json"]
                }
            },
            "required": ["query"]
        }),
    }
}

fn graph_context_def() -> ToolDef {
    ToolDef {
        name: "graph_context".into(),
        description: "PRIMARY TOOL for 'how does X work' / architecture / bug-context questions. \
            Composes search + BFS + type-hierarchy expansion + per-file diversity cap + intent \
            detection into ONE call. Returns entry points, related symbols grouped by file, \
            relationship maps, and optional line-numbered source blocks for entry points. Prefer \
            this over chaining graph_search + graph_query + Read for architecture or bug context. \
            NOTE: surfaces CODE context, not product requirements — for new features still \
            clarify UX/edge cases with the user."
            .into(),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "task": {
                    "type": "string",
                    "description": "Free-form description of the task, bug, or feature to build context for."
                },
                "max_nodes": {
                    "type": "number",
                    "description": "Maximum entry-point + related symbols to include (default 20, capped at 100)."
                },
                "include_code": {
                    "type": "boolean",
                    "description": "Whether to embed source-code blocks for entry-point symbols (default true)."
                },
                "format": {
                    "type": "string",
                    "description": "Output format: `markdown` (default) or `json` (schema-versioned ContextResult envelope).",
                    "enum": ["markdown", "json"]
                }
            },
            "required": ["task"]
        }),
    }
}

fn graph_search_def() -> ToolDef {
    ToolDef {
        name: "graph_search".into(),
        description: "Find symbols by name with qualified-name support. Accepts simple names \
            (`foo`), `::`-qualified (`stage_apply::run`), `.`-qualified (`Session.request`), \
            and `/`-qualified (`stage_apply/run`) lookups. Rust prefixes `crate::`, `super::`, \
            `self::` are stripped automatically. Returns kind, location (with full `:start-end` \
            line range), signature, visibility, and a chainable handle for each match. Set \
            `include_code=true` to get each hit's full source body inline — this replaces the \
            `graph_search` → `sed -n 'start,end p'` round-trip with a single call. Use this when \
            you want the symbol's *shape* or *body*; for code structure use graph_context."
            .into(),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Symbol name or qualified path (e.g. `foo`, `Session::request`, `stage_apply/run`)."
                },
                "limit": {
                    "type": "number",
                    "description": "Maximum results (default 10, capped at 100)."
                },
                "include_code": {
                    "type": "boolean",
                    "description": "When true, render each function/method hit's full source body inline (line-numbered). Avoids a follow-up sed/Read. Default false."
                },
                "format": {
                    "type": "string",
                    "description": "Output format: `markdown` (default) or `json` (schema-versioned envelope with NodeId list).",
                    "enum": ["markdown", "json"]
                }
            },
            "required": ["query"]
        }),
    }
}

fn graph_outline_def() -> ToolDef {
    ToolDef {
        name: "graph_outline".into(),
        description: "Structural outline of ONE file: every indexed symbol (functions, structs, \
            enums, fields, …) with its kind and `:start-end` line range, ordered by position. \
            This is the graph-native replacement for `nl -ba file` / `sed -n` line-number hunting \
            — call it once to get a stable map of where everything lives, then `graph_node` or \
            `graph_search include_code=true` to read a specific symbol's body. Accepts a \
            repo-relative path or a bare filename."
            .into(),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "file": {
                    "type": "string",
                    "description": "File path (repo-relative, e.g. `crates/jfc/src/app/state.rs`) or bare filename."
                }
            },
            "required": ["file"]
        }),
    }
}

fn graph_grep_def() -> ToolDef {
    ToolDef {
        name: "graph_grep".into(),
        description: "Regex CONTENT search across indexed files, with each match enriched by its \
            enclosing symbol (which function/struct the line lives in). Use this for the searches \
            the symbol index can't answer: log messages, error strings, `tracing::` targets, \
            config keys, magic constants, comment text. This is graph-aware grep — prefer it over \
            a raw Bash `rg` when you want to know *where in the code structure* a string lives. \
            For finding symbols by name use graph_search; for callers/callees use those tools."
            .into(),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "pattern": {
                    "type": "string",
                    "description": "Rust-regex pattern matched against each line (e.g. `Stream idle timeout`, `tracing::warn!`, `0x[0-9a-f]+`)."
                },
                "glob": {
                    "type": "string",
                    "description": "Optional path substring filter (e.g. `providers/`, `.ts`). Only files whose path contains this are searched."
                },
                "limit": {
                    "type": "number",
                    "description": "Maximum matches (default 50, capped at 500)."
                }
            },
            "required": ["pattern"]
        }),
    }
}

fn graph_callers_def() -> ToolDef {
    ToolDef {
        name: "graph_callers".into(),
        description: "Find every function that calls `symbol`. Aggregates across all matching \
            symbols when the name is ambiguous (multiple `execute` methods, etc.) and notes the \
            aggregation in the result. Output is file-grouped with signatures inline. Use to \
            understand usage patterns or estimate change impact (combined with graph_impact)."
            .into(),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "symbol": {
                    "type": "string",
                    "description": "Symbol name or qualified path to find callers of."
                },
                "limit": {
                    "type": "number",
                    "description": "Maximum callers to return (default 20, capped at 100)."
                },
                "format": {
                    "type": "string",
                    "description": "Output format: `markdown` (default) or `json` envelope.",
                    "enum": ["markdown", "json"]
                }
            },
            "required": ["symbol"]
        }),
    }
}

fn graph_callees_def() -> ToolDef {
    ToolDef {
        name: "graph_callees".into(),
        description: "Find every function that `symbol` calls. Symmetric to graph_callers. \
            Use to understand dependencies, derive a dataflow trace, or check whether a fn \
            touches a sensitive subsystem (e.g. `graph_callees on auth_handler`)."
            .into(),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "symbol": {
                    "type": "string",
                    "description": "Symbol name or qualified path to find callees of."
                },
                "limit": {
                    "type": "number",
                    "description": "Maximum callees to return (default 20, capped at 100)."
                },
                "format": {
                    "type": "string",
                    "description": "Output format: `markdown` (default) or `json` envelope.",
                    "enum": ["markdown", "json"]
                }
            },
            "required": ["symbol"]
        }),
    }
}

fn graph_impact_def() -> ToolDef {
    ToolDef {
        name: "graph_impact".into(),
        description: "Walk incoming calls outward N hops to surface every symbol whose \
            behaviour might shift if `symbol` changes. Output is grouped by file with \
            `name:line` inline lists — scannable at a glance. The depth controls reach: 1 = \
            direct callers, 2 = callers of callers (default), 3+ = full ripple. Use BEFORE \
            touching a public-API symbol to scope the blast radius."
            .into(),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "symbol": {
                    "type": "string",
                    "description": "Symbol name or qualified path to analyse impact for."
                },
                "depth": {
                    "type": "number",
                    "description": "Hops of incoming-edge expansion (default 2, capped at 10)."
                },
                "format": {
                    "type": "string",
                    "description": "Output format: `markdown` (default) or `json` envelope.",
                    "enum": ["markdown", "json"]
                }
            },
            "required": ["symbol"]
        }),
    }
}

fn graph_node_def() -> ToolDef {
    ToolDef {
        name: "graph_node".into(),
        description: "Get detailed info about ONE symbol (location, signature, docstring). \
            Pass includeCode=true for source: a function/method returns its body; a \
            class/interface/struct/enum returns a compact member OUTLINE (fields + method \
            signatures + line numbers), not every method body — Read or graph_node a specific \
            member for its body. Keep includeCode=false to minimize context. For SEVERAL \
            related symbols, make ONE graph_explore call instead of many node calls — repeated \
            node calls each re-read the whole context and cost far more."
            .into(),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "symbol": {
                    "type": "string",
                    "description": "Name of the symbol to get details for."
                },
                "include_code": {
                    "type": "boolean",
                    "description": "Include full source code (default false)."
                }
            },
            "required": ["symbol"]
        }),
    }
}

fn graph_explore_def() -> ToolDef {
    ToolDef {
        name: "graph_explore".into(),
        description: "Returns source for SEVERAL related symbols grouped by file, plus a \
            relationship map, in ONE capped call. Output is line-numbered, adaptively budgeted \
            by project size, and includes additional relevant files / completeness notes when \
            useful. This is the efficient way to inspect many related symbols at once — strongly \
            prefer it over a series of graph_node or Read calls (each separate call re-reads the \
            whole context, so 8 node calls cost far more than 1 explore). Query with specific \
            symbol/file/code terms, NOT natural-language sentences — run graph_search first to \
            find names when you only have prose."
            .into(),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Symbol names, file names, or short code terms to explore."
                },
                "max_files": {
                    "type": "number",
                    "description": "Maximum number of files to include source from (default 12)."
                }
            },
            "required": ["query"]
        }),
    }
}

fn graph_status_def() -> ToolDef {
    ToolDef {
        name: "graph_status".into(),
        description: "Check the health and size of the project's code graph index. Returns \
            node/edge counts, last-updated timestamp, and whether the file watcher is active. \
            Use this to verify the graph is initialized before running other graph_* tools."
            .into(),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {},
            "required": []
        }),
    }
}

fn graph_files_def() -> ToolDef {
    ToolDef {
        name: "graph_files".into(),
        description: "List files indexed in the code graph. Optionally filter by a path \
            substring. Returns file paths sorted alphabetically. Use this to discover what \
            source files are available before querying symbols."
            .into(),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Optional path substring to filter files (e.g. 'src/tools' or '.rs')."
                }
            },
            "required": []
        }),
    }
}

fn run_coverage_def() -> ToolDef {
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
    }
}

fn symbol_edit_def() -> ToolDef {
    ToolDef {
        name: "symbol_edit".into(),
        description: "Edit a function/struct/etc. by *symbol handle* instead of \
            file:line. Handles look like `fn:module::name` or `struct:Name` and \
            are returned by `graph_query`. The tool resolves the handle to its \
            exact span and replaces it atomically. With `validate=true`, runs \
            signature-compatibility checks against all callers first and refuses \
            edits that would break call sites. Prefer this over Edit when \
            changing signatures, since it surfaces affected callers automatically. \
            If the handle isn't found, the error suggests up to 5 fuzzy matches."
            .into(),
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
    }
}

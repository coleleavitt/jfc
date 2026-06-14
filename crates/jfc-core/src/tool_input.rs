use crate::{TaskInput, ToolKind};

// ───────────────────────────────────────────────────────────────────────────
// Declarative consistency macro for the regular ToolInput variants.
//
// Rationale (rust-lang wg-macros best-practices, "Please help me figure out
// these very old macro guidelines" + "What makes a Decl. Macro hard to
// maintain"):
//   • kpreid: a macro is the right tool when it "generates an enum, and a
//     function guaranteed to take action on all of the items in that list …
//     significantly reduces potential for human error." Here ONE row per tool
//     drives the enum variant, the `from_value` parse arm, AND the `to_value`
//     serialize arm — they cannot drift.
//   • kpreid: "favor narrow-purpose macro_rules! defined in the same module" —
//     these live right next to `ToolInput`, take only `$crate`-free local
//     paths, and are not exported.
//   • Trevor Gross / Jacob Lifshay: parallel repetition with implied equal
//     counts is the #1 footgun. We use a single, properly *nested*
//     `$( … $( … )* )*` (variants × their own fields) — never two sibling
//     repetitions that must stay the same length.
//   • Mario Carneiro: tt-munchers are quadratic. There is NO recursion here —
//     one flat expansion; per-field rules are dispatched to tiny helper
//     macros (`ti_parse!`, `ti_ser!`) that match a fixed rule keyword.
//   • Rule of Least Power: declarative `macro_rules!`, not a proc-macro.
//
// Irregular variants (Bash empty-check, TaskCreate dual-fallback, Skill alias,
// SendMessage coercion, Task nested struct, Mcp, the Server*/Generic/Unknown
// terminals) are NOT forced through the grammar — they stay hand-written in
// `from_value`'s match and are simply not listed here. `summary` is likewise
// hand-written: it is pure display with a unique template per variant and is
// already compiler-exhaustive on `&self`.
// ───────────────────────────────────────────────────────────────────────────

/// `from_value` parse expression for one field rule. `$obj` is the
/// `Option<&Map>`, `$tool` the tool-name closure, `$k` the JSON key.
macro_rules! ti_parse {
    ($obj:ident, $tool:ident, req_str, $k:literal) => {
        match $obj.and_then(|m| m.get($k)) {
            None | Some(serde_json::Value::Null) => {
                return Err(ToolInputError::MissingField {
                    tool: $tool(),
                    field: $k,
                });
            }
            Some(serde_json::Value::String(s)) => s.clone(),
            Some(other) => {
                return Err(ToolInputError::WrongType {
                    tool: $tool(),
                    field: $k,
                    expected: "string",
                    got: json_type_name(other),
                });
            }
        }
    };
    ($obj:ident, $tool:ident, opt_str, $k:literal) => {
        $obj.and_then(|m| m.get($k))
            .and_then(|v| v.as_str())
            .map(str::to_owned)
    };
    ($obj:ident, $tool:ident, opt_u64, $k:literal) => {
        $obj.and_then(|m| m.get($k)).and_then(|v| v.as_u64())
    };
    ($obj:ident, $tool:ident, opt_u64_as_usize, $k:literal) => {
        $obj.and_then(|m| m.get($k))
            .and_then(|v| v.as_u64())
            .map(|n| n as usize)
    };
    ($obj:ident, $tool:ident, opt_u64_as_u32, $k:literal) => {
        $obj.and_then(|m| m.get($k))
            .and_then(|v| v.as_u64())
            .map(|n| n as u32)
    };
    ($obj:ident, $tool:ident, opt_u64_as_u8, $k:literal) => {
        $obj.and_then(|m| m.get($k))
            .and_then(|v| v.as_u64())
            .map(|n| n.min(255) as u8)
    };
    ($obj:ident, $tool:ident, opt_u64_loose, $k:literal) => {
        $obj.and_then(|m| m.get($k)).and_then(|v| {
            v.as_u64()
                .or_else(|| v.as_str().and_then(|s| s.trim().parse::<u64>().ok()))
        })
    };
    ($obj:ident, $tool:ident, u64_or_0, $k:literal) => {
        $obj.and_then(|m| m.get($k))
            .and_then(|v| v.as_u64())
            .unwrap_or(0)
    };
    ($obj:ident, $tool:ident, u32_or_0, $k:literal) => {
        $obj.and_then(|m| m.get($k))
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as u32
    };
    ($obj:ident, $tool:ident, bool_field, $k:literal) => {
        $obj.and_then(|m| m.get($k))
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
    };
    ($obj:ident, $tool:ident, bool_true, $k:literal) => {
        $obj.and_then(|m| m.get($k))
            .and_then(|v| v.as_bool())
            .unwrap_or(true)
    };
    ($obj:ident, $tool:ident, raw_bool_opt, $k:literal) => {
        $obj.and_then(|m| m.get($k)).and_then(|v| v.as_bool())
    };
    ($obj:ident, $tool:ident, replacement, $k:literal) => {
        ReplacementMode::from_replace_all(
            $obj.and_then(|m| m.get($k))
                .and_then(|v| v.as_bool())
                .unwrap_or(false),
        )
    };
    ($obj:ident, $tool:ident, raw_or_empty_array, $k:literal) => {
        $obj.and_then(|m| m.get($k))
            .cloned()
            .unwrap_or(serde_json::Value::Array(vec![]))
    };
    // AskUserQuestion accepts the canonical `questions: [...]` array, OR the
    // legacy single-question form `{question, options, multi_select}` which is
    // normalized into a one-element array so old payloads and resumed sessions
    // still parse. Always yields a JSON array. (`$k` is fixed as "questions".)
    ($obj:ident, $tool:ident, ask_user_questions, $k:literal) => {
        match $obj.and_then(|m| m.get($k)) {
            Some(serde_json::Value::Array(qs)) => serde_json::Value::Array(qs.clone()),
            _ => {
                let question = $obj
                    .and_then(|m| m.get("question"))
                    .cloned()
                    .unwrap_or_else(|| serde_json::Value::String(String::new()));
                let options = $obj
                    .and_then(|m| m.get("options"))
                    .cloned()
                    .unwrap_or_else(|| serde_json::Value::Array(vec![]));
                let multi = $obj
                    .and_then(|m| m.get("multi_select"))
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                serde_json::json!([{
                    "question": question,
                    "options": options,
                    "multiSelect": multi,
                }])
            }
        }
    };
    ($obj:ident, $tool:ident, raw_opt, $k:literal) => {
        $obj.and_then(|m| m.get($k)).cloned()
    };
    ($obj:ident, $tool:ident, str_vec, $k:literal) => {
        $obj.and_then(|m| m.get($k))
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default()
    };
    // str_vec with a fallback alias key — tries $k first, falls back to $alias
    ($obj:ident, $tool:ident, str_vec_alias, $k:literal, $alias:literal) => {
        $obj.and_then(|m| m.get($k).or_else(|| m.get($alias)))
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default()
    };
    ($obj:ident, $tool:ident, opt_u8, $k:literal) => {
        $obj.and_then(|m| m.get($k))
            .and_then(|v| v.as_u64())
            .map(|n| n.min(9) as u8)
    };
}

/// `to_value` serialize stanza for one field rule. Appends to `$v` (a
/// `serde_json::Value` object) under key `$k`, reading binding `$field`.
/// Optional rules only emit the key when the value is present / non-default,
/// matching the original hand-written behavior exactly.
macro_rules! ti_ser {
    ($v:ident, $field:ident, req_str, $k:literal) => {
        $v[$k] = serde_json::json!($field);
    };
    ($v:ident, $field:ident, u64_or_0, $k:literal) => {
        $v[$k] = serde_json::json!($field);
    };
    ($v:ident, $field:ident, u32_or_0, $k:literal) => {
        $v[$k] = serde_json::json!($field);
    };
    ($v:ident, $field:ident, opt_str, $k:literal) => {
        if let Some(x) = $field {
            $v[$k] = serde_json::json!(x);
        }
    };
    ($v:ident, $field:ident, opt_u64, $k:literal) => {
        if let Some(x) = $field {
            $v[$k] = serde_json::json!(x);
        }
    };
    ($v:ident, $field:ident, opt_u64_as_usize, $k:literal) => {
        if let Some(x) = $field {
            $v[$k] = serde_json::json!(x);
        }
    };
    ($v:ident, $field:ident, opt_u64_as_u32, $k:literal) => {
        if let Some(x) = $field {
            $v[$k] = serde_json::json!(x);
        }
    };
    ($v:ident, $field:ident, opt_u64_as_u8, $k:literal) => {
        if let Some(x) = $field {
            $v[$k] = serde_json::json!(x);
        }
    };
    ($v:ident, $field:ident, opt_u64_loose, $k:literal) => {
        if let Some(x) = $field {
            $v[$k] = serde_json::json!(x);
        }
    };
    ($v:ident, $field:ident, raw_opt, $k:literal) => {
        if let Some(x) = $field {
            $v[$k] = x.clone();
        }
    };
    ($v:ident, $field:ident, raw_or_empty_array, $k:literal) => {
        $v[$k] = $field.clone();
    };
    ($v:ident, $field:ident, ask_user_questions, $k:literal) => {
        $v[$k] = $field.clone();
    };
    ($v:ident, $field:ident, bool_field, $k:literal) => {
        if *$field {
            $v[$k] = serde_json::json!(true);
        }
    };
    ($v:ident, $field:ident, replacement, $k:literal) => {
        if $field.replace_all() {
            $v[$k] = serde_json::json!(true);
        }
    };
    ($v:ident, $field:ident, bool_true, $k:literal) => {
        if !*$field {
            $v[$k] = serde_json::json!(false);
        }
    };
    ($v:ident, $field:ident, raw_bool_opt, $k:literal) => {
        if let Some(x) = $field {
            $v[$k] = serde_json::json!(x);
        }
    };
    ($v:ident, $field:ident, str_vec, $k:literal) => {
        if !$field.is_empty() {
            $v[$k] = serde_json::json!($field);
        }
    };
    // alias variant — serialize under primary key only
    ($v:ident, $field:ident, str_vec_alias, $k:literal, $alias:literal) => {
        if !$field.is_empty() {
            $v[$k] = serde_json::json!($field);
        }
    };
    ($v:ident, $field:ident, opt_u8, $k:literal) => {
        if let Some(x) = $field {
            $v[$k] = serde_json::json!(x);
        }
    };
}

/// **The table.** Single source of truth for every *regular* (rule-driven)
/// ToolInput variant. It replays its rows to a callback macro `$cb`, so the
/// exact same field list drives both the `from_value` parse arm and the
/// `to_value` serialize arm — they cannot drift out of sync. Adding a regular
/// tool means adding exactly one row here.
///
/// Each row is `Variant => { field: rule @ "json_key", … }`. The leading
/// `$cb` token is the generator macro to forward to (one flat, non-recursive
/// replay — no tt-munching).
macro_rules! for_each_regular_tool_input {
    ($cb:ident) => {
        $cb! {
            Edit => { file_path: req_str @ "file_path", old_string: req_str @ "old_string", new_string: req_str @ "new_string", replacement: replacement @ "replace_all" }
            Write => { file_path: req_str @ "file_path", content: req_str @ "content" }
            Read => { file_path: req_str @ "file_path", offset: opt_u64_loose @ "offset", limit: opt_u64_loose @ "limit" }
            Glob => { pattern: req_str @ "pattern", path: opt_str @ "path" }
            Grep => { pattern: req_str @ "pattern", path: opt_str @ "path", glob: opt_str @ "glob", output_mode: opt_str @ "output_mode" }
            Search => { query: req_str @ "query", path: opt_str @ "path" }
            ApplyPatch => { patch: req_str @ "patch" }
            TaskList => { status_filter: opt_str @ "status_filter", owner_filter: opt_str @ "owner_filter", include_history: raw_bool_opt @ "include_history", history_query: opt_str @ "history_query" }
            TaskDone => { task_id: req_str @ "task_id" }
            TaskStop => { task_id: req_str @ "task_id" }
            TaskGet => { task_id: req_str @ "task_id" }
            ToolSearch => { query: req_str @ "query", limit: opt_u64 @ "limit" }
            ToolSuggest => { intent: req_str @ "intent", limit: opt_u64 @ "limit" }
            MemoryCreate => { level: req_str @ "level", memory_type: req_str @ "memory_type", scope: req_str @ "scope", body: req_str @ "body" }
            MemoryDelete => { path: req_str @ "path" }
            TeamCreate => { team_name: req_str @ "team_name", description: opt_str @ "description" }
            TeamMemberMode => { member_name: req_str @ "member_name", mode: req_str @ "mode" }
            BashOutput => { task_id: req_str @ "task_id", offset: opt_u64 @ "offset", limit: opt_u64 @ "limit", block: raw_bool_opt @ "block", timeout: opt_u64_loose @ "timeout", wait_up_to: opt_u64_loose @ "wait_up_to" }
            PlanCreate => { title: req_str @ "title", body: opt_str @ "body" }
            PlanList => { status: opt_str @ "status" }
            PlanShow => { slug: req_str @ "slug" }
            PlanAdvance => { slug: req_str @ "slug", summary: req_str @ "summary" }
            PlanArchive => { slug: req_str @ "slug", reason: opt_str @ "reason" }
            PlanMaterialize => { slug: req_str @ "slug" }
            LearnStatus => {}
            LearnHistorize => {}
            LearnDream => {}
            LearnKeyFilesList => {}
            LearnUserProfileShow => {}
            PostBounty => { description: req_str @ "description", budget: u64_or_0 @ "budget", acceptance_criteria: req_str @ "acceptance_criteria", max_solvers: opt_u64_as_u8 @ "max_solvers", auto_dispatch: bool_field @ "auto_dispatch" }
            MarketStatus => { bounty_id: opt_str @ "bounty_id" }
            RunBounty => { bounty_id: req_str @ "bounty_id", max_solvers: opt_u64_as_u8 @ "max_solvers" }
            ExitPlanMode => { plan: req_str @ "plan" }
            SubmitPlan => { short_name: req_str @ "short_name", summary: req_str @ "summary", plan: req_str @ "plan" }
            AddReviewComment => { file_path: req_str @ "file_path", start_line: u32_or_0 @ "start_line", end_line: u32_or_0 @ "end_line", text: req_str @ "text" }
            SuggestCommitMessage => { message: req_str @ "message", scope: opt_str @ "scope" }
            MultiEdit => { file_path: req_str @ "file_path", edits: raw_or_empty_array @ "edits" }
            AskUserQuestion => { questions: ask_user_questions @ "questions" }
            WebFetch => { url: req_str @ "url", prompt: opt_str @ "prompt" }
            WebSearch => { query: req_str @ "query", max_results: opt_u64_as_u32 @ "max_results" }
            CronCreate => { schedule: req_str @ "schedule", command: req_str @ "command", description: req_str @ "description" }
            CronDelete => { id: req_str @ "id" }
            ScheduleWakeup => { delay_seconds: u32_or_0 @ "delay_seconds", prompt: req_str @ "prompt", reason: req_str @ "reason" }
            Monitor => { command: req_str @ "command", until: req_str @ "until" }
            Lsp => { kind: req_str @ "kind", file: req_str @ "file", line: u32_or_0 @ "line", column: u32_or_0 @ "column" }
            PushNotification => { message: req_str @ "message", title: opt_str @ "title" }
            RemoteTrigger => { trigger_id: req_str @ "trigger_id", payload: raw_opt @ "payload" }
            EnterPlanMode => { reason: req_str @ "reason" }
            EnterWorktree => { name: req_str @ "name", branch: opt_str @ "branch" }
            NotebookRead => { path: req_str @ "path" }
            NotebookEdit => { path: req_str @ "path", cell_id: req_str @ "cell_id", new_source: req_str @ "new_source", edit_mode: opt_str @ "edit_mode" }
            ScratchpadRead => { key: req_str @ "key" }
            ScratchpadWrite => { key: req_str @ "key", value: req_str @ "value" }
            Workflow => { script: opt_str @ "script", name: opt_str @ "name", script_path: opt_str @ "scriptPath", args: raw_opt @ "args", resume_from_run_id: opt_str @ "resumeFromRunId" }
            DesignProjectCreate => { title: req_str @ "title" }
            DesignProjectList => {}
            DesignProjectSetMeta => { project_id: req_str @ "project_id", title: opt_str @ "title", is_design_system: raw_bool_opt @ "is_design_system" }
            DesignListFiles => { project_id: req_str @ "project_id" }
            DesignReadFile => { project_id: req_str @ "project_id", path: req_str @ "path" }
            DesignWriteFile => { project_id: req_str @ "project_id", path: req_str @ "path", content: req_str @ "content", asset_name: opt_str @ "asset_name" }
            DesignDeleteFile => { project_id: req_str @ "project_id", path: req_str @ "path" }
            DesignCopyFile => { project_id: req_str @ "project_id", from_path: req_str @ "from_path", to_path: req_str @ "to_path" }
            DesignRegisterAsset => { project_id: req_str @ "project_id", name: req_str @ "name", path: req_str @ "path" }
            DesignUnregisterAsset => { project_id: req_str @ "project_id", path: req_str @ "path" }
            DesignBundleHtml => { input: req_str @ "input", output: opt_str @ "output", require_thumbnail: raw_bool_opt @ "require_thumbnail" }
            DesignHandoff => { project_dir: req_str @ "project_dir", feature: req_str @ "feature", files: str_vec @ "files" }
            DesignCheckSystem => { project_dir: req_str @ "project_dir" }
            DesignCapabilities => { format: opt_str @ "format" }
            DesignServe => { project_dir: req_str @ "project_dir", port: opt_u64_as_u32 @ "port", file: opt_str @ "file" }
            SetGoal => { condition: req_str @ "condition" }
            Research => { question: req_str @ "question", export: bool_field @ "export" }
            Council => { question: req_str @ "question", models: str_vec @ "models" }
        }
    };
}

/// Supplementary table: variants whose `to_value` serialization follows the
/// regular field-rule pattern but whose `from_value` parsing is bespoke (dual
/// fallbacks, empty-checks, coercions). Listing them here keeps their
/// serialize arm table-driven and drift-proof, while their parse arm stays
/// hand-written in `from_value`. The rule tokens here only feed `ti_ser!`.
macro_rules! for_each_to_value_only_tool_input {
    ($cb:ident) => {
        $cb! {
            Bash => { command: req_str @ "command", timeout: opt_u64 @ "timeout", workdir: opt_str @ "workdir", run_in_background: raw_bool_opt @ "run_in_background" }
            TaskCreate => { subject: req_str @ "subject", description: req_str @ "description", active_form: opt_str @ "active_form", blocked_by: str_vec @ "blocked_by", acceptance_criteria: opt_str @ "acceptance_criteria", verification_command: opt_str @ "verification_command", risk: opt_str @ "risk", parent_id: opt_str @ "parent_id", kind: opt_str @ "kind", tags: str_vec @ "tags", priority: opt_u8 @ "priority", effort: opt_str @ "effort", model: opt_str @ "model" }
            Skill => { name: req_str @ "name", args: opt_str @ "args" }
            SendMessage => { to: req_str @ "to", message: req_str @ "message", summary: opt_str @ "summary" }
        }
    };
}

/// Generator callback for `from_value`: expands the table into a single
/// associated fn `parse_regular` that returns `Some(Ok(..))` for a regular
/// kind, `Some(Err(..))` on a parse failure, and `None` for any kind not in
/// the table (the bespoke + terminal kinds, which the caller handles). This
/// keeps the table-driven arms and the hand-written bespoke arms in one
/// `match` without the "macro can't emit multiple arms" limitation — the
/// whole regular `match` lives inside the generated fn.
macro_rules! gen_regular_from_value {
    ( $( $variant:ident => { $( $field:ident : $rule:ident @ $key:literal ),* $(,)? } )* ) => {
        fn parse_regular(
            kind: &ToolKind,
            obj: Option<&serde_json::Map<String, serde_json::Value>>,
            tool: &dyn Fn() -> String,
        ) -> Option<Result<ToolInput, ToolInputError>> {
            // Local re-impl of the json type-namer the req_str rule needs,
            // kept here so the generated fn is self-contained.
            fn json_type_name(value: &serde_json::Value) -> &'static str {
                match value {
                    serde_json::Value::Null => "null",
                    serde_json::Value::Bool(_) => "bool",
                    serde_json::Value::Number(_) => "number",
                    serde_json::Value::String(_) => "string",
                    serde_json::Value::Array(_) => "array",
                    serde_json::Value::Object(_) => "object",
                }
            }
            // `?`-on-ToolInputError needs the fn to return Result; wrap in a
            // closure so `?` short-circuits into `Some(Err(..))`.
            let attempt = || -> Result<Option<ToolInput>, ToolInputError> {
                Ok(Some(match kind {
                    $(
                        ToolKind::$variant => ToolInput::$variant {
                            $( $field: ti_parse!(obj, tool, $rule, $key), )*
                        },
                    )*
                    _ => return Ok(None),
                }))
            };
            match attempt() {
                Ok(Some(parsed)) => Some(Ok(parsed)),
                Ok(None) => None,
                Err(e) => Some(Err(e)),
            }
        }
    };
}

/// Generator callback for `to_value`: expands the main table into the fn
/// `serialize_regular` returning `Some(value)` for a main-table variant and
/// `None` otherwise (bespoke or supplementary).
macro_rules! gen_regular_to_value {
    ( $( $variant:ident => { $( $field:ident : $rule:ident @ $key:literal ),* $(,)? } )* ) => {
        fn serialize_regular(this: &ToolInput) -> Option<serde_json::Value> {
            #[allow(unused_mut)]
            Some(match this {
                $(
                    ToolInput::$variant { $( $field, )* } => {
                        let mut v = serde_json::json!({});
                        $( ti_ser!(v, $field, $rule, $key); )*
                        v
                    }
                )*
                _ => return None,
            })
        }
    };
}

/// Builds `serialize_extra` from the supplementary (serialize-only) table.
/// Uses `..` in the destructure since bespoke-parse variants may carry fields
/// the serialize rules don't read (e.g. TaskCreate's exhaustive field set is
/// listed, but the macro only binds what each rule needs).
macro_rules! gen_serialize_extra {
    () => {
        for_each_to_value_only_tool_input!(gen_serialize_extra_impl);
    };
}
macro_rules! gen_serialize_extra_impl {
    ( $( $variant:ident => { $( $field:ident : $rule:ident @ $key:literal ),* $(,)? } )* ) => {
        fn serialize_extra(this: &ToolInput) -> Option<serde_json::Value> {
            #[allow(unused_mut)]
            Some(match this {
                $(
                    ToolInput::$variant { $( $field, )* } => {
                        let mut v = serde_json::json!({});
                        $( ti_ser!(v, $field, $rule, $key); )*
                        v
                    }
                )*
                _ => return None,
            })
        }
    };
}

#[derive(Clone, Debug, serde::Serialize)]
pub enum ToolInput {
    Edit {
        file_path: String,
        old_string: String,
        new_string: String,
        replacement: ReplacementMode,
    },
    Write {
        file_path: String,
        content: String,
    },
    Read {
        file_path: String,
        offset: Option<u64>,
        limit: Option<u64>,
    },
    Bash {
        command: String,
        timeout: Option<u64>,
        workdir: Option<String>,
        run_in_background: Option<bool>,
    },
    BashOutput {
        task_id: String,
        offset: Option<u64>,
        limit: Option<u64>,
        block: Option<bool>,
        timeout: Option<u64>,
        wait_up_to: Option<u64>,
    },
    Glob {
        pattern: String,
        path: Option<String>,
    },
    Grep {
        pattern: String,
        path: Option<String>,
        glob: Option<String>,
        output_mode: Option<String>,
    },
    Search {
        query: String,
        path: Option<String>,
    },
    ApplyPatch {
        patch: String,
    },
    Task(TaskInput),
    TaskCreate {
        subject: String,
        description: String,
        active_form: Option<String>,
        blocked_by: Vec<String>,
        acceptance_criteria: Option<String>,
        verification_command: Option<String>,
        risk: Option<String>,
        parent_id: Option<String>,
        kind: Option<String>,
        tags: Vec<String>,
        priority: Option<u8>,
        effort: Option<String>,
        model: Option<String>,
    },
    TaskUpdate {
        task_id: String,
        status: Option<String>,
        subject: Option<String>,
        description: Option<String>,
        owner: Option<String>,
        acceptance_criteria: Option<String>,
        verification_command: Option<String>,
        risk: Option<String>,
        parent_id: Option<String>,
        kind: Option<String>,
        blocked_by: Vec<String>,
        tags: Vec<String>,
        priority: Option<u8>,
        effort: Option<String>,
        model: Option<String>,
    },
    TaskList {
        status_filter: Option<String>,
        owner_filter: Option<String>,
        /// When true, also return the archived task-history log (durable record
        /// of pruned terminal tasks) alongside the live task list.
        include_history: Option<bool>,
        /// Case-insensitive substring filter applied to archived history
        /// records (subject/id/tags). Ignored unless `include_history` is set.
        history_query: Option<String>,
    },
    TaskDone {
        task_id: String,
    },
    TaskStop {
        task_id: String,
    },
    TaskGet {
        task_id: String,
    },
    TaskValidate,
    Skill {
        name: String,
        args: Option<String>,
    },
    ToolSearch {
        query: String,
        limit: Option<u64>,
    },
    ToolSuggest {
        intent: String,
        limit: Option<u64>,
    },
    MemoryCreate {
        level: String,
        memory_type: String,
        scope: String,
        body: String,
    },
    MemoryDelete {
        path: String,
    },
    TeamCreate {
        team_name: String,
        description: Option<String>,
    },
    TeamDelete,
    SendMessage {
        to: String,
        message: String,
        summary: Option<String>,
    },
    TeamMemberMode {
        member_name: String,
        mode: String,
    },
    PostBounty {
        description: String,
        budget: u64,
        acceptance_criteria: String,
        #[serde(default)]
        max_solvers: Option<u8>,
        #[serde(default)]
        auto_dispatch: bool,
    },
    MarketStatus {
        #[serde(default)]
        bounty_id: Option<String>,
    },
    RunBounty {
        bounty_id: String,
        #[serde(default)]
        max_solvers: Option<u8>,
    },
    PlanCreate {
        title: String,
        #[serde(default)]
        body: Option<String>,
    },
    PlanList {
        #[serde(default)]
        status: Option<String>,
    },
    PlanShow {
        slug: String,
    },
    PlanAdvance {
        slug: String,
        summary: String,
    },
    PlanArchive {
        slug: String,
        #[serde(default)]
        reason: Option<String>,
    },
    PlanMaterialize {
        slug: String,
    },
    LearnStatus {},
    LearnHistorize {},
    LearnDream {},
    LearnKeyFilesList {},
    LearnUserProfileShow {},
    ExitPlanMode {
        plan: String,
    },
    SubmitPlan {
        short_name: String,
        summary: String,
        plan: String,
    },
    AddReviewComment {
        file_path: String,
        start_line: u32,
        end_line: u32,
        text: String,
    },
    SuggestCommitMessage {
        message: String,
        scope: Option<String>,
    },
    MultiEdit {
        file_path: String,
        edits: serde_json::Value,
    },
    AskUserQuestion {
        /// Normalized `questions` array (1-4). Each element is
        /// `{question, header?, options:[{label,description?,preview?}], multiSelect?}`.
        /// The legacy single-question form is normalized into a 1-element array
        /// at parse time (see the `ask_user_questions` rule).
        questions: serde_json::Value,
    },
    WebFetch {
        url: String,
        prompt: Option<String>,
    },
    WebSearch {
        query: String,
        max_results: Option<u32>,
    },
    Mcp {
        name: String,
        arguments: serde_json::Value,
    },
    CronCreate {
        schedule: String,
        command: String,
        description: String,
    },
    CronList,
    CronDelete {
        id: String,
    },
    ScheduleWakeup {
        delay_seconds: u32,
        prompt: String,
        reason: String,
    },
    Monitor {
        command: String,
        until: String,
    },
    Lsp {
        kind: String,
        file: String,
        line: u32,
        column: u32,
    },
    PushNotification {
        message: String,
        title: Option<String>,
    },
    RemoteTrigger {
        trigger_id: String,
        #[serde(default)]
        payload: Option<serde_json::Value>,
    },
    EnterPlanMode {
        reason: String,
    },
    EnterWorktree {
        name: String,
        branch: Option<String>,
    },
    ExitWorktree,
    NotebookRead {
        path: String,
    },
    NotebookEdit {
        path: String,
        cell_id: String,
        new_source: String,
        edit_mode: Option<String>,
    },
    ScratchpadRead {
        key: String,
    },
    ScratchpadWrite {
        key: String,
        value: String,
    },
    Workflow {
        script: Option<String>,
        name: Option<String>,
        script_path: Option<String>,
        args: Option<serde_json::Value>,
        resume_from_run_id: Option<String>,
    },
    /// Run a curated, allowlisted slash command on the agent's behalf, e.g.
    /// `/research <q>`, `/review`, `/commit`, `/workflow <name>`. `command` is
    /// the command name (with or without a leading `/`); `args` is the rest of
    /// the command line.
    SlashCommand {
        command: String,
        #[serde(default)]
        args: Option<String>,
    },
    SendUserMessage {
        message: String,
        #[serde(default)]
        summary: Option<String>,
        #[serde(default)]
        attachments: Option<serde_json::Value>,
        #[serde(default)]
        status: Option<String>,
    },
    SendUserFile {
        files: serde_json::Value,
        #[serde(default)]
        caption: Option<String>,
        #[serde(default)]
        status: Option<String>,
    },
    StructuredOutput {
        #[serde(flatten)]
        data: serde_json::Value,
    },
    WaitForMcpServers {
        #[serde(default)]
        timeout_ms: Option<u64>,
    },
    ListMcpResources {
        #[serde(default)]
        server: Option<String>,
    },
    ReadMcpResource {
        server: String,
        uri: String,
    },
    Advisor {},
    ConnectGitHub {},
    DesignProjectCreate {
        title: String,
    },
    DesignProjectList {},
    DesignProjectSetMeta {
        project_id: String,
        title: Option<String>,
        is_design_system: Option<bool>,
    },
    DesignListFiles {
        project_id: String,
    },
    DesignReadFile {
        project_id: String,
        path: String,
    },
    DesignWriteFile {
        project_id: String,
        path: String,
        content: String,
        asset_name: Option<String>,
    },
    DesignDeleteFile {
        project_id: String,
        path: String,
    },
    DesignCopyFile {
        project_id: String,
        from_path: String,
        to_path: String,
    },
    DesignRegisterAsset {
        project_id: String,
        name: String,
        path: String,
    },
    DesignUnregisterAsset {
        project_id: String,
        path: String,
    },
    DesignBundleHtml {
        input: String,
        output: Option<String>,
        require_thumbnail: Option<bool>,
    },
    DesignHandoff {
        project_dir: String,
        feature: String,
        files: Vec<String>,
    },
    DesignCheckSystem {
        project_dir: String,
    },
    DesignCapabilities {
        format: Option<String>,
    },
    DesignServe {
        project_dir: String,
        port: Option<u32>,
        file: Option<String>,
    },
    /// Model-invocable session goal / stop-condition. The agent reads a task,
    /// distills the condition under which it's "done", and registers it — the
    /// goal loop then keeps the agent working until an evaluator says the
    /// condition is met (or the iteration cap is hit). `condition` empty or a
    /// clear-word (`clear`/`stop`/…) clears any active goal.
    SetGoal {
        condition: String,
    },
    Research {
        question: String,
        export: bool,
    },
    Council {
        question: String,
        models: Vec<String>,
    },
    Generic {
        summary: String,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
pub enum ReplacementMode {
    FirstOnly,
    All,
}

impl ReplacementMode {
    pub fn from_replace_all(replace_all: bool) -> Self {
        if replace_all {
            Self::All
        } else {
            Self::FirstOnly
        }
    }

    pub fn replace_all(self) -> bool {
        matches!(self, Self::All)
    }
}

#[derive(thiserror::Error, Debug, PartialEq, Eq)]
pub enum ToolInputError {
    #[error("tool `{tool}`: missing required field `{field}`")]
    MissingField { tool: String, field: &'static str },
    #[error("tool `{tool}`: field `{field}` has wrong type (expected {expected}, got {got})")]
    WrongType {
        tool: String,
        field: &'static str,
        expected: &'static str,
        got: &'static str,
    },
    #[error("tool `{tool}`: invalid input — {reason}")]
    InvalidShape { tool: String, reason: String },
}

impl ToolInput {
    pub fn summary(&self) -> String {
        match self {
            Self::Edit { file_path, .. } => file_path.clone(),
            Self::Write { file_path, .. } => file_path.clone(),
            Self::Read { file_path, .. } => file_path.clone(),
            Self::Bash {
                command, workdir, ..
            } => match workdir {
                Some(workdir) => format!("{command} in {workdir}"),
                None => command.clone(),
            },
            Self::BashOutput { task_id, .. } => format!("output for {task_id}"),
            Self::Glob { pattern, path } => match path {
                Some(path) => format!("{pattern} in {path}"),
                None => pattern.clone(),
            },
            Self::Grep { pattern, path, .. } => match path {
                Some(path) => format!("{pattern} in {path}"),
                None => pattern.clone(),
            },
            Self::Search { query, path } => match path {
                Some(path) => format!("{query} in {path}"),
                None => query.clone(),
            },
            Self::ApplyPatch { patch } => format!("apply patch ({} bytes)", patch.len()),
            Self::TaskCreate { subject, .. } => format!("create: {subject}"),
            Self::TaskUpdate { task_id, .. } => format!("update: {task_id}"),
            Self::TaskList {
                status_filter,
                include_history,
                ..
            } => {
                let hist = if include_history.unwrap_or(false) {
                    " +history"
                } else {
                    ""
                };
                match status_filter {
                    Some(f) => format!("list tasks ({f}){hist}"),
                    None => format!("list tasks{hist}"),
                }
            }
            Self::TaskDone { task_id } => format!("done: {task_id}"),
            Self::TaskStop { task_id } => format!("stop: {task_id}"),
            Self::TaskGet { task_id } => format!("get: {task_id}"),
            Self::TaskValidate => "validate task graph".into(),
            Self::Task(task_input) => task_input.summary(),
            Self::Skill { name, args } => match args.as_deref().filter(|s| !s.is_empty()) {
                Some(args) => format!("{name}: {args}"),
                None => name.clone(),
            },
            Self::ToolSearch { query, .. } => format!("tool search: {query}"),
            Self::ToolSuggest { intent, .. } => format!("tool suggest: {intent}"),
            Self::MemoryCreate { body, level, .. } => {
                let preview: String = body.chars().take(50).collect();
                format!("remember ({level}): {preview}")
            }
            Self::MemoryDelete { path } => format!("forget: {path}"),
            Self::TeamCreate { team_name, .. } => format!("create team: {team_name}"),
            Self::TeamDelete => "cleanup team".into(),
            Self::SendMessage { to, summary, .. } => match summary {
                Some(summary) => format!("→ {to}: {summary}"),
                None => format!("→ {to}"),
            },
            Self::TeamMemberMode { member_name, mode } => {
                format!("set {member_name} → {mode}")
            }
            Self::PlanCreate { title, .. } => format!("plan_create: {title}"),
            Self::PlanList { .. } => "plan_list".into(),
            Self::PlanShow { slug, .. } => format!("plan_show: {slug}"),
            Self::PlanAdvance { slug, summary } => format!("plan_advance: {slug} — {summary}"),
            Self::PlanArchive { slug, .. } => format!("plan_archive: {slug}"),
            Self::PlanMaterialize { slug } => format!("plan_materialize: {slug}"),
            Self::LearnStatus { .. } => "learn_status".into(),
            Self::LearnHistorize { .. } => "learn_historize".into(),
            Self::LearnDream { .. } => "learn_dream".into(),
            Self::LearnKeyFilesList { .. } => "learn_key_files_list".into(),
            Self::LearnUserProfileShow { .. } => "learn_user_profile_show".into(),
            Self::PostBounty {
                description,
                budget,
                ..
            } => {
                format!(
                    "bounty ({budget} tok): {}",
                    description.chars().take(60).collect::<String>()
                )
            }
            Self::MarketStatus { bounty_id } => match bounty_id {
                Some(id) => format!("market status: {id}"),
                None => "market status".into(),
            },
            Self::RunBounty { bounty_id, .. } => format!("run bounty: {bounty_id}"),
            Self::ExitPlanMode { plan } => {
                let head: String = plan.lines().next().unwrap_or("").chars().take(60).collect();
                format!("exit plan mode: {head}")
            }
            Self::SubmitPlan { short_name, .. } => format!("submit plan: {short_name}"),
            Self::AddReviewComment {
                file_path,
                start_line,
                end_line,
                ..
            } => format!("review comment: {file_path}:{start_line}-{end_line}"),
            Self::SuggestCommitMessage { message, .. } => {
                let preview: String = message.chars().take(60).collect();
                format!("commit message: {preview}")
            }
            Self::MultiEdit { file_path, edits } => {
                let count = edits.as_array().map(|a| a.len()).unwrap_or(0);
                format!(
                    "{file_path} ({count} edit{})",
                    if count == 1 { "" } else { "s" }
                )
            }
            Self::AskUserQuestion { questions } => {
                let arr = questions.as_array();
                let first = arr
                    .and_then(|a| a.first())
                    .and_then(|q| q.get("question"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let n = arr.map(|a| a.len()).unwrap_or(0);
                if n > 1 {
                    format!(
                        "ask ({n} questions): {}",
                        first.chars().take(48).collect::<String>()
                    )
                } else {
                    format!("ask: {}", first.chars().take(60).collect::<String>())
                }
            }
            Self::WebFetch { url, .. } => format!("fetch: {url}"),
            Self::WebSearch { query, .. } => format!("search: {query}"),
            Self::Mcp { name, arguments } => {
                let label = split_advertised_mcp(name)
                    .map(|(server, tool)| format!("{tool}@{server}"))
                    .unwrap_or_else(|| name.clone());
                let preview: String = arguments.to_string().chars().take(60).collect();
                format!("{label}: {preview}")
            }
            Self::CronCreate {
                schedule,
                description,
                ..
            } => format!("cron `{schedule}`: {description}"),
            Self::CronList => "list cron jobs".into(),
            Self::CronDelete { id } => format!("delete cron: {id}"),
            Self::ScheduleWakeup {
                delay_seconds,
                reason,
                ..
            } => format!("wake in {delay_seconds}s: {reason}"),
            Self::Monitor { command, until } => {
                let preview: String = command.chars().take(40).collect();
                format!("monitor `{preview}` until /{until}/")
            }
            Self::Lsp {
                kind, file, line, ..
            } => format!("lsp {kind} {file}:{line}"),
            Self::PushNotification { message, title } => match title {
                Some(title) if !title.is_empty() => format!("{title}: {message}"),
                _ => message.clone(),
            },
            Self::RemoteTrigger { trigger_id, .. } => format!("trigger: {trigger_id}"),
            Self::EnterPlanMode { reason } => {
                let preview: String = reason.chars().take(60).collect();
                format!("enter plan mode: {preview}")
            }
            Self::EnterWorktree { name, branch } => match branch {
                Some(branch) => format!("enter worktree {name} ({branch})"),
                None => format!("enter worktree {name}"),
            },
            Self::ExitWorktree => "exit worktree".into(),
            Self::NotebookRead { path } => path.clone(),
            Self::NotebookEdit {
                path,
                cell_id,
                edit_mode,
                ..
            } => {
                let mode = edit_mode.as_deref().unwrap_or("replace");
                format!("notebook {mode} {path}#{cell_id}")
            }
            Self::ScratchpadRead { key } => format!("scratchpad read: {key}"),
            Self::ScratchpadWrite { key, .. } => format!("scratchpad write: {key}"),
            Self::Workflow {
                name, script_path, ..
            } => {
                if let Some(n) = name {
                    format!("workflow: {n}")
                } else if let Some(p) = script_path {
                    format!("workflow: {p}")
                } else {
                    "workflow (inline script)".into()
                }
            }
            Self::SlashCommand { command, args } => match args {
                Some(a) if !a.is_empty() => {
                    format!("/{} {a}", command.trim_start_matches('/'))
                }
                _ => format!("/{}", command.trim_start_matches('/')),
            },
            Self::Generic { summary } => generic_summary_display(summary),
            Self::SendUserMessage { message, .. } => {
                let preview = if message.len() > 60 {
                    &message[..message.floor_char_boundary(60)]
                } else {
                    message.as_str()
                };
                format!("message: {preview}")
            }
            Self::SendUserFile { caption, .. } => {
                caption.clone().unwrap_or_else(|| "file(s)".into())
            }
            Self::StructuredOutput { .. } => "structured output".into(),
            Self::WaitForMcpServers { .. } => "waiting for MCP servers".into(),
            Self::ListMcpResources { .. } => "listing MCP resources".into(),
            Self::ReadMcpResource { uri, .. } => format!("reading MCP resource: {uri}"),
            Self::Advisor {} => "consulting advisor".into(),
            Self::ConnectGitHub {} => "connecting GitHub".into(),
            Self::DesignProjectCreate { title } => format!("design project: {title}"),
            Self::DesignProjectList {} => "list design projects".into(),
            Self::DesignProjectSetMeta { project_id, .. } => {
                format!("design project meta: {project_id}")
            }
            Self::DesignListFiles { project_id } => format!("design files: {project_id}"),
            Self::DesignReadFile { project_id, path } => {
                format!("design read: {project_id}/{path}")
            }
            Self::DesignWriteFile {
                project_id, path, ..
            } => {
                format!("design write: {project_id}/{path}")
            }
            Self::DesignDeleteFile { project_id, path } => {
                format!("design delete: {project_id}/{path}")
            }
            Self::DesignCopyFile {
                project_id,
                from_path,
                to_path,
            } => format!("design copy: {project_id}/{from_path} -> {to_path}"),
            Self::DesignRegisterAsset {
                project_id,
                name,
                path,
            } => format!("design asset: {project_id}/{path} ({name})"),
            Self::DesignUnregisterAsset { project_id, path } => {
                format!("design unasset: {project_id}/{path}")
            }
            Self::DesignBundleHtml { input, output, .. } => match output {
                Some(output) => format!("bundle html: {input} -> {output}"),
                None => format!("bundle html: {input}"),
            },
            Self::DesignHandoff {
                project_dir,
                feature,
                ..
            } => format!("design handoff: {feature} in {project_dir}"),
            Self::DesignCheckSystem { project_dir } => {
                format!("check design system: {project_dir}")
            }
            Self::DesignCapabilities { .. } => "design capabilities".into(),
            Self::DesignServe {
                project_dir, file, ..
            } => match file {
                Some(file) => format!("serve design: {project_dir}/{file}"),
                None => format!("serve design: {project_dir}"),
            },
            Self::SetGoal { condition } => {
                if condition.trim().is_empty() {
                    "clear goal".into()
                } else {
                    let preview: String = condition.chars().take(60).collect();
                    format!("set goal: {preview}")
                }
            }
            Self::Research { question, .. } => {
                let preview: String = question.chars().take(60).collect();
                format!("research: {preview}")
            }
            Self::Council { question, .. } => {
                let preview: String = question.chars().take(60).collect();
                format!("council: {preview}")
            }
        }
    }

    pub fn from_value(tool_name: &str, value: serde_json::Value) -> Result<Self, ToolInputError> {
        let obj = match &value {
            serde_json::Value::Object(map) => Some(map),
            _ => None,
        };
        let json_type_name = |value: &serde_json::Value| -> &'static str {
            match value {
                serde_json::Value::Null => "null",
                serde_json::Value::Bool(_) => "bool",
                serde_json::Value::Number(_) => "number",
                serde_json::Value::String(_) => "string",
                serde_json::Value::Array(_) => "array",
                serde_json::Value::Object(_) => "object",
            }
        };
        let tool = || tool_name.to_owned();
        let req_str = |key: &'static str| -> Result<String, ToolInputError> {
            let Some(map) = obj else {
                return Err(ToolInputError::InvalidShape {
                    tool: tool(),
                    reason: "tool input was not a JSON object".into(),
                });
            };
            match map.get(key) {
                None | Some(serde_json::Value::Null) => Err(ToolInputError::MissingField {
                    tool: tool(),
                    field: key,
                }),
                Some(serde_json::Value::String(s)) => Ok(s.clone()),
                Some(other) => Err(ToolInputError::WrongType {
                    tool: tool(),
                    field: key,
                    expected: "string",
                    got: json_type_name(other),
                }),
            }
        };
        let opt_str_field = |key: &str| -> Option<String> {
            obj.and_then(|map| map.get(key))
                .and_then(|value| value.as_str())
                .map(str::to_owned)
        };
        let opt_u64_field = |key: &str| -> Option<u64> {
            obj.and_then(|map| map.get(key))
                .and_then(|value| value.as_u64())
        };
        let raw_bool_opt_field = |key: &str| -> Option<bool> {
            obj.and_then(|map| map.get(key))
                .and_then(|value| value.as_bool())
        };
        let opt_u32_field = |key: &str| -> Option<u32> {
            obj.and_then(|map| map.get(key))
                .and_then(|value| value.as_u64())
                .map(|value| value as u32)
        };
        let opt_u64_loose_field = |key: &str| -> Option<u64> {
            obj.and_then(|map| map.get(key)).and_then(|value| {
                value
                    .as_u64()
                    .or_else(|| value.as_str().and_then(|s| s.trim().parse::<u64>().ok()))
            })
        };
        let bool_field = |key: &str| -> bool {
            obj.and_then(|map| map.get(key))
                .and_then(|value| value.as_bool())
                .unwrap_or(false)
        };
        let req_str_one_of =
            |primary: &'static str, aliases: &[&'static str]| -> Result<String, ToolInputError> {
                let Some(map) = obj else {
                    return Err(ToolInputError::InvalidShape {
                        tool: tool(),
                        reason: "tool input was not a JSON object".into(),
                    });
                };
                let key = std::iter::once(primary)
                    .chain(aliases.iter().copied())
                    .find(|key| map.contains_key(*key))
                    .unwrap_or(primary);
                match map.get(key) {
                    None | Some(serde_json::Value::Null) => Err(ToolInputError::MissingField {
                        tool: tool(),
                        field: primary,
                    }),
                    Some(serde_json::Value::String(s)) => Ok(s.clone()),
                    Some(other) => Err(ToolInputError::WrongType {
                        tool: tool(),
                        field: key,
                        expected: "string",
                        got: json_type_name(other),
                    }),
                }
            };
        let kind = ToolKind::from_name(tool_name);
        let needs_object = !matches!(
            kind,
            ToolKind::Generic(_)
                | ToolKind::Mcp(_)
                | ToolKind::UnknownTool { .. }
                | ToolKind::ServerWebSearch
                | ToolKind::ServerCodeExecution
                | ToolKind::ServerAdvisor
        );
        if needs_object && obj.is_none() {
            return Err(ToolInputError::InvalidShape {
                tool: tool(),
                reason: format!(
                    "tool input must be a JSON object, got {}",
                    json_type_name(&value)
                ),
            });
        }
        let parsed = match kind {
            // ─── Bespoke arms: irregular parsing kept hand-written ───
            ToolKind::Edit => Self::Edit {
                file_path: req_str("file_path")?,
                old_string: req_str_one_of("old_string", &["old_str"])?,
                new_string: req_str_one_of("new_string", &["new_str"])?,
                replacement: ReplacementMode::from_replace_all(bool_field("replace_all")),
            },
            ToolKind::Read => Self::Read {
                file_path: req_str("file_path")?,
                offset: opt_u64_loose_field("offset"),
                limit: opt_u64_loose_field("limit"),
            },
            ToolKind::Bash => {
                let command = req_str("command")?;
                if command.is_empty() {
                    return Err(ToolInputError::InvalidShape {
                        tool: tool(),
                        reason: "Bash command must not be empty".into(),
                    });
                }
                Self::Bash {
                    command,
                    timeout: opt_u64_field("timeout"),
                    workdir: opt_str_field("workdir"),
                    run_in_background: raw_bool_opt_field("run_in_background"),
                }
            }
            ToolKind::TaskCreate => {
                // depends_on is an alias for blocked_by
                let blocked_by = obj
                    .and_then(|map| map.get("blocked_by").or_else(|| map.get("depends_on")))
                    .and_then(|value| value.as_array())
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|value| value.as_str().map(str::to_owned))
                            .collect()
                    })
                    .unwrap_or_default();
                let tags = obj
                    .and_then(|map| map.get("tags"))
                    .and_then(|value| value.as_array())
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|value| value.as_str().map(str::to_owned))
                            .collect()
                    })
                    .unwrap_or_default();
                let priority = obj
                    .and_then(|map| map.get("priority"))
                    .and_then(|v| v.as_u64())
                    .map(|v| v.min(9) as u8);
                let subject = opt_str_field("subject")
                    .or_else(|| opt_str_field("description"))
                    .ok_or_else(|| ToolInputError::MissingField {
                        tool: tool(),
                        field: "subject",
                    })?;
                let description = opt_str_field("description")
                    .or_else(|| opt_str_field("subject"))
                    .ok_or_else(|| ToolInputError::MissingField {
                        tool: tool(),
                        field: "description",
                    })?;
                Self::TaskCreate {
                    subject,
                    description,
                    active_form: opt_str_field("active_form"),
                    blocked_by,
                    acceptance_criteria: opt_str_field("acceptance_criteria"),
                    verification_command: opt_str_field("verification_command"),
                    risk: opt_str_field("risk"),
                    parent_id: opt_str_field("parent_id"),
                    kind: opt_str_field("kind"),
                    tags,
                    priority,
                    effort: opt_str_field("effort"),
                    model: opt_str_field("model"),
                }
            }
            ToolKind::TaskStop => Self::TaskStop {
                task_id: opt_str_field("task_id")
                    .or_else(|| opt_str_field("agentId"))
                    .or_else(|| opt_str_field("bash_id"))
                    .ok_or_else(|| ToolInputError::MissingField {
                        tool: tool(),
                        field: "task_id",
                    })?,
            },
            ToolKind::TaskValidate => Self::TaskValidate,
            ToolKind::Task => Self::Task(TaskInput {
                description: req_str("description")?,
                prompt: req_str("prompt")?,
                subagent_type: opt_str_field("subagent_type"),
                category: opt_str_field("category"),
                run_in_background: bool_field("run_in_background"),
                model: opt_str_field("model"),
                effort: opt_str_field("effort"),
                name: opt_str_field("name"),
                team_name: opt_str_field("team_name"),
                mode: opt_str_field("mode"),
                isolation: opt_str_field("isolation"),
                parent_task_id: opt_str_field("parent_task_id"),
                schema: obj.and_then(|m| m.get("schema")).cloned(),
            }),
            ToolKind::Skill => Self::Skill {
                name: opt_str_field("name")
                    .or_else(|| opt_str_field("skill"))
                    .ok_or_else(|| ToolInputError::MissingField {
                        tool: tool(),
                        field: "name",
                    })?,
                args: opt_str_field("args"),
            },
            ToolKind::TeamDelete => Self::TeamDelete,
            ToolKind::SendMessage => {
                let to = req_str("to")?;
                let message = match obj.and_then(|map| map.get("message")) {
                    None | Some(serde_json::Value::Null) => {
                        return Err(ToolInputError::MissingField {
                            tool: tool(),
                            field: "message",
                        });
                    }
                    Some(serde_json::Value::String(s)) => s.clone(),
                    Some(other) => other.to_string(),
                };
                Self::SendMessage {
                    to,
                    message,
                    summary: opt_str_field("summary"),
                }
            }
            ToolKind::Mcp(name) => Self::Mcp {
                name,
                arguments: value.clone(),
            },
            ToolKind::Lsp => {
                let operation = opt_str_field("kind")
                    .or_else(|| opt_str_field("operation"))
                    .ok_or_else(|| ToolInputError::MissingField {
                        tool: tool(),
                        field: "kind",
                    })?;
                Self::Lsp {
                    kind: normalize_lsp_operation(&operation).to_owned(),
                    file: opt_str_field("file")
                        .or_else(|| opt_str_field("filePath"))
                        .or_else(|| opt_str_field("path"))
                        .ok_or_else(|| ToolInputError::MissingField {
                            tool: tool(),
                            field: "file",
                        })?,
                    line: opt_u32_field("line").unwrap_or(0),
                    column: opt_u32_field("column")
                        .or_else(|| opt_u32_field("character"))
                        .unwrap_or(0),
                }
            }
            ToolKind::CronList => Self::CronList,
            ToolKind::ExitWorktree => Self::ExitWorktree,
            ToolKind::ServerWebSearch => Self::Generic {
                summary: obj
                    .and_then(|map| map.get("query"))
                    .and_then(|query| query.as_str())
                    .map(|query| format!("\u{1f50d} {query}"))
                    .unwrap_or_else(|| value.to_string()),
            },
            ToolKind::ServerCodeExecution => Self::Generic {
                summary: obj
                    .and_then(|map| map.get("code"))
                    .and_then(|code| code.as_str())
                    .map(|code| {
                        let preview: String = code.chars().take(120).collect();
                        format!("\u{26a1} {preview}")
                    })
                    .unwrap_or_else(|| value.to_string()),
            },
            ToolKind::ServerAdvisor => Self::Generic {
                summary: if value.is_object() && value.as_object().is_some_and(|map| map.is_empty())
                {
                    "advisor".to_owned()
                } else {
                    value.to_string()
                },
            },
            ToolKind::Generic(_) => Self::Generic {
                summary: value.to_string(),
            },
            ToolKind::UnknownTool { .. } => Self::Generic {
                summary: value.to_string(),
            },
            ToolKind::SendUserMessage => Self::SendUserMessage {
                message: opt_str_field("message").ok_or_else(|| ToolInputError::MissingField {
                    tool: tool(),
                    field: "message",
                })?,
                summary: opt_str_field("summary"),
                attachments: obj.and_then(|m| m.get("attachments")).cloned(),
                status: opt_str_field("status"),
            },
            ToolKind::SendUserFile => Self::SendUserFile {
                files: obj
                    .and_then(|m| m.get("files"))
                    .cloned()
                    .unwrap_or(serde_json::Value::Array(vec![])),
                caption: opt_str_field("caption"),
                status: opt_str_field("status"),
            },
            ToolKind::StructuredOutput => Self::StructuredOutput {
                data: value.clone(),
            },
            ToolKind::WaitForMcpServers => Self::WaitForMcpServers {
                timeout_ms: obj
                    .and_then(|m| m.get("timeout_ms"))
                    .and_then(|v| v.as_u64()),
            },
            ToolKind::ListMcpResources => Self::ListMcpResources {
                server: opt_str_field("server"),
            },
            ToolKind::ReadMcpResource => Self::ReadMcpResource {
                server: opt_str_field("server").unwrap_or_default(),
                uri: opt_str_field("uri").unwrap_or_default(),
            },
            ToolKind::Advisor => Self::Advisor {},
            ToolKind::SlashCommand => Self::SlashCommand {
                command: req_str("command")?,
                args: opt_str_field("args"),
            },
            ToolKind::ConnectGitHub => Self::ConnectGitHub {},
            ToolKind::TaskUpdate => {
                // depends_on is an alias for blocked_by
                let blocked_by = obj
                    .and_then(|map| map.get("blocked_by").or_else(|| map.get("depends_on")))
                    .and_then(|value| value.as_array())
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|value| value.as_str().map(str::to_owned))
                            .collect()
                    })
                    .unwrap_or_default();
                let tags = obj
                    .and_then(|map| map.get("tags"))
                    .and_then(|value| value.as_array())
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|value| value.as_str().map(str::to_owned))
                            .collect()
                    })
                    .unwrap_or_default();
                let priority = obj
                    .and_then(|map| map.get("priority"))
                    .and_then(|v| v.as_u64())
                    .map(|v| v.min(9) as u8);
                Self::TaskUpdate {
                    task_id: req_str("task_id")?,
                    status: opt_str_field("status"),
                    subject: opt_str_field("subject"),
                    description: opt_str_field("description"),
                    owner: opt_str_field("owner"),
                    acceptance_criteria: opt_str_field("acceptance_criteria"),
                    verification_command: opt_str_field("verification_command"),
                    risk: opt_str_field("risk"),
                    parent_id: opt_str_field("parent_id"),
                    kind: opt_str_field("kind"),
                    blocked_by,
                    tags,
                    priority,
                    effort: opt_str_field("effort"),
                    model: opt_str_field("model"),
                }
            }
            // ─── Regular kinds: parsed by the table-generated fn ───
            other => {
                for_each_regular_tool_input!(gen_regular_from_value);
                match parse_regular(&other, obj, &tool) {
                    Some(result) => result?,
                    None => {
                        return Err(ToolInputError::InvalidShape {
                            tool: tool(),
                            reason: format!("unhandled tool kind: {other:?}"),
                        });
                    }
                }
            }
        };
        Ok(parsed)
    }

    /// Parse with a coercion pre-pass, recovering from the common shape mistakes
    /// models make rather than hard-failing. Mirrors Claude Code 2.1.170's
    /// `tengu_tool_input_coerced`: when the raw args don't match the schema, we
    /// normalize the JSON first (alias common key typos, unwrap a single-key
    /// wrapper, scalar↔array) and re-parse.
    ///
    /// Returns the parsed input plus a [`CoercionOutcome`] describing what was
    /// done, so the caller can log a `tool_input_coerced` event. The happy path
    /// (`from_value` already succeeds) yields [`CoercionOutcome::Unchanged`] and
    /// is byte-for-byte the existing behavior — coercion only runs on failure.
    pub fn from_value_coerced(
        tool_name: &str,
        value: serde_json::Value,
    ) -> (Result<Self, ToolInputError>, CoercionOutcome) {
        // Fast path: already valid → no coercion.
        if let Ok(parsed) = Self::from_value(tool_name, value.clone()) {
            return (Ok(parsed), CoercionOutcome::Unchanged);
        }
        // Slow path: try to coerce, then re-parse.
        let serde_json::Value::Object(map) = value else {
            // Non-object inputs can't be coerced field-wise; report the original error.
            return (
                Self::from_value(tool_name, value),
                CoercionOutcome::Rejected,
            );
        };
        let (coerced, shape) = coerce_object(&map);
        if shape.is_empty() {
            // Nothing to coerce — the failure is genuine.
            return (
                Self::from_value(tool_name, serde_json::Value::Object(map)),
                CoercionOutcome::Rejected,
            );
        }
        match Self::from_value(tool_name, serde_json::Value::Object(coerced)) {
            Ok(parsed) => (Ok(parsed), CoercionOutcome::Coerced { shape }),
            // Coercion didn't make it parseable — surface the original-shape error.
            Err(e) => (Err(e), CoercionOutcome::CoercedStillInvalid { shape }),
        }
    }

    pub fn to_value(&self) -> serde_json::Value {
        use serde_json::json;
        match self {
            // ─── Bespoke serialize arms (not table-driven) ───
            Self::TaskValidate => json!({}),
            Self::Task(task_input) => {
                let mut value = json!({
                    "description": task_input.description,
                    "prompt": task_input.prompt,
                    "run_in_background": task_input.run_in_background,
                });
                if let Some(subagent_type) = &task_input.subagent_type {
                    value["subagent_type"] = json!(subagent_type);
                }
                if let Some(category) = &task_input.category {
                    value["category"] = json!(category);
                }
                if let Some(model) = &task_input.model {
                    value["model"] = json!(model);
                }
                if let Some(effort) = &task_input.effort {
                    value["effort"] = json!(effort);
                }
                if let Some(parent_task_id) = &task_input.parent_task_id {
                    value["parent_task_id"] = json!(parent_task_id);
                }
                value
            }
            Self::TeamDelete => json!({}),
            Self::Mcp { arguments, .. } => arguments.clone(),
            Self::CronList => json!({}),
            Self::ExitWorktree => json!({}),
            Self::Generic { summary } => match serde_json::from_str::<serde_json::Value>(summary) {
                Ok(serde_json::Value::Object(map)) => serde_json::Value::Object(map),
                Ok(_) | Err(_) => json!({ "input": summary }),
            },
            Self::SendUserMessage {
                message,
                summary,
                attachments,
                status,
            } => json!({
                "message": message,
                "summary": summary,
                "attachments": attachments,
                "status": status,
            }),
            Self::SendUserFile {
                files,
                caption,
                status,
            } => json!({
                "files": files,
                "caption": caption,
                "status": status,
            }),
            Self::StructuredOutput { data } => data.clone(),
            Self::WaitForMcpServers { timeout_ms } => json!({ "timeout_ms": timeout_ms }),
            Self::ListMcpResources { server } => json!({ "server": server }),
            Self::ReadMcpResource { server, uri } => json!({ "server": server, "uri": uri }),
            Self::Advisor {} => json!({}),
            Self::SlashCommand { command, args } => json!({ "command": command, "args": args }),
            Self::ConnectGitHub {} => json!({}),
            Self::TaskUpdate {
                task_id,
                status,
                subject,
                description,
                owner,
                acceptance_criteria,
                verification_command,
                risk,
                parent_id,
                kind,
                blocked_by,
                tags,
                priority,
                effort,
                model,
            } => {
                let mut value = json!({
                    "task_id": task_id,
                });
                if let Some(s) = status {
                    value["status"] = json!(s);
                }
                if let Some(s) = subject {
                    value["subject"] = json!(s);
                }
                if let Some(s) = description {
                    value["description"] = json!(s);
                }
                if let Some(s) = owner {
                    value["owner"] = json!(s);
                }
                if let Some(s) = acceptance_criteria {
                    value["acceptance_criteria"] = json!(s);
                }
                if let Some(s) = verification_command {
                    value["verification_command"] = json!(s);
                }
                if let Some(s) = risk {
                    value["risk"] = json!(s);
                }
                if let Some(s) = parent_id {
                    value["parent_id"] = json!(s);
                }
                if let Some(s) = kind {
                    value["kind"] = json!(s);
                }
                if !blocked_by.is_empty() {
                    value["blocked_by"] = json!(blocked_by);
                }
                if !tags.is_empty() {
                    value["tags"] = json!(tags);
                }
                if let Some(p) = priority {
                    value["priority"] = json!(p);
                }
                if let Some(s) = effort {
                    value["effort"] = json!(s);
                }
                if let Some(s) = model {
                    value["model"] = json!(s);
                }
                value
            }
            // ─── Regular variants: serialized by the two table-generated fns ───
            // (split into two tables — main table is parse+serialize, the
            // supplementary table is serialize-only for variants whose
            // parsing is bespoke but serialization is still rule-driven).
            other => {
                for_each_regular_tool_input!(gen_regular_to_value);
                gen_serialize_extra!();
                if let Some(v) = serialize_regular(other) {
                    return v;
                }
                if let Some(v) = serialize_extra(other) {
                    return v;
                }
                unreachable!("variant must be in one of the two serialize tables: {other:?}")
            }
        }
    }
}

/// Result of [`ToolInput::from_value_coerced`]. Mirrors Claude Code 2.1.170's
/// coercion outcomes (`unchanged` / `coerced` / `coerced_still` / `rejected`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CoercionOutcome {
    /// The raw args already matched the schema; nothing was changed.
    Unchanged,
    /// Coercion was applied and the result parsed. `shape` lists the fixes
    /// (e.g. `["alias_file→file_path", "wrap_array:edits"]`) for telemetry.
    Coerced { shape: Vec<String> },
    /// Coercion was applied but the result still didn't parse.
    CoercedStillInvalid { shape: Vec<String> },
    /// Input was un-coercible (not an object, or no known fix applied).
    Rejected,
}

impl CoercionOutcome {
    /// Stable label for a `tool_input_coerced` telemetry event.
    pub fn label(&self) -> &'static str {
        match self {
            CoercionOutcome::Unchanged => "unchanged",
            CoercionOutcome::Coerced { .. } => "coerced",
            CoercionOutcome::CoercedStillInvalid { .. } => "coerced_still",
            CoercionOutcome::Rejected => "rejected",
        }
    }

    /// The `+`-joined shape classes (CC's `shapeClass`), empty when none.
    pub fn shape_class(&self) -> String {
        match self {
            CoercionOutcome::Coerced { shape } | CoercionOutcome::CoercedStillInvalid { shape } => {
                shape.join("+")
            }
            _ => String::new(),
        }
    }
}

/// Common key aliases models emit instead of the canonical schema key. Each
/// entry is `(canonical, &[aliases])`. When the canonical key is absent but an
/// alias is present (and the canonical isn't), we rename it. Mirrors the alias
/// table CC applies in its TaskCreate/TodoWrite coercion (`alias_<key>`).
const FIELD_ALIASES: &[(&str, &[&str])] = &[
    ("file_path", &["filepath", "path", "filename", "file"]),
    ("command", &["cmd", "bash", "script"]),
    ("pattern", &["query", "regex", "search"]),
    ("content", &["text", "body", "data"]),
    ("old_string", &["old", "search", "from"]),
    ("new_string", &["new", "replace", "to"]),
    ("subject", &["title", "name", "summary"]),
    ("description", &["desc", "details", "body"]),
    ("prompt", &["instructions", "task", "message"]),
    ("url", &["uri", "link", "href"]),
];

/// Keys whose value is frequently a single object/string when the schema wants
/// an array — wrap them. Mirrors CC's scalar→array coercion.
const WRAP_ARRAY_KEYS: &[&str] = &["edits", "questions", "todos", "tasks", "files"];

/// Coerce a raw tool-input object toward the schema. Returns the new object plus
/// a list of applied "shape classes" (empty = nothing changed). This is
/// intentionally conservative: it only renames absent-canonical aliases, unwraps
/// a single-key wrapper, and wraps known array fields — never drops data.
fn coerce_object(
    map: &serde_json::Map<String, serde_json::Value>,
) -> (serde_json::Map<String, serde_json::Value>, Vec<String>) {
    use serde_json::Value;
    let mut out = map.clone();
    let mut shape = Vec::new();

    // 1. Unwrap a single-key wrapper object: `{"input": {...}}` / `{"args": {...}}`
    //    / `{"params": {...}}` → the inner object. (CC's `task_wrapper_object`.)
    if out.len() == 1 {
        for wrapper in ["input", "args", "arguments", "params", "tool_input"] {
            if let Some(Value::Object(inner)) = out.get(wrapper) {
                let inner = inner.clone();
                out = inner;
                shape.push(format!("unwrap:{wrapper}"));
                break;
            }
        }
    }

    // 1b. Unwrap a single-key wrapper whose value is a STRINGIFIED JSON
    //     object — e.g. `{"command": "{\"task_id\":\"bash_…\"}"}`, seen when
    //     a model double-encodes args under whatever key it last used. Safe
    //     for any key (including legit string fields like Bash's `command`):
    //     coercion only runs after `from_value` already rejected the input,
    //     and the unwrapped result must re-parse against the tool schema or
    //     the original-shape error is surfaced instead.
    if out.len() == 1
        && let Some((key, Value::String(s))) = out.iter().next().map(|(k, v)| (k.clone(), v))
        && let Ok(Value::Object(inner)) = serde_json::from_str::<Value>(s)
    {
        out = inner;
        shape.push(format!("unwrap_str:{key}"));
    }

    // 2. Rename aliases where the canonical key is missing.
    for (canonical, aliases) in FIELD_ALIASES {
        if out.contains_key(*canonical) {
            continue;
        }
        for alias in *aliases {
            if let Some(v) = out.remove(*alias) {
                out.insert((*canonical).to_string(), v);
                shape.push(format!("alias_{alias}->{canonical}"));
                break;
            }
        }
    }

    // 3. Wrap scalar/object values for known array fields.
    for key in WRAP_ARRAY_KEYS {
        if let Some(v) = out.get(*key) {
            if !v.is_array() && !v.is_null() {
                let wrapped = Value::Array(vec![v.clone()]);
                out.insert((*key).to_string(), wrapped);
                shape.push(format!("wrap_array:{key}"));
            }
        }
    }

    (out, shape)
}

fn split_advertised_mcp(name: &str) -> Option<(&str, &str)> {
    let rest = name.strip_prefix("mcp__")?;
    let (server, tool) = rest.split_once("__")?;
    if server.is_empty() || tool.is_empty() {
        None
    } else {
        Some((server, tool))
    }
}

fn normalize_lsp_operation(operation: &str) -> &str {
    match operation {
        "goToDefinition" | "gotoDefinition" | "definition" => "definition",
        "findReferences" | "references" => "references",
        "goToImplementation" | "implementation" => "implementation",
        "goToTypeDefinition" | "typeDefinition" | "type_definition" => "type_definition",
        "documentSymbol" | "documentSymbols" | "document_symbols" => "document_symbols",
        "workspaceSymbol" | "workspaceSymbols" | "workspace_symbols" => "workspace_symbols",
        "incomingCalls" | "incoming_calls" => "incoming_calls",
        "outgoingCalls" | "outgoing_calls" => "outgoing_calls",
        "prepareCallHierarchy" | "prepare_call_hierarchy" => "incoming_calls",
        "hover" => "hover",
        other => other,
    }
}

fn generic_summary_display(summary: &str) -> String {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(summary) else {
        return summary.to_owned();
    };
    if let (Some(kind), Some(query), Some(path)) = (
        value.get("kind").and_then(|v| v.as_str()),
        value.get("query").and_then(|v| v.as_str()),
        value.get("path").and_then(|v| v.as_str()),
    ) {
        return format!("code index (kind={kind}, query={query}, path={path})");
    }
    if let Some(query) = value.get("query").and_then(|v| v.as_str())
        && value.get("max_tokens").is_some()
    {
        return query.to_owned();
    }
    if let Some(path) = value.get("lcov_path").and_then(|v| v.as_str()) {
        return format!("coverage({path})");
    }
    if let Some(handle) = value.get("handle").and_then(|v| v.as_str())
        && value.get("new_content").is_some()
    {
        return format!("edit: {handle}");
    }
    summary.to_owned()
}

#[cfg(test)]
mod macro_equivalence_tests {
    use super::*;
    use serde_json::json;

    /// Representative input for every ToolKind that has a from_value mapping.
    /// Drives from_value → to_value → summary so a refactor that changes any
    /// field name, key, parse rule, or serialize stanza is caught.
    fn cases() -> Vec<(&'static str, serde_json::Value)> {
        vec![
            (
                "Edit",
                json!({"file_path":"a.rs","old_string":"x","new_string":"y","replace_all":true}),
            ),
            ("Write", json!({"file_path":"a.rs","content":"c"})),
            ("Read", json!({"file_path":"a.rs","offset":3,"limit":9})),
            (
                "Bash",
                json!({"command":"ls","timeout":500,"workdir":"/tmp"}),
            ),
            ("Glob", json!({"pattern":"*.rs","path":"src"})),
            (
                "Grep",
                json!({"pattern":"fn","path":"src","glob":"*.rs","output_mode":"content"}),
            ),
            ("Search", json!({"query":"foo","path":"src"})),
            ("ApplyPatch", json!({"patch":"diff"})),
            (
                "TaskCreate",
                json!({"subject":"s","description":"d","blocked_by":["t1"],"risk":"low"}),
            ),
            (
                "TaskUpdate",
                json!({"task_id":"t1","status":"done","owner":"me"}),
            ),
            ("TaskList", json!({"status_filter":"pending"})),
            ("TaskDone", json!({"task_id":"t1"})),
            ("TaskStop", json!({"task_id":"t1"})),
            ("TaskGet", json!({"task_id":"t1"})),
            ("TaskValidate", json!({})),
            (
                "Task",
                json!({"description":"d","prompt":"p","run_in_background":true,"subagent_type":"explore"}),
            ),
            ("Skill", json!({"name":"sk","args":"a"})),
            ("ToolSearch", json!({"query":"q","limit":5})),
            ("ToolSuggest", json!({"intent":"i","limit":5})),
            (
                "MemoryCreate",
                json!({"level":"user","memory_type":"pref","scope":"private","body":"b"}),
            ),
            ("MemoryDelete", json!({"path":"/m"})),
            ("TeamCreate", json!({"team_name":"t","description":"d"})),
            ("TeamDelete", json!({})),
            ("SendMessage", json!({"to":"a","message":"m","summary":"s"})),
            ("TeamMemberMode", json!({"member_name":"a","mode":"plan"})),
            (
                "code_index",
                json!({"path":"src","query":"q","kind":"function","max_entries":10}),
            ),
            (
                "graph_query",
                json!({"query":"fn(\"x\")","max_tokens":2000,"include_handles":false}),
            ),
            (
                "run_coverage",
                json!({"lcov_path":"/c","include_untested_list":false}),
            ),
            (
                "symbol_edit",
                json!({"handle":"fn:x","new_content":"...","validate":true,"dispatch_cascade":true}),
            ),
            (
                "post_bounty",
                json!({"description":"d","budget":100,"acceptance_criteria":"ac","max_solvers":3,"auto_dispatch":true}),
            ),
            ("market_status", json!({"bounty_id":"b1"})),
            ("run_bounty", json!({"bounty_id":"b1","max_solvers":2})),
            ("ExitPlanMode", json!({"plan":"p"})),
            (
                "MultiEdit",
                json!({"file_path":"a.rs","edits":[{"old":"x"}]}),
            ),
            (
                "AskUserQuestion",
                json!({"question":"q?","options":[{"label":"a"}],"multi_select":true}),
            ),
            ("WebFetch", json!({"url":"http://x","prompt":"p"})),
            ("WebSearch", json!({"query":"q","max_results":5})),
            (
                "CronCreate",
                json!({"schedule":"@daily","command":"c","description":"d"}),
            ),
            ("CronList", json!({})),
            ("CronDelete", json!({"id":"j1"})),
            (
                "ScheduleWakeup",
                json!({"delay_seconds":60,"prompt":"p","reason":"r"}),
            ),
            ("Monitor", json!({"command":"c","until":"done"})),
            (
                "LSP",
                json!({"kind":"hover","file":"a.rs","line":3,"column":5}),
            ),
            ("PushNotification", json!({"message":"m","title":"t"})),
            (
                "RemoteTrigger",
                json!({"trigger_id":"ci","payload":{"x":1}}),
            ),
            ("EnterPlanMode", json!({"reason":"r"})),
            ("SetGoal", json!({"condition":"all tests pass"})),
            ("EnterWorktree", json!({"name":"w","branch":"b"})),
            ("ExitWorktree", json!({})),
            ("NotebookRead", json!({"path":"n.ipynb"})),
            (
                "NotebookEdit",
                json!({"path":"n.ipynb","cell_id":"c1","new_source":"s","edit_mode":"insert"}),
            ),
            ("ScratchpadRead", json!({"key":"k"})),
            ("ScratchpadWrite", json!({"key":"k","value":"v"})),
            (
                "SlashCommand",
                json!({"command":"research","args":"how do monads work"}),
            ),
            ("DesignProjectCreate", json!({"title":"Deck"})),
            ("DesignProjectList", json!({})),
            (
                "DesignProjectSetMeta",
                json!({"project_id":"p1","title":"Deck 2","is_design_system":true}),
            ),
            ("DesignListFiles", json!({"project_id":"p1"})),
            (
                "DesignReadFile",
                json!({"project_id":"p1","path":"index.html"}),
            ),
            (
                "DesignWriteFile",
                json!({"project_id":"p1","path":"index.html","content":"<h1>x</h1>","asset_name":"Landing"}),
            ),
            (
                "DesignDeleteFile",
                json!({"project_id":"p1","path":"old.html"}),
            ),
            (
                "DesignCopyFile",
                json!({"project_id":"p1","from_path":"a.html","to_path":"b.html"}),
            ),
            (
                "DesignRegisterAsset",
                json!({"project_id":"p1","name":"Landing","path":"index.html"}),
            ),
            (
                "DesignUnregisterAsset",
                json!({"project_id":"p1","path":"index.html"}),
            ),
            (
                "DesignBundleHtml",
                json!({"input":"index.html","output":"standalone.html","require_thumbnail":false}),
            ),
            (
                "DesignHandoff",
                json!({"project_dir":".","feature":"Onboarding","files":["index.html"]}),
            ),
            ("DesignCheckSystem", json!({"project_dir":"."})),
            ("DesignCapabilities", json!({"format":"markdown"})),
            (
                "DesignServe",
                json!({"project_dir":".","port":4321,"file":"index.html"}),
            ),
        ]
    }

    #[test]
    fn tool_input_round_trip_snapshot_is_stable() {
        let mut snapshot = String::new();
        for (name, input) in cases() {
            let parsed = ToolInput::from_value(name, input.clone())
                .unwrap_or_else(|e| panic!("from_value({name}) failed: {e}"));
            let serialized = parsed.to_value();
            let summary = parsed.summary();
            snapshot.push_str(&format!(
                "{name}\n  to_value={}\n  summary={summary}\n",
                serde_json::to_string(&serialized).unwrap()
            ));
        }
        // Locked snapshot of current behavior. If the macro refactor changes
        // any field name / JSON key / parse rule / serialize stanza / summary
        // template, this digest changes and the test fails.
        let expected = include_str!("tool_input_snapshot.txt");
        assert_eq!(
            snapshot.trim(),
            expected.trim(),
            "tool_input behavior changed. If intentional, regenerate \
            tool_input_snapshot.txt by temporarily swapping this assert for \
             a std::fs::write of `snapshot.trim()`."
        );
    }

    // Each alias case is a separate test so a failure names the exact broken alias.

    #[test]
    fn edit_old_str_new_str_aliases_parse_normal() {
        assert!(matches!(
            ToolInput::from_value(
                "Edit",
                json!({"file_path":"a.rs","old_str":"old","new_str":"new"})
            )
            .unwrap(),
            ToolInput::Edit { ref old_string, ref new_string, .. }
                if old_string == "old" && new_string == "new"
        ));
    }

    #[test]
    fn read_string_offset_limit_coerced_normal() {
        assert!(matches!(
            ToolInput::from_value(
                "Read",
                json!({"file_path":"a.rs","offset":"12","limit":"34"})
            )
            .unwrap(),
            ToolInput::Read {
                offset: Some(12),
                limit: Some(34),
                ..
            }
        ));
    }

    #[test]
    fn task_stop_agent_id_alias_parses_normal() {
        assert!(matches!(
            ToolInput::from_value("TaskStop", json!({"agentId":"agent-1"})).unwrap(),
            ToolInput::TaskStop { ref task_id } if task_id == "agent-1"
        ));
    }

    #[test]
    fn task_stop_bash_id_alias_parses_normal() {
        assert!(matches!(
            ToolInput::from_value("TaskStop", json!({"bash_id":"bash-1"})).unwrap(),
            ToolInput::TaskStop { ref task_id } if task_id == "bash-1"
        ));
    }

    #[test]
    fn task_update_depends_on_alias_parses_normal() {
        assert!(matches!(
            ToolInput::from_value(
                "TaskUpdate",
                json!({"task_id":"t1","depends_on":["t2","t3"]})
            )
            .unwrap(),
            ToolInput::TaskUpdate { ref task_id, ref blocked_by, .. }
                if task_id == "t1"
                    && *blocked_by == vec!["t2".to_string(), "t3".to_string()]
        ));
    }

    #[test]
    fn task_update_blocked_by_primary_field_parses_normal() {
        assert!(matches!(
            ToolInput::from_value(
                "TaskUpdate",
                json!({"task_id":"t1","blocked_by":["t4","t5"]})
            )
            .unwrap(),
            ToolInput::TaskUpdate { ref task_id, ref blocked_by, .. }
                if task_id == "t1"
                    && *blocked_by == vec!["t4".to_string(), "t5".to_string()]
        ));
    }

    #[test]
    fn task_update_blocked_by_wins_over_depends_on_robust() {
        // When both are supplied, blocked_by takes precedence.
        assert!(matches!(
            ToolInput::from_value(
                "TaskUpdate",
                json!({"task_id":"t1","blocked_by":["t6"],"depends_on":["t7"]})
            )
            .unwrap(),
            ToolInput::TaskUpdate { ref task_id, ref blocked_by, .. }
                if task_id == "t1" && *blocked_by == vec!["t6".to_string()]
        ));
    }

    // ─── SetGoal (model-invocable goal) ─────────────────────────────────────

    // Normal: SetGoal parses by name and round-trips through to_value.
    #[test]
    fn set_goal_parses_and_serializes_normal() {
        let input =
            ToolInput::from_value("SetGoal", json!({"condition": "all tests pass"})).unwrap();
        assert!(
            matches!(input, ToolInput::SetGoal { ref condition } if condition == "all tests pass")
        );
        assert_eq!(input.to_value()["condition"], json!("all tests pass"));
        assert!(input.summary().contains("set goal"));
    }

    // Robust: the tool name resolves through ToolKind (incl. snake_case alias).
    #[test]
    fn set_goal_kind_resolves_robust() {
        assert_eq!(ToolKind::from_name("SetGoal"), ToolKind::SetGoal);
        assert_eq!(ToolKind::from_name("set_goal"), ToolKind::SetGoal);
        assert_eq!(ToolKind::SetGoal.api_name(), "set_goal");
    }

    // ─── Research / Council (model-invocable) ───────────────────────────────

    #[test]
    fn research_parses_and_serializes_normal() {
        let input = ToolInput::from_value(
            "Research",
            json!({"question": "rust async", "export": true}),
        )
        .unwrap();
        assert!(
            matches!(input, ToolInput::Research { ref question, export } if question == "rust async" && export)
        );
        assert_eq!(input.to_value()["question"], json!("rust async"));
        assert_eq!(input.to_value()["export"], json!(true));
        assert!(input.summary().contains("research"));
    }

    #[test]
    fn research_export_defaults_false_robust() {
        let input = ToolInput::from_value("research", json!({"question": "q"})).unwrap();
        assert!(matches!(input, ToolInput::Research { export: false, .. }));
        assert_eq!(ToolKind::from_name("deep_research"), ToolKind::Research);
        assert_eq!(ToolKind::Research.api_name(), "research");
    }

    #[test]
    fn council_parses_and_serializes_normal() {
        let input = ToolInput::from_value(
            "Council",
            json!({"question": "which db?", "models": ["a", "b"]}),
        )
        .unwrap();
        assert!(
            matches!(input, ToolInput::Council { ref question, ref models } if question == "which db?" && models.len() == 2)
        );
        assert_eq!(input.to_value()["models"], json!(["a", "b"]));
        assert!(input.summary().contains("council"));
        assert_eq!(ToolKind::from_name("model_council"), ToolKind::Council);
    }

    #[test]
    fn council_models_default_empty_robust() {
        let input = ToolInput::from_value("council", json!({"question": "q"})).unwrap();
        assert!(matches!(input, ToolInput::Council { ref models, .. } if models.is_empty()));
    }

    // ─── tool_input_coerced (CC 2.1.170 parity) ─────────────────────────────

    // Normal: valid input is passed through untouched (Unchanged), no coercion.
    #[test]
    fn coerce_valid_input_is_unchanged_normal() {
        let (parsed, outcome) =
            ToolInput::from_value_coerced("Read", json!({ "file_path": "src/main.rs" }));
        assert!(parsed.is_ok());
        assert_eq!(outcome, CoercionOutcome::Unchanged);
        assert_eq!(outcome.label(), "unchanged");
    }

    // Normal: a common key alias (`path` instead of `file_path`) is renamed and
    // the input then parses, with the shape class recorded.
    #[test]
    fn coerce_aliases_path_to_file_path_normal() {
        let (parsed, outcome) =
            ToolInput::from_value_coerced("Read", json!({ "path": "src/main.rs" }));
        assert!(
            matches!(parsed, Ok(ToolInput::Read { ref file_path, .. }) if file_path == "src/main.rs")
        );
        assert_eq!(outcome.label(), "coerced");
        assert!(outcome.shape_class().contains("alias_path->file_path"));
    }

    // Robust: a single-key wrapper object (`{"input": {...}}`) is unwrapped.
    #[test]
    fn coerce_unwraps_input_wrapper_robust() {
        let (parsed, outcome) =
            ToolInput::from_value_coerced("Read", json!({ "input": { "file_path": "a.rs" } }));
        assert!(parsed.is_ok());
        assert!(outcome.shape_class().contains("unwrap:input"));
    }

    // Robust: a genuinely wrong input that no fix can repair stays an error and
    // is reported as Rejected — coercion never masks a real schema violation.
    #[test]
    fn coerce_uncoercible_input_is_rejected_robust() {
        let (parsed, outcome) = ToolInput::from_value_coerced("Read", json!({ "nonsense": 42 }));
        assert!(parsed.is_err());
        assert!(matches!(
            outcome,
            CoercionOutcome::Rejected | CoercionOutcome::CoercedStillInvalid { .. }
        ));
    }

    // Regression: a model double-encoding args as a stringified JSON object
    // under a stray single key — observed live as BashOutput
    // `{"command": "{\"task_id\":\"bash_…\"}"}` failing with
    // "missing required field `task_id`". The string payload is parsed and
    // unwrapped, then must satisfy the real schema.
    #[test]
    fn coerce_unwraps_stringified_json_wrapper_regression() {
        let (parsed, outcome) = ToolInput::from_value_coerced(
            "BashOutput",
            json!({ "command": "{\"task_id\":\"bash_115a3e377b46\"}" }),
        );
        let parsed = parsed.expect("stringified wrapper should coerce");
        assert!(matches!(
            parsed,
            ToolInput::BashOutput { ref task_id, .. } if task_id == "bash_115a3e377b46"
        ));
        assert!(outcome.shape_class().contains("unwrap_str:command"));
    }

    // Robust: a legit string field that happens to contain JSON must NOT be
    // unwrapped when the input is already schema-valid — Bash's `command`
    // can legitimately be `{"foo": 1}` (e.g. echoing JSON).
    #[test]
    fn coerce_leaves_valid_json_command_string_alone_robust() {
        let (parsed, outcome) =
            ToolInput::from_value_coerced("Bash", json!({ "command": "{\"foo\": 1}" }));
        let parsed = parsed.expect("valid Bash input must parse unchanged");
        assert!(matches!(
            parsed,
            ToolInput::Bash { ref command, .. } if command == "{\"foo\": 1}"
        ));
        assert!(matches!(outcome, CoercionOutcome::Unchanged));
    }
}

use crate::{TaskInput, ToolKind};

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
    },
    TaskList {
        status_filter: Option<String>,
        owner_filter: Option<String>,
    },
    TaskDone {
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
    CodeIndex {
        #[serde(default)]
        path: Option<String>,
        #[serde(default)]
        query: Option<String>,
        #[serde(default)]
        kind: Option<String>,
        #[serde(default)]
        max_entries: Option<usize>,
    },
    GraphQuery {
        query: String,
        max_tokens: Option<usize>,
        #[serde(default)]
        include_handles: Option<bool>,
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
    RunCoverage {
        #[serde(default)]
        lcov_path: Option<String>,
        include_untested_list: bool,
    },
    SymbolEdit {
        handle: String,
        new_content: String,
        #[serde(default)]
        validate: bool,
        #[serde(default, rename = "dispatch_cascade")]
        dispatch_cascade: bool,
    },
    ExitPlanMode {
        plan: String,
    },
    MultiEdit {
        file_path: String,
        edits: serde_json::Value,
    },
    AskUserQuestion {
        question: String,
        options: serde_json::Value,
        multi_select: bool,
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
            Self::TaskList { status_filter, .. } => match status_filter {
                Some(f) => format!("list tasks ({f})"),
                None => "list tasks".into(),
            },
            Self::TaskDone { task_id } => format!("done: {task_id}"),
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
            Self::CodeIndex {
                path, query, kind, ..
            } => {
                let mut parts = Vec::new();
                if let Some(kind) = kind.as_deref().filter(|s| !s.is_empty()) {
                    parts.push(format!("kind={kind}"));
                }
                if let Some(query) = query.as_deref().filter(|s| !s.is_empty()) {
                    parts.push(format!("query={query}"));
                }
                if let Some(path) = path.as_deref().filter(|s| !s.is_empty()) {
                    parts.push(format!("path={path}"));
                }
                if parts.is_empty() {
                    "code index".into()
                } else {
                    format!("code index ({})", parts.join(", "))
                }
            }
            Self::GraphQuery { query, .. } => query.clone(),
            Self::RunCoverage { lcov_path, .. } => {
                format!("coverage({})", lcov_path.as_deref().unwrap_or("auto"))
            }
            Self::SymbolEdit { handle, .. } => format!("edit: {handle}"),
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
            Self::MultiEdit { file_path, edits } => {
                let count = edits.as_array().map(|a| a.len()).unwrap_or(0);
                format!(
                    "{file_path} ({count} edit{})",
                    if count == 1 { "" } else { "s" }
                )
            }
            Self::AskUserQuestion { question, .. } => {
                format!("ask: {}", question.chars().take(60).collect::<String>())
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
            Self::Generic { summary } => summary.clone(),
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
        let bool_field = |key: &str| -> bool {
            obj.and_then(|map| map.get(key))
                .and_then(|value| value.as_bool())
                .unwrap_or(false)
        };
        let kind = ToolKind::from_name(tool_name);
        let needs_object = !matches!(
            kind,
            ToolKind::Generic(_)
                | ToolKind::Mcp(_)
                | ToolKind::UnknownTool { .. }
                | ToolKind::ServerWebSearch
                | ToolKind::ServerCodeExecution
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
            ToolKind::Edit => Self::Edit {
                file_path: req_str("file_path")?,
                old_string: req_str("old_string")?,
                new_string: req_str("new_string")?,
                replacement: ReplacementMode::from_replace_all(bool_field("replace_all")),
            },
            ToolKind::Write => Self::Write {
                file_path: req_str("file_path")?,
                content: req_str("content")?,
            },
            ToolKind::Read => Self::Read {
                file_path: req_str("file_path")?,
                offset: opt_u64_field("offset"),
                limit: opt_u64_field("limit"),
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
                }
            }
            ToolKind::Glob => Self::Glob {
                pattern: req_str("pattern")?,
                path: opt_str_field("path"),
            },
            ToolKind::Grep => Self::Grep {
                pattern: req_str("pattern")?,
                path: opt_str_field("path"),
                glob: opt_str_field("glob"),
                output_mode: opt_str_field("output_mode"),
            },
            ToolKind::Search => Self::Search {
                query: req_str("query")?,
                path: opt_str_field("path"),
            },
            ToolKind::ApplyPatch => Self::ApplyPatch {
                patch: req_str("patch")?,
            },
            ToolKind::TaskCreate => {
                let blocked_by = obj
                    .and_then(|map| map.get("blocked_by"))
                    .and_then(|value| value.as_array())
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|value| value.as_str().map(str::to_owned))
                            .collect()
                    })
                    .unwrap_or_default();
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
                }
            }
            ToolKind::TaskUpdate => Self::TaskUpdate {
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
            },
            ToolKind::TaskList => Self::TaskList {
                status_filter: opt_str_field("status_filter"),
                owner_filter: opt_str_field("owner_filter"),
            },
            ToolKind::TaskDone => Self::TaskDone {
                task_id: req_str("task_id")?,
            },
            ToolKind::TaskGet => Self::TaskGet {
                task_id: req_str("task_id")?,
            },
            ToolKind::TaskValidate => Self::TaskValidate,
            ToolKind::Task => Self::Task(TaskInput {
                description: req_str("description")?,
                prompt: req_str("prompt")?,
                subagent_type: opt_str_field("subagent_type"),
                category: opt_str_field("category"),
                run_in_background: bool_field("run_in_background"),
                model: opt_str_field("model"),
                name: opt_str_field("name"),
                team_name: opt_str_field("team_name"),
                mode: opt_str_field("mode"),
                isolation: opt_str_field("isolation"),
                parent_task_id: opt_str_field("parent_task_id"),
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
            ToolKind::ToolSearch => Self::ToolSearch {
                query: req_str("query")?,
                limit: opt_u64_field("limit"),
            },
            ToolKind::ToolSuggest => Self::ToolSuggest {
                intent: req_str("intent")?,
                limit: opt_u64_field("limit"),
            },
            ToolKind::MemoryCreate => Self::MemoryCreate {
                level: req_str("level")?,
                memory_type: req_str("memory_type")?,
                scope: req_str("scope")?,
                body: req_str("body")?,
            },
            ToolKind::MemoryDelete => Self::MemoryDelete {
                path: req_str("path")?,
            },
            ToolKind::TeamCreate => Self::TeamCreate {
                team_name: req_str("team_name")?,
                description: opt_str_field("description"),
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
            ToolKind::TeamMemberMode => Self::TeamMemberMode {
                member_name: req_str("member_name")?,
                mode: req_str("mode")?,
            },
            ToolKind::CodeIndex => Self::CodeIndex {
                path: opt_str_field("path"),
                query: opt_str_field("query"),
                kind: opt_str_field("kind"),
                max_entries: obj
                    .and_then(|map| map.get("max_entries"))
                    .and_then(|value| value.as_u64())
                    .map(|value| value as usize),
            },
            ToolKind::GraphQuery => Self::GraphQuery {
                query: req_str("query")?,
                max_tokens: obj
                    .and_then(|map| map.get("max_tokens"))
                    .and_then(|value| value.as_u64())
                    .map(|value| value as usize),
                include_handles: obj
                    .and_then(|map| map.get("include_handles"))
                    .and_then(|value| value.as_bool()),
            },
            ToolKind::RunCoverage => Self::RunCoverage {
                lcov_path: opt_str_field("lcov_path"),
                include_untested_list: obj
                    .and_then(|map| map.get("include_untested_list"))
                    .and_then(|value| value.as_bool())
                    .unwrap_or(true),
            },
            ToolKind::SymbolEdit => Self::SymbolEdit {
                handle: req_str("handle")?,
                new_content: req_str("new_content")?,
                validate: bool_field("validate"),
                dispatch_cascade: bool_field("dispatch_cascade"),
            },
            ToolKind::PostBounty => Self::PostBounty {
                description: req_str("description")?,
                budget: obj
                    .and_then(|map| map.get("budget"))
                    .and_then(|value| value.as_u64())
                    .unwrap_or(0),
                acceptance_criteria: req_str("acceptance_criteria")?,
                max_solvers: obj
                    .and_then(|map| map.get("max_solvers"))
                    .and_then(|value| value.as_u64())
                    .map(|n| n.min(255) as u8),
                auto_dispatch: bool_field("auto_dispatch"),
            },
            ToolKind::MarketStatus => Self::MarketStatus {
                bounty_id: opt_str_field("bounty_id"),
            },
            ToolKind::RunBounty => Self::RunBounty {
                bounty_id: req_str("bounty_id")?,
                max_solvers: obj
                    .and_then(|map| map.get("max_solvers"))
                    .and_then(|value| value.as_u64())
                    .map(|n| n.min(255) as u8),
            },
            ToolKind::ExitPlanMode => Self::ExitPlanMode {
                plan: req_str("plan")?,
            },
            ToolKind::MultiEdit => Self::MultiEdit {
                file_path: req_str("file_path")?,
                edits: obj
                    .and_then(|map| map.get("edits"))
                    .cloned()
                    .unwrap_or(serde_json::Value::Array(vec![])),
            },
            ToolKind::AskUserQuestion => Self::AskUserQuestion {
                question: req_str("question")?,
                options: obj
                    .and_then(|map| map.get("options"))
                    .cloned()
                    .unwrap_or(serde_json::Value::Array(vec![])),
                multi_select: bool_field("multi_select"),
            },
            ToolKind::WebFetch => Self::WebFetch {
                url: req_str("url")?,
                prompt: opt_str_field("prompt"),
            },
            ToolKind::WebSearch => Self::WebSearch {
                query: req_str("query")?,
                max_results: obj
                    .and_then(|map| map.get("max_results"))
                    .and_then(|value| value.as_u64())
                    .map(|n| n as u32),
            },
            ToolKind::Mcp(name) => Self::Mcp {
                name,
                arguments: value.clone(),
            },
            ToolKind::CronCreate => Self::CronCreate {
                schedule: req_str("schedule")?,
                command: req_str("command")?,
                description: req_str("description")?,
            },
            ToolKind::CronList => Self::CronList,
            ToolKind::CronDelete => Self::CronDelete { id: req_str("id")? },
            ToolKind::ScheduleWakeup => Self::ScheduleWakeup {
                delay_seconds: obj
                    .and_then(|map| map.get("delay_seconds"))
                    .and_then(|value| value.as_u64())
                    .map(|n| n.min(u32::MAX as u64) as u32)
                    .unwrap_or(0),
                prompt: req_str("prompt")?,
                reason: req_str("reason")?,
            },
            ToolKind::Monitor => Self::Monitor {
                command: req_str("command")?,
                until: req_str("until")?,
            },
            ToolKind::Lsp => Self::Lsp {
                kind: req_str("kind")?,
                file: req_str("file")?,
                line: obj
                    .and_then(|map| map.get("line"))
                    .and_then(|value| value.as_u64())
                    .unwrap_or(0) as u32,
                column: obj
                    .and_then(|map| map.get("column"))
                    .and_then(|value| value.as_u64())
                    .unwrap_or(0) as u32,
            },
            ToolKind::PushNotification => Self::PushNotification {
                message: req_str("message")?,
                title: opt_str_field("title"),
            },
            ToolKind::RemoteTrigger => Self::RemoteTrigger {
                trigger_id: req_str("trigger_id")?,
                payload: obj.and_then(|map| map.get("payload")).cloned(),
            },
            ToolKind::EnterPlanMode => Self::EnterPlanMode {
                reason: req_str("reason")?,
            },
            ToolKind::EnterWorktree => Self::EnterWorktree {
                name: req_str("name")?,
                branch: opt_str_field("branch"),
            },
            ToolKind::ExitWorktree => Self::ExitWorktree,
            ToolKind::NotebookRead => Self::NotebookRead {
                path: req_str("path")?,
            },
            ToolKind::NotebookEdit => Self::NotebookEdit {
                path: req_str("path")?,
                cell_id: req_str("cell_id")?,
                new_source: req_str("new_source")?,
                edit_mode: opt_str_field("edit_mode"),
            },
            ToolKind::ScratchpadRead => Self::ScratchpadRead {
                key: req_str("key")?,
            },
            ToolKind::ScratchpadWrite => Self::ScratchpadWrite {
                key: req_str("key")?,
                value: req_str("value")?,
            },
            ToolKind::ServerWebSearch => Self::Generic {
                summary: obj
                    .and_then(|map| map.get("query"))
                    .and_then(|query| query.as_str())
                    .map(|query| format!("🔍 {query}"))
                    .unwrap_or_else(|| value.to_string()),
            },
            ToolKind::ServerCodeExecution => Self::Generic {
                summary: obj
                    .and_then(|map| map.get("code"))
                    .and_then(|code| code.as_str())
                    .map(|code| {
                        let preview: String = code.chars().take(120).collect();
                        format!("⚡ {preview}")
                    })
                    .unwrap_or_else(|| value.to_string()),
            },
            ToolKind::Generic(_) | ToolKind::UnknownTool { .. } => Self::Generic {
                summary: value.to_string(),
            },
        };
        Ok(parsed)
    }

    pub fn to_value(&self) -> serde_json::Value {
        use serde_json::json;
        match self {
            Self::Edit {
                file_path,
                old_string,
                new_string,
                replacement,
            } => {
                let mut value = json!({
                    "file_path": file_path,
                    "old_string": old_string,
                    "new_string": new_string,
                });
                if replacement.replace_all() {
                    value["replace_all"] = json!(true);
                }
                value
            }
            Self::Write { file_path, content } => {
                json!({ "file_path": file_path, "content": content })
            }
            Self::Read {
                file_path,
                offset,
                limit,
            } => {
                let mut value = json!({ "file_path": file_path });
                if let Some(offset) = offset {
                    value["offset"] = json!(offset);
                }
                if let Some(limit) = limit {
                    value["limit"] = json!(limit);
                }
                value
            }
            Self::Bash {
                command,
                timeout,
                workdir,
            } => {
                let mut value = json!({ "command": command });
                if let Some(timeout) = timeout {
                    value["timeout"] = json!(timeout);
                }
                if let Some(workdir) = workdir {
                    value["workdir"] = json!(workdir);
                }
                value
            }
            Self::Glob { pattern, path } => {
                let mut value = json!({ "pattern": pattern });
                if let Some(path) = path {
                    value["path"] = json!(path);
                }
                value
            }
            Self::Grep {
                pattern,
                path,
                glob,
                output_mode,
            } => {
                let mut value = json!({ "pattern": pattern });
                if let Some(path) = path {
                    value["path"] = json!(path);
                }
                if let Some(glob) = glob {
                    value["glob"] = json!(glob);
                }
                if let Some(output_mode) = output_mode {
                    value["output_mode"] = json!(output_mode);
                }
                value
            }
            Self::Search { query, path } => {
                let mut value = json!({ "query": query });
                if let Some(path) = path {
                    value["path"] = json!(path);
                }
                value
            }
            Self::ApplyPatch { patch } => json!({ "patch": patch }),
            Self::TaskCreate {
                subject,
                description,
                active_form,
                blocked_by,
                acceptance_criteria,
                verification_command,
                risk,
                parent_id,
                kind,
            } => {
                let mut value = json!({ "subject": subject, "description": description });
                if let Some(active_form) = active_form {
                    value["active_form"] = json!(active_form);
                }
                if !blocked_by.is_empty() {
                    value["blocked_by"] = json!(blocked_by);
                }
                if let Some(acceptance_criteria) = acceptance_criteria {
                    value["acceptance_criteria"] = json!(acceptance_criteria);
                }
                if let Some(verification_command) = verification_command {
                    value["verification_command"] = json!(verification_command);
                }
                if let Some(risk) = risk {
                    value["risk"] = json!(risk);
                }
                if let Some(parent_id) = parent_id {
                    value["parent_id"] = json!(parent_id);
                }
                if let Some(kind) = kind {
                    value["kind"] = json!(kind);
                }
                value
            }
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
            } => {
                let mut value = json!({ "task_id": task_id });
                if let Some(status) = status {
                    value["status"] = json!(status);
                }
                if let Some(subject) = subject {
                    value["subject"] = json!(subject);
                }
                if let Some(description) = description {
                    value["description"] = json!(description);
                }
                if let Some(owner) = owner {
                    value["owner"] = json!(owner);
                }
                if let Some(acceptance_criteria) = acceptance_criteria {
                    value["acceptance_criteria"] = json!(acceptance_criteria);
                }
                if let Some(verification_command) = verification_command {
                    value["verification_command"] = json!(verification_command);
                }
                if let Some(risk) = risk {
                    value["risk"] = json!(risk);
                }
                if let Some(parent_id) = parent_id {
                    value["parent_id"] = json!(parent_id);
                }
                if let Some(kind) = kind {
                    value["kind"] = json!(kind);
                }
                value
            }
            Self::TaskList {
                status_filter,
                owner_filter,
            } => {
                let mut value = json!({});
                if let Some(status_filter) = status_filter {
                    value["status_filter"] = json!(status_filter);
                }
                if let Some(owner_filter) = owner_filter {
                    value["owner_filter"] = json!(owner_filter);
                }
                value
            }
            Self::TaskDone { task_id } => json!({ "task_id": task_id }),
            Self::TaskGet { task_id } => json!({ "task_id": task_id }),
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
                if let Some(parent_task_id) = &task_input.parent_task_id {
                    value["parent_task_id"] = json!(parent_task_id);
                }
                value
            }
            Self::Skill { name, args } => {
                let mut value = json!({ "name": name });
                if let Some(args) = args {
                    value["args"] = json!(args);
                }
                value
            }
            Self::ToolSearch { query, limit } => {
                let mut value = json!({ "query": query });
                if let Some(limit) = limit {
                    value["limit"] = json!(limit);
                }
                value
            }
            Self::ToolSuggest { intent, limit } => {
                let mut value = json!({ "intent": intent });
                if let Some(limit) = limit {
                    value["limit"] = json!(limit);
                }
                value
            }
            Self::MemoryCreate {
                level,
                memory_type,
                scope,
                body,
            } => json!({
                "level": level,
                "memory_type": memory_type,
                "scope": scope,
                "body": body,
            }),
            Self::MemoryDelete { path } => json!({ "path": path }),
            Self::TeamCreate {
                team_name,
                description,
            } => {
                let mut value = json!({ "team_name": team_name });
                if let Some(description) = description {
                    value["description"] = json!(description);
                }
                value
            }
            Self::TeamDelete => json!({}),
            Self::SendMessage {
                to,
                message,
                summary,
            } => {
                let mut value = json!({ "to": to, "message": message });
                if let Some(summary) = summary {
                    value["summary"] = json!(summary);
                }
                value
            }
            Self::TeamMemberMode { member_name, mode } => {
                json!({ "member_name": member_name, "mode": mode })
            }
            Self::CodeIndex {
                path,
                query,
                kind,
                max_entries,
            } => {
                let mut value = json!({});
                if let Some(path) = path {
                    value["path"] = json!(path);
                }
                if let Some(query) = query {
                    value["query"] = json!(query);
                }
                if let Some(kind) = kind {
                    value["kind"] = json!(kind);
                }
                if let Some(max_entries) = max_entries {
                    value["max_entries"] = json!(max_entries);
                }
                value
            }
            Self::GraphQuery {
                query,
                max_tokens,
                include_handles,
            } => {
                let mut value = json!({ "query": query });
                if let Some(max_tokens) = max_tokens {
                    value["max_tokens"] = json!(max_tokens);
                }
                if let Some(include_handles) = include_handles {
                    value["include_handles"] = json!(include_handles);
                }
                value
            }
            Self::RunCoverage {
                lcov_path,
                include_untested_list,
            } => {
                let mut value = json!({});
                if let Some(lcov_path) = lcov_path {
                    value["lcov_path"] = json!(lcov_path);
                }
                if !include_untested_list {
                    value["include_untested_list"] = json!(false);
                }
                value
            }
            Self::SymbolEdit {
                handle,
                new_content,
                validate,
                dispatch_cascade,
            } => {
                let mut value = json!({ "handle": handle, "new_content": new_content });
                if *validate {
                    value["validate"] = json!(true);
                }
                if *dispatch_cascade {
                    value["dispatch_cascade"] = json!(true);
                }
                value
            }
            Self::PostBounty {
                description,
                budget,
                acceptance_criteria,
                max_solvers,
                auto_dispatch,
            } => {
                let mut value = json!({
                    "description": description,
                    "budget": budget,
                    "acceptance_criteria": acceptance_criteria,
                });
                if let Some(max_solvers) = max_solvers {
                    value["max_solvers"] = json!(max_solvers);
                }
                if *auto_dispatch {
                    value["auto_dispatch"] = json!(true);
                }
                value
            }
            Self::MarketStatus { bounty_id } => {
                let mut value = json!({});
                if let Some(bounty_id) = bounty_id {
                    value["bounty_id"] = json!(bounty_id);
                }
                value
            }
            Self::RunBounty {
                bounty_id,
                max_solvers,
            } => {
                let mut value = json!({ "bounty_id": bounty_id });
                if let Some(max_solvers) = max_solvers {
                    value["max_solvers"] = json!(max_solvers);
                }
                value
            }
            Self::ExitPlanMode { plan } => json!({ "plan": plan }),
            Self::MultiEdit { file_path, edits } => json!({
                "file_path": file_path,
                "edits": edits,
            }),
            Self::AskUserQuestion {
                question,
                options,
                multi_select,
            } => json!({
                "question": question,
                "options": options,
                "multi_select": multi_select,
            }),
            Self::WebFetch { url, prompt } => {
                let mut value = json!({ "url": url });
                if let Some(prompt) = prompt {
                    value["prompt"] = json!(prompt);
                }
                value
            }
            Self::WebSearch { query, max_results } => {
                let mut value = json!({ "query": query });
                if let Some(max_results) = max_results {
                    value["max_results"] = json!(max_results);
                }
                value
            }
            Self::Mcp { arguments, .. } => arguments.clone(),
            Self::CronCreate {
                schedule,
                command,
                description,
            } => json!({
                "schedule": schedule,
                "command": command,
                "description": description,
            }),
            Self::CronList => json!({}),
            Self::CronDelete { id } => json!({ "id": id }),
            Self::ScheduleWakeup {
                delay_seconds,
                prompt,
                reason,
            } => json!({
                "delay_seconds": delay_seconds,
                "prompt": prompt,
                "reason": reason,
            }),
            Self::Monitor { command, until } => json!({
                "command": command,
                "until": until,
            }),
            Self::Lsp {
                kind,
                file,
                line,
                column,
            } => json!({ "kind": kind, "file": file, "line": line, "column": column }),
            Self::PushNotification { message, title } => {
                let mut value = json!({ "message": message });
                if let Some(title) = title {
                    value["title"] = json!(title);
                }
                value
            }
            Self::RemoteTrigger {
                trigger_id,
                payload,
            } => {
                let mut value = json!({ "trigger_id": trigger_id });
                if let Some(payload) = payload {
                    value["payload"] = payload.clone();
                }
                value
            }
            Self::EnterPlanMode { reason } => json!({ "reason": reason }),
            Self::EnterWorktree { name, branch } => {
                let mut value = json!({ "name": name });
                if let Some(branch) = branch {
                    value["branch"] = json!(branch);
                }
                value
            }
            Self::ExitWorktree => json!({}),
            Self::NotebookRead { path } => json!({ "path": path }),
            Self::NotebookEdit {
                path,
                cell_id,
                new_source,
                edit_mode,
            } => {
                let mut value = json!({
                    "path": path,
                    "cell_id": cell_id,
                    "new_source": new_source,
                });
                if let Some(edit_mode) = edit_mode {
                    value["edit_mode"] = json!(edit_mode);
                }
                value
            }
            Self::ScratchpadRead { key } => json!({ "key": key }),
            Self::ScratchpadWrite { key, value } => json!({ "key": key, "value": value }),
            Self::Generic { summary } => match serde_json::from_str::<serde_json::Value>(summary) {
                Ok(serde_json::Value::Object(map)) => serde_json::Value::Object(map),
                Ok(_) | Err(_) => json!({ "input": summary }),
            },
        }
    }
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

#![allow(dead_code)]

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum McpStatus {
    Connected,
    Disabled,
    Error,
}

impl McpStatus {
    pub fn label(self) -> &'static str {
        match self {
            Self::Connected => "Connected",
            Self::Disabled => "Disabled",
            Self::Error => "Error",
        }
    }
}

#[derive(Clone, Debug)]
pub struct McpServerInfo {
    pub name: String,
    pub status: McpStatus,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LspStatus {
    Active,
    Inactive,
}

#[derive(Clone, Debug)]
pub struct LspServerInfo {
    pub name: String,
    pub status: LspStatus,
}

#[derive(Clone, Debug, Default)]
pub struct ModelUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_write_tokens: u64,
    pub cost_usd: Option<f64>,
}

impl ModelUsage {
    pub fn add_delta(&mut self, input: u32, output: u32, cache_read: u32, cache_write: u32) {
        self.input_tokens += input as u64;
        self.output_tokens += output as u64;
        self.cache_read_tokens += cache_read as u64;
        self.cache_write_tokens += cache_write as u64;
    }

    pub fn cache_hit_pct(&self) -> f64 {
        if self.input_tokens == 0 {
            return 0.0;
        }
        (self.cache_read_tokens as f64 / self.input_tokens as f64 * 100.0).min(100.0)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Role {
    User,
    Assistant,
}

#[derive(Clone, Debug)]
pub enum MessagePart {
    Text(String),
    Reasoning(String),
    Tool(ToolCall),
    TaskStatus(TaskStatusPart),
    CompactBoundary { pre_tokens: usize },
}

impl MessagePart {
    pub fn approx_text_len(&self) -> usize {
        match self {
            Self::Text(s) | Self::Reasoning(s) => s.len(),
            Self::Tool(tc) => tc.input.summary().len() + tc.output.approx_text_len(),
            Self::TaskStatus(ts) => {
                ts.description.len() + ts.summary.as_deref().map_or(0, |s| s.len())
            }
            Self::CompactBoundary { .. } => 0,
        }
    }

    pub fn text_only(&self) -> String {
        match self {
            Self::Text(s) | Self::Reasoning(s) => s.clone(),
            Self::Tool(tc) => {
                format!("[Tool: {} → {}]", tc.kind.label(), tc.output.text_only())
            }
            Self::TaskStatus(ts) => {
                format!("[Task {}: {}]", ts.task_id, ts.description)
            }
            Self::CompactBoundary { pre_tokens } => {
                format!("[Compact boundary, pre={pre_tokens} tokens]")
            }
        }
    }

    pub fn to_display_string(&self) -> String {
        match self {
            Self::Text(s) => s.clone(),
            Self::Reasoning(s) => format!("[Reasoning: {}]", s),
            Self::Tool(tc) => {
                format!(
                    "[Tool: {} | Input: {} | Output: {}]",
                    tc.kind.label(),
                    tc.input.summary(),
                    tc.output.to_display_string(),
                )
            }
            Self::TaskStatus(ts) => {
                format!(
                    "[Task {} | {} | {:?}]",
                    ts.task_id, ts.description, ts.status
                )
            }
            Self::CompactBoundary { pre_tokens } => {
                format!("[Compact boundary, pre={pre_tokens} tokens]")
            }
        }
    }
}

#[derive(Clone, Debug)]
pub struct ToolCall {
    pub id: String,
    pub kind: ToolKind,
    pub status: ToolStatus,
    pub input: ToolInput,
    pub output: ToolOutput,
    pub is_collapsed: bool,
}

/// The lifecycle state of a spawned sub-agent task.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TaskLifecycle {
    Pending,
    Running,
    Completed,
    Failed,
    Cancelled,
}

impl TaskLifecycle {
    pub fn label(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Running => "running",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }

    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Failed | Self::Cancelled)
    }
}

#[derive(Clone, Debug)]
pub struct TaskStatusPart {
    pub task_id: String,
    pub description: String,
    pub status: TaskLifecycle,
    pub summary: Option<String>,
    pub error: Option<String>,
    pub elapsed_ms: Option<u64>,
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct TaskInput {
    pub description: String,
    pub prompt: String,
    pub subagent_type: Option<String>,
    pub category: Option<String>,
    pub run_in_background: bool,
    pub model: Option<String>,
}

impl TaskInput {
    pub fn summary(&self) -> String {
        format!(
            "{} ({})",
            self.description,
            if self.run_in_background {
                "background"
            } else {
                "foreground"
            }
        )
    }
}

#[derive(Clone, Debug)]
pub struct LargeText {
    pub content: String,
    pub line_count: usize,
    pub byte_count: usize,
}

impl LargeText {
    pub const COLLAPSE_LINES: usize = 500;
    pub const COLLAPSE_BYTES: usize = 30_720;

    pub fn new(content: String) -> Self {
        let line_count = content.lines().count();
        let byte_count = content.len();
        Self {
            content,
            line_count,
            byte_count,
        }
    }

    pub fn should_collapse(text: &str) -> bool {
        text.len() > Self::COLLAPSE_BYTES || text.lines().count() > Self::COLLAPSE_LINES
    }

    pub fn size_label(&self) -> String {
        let kb = self.byte_count as f64 / 1024.0;
        format!("{} lines · {:.1} KB", self.line_count, kb)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum ToolKind {
    Edit,
    Write,
    Read,
    Bash,
    Glob,
    Grep,
    Search,
    ApplyPatch,
    TaskCreate,
    TaskUpdate,
    TaskList,
    TaskDone,
    Task,
    Generic(String),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ToolStatus {
    Pending,
    Running,
    Complete,
    Failed,
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
    },
    TaskUpdate {
        task_id: String,
        status: Option<String>,
        subject: Option<String>,
        description: Option<String>,
        owner: Option<String>,
    },
    TaskList {
        status_filter: Option<String>,
        owner_filter: Option<String>,
    },
    TaskDone {
        task_id: String,
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

#[derive(Clone, Debug)]
pub enum ToolOutput {
    Text(String),
    LargeText(LargeText),
    Diff(DiffView),
    FileContent {
        path: String,
        content: String,
        language: String,
    },
    Command {
        stdout: String,
        stderr: String,
        exit_code: Option<i32>,
    },
    FileList(Vec<String>),
    Empty,
}

impl ToolOutput {
    pub fn approx_text_len(&self) -> usize {
        match self {
            Self::Text(s) => s.len(),
            Self::LargeText(lt) => lt.byte_count,
            Self::Diff(d) => d
                .hunks
                .iter()
                .flat_map(|h| &h.lines)
                .map(|l| l.content.len())
                .sum(),
            Self::FileContent { content, .. } => content.len(),
            Self::Command { stdout, stderr, .. } => stdout.len() + stderr.len(),
            Self::FileList(files) => files.iter().map(|f| f.len()).sum(),
            Self::Empty => 0,
        }
    }

    pub fn text_only(&self) -> String {
        match self {
            Self::Text(s) => s.clone(),
            Self::LargeText(lt) => format!("[large: {}]", lt.size_label()),
            Self::Diff(d) => format!("{} (+{}/-{})", d.file_path, d.additions, d.deletions),
            Self::FileContent { path, .. } => format!("[file: {}]", path),
            Self::Command {
                stdout,
                stderr,
                exit_code,
            } => {
                let code = exit_code
                    .map(|c| c.to_string())
                    .unwrap_or_else(|| "?".into());
                format!(
                    "exit={} stdout={}B stderr={}B",
                    code,
                    stdout.len(),
                    stderr.len()
                )
            }
            Self::FileList(files) => format!("{} files", files.len()),
            Self::Empty => String::new(),
        }
    }

    pub fn to_display_string(&self) -> String {
        match self {
            Self::Text(s) => s.clone(),
            Self::LargeText(lt) => lt.content.clone(),
            Self::Diff(d) => format!("{} (+{}/-{})", d.file_path, d.additions, d.deletions),
            Self::FileContent { path, content, .. } => {
                format!("{} ({} chars)", path, content.len())
            }
            Self::Command {
                stdout, exit_code, ..
            } => {
                let code = exit_code
                    .map(|c| c.to_string())
                    .unwrap_or_else(|| "?".into());
                let preview = if stdout.len() > 100 {
                    format!("{}...", &stdout[..100])
                } else {
                    stdout.clone()
                };
                format!("exit={}: {}", code, preview)
            }
            Self::FileList(files) => format!("{} files", files.len()),
            Self::Empty => "[empty]".into(),
        }
    }

    pub fn to_api_text(&self) -> String {
        match self {
            Self::LargeText(lt) => lt.content.clone(),
            other => other.to_display_string(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct DiffView {
    pub file_path: String,
    pub hunks: Vec<DiffHunk>,
    pub additions: usize,
    pub deletions: usize,
}

#[derive(Clone, Debug)]
pub struct DiffHunk {
    pub old_start: usize,
    pub new_start: usize,
    pub header: String,
    pub lines: Vec<DiffLine>,
}

#[derive(Clone, Debug)]
pub struct DiffLine {
    pub kind: DiffLineKind,
    pub old_line: Option<usize>,
    pub new_line: Option<usize>,
    pub content: String,
}

#[derive(Clone, Copy, Debug)]
pub enum DiffLineKind {
    Context,
    Added,
    Removed,
}

#[derive(Clone, Debug)]
pub struct ChatMessage {
    pub role: Role,
    pub parts: Vec<MessagePart>,
    pub agent_name: Option<String>,
    pub model_name: Option<String>,
    pub cost_tier: Option<String>,
    pub elapsed: Option<String>,
}

impl ChatMessage {
    pub fn user(content: String) -> Self {
        Self {
            role: Role::User,
            parts: vec![MessagePart::Text(content)],
            agent_name: None,
            model_name: None,
            cost_tier: None,
            elapsed: None,
        }
    }

    pub fn assistant(content: String) -> Self {
        Self {
            role: Role::Assistant,
            parts: vec![MessagePart::Text(content)],
            agent_name: Some("Sisyphus - Ultraworker".into()),
            model_name: Some("Anthropic - Claude Opus 4.6".into()),
            cost_tier: Some("$$$$".into()),
            elapsed: Some("3.9s".into()),
        }
    }

    pub fn assistant_parts(parts: Vec<MessagePart>) -> Self {
        Self {
            role: Role::Assistant,
            parts,
            agent_name: Some("Sisyphus - Ultraworker".into()),
            model_name: Some("Anthropic - Claude Opus 4.6".into()),
            cost_tier: Some("$$$$".into()),
            elapsed: Some("3.9s".into()),
        }
    }

    pub fn compact_boundary(summary: &str, pre_tokens: usize) -> Self {
        Self {
            role: Role::Assistant,
            parts: vec![
                MessagePart::CompactBoundary { pre_tokens },
                MessagePart::Text(format!("Summary of earlier conversation:\n\n{}", summary)),
            ],
            agent_name: Some("system".into()),
            model_name: None,
            cost_tier: None,
            elapsed: None,
        }
    }

    pub fn role_is_user(&self) -> bool {
        self.role == Role::User
    }

    pub fn is_compact_boundary(&self) -> bool {
        self.parts
            .iter()
            .any(|p| matches!(p, MessagePart::CompactBoundary { .. }))
    }
}

impl ToolKind {
    pub fn from_name(name: &str) -> Self {
        match name {
            "Edit" | "str_replace_based_edit_tool" | "edit" => Self::Edit,
            "Write" | "write_file" | "write" => Self::Write,
            "Read" | "read_file" | "read" => Self::Read,
            "Bash" | "run_bash" | "bash" => Self::Bash,
            "Glob" | "glob" => Self::Glob,
            "Grep" | "grep" => Self::Grep,
            "codebase_search" | "search" => Self::Search,
            "apply_patch" => Self::ApplyPatch,
            "TaskCreate" | "task_create" => Self::TaskCreate,
            "TaskUpdate" | "task_update" => Self::TaskUpdate,
            "TaskList" | "task_list" => Self::TaskList,
            "TaskDone" | "task_done" => Self::TaskDone,
            "Task" | "task" => Self::Task,
            other => Self::Generic(other.to_owned()),
        }
    }

    pub fn label(&self) -> &str {
        match self {
            Self::Edit => "Edit",
            Self::Write => "Write",
            Self::Read => "Read",
            Self::Bash => "Bash",
            Self::Glob => "Glob",
            Self::Grep => "Grep",
            Self::Search => "Search",
            Self::ApplyPatch => "Patch",
            Self::TaskCreate => "TaskCreate",
            Self::TaskUpdate => "TaskUpdate",
            Self::TaskList => "TaskList",
            Self::TaskDone => "TaskDone",
            Self::Task => "Task",
            Self::Generic(name) => name.as_str(),
        }
    }

    pub fn api_name(&self) -> &str {
        match self {
            Self::Edit => "Edit",
            Self::Write => "Write",
            Self::Read => "Read",
            Self::Bash => "Bash",
            Self::Glob => "Glob",
            Self::Grep => "Grep",
            Self::Search => "codebase_search",
            Self::ApplyPatch => "apply_patch",
            Self::TaskCreate => "TaskCreate",
            Self::TaskUpdate => "TaskUpdate",
            Self::TaskList => "TaskList",
            Self::TaskDone => "TaskDone",
            Self::Task => "Task",
            Self::Generic(name) => name.as_str(),
        }
    }
}

impl ToolStatus {
    pub fn label(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Running => "running",
            Self::Complete => "done",
            Self::Failed => "failed",
        }
    }
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
            Self::Task(ti) => ti.summary(),
            Self::Generic { summary } => summary.clone(),
        }
    }

    pub fn from_value(tool_name: &str, v: serde_json::Value) -> Self {
        let obj = match &v {
            serde_json::Value::Object(m) => Some(m),
            _ => None,
        };
        let str_field = |key: &str| -> String {
            obj.and_then(|m| m.get(key))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_owned()
        };
        let opt_str_field = |key: &str| -> Option<String> {
            obj.and_then(|m| m.get(key))
                .and_then(|v| v.as_str())
                .map(str::to_owned)
        };
        let opt_u64_field =
            |key: &str| -> Option<u64> { obj.and_then(|m| m.get(key)).and_then(|v| v.as_u64()) };
        let bool_field = |key: &str| -> bool {
            obj.and_then(|m| m.get(key))
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
        };
        match ToolKind::from_name(tool_name) {
            ToolKind::Edit => Self::Edit {
                file_path: str_field("file_path"),
                old_string: str_field("old_string"),
                new_string: str_field("new_string"),
                replacement: ReplacementMode::from_replace_all(bool_field("replace_all")),
            },
            ToolKind::Write => Self::Write {
                file_path: str_field("file_path"),
                content: str_field("content"),
            },
            ToolKind::Read => Self::Read {
                file_path: str_field("file_path"),
                offset: opt_u64_field("offset"),
                limit: opt_u64_field("limit"),
            },
            ToolKind::Bash => Self::Bash {
                command: str_field("command"),
                timeout: opt_u64_field("timeout"),
                workdir: opt_str_field("workdir"),
            },
            ToolKind::Glob => Self::Glob {
                pattern: str_field("pattern"),
                path: opt_str_field("path"),
            },
            ToolKind::Grep => Self::Grep {
                pattern: str_field("pattern"),
                path: opt_str_field("path"),
                glob: opt_str_field("glob"),
                output_mode: opt_str_field("output_mode"),
            },
            ToolKind::Search => Self::Search {
                query: str_field("query"),
                path: opt_str_field("path"),
            },
            ToolKind::ApplyPatch => Self::ApplyPatch {
                patch: str_field("patch"),
            },
            ToolKind::TaskCreate => {
                let blocked_by = obj
                    .and_then(|m| m.get("blocked_by"))
                    .and_then(|v| v.as_array())
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|v| v.as_str().map(str::to_owned))
                            .collect()
                    })
                    .unwrap_or_default();
                Self::TaskCreate {
                    subject: str_field("subject"),
                    description: str_field("description"),
                    active_form: opt_str_field("active_form"),
                    blocked_by,
                }
            }
            ToolKind::TaskUpdate => Self::TaskUpdate {
                task_id: str_field("task_id"),
                status: opt_str_field("status"),
                subject: opt_str_field("subject"),
                description: opt_str_field("description"),
                owner: opt_str_field("owner"),
            },
            ToolKind::TaskList => Self::TaskList {
                status_filter: opt_str_field("status_filter"),
                owner_filter: opt_str_field("owner_filter"),
            },
            ToolKind::TaskDone => Self::TaskDone {
                task_id: str_field("task_id"),
            },
            ToolKind::Task => Self::Task(TaskInput {
                description: str_field("description"),
                prompt: str_field("prompt"),
                subagent_type: opt_str_field("subagent_type"),
                category: opt_str_field("category"),
                run_in_background: bool_field("run_in_background"),
                model: opt_str_field("model"),
            }),
            ToolKind::Generic(_) => Self::Generic {
                summary: v.to_string(),
            },
        }
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
                let mut v = json!({ "file_path": file_path, "old_string": old_string, "new_string": new_string });
                if replacement.replace_all() {
                    v["replace_all"] = json!(true);
                }
                v
            }
            Self::Write { file_path, content } => {
                json!({ "file_path": file_path, "content": content })
            }
            Self::Read {
                file_path,
                offset,
                limit,
            } => {
                let mut v = json!({ "file_path": file_path });
                if let Some(o) = offset {
                    v["offset"] = json!(o);
                }
                if let Some(l) = limit {
                    v["limit"] = json!(l);
                }
                v
            }
            Self::Bash {
                command,
                timeout,
                workdir,
            } => {
                let mut v = json!({ "command": command });
                if let Some(t) = timeout {
                    v["timeout"] = json!(t);
                }
                if let Some(w) = workdir {
                    v["workdir"] = json!(w);
                }
                v
            }
            Self::Glob { pattern, path } => {
                let mut v = json!({ "pattern": pattern });
                if let Some(p) = path {
                    v["path"] = json!(p);
                }
                v
            }
            Self::Grep {
                pattern,
                path,
                glob,
                output_mode,
            } => {
                let mut v = json!({ "pattern": pattern });
                if let Some(p) = path {
                    v["path"] = json!(p);
                }
                if let Some(g) = glob {
                    v["glob"] = json!(g);
                }
                if let Some(m) = output_mode {
                    v["output_mode"] = json!(m);
                }
                v
            }
            Self::Search { query, path } => {
                let mut v = json!({ "query": query });
                if let Some(p) = path {
                    v["path"] = json!(p);
                }
                v
            }
            Self::ApplyPatch { patch } => json!({ "patch": patch }),
            Self::TaskCreate {
                subject,
                description,
                active_form,
                blocked_by,
            } => {
                let mut v = json!({ "subject": subject, "description": description });
                if let Some(af) = active_form {
                    v["active_form"] = json!(af);
                }
                if !blocked_by.is_empty() {
                    v["blocked_by"] = json!(blocked_by);
                }
                v
            }
            Self::TaskUpdate {
                task_id,
                status,
                subject,
                description,
                owner,
            } => {
                let mut v = json!({ "task_id": task_id });
                if let Some(s) = status {
                    v["status"] = json!(s);
                }
                if let Some(s) = subject {
                    v["subject"] = json!(s);
                }
                if let Some(d) = description {
                    v["description"] = json!(d);
                }
                if let Some(o) = owner {
                    v["owner"] = json!(o);
                }
                v
            }
            Self::TaskList {
                status_filter,
                owner_filter,
            } => {
                let mut v = json!({});
                if let Some(f) = status_filter {
                    v["status_filter"] = json!(f);
                }
                if let Some(f) = owner_filter {
                    v["owner_filter"] = json!(f);
                }
                v
            }
            Self::TaskDone { task_id } => json!({ "task_id": task_id }),
            Self::Task(ti) => {
                let mut v = json!({
                    "description": ti.description,
                    "prompt": ti.prompt,
                    "run_in_background": ti.run_in_background,
                });
                if let Some(s) = &ti.subagent_type {
                    v["subagent_type"] = json!(s);
                }
                if let Some(c) = &ti.category {
                    v["category"] = json!(c);
                }
                if let Some(m) = &ti.model {
                    v["model"] = json!(m);
                }
                v
            }
            Self::Generic { summary } => {
                serde_json::from_str(summary).unwrap_or(json!({ "input": summary }))
            }
        }
    }
}

pub fn sample_tool_harness_message() -> ChatMessage {
    let diff = parse_unified_diff(
        "crates/jfc-ui/src/tools.rs",
        r#"@@ -180,2 +180,2 @@
-async fn execute_bash(command: &str, timeout_ms: Option<u64>, cwd: &Path) -> ExecutionResult {
-    let timeout = timeout_ms.unwrap_or(120_000);
+async fn execute_bash(command: &str, timeout_ms: Option<u64>, cwd: &Path) -> ExecutionResult {
+    let timeout = timeout_ms.unwrap_or(300_000);
"#,
    );

    ChatMessage::assistant_parts(vec![
        MessagePart::Reasoning("Increase default bash timeout from 2min to 5min.".into()),
        MessagePart::Tool(ToolCall {
            id: "edit-1".into(),
            kind: ToolKind::Edit,
            status: ToolStatus::Complete,
            input: ToolInput::Edit {
                file_path: "crates/jfc-ui/src/tools.rs".into(),
                old_string: "let timeout = timeout_ms.unwrap_or(120_000);".into(),
                new_string: "let timeout = timeout_ms.unwrap_or(300_000);".into(),
                replacement: ReplacementMode::FirstOnly,
            },
            output: ToolOutput::Diff(diff),
            is_collapsed: false,
        }),
        MessagePart::Tool(ToolCall {
            id: "bash-1".into(),
            kind: ToolKind::Bash,
            status: ToolStatus::Complete,
            input: ToolInput::Bash {
                command: "cargo check -p jfc-ui".into(),
                timeout: None,
                workdir: None,
            },
            output: ToolOutput::Command {
                stdout: "Finished `dev` profile [unoptimized + debuginfo] target(s) in 1.38s"
                    .into(),
                stderr: String::new(),
                exit_code: Some(0),
            },
            is_collapsed: false,
        }),
        MessagePart::Tool(ToolCall {
            id: "read-1".into(),
            kind: ToolKind::Read,
            status: ToolStatus::Complete,
            input: ToolInput::Read {
                file_path: "crates/jfc-ui/src/main.rs".into(),
                offset: Some(1),
                limit: Some(80),
            },
            output: ToolOutput::FileContent {
                path: "crates/jfc-ui/src/main.rs".into(),
                language: "rust".into(),
                content: "mod app;\nmod context;\n\nuse std::sync::Arc;\nuse tokio::sync::mpsc;"
                    .into(),
            },
            is_collapsed: true,
        }),
        MessagePart::Tool(ToolCall {
            id: "write-1".into(),
            kind: ToolKind::Write,
            status: ToolStatus::Pending,
            input: ToolInput::Write {
                file_path: "crates/jfc-ui/src/tool_harness.rs".into(),
                content: "pub enum MessagePart { Text(String), Tool(ToolCall) }".into(),
            },
            output: ToolOutput::Text("Waiting for approval".into()),
            is_collapsed: true,
        }),
        MessagePart::Tool(ToolCall {
            id: "search-1".into(),
            kind: ToolKind::Search,
            status: ToolStatus::Running,
            input: ToolInput::Search {
                query: "ToolRegistry|DiffChanges|tool_result".into(),
                path: Some("research/opencode".into()),
            },
            output: ToolOutput::FileList(vec![
                "packages/ui/src/components/message-part.tsx".into(),
                "packages/ui/src/components/diff-changes.tsx".into(),
                "packages/opencode/src/tool/edit.ts".into(),
            ]),
            is_collapsed: true,
        }),
        MessagePart::Tool(ToolCall {
            id: "patch-1".into(),
            kind: ToolKind::ApplyPatch,
            status: ToolStatus::Complete,
            input: ToolInput::ApplyPatch {
                patch: "*** Begin Patch\n*** Update File: crates/jfc-ui/src/main.rs".into(),
            },
            output: ToolOutput::Diff(parse_unified_diff(
                "crates/jfc-ui/src/main.rs",
                r#"@@ -10,1 +10,1 @@
-struct ChatMessage;
+enum MessagePart;
"#,
            )),
            is_collapsed: true,
        }),
        MessagePart::Tool(ToolCall {
            id: "generic-1".into(),
            kind: ToolKind::Generic("Delegate".into()),
            status: ToolStatus::Failed,
            input: ToolInput::Generic {
                summary: "OpenClaude remote lookup".into(),
            },
            output: ToolOutput::Empty,
            is_collapsed: true,
        }),
    ])
}

pub fn parse_unified_diff(file_path: &str, patch: &str) -> DiffView {
    let mut hunks = Vec::new();
    let mut current: Option<DiffHunk> = None;
    let mut old_line = 0usize;
    let mut new_line = 0usize;
    let mut additions = 0usize;
    let mut deletions = 0usize;

    for raw_line in patch.lines() {
        if raw_line.starts_with("@@") {
            if let Some(hunk) = current.take() {
                hunks.push(hunk);
            }

            let (old_start, new_start, header) = parse_hunk_header(raw_line);
            old_line = old_start;
            new_line = new_start;
            current = Some(DiffHunk {
                old_start,
                new_start,
                header,
                lines: Vec::new(),
            });
            continue;
        }

        let Some(hunk) = current.as_mut() else {
            continue;
        };

        let (kind, content) = match raw_line.chars().next() {
            Some('+') => (DiffLineKind::Added, &raw_line[1..]),
            Some('-') => (DiffLineKind::Removed, &raw_line[1..]),
            Some(' ') => (DiffLineKind::Context, &raw_line[1..]),
            _ => (DiffLineKind::Context, raw_line),
        };

        match kind {
            DiffLineKind::Added => {
                additions += 1;
                hunk.lines.push(DiffLine {
                    kind,
                    old_line: None,
                    new_line: Some(new_line),
                    content: content.into(),
                });
                new_line += 1;
            }
            DiffLineKind::Removed => {
                deletions += 1;
                hunk.lines.push(DiffLine {
                    kind,
                    old_line: Some(old_line),
                    new_line: None,
                    content: content.into(),
                });
                old_line += 1;
            }
            DiffLineKind::Context => {
                hunk.lines.push(DiffLine {
                    kind,
                    old_line: Some(old_line),
                    new_line: Some(new_line),
                    content: content.into(),
                });
                old_line += 1;
                new_line += 1;
            }
        }
    }

    if let Some(hunk) = current {
        hunks.push(hunk);
    }

    DiffView {
        file_path: file_path.into(),
        hunks,
        additions,
        deletions,
    }
}

pub fn parse_hunk_header(header: &str) -> (usize, usize, String) {
    let mut parts = header.split_whitespace();
    let _at = parts.next();
    let old = parts.next().unwrap_or("-1");
    let new = parts.next().unwrap_or("+1");
    let tail = parts.collect::<Vec<_>>().join(" ");
    (parse_hunk_start(old), parse_hunk_start(new), tail)
}

pub fn parse_hunk_start(token: &str) -> usize {
    token
        .trim_start_matches(['-', '+'])
        .split(',')
        .next()
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(1)
}

pub fn truncate_lines(text: &str, max_lines: usize) -> String {
    let lines: Vec<_> = text.lines().collect();
    let mut result = lines
        .iter()
        .take(max_lines)
        .copied()
        .collect::<Vec<_>>()
        .join("\n");
    if lines.len() > max_lines {
        if !result.is_empty() {
            result.push('\n');
        }
        result.push_str(&format!("… {} more lines", lines.len() - max_lines));
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edit_input_json_snapshot_omits_default_replacement_mode() {
        let input = ToolInput::Edit {
            file_path: "src/main.rs".into(),
            old_string: "old".into(),
            new_string: "new".into(),
            replacement: ReplacementMode::FirstOnly,
        };

        assert_eq!(
            input.to_value().to_string(),
            r#"{"file_path":"src/main.rs","old_string":"old","new_string":"new"}"#
        );
    }

    #[test]
    fn edit_input_json_snapshot_preserves_replace_all_wire_shape() {
        let input = ToolInput::Edit {
            file_path: "src/main.rs".into(),
            old_string: "old".into(),
            new_string: "new".into(),
            replacement: ReplacementMode::All,
        };

        assert_eq!(
            input.to_value().to_string(),
            r#"{"file_path":"src/main.rs","old_string":"old","new_string":"new","replace_all":true}"#
        );
    }

    #[test]
    fn large_text_collapses_above_threshold() {
        let short = "line\n".repeat(10);
        assert!(!LargeText::should_collapse(&short));

        let tall = "line\n".repeat(LargeText::COLLAPSE_LINES + 1);
        assert!(LargeText::should_collapse(&tall));

        let fat = "x".repeat(LargeText::COLLAPSE_BYTES + 1);
        assert!(LargeText::should_collapse(&fat));
    }

    #[test]
    fn large_text_size_label_formats_correctly() {
        let lt = LargeText::new("hello\nworld\n".into());
        assert_eq!(lt.line_count, 2);
        assert!(lt.size_label().contains("lines"));
        assert!(lt.size_label().contains("KB"));
    }

    #[test]
    fn task_lifecycle_is_terminal() {
        assert!(TaskLifecycle::Completed.is_terminal());
        assert!(TaskLifecycle::Failed.is_terminal());
        assert!(TaskLifecycle::Cancelled.is_terminal());
        assert!(!TaskLifecycle::Running.is_terminal());
        assert!(!TaskLifecycle::Pending.is_terminal());
    }

    #[test]
    fn task_input_summary_background_flag() {
        let fg = TaskInput {
            description: "do thing".into(),
            prompt: "please do it".into(),
            subagent_type: None,
            category: None,
            run_in_background: false,
            model: None,
        };
        assert!(fg.summary().contains("foreground"));

        let bg = TaskInput {
            run_in_background: true,
            ..fg
        };
        assert!(bg.summary().contains("background"));
    }

    #[test]
    fn task_input_to_value_roundtrip() {
        let input = ToolInput::Task(TaskInput {
            description: "research".into(),
            prompt: "find patterns".into(),
            subagent_type: Some("explore".into()),
            category: None,
            run_in_background: true,
            model: None,
        });
        let v = input.to_value();
        assert_eq!(v["description"], "research");
        assert_eq!(v["subagent_type"], "explore");
        assert_eq!(v["run_in_background"], true);
        assert!(v.get("category").is_none() || v["category"].is_null());
    }

    #[test]
    fn tool_kind_task_parses_from_string() {
        assert_eq!(ToolKind::from_name("Task"), ToolKind::Task);
        assert_eq!(ToolKind::from_name("task"), ToolKind::Task);
    }

    #[test]
    fn tool_output_large_text_api_text_returns_full_content() {
        let lt = LargeText::new("abc\ndef\n".into());
        let out = ToolOutput::LargeText(lt);
        assert_eq!(out.to_api_text(), "abc\ndef\n");
    }
}

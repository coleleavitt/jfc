#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;

use tokio::process::Command;
use tokio::sync::Mutex;

#[cfg(unix)]
unsafe extern "C" {
    fn setsid() -> i32;
}

use tracing::{debug, info, trace, warn};

use crate::context::ReadDedupCache;
use crate::provider::ToolDef;
use crate::tasks::{DeletedFilter, TaskPatch, TaskStatus, TaskStore};
use crate::types::{ReplacementMode, ToolInput, ToolKind};

/// REQ-TOOLS-001: Tool definitions sent to Anthropic API.
/// Field names and schemas match claude-code source exactly to avoid 400 errors.
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
    ]
}

#[derive(Debug, Clone)]
pub struct ExecutionResult {
    pub output: String,
    pub outcome: ToolOutcome,
    pub diagnostics: Vec<ToolDiagnostic>,
    pub provenance: Option<ToolProvenance>,
    /// When set, the renderer prefers this structured diff over
    /// `output`/`Text`. Used by Edit (and Write-as-overwrite) to surface
    /// a colorized diff in the transcript instead of a flat
    /// "file updated successfully" string.
    pub diff: Option<crate::types::DiffView>,
}

impl ExecutionResult {
    pub fn success(output: impl Into<String>) -> Self {
        Self {
            output: output.into(),
            outcome: ToolOutcome::Success,
            diagnostics: Vec::new(),
            provenance: None,
            diff: None,
        }
    }

    pub fn failure(output: impl Into<String>) -> Self {
        let output = output.into();
        Self {
            diagnostics: vec![ToolDiagnostic::error(output.clone())],
            output,
            outcome: ToolOutcome::Failed,
            provenance: None,
            diff: None,
        }
    }

    pub fn with_provenance(mut self, provenance: ToolProvenance) -> Self {
        self.provenance = Some(provenance);
        self
    }

    pub fn with_diff(mut self, diff: crate::types::DiffView) -> Self {
        self.diff = Some(diff);
        self
    }

    pub fn is_error(&self) -> bool {
        matches!(self.outcome, ToolOutcome::Failed)
    }
}

fn configure_tool_command(command: &mut Command) {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("SUDO_ASKPASS", "/bin/false")
        .env("SSH_ASKPASS", "/bin/false");

    #[cfg(unix)]
    unsafe {
        command.pre_exec(|| {
            if setsid() == -1 {
                Err(std::io::Error::last_os_error())
            } else {
                Ok(())
            }
        });
    }
}

fn terminal_safe_text(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();

    while let Some(ch) = chars.next() {
        match ch {
            '\u{1b}' => match chars.peek().copied() {
                Some('[') => {
                    chars.next();
                    for c in chars.by_ref() {
                        if ('@'..='~').contains(&c) {
                            break;
                        }
                    }
                }
                Some(']') => {
                    chars.next();
                    let mut previous_was_esc = false;
                    for c in chars.by_ref() {
                        if c == '\u{7}' || (previous_was_esc && c == '\\') {
                            break;
                        }
                        previous_was_esc = c == '\u{1b}';
                    }
                }
                Some(_) => {
                    chars.next();
                }
                None => {}
            },
            '\t' | '\n' | '\r' => out.push(ch),
            c if c.is_control() => {}
            c => out.push(c),
        }
    }

    out
}

fn non_interactive_shell_command(command: &str) -> String {
    let trimmed = command.trim_start();
    let leading_len = command.len() - trimmed.len();

    if trimmed == "sudo" {
        return format!("{}sudo -n", &command[..leading_len]);
    }

    let Some(rest) = trimmed.strip_prefix("sudo ") else {
        return command.to_string();
    };

    if rest.starts_with("-n ") || rest == "-n" || rest.starts_with("--non-interactive ") {
        command.to_string()
    } else {
        format!("{}sudo -n {}", &command[..leading_len], rest)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolOutcome {
    Success,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolDiagnostic {
    pub level: DiagnosticLevel,
    pub message: String,
    pub help: Option<String>,
}

impl ToolDiagnostic {
    fn error(message: impl Into<String>) -> Self {
        Self {
            level: DiagnosticLevel::Error,
            message: message.into(),
            help: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub enum DiagnosticLevel {
    Error,
    Warning,
    Help,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolProvenance {
    pub cwd: PathBuf,
    pub source: ToolSource,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub enum ToolSource {
    ModelRequested,
    LocalExecutor,
}

/// REQ-TOOLS-002: Tool executors — bash/read/write/edit/glob/grep/task via tokio + fs.
#[tracing::instrument(target = "jfc::tools", skip(input, cwd, dedup, task_store), fields(kind = ?kind))]
pub async fn execute_tool(
    kind: ToolKind,
    input: ToolInput,
    cwd: std::path::PathBuf,
    dedup: Option<Arc<Mutex<ReadDedupCache>>>,
    task_store: Option<Arc<TaskStore>>,
    active_team_name: Option<&str>,
) -> ExecutionResult {
    match (kind, input) {
        (
            ToolKind::Bash,
            ToolInput::Bash {
                command, timeout, ..
            },
        ) => execute_bash(&command, timeout, &cwd).await,
        (
            ToolKind::Read,
            ToolInput::Read {
                file_path,
                offset,
                limit,
            },
        ) => execute_read(&file_path, offset, limit, dedup.as_ref()).await,
        (ToolKind::Write, ToolInput::Write { file_path, content }) => {
            let result = execute_write(&file_path, &content).await;
            if !result.is_error() {
                if let Some(cache) = &dedup {
                    cache.lock().await.invalidate(Path::new(&file_path));
                }
            }
            result
        }
        (
            ToolKind::Edit,
            ToolInput::Edit {
                file_path,
                old_string,
                new_string,
                replacement,
            },
        ) => {
            let result = execute_edit(&file_path, &old_string, &new_string, replacement).await;
            if !result.is_error() {
                if let Some(cache) = &dedup {
                    cache.lock().await.invalidate(Path::new(&file_path));
                }
            }
            result
        }
        (ToolKind::Glob, ToolInput::Glob { pattern, path }) => {
            execute_glob(&pattern, path.as_deref(), &cwd).await
        }
        (
            ToolKind::Grep,
            ToolInput::Grep {
                pattern,
                path,
                glob,
                output_mode,
            },
        ) => {
            execute_grep(
                &pattern,
                path.as_deref(),
                glob.as_deref(),
                output_mode.as_deref(),
                &cwd,
            )
            .await
        }
        (
            ToolKind::TaskCreate,
            ToolInput::TaskCreate {
                subject,
                description,
                active_form,
                blocked_by,
            },
        ) => execute_task_create(task_store, subject, description, active_form, blocked_by),
        (
            ToolKind::TaskUpdate,
            ToolInput::TaskUpdate {
                task_id,
                status,
                subject,
                description,
                owner,
            },
        ) => execute_task_update(task_store, &task_id, status, subject, description, owner),
        (
            ToolKind::TaskList,
            ToolInput::TaskList {
                status_filter,
                owner_filter,
            },
        ) => execute_task_list(
            task_store,
            status_filter.as_deref(),
            owner_filter.as_deref(),
        ),
        (ToolKind::TaskDone, ToolInput::TaskDone { task_id }) => {
            execute_task_done(task_store, &task_id)
        }
        (ToolKind::Task, ToolInput::Task(_)) => {
            ExecutionResult::failure("Task tool must be dispatched via the streaming executor")
        }
        (ToolKind::Skill, ToolInput::Skill { name, args }) => {
            execute_skill(&name, args.as_deref()).await
        }
        (
            ToolKind::MemoryCreate,
            ToolInput::MemoryCreate {
                level,
                memory_type,
                scope,
                body,
            },
        ) => execute_memory_create(&level, &memory_type, &scope, &body, &cwd),
        (ToolKind::MemoryDelete, ToolInput::MemoryDelete { path }) => {
            execute_memory_delete(&path)
        }
        (ToolKind::TeamCreate, ToolInput::TeamCreate { team_name, description }) => {
            execute_team_create(&team_name, description.as_deref(), &cwd).await
        }
        (ToolKind::TeamDelete, ToolInput::TeamDelete) => {
            execute_team_delete(active_team_name).await
        }
        (ToolKind::SendMessage, ToolInput::SendMessage { to, message, summary }) => {
            execute_send_message(&to, &message, summary.as_deref(), active_team_name).await
        }
        (ToolKind::TeamMemberMode, ToolInput::TeamMemberMode { member_name, mode }) => {
            execute_team_member_mode(&member_name, &mode, active_team_name).await
        }
        (kind, _) => ExecutionResult::failure(format!("Tool {:?} not yet implemented", kind)),
    }
}

async fn execute_team_member_mode(
    member_name: &str,
    mode: &str,
    active_team_name: Option<&str>,
) -> ExecutionResult {
    // Validate the mode string against the same vocabulary the leader's
    // `PermissionMode` understands. Reject anything else so a typo
    // doesn't silently leave the teammate in an undefined state.
    const VALID_MODES: &[&str] =
        &["plan", "default", "acceptEdits", "bypassPermissions"];
    if !VALID_MODES.iter().any(|v| v.eq_ignore_ascii_case(mode)) {
        return ExecutionResult::failure(format!(
            "Invalid mode '{mode}'. Must be one of: plan | default | acceptEdits | bypassPermissions"
        ));
    }
    let team_name = match active_team_name {
        Some(t) => t,
        None => {
            return ExecutionResult::failure(
                "No active team. Use TeamCreate first to establish a team.",
            );
        }
    };
    match crate::swarm::team_helpers::set_member_mode(team_name, member_name, mode).await {
        Ok(_) => ExecutionResult::success(format!(
            "{member_name} mode set to {mode}"
        )),
        Err(e) => ExecutionResult::failure(format!(
            "Failed to update {member_name}'s mode: {e}"
        )),
    }
}

async fn execute_bash(command: &str, timeout_ms: Option<u64>, cwd: &Path) -> ExecutionResult {
    let timeout = timeout_ms.unwrap_or(120_000);
    let command = non_interactive_shell_command(command);

    let cmd_preview: String = command.chars().take(100).collect();
    info!(target: "jfc::tools", cmd = %cmd_preview, timeout_ms = timeout, cwd = %cwd.display(), "bash: executing");

    let mut cmd = Command::new("bash");
    cmd.arg("-c")
        .arg(&command)
        .current_dir(cwd)
        .env("CI", "true")
        .env("TERM", "dumb")
        .env("NO_COLOR", "1")
        .env("CLICOLOR", "0")
        .env("DEBIAN_FRONTEND", "noninteractive")
        .env("GCM_INTERACTIVE", "never")
        .env("GIT_EDITOR", ":")
        .env("GIT_MERGE_AUTOEDIT", "no")
        .env("GIT_PAGER", "cat")
        .env("GIT_SEQUENCE_EDITOR", ":")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("HOMEBREW_NO_AUTO_UPDATE", "1")
        .env("PAGER", "cat")
        .env("PIP_NO_INPUT", "1")
        .env("VISUAL", "")
        .env_remove("CLICOLOR_FORCE")
        .env_remove("COLORTERM")
        .env_remove("EDITOR")
        .env_remove("FORCE_COLOR")
        .env_remove("GREP_COLORS")
        .env_remove("LS_COLORS");
    configure_tool_command(&mut cmd);
    let result =
        tokio::time::timeout(std::time::Duration::from_millis(timeout), cmd.output()).await;

    match result {
        Ok(Ok(out)) => {
            let stdout = String::from_utf8_lossy(&out.stdout);
            let stderr = String::from_utf8_lossy(&out.stderr);
            let exit = out.status.code().unwrap_or(-1);
            debug!(target: "jfc::tools", exit_code = exit, stdout_len = stdout.len(), stderr_len = stderr.len(), "bash: completed");
            // Bash semantics per Anthropic's reference tool: a non-zero exit
            // code is part of the *output*, not a tool failure. Many shell
            // utilities use exit 1 as a normal signal (`grep` with no matches,
            // `diff` finding differences, `test` for false). Marking those
            // Failed shows the tool row as red even though the command ran
            // perfectly. Always Complete; the model reads the exit code in
            // the output prefix and interprets.
            let exit = out.status.code().unwrap_or(-1);
            let header = if exit == 0 {
                String::new()
            } else {
                format!("[exit {exit}]\n")
            };
            let body = if stderr.is_empty() {
                stdout.to_string()
            } else if stdout.is_empty() {
                stderr.to_string()
            } else {
                format!("{stdout}\n---stderr---\n{stderr}")
            };
            let body = terminal_safe_text(body.trim_end());
            ExecutionResult::success(format!("{header}{body}")).with_provenance(ToolProvenance {
                cwd: cwd.to_path_buf(),
                source: ToolSource::LocalExecutor,
            })
        }
        Ok(Err(e)) => {
            warn!(target: "jfc::tools", error = %e, "bash: failed to spawn");
            ExecutionResult::failure(format!("Failed to spawn bash: {e}"))
        }
        Err(_) => {
            warn!(target: "jfc::tools", timeout_ms = timeout, "bash: command timed out");
            ExecutionResult::failure(format!("Command timed out after {timeout}ms"))
        }
    }
}

async fn execute_read(
    file_path: &str,
    offset: Option<u64>,
    limit: Option<u64>,
    dedup: Option<&Arc<Mutex<ReadDedupCache>>>,
) -> ExecutionResult {
    debug!(target: "jfc::tools", file_path, offset, limit, "read: starting");
    let path = PathBuf::from(file_path);

    if path.is_dir() {
        match tokio::fs::read_dir(&path).await {
            Ok(mut entries) => {
                let mut names = Vec::new();
                while let Ok(Some(entry)) = entries.next_entry().await {
                    let name = entry.file_name().to_string_lossy().to_string();
                    if entry.path().is_dir() {
                        names.push(format!("{name}/"));
                    } else {
                        names.push(name);
                    }
                }
                names.sort();
                debug!(target: "jfc::tools", entry_count = names.len(), "read: directory listed");
                ExecutionResult::success(names.join("\n"))
            }
            Err(e) => {
                warn!(target: "jfc::tools", file_path, error = %e, "read: cannot read directory");
                ExecutionResult::failure(format!("Cannot read directory: {e}"))
            }
        }
    } else {
        // Dedup only applies to a full re-read (no offset, no limit).
        // Paginated reads (offset/limit set) are how the model walks
        // long files — blocking those leaves it stuck after the first
        // page. The previous behavior treated every Read as "already
        // saw it" because the cache keyed on path alone, so attempts
        // to read line 2000+ of a file got the unchanged stub.
        let is_full_read = offset.is_none() && limit.is_none();
        if is_full_read {
            if let Some(cache) = dedup {
                let guard = cache.lock().await;
                if guard.is_unchanged(&path) {
                    trace!(target: "jfc::tools", file_path, "read: dedup cache hit on full re-read");
                    return ExecutionResult::success(
                        "File unchanged since last full read. The content from \
                         the earlier Read tool_result in this conversation is \
                         still current — refer to that, or pass `offset`/`limit` \
                         to read a specific range."
                            .to_string(),
                    );
                }
                drop(guard);
            }
        }

        match tokio::fs::read_to_string(&path).await {
            Ok(content) => {
                let max_lines = limit.unwrap_or(2000) as usize;
                let start = offset.unwrap_or(1).saturating_sub(1) as usize;
                let lines: Vec<&str> = content.lines().collect();
                let total_lines = lines.len();
                let slice = &lines[start.min(total_lines)..];
                let slice = &slice[..slice.len().min(max_lines)];
                let numbered: String = slice
                    .iter()
                    .enumerate()
                    .map(|(i, line)| format!("{}: {line}", start + i + 1))
                    .collect::<Vec<_>>()
                    .join("\n");

                // Only record a "full read" in the cache so partial
                // reads don't poison subsequent full reads with a
                // false-positive unchanged stub.
                if is_full_read {
                    if let Some(cache) = dedup {
                        cache.lock().await.record_read(path);
                    }
                }

                debug!(
                    target: "jfc::tools",
                    file_path, line_count = slice.len(), total_lines, start,
                    "read: success"
                );
                ExecutionResult::success(numbered)
            }
            Err(e) => {
                warn!(target: "jfc::tools", file_path, error = %e, "read: cannot read file");
                ExecutionResult::failure(format!("Cannot read file: {e}"))
            }
        }
    }
}

async fn execute_write(file_path: &str, content: &str) -> ExecutionResult {
    info!(target: "jfc::tools", file_path, content_len = content.len(), "write: starting");
    let path = PathBuf::from(file_path);
    if let Some(parent) = path.parent() {
        if let Err(e) = tokio::fs::create_dir_all(parent).await {
            warn!(target: "jfc::tools", file_path, error = %e, "write: cannot create directories");
            return ExecutionResult::failure(format!("Cannot create directories: {e}"));
        }
    }
    // Capture the prior contents so we can emit a real diff when this
    // is an *overwrite* (Edit-shaped change) instead of a new file.
    // v126 always renders a diff for Write so the user sees what
    // actually changed; a bare "Written 97 bytes" tells them nothing.
    let prior = tokio::fs::read_to_string(&path).await.ok();
    match tokio::fs::write(&path, content).await {
        Ok(_) => {
            let line_count = content.lines().count();
            let bytes = content.len();
            debug!(target: "jfc::tools", file_path, bytes, line_count, "write: success");
            let header = match &prior {
                Some(_) => format!("Updated {file_path} ({bytes} bytes, {line_count} lines)"),
                None => format!("Wrote {file_path} ({bytes} bytes, {line_count} lines)"),
            };
            // Output clean, unprefixed code — the renderer's syntax
            // highlighter (`render_highlighted_with_line_numbers` →
            // syntect) needs valid source to colorize. Earlier the
            // body had each line prefixed with `+ ` for diff-style
            // visual cues, but that turned every line into invalid
            // syntax (`+ const std = ...` parses as a stray binary-
            // add expression in every language) so highlighting
            // silently fell back to plain text. The diff/sigil
            // semantics belong on `ToolOutput::Diff`, not on a
            // Write's plain text output. The header stays on its own
            // line at the top — it's not part of the highlighted body.
            const PREVIEW_LINES: usize = 30;
            let preview: String = content
                .lines()
                .take(PREVIEW_LINES)
                .collect::<Vec<_>>()
                .join("\n");
            let footer = if line_count > PREVIEW_LINES {
                format!(
                    "\n\n… ({} more lines, full content on disk)",
                    line_count - PREVIEW_LINES
                )
            } else {
                String::new()
            };
            ExecutionResult::success(format!("{header}\n\n{preview}{footer}"))
        }
        Err(e) => {
            warn!(target: "jfc::tools", file_path, error = %e, "write: cannot write file");
            ExecutionResult::failure(format!("Cannot write file: {e}"))
        }
    }
}

async fn execute_edit(
    file_path: &str,
    old_string: &str,
    new_string: &str,
    replacement: ReplacementMode,
) -> ExecutionResult {
    let replace_all = replacement.replace_all();
    info!(target: "jfc::tools", file_path, old_len = old_string.len(), new_len = new_string.len(), replace_all, "edit: starting");
    match tokio::fs::read_to_string(file_path).await {
        Ok(content) => {
            if old_string.is_empty() && !content.is_empty() {
                return ExecutionResult::failure(
                    "old_string is empty but file is not empty. Provide text to replace.",
                );
            }
            let count = content.matches(old_string).count();
            if count == 0 {
                warn!(target: "jfc::tools", file_path, "edit: old_string not found");
                return ExecutionResult::failure(format!("old_string not found in {file_path}"));
            }
            if count > 1 && !replacement.replace_all() {
                warn!(target: "jfc::tools", file_path, count, "edit: multiple matches found");
                return ExecutionResult::failure(format!(
                    "Found {count} matches for old_string in {file_path}. Use replace_all=true or provide more context."
                ));
            }
            let new_content = if replacement.replace_all() {
                content.replace(old_string, new_string)
            } else {
                content.replacen(old_string, new_string, 1)
            };
            match tokio::fs::write(file_path, &new_content).await {
                Ok(_) => {
                    debug!(target: "jfc::tools", file_path, count, "edit: success");
                    // Compute line-level diff stats (matches v126's "Added N lines, Removed M lines")
                    let old_lines = old_string.lines().count();
                    let new_lines = new_string.lines().count();
                    let lines_added = new_lines.saturating_sub(old_lines);
                    let lines_removed = old_lines.saturating_sub(new_lines);
                    let line_summary = match (lines_added, lines_removed) {
                        (0, 0) => format!("{} lines changed", old_lines.max(1)),
                        (a, 0) => format!("+{a} lines"),
                        (0, r) => format!("-{r} lines"),
                        (a, r) => format!("+{a}/-{r} lines"),
                    };
                    // Build a structured DiffView so the renderer
                    // shows a colorized hunk like Write does for new
                    // files. The previous "file updated successfully"
                    // string told the user nothing about WHAT changed
                    // — they had to open the file to verify. Mirrors
                    // v126's Edit-tool diff display.
                    let diff = build_edit_diff_view(
                        file_path,
                        &content,
                        &new_content,
                    );
                    let header = if replacement.replace_all() && count > 1 {
                        format!(
                            "{file_path} updated ({line_summary}, {count} occurrences)"
                        )
                    } else {
                        format!("{file_path} updated ({line_summary})")
                    };
                    ExecutionResult::success(header).with_diff(diff)
                }
                Err(e) => {
                    warn!(target: "jfc::tools", file_path, error = %e, "edit: cannot write after edit");
                    ExecutionResult::failure(format!("Cannot write file after edit: {e}"))
                }
            }
        }
        Err(_) if old_string.is_empty() => match tokio::fs::write(file_path, new_string).await {
            Ok(_) => {
                debug!(target: "jfc::tools", file_path, "edit: created new file");
                ExecutionResult::success(format!("Created new file {file_path}"))
            }
            Err(e2) => {
                warn!(target: "jfc::tools", file_path, error = %e2, "edit: cannot create file");
                ExecutionResult::failure(format!("Cannot create file: {e2}"))
            }
        },
        Err(e) => {
            warn!(target: "jfc::tools", file_path, error = %e, "edit: cannot read file");
            ExecutionResult::failure(format!("Cannot read file: {e}"))
        }
    }
}

/// Build a `DiffView` that walks the line-by-line difference between
/// `old` and `new` and groups changed-region(s) into hunks with a few
/// lines of context. Not as fancy as a real LCS-based diff (no min-edit
/// guarantees) but adequate for Edit-tool display where the change is a
/// localized old_string→new_string replacement. Mirrors what unified
/// diff renders look like, fed straight into the existing
/// `ToolOutput::Diff` renderer.
fn build_edit_diff_view(
    file_path: &str,
    old: &str,
    new: &str,
) -> crate::types::DiffView {
    use crate::types::{DiffHunk, DiffLine, DiffLineKind, DiffView};
    const CONTEXT: usize = 3;
    let old_lines: Vec<&str> = old.lines().collect();
    let new_lines: Vec<&str> = new.lines().collect();

    // Find the first and last lines that differ. If the file is
    // unchanged, this yields an empty hunk list and the renderer just
    // shows the title — matches v126's "no-op edit" rendering.
    let mut first = 0;
    while first < old_lines.len()
        && first < new_lines.len()
        && old_lines[first] == new_lines[first]
    {
        first += 1;
    }
    let mut last_old = old_lines.len();
    let mut last_new = new_lines.len();
    while last_old > first
        && last_new > first
        && old_lines[last_old - 1] == new_lines[last_new - 1]
    {
        last_old -= 1;
        last_new -= 1;
    }

    let mut additions = 0usize;
    let mut deletions = 0usize;
    let mut hunks: Vec<DiffHunk> = Vec::new();
    let has_change = last_old > first || last_new > first;
    if has_change {
        let ctx_start = first.saturating_sub(CONTEXT);
        let ctx_end_old = (last_old + CONTEXT).min(old_lines.len());
        let ctx_end_new = (last_new + CONTEXT).min(new_lines.len());
        let mut lines: Vec<DiffLine> = Vec::new();
        // Leading context (from old; identical in new at this offset).
        let mut old_lineno = ctx_start + 1;
        let mut new_lineno = ctx_start + 1;
        for line in &old_lines[ctx_start..first] {
            lines.push(DiffLine {
                kind: DiffLineKind::Context,
                old_line: Some(old_lineno),
                new_line: Some(new_lineno),
                content: (*line).to_owned(),
            });
            old_lineno += 1;
            new_lineno += 1;
        }
        // Removed lines.
        for line in &old_lines[first..last_old] {
            lines.push(DiffLine {
                kind: DiffLineKind::Removed,
                old_line: Some(old_lineno),
                new_line: None,
                content: (*line).to_owned(),
            });
            old_lineno += 1;
            deletions += 1;
        }
        // Added lines.
        for line in &new_lines[first..last_new] {
            lines.push(DiffLine {
                kind: DiffLineKind::Added,
                old_line: None,
                new_line: Some(new_lineno),
                content: (*line).to_owned(),
            });
            new_lineno += 1;
            additions += 1;
        }
        // Trailing context.
        for (i, line) in old_lines[last_old..ctx_end_old].iter().enumerate() {
            lines.push(DiffLine {
                kind: DiffLineKind::Context,
                old_line: Some(old_lineno + i),
                new_line: Some(new_lineno + i),
                content: (*line).to_owned(),
            });
        }
        let _ = ctx_end_new; // reserved for future LCS-based hunks
        let header = format!(
            "@@ -{old_start},{old_count} +{new_start},{new_count} @@",
            old_start = ctx_start + 1,
            old_count = ctx_end_old - ctx_start,
            new_start = ctx_start + 1,
            new_count = (ctx_end_old - ctx_start)
                + new_lines.len()
                - old_lines.len(),
        );
        hunks.push(DiffHunk {
            old_start: ctx_start + 1,
            new_start: ctx_start + 1,
            header,
            lines,
        });
    }

    DiffView {
        file_path: file_path.to_owned(),
        hunks,
        additions,
        deletions,
    }
}

async fn execute_glob(pattern: &str, path: Option<&str>, cwd: &Path) -> ExecutionResult {
    debug!(target: "jfc::tools", pattern, path, "glob: searching");
    let base = path.map(PathBuf::from).unwrap_or_else(|| cwd.to_path_buf());
    let mut cmd = Command::new("rg");
    cmd.arg("--files")
        .arg("--glob")
        .arg(pattern)
        .current_dir(&base);
    configure_tool_command(&mut cmd);
    match cmd.output().await {
        Ok(out) => {
            let stdout = terminal_safe_text(String::from_utf8_lossy(&out.stdout).trim());
            if stdout.is_empty() {
                debug!(target: "jfc::tools", pattern, "glob: no files matched");
                ExecutionResult::success("No files matched")
            } else {
                let count = stdout.lines().count();
                debug!(target: "jfc::tools", pattern, count, "glob: matches found");
                ExecutionResult::success(stdout)
            }
        }
        Err(_) => {
            let cmd_str = format!(
                "find '{}' -name '{}' 2>/dev/null | sort",
                base.display(),
                pattern
            );
            execute_bash(&cmd_str, Some(10_000), cwd).await
        }
    }
}

async fn execute_grep(
    pattern: &str,
    path: Option<&str>,
    glob: Option<&str>,
    output_mode: Option<&str>,
    cwd: &Path,
) -> ExecutionResult {
    debug!(target: "jfc::tools", pattern, path, output_mode, "grep: searching");
    let search_path = path.unwrap_or(".");
    let mut cmd = Command::new("rg");
    cmd.arg("--no-heading").arg("-n");

    match output_mode.unwrap_or("content") {
        "files_with_matches" => {
            cmd.arg("-l");
        }
        "count" => {
            cmd.arg("-c");
        }
        _ => {}
    }

    if let Some(g) = glob {
        cmd.arg("--glob").arg(g);
    }

    cmd.arg(pattern).arg(search_path).current_dir(cwd);
    configure_tool_command(&mut cmd);

    match cmd.output().await {
        Ok(out) => {
            let stdout = terminal_safe_text(String::from_utf8_lossy(&out.stdout).trim());
            let stderr = terminal_safe_text(String::from_utf8_lossy(&out.stderr).trim());
            if stdout.is_empty() && out.status.code() == Some(1) {
                debug!(target: "jfc::tools", pattern, "grep: no matches found");
                ExecutionResult::success("No matches found")
            } else if !stderr.is_empty() && stdout.is_empty() {
                warn!(target: "jfc::tools", pattern, error = %stderr, "grep: rg error");
                ExecutionResult::failure(stderr)
            } else {
                let result_lines = stdout.lines().count();
                debug!(target: "jfc::tools", pattern, result_lines, "grep: matches found");
                ExecutionResult::success(stdout)
            }
        }
        Err(e) => {
            warn!(target: "jfc::tools", error = %e, "grep: rg not found or failed");
            ExecutionResult::failure(format!("rg not found or failed: {e}"))
        }
    }
}

fn execute_task_create(
    store: Option<Arc<TaskStore>>,
    subject: String,
    description: String,
    active_form: Option<String>,
    blocked_by: Vec<String>,
) -> ExecutionResult {
    debug!(target: "jfc::tools", %subject, blocked_count = blocked_by.len(), "task_create: creating");
    let Some(store) = store else {
        return ExecutionResult::failure("Task store not available");
    };
    match store.create(subject, description, active_form, blocked_by) {
        Ok(task) => {
            debug!(target: "jfc::tools", task_id = %task.id, "task_create: success");
            ExecutionResult::success(
                serde_json::to_string_pretty(&task).unwrap_or_else(|_| format!("{task:?}")),
            )
        }
        Err(e) => {
            warn!(target: "jfc::tools", error = %e, "task_create: failed");
            ExecutionResult::failure(e.to_string())
        }
    }
}

fn execute_task_update(
    store: Option<Arc<TaskStore>>,
    task_id: &str,
    status: Option<String>,
    subject: Option<String>,
    description: Option<String>,
    owner: Option<String>,
) -> ExecutionResult {
    debug!(target: "jfc::tools", task_id, status = status.as_deref(), "task_update: updating");
    let Some(store) = store else {
        return ExecutionResult::failure("Task store not available");
    };
    let parsed_status = status.as_deref().and_then(|s| match s {
        "pending" => Some(TaskStatus::Pending),
        "in_progress" => Some(TaskStatus::InProgress),
        "completed" => Some(TaskStatus::Completed),
        "deleted" => Some(TaskStatus::Deleted),
        _ => None,
    });
    let patch = TaskPatch {
        subject,
        description,
        status: parsed_status,
        owner,
        ..Default::default()
    };
    match store.update(task_id, patch) {
        Ok(task) => {
            debug!(target: "jfc::tools", task_id, "task_update: success");
            ExecutionResult::success(
                serde_json::to_string_pretty(&task).unwrap_or_else(|_| format!("{task:?}")),
            )
        }
        Err(e) => {
            warn!(target: "jfc::tools", task_id, error = %e, "task_update: failed");
            ExecutionResult::failure(e.to_string())
        }
    }
}

fn execute_task_list(
    store: Option<Arc<TaskStore>>,
    status_filter: Option<&str>,
    owner_filter: Option<&str>,
) -> ExecutionResult {
    debug!(target: "jfc::tools", status_filter, owner_filter, "task_list: listing");
    let Some(store) = store else {
        return ExecutionResult::failure("Task store not available");
    };
    let mut tasks = store.list(DeletedFilter::Exclude);
    if let Some(sf) = status_filter {
        tasks.retain(|t| {
            let s = serde_json::to_value(&t.status)
                .ok()
                .and_then(|v| v.as_str().map(str::to_owned));
            s.as_deref() == Some(sf)
        });
    }
    if let Some(of) = owner_filter {
        tasks.retain(|t| t.owner.as_deref() == Some(of));
    }
    debug!(target: "jfc::tools", count = tasks.len(), "task_list: result");
    let output =
        serde_json::to_string_pretty(&tasks).unwrap_or_else(|_| format!("{} tasks", tasks.len()));
    ExecutionResult::success(output)
}

fn execute_task_done(store: Option<Arc<TaskStore>>, task_id: &str) -> ExecutionResult {
    debug!(target: "jfc::tools", task_id, "task_done: marking complete");
    let Some(store) = store else {
        return ExecutionResult::failure("Task store not available");
    };
    let patch = TaskPatch {
        status: Some(TaskStatus::Completed),
        ..Default::default()
    };
    match store.update(task_id, patch) {
        Ok(task) => {
            debug!(target: "jfc::tools", task_id, "task_done: success");
            ExecutionResult::success(
                serde_json::to_string_pretty(&task).unwrap_or_else(|_| format!("{task:?}")),
            )
        }
        Err(e) => {
            warn!(target: "jfc::tools", task_id, error = %e, "task_done: failed");
            ExecutionResult::failure(e.to_string())
        }
    }
}

/// Resolve a registered skill by name and return its markdown body as the
/// tool result. Optional `args` (when non-empty) are appended under an
/// `# Args` header so the model can incorporate the caller's context.
///
/// This is read-only by construction — `load_skills` walks the filesystem
/// but doesn't mutate anything, and the body returned here is just a string
/// the model already has the right to read (it's already in the system
/// prompt listing).
pub async fn execute_skill(name: &str, args: Option<&str>) -> ExecutionResult {
    info!(target: "jfc::tools", skill_name = name, has_args = args.is_some(), "skill: invoking");
    let cwd = std::env::current_dir().unwrap_or_else(|_| ".".into());
    execute_skill_in(&cwd, name, args).await
}

/// Cwd-parameterized form used by tests so skill discovery is hermetic.
async fn execute_skill_in(cwd: &Path, name: &str, args: Option<&str>) -> ExecutionResult {
    let skills = crate::agents::load_skills(cwd);
    // Be permissive with what the model passes in. v126 lets the model
    // call a skill by its name (`do-178b`), but in practice the model
    // sometimes passes the inner-file path it sees in the listing
    // (`do-178b/SKILL`) or the full `.md` filename. Strip the suffix
    // and any "/SKILL" tail before lookup so a small naming wobble
    // doesn't return Unknown.
    let normalized = name
        .trim()
        .trim_end_matches(".md")
        .trim_end_matches("/SKILL")
        .trim_end_matches("/Skill")
        .trim_end_matches("/skill")
        .trim_end_matches('/');
    let candidates: [&str; 2] = [normalized, name];
    let found = candidates
        .iter()
        .find_map(|c| crate::agents::find_skill_by_name(&skills, c));
    match found {
        Some(skill) => {
            let body = match args.filter(|s| !s.is_empty()) {
                Some(a) => format!("{}\n\n# Args\n{}", skill.body, a),
                None => skill.body.clone(),
            };
            ExecutionResult::success(body)
        }
        None => {
            // Surface the available skills in the error so the model
            // can self-correct without having to ask the user. The
            // previous bare "Unknown skill: do-178b" gave it nothing
            // to recover with.
            let available: Vec<&str> = skills.iter().map(|s| s.name.as_str()).collect();
            let suffix = if available.is_empty() {
                String::from(" (no skills installed)")
            } else {
                format!(". Available: {}", available.join(", "))
            };
            ExecutionResult::failure(format!("Unknown skill: {name}{suffix}"))
        }
    }
}

/// Default agentic-loop bound when an agent definition doesn't pin one.
/// Generous enough that legitimate multi-tool tasks complete; tight enough
/// that a runaway subagent can't burn unlimited tokens. Mirrors v126's
/// `MAX_AGENT_TURNS` default in cli.2.1.126 (the subagent runner there
/// caps at ~20 iterations).
const DEFAULT_AGENT_MAX_TURNS: u32 = 20;

/// Apply an agent's `allowedTools` (allowlist) and `disallowedTools`
/// (blocklist) to the parent's full tool catalogue. An empty `allowed`
/// means "all tools allowed" (matches v126 conventions); a non-empty
/// `allowed` is exact membership. `disallowed` always subtracts.
/// The Task tool itself is also dropped — recursive subagent spawning
/// is intentionally not wired (would deadlock the single-stream model).
fn filter_tools_for_agent(
    all: Vec<ToolDef>,
    allowed: &[String],
    disallowed: &[String],
) -> Vec<ToolDef> {
    let allow_all = allowed.is_empty();
    all.into_iter()
        .filter(|t| {
            if t.name.eq_ignore_ascii_case("Task") {
                return false;
            }
            if !allow_all && !allowed.iter().any(|a| a.eq_ignore_ascii_case(&t.name)) {
                return false;
            }
            !disallowed.iter().any(|d| d.eq_ignore_ascii_case(&t.name))
        })
        .collect()
}

/// Run a subagent. The agent gets its own system prompt, tool catalogue
/// (filtered by the agent's allow/disallow lists), an optional cwd
/// override (used for worktree isolation), and a turn cap from
/// `agent_def.max_turns` (defaults to `DEFAULT_AGENT_MAX_TURNS`).
///
/// This is a real agentic loop — when the subagent emits `tool_use`,
/// we execute the tool here and feed the `tool_result` back to the
/// model on the next iteration, exactly like the parent stream loop in
/// `stream::stream_response`. Without the loop the subagent could never
/// `Read` a file or run `Bash`; it could only produce prose.
pub async fn execute_task(
    task_input: &crate::types::TaskInput,
    provider: &dyn crate::provider::Provider,
    model_id: crate::provider::ModelId,
    tx: Option<&tokio::sync::mpsc::UnboundedSender<crate::app::AppEvent>>,
    task_id: Option<&str>,
    agent_def: Option<&crate::agents::AgentDef>,
    cwd_override: Option<PathBuf>,
) -> ExecutionResult {
    use crate::provider::{
        ProviderContent, ProviderMessage, ProviderRole, StopReason, StreamEvent, StreamOptions,
    };
    use futures::StreamExt;

    let model = if let Some(m) = &task_input.model {
        crate::provider::ModelId::new(m.clone())
    } else {
        model_id
    };

    let cwd = cwd_override
        .clone()
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));

    // System prompt: prefer the agent's compiled prompt when we have a
    // definition. Without one, fall back to a minimal default that
    // tells the model it's a subagent with tools — without ANY system
    // prompt some models just ack and emit `end_turn` immediately,
    // which produced the "Task completed in 3 seconds with empty
    // output" symptom when subagent_type lookup missed.
    let system_prompt = match agent_def {
        Some(agent) => {
            let skills = crate::agents::load_skills(&cwd);
            Some(crate::agents::build_agent_system_prompt(agent, &skills))
        }
        None => Some(
            "You are a subagent dispatched to handle a specific task. You have \
             direct access to the user's filesystem and shell via tools (Bash, \
             Read, Write, Edit, Glob, Grep, etc.). Use the tools to complete the \
             task — don't just describe what you would do. When you have enough \
             information, write a thorough text summary of your findings and \
             stop. Working directory: "
                .to_owned()
                + cwd.display().to_string().as_str(),
        ),
    };

    // Tool catalogue: full list filtered by the agent's allow/disallow.
    // When there's no agent definition we still drop `Task` to avoid
    // recursive subagent spawning, but otherwise pass everything.
    let (allowed, disallowed): (&[String], &[String]) = match agent_def {
        Some(a) => (&a.allowed_tools, &a.disallowed_tools),
        None => (&[], &[]),
    };
    let tools = filter_tools_for_agent(all_tool_defs(), allowed, disallowed);

    let max_turns = agent_def
        .and_then(|a| a.max_turns)
        .unwrap_or(DEFAULT_AGENT_MAX_TURNS);

    let mut conversation = vec![ProviderMessage {
        role: ProviderRole::User,
        content: vec![ProviderContent::Text(task_input.prompt.clone())],
    }];
    let mut final_text = String::new();
    let mut last_error: Option<String> = None;
    let mut turn: u32 = 0;

    'outer: loop {
        turn += 1;
        if turn > max_turns {
            warn!(
                target: "jfc::tools",
                task_id = ?task_id,
                turn,
                max_turns,
                "subagent exceeded max_turns — bailing"
            );
            last_error = Some(format!(
                "Subagent exceeded max_turns ({max_turns}). Returning partial output."
            ));
            break;
        }

        let mut options = StreamOptions::new(model.clone()).tools(tools.clone());
        if let Some(sp) = &system_prompt {
            options = options.system(sp.clone());
        }

        let stream = match provider.stream(conversation.clone(), &options).await {
            Ok(s) => s,
            Err(e) => return ExecutionResult::failure(format!("Subagent stream error: {e}")),
        };
        tokio::pin!(stream);

        // Per-iteration accumulators. `tool_uses` collects every
        // tool_use block the model emits this turn so we can execute
        // them in order and feed the results back on the next pass.
        let mut turn_text = String::new();
        let mut tool_uses: Vec<(String, String, String)> = Vec::new(); // (id, name, input_json)
        let mut stop_reason: Option<StopReason> = None;

        while let Some(event) = stream.next().await {
            match event {
                Ok(StreamEvent::TextDelta { delta, .. }) => {
                    // Pipe deltas through to the task panel so the user
                    // sees the subagent's prose stream live.
                    if let (Some(tx), Some(id)) = (tx, task_id) {
                        let _ = tx.send(crate::app::AppEvent::AgentChunk {
                            task_id: id.to_owned(),
                            text: delta.clone(),
                        });
                    }
                    turn_text.push_str(&delta);
                }
                Ok(StreamEvent::TextDone { text: t, .. }) => {
                    if turn_text.is_empty() {
                        turn_text = t;
                    }
                }
                Ok(StreamEvent::ToolDone {
                    tool_name,
                    tool_use_id,
                    input_json,
                    ..
                }) => {
                    tool_uses.push((tool_use_id, tool_name, input_json));
                }
                Ok(StreamEvent::Done { stop_reason: sr }) => {
                    stop_reason = Some(sr);
                }
                Ok(StreamEvent::Error { message }) => {
                    last_error = Some(message);
                    break 'outer;
                }
                Err(e) => {
                    last_error = Some(e.to_string());
                    break 'outer;
                }
                Ok(_) => {}
            }
        }

        // Append the assistant turn (text + tool_uses, if any) so the
        // next iteration's request reflects the running history.
        let mut assistant_content = Vec::new();
        if !turn_text.is_empty() {
            assistant_content.push(ProviderContent::Text(turn_text.clone()));
        }
        for (id, name, input_json) in &tool_uses {
            let parsed_input: serde_json::Value =
                serde_json::from_str(input_json).unwrap_or(serde_json::Value::Null);
            assistant_content.push(ProviderContent::ToolUse {
                id: id.clone(),
                name: name.clone(),
                input: parsed_input,
            });
        }
        if !assistant_content.is_empty() {
            conversation.push(ProviderMessage {
                role: ProviderRole::Assistant,
                content: assistant_content,
            });
        }

        if !turn_text.is_empty() {
            // Replace, not append — the most recent text is the one to
            // surface as the subagent's final reply when the loop ends.
            final_text = turn_text;
        }

        // No tool calls → subagent is done speaking. Don't also gate on
        // `stop_reason == EndTurn`: the OWUI/LiteLLM proxy emits
        // `Done{EndTurn}` on the final `[DONE]` SSE marker even when
        // the chunk that *finished* the turn carried tool_calls — so
        // the stop_reason we end up with is `EndTurn` despite there
        // being unexecuted tool_uses. Trusting it would cause the
        // subagent to return empty in 3–7s without ever calling Read /
        // Glob / Grep, exactly the symptom in the user's screenshot.
        if tool_uses.is_empty() {
            break;
        }
        let _ = stop_reason; // suppress "unused" — kept for future use

        // Execute every tool the subagent requested this turn, then
        // feed the results back as a single user turn (Anthropic API
        // requires all `tool_result`s to be batched in one user msg
        // immediately following the assistant turn that called them).
        let mut tool_results: Vec<ProviderContent> = Vec::new();
        for (id, name, input_json) in tool_uses {
            // Defense in depth: even though the tool list was filtered
            // upstream, re-check here in case the model hallucinated a
            // disallowed name. Provider-side filtering should already
            // make this unreachable for compliant models.
            if !disallowed.is_empty()
                && disallowed.iter().any(|d| d.eq_ignore_ascii_case(&name))
            {
                tool_results.push(ProviderContent::ToolResult {
                    tool_use_id: id.clone(),
                    content: format!("Tool '{name}' is not allowed for this agent."),
                    is_error: true,
                });
                continue;
            }
            let kind = ToolKind::from_name(&name);
            let parsed: serde_json::Value =
                serde_json::from_str(&input_json).unwrap_or(serde_json::Value::Null);
            let input = ToolInput::from_value(&name, parsed);
            let result = execute_tool(kind, input, cwd.clone(), None, None, None).await;
            let is_error = result.is_error();
            tool_results.push(ProviderContent::ToolResult {
                tool_use_id: id.clone(),
                content: result.output,
                is_error,
            });
        }
        conversation.push(ProviderMessage {
            role: ProviderRole::User,
            content: tool_results,
        });
    }

    if let Some(err) = last_error {
        if final_text.is_empty() {
            ExecutionResult::failure(err)
        } else {
            ExecutionResult::success(format!("{final_text}\n\n[note: {err}]"))
        }
    } else {
        ExecutionResult::success(final_text)
    }
}

// ─── Memory tool executors ───────────────────────────────────────────────────

fn execute_memory_create(
    level: &str,
    memory_type: &str,
    scope: &str,
    body: &str,
    project_root: &Path,
) -> ExecutionResult {
    use crate::memory;

    let mem_level = match level.to_lowercase().as_str() {
        "user" => memory::MemoryLevel::User,
        "project" => memory::MemoryLevel::Project,
        other => {
            return ExecutionResult::failure(format!(
                "Invalid level '{other}'. Use 'user' or 'project'."
            ))
        }
    };

    let mem_type = match memory_type.parse::<memory::MemoryType>() {
        Ok(t) => t,
        Err(e) => return ExecutionResult::failure(e),
    };

    let mem_scope = match scope.parse::<memory::MemoryScope>() {
        Ok(s) => s,
        Err(e) => return ExecutionResult::failure(e),
    };

    if body.trim().is_empty() {
        return ExecutionResult::failure("Memory body cannot be empty.");
    }

    match memory::create_memory(mem_level, mem_type, mem_scope, body.trim(), project_root) {
        Ok(path) => ExecutionResult::success(format!(
            "Memory saved to: {}\n\nThis memory will be included in future conversations.",
            path.display()
        )),
        Err(e) => ExecutionResult::failure(format!("Failed to create memory: {e}")),
    }
}

fn execute_memory_delete(path_str: &str) -> ExecutionResult {
    use crate::memory;
    use std::path::PathBuf;

    let path = PathBuf::from(path_str);

    if !path.exists() {
        return ExecutionResult::failure(format!("File not found: {}", path.display()));
    }

    match memory::delete_memory(&path) {
        Ok(()) => ExecutionResult::success(format!(
            "Memory deleted: {}\n\nThis memory will no longer be included in future conversations.",
            path.display()
        )),
        Err(e) => ExecutionResult::failure(format!("Failed to delete memory: {e}")),
    }
}

// ─── Swarm tools ─────────────────────────────────────────────────────────────

async fn execute_team_create(
    team_name: &str,
    description: Option<&str>,
    cwd: &Path,
) -> ExecutionResult {
    use crate::swarm::{self, team_helpers, types::make_agent_id};

    let lead_id = make_agent_id(swarm::TEAM_LEAD_NAME, team_name);

    match team_helpers::create_team(
        team_name,
        description,
        &lead_id,
        None,
        &cwd.to_string_lossy(),
    )
    .await
    {
        Ok(_team_file) => {
            let file_path = team_helpers::team_file_path(team_name);
            let result = serde_json::json!({
                "team_name": team_name,
                "team_file_path": file_path.to_string_lossy(),
                "lead_agent_id": lead_id,
            });
            ExecutionResult::success(serde_json::to_string_pretty(&result).unwrap_or_default())
        }
        Err(e) => ExecutionResult::failure(format!("Failed to create team: {e}")),
    }
}

async fn execute_team_delete(active_team_name: Option<&str>) -> ExecutionResult {
    use crate::swarm::team_helpers;

    let team_name = match active_team_name {
        Some(name) => name,
        None => {
            return ExecutionResult::failure(
                "No active team. Use TeamCreate first to establish a team.",
            );
        }
    };

    match team_helpers::delete_team(team_name).await {
        Ok(()) => {
            let result = serde_json::json!({
                "success": true,
                "message": format!("Cleaned up directories for team \"{team_name}\""),
                "team_name": team_name,
            });
            ExecutionResult::success(serde_json::to_string_pretty(&result).unwrap_or_default())
        }
        Err(e) => ExecutionResult::failure(format!("Failed to delete team: {e}")),
    }
}

async fn execute_send_message(
    to: &str,
    message: &str,
    summary: Option<&str>,
    active_team_name: Option<&str>,
) -> ExecutionResult {
    use crate::swarm::mailbox;
    use crate::swarm::types::MailboxMessage;

    let team_name = active_team_name.unwrap_or(crate::swarm::DEFAULT_TEAM_NAME);

    let msg = MailboxMessage {
        from: crate::swarm::TEAM_LEAD_NAME.to_owned(),
        text: message.to_owned(),
        timestamp: chrono::Utc::now().to_rfc3339(),
        color: None,
        summary: summary.map(str::to_owned),
        read: false,
    };

    match mailbox::write_to_mailbox(to, msg, team_name).await {
        Ok(()) => {
            let result = serde_json::json!({
                "success": true,
                "message": format!("Message sent to {to}'s inbox"),
                "routing": {
                    "sender": crate::swarm::TEAM_LEAD_NAME,
                    "target": format!("@{to}"),
                    "summary": summary,
                }
            });
            ExecutionResult::success(serde_json::to_string_pretty(&result).unwrap_or_default())
        }
        Err(e) => ExecutionResult::failure(format!("Failed to send message: {e}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn execution_result_failure_carries_diagnostic() {
        let result = ExecutionResult::failure("command failed");

        assert!(result.is_error());
        assert_eq!(result.outcome, ToolOutcome::Failed);
        assert_eq!(result.diagnostics.len(), 1);
        assert_eq!(result.diagnostics[0].level, DiagnosticLevel::Error);
        assert_eq!(result.diagnostics[0].message, "command failed");
    }

    #[tokio::test]
    async fn bash_runs_without_inherited_terminal_or_stdin() {
        let result = execute_bash(
            "read -t 0.1 value || true; (cat /dev/tty >/dev/null 2>&1 && echo has-tty || echo no-tty); if [ -n \"${value:-}\" ]; then echo stdin-leaked; fi",
            Some(5_000),
            Path::new("."),
        )
        .await;

        assert!(!result.is_error(), "{}", result.output);
        assert!(result.output.contains("no-tty"), "{}", result.output);
        assert!(!result.output.contains("stdin-leaked"), "{}", result.output);
    }

    #[test]
    fn leading_sudo_is_forced_non_interactive() {
        assert_eq!(non_interactive_shell_command("sudo true"), "sudo -n true");
        assert_eq!(
            non_interactive_shell_command("  sudo --non-interactive true"),
            "  sudo --non-interactive true"
        );
        assert_eq!(
            non_interactive_shell_command("echo sudo true"),
            "echo sudo true"
        );
    }

    #[tokio::test]
    async fn sudo_prompt_does_not_escape_or_hang() {
        let result = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            execute_bash("sudo true", Some(4_000), Path::new(".")),
        )
        .await
        .expect("sudo command should fail or succeed without hanging");

        assert!(!result.output.contains("Password:"), "{}", result.output);
        assert!(!result.output.contains('\u{1b}'), "{}", result.output);
    }

    #[test]
    fn terminal_safe_text_strips_control_sequences() {
        let raw =
            "\u{1b}[31mred\u{1b}[0m \u{1b}[<35;82;42MPassword:\u{7}\u{1b}]0;title\u{7} ok\u{0}";

        assert_eq!(terminal_safe_text(raw), "red Password: ok");
    }

    /// Best-effort temp-dir helper — returns `None` if temp creation
    /// fails so tests skip rather than fail on sandboxes without
    /// writable temp.
    fn skill_tempdir_or_skip() -> Option<PathBuf> {
        let mut p = std::env::temp_dir();
        p.push(format!(
            "jfc_skill_test_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .ok()?
                .as_nanos()
        ));
        std::fs::create_dir_all(p.join(".claude/skills")).ok()?;
        Some(p)
    }

    fn write_skill(root: &Path, name: &str, body: &str) {
        let path = root.join(".claude/skills").join(format!("{name}.md"));
        let frontmatter = format!("---\nname: {name}\n---\n{body}");
        std::fs::write(&path, frontmatter).expect("write skill");
    }

    #[tokio::test]
    async fn execute_skill_unknown_returns_failure_robust() {
        let Some(root) = skill_tempdir_or_skip() else {
            return;
        };
        // Use a very unlikely name so a stray user-level skill at
        // ~/.claude/skills cannot satisfy the lookup.
        let result =
            execute_skill_in(&root, "definitely-not-a-real-skill-xyz-9831", None).await;
        assert!(result.is_error(), "unknown skill must report failure");
        assert!(
            result.output.contains("Unknown skill"),
            "expected 'Unknown skill' marker, got: {}",
            result.output
        );
    }

    #[tokio::test]
    async fn execute_skill_known_returns_body_normal() {
        let Some(root) = skill_tempdir_or_skip() else {
            return;
        };
        write_skill(&root, "jfc-test-known", "Do the thing carefully.");

        let result = execute_skill_in(&root, "jfc-test-known", None).await;
        assert!(!result.is_error(), "known skill must succeed: {:?}", result);
        assert!(
            result.output.contains("Do the thing carefully."),
            "skill body should be returned, got: {}",
            result.output
        );
    }

    #[tokio::test]
    async fn execute_skill_appends_args_normal() {
        let Some(root) = skill_tempdir_or_skip() else {
            return;
        };
        write_skill(&root, "jfc-test-args", "Body content.");

        let result =
            execute_skill_in(&root, "jfc-test-args", Some("focus on auth")).await;
        assert!(!result.is_error(), "skill with args must succeed");
        assert!(result.output.contains("Body content."));
        assert!(
            result.output.contains("# Args"),
            "args block should have header, got: {}",
            result.output
        );
        assert!(
            result.output.contains("focus on auth"),
            "args text should be embedded, got: {}",
            result.output
        );
    }

    #[tokio::test]
    async fn execute_skill_no_args_no_header_normal() {
        let Some(root) = skill_tempdir_or_skip() else {
            return;
        };
        write_skill(&root, "jfc-test-no-args", "Plain body.");

        let result = execute_skill_in(&root, "jfc-test-no-args", None).await;
        assert!(!result.is_error());
        assert!(
            !result.output.contains("# Args"),
            "no args means no Args section, got: {}",
            result.output
        );
    }

    // ─── all_tool_defs catalogue checks ──────────────────────────────────

    #[test]
    fn all_tool_defs_includes_every_canonical_tool_normal() {
        // Every primary tool name must appear in the catalogue. If a refactor
        // accidentally drops one (e.g. by gating it behind a feature flag),
        // the API call will 400 with "tool X not found".
        let defs = all_tool_defs();
        let names: Vec<&str> = defs.iter().map(|d| d.name.as_str()).collect();
        for required in [
            "Bash",
            "Read",
            "Write",
            "Edit",
            "Glob",
            "Grep",
            "TaskCreate",
            "TaskUpdate",
            "TaskList",
            "TaskDone",
            "Skill",
            "Task",
            "MemoryCreate",
            "MemoryDelete",
            "TeamCreate",
            "TeamDelete",
            "SendMessage",
            "TeamMemberMode",
        ] {
            assert!(
                names.contains(&required),
                "all_tool_defs missing {required}; got {names:?}",
            );
        }
    }

    #[test]
    fn all_tool_defs_have_object_schemas_robust() {
        // Anthropic's tool API requires `input_schema.type == "object"`. If
        // any tool ships a bare scalar schema the entire stream errors at
        // request time.
        for def in all_tool_defs() {
            assert_eq!(
                def.input_schema.get("type").and_then(|v| v.as_str()),
                Some("object"),
                "tool {} schema must be object-typed",
                def.name,
            );
        }
    }

    // ─── filter_tools_for_agent ──────────────────────────────────────────

    fn make_tool_def(name: &str) -> ToolDef {
        ToolDef {
            name: name.into(),
            description: "test".into(),
            input_schema: serde_json::json!({"type": "object"}),
        }
    }

    #[test]
    fn filter_tools_drops_task_unconditionally_robust() {
        // Recursive subagent spawning is intentionally blocked.
        let all = vec![make_tool_def("Bash"), make_tool_def("Task")];
        let filtered = filter_tools_for_agent(all, &[], &[]);
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].name, "Bash");
    }

    #[test]
    fn filter_tools_empty_allowed_means_all_normal() {
        let all = vec![
            make_tool_def("Bash"),
            make_tool_def("Read"),
            make_tool_def("Write"),
        ];
        let filtered = filter_tools_for_agent(all, &[], &[]);
        assert_eq!(filtered.len(), 3);
    }

    #[test]
    fn filter_tools_allowed_is_exact_membership_normal() {
        let all = vec![
            make_tool_def("Bash"),
            make_tool_def("Read"),
            make_tool_def("Write"),
        ];
        let filtered =
            filter_tools_for_agent(all, &["Read".into(), "Write".into()], &[]);
        assert_eq!(filtered.len(), 2);
        assert!(filtered.iter().any(|t| t.name == "Read"));
        assert!(filtered.iter().any(|t| t.name == "Write"));
    }

    #[test]
    fn filter_tools_disallowed_subtracts_from_allowed_normal() {
        let all = vec![
            make_tool_def("Bash"),
            make_tool_def("Read"),
            make_tool_def("Write"),
        ];
        let filtered = filter_tools_for_agent(all, &[], &["Bash".into()]);
        assert_eq!(filtered.len(), 2);
        assert!(filtered.iter().all(|t| t.name != "Bash"));
    }

    #[test]
    fn filter_tools_case_insensitive_robust() {
        // Allow/disallow lists in agent definitions are user-edited; case
        // variation must not silently drop or skip tools.
        let all = vec![make_tool_def("Bash"), make_tool_def("Read")];
        let filtered = filter_tools_for_agent(all, &["BASH".into()], &[]);
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].name, "Bash");
    }

    #[test]
    fn filter_tools_disallow_overrides_allow_robust() {
        let all = vec![make_tool_def("Bash"), make_tool_def("Read")];
        // Same tool both allow- and disallow-listed: disallow wins.
        let filtered =
            filter_tools_for_agent(all, &["Bash".into()], &["Bash".into()]);
        assert_eq!(filtered.len(), 0);
    }

    // ─── configure_tool_command — env stripping ──────────────────────────

    #[test]
    fn configure_tool_command_sets_no_prompt_envs_normal() {
        // We can't actually inspect the configured env without spawning,
        // so verify by running a bash command and checking the env it
        // sees. (If configure_tool_command silently regressed, the env
        // wouldn't be set and `$GIT_TERMINAL_PROMPT` would be empty.)
        let mut cmd = Command::new("bash");
        cmd.arg("-c")
            .arg("echo \"$GIT_TERMINAL_PROMPT|$SUDO_ASKPASS|$SSH_ASKPASS\"");
        configure_tool_command(&mut cmd);
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let out = rt.block_on(async { cmd.output().await.unwrap() });
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(stdout.contains("0|/bin/false|/bin/false"), "got: {stdout}");
    }

    // ─── non_interactive_shell_command — extra cases ─────────────────────

    #[test]
    fn non_interactive_bare_sudo_gets_minus_n_normal() {
        // Plain "sudo" with no args ought to still be made non-interactive.
        assert_eq!(non_interactive_shell_command("sudo"), "sudo -n");
    }

    #[test]
    fn non_interactive_already_minus_n_is_unchanged_robust() {
        assert_eq!(
            non_interactive_shell_command("sudo -n true"),
            "sudo -n true"
        );
    }

    #[test]
    fn non_interactive_preserves_leading_whitespace_normal() {
        // Pre-existing indentation in the user's script must stay intact —
        // shell heredocs and `set -e; sudo …` blocks rely on it.
        let cmd = "  sudo apt update";
        let out = non_interactive_shell_command(cmd);
        assert!(out.starts_with("  "), "leading ws lost: {out}");
        assert!(out.contains("sudo -n"), "{out}");
    }

    #[test]
    fn non_interactive_unrelated_command_unchanged_normal() {
        assert_eq!(non_interactive_shell_command("ls"), "ls");
        assert_eq!(non_interactive_shell_command(""), "");
    }

    // ─── terminal_safe_text — extra cases ────────────────────────────────

    #[test]
    fn terminal_safe_text_preserves_tab_newline_cr_normal() {
        let raw = "a\tb\nc\rd";
        assert_eq!(terminal_safe_text(raw), "a\tb\nc\rd");
    }

    #[test]
    fn terminal_safe_text_drops_lone_escape_normal() {
        // Lone escape with no follow-up is dropped (no terminal sequence
        // to consume) — all that remains is the surrounding text.
        let raw = "before\u{1b}";
        assert_eq!(terminal_safe_text(raw), "before");
    }

    #[test]
    fn terminal_safe_text_handles_osc_terminator_with_st_robust() {
        // OSC sequences can terminate with either BEL (\x07) or ST (ESC \\).
        let raw = "\u{1b}]0;title\u{1b}\\after";
        assert_eq!(terminal_safe_text(raw), "after");
    }

    #[test]
    fn terminal_safe_text_handles_unrecognized_escape_robust() {
        // ESC followed by something other than [ or ] consumes the next
        // byte and continues — no panic, no mojibake.
        let raw = "\u{1b}=normal";
        assert_eq!(terminal_safe_text(raw), "normal");
    }

    #[test]
    fn terminal_safe_text_passes_unicode_normal() {
        let raw = "héllo wörld 世界";
        assert_eq!(terminal_safe_text(raw), "héllo wörld 世界");
    }

    // ─── ExecutionResult builders ────────────────────────────────────────

    #[test]
    fn execution_result_success_has_no_diagnostics_normal() {
        let r = ExecutionResult::success("ok");
        assert!(!r.is_error());
        assert!(r.diagnostics.is_empty());
        assert!(r.diff.is_none());
        assert!(r.provenance.is_none());
    }

    #[test]
    fn execution_result_with_provenance_attaches_normal() {
        let r = ExecutionResult::success("ok").with_provenance(ToolProvenance {
            cwd: PathBuf::from("/tmp"),
            source: ToolSource::LocalExecutor,
        });
        assert!(r.provenance.is_some());
        assert_eq!(r.provenance.unwrap().cwd, PathBuf::from("/tmp"));
    }

    #[test]
    fn execution_result_with_diff_attaches_normal() {
        let view = crate::types::parse_unified_diff(
            "x.rs",
            "@@ -1,1 +1,1 @@\n-a\n+b\n",
        );
        let r = ExecutionResult::success("ok").with_diff(view);
        assert!(r.diff.is_some());
    }

    // ─── execute_bash dispatch ────────────────────────────────────────────

    #[tokio::test]
    async fn execute_bash_success_carries_provenance_normal() {
        let result = execute_bash("echo hello", Some(5_000), Path::new(".")).await;
        assert!(!result.is_error());
        assert!(result.output.contains("hello"), "{}", result.output);
        // Successful bash should attach provenance pointing at the cwd.
        assert!(result.provenance.is_some(), "bash success must carry cwd");
        assert_eq!(
            result.provenance.unwrap().source,
            ToolSource::LocalExecutor
        );
    }

    #[tokio::test]
    async fn execute_bash_nonzero_exit_is_complete_with_header_normal() {
        // Per Anthropic semantics, a non-zero exit code is *output*, not
        // a tool failure. The result is still Success and includes
        // `[exit N]` at the top so the model can read the code.
        let result =
            execute_bash("false", Some(5_000), Path::new(".")).await;
        assert!(!result.is_error(), "exit-1 must be Success: {:?}", result);
        assert!(result.output.contains("[exit 1]"), "{}", result.output);
    }

    #[tokio::test]
    async fn execute_bash_timeout_returns_failure_robust() {
        // sleep longer than the timeout — must time out cleanly.
        let result =
            execute_bash("sleep 5", Some(100), Path::new(".")).await;
        assert!(result.is_error());
        assert!(result.output.contains("timed out"), "{}", result.output);
    }

    #[tokio::test]
    async fn execute_bash_combines_stdout_and_stderr_normal() {
        let result = execute_bash(
            "echo out; echo err >&2",
            Some(5_000),
            Path::new("."),
        )
        .await;
        assert!(!result.is_error());
        assert!(result.output.contains("out"), "{}", result.output);
        assert!(result.output.contains("err"), "{}", result.output);
        assert!(
            result.output.contains("---stderr---"),
            "stdout+stderr split marker missing: {}",
            result.output
        );
    }

    #[tokio::test]
    async fn execute_bash_strips_ansi_escape_codes_normal() {
        // bash subprocess emits ANSI red — terminal_safe_text strips it.
        let result = execute_bash(
            "printf '\\033[31mred\\033[0m'",
            Some(5_000),
            Path::new("."),
        )
        .await;
        assert!(!result.is_error());
        assert!(!result.output.contains('\u{1b}'), "ANSI leaked: {:?}", result.output);
        assert!(result.output.contains("red"), "{}", result.output);
    }

    // ─── execute_read ─────────────────────────────────────────────────────

    #[tokio::test]
    async fn execute_read_returns_numbered_lines_normal() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("hello.txt");
        tokio::fs::write(&path, "alpha\nbravo\ncharlie\n").await.unwrap();

        let result =
            execute_read(path.to_str().unwrap(), None, None, None).await;
        assert!(!result.is_error());
        assert!(result.output.contains("1: alpha"), "{}", result.output);
        assert!(result.output.contains("2: bravo"), "{}", result.output);
        assert!(result.output.contains("3: charlie"), "{}", result.output);
    }

    #[tokio::test]
    async fn execute_read_directory_lists_entries_with_slash_suffix_normal() {
        let dir = tempfile::tempdir().expect("temp dir");
        tokio::fs::write(dir.path().join("a.txt"), "x").await.unwrap();
        tokio::fs::create_dir(dir.path().join("subdir")).await.unwrap();

        let result =
            execute_read(dir.path().to_str().unwrap(), None, None, None).await;
        assert!(!result.is_error());
        assert!(result.output.contains("a.txt"), "{}", result.output);
        assert!(result.output.contains("subdir/"), "dir suffix missing: {}", result.output);
    }

    #[tokio::test]
    async fn execute_read_offset_and_limit_paginate_normal() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("big.txt");
        let body: String = (1..=20).map(|i| format!("line{i}\n")).collect();
        tokio::fs::write(&path, body).await.unwrap();

        let result =
            execute_read(path.to_str().unwrap(), Some(5), Some(3), None).await;
        assert!(!result.is_error());
        // Should show lines 5, 6, 7 only.
        assert!(result.output.contains("5: line5"), "{}", result.output);
        assert!(result.output.contains("7: line7"), "{}", result.output);
        assert!(!result.output.contains("8: line8"), "{}", result.output);
        assert!(!result.output.contains("4: line4"), "{}", result.output);
    }

    #[tokio::test]
    async fn execute_read_missing_file_returns_failure_robust() {
        let result = execute_read(
            "/tmp/jfc-definitely-not-here-9999/x.txt",
            None,
            None,
            None,
        )
        .await;
        assert!(result.is_error());
        assert!(result.output.contains("Cannot read"), "{}", result.output);
    }

    #[tokio::test]
    async fn execute_read_dedup_returns_unchanged_marker_robust() {
        use crate::context::ReadDedupCache;

        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("d.txt");
        tokio::fs::write(&path, "stable\n").await.unwrap();

        let cache = Arc::new(Mutex::new(ReadDedupCache::new()));
        // First full read: populates the cache.
        let r1 = execute_read(
            path.to_str().unwrap(),
            None,
            None,
            Some(&cache),
        )
        .await;
        assert!(!r1.is_error());

        // Second full read on the unchanged file returns the dedup marker.
        let r2 = execute_read(
            path.to_str().unwrap(),
            None,
            None,
            Some(&cache),
        )
        .await;
        assert!(!r2.is_error());
        assert!(
            r2.output.contains("File unchanged since last full read"),
            "expected dedup stub, got: {}",
            r2.output
        );
    }

    #[tokio::test]
    async fn execute_read_paginated_skips_dedup_robust() {
        use crate::context::ReadDedupCache;

        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("p.txt");
        let body: String = (1..=10).map(|i| format!("L{i}\n")).collect();
        tokio::fs::write(&path, body).await.unwrap();

        let cache = Arc::new(Mutex::new(ReadDedupCache::new()));
        // Full read populates cache.
        let _ = execute_read(
            path.to_str().unwrap(),
            None,
            None,
            Some(&cache),
        )
        .await;
        // Paginated read on the same path: dedup must NOT short-circuit.
        let r = execute_read(
            path.to_str().unwrap(),
            Some(2),
            Some(3),
            Some(&cache),
        )
        .await;
        assert!(!r.is_error());
        assert!(!r.output.contains("File unchanged"), "{}", r.output);
        assert!(r.output.contains("2: L2"), "{}", r.output);
    }

    // ─── execute_write ────────────────────────────────────────────────────

    #[tokio::test]
    async fn execute_write_creates_file_with_summary_normal() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("new.txt");

        let result =
            execute_write(path.to_str().unwrap(), "hello\nworld\n").await;
        assert!(!result.is_error());
        assert!(path.exists(), "file should exist after write");
        let on_disk = tokio::fs::read_to_string(&path).await.unwrap();
        assert_eq!(on_disk, "hello\nworld\n");
        assert!(result.output.starts_with("Wrote "), "{}", result.output);
    }

    #[tokio::test]
    async fn execute_write_overwrite_uses_updated_header_normal() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("ow.txt");
        tokio::fs::write(&path, "original").await.unwrap();

        let result =
            execute_write(path.to_str().unwrap(), "replaced").await;
        assert!(!result.is_error());
        assert!(result.output.starts_with("Updated "), "{}", result.output);
    }

    #[tokio::test]
    async fn execute_write_creates_parent_dirs_normal() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("nested/two/three/file.txt");

        let result =
            execute_write(path.to_str().unwrap(), "x").await;
        assert!(!result.is_error(), "{}", result.output);
        assert!(path.exists());
    }

    #[tokio::test]
    async fn execute_write_long_content_truncates_preview_robust() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("long.txt");
        let body: String = (1..=100).map(|i| format!("line{i}\n")).collect();

        let result =
            execute_write(path.to_str().unwrap(), &body).await;
        assert!(!result.is_error());
        assert!(
            result.output.contains("more lines"),
            "should announce truncation: {}",
            result.output
        );
        // File on disk has the full content, even though preview is short.
        let on_disk = tokio::fs::read_to_string(&path).await.unwrap();
        assert_eq!(on_disk.lines().count(), 100);
    }

    // ─── execute_edit ─────────────────────────────────────────────────────

    #[tokio::test]
    async fn execute_edit_first_only_replaces_one_normal() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("e.txt");
        tokio::fs::write(&path, "foo bar foo").await.unwrap();

        let result = execute_edit(
            path.to_str().unwrap(),
            "foo",
            "BAZ",
            ReplacementMode::All,
        )
        .await;
        assert!(!result.is_error(), "{}", result.output);
        let after = tokio::fs::read_to_string(&path).await.unwrap();
        assert_eq!(after, "BAZ bar BAZ");
        assert!(result.diff.is_some(), "Edit must produce a DiffView");
    }

    #[tokio::test]
    async fn execute_edit_multiple_matches_without_replace_all_fails_robust() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("m.txt");
        tokio::fs::write(&path, "a a a").await.unwrap();

        let result = execute_edit(
            path.to_str().unwrap(),
            "a",
            "b",
            ReplacementMode::FirstOnly,
        )
        .await;
        assert!(result.is_error());
        assert!(
            result.output.contains("matches"),
            "expected 'multiple matches' error: {}",
            result.output
        );
    }

    #[tokio::test]
    async fn execute_edit_old_string_not_found_fails_robust() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("nf.txt");
        tokio::fs::write(&path, "abc").await.unwrap();

        let result = execute_edit(
            path.to_str().unwrap(),
            "missing",
            "x",
            ReplacementMode::FirstOnly,
        )
        .await;
        assert!(result.is_error());
        assert!(result.output.contains("not found"), "{}", result.output);
    }

    #[tokio::test]
    async fn execute_edit_empty_old_on_nonempty_file_rejects_robust() {
        // Empty old_string on a non-empty file is ambiguous (where to
        // insert?) so we reject — only allowed on a missing/empty file
        // as a "create" path.
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("ne.txt");
        tokio::fs::write(&path, "stuff").await.unwrap();

        let result = execute_edit(
            path.to_str().unwrap(),
            "",
            "new",
            ReplacementMode::FirstOnly,
        )
        .await;
        assert!(result.is_error());
        assert!(result.output.contains("old_string is empty"), "{}", result.output);
    }

    #[tokio::test]
    async fn execute_edit_empty_old_on_missing_file_creates_normal() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("create.txt");

        let result = execute_edit(
            path.to_str().unwrap(),
            "",
            "fresh content",
            ReplacementMode::FirstOnly,
        )
        .await;
        assert!(!result.is_error(), "{}", result.output);
        let body = tokio::fs::read_to_string(&path).await.unwrap();
        assert_eq!(body, "fresh content");
        assert!(result.output.contains("Created new file"), "{}", result.output);
    }

    #[tokio::test]
    async fn execute_edit_replace_all_mentions_count_normal() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("r.txt");
        tokio::fs::write(&path, "x x x x").await.unwrap();

        let result = execute_edit(
            path.to_str().unwrap(),
            "x",
            "Y",
            ReplacementMode::All,
        )
        .await;
        assert!(!result.is_error());
        assert!(
            result.output.contains("4 occurrences"),
            "{}",
            result.output
        );
    }

    // ─── build_edit_diff_view ────────────────────────────────────────────

    #[test]
    fn build_edit_diff_view_no_change_yields_empty_hunks_normal() {
        let view = build_edit_diff_view("x.rs", "abc\n", "abc\n");
        assert!(view.hunks.is_empty());
        assert_eq!(view.additions, 0);
        assert_eq!(view.deletions, 0);
    }

    #[test]
    fn build_edit_diff_view_counts_added_removed_normal() {
        let view = build_edit_diff_view(
            "x.rs",
            "a\nb\nc\n",
            "a\nB\nc\n",
        );
        assert_eq!(view.additions, 1);
        assert_eq!(view.deletions, 1);
        assert_eq!(view.hunks.len(), 1);
        assert_eq!(view.file_path, "x.rs");
    }

    #[test]
    fn build_edit_diff_view_pure_addition_robust() {
        let view = build_edit_diff_view(
            "x.rs",
            "a\nb\n",
            "a\nb\nc\n",
        );
        assert_eq!(view.additions, 1);
        assert_eq!(view.deletions, 0);
    }

    // ─── execute_glob ─────────────────────────────────────────────────────

    #[tokio::test]
    async fn execute_glob_matches_files_normal() {
        let dir = tempfile::tempdir().expect("temp dir");
        tokio::fs::write(dir.path().join("a.rs"), "").await.unwrap();
        tokio::fs::write(dir.path().join("b.rs"), "").await.unwrap();
        tokio::fs::write(dir.path().join("c.txt"), "").await.unwrap();

        let result = execute_glob("*.rs", None, dir.path()).await;
        assert!(!result.is_error(), "{}", result.output);
        assert!(result.output.contains("a.rs"), "{}", result.output);
        assert!(result.output.contains("b.rs"), "{}", result.output);
        assert!(!result.output.contains("c.txt"), "{}", result.output);
    }

    #[tokio::test]
    async fn execute_glob_no_match_returns_message_normal() {
        let dir = tempfile::tempdir().expect("temp dir");
        let result = execute_glob("*.zzz", None, dir.path()).await;
        assert!(!result.is_error());
        assert!(
            result.output.contains("No files matched"),
            "{}",
            result.output
        );
    }

    // ─── execute_grep ─────────────────────────────────────────────────────

    #[tokio::test]
    async fn execute_grep_finds_pattern_normal() {
        let dir = tempfile::tempdir().expect("temp dir");
        tokio::fs::write(
            dir.path().join("a.txt"),
            "line one\nlooking-for-this\nfinal\n",
        )
        .await
        .unwrap();

        let result =
            execute_grep("looking-for-this", None, None, None, dir.path()).await;
        assert!(!result.is_error(), "{}", result.output);
        assert!(result.output.contains("looking-for-this"), "{}", result.output);
    }

    #[tokio::test]
    async fn execute_grep_no_match_returns_message_normal() {
        let dir = tempfile::tempdir().expect("temp dir");
        tokio::fs::write(dir.path().join("a.txt"), "x\n").await.unwrap();

        let result =
            execute_grep("never-here-zzz", None, None, None, dir.path()).await;
        assert!(!result.is_error());
        assert!(result.output.contains("No matches"), "{}", result.output);
    }

    #[tokio::test]
    async fn execute_grep_files_with_matches_mode_normal() {
        let dir = tempfile::tempdir().expect("temp dir");
        tokio::fs::write(dir.path().join("a.txt"), "needle here\n")
            .await
            .unwrap();
        tokio::fs::write(dir.path().join("b.txt"), "no needle\n")
            .await
            .unwrap();

        let result = execute_grep(
            "needle",
            None,
            None,
            Some("files_with_matches"),
            dir.path(),
        )
        .await;
        assert!(!result.is_error(), "{}", result.output);
        assert!(result.output.contains("a.txt"), "{}", result.output);
    }

    // ─── execute_task_create / update / list / done ──────────────────────

    #[test]
    fn execute_task_create_without_store_fails_robust() {
        let r = execute_task_create(None, "subj".into(), "desc".into(), None, vec![]);
        assert!(r.is_error());
        assert!(r.output.contains("Task store not available"));
    }

    #[test]
    fn execute_task_create_with_store_returns_task_json_normal() {
        let store = TaskStore::in_memory();
        let r = execute_task_create(
            Some(store.clone()),
            "ship".into(),
            "release v1".into(),
            None,
            vec![],
        );
        assert!(!r.is_error(), "{:?}", r);
        // The output is the JSON of the created task — should mention the
        // subject and a `t1` id.
        assert!(r.output.contains("ship"), "{}", r.output);
        assert!(r.output.contains("t1"), "{}", r.output);
    }

    #[test]
    fn execute_task_create_with_unknown_dependency_fails_robust() {
        let store = TaskStore::in_memory();
        let r = execute_task_create(
            Some(store),
            "x".into(),
            "y".into(),
            None,
            vec!["t999".into()],
        );
        assert!(r.is_error(), "{:?}", r);
    }

    #[test]
    fn execute_task_update_without_store_fails_robust() {
        let r = execute_task_update(None, "t1", None, None, None, None);
        assert!(r.is_error());
    }

    #[test]
    fn execute_task_update_changes_status_normal() {
        let store = TaskStore::in_memory();
        let create = execute_task_create(
            Some(store.clone()),
            "alpha".into(),
            "do alpha".into(),
            None,
            vec![],
        );
        assert!(!create.is_error());
        // First-created task gets id `t1`.
        let r = execute_task_update(
            Some(store.clone()),
            "t1",
            Some("in_progress".into()),
            None,
            None,
            None,
        );
        assert!(!r.is_error(), "{}", r.output);
        assert!(r.output.contains("in_progress"), "{}", r.output);
    }

    #[test]
    fn execute_task_update_invalid_status_does_not_set_robust() {
        // Garbage status string: parser yields None and the patch leaves
        // status untouched. The update otherwise succeeds.
        let store = TaskStore::in_memory();
        execute_task_create(
            Some(store.clone()),
            "x".into(),
            "y".into(),
            None,
            vec![],
        );
        let r = execute_task_update(
            Some(store),
            "t1",
            Some("not_a_status".into()),
            Some("renamed".into()),
            None,
            None,
        );
        assert!(!r.is_error(), "{}", r.output);
        assert!(r.output.contains("renamed"), "{}", r.output);
    }

    #[test]
    fn execute_task_done_marks_completed_normal() {
        let store = TaskStore::in_memory();
        execute_task_create(
            Some(store.clone()),
            "do".into(),
            "it".into(),
            None,
            vec![],
        );
        let r = execute_task_done(Some(store), "t1");
        assert!(!r.is_error(), "{}", r.output);
        assert!(r.output.contains("completed"), "{}", r.output);
    }

    #[test]
    fn execute_task_done_unknown_id_fails_robust() {
        let store = TaskStore::in_memory();
        let r = execute_task_done(Some(store), "tnosuch");
        assert!(r.is_error());
    }

    #[test]
    fn execute_task_list_without_store_fails_robust() {
        let r = execute_task_list(None, None, None);
        assert!(r.is_error());
    }

    #[test]
    fn execute_task_list_returns_tasks_normal() {
        let store = TaskStore::in_memory();
        execute_task_create(
            Some(store.clone()),
            "alpha".into(),
            "first".into(),
            None,
            vec![],
        );
        execute_task_create(
            Some(store.clone()),
            "bravo".into(),
            "second".into(),
            None,
            vec![],
        );
        let r = execute_task_list(Some(store), None, None);
        assert!(!r.is_error(), "{}", r.output);
        assert!(r.output.contains("alpha"), "{}", r.output);
        assert!(r.output.contains("bravo"), "{}", r.output);
    }

    #[test]
    fn execute_task_list_filters_by_owner_robust() {
        let store = TaskStore::in_memory();
        execute_task_create(
            Some(store.clone()),
            "x".into(),
            "y".into(),
            None,
            vec![],
        );
        execute_task_update(
            Some(store.clone()),
            "t1",
            None,
            None,
            None,
            Some("alice".into()),
        );
        let only_alice =
            execute_task_list(Some(store.clone()), None, Some("alice"));
        assert!(only_alice.output.contains("alice"), "{}", only_alice.output);

        let only_bob = execute_task_list(Some(store), None, Some("bob"));
        assert!(!only_bob.output.contains("alice"), "{}", only_bob.output);
    }

    // ─── execute_memory_create / delete ──────────────────────────────────

    #[test]
    fn execute_memory_create_invalid_level_fails_robust() {
        let dir = tempfile::tempdir().expect("temp dir");
        let r = execute_memory_create(
            "bogus", "context", "private", "body", dir.path(),
        );
        assert!(r.is_error());
        assert!(r.output.contains("Invalid level"), "{}", r.output);
    }

    #[test]
    fn execute_memory_create_invalid_type_fails_robust() {
        let dir = tempfile::tempdir().expect("temp dir");
        let r = execute_memory_create(
            "user", "wibble", "private", "body", dir.path(),
        );
        assert!(r.is_error());
    }

    #[test]
    fn execute_memory_create_invalid_scope_fails_robust() {
        let dir = tempfile::tempdir().expect("temp dir");
        let r = execute_memory_create(
            "user", "context", "wibble", "body", dir.path(),
        );
        assert!(r.is_error());
    }

    #[test]
    fn execute_memory_create_empty_body_fails_robust() {
        let dir = tempfile::tempdir().expect("temp dir");
        let r = execute_memory_create(
            "project", "context", "private", "   ", dir.path(),
        );
        assert!(r.is_error());
        assert!(r.output.contains("body cannot be empty"), "{}", r.output);
    }

    #[test]
    fn execute_memory_create_project_writes_file_normal() {
        let dir = tempfile::tempdir().expect("temp dir");
        let r = execute_memory_create(
            "project",
            "context",
            "private",
            "Remember the alamo.",
            dir.path(),
        );
        assert!(!r.is_error(), "{}", r.output);
        assert!(r.output.contains("Memory saved to"), "{}", r.output);
    }

    #[test]
    fn execute_memory_delete_missing_path_fails_robust() {
        let r = execute_memory_delete("/tmp/jfc-no-such-memory-path-xyz-9831.md");
        assert!(r.is_error());
        assert!(r.output.contains("File not found"), "{}", r.output);
    }

    #[test]
    fn execute_memory_delete_outside_memory_dir_rejected_robust() {
        // delete_memory refuses paths outside ~/.config/jfc/memory or
        // <project>/.jfc/memory. A scratch file in tempdir hits that
        // guardrail — the executor surfaces the failure cleanly.
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("not-a-memory.md");
        std::fs::write(&path, "scratch").unwrap();
        let r = execute_memory_delete(path.to_str().unwrap());
        assert!(r.is_error(), "expected failure for path outside memory dir");
        assert!(
            r.output.contains("Failed to delete memory"),
            "{}",
            r.output
        );
    }

    // ─── execute_team_member_mode validation ─────────────────────────────

    #[tokio::test]
    async fn execute_team_member_mode_invalid_mode_fails_robust() {
        let r =
            execute_team_member_mode("alice", "godmode", Some("alpha")).await;
        assert!(r.is_error());
        assert!(r.output.contains("Invalid mode"), "{}", r.output);
    }

    #[tokio::test]
    async fn execute_team_member_mode_no_team_fails_robust() {
        // Mode is valid but there's no active team.
        let r = execute_team_member_mode("alice", "default", None).await;
        assert!(r.is_error());
        assert!(r.output.contains("No active team"), "{}", r.output);
    }

    // ─── execute_tool dispatch ────────────────────────────────────────────

    #[tokio::test]
    async fn execute_tool_dispatches_bash_normal() {
        let dir = tempfile::tempdir().expect("temp dir");
        let result = execute_tool(
            ToolKind::Bash,
            ToolInput::Bash {
                command: "echo dispatched".into(),
                timeout: Some(5_000),
                workdir: None,
            },
            dir.path().to_path_buf(),
            None,
            None,
            None,
        )
        .await;
        assert!(!result.is_error(), "{}", result.output);
        assert!(result.output.contains("dispatched"), "{}", result.output);
    }

    #[tokio::test]
    async fn execute_tool_task_kind_rejects_with_streaming_message_robust() {
        // The Task tool can't be dispatched through the normal executor;
        // it requires the streaming path. The dispatcher returns a clear
        // error rather than silently no-op'ing.
        let r = execute_tool(
            ToolKind::Task,
            ToolInput::Task(crate::types::TaskInput {
                description: "x".into(),
                prompt: "y".into(),
                subagent_type: None,
                category: None,
                run_in_background: false,
                model: None,
                name: None,
                team_name: None,
                mode: None,
                isolation: None,
            }),
            PathBuf::from("."),
            None,
            None,
            None,
        )
        .await;
        assert!(r.is_error());
        assert!(r.output.contains("streaming"), "{}", r.output);
    }

    #[tokio::test]
    async fn execute_tool_kind_input_mismatch_falls_through_robust() {
        // Mismatched kind/input pair returns "not yet implemented" so a
        // routing bug surfaces clearly rather than silently dropping.
        let r = execute_tool(
            ToolKind::Bash,
            ToolInput::Generic {
                summary: "wrong shape".into(),
            },
            PathBuf::from("."),
            None,
            None,
            None,
        )
        .await;
        assert!(r.is_error());
        assert!(r.output.contains("not yet implemented"), "{}", r.output);
    }

    #[tokio::test]
    async fn execute_tool_invalidates_dedup_after_write_normal() {
        use crate::context::ReadDedupCache;

        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("inv.txt");
        tokio::fs::write(&path, "v1\n").await.unwrap();

        let cache = Arc::new(Mutex::new(ReadDedupCache::new()));

        // Prime cache with a Read.
        let r1 = execute_tool(
            ToolKind::Read,
            ToolInput::Read {
                file_path: path.to_string_lossy().to_string(),
                offset: None,
                limit: None,
            },
            dir.path().to_path_buf(),
            Some(cache.clone()),
            None,
            None,
        )
        .await;
        assert!(!r1.is_error());

        // Write through the dispatcher — this should invalidate the cache.
        let w = execute_tool(
            ToolKind::Write,
            ToolInput::Write {
                file_path: path.to_string_lossy().to_string(),
                content: "v2\n".into(),
            },
            dir.path().to_path_buf(),
            Some(cache.clone()),
            None,
            None,
        )
        .await;
        assert!(!w.is_error());

        // Next Read should NOT short-circuit with the dedup stub.
        let r2 = execute_tool(
            ToolKind::Read,
            ToolInput::Read {
                file_path: path.to_string_lossy().to_string(),
                offset: None,
                limit: None,
            },
            dir.path().to_path_buf(),
            Some(cache),
            None,
            None,
        )
        .await;
        assert!(!r2.is_error());
        assert!(
            !r2.output.contains("File unchanged"),
            "Write should have invalidated the dedup cache: {}",
            r2.output
        );
        assert!(r2.output.contains("v2"), "{}", r2.output);
    }

    #[tokio::test]
    async fn execute_tool_dispatches_glob_normal() {
        let dir = tempfile::tempdir().expect("temp dir");
        tokio::fs::write(dir.path().join("hit.rs"), "").await.unwrap();
        let r = execute_tool(
            ToolKind::Glob,
            ToolInput::Glob {
                pattern: "*.rs".into(),
                path: None,
            },
            dir.path().to_path_buf(),
            None,
            None,
            None,
        )
        .await;
        assert!(!r.is_error(), "{}", r.output);
        assert!(r.output.contains("hit.rs"), "{}", r.output);
    }

    #[tokio::test]
    async fn execute_tool_dispatches_task_create_normal() {
        let store = TaskStore::in_memory();
        let r = execute_tool(
            ToolKind::TaskCreate,
            ToolInput::TaskCreate {
                subject: "via dispatcher".into(),
                description: "test".into(),
                active_form: None,
                blocked_by: vec![],
            },
            PathBuf::from("."),
            None,
            Some(store),
            None,
        )
        .await;
        assert!(!r.is_error(), "{}", r.output);
        assert!(r.output.contains("via dispatcher"), "{}", r.output);
    }
}

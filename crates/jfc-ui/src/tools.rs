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
            description: "Spawn a sub-agent to handle a focused task. The sub-agent runs with the same provider/model and returns a result. Use run_in_background=true for parallel work.".into(),
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
                    }
                },
                "required": ["description", "prompt", "run_in_background"]
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
}

impl ExecutionResult {
    pub fn success(output: impl Into<String>) -> Self {
        Self {
            output: output.into(),
            outcome: ToolOutcome::Success,
            diagnostics: Vec::new(),
            provenance: None,
        }
    }

    pub fn failure(output: impl Into<String>) -> Self {
        let output = output.into();
        Self {
            diagnostics: vec![ToolDiagnostic::error(output.clone())],
            output,
            outcome: ToolOutcome::Failed,
            provenance: None,
        }
    }

    pub fn with_provenance(mut self, provenance: ToolProvenance) -> Self {
        self.provenance = Some(provenance);
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
        (kind, _) => ExecutionResult::failure(format!("Tool {:?} not yet implemented", kind)),
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
        if let Some(cache) = dedup {
            let guard = cache.lock().await;
            if guard.is_unchanged(&path) {
                trace!(target: "jfc::tools", file_path, "read: dedup cache hit, file unchanged");
                return ExecutionResult::success(
                    "File unchanged since last read. The content from the \
                             earlier Read tool_result in this conversation is still \
                             current — refer to that instead of re-reading."
                        .to_string(),
                );
            }
            drop(guard);
        }

        match tokio::fs::read_to_string(&path).await {
            Ok(content) => {
                let max_lines = limit.unwrap_or(2000) as usize;
                let start = offset.unwrap_or(1).saturating_sub(1) as usize;
                let lines: Vec<&str> = content.lines().collect();
                let slice = &lines[start.min(lines.len())..];
                let slice = &slice[..slice.len().min(max_lines)];
                let numbered: String = slice
                    .iter()
                    .enumerate()
                    .map(|(i, line)| format!("{}: {line}", start + i + 1))
                    .collect::<Vec<_>>()
                    .join("\n");

                if let Some(cache) = dedup {
                    cache.lock().await.record_read(path);
                }

                debug!(target: "jfc::tools", file_path, line_count = slice.len(), "read: success");
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
                    if replacement.replace_all() && count > 1 {
                        ExecutionResult::success(format!(
                            "The file {file_path} has been updated ({line_summary}). All {count} occurrences replaced."
                        ))
                    } else {
                        ExecutionResult::success(format!(
                            "The file {file_path} has been updated successfully ({line_summary})."
                        ))
                    }
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
    match crate::agents::find_skill_by_name(&skills, name) {
        Some(skill) => {
            let body = match args.filter(|s| !s.is_empty()) {
                Some(a) => format!("{}\n\n# Args\n{}", skill.body, a),
                None => skill.body.clone(),
            };
            ExecutionResult::success(body)
        }
        None => ExecutionResult::failure(format!("Unknown skill: {name}")),
    }
}

pub async fn execute_task(
    task_input: &crate::types::TaskInput,
    provider: &dyn crate::provider::Provider,
    model_id: crate::provider::ModelId,
    tx: Option<&tokio::sync::mpsc::UnboundedSender<crate::app::AppEvent>>,
    task_id: Option<&str>,
    agent_def: Option<&crate::agents::AgentDef>,
) -> ExecutionResult {
    use crate::provider::{
        ProviderContent, ProviderMessage, ProviderRole, StreamEvent, StreamOptions,
    };
    use futures::StreamExt;

    let messages = vec![ProviderMessage {
        role: ProviderRole::User,
        content: vec![ProviderContent::Text(task_input.prompt.clone())],
    }];

    let model = if let Some(m) = &task_input.model {
        crate::provider::ModelId::new(m.clone())
    } else {
        model_id
    };

    // If a matching `AgentDef` was passed, build its effective system
    // prompt by concatenating each referenced skill body. Skipped skills
    // (missing names) are logged as warnings inside
    // `build_agent_system_prompt`. When no agent matched, the spawned
    // task runs without a system prompt, preserving prior behavior.
    let options = StreamOptions::new(model);
    let options = match agent_def {
        Some(agent) => {
            let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
            let skills = crate::agents::load_skills(&cwd);
            let system_prompt = crate::agents::build_agent_system_prompt(agent, &skills);
            options.system(system_prompt)
        }
        None => options,
    };

    let stream = match provider.stream(messages, &options).await {
        Ok(s) => s,
        Err(e) => return ExecutionResult::failure(format!("Task stream error: {e}")),
    };

    tokio::pin!(stream);

    let mut text = String::new();
    let mut error: Option<String> = None;

    while let Some(event) = stream.next().await {
        match event {
            Ok(StreamEvent::TextDelta { delta, .. }) => {
                // Pipe each chunk into the parent's event loop tagged
                // with this subagent's task id. The main handler
                // appends it to `BackgroundTask.messages` so the task
                // view shows the agent's prose live as it streams.
                // Mirrors v126's per-agent stream forwarding.
                if let (Some(tx), Some(id)) = (tx, task_id) {
                    let _ = tx.send(crate::app::AppEvent::AgentChunk {
                        task_id: id.to_owned(),
                        text: delta.clone(),
                    });
                }
                text.push_str(&delta);
            }
            Ok(StreamEvent::TextDone { text: t, .. }) => {
                if text.is_empty() {
                    text = t;
                }
            }
            Ok(StreamEvent::Error { message }) => {
                error = Some(message);
                break;
            }
            Err(e) => {
                error = Some(e.to_string());
                break;
            }
            Ok(_) => {}
        }
    }

    if let Some(err) = error {
        ExecutionResult::failure(err)
    } else {
        ExecutionResult::success(text)
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
}

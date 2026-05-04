use std::path::{Path, PathBuf};
use std::sync::Arc;

use tokio::process::Command;
use tokio::sync::Mutex;

use crate::context::ReadDedupCache;
use crate::provider::ToolDef;
use crate::tasks::{TaskPatch, TaskStatus, TaskStore};
use crate::types::{ToolInput, ToolKind};

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
    ]
}

pub struct ExecutionResult {
    pub output: String,
    pub is_error: bool,
}

/// REQ-TOOLS-002: Tool executors — bash/read/write/edit/glob/grep/task via tokio + fs.
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
            if !result.is_error {
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
                replace_all,
            },
        ) => {
            let result = execute_edit(&file_path, &old_string, &new_string, replace_all).await;
            if !result.is_error {
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
        (kind, _) => ExecutionResult {
            output: format!("Tool {:?} not yet implemented", kind),
            is_error: true,
        },
    }
}

async fn execute_bash(command: &str, timeout_ms: Option<u64>, cwd: &Path) -> ExecutionResult {
    let timeout = timeout_ms.unwrap_or(120_000);
    let mut cmd = Command::new("bash");
    cmd.arg("-c")
        .arg(command)
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
    let result =
        tokio::time::timeout(std::time::Duration::from_millis(timeout), cmd.output()).await;

    match result {
        Ok(Ok(out)) => {
            let stdout = String::from_utf8_lossy(&out.stdout);
            let stderr = String::from_utf8_lossy(&out.stderr);
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
            ExecutionResult {
                output: format!("{header}{}", body.trim_end()),
                is_error: false,
            }
        }
        Ok(Err(e)) => ExecutionResult {
            output: format!("Failed to spawn bash: {e}"),
            is_error: true,
        },
        Err(_) => ExecutionResult {
            output: format!("Command timed out after {timeout}ms"),
            is_error: true,
        },
    }
}

async fn execute_read(
    file_path: &str,
    offset: Option<u64>,
    limit: Option<u64>,
    dedup: Option<&Arc<Mutex<ReadDedupCache>>>,
) -> ExecutionResult {
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
                ExecutionResult {
                    output: names.join("\n"),
                    is_error: false,
                }
            }
            Err(e) => ExecutionResult {
                output: format!("Cannot read directory: {e}"),
                is_error: true,
            },
        }
    } else {
        if let Some(cache) = dedup {
            let guard = cache.lock().await;
            if guard.is_unchanged(&path) {
                return ExecutionResult {
                    output: "File unchanged since last read. The content from the \
                             earlier Read tool_result in this conversation is still \
                             current — refer to that instead of re-reading."
                        .to_string(),
                    is_error: false,
                };
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

                ExecutionResult {
                    output: numbered,
                    is_error: false,
                }
            }
            Err(e) => ExecutionResult {
                output: format!("Cannot read file: {e}"),
                is_error: true,
            },
        }
    }
}

async fn execute_write(file_path: &str, content: &str) -> ExecutionResult {
    let path = PathBuf::from(file_path);
    if let Some(parent) = path.parent() {
        if let Err(e) = tokio::fs::create_dir_all(parent).await {
            return ExecutionResult {
                output: format!("Cannot create directories: {e}"),
                is_error: true,
            };
        }
    }
    match tokio::fs::write(&path, content).await {
        Ok(_) => ExecutionResult {
            output: format!("Written {} bytes to {file_path}", content.len()),
            is_error: false,
        },
        Err(e) => ExecutionResult {
            output: format!("Cannot write file: {e}"),
            is_error: true,
        },
    }
}

async fn execute_edit(
    file_path: &str,
    old_string: &str,
    new_string: &str,
    replace_all: bool,
) -> ExecutionResult {
    match tokio::fs::read_to_string(file_path).await {
        Ok(content) => {
            if old_string.is_empty() && !content.is_empty() {
                return ExecutionResult {
                    output: "old_string is empty but file is not empty. Provide text to replace."
                        .into(),
                    is_error: true,
                };
            }
            let count = content.matches(old_string).count();
            if count == 0 {
                return ExecutionResult {
                    output: format!("old_string not found in {file_path}"),
                    is_error: true,
                };
            }
            if count > 1 && !replace_all {
                return ExecutionResult {
                    output: format!(
                        "Found {count} matches for old_string in {file_path}. Use replace_all=true or provide more context."
                    ),
                    is_error: true,
                };
            }
            let new_content = if replace_all {
                content.replace(old_string, new_string)
            } else {
                content.replacen(old_string, new_string, 1)
            };
            match tokio::fs::write(file_path, &new_content).await {
                Ok(_) => ExecutionResult {
                    output: format!("Replaced {count} occurrence(s) in {file_path}"),
                    is_error: false,
                },
                Err(e) => ExecutionResult {
                    output: format!("Cannot write file after edit: {e}"),
                    is_error: true,
                },
            }
        }
        Err(_) if old_string.is_empty() => match tokio::fs::write(file_path, new_string).await {
            Ok(_) => ExecutionResult {
                output: format!("Created new file {file_path}"),
                is_error: false,
            },
            Err(e2) => ExecutionResult {
                output: format!("Cannot create file: {e2}"),
                is_error: true,
            },
        },
        Err(e) => ExecutionResult {
            output: format!("Cannot read file: {e}"),
            is_error: true,
        },
    }
}

async fn execute_glob(pattern: &str, path: Option<&str>, cwd: &Path) -> ExecutionResult {
    let base = path.map(PathBuf::from).unwrap_or_else(|| cwd.to_path_buf());
    let mut cmd = Command::new("rg");
    cmd.arg("--files")
        .arg("--glob")
        .arg(pattern)
        .current_dir(&base);
    match cmd.output().await {
        Ok(out) => {
            let stdout = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if stdout.is_empty() {
                ExecutionResult {
                    output: "No files matched".into(),
                    is_error: false,
                }
            } else {
                ExecutionResult {
                    output: stdout,
                    is_error: false,
                }
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

    match cmd.output().await {
        Ok(out) => {
            let stdout = String::from_utf8_lossy(&out.stdout).trim().to_string();
            let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
            if stdout.is_empty() && out.status.code() == Some(1) {
                ExecutionResult {
                    output: "No matches found".into(),
                    is_error: false,
                }
            } else if !stderr.is_empty() && stdout.is_empty() {
                ExecutionResult {
                    output: stderr,
                    is_error: true,
                }
            } else {
                ExecutionResult {
                    output: stdout,
                    is_error: false,
                }
            }
        }
        Err(e) => ExecutionResult {
            output: format!("rg not found or failed: {e}"),
            is_error: true,
        },
    }
}

fn execute_task_create(
    store: Option<Arc<TaskStore>>,
    subject: String,
    description: String,
    active_form: Option<String>,
    blocked_by: Vec<String>,
) -> ExecutionResult {
    let Some(store) = store else {
        return ExecutionResult {
            output: "Task store not available".into(),
            is_error: true,
        };
    };
    match store.create(subject, description, active_form, blocked_by) {
        Ok(task) => ExecutionResult {
            output: serde_json::to_string_pretty(&task).unwrap_or_else(|_| format!("{task:?}")),
            is_error: false,
        },
        Err(e) => ExecutionResult {
            output: e,
            is_error: true,
        },
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
    let Some(store) = store else {
        return ExecutionResult {
            output: "Task store not available".into(),
            is_error: true,
        };
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
        Ok(task) => ExecutionResult {
            output: serde_json::to_string_pretty(&task).unwrap_or_else(|_| format!("{task:?}")),
            is_error: false,
        },
        Err(e) => ExecutionResult {
            output: e,
            is_error: true,
        },
    }
}

fn execute_task_list(
    store: Option<Arc<TaskStore>>,
    status_filter: Option<&str>,
    owner_filter: Option<&str>,
) -> ExecutionResult {
    let Some(store) = store else {
        return ExecutionResult {
            output: "Task store not available".into(),
            is_error: true,
        };
    };
    let mut tasks = store.list(false);
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
    let output =
        serde_json::to_string_pretty(&tasks).unwrap_or_else(|_| format!("{} tasks", tasks.len()));
    ExecutionResult {
        output,
        is_error: false,
    }
}

fn execute_task_done(store: Option<Arc<TaskStore>>, task_id: &str) -> ExecutionResult {
    let Some(store) = store else {
        return ExecutionResult {
            output: "Task store not available".into(),
            is_error: true,
        };
    };
    let patch = TaskPatch {
        status: Some(TaskStatus::Completed),
        ..Default::default()
    };
    match store.update(task_id, patch) {
        Ok(task) => ExecutionResult {
            output: serde_json::to_string_pretty(&task).unwrap_or_else(|_| format!("{task:?}")),
            is_error: false,
        },
        Err(e) => ExecutionResult {
            output: e,
            is_error: true,
        },
    }
}

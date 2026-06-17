//! Slash handlers: task-store CRUD.

use crate::commands::prelude::*;
use crate::runtime::EngineEvent;

pub(super) async fn cmd_task_list(
    state: &mut EngineState,
    parts: &[&str],
    text: &str,
    tx: Option<&mpsc::Sender<EngineEvent>>,
) {
    if parts
        .get(1)
        .is_some_and(|arg| arg.split_whitespace().next() == Some("clear"))
    {
        cmd_task_clear(state, parts, text, tx).await;
        return;
    }

    let tasks = state.task_store.list(jfc_session::DeletedFilter::Exclude);
    let body = if tasks.is_empty() {
        "No tasks. Use `/task-add <subject>` to create one.".to_owned()
    } else {
        let mut s = format!("**{} task(s):**\n\n", tasks.len());
        for t in &tasks {
            let icon = t.status.glyph();
            let owner = t
                .owner
                .as_deref()
                .map(|o| format!(" (@{o})"))
                .unwrap_or_default();
            let blocks = if t.blocked_by.is_empty() {
                String::new()
            } else {
                format!(
                    " · blocked by {}",
                    t.blocked_by
                        .iter()
                        .map(|id| id.as_str())
                        .collect::<Vec<_>>()
                        .join(",")
                )
            };
            s.push_str(&format!(
                "{} `{}` {}{}{}\n",
                icon, t.id, t.subject, owner, blocks
            ));
        }
        let c = state.task_store.counts();
        s.push_str(&format!(
            "\n*{} pending, {} in progress, {} completed*",
            c.pending, c.in_progress, c.completed
        ));
        s
    };
    state.messages.push(ChatMessage::user("/tasks".into()));
    state.messages.push(ChatMessage::assistant(body));
}

pub(super) async fn cmd_task_clear(
    state: &mut EngineState,
    parts: &[&str],
    text: &str,
    _tx: Option<&mpsc::Sender<EngineEvent>>,
) {
    let mut args = parts.get(1).copied().unwrap_or("").split_whitespace();
    let first = args.next().unwrap_or("");
    let mode = if first == "clear" {
        args.next().unwrap_or("terminal")
    } else if first.is_empty() {
        "terminal"
    } else {
        first
    };
    let mode = match mode {
        "" | "done" | "completed" | "terminal" => TaskClearMode::Terminal,
        "open" | "active" | "pending" | "todo" | "todos" => TaskClearMode::Open,
        "all" => TaskClearMode::All,
        other => {
            state.messages.push(ChatMessage::assistant(format!(
                "Usage: `/task-clear [terminal|open|all]` (unknown mode `{other}`)"
            )));
            return;
        }
    };

    let tasks = state.task_store.list(jfc_session::DeletedFilter::Exclude);
    let open_count = tasks.iter().filter(|task| task.status.is_open()).count();
    let ids: Vec<String> = tasks
        .iter()
        .filter(|task| mode.matches(task.status))
        .map(|task| task.id.as_str().to_owned())
        .collect();
    for id in &ids {
        let _ = state.task_store.delete(id);
    }

    let label = match mode {
        TaskClearMode::Terminal => "terminal",
        TaskClearMode::Open => "open",
        TaskClearMode::All => "all",
    };
    let command = if text.trim().is_empty() {
        format!("/task-clear {label}")
    } else {
        text.trim().to_owned()
    };
    state.messages.push(ChatMessage::user(command));
    if ids.is_empty() {
        let hint = if mode == TaskClearMode::Terminal && open_count > 0 {
            format!(
                " {open_count} open task(s) remain; use `/task-clear open` to clear the visible open list, or `/task-clear all` to remove every task."
            )
        } else {
            String::new()
        };
        state.messages.push(ChatMessage::assistant(format!(
            "No {label} tasks to clear.{hint}"
        )));
    } else {
        state.messages.push(ChatMessage::assistant(format!(
            "Cleared {} {label} task(s).",
            ids.len()
        )));
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TaskClearMode {
    Terminal,
    Open,
    All,
}

impl TaskClearMode {
    fn matches(self, status: jfc_session::TaskStatus) -> bool {
        match self {
            Self::Terminal => status.is_terminal() && status != jfc_session::TaskStatus::Deleted,
            Self::Open => status.is_open(),
            Self::All => true,
        }
    }
}

pub(super) async fn cmd_task_add(
    state: &mut EngineState,
    parts: &[&str],
    _text: &str,
    _tx: Option<&mpsc::Sender<EngineEvent>>,
) {
    let subject = parts.get(1).copied().unwrap_or("").trim();
    if subject.is_empty() {
        state.messages.push(ChatMessage::assistant(
            "Usage: `/task-add <subject>`".into(),
        ));
    } else {
        match state.task_store.create(
            subject.to_owned(),
            String::new(),
            None,
            Vec::<jfc_session::TaskId>::new(),
        ) {
            Ok(t) => {
                state
                    .messages
                    .push(ChatMessage::user(format!("/task-add {subject}")));
                state.messages.push(ChatMessage::assistant(format!(
                    "Created task `{}`: {}",
                    t.id, t.subject
                )));
            }
            Err(e) => {
                state
                    .messages
                    .push(ChatMessage::assistant(format!("**Error:** {e}")));
            }
        }
    }
}

pub(super) async fn cmd_task_done(
    state: &mut EngineState,
    parts: &[&str],
    _text: &str,
    _tx: Option<&mpsc::Sender<EngineEvent>>,
) {
    let id = parts.get(1).copied().unwrap_or("").trim();
    if id.is_empty() {
        let open_count = state
            .task_store
            .list(jfc_session::DeletedFilter::Exclude)
            .iter()
            .filter(|task| task.status.is_open())
            .count();
        let hint = if open_count > 0 {
            format!(" Use `/task-done all` to complete the {open_count} open task(s).")
        } else {
            String::new()
        };
        state.messages.push(ChatMessage::assistant(format!(
            "Usage: `/task-done <id|all>` (e.g. `/task-done t3`).{hint}"
        )));
    } else if matches!(id, "all" | "open") {
        let tasks = state.task_store.list(jfc_session::DeletedFilter::Exclude);
        let ids: Vec<String> = tasks
            .iter()
            .filter(|task| task.status.is_open())
            .map(|task| task.id.as_str().to_owned())
            .collect();
        for id in &ids {
            let _ = state.task_store.update(
                id,
                jfc_session::TaskPatch {
                    status: Some(jfc_session::TaskStatus::Completed),
                    ..Default::default()
                },
            );
        }
        state
            .messages
            .push(ChatMessage::user("/task-done all".into()));
        if ids.is_empty() {
            state
                .messages
                .push(ChatMessage::assistant("No open tasks to complete.".into()));
        } else {
            state.messages.push(ChatMessage::assistant(format!(
                "Completed {} open task(s).",
                ids.len()
            )));
        }
    } else {
        match state.task_store.update(
            id,
            jfc_session::TaskPatch {
                status: Some(jfc_session::TaskStatus::Completed),
                ..Default::default()
            },
        ) {
            Ok(t) => {
                state
                    .messages
                    .push(ChatMessage::user(format!("/task-done {id}")));
                state.messages.push(ChatMessage::assistant(format!(
                    "✓ Completed `{}`: {}",
                    t.id, t.subject
                )));
            }
            Err(e) => {
                state
                    .messages
                    .push(ChatMessage::assistant(format!("**Error:** {e}")));
            }
        }
    }
}

pub(super) async fn cmd_task_rm(
    state: &mut EngineState,
    parts: &[&str],
    _text: &str,
    _tx: Option<&mpsc::Sender<EngineEvent>>,
) {
    let id = parts.get(1).copied().unwrap_or("").trim();
    if id.is_empty() {
        state
            .messages
            .push(ChatMessage::assistant("Usage: `/task-rm <id>`".into()));
    } else {
        match state.task_store.delete(id) {
            Ok(()) => {
                state
                    .messages
                    .push(ChatMessage::user(format!("/task-rm {id}")));
                state
                    .messages
                    .push(ChatMessage::assistant(format!("Deleted task `{id}`.")));
            }
            Err(e) => {
                state
                    .messages
                    .push(ChatMessage::assistant(format!("**Error:** {e}")));
            }
        }
    }
}

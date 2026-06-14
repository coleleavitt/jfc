//! Slash handlers: task-store CRUD.

use crate::commands::prelude::*;
use crate::runtime::EngineEvent;

pub(super) async fn cmd_task_list(
    state: &mut EngineState,
    _parts: &[&str],
    _text: &str,
    _tx: Option<&mpsc::Sender<EngineEvent>>,
) {
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
        state.messages.push(ChatMessage::assistant(
            "Usage: `/task-done <id>` (e.g. `/task-done t3`)".into(),
        ));
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

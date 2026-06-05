//! Slash handlers: task-store CRUD.

use super::*;

pub(super) async fn cmd_task_list(
    app: &mut App,
    _parts: &[&str],
    _text: &str,
    _tx: Option<&mpsc::Sender<AppEvent>>,
) {
    let tasks = app.task_store.list(jfc_session::DeletedFilter::Exclude);
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
        let c = app.task_store.counts();
        s.push_str(&format!(
            "\n*{} pending, {} in progress, {} completed*",
            c.pending, c.in_progress, c.completed
        ));
        s
    };
    app.messages.push(ChatMessage::user("/tasks".into()));
    app.messages.push(ChatMessage::assistant(body));
}

pub(super) async fn cmd_task_add(
    app: &mut App,
    parts: &[&str],
    _text: &str,
    _tx: Option<&mpsc::Sender<AppEvent>>,
) {
    let subject = parts.get(1).copied().unwrap_or("").trim();
    if subject.is_empty() {
        app.messages.push(ChatMessage::assistant(
            "Usage: `/task-add <subject>`".into(),
        ));
    } else {
        match app.task_store.create(
            subject.to_owned(),
            String::new(),
            None,
            Vec::<jfc_session::TaskId>::new(),
        ) {
            Ok(t) => {
                app.messages
                    .push(ChatMessage::user(format!("/task-add {subject}")));
                app.messages.push(ChatMessage::assistant(format!(
                    "Created task `{}`: {}",
                    t.id, t.subject
                )));
            }
            Err(e) => {
                app.messages
                    .push(ChatMessage::assistant(format!("**Error:** {e}")));
            }
        }
    }
}

pub(super) async fn cmd_task_done(
    app: &mut App,
    parts: &[&str],
    _text: &str,
    _tx: Option<&mpsc::Sender<AppEvent>>,
) {
    let id = parts.get(1).copied().unwrap_or("").trim();
    if id.is_empty() {
        app.messages.push(ChatMessage::assistant(
            "Usage: `/task-done <id>` (e.g. `/task-done t3`)".into(),
        ));
    } else {
        match app.task_store.update(
            id,
            jfc_session::TaskPatch {
                status: Some(jfc_session::TaskStatus::Completed),
                ..Default::default()
            },
        ) {
            Ok(t) => {
                app.messages
                    .push(ChatMessage::user(format!("/task-done {id}")));
                app.messages.push(ChatMessage::assistant(format!(
                    "✓ Completed `{}`: {}",
                    t.id, t.subject
                )));
            }
            Err(e) => {
                app.messages
                    .push(ChatMessage::assistant(format!("**Error:** {e}")));
            }
        }
    }
}

pub(super) async fn cmd_task_rm(
    app: &mut App,
    parts: &[&str],
    _text: &str,
    _tx: Option<&mpsc::Sender<AppEvent>>,
) {
    let id = parts.get(1).copied().unwrap_or("").trim();
    if id.is_empty() {
        app.messages
            .push(ChatMessage::assistant("Usage: `/task-rm <id>`".into()));
    } else {
        match app.task_store.delete(id) {
            Ok(()) => {
                app.messages
                    .push(ChatMessage::user(format!("/task-rm {id}")));
                app.messages
                    .push(ChatMessage::assistant(format!("Deleted task `{id}`.")));
            }
            Err(e) => {
                app.messages
                    .push(ChatMessage::assistant(format!("**Error:** {e}")));
            }
        }
    }
}

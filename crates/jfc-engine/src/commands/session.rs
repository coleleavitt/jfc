//! Slash handlers: session & transcript lifecycle.

use crate::commands::prelude::*;
use crate::runtime::EngineEvent;

pub(super) async fn cmd_rename(
    state: &mut EngineState,
    parts: &[&str],
    _text: &str,
    _tx: Option<&mpsc::Sender<EngineEvent>>,
) {
    // Set a custom title on the current session. v126 cli.js:39786
    // calls this `customTitle` and it sits at the top of the title
    // precedence chain (custom → ai → firstPrompt → id-slice).
    // Persisted to the session JSON so it survives restarts.
    let new_title = parts.get(1).copied().unwrap_or("").trim().to_owned();
    state
        .messages
        .push(ChatMessage::user(format!("/rename {new_title}")));
    match (&state.current_session_id, new_title.is_empty()) {
        (None, _) => {
            state.messages.push(ChatMessage::assistant(
                "No active session to rename. Send a message first.".into(),
            ));
        }
        (_, true) => {
            state.messages.push(ChatMessage::assistant(
                        "Usage: `/rename <title>`. Pass any text to set the session title; the picker / sidebar will show it.".into(),
                    ));
        }
        (Some(id), false) => {
            crate::session::set_session_title(id, &new_title).await;
            state.messages.push(ChatMessage::assistant(format!(
                "Session `{id}` renamed to **{new_title}**.",
            )));
        }
    }
}

pub(super) async fn cmd_clear(
    state: &mut EngineState,
    _parts: &[&str],
    _text: &str,
    _tx: Option<&mpsc::Sender<EngineEvent>>,
) {
    state.messages.clear();
    state.streaming_text.clear();
    state.streaming_reasoning.clear();
    state.streaming_response_bytes = 0;
    state.streaming_assistant_idx = None;
    state.clear_active_stream_scope();
    // Mint a fresh session id and wipe per-session state (tasks,
    // completion timers). v126 cli.js:271511 keys todos by sessionId
    // so a new session inherently has an empty list — match that.
    state.switch_session(None);
}

pub(super) async fn cmd_continue(
    state: &mut EngineState,
    parts: &[&str],
    _text: &str,
    _tx: Option<&mpsc::Sender<EngineEvent>>,
) {
    // v126 / codex-rs parity: `/continue` is cwd-scoped by default.
    // `/continue all` (or `/c all`) shows the globally most recent
    // — useful when the user moved a project or wants any session.
    // The original behavior (global most-recent) caused the
    // "continue from project A accidentally resumed project B"
    // confusion the user reported.
    let want_global = parts.get(1).copied().map(str::trim) == Some("all");
    let session_id = if want_global {
        jfc_session::most_recent_session().await
    } else {
        let cwd_str = std::env::current_dir()
            .ok()
            .map(|p| p.display().to_string());
        jfc_session::most_recent_session_for_cwd(cwd_str.as_deref()).await
    };
    if let Some(session_id) = session_id {
        if let Some(messages) = crate::session::load_session(&session_id).await {
            let msg_count = messages.len();
            state.messages = messages;
            let session_id_for_msg = session_id.clone();
            state.switch_session(Some(session_id));
            state.streaming_text.clear();
            state.streaming_reasoning.clear();
            state.streaming_response_bytes = 0;
            state.streaming_assistant_idx = None;
            state.clear_active_stream_scope();
            state.push_effect(crate::app::EngineEffect::ScrollToBottom);
            let scope = if want_global { "any cwd" } else { "this cwd" };
            state.messages.push(ChatMessage::assistant(format!(
                "**Resumed session `{session_id_for_msg}`** ({scope}) — {msg_count} message(s) loaded."
            )));
        } else {
            state.messages.push(ChatMessage::assistant(format!(
                "**Error:** Failed to load session `{session_id}`."
            )));
        }
    } else {
        let hint = if want_global {
            "No previous sessions found anywhere."
        } else {
            "No previous sessions found in this cwd. Try `/continue all` for any session."
        };
        state.messages.push(ChatMessage::assistant(hint.into()));
    }
}

pub(super) async fn cmd_resume(
    state: &mut EngineState,
    parts: &[&str],
    _text: &str,
    _tx: Option<&mpsc::Sender<EngineEvent>>,
) {
    // Resume a specific session by id. Accepts an optional
    // `--force` token to suppress the cwd-mismatch warning
    // (mirrors codex-rs `tui/src/session_resume.rs:99-111`,
    // where the user explicitly opts in to a cross-project
    // resume).
    let raw_args = parts.get(1).copied().unwrap_or("").trim();
    let mut force = false;
    let mut session_id = "";
    for tok in raw_args.split_whitespace() {
        if tok == "--force" {
            force = true;
        } else if session_id.is_empty() {
            session_id = tok;
        }
    }
    if session_id.is_empty() {
        // List available sessions
        let sessions = jfc_session::list_sessions().await;
        if sessions.is_empty() {
            state.messages.push(ChatMessage::assistant(
                "No sessions found. Usage: `/resume <session_id>`".into(),
            ));
        } else {
            let list = sessions
                .iter()
                .take(10)
                .map(|s| format!("  - `{s}`"))
                .collect::<Vec<_>>()
                .join("\n");
            let more = if sessions.len() > 10 {
                format!("\n  ... and {} more", sessions.len() - 10)
            } else {
                String::new()
            };
            state.messages.push(ChatMessage::assistant(format!(
                "**Usage:** `/resume <session_id>`\n\n**Available sessions:**\n{list}{more}"
            )));
        }
    } else {
        let typed_session_id = crate::ids::SessionId::new(session_id);
        if let Some(messages) = crate::session::load_session(&typed_session_id).await {
            let msg_count = messages.len();
            // Compare the loaded session's recorded cwd against the
            // current process cwd before mutating state state. The
            // resume still proceeds either way — the toast is just
            // informational so the user notices they may be
            // pointing at the wrong project.
            if !force {
                let session_cwd = jfc_session::load_session_metadata(&typed_session_id)
                    .await
                    .and_then(|m| m.cwd);
                let current_cwd = std::env::current_dir()
                    .map(|p| p.to_string_lossy().into_owned())
                    .unwrap_or_default();
                if let Some(msg) =
                    jfc_session::cwd_mismatch_message(session_cwd.as_deref(), &current_cwd)
                {
                    crate::toast::push_with_cap(
                        &mut state.toasts,
                        crate::toast::Toast::new(crate::toast::ToastKind::Warning, msg),
                    );
                }
            }
            state.messages = messages;
            state.switch_session(Some(typed_session_id.clone()));
            state.streaming_text.clear();
            state.streaming_reasoning.clear();
            state.streaming_response_bytes = 0;
            state.streaming_assistant_idx = None;
            state.clear_active_stream_scope();
            state.push_effect(crate::app::EngineEffect::ScrollToBottom);
            state.messages.push(ChatMessage::assistant(format!(
                "**Resumed session `{typed_session_id}`** — {msg_count} message(s) loaded."
            )));
        } else {
            state.messages.push(ChatMessage::assistant(format!(
                "**Error:** Session `{typed_session_id}` not found."
            )));
        }
    }
}

pub(super) async fn cmd_sessions(
    state: &mut EngineState,
    _parts: &[&str],
    _text: &str,
    _tx: Option<&mpsc::Sender<EngineEvent>>,
) {
    // List all sessions with metadata
    let sessions = jfc_session::list_sessions_with_metadata().await;
    if sessions.is_empty() {
        state
            .messages
            .push(ChatMessage::assistant("No sessions found.".into()));
    } else {
        let mut body = format!("**{} session(s):**\n\n", sessions.len());
        for (i, s) in sessions.iter().take(20).enumerate() {
            let prompt = s.first_prompt.as_deref().unwrap_or("(no prompt)");
            let prompt_display = if prompt.len() > 50 {
                let boundary = prompt.floor_char_boundary(50);
                format!("{}…", &prompt[..boundary])
            } else {
                prompt.to_string()
            };
            let current = state.current_session_id.as_ref() == Some(&s.id);
            let marker = if current { " ← current" } else { "" };
            body.push_str(&format!(
                "{}. `{}`{} — {} msg(s)\n   {}\n",
                i + 1,
                s.id,
                marker,
                s.message_count,
                prompt_display
            ));
        }
        if sessions.len() > 20 {
            body.push_str(&format!(
                "\n... and {} more (use Ctrl+B sidebar)",
                sessions.len() - 20
            ));
        }
        state.messages.push(ChatMessage::user("/sessions".into()));
        state.messages.push(ChatMessage::assistant(body));
    }
}
pub(super) async fn cmd_fork(
    state: &mut EngineState,
    parts: &[&str],
    text: &str,
    _tx: Option<&mpsc::Sender<EngineEvent>>,
) {
    state.messages.push(ChatMessage::user(text.to_owned()));
    let arg = parts
        .get(1)
        .copied()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let upto = match arg {
        None => state.messages.len(),
        Some(s) => match s.parse::<usize>() {
            Ok(n) if n <= state.messages.len() => n,
            _ => {
                state.messages.push(ChatMessage::assistant(format!(
                            "Usage: `/fork [N]` — snapshot first N messages as a new session. \
                             Got `{s}`, which doesn't parse or exceeds the current message count ({}).",
                            state.messages.len()
                        )));
                return;
            }
        },
    };
    if upto == 0 {
        state.messages.push(ChatMessage::assistant(
            "Can't fork at message 0 — there's nothing to snapshot. Send a message first."
                .to_owned(),
        ));
        return;
    }
    // Snapshot to a brand-new session id. We keep `state.messages`
    // truncated to `upto` to mirror what `git checkout -b` does
    // visually, then mint a fresh id; the parent session JSON on
    // disk is untouched because `switch_session` only points at
    // the new id from here on out.
    state.messages.truncate(upto);
    state.streaming_text.clear();
    state.streaming_reasoning.clear();
    state.streaming_response_bytes = 0;
    state.streaming_assistant_idx = None;
    state.clear_active_stream_scope();
    // Mint a fresh session id (same flow as /clear) — the next
    // turn will save under the new id, and `state.current_session_id`
    // becomes the fork's anchor.
    state.switch_session(None);
    let new_id = state
        .current_session_id
        .as_ref()
        .map(|s| s.as_str().to_owned())
        .unwrap_or_else(|| "(unset)".to_owned());
    state.messages.push(ChatMessage::assistant(format!(
        "**Forked** at message {upto}/{total}. New session: `{new_id}`. \
                 The original is preserved — `/resume` it any time.",
        total = upto
    )));
}

pub(super) async fn cmd_undo(
    state: &mut EngineState,
    _parts: &[&str],
    text: &str,
    _tx: Option<&mpsc::Sender<EngineEvent>>,
) {
    // Revert the most recent Edit / Write / MultiEdit /
    // ApplyPatch tool's filesystem mutation. Pulls from
    // the static undo stack in tools/registry which the
    // tool dispatcher populates by capturing pre-mutation
    // file content before the tool executes. Only undoes
    // ONE step; run /undo repeatedly to walk back further.
    state.messages.push(ChatMessage::user(text.to_owned()));
    let entry = crate::tools::pop_undo_entry();
    let Some(entry) = entry else {
        state.messages.push(ChatMessage::assistant(
            "Nothing to undo — no recent file mutation captured this session.".into(),
        ));
        return;
    };
    let path = std::path::PathBuf::from(&entry.file_path);
    match entry.previous_content.clone() {
        Some(prev) => match std::fs::write(&path, &prev) {
            Ok(()) => {
                state.messages.push(ChatMessage::assistant(format!(
                    "Reverted `{}` to its pre-{} state ({} bytes restored).",
                    path.display(),
                    entry.op_label,
                    prev.len()
                )));
            }
            Err(e) => {
                crate::tools::restore_undo_entry(entry);
                state.messages.push(ChatMessage::assistant(format!(
                    "Failed to write `{}`: {e} (kept the entry, run /undo again after fixing)",
                    path.display(),
                )));
            }
        },
        None => match std::fs::remove_file(&path) {
            Ok(()) => {
                state.messages.push(ChatMessage::assistant(format!(
                    "Reverted `{}` (deleted; was newly-created by `{}`).",
                    path.display(),
                    entry.op_label
                )));
            }
            Err(e) => {
                crate::tools::restore_undo_entry(entry);
                state.messages.push(ChatMessage::assistant(format!(
                    "Failed to remove `{}`: {e}",
                    path.display(),
                )));
            }
        },
    }
}

pub(super) async fn cmd_recap(
    state: &mut EngineState,
    _parts: &[&str],
    _text: &str,
    _tx: Option<&mpsc::Sender<EngineEvent>>,
) {
    // Generate a one-line session recap from all messages.
    // Mirrors the auto-generation in ui_actions.rs that fires on
    // idle return (>5 min inactivity). Reuses the same recap generator
    // so the logic is consistent across both paths.
    state.messages.push(ChatMessage::user("/recap".into()));

    // Collect RecapMessage from all current messages, same as
    // the idle-return handler does (but with all messages, not
    // just those since last interaction).
    let recap_messages: Vec<crate::session_recap::RecapMessage> = state
        .messages
        .iter()
        .map(|m| {
            let text_preview = m
                .parts
                .iter()
                .find_map(|p| match p {
                    jfc_core::MessagePart::Text(t) if !t.is_empty() => {
                        Some(t.chars().take(160).collect::<String>())
                    }
                    _ => None,
                })
                .unwrap_or_default();
            let tool_calls: Vec<String> = m
                .parts
                .iter()
                .filter_map(|p| match p {
                    jfc_core::MessagePart::Tool(t) => Some(t.kind.label().to_string()),
                    _ => None,
                })
                .collect();
            let files_changed: Vec<String> = m
                .parts
                .iter()
                .filter_map(|p| match p {
                    jfc_core::MessagePart::Tool(t) => match &t.input {
                        jfc_core::ToolInput::Edit { file_path, .. }
                        | jfc_core::ToolInput::Write { file_path, .. } => Some(file_path.clone()),
                        _ => None,
                    },
                    _ => None,
                })
                .collect();
            let had_error = m.parts.iter().any(|p| {
                matches!(
                    p,
                    jfc_core::MessagePart::Tool(t)
                        if t.status == jfc_core::ExecutionStatus::Failed
                )
            });
            crate::session_recap::RecapMessage {
                is_assistant: m.role == jfc_core::Role::Assistant,
                tool_calls,
                had_error,
                files_changed,
                text_preview,
            }
        })
        .collect();

    // Generate the recap from collected messages.
    match crate::session_recap::generate_recap(&recap_messages) {
        Some(recap) => {
            state.messages.push(ChatMessage::assistant(recap));
        }
        None => {
            state.messages.push(ChatMessage::assistant(
                "No meaningful activity to recap — no tool calls, file changes, or errors detected."
                    .into(),
            ));
        }
    }
}

pub(super) async fn cmd_export(
    state: &mut EngineState,
    parts: &[&str],
    text: &str,
    _tx: Option<&mpsc::Sender<EngineEvent>>,
) {
    // /export <path>: write the transcript as markdown to the
    // given path (defaults to ./jfc-transcript.md).
    state.messages.push(ChatMessage::user(text.to_owned()));
    let raw_path = parts.get(1).copied().unwrap_or("").trim();
    let path: std::path::PathBuf = if raw_path.is_empty() {
        std::path::PathBuf::from("jfc-transcript.md")
    } else {
        std::path::PathBuf::from(raw_path)
    };
    let mut body = String::from("# jfc transcript\n\n");
    for msg in &state.messages {
        let role = match msg.role {
            jfc_core::Role::User => "User",
            jfc_core::Role::Assistant => "Assistant",
        };
        body.push_str(&format!("## {role}\n\n"));
        for part in &msg.parts {
            match part {
                jfc_core::MessagePart::Text(t) => {
                    body.push_str(t);
                    body.push_str("\n\n");
                }
                jfc_core::MessagePart::Reasoning(t) => {
                    body.push_str("> _thinking_\n> \n> ");
                    body.push_str(&t.replace('\n', "\n> "));
                    body.push_str("\n\n");
                }
                jfc_core::MessagePart::Tool(tc) => {
                    body.push_str(&format!(
                        "- **Tool: {}** ({})\n",
                        tc.kind.label(),
                        tc.status.label()
                    ));
                    body.push_str(&format!("  Input: {}\n", tc.input.summary()));
                    body.push('\n');
                }
                _ => {}
            }
        }
    }
    match std::fs::write(&path, &body) {
        Ok(()) => {
            let message = format!(
                "Wrote transcript ({} bytes) to `{}`.",
                body.len(),
                path.display()
            );
            state.messages.push(ChatMessage::assistant(message.clone()));
            crate::toast::push_with_cap(
                &mut state.toasts,
                crate::toast::Toast::new(crate::toast::ToastKind::Success, message),
            );
        }
        Err(e) => {
            let message = format!("Failed to write `{}`: {e}", path.display());
            state.messages.push(ChatMessage::assistant(message.clone()));
            crate::toast::push_with_cap(
                &mut state.toasts,
                crate::toast::Toast::new(crate::toast::ToastKind::Error, message),
            );
        }
    }
}

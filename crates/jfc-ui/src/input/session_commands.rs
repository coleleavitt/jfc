//! Slash handlers: session & transcript lifecycle.

use super::*;

pub(super) async fn cmd_rename(
    app: &mut App,
    parts: &[&str],
    _text: &str,
    _tx: Option<&mpsc::Sender<AppEvent>>,
) {
    // Set a custom title on the current session. v126 cli.js:39786
    // calls this `customTitle` and it sits at the top of the title
    // precedence chain (custom → ai → firstPrompt → id-slice).
    // Persisted to the session JSON so it survives restarts.
    let new_title = parts.get(1).copied().unwrap_or("").trim().to_owned();
    app.messages
        .push(ChatMessage::user(format!("/rename {new_title}")));
    match (&app.current_session_id, new_title.is_empty()) {
        (None, _) => {
            app.messages.push(ChatMessage::assistant(
                "No active session to rename. Send a message first.".into(),
            ));
        }
        (_, true) => {
            app.messages.push(ChatMessage::assistant(
                        "Usage: `/rename <title>`. Pass any text to set the session title; the picker / sidebar will show it.".into(),
                    ));
        }
        (Some(id), false) => {
            crate::session::set_session_title(id, &new_title).await;
            app.messages.push(ChatMessage::assistant(format!(
                "Session `{id}` renamed to **{new_title}**.",
            )));
        }
    }
}

pub(super) async fn cmd_clear(
    app: &mut App,
    _parts: &[&str],
    _text: &str,
    _tx: Option<&mpsc::Sender<AppEvent>>,
) {
    app.messages.clear();
    app.streaming_text.clear();
    app.streaming_reasoning.clear();
    app.streaming_response_bytes = 0;
    app.streaming_assistant_idx = None;
    // Mint a fresh session id and wipe per-session state (tasks,
    // completion timers). v126 cli.js:271511 keys todos by sessionId
    // so a new session inherently has an empty list — match that.
    app.switch_session(None);
}

pub(super) async fn cmd_continue(
    app: &mut App,
    parts: &[&str],
    _text: &str,
    _tx: Option<&mpsc::Sender<AppEvent>>,
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
            app.messages = messages;
            let session_id_for_msg = session_id.clone();
            app.switch_session(Some(session_id));
            app.streaming_text.clear();
            app.streaming_reasoning.clear();
            app.streaming_response_bytes = 0;
            app.streaming_assistant_idx = None;
            app.scroll_to_bottom();
            let scope = if want_global { "any cwd" } else { "this cwd" };
            app.messages.push(ChatMessage::assistant(format!(
                "**Resumed session `{session_id_for_msg}`** ({scope}) — {} message(s) loaded.",
                app.messages.len() - 1
            )));
        } else {
            app.messages.push(ChatMessage::assistant(format!(
                "**Error:** Failed to load session `{session_id}`."
            )));
        }
    } else {
        let hint = if want_global {
            "No previous sessions found anywhere."
        } else {
            "No previous sessions found in this cwd. Try `/continue all` for any session."
        };
        app.messages.push(ChatMessage::assistant(hint.into()));
    }
}

pub(super) async fn cmd_resume(
    app: &mut App,
    parts: &[&str],
    _text: &str,
    _tx: Option<&mpsc::Sender<AppEvent>>,
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
            app.messages.push(ChatMessage::assistant(
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
            app.messages.push(ChatMessage::assistant(format!(
                "**Usage:** `/resume <session_id>`\n\n**Available sessions:**\n{list}{more}"
            )));
        }
    } else {
        let typed_session_id = crate::ids::SessionId::new(session_id);
        if let Some(messages) = crate::session::load_session(&typed_session_id).await {
            let msg_count = messages.len();
            // Compare the loaded session's recorded cwd against the
            // current process cwd before mutating app state. The
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
                        &mut app.toasts,
                        crate::toast::Toast::new(crate::toast::ToastKind::Warning, msg),
                    );
                }
            }
            app.messages = messages;
            app.switch_session(Some(typed_session_id.clone()));
            app.streaming_text.clear();
            app.streaming_reasoning.clear();
            app.streaming_response_bytes = 0;
            app.streaming_assistant_idx = None;
            app.scroll_to_bottom();
            app.messages.push(ChatMessage::assistant(format!(
                "**Resumed session `{typed_session_id}`** — {msg_count} message(s) loaded."
            )));
        } else {
            app.messages.push(ChatMessage::assistant(format!(
                "**Error:** Session `{typed_session_id}` not found."
            )));
        }
    }
}

pub(super) async fn cmd_sessions(
    app: &mut App,
    _parts: &[&str],
    _text: &str,
    _tx: Option<&mpsc::Sender<AppEvent>>,
) {
    // List all sessions with metadata
    let sessions = jfc_session::list_sessions_with_metadata().await;
    if sessions.is_empty() {
        app.messages
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
            let current = app.current_session_id.as_ref() == Some(&s.id);
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
        app.messages.push(ChatMessage::user("/sessions".into()));
        app.messages.push(ChatMessage::assistant(body));
    }
}

pub(super) async fn cmd_copy(
    app: &mut App,
    parts: &[&str],
    text: &str,
    _tx: Option<&mpsc::Sender<AppEvent>>,
) {
    app.messages.push(ChatMessage::user(text.to_owned()));
    let arg = parts
        .get(1)
        .copied()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let (payload, scope_label) = match arg {
        None | Some("last") => {
            let body = crate::runtime::last_assistant_text(app).unwrap_or_default();
            (body, "last assistant message".to_owned())
        }
        Some("all") => {
            let body = crate::runtime::full_transcript_text(app);
            (body, "full transcript".to_owned())
        }
        Some(other) => {
            // Numeric tail (`/copy 3` → last 3 messages). On parse
            // failure, fall back to `last` so a typo still copies
            // something useful rather than yielding an error.
            match other.parse::<usize>() {
                Ok(n) if n > 0 => {
                    let body = crate::runtime::tail_transcript_text(app, n);
                    (body, format!("last {n} message(s)"))
                }
                _ => {
                    let body = crate::runtime::last_assistant_text(app).unwrap_or_default();
                    (
                        body,
                        format!("last assistant message (unrecognized arg `{other}`)"),
                    )
                }
            }
        }
    };
    if payload.is_empty() {
        app.messages.push(ChatMessage::assistant(
            "Nothing to copy — the requested scope contains no text.".to_owned(),
        ));
    } else {
        crate::runtime::copy_to_clipboard(&payload, "/copy");
        app.messages.push(ChatMessage::assistant(format!(
                    "Copied {scope_label} ({} chars) to clipboard. OSC 52 escape emitted for SSH/tmux clients.",
                    payload.chars().count()
                )));
    }
}

pub(super) async fn cmd_fork(
    app: &mut App,
    parts: &[&str],
    text: &str,
    _tx: Option<&mpsc::Sender<AppEvent>>,
) {
    app.messages.push(ChatMessage::user(text.to_owned()));
    let arg = parts
        .get(1)
        .copied()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let upto = match arg {
        None => app.messages.len(),
        Some(s) => match s.parse::<usize>() {
            Ok(n) if n <= app.messages.len() => n,
            _ => {
                app.messages.push(ChatMessage::assistant(format!(
                            "Usage: `/fork [N]` — snapshot first N messages as a new session. \
                             Got `{s}`, which doesn't parse or exceeds the current message count ({}).",
                            app.messages.len()
                        )));
                return;
            }
        },
    };
    if upto == 0 {
        app.messages.push(ChatMessage::assistant(
            "Can't fork at message 0 — there's nothing to snapshot. Send a message first."
                .to_owned(),
        ));
        return;
    }
    // Snapshot to a brand-new session id. We keep `app.messages`
    // truncated to `upto` to mirror what `git checkout -b` does
    // visually, then mint a fresh id; the parent session JSON on
    // disk is untouched because `switch_session` only points at
    // the new id from here on out.
    app.messages.truncate(upto);
    app.streaming_text.clear();
    app.streaming_reasoning.clear();
    app.streaming_response_bytes = 0;
    app.streaming_assistant_idx = None;
    // Mint a fresh session id (same flow as /clear) — the next
    // turn will save under the new id, and `app.current_session_id`
    // becomes the fork's anchor.
    app.switch_session(None);
    let new_id = app
        .current_session_id
        .as_ref()
        .map(|s| s.as_str().to_owned())
        .unwrap_or_else(|| "(unset)".to_owned());
    app.messages.push(ChatMessage::assistant(format!(
        "**Forked** at message {upto}/{total}. New session: `{new_id}`. \
                 The original is preserved — `/resume` it any time.",
        total = upto
    )));
}

pub(super) async fn cmd_undo(
    app: &mut App,
    _parts: &[&str],
    text: &str,
    _tx: Option<&mpsc::Sender<AppEvent>>,
) {
    // Revert the most recent Edit / Write / MultiEdit /
    // ApplyPatch tool's filesystem mutation. Pulls from
    // the static undo stack in tools/registry which the
    // tool dispatcher populates by capturing pre-mutation
    // file content before the tool executes. Only undoes
    // ONE step; run /undo repeatedly to walk back further.
    app.messages.push(ChatMessage::user(text.to_owned()));
    let entry = crate::tools::pop_undo_entry();
    let Some(entry) = entry else {
        app.messages.push(ChatMessage::assistant(
            "Nothing to undo — no recent file mutation captured this session.".into(),
        ));
        return;
    };
    let path = std::path::PathBuf::from(&entry.file_path);
    match entry.previous_content.clone() {
        Some(prev) => match std::fs::write(&path, &prev) {
            Ok(()) => {
                app.messages.push(ChatMessage::assistant(format!(
                    "Reverted `{}` to its pre-{} state ({} bytes restored).",
                    path.display(),
                    entry.op_label,
                    prev.len()
                )));
            }
            Err(e) => {
                crate::tools::restore_undo_entry(entry.clone());
                app.messages.push(ChatMessage::assistant(format!(
                    "Failed to write `{}`: {e} (kept the entry, run /undo again after fixing)",
                    path.display(),
                )));
            }
        },
        None => match std::fs::remove_file(&path) {
            Ok(()) => {
                app.messages.push(ChatMessage::assistant(format!(
                    "Reverted `{}` (deleted; was newly-created by `{}`).",
                    path.display(),
                    entry.op_label
                )));
            }
            Err(e) => {
                crate::tools::restore_undo_entry(entry.clone());
                app.messages.push(ChatMessage::assistant(format!(
                    "Failed to remove `{}`: {e}",
                    path.display(),
                )));
            }
        },
    }
}

pub(super) async fn cmd_export(
    app: &mut App,
    parts: &[&str],
    text: &str,
    _tx: Option<&mpsc::Sender<AppEvent>>,
) {
    // /export <path>: write the transcript as markdown to the
    // given path (defaults to ./jfc-transcript.md).
    app.messages.push(ChatMessage::user(text.to_owned()));
    let raw_path = parts.get(1).copied().unwrap_or("").trim();
    let path: std::path::PathBuf = if raw_path.is_empty() {
        std::path::PathBuf::from("jfc-transcript.md")
    } else {
        std::path::PathBuf::from(raw_path)
    };
    let mut body = String::from("# jfc transcript\n\n");
    for msg in &app.messages {
        let role = match msg.role {
            crate::types::Role::User => "User",
            crate::types::Role::Assistant => "Assistant",
        };
        body.push_str(&format!("## {role}\n\n"));
        for part in &msg.parts {
            match part {
                crate::types::MessagePart::Text(t) => {
                    body.push_str(t);
                    body.push_str("\n\n");
                }
                crate::types::MessagePart::Reasoning(t) => {
                    body.push_str("> _thinking_\n> \n> ");
                    body.push_str(&t.replace('\n', "\n> "));
                    body.push_str("\n\n");
                }
                crate::types::MessagePart::Tool(tc) => {
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
            app.messages.push(ChatMessage::assistant(message.clone()));
            crate::toast::push_with_cap(
                &mut app.toasts,
                crate::toast::Toast::new(crate::toast::ToastKind::Success, message),
            );
        }
        Err(e) => {
            let message = format!("Failed to write `{}`: {e}", path.display());
            app.messages.push(ChatMessage::assistant(message.clone()));
            crate::toast::push_with_cap(
                &mut app.toasts,
                crate::toast::Toast::new(crate::toast::ToastKind::Error, message),
            );
        }
    }
}

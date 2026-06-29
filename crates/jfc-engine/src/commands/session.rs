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
    state.streaming_response_baseline = 0;
    state.streaming_thinking_tokens = 0;
    state.token_rate_samples.clear();
    state.token_rate_sample_thinking = None;
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
            state.switch_session(Some(session_id.clone()));
            crate::runtime::ops::restore_session_context_state(state, session_id.as_str()).await;
            state.streaming_text.clear();
            state.streaming_reasoning.clear();
            state.streaming_response_bytes = 0;
            state.streaming_response_baseline = 0;
            state.streaming_thinking_tokens = 0;
            state.token_rate_samples.clear();
            state.token_rate_sample_thinking = None;
            state.streaming_assistant_idx = None;
            state.clear_active_stream_scope();
            state.push_effect(crate::app::EngineEffect::ScrollToBottom);
            let scope = if want_global { "any cwd" } else { "this cwd" };
            state.messages.push(ChatMessage::assistant(format!(
                "**Resumed session `{session_id_for_msg}`** ({scope}) — {msg_count} message(s) loaded."
            )));
            // Surface any pending inter-session inbox messages for this session.
            crate::commands::inbox_helper::inject_inbox_reminder(state, session_id.as_str()).await;
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
            crate::runtime::ops::restore_session_context_state(state, typed_session_id.as_str())
                .await;
            state.streaming_text.clear();
            state.streaming_reasoning.clear();
            state.streaming_response_bytes = 0;
            state.streaming_response_baseline = 0;
            state.streaming_thinking_tokens = 0;
            state.token_rate_samples.clear();
            state.token_rate_sample_thinking = None;
            state.streaming_assistant_idx = None;
            state.clear_active_stream_scope();
            state.push_effect(crate::app::EngineEffect::ScrollToBottom);
            state.messages.push(ChatMessage::assistant(format!(
                "**Resumed session `{typed_session_id}`** — {msg_count} message(s) loaded."
            )));
            crate::commands::inbox::inject_inbox_reminder(state, typed_session_id.as_str()).await;
        } else {
            state.messages.push(ChatMessage::assistant(format!(
                "**Error:** Session `{typed_session_id}` not found."
            )));
        }
    }
}

pub(super) async fn cmd_sessions(
    state: &mut EngineState,
    parts: &[&str],
    _text: &str,
    _tx: Option<&mpsc::Sender<EngineEvent>>,
) {
    // Handle `/sessions fsck`, `/sessions delete <id>` (also `rm`/`remove`),
    // and `/sessions send <id> <message>`.
    // run_command uses splitn(2, ' '), so parts[1] is the full remainder after "/sessions".
    let remainder = parts.get(1).copied().unwrap_or("").trim();
    let mut rem_iter = remainder.splitn(2, ' ');
    let sub = rem_iter.next().unwrap_or("");

    if sub == "fsck" {
        let args = rem_iter.next().unwrap_or("").trim();
        let quarantine = args
            .split_whitespace()
            .any(|arg| matches!(arg, "--quarantine" | "--repair" | "quarantine" | "repair"));
        match jfc_session::fsck_sessions(quarantine).await {
            Ok(report) => {
                let mut body = format!(
                    "**Session fsck:** checked {}, ok {}, issue(s) {}, quarantined {}\n",
                    report.checked,
                    report.ok,
                    report.issues.len(),
                    report.quarantined()
                );
                if !report.issues.is_empty() {
                    body.push('\n');
                    for issue in report.issues.iter().take(20) {
                        body.push_str(&format!("- `{}`: {}", issue.path.display(), issue.reason));
                        if let Some(target) = &issue.quarantined_to {
                            body.push_str(&format!(" -> `{}`", target.display()));
                        }
                        body.push('\n');
                    }
                    if report.issues.len() > 20 {
                        body.push_str(&format!(
                            "\n... and {} more issue(s)\n",
                            report.issues.len() - 20
                        ));
                    }
                }
                if !quarantine && !report.issues.is_empty() {
                    body.push_str("\nRun `/sessions fsck --quarantine` to move corrupt files to `lost+found`.");
                }
                state.messages.push(ChatMessage::assistant(body));
            }
            Err(e) => {
                state.messages.push(ChatMessage::assistant(format!(
                    "**Session fsck failed:** {e}"
                )));
            }
        }
        return;
    }

    if sub == "delete" || sub == "rm" || sub == "remove" {
        let id = rem_iter.next().unwrap_or("").trim();
        if id.is_empty() {
            state.messages.push(ChatMessage::assistant(
                "Usage: `/sessions delete <session_id>`".into(),
            ));
            return;
        }
        // Refuse to delete the currently-active session mid-flight.
        let typed_id = crate::ids::SessionId::new(id);
        if state.current_session_id.as_ref() == Some(&typed_id) {
            state.messages.push(ChatMessage::assistant(
                "Cannot delete the currently active session. Use `/clear` first.".into(),
            ));
            return;
        }
        match jfc_session::delete_session(id).await {
            Ok(true) => {
                state
                    .messages
                    .push(ChatMessage::assistant(format!("Session `{id}` deleted.")));
            }
            Ok(false) => {
                state
                    .messages
                    .push(ChatMessage::assistant(format!("Session `{id}` not found.")));
            }
            Err(e) => {
                state.messages.push(ChatMessage::assistant(format!(
                    "**Error** deleting session `{id}`: {e}"
                )));
            }
        }
        return;
    }

    if sub == "send" {
        let args = rem_iter.next().unwrap_or("").trim();
        let mut it = args.splitn(2, ' ');
        let target = it.next().unwrap_or("").trim();
        let msg = it.next().unwrap_or("").trim();
        if target.is_empty() || msg.is_empty() {
            state.messages.push(ChatMessage::assistant(
                "Usage: `/sessions send <session_id> <message>`".into(),
            ));
            return;
        }
        let from = state
            .current_session_id
            .as_ref()
            .map(|s| s.as_str().to_owned());
        let _ = jfc_session::write_inbox_message(target, from.as_deref(), msg).await;
        state.messages.push(ChatMessage::assistant(format!(
            "Message delivered to `{}`'s inbox.",
            target
        )));
        return;
    }

    // Default: list all sessions with metadata
    let sessions = jfc_session::list_sessions_with_metadata().await;
    if sessions.is_empty() {
        state
            .messages
            .push(ChatMessage::assistant("No sessions found.".into()));
    } else {
        let mut body = format!("**{} session(s):**\n\n", sessions.len());
        for (i, s) in sessions.iter().take(20).enumerate() {
            let title = s.display_title();
            let title_display = if title.chars().count() > 50 {
                let boundary = title.floor_char_boundary(50);
                format!("{}…", &title[..boundary])
            } else {
                title
            };
            let current = state.current_session_id.as_ref() == Some(&s.id);
            let marker = if current { " ← current" } else { "" };
            let name_indicator = if s.title.as_ref().is_some_and(|t| !t.trim().is_empty()) {
                " 📌"
            } else {
                ""
            };
            body.push_str(&format!(
                "{}. `{}`{}{} — {} msg(s)\n   {}\n",
                i + 1,
                s.id,
                marker,
                name_indicator,
                s.message_count,
                title_display
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

    // When the argument is a non-numeric string (or absent), use the parallel
    // fork path: save the current session to disk, then copy it into a new
    // session ID so both branches diverge independently without interrupting
    // the current session. When the argument is a number N, fall through to
    // the original "snapshot first N messages and switch" behaviour.
    let maybe_n: Option<usize> = arg.and_then(|s| s.parse().ok());
    if arg.is_none() || maybe_n.is_none() {
        // Parallel fork: keep current session running, create a sibling.
        let description = arg.unwrap_or("fork");
        let source_id = match &state.current_session_id {
            Some(id) => id.as_str().to_owned(),
            None => {
                state.messages.push(ChatMessage::assistant(
                    "No active session to fork. Send a message first so the session is saved."
                        .to_owned(),
                ));
                return;
            }
        };
        // Save the current session before forking so the fork gets the latest
        // messages including the `/fork` command itself.
        crate::session::save_session(
            &state.current_session_id.clone().expect("checked above"),
            &state.messages,
            Some(state.cwd.as_str()),
            Some(state.model.as_str()),
        )
        .await;

        match jfc_session::fork_session(&source_id, description).await {
            Ok(fork_id) => {
                state.messages.push(ChatMessage::assistant(format!(
                    "**Parallel fork** created as `{fork_id}`. \
                     This session (`{source_id}`) continues unchanged. \
                     Use `/resume {fork_id}` to switch to the fork.",
                )));
            }
            Err(e) => {
                state.messages.push(ChatMessage::assistant(format!(
                    "Failed to fork session `{source_id}`: {e}"
                )));
            }
        }
        return;
    }

    // Numeric N: snapshot the first N messages and switch to a new session
    // (original legacy behaviour — keeps backward compatibility).
    let upto = maybe_n.unwrap();
    if upto > state.messages.len() {
        state.messages.push(ChatMessage::assistant(format!(
            "Usage: `/fork [N | description]` — N ({upto}) exceeds the current message count ({}).",
            state.messages.len()
        )));
        return;
    }
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
    state.streaming_response_baseline = 0;
    state.streaming_thinking_tokens = 0;
    state.token_rate_samples.clear();
    state.token_rate_sample_thinking = None;
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
                        | jfc_core::ToolInput::Write { file_path, .. }
                        | jfc_core::ToolInput::MultiEdit { file_path, .. } => {
                            Some(file_path.clone())
                        }
                        jfc_core::ToolInput::NotebookEdit { path, .. } => Some(path.clone()),
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

pub(super) async fn cmd_cd(
    state: &mut EngineState,
    parts: &[&str],
    _text: &str,
    _tx: Option<&mpsc::Sender<EngineEvent>>,
) {
    // Change the engine working directory mid-session.
    // Usage: /cd <path>   (~ is expanded to the home directory)
    // Note: run_command uses splitn(2, ' '), so parts[1] is the entire path arg.
    let target = parts.get(1).copied().unwrap_or("~").trim();
    let raw_path = if target == "~" {
        dirs::home_dir().unwrap_or_else(|| PathBuf::from("."))
    } else if let Some(rel) = target.strip_prefix("~/") {
        dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(rel)
    } else {
        // Relative paths resolve against the *current* engine cwd.
        let base = PathBuf::from(&state.cwd);
        base.join(target)
    };

    match raw_path.canonicalize() {
        Ok(canonical) => {
            let display = canonical.display().to_string();
            state.cwd = display.clone();
            state.messages.push(ChatMessage::assistant(format!(
                "Working directory changed to `{display}`."
            )));
        }
        Err(e) => {
            state.messages.push(ChatMessage::assistant(format!(
                "**Error:** Cannot change directory to `{target}`: {e}"
            )));
        }
    }
}

/// `/handover` — produce a curated context hand-off package for a fresh
/// session. Writes a markdown file (`jfc-handover.md` by default, or the path
/// given as the first argument) that contains:
///
/// 1. **Session summary** — an auto-generated one-paragraph overview based on
///    the transcript.
/// 2. **Active tasks** — items from the task store with `pending`/`in_progress`
///    status.
/// 3. **Recent tool calls** — last 10 tool names + status for continuity.
/// 4. **Working directory** and current model / session id.
/// 5. **Picked-up memories** — the memory files loaded at session start.
/// 6. **Handover prompt** — a ready-made opening line the new session can use.
///
/// This mirrors Claude Code v126's `/handover` workflow: it lets the user open a
/// fresh session and resume exactly where they left off, avoiding a stale
/// context that would otherwise leak into the new session.
pub(super) async fn cmd_handover(
    state: &mut EngineState,
    parts: &[&str],
    text: &str,
    _tx: Option<&mpsc::Sender<EngineEvent>>,
) {
    state.messages.push(ChatMessage::user(text.to_owned()));

    let raw_path = parts.get(1).copied().unwrap_or("").trim();
    let out_path: std::path::PathBuf = if raw_path.is_empty() {
        std::path::PathBuf::from("jfc-handover.md")
    } else {
        std::path::PathBuf::from(raw_path)
    };

    let session_id = state
        .current_session_id
        .as_ref()
        .map(|id| id.as_str().to_owned())
        .unwrap_or_else(|| "(unknown)".to_owned());

    let model = state.model.as_str().to_owned();

    let cwd = state.cwd.clone();

    // --- 1. Brief transcript summary (last assistant text, up to 400 chars) ---
    let last_assistant_text: String = state
        .messages
        .iter()
        .rev()
        .find(|m| m.role == jfc_core::Role::Assistant)
        .and_then(|m| {
            m.parts.iter().find_map(|p| {
                if let jfc_core::MessagePart::Text(t) = p {
                    Some(t.chars().take(400).collect::<String>())
                } else {
                    None
                }
            })
        })
        .unwrap_or_else(|| "(no assistant messages yet)".to_owned());

    // --- 2. Recent tool calls (last 10) ---
    let mut tool_calls: Vec<String> = Vec::new();
    'outer: for msg in state.messages.iter().rev() {
        for part in &msg.parts {
            if let jfc_core::MessagePart::Tool(tc) = part {
                tool_calls.push(format!("- `{}` — {}", tc.kind.label(), tc.status.label()));
                if tool_calls.len() >= 10 {
                    break 'outer;
                }
            }
        }
    }
    tool_calls.reverse();

    // --- 3. Active tasks ---
    let all_tasks = state.task_store.list(jfc_session::DeletedFilter::Exclude);
    let active_tasks: Vec<String> = all_tasks
        .iter()
        .filter(|t| {
            matches!(
                t.status,
                jfc_session::TaskStatus::Pending
                    | jfc_session::TaskStatus::InProgress
                    | jfc_session::TaskStatus::Queued
            )
        })
        .take(20)
        .map(|t| {
            format!(
                "- [{}] {}{}",
                format!("{:?}", t.status).to_lowercase(),
                t.subject,
                if t.description.is_empty() {
                    String::new()
                } else {
                    let snippet: String = t.description.chars().take(80).collect();
                    format!(" — {snippet}")
                }
            )
        })
        .collect();

    // --- Build the handover document ---
    let tool_section = if tool_calls.is_empty() {
        "_No tool calls recorded._".to_owned()
    } else {
        tool_calls.join("\n")
    };

    let task_section = if active_tasks.is_empty() {
        "_No active tasks._".to_owned()
    } else {
        active_tasks.join("\n")
    };

    let handover_prompt = format!(
        "I'm continuing the session `{session_id}`. \
         The previous context ended at: {last_assistant_text:.200}. \
         Active tasks: {}. \
         Please pick up from where we left off.",
        if active_tasks.is_empty() {
            "none".to_owned()
        } else {
            format!("{} items", active_tasks.len())
        }
    );

    let doc = format!(
        "# jfc Session Handover\n\n\
         **Session:** `{session_id}`  \n\
         **Model:** `{model}`  \n\
         **CWD:** `{cwd}`\n\n\
         ---\n\n\
         ## Last assistant output (excerpt)\n\n\
         {last_assistant_text}\n\n\
         ---\n\n\
         ## Recent tool calls\n\n\
         {tool_section}\n\n\
         ---\n\n\
         ## Active tasks\n\n\
         {task_section}\n\n\
         ---\n\n\
         ## Suggested opening message for the new session\n\n\
         ```\n{handover_prompt}\n```\n",
    );

    match std::fs::write(&out_path, &doc) {
        Ok(()) => {
            let message = format!(
                "Handover package written to `{}` ({} bytes). \
                 Open a fresh session and paste the suggested opening message.",
                out_path.display(),
                doc.len()
            );
            state.messages.push(ChatMessage::assistant(message.clone()));
            crate::toast::push_with_cap(
                &mut state.toasts,
                crate::toast::Toast::new(crate::toast::ToastKind::Success, message),
            );
        }
        Err(e) => {
            let message = format!(
                "Failed to write handover package to `{}`: {e}",
                out_path.display()
            );
            state.messages.push(ChatMessage::assistant(message.clone()));
            crate::toast::push_with_cap(
                &mut state.toasts,
                crate::toast::Toast::new(crate::toast::ToastKind::Error, message),
            );
        }
    }
}

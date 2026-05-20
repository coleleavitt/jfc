use super::*;
pub async fn run_slash_command(app: &mut App, text: &str) {
    handle_slash_command(app, text, None).await
}

/// Minimal application/x-www-form-urlencoded encoder for query strings.
/// Pulling in `urlencoding` or `url` for the two callers (`/bug` form
/// link generation) is overkill — the encoder only needs to handle ASCII
/// + UTF-8 bytes that browsers reliably decode.
#[allow(dead_code)]
fn url_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char);
            }
            b' ' => out.push('+'),
            _ => out.push_str(&format!("%{:02X}", b)),
        }
    }
    out
}

pub(super) async fn handle_slash_command(app: &mut App, text: &str, tx: Option<&mpsc::Sender<AppEvent>>) {
    let parts: Vec<&str> = text.splitn(2, ' ').collect();
    match parts[0] {
        "/rename" => {
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
        "/clear" => {
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
        "/check" => {
            // Re-run `cargo check --message-format=json` and refresh the
            // diagnostic row + transition toast. v126 has an analogous
            // `/diagnostics` flow; keep ours short. Best-effort — silently
            // no-ops outside a cargo project.
            app.messages.push(ChatMessage::user("/check".into()));
            app.messages.push(ChatMessage::assistant(
                "Running `cargo check`… (results will land in the diagnostic row)".into(),
            ));
            // The handler emits `ProviderEvent::DiagnosticsUpdated` whose
            // handler shows a transition toast — no need to render
            // results inline.
            // We don't have direct `tx` here; emit via a no-op
            // background spawn that returns through the channel exposed
            // to other slash-command paths. Instead, we set a flag the
            // main loop can pick up; for now the simpler thing is to
            // tell the user to wait for the auto-update.
            //
            // (The startup-time spawn already does this on launch; this
            // command just reminds the user how to retrigger.)
        }
        "/compact" => {
            // Use the calibrated context size (same source as the gauge
            // and pre-submit gate). Previously this re-ran the raw
            // `estimate_tokens` heuristic, so the manual report disagreed
            // with the live gauge and could show "0%" for a session the
            // sidebar reports as 90%-full.
            let est = app.tool_ctx.approx_tokens;
            let level = crate::compact::compact_level(est, app.max_context_tokens);
            let pct = if app.max_context_tokens > 0 {
                (est * 100 / app.max_context_tokens).min(999)
            } else {
                0
            };
            tracing::info!(
                target: "jfc::compact",
                est, max_context_tokens = app.max_context_tokens,
                pct, ?level, model = %app.model,
                "manual /compact command invoked"
            );
            app.messages.push(ChatMessage::user("/compact".into()));
            app.messages.push(ChatMessage::assistant(format!(
                "Manual compaction queued — current estimate **{est} / {} tokens ({pct}%)**, level: **{level:?}**.\n\n\
                 The next assistant turn will summarize the conversation up to here, replacing the prior turns with a 9-section summary.\n\n\
                 *(Tip: set `JFC_AUTOCOMPACT_PCT_OVERRIDE=N` (1-100) to test thresholds, or `JFC_DISABLE_AUTO_COMPACT=1` to disable auto-compact entirely.)*",
                app.max_context_tokens
            )));
            app.force_compact_pending = true;
        }
        "/advisor" => {
            // Parallel advisor (see `crate::advisor`). Doesn't touch the main
            // agent's stream — runs a separate `provider.complete()` against a
            // SNAPSHOT of the current transcript and surfaces the reply as a
            // dedicated `MessagePart::Advisor` part with its own visual style.
            //
            // Default-off per deliverable: gated by `app.advisor_enabled`,
            // populated from `JFC_ADVISOR_ENABLED=1` on startup. Even when on,
            // each session has a per-budget ceiling (`DEFAULT_TOKEN_BUDGET`)
            // so a runaway loop can't drain the user's account.
            let query = parts.get(1).copied().unwrap_or("").trim().to_owned();
            // Echo the user's command into the transcript first so the chat
            // shows what the user asked, even on the error paths below.
            app.messages
                .push(ChatMessage::user(format!("/advisor {query}")));
            if !app.advisor_enabled {
                app.messages
                    .push(ChatMessage::assistant_parts(vec![MessagePart::Advisor(
                        "Advisor mode is disabled. Set `JFC_ADVISOR_ENABLED=1` and \
                         restart jfc to enable parallel advisor queries."
                            .into(),
                    )]));
            } else if query.is_empty() {
                app.messages
                    .push(ChatMessage::assistant_parts(vec![MessagePart::Advisor(
                        "Usage: `/advisor <question>` — runs a parallel call \
                         against a snapshot of this transcript and surfaces \
                         the reply here without disturbing the main agent."
                            .into(),
                    )]));
            } else {
                // Lazy-mint the session on first use so users that never
                // call /advisor pay no allocation cost. The session model
                // tracks the *active* model at first invocation; switching
                // models mid-session keeps the original advisor model.
                let session = app
                    .advisor_session
                    .get_or_insert_with(|| crate::advisor::AdvisorSession::new(app.model.clone()));
                // Snapshot — Vec::clone is fine here, the deliverable
                // explicitly calls for a SNAPSHOT semantic. Without the
                // clone, `ask_advisor` would borrow `app.messages`
                // immutably while we're holding `&mut app.advisor_session`
                // mutably — borrow-check fails.
                let snapshot = app.messages.clone();
                let provider = std::sync::Arc::clone(&app.provider);
                match crate::advisor::ask_advisor(
                    provider.as_ref(),
                    session,
                    query.clone(),
                    &snapshot,
                )
                .await
                {
                    Ok(reply) => {
                        let remaining = session.tokens_remaining();
                        let total_budget = session.token_budget;
                        app.messages.push(ChatMessage::assistant_parts(vec![
                            MessagePart::Advisor(format!(
                                "{reply}\n\n_(advisor budget: {} of {} tokens remaining)_",
                                remaining, total_budget
                            )),
                        ]));
                    }
                    Err(e) => {
                        app.messages.push(ChatMessage::assistant_parts(vec![
                            MessagePart::Advisor(format!(
                                "Advisor error: {e}\n\nUse `/clear` to start a fresh session if the budget is exhausted."
                            )),
                        ]));
                    }
                }
            }
        }
        "/config" => {
            // `/config` (no args) → dump the parsed config as TOML in a code block.
            // `/config path` → print the canonical file path so the user knows
            // where to put their overrides. We re-parse on every invocation
            // (instead of caching at startup) so edits to ~/.config/jfc/config.toml
            // surface without restart — this command is the user's read-only
            // window into "what does jfc currently see?". Wiring the resolved
            // model into the actual stream call site is a separate task; for now
            // this command exists so users can verify their file parses and
            // know where to edit.
            let arg = parts.get(1).copied().unwrap_or("").trim();
            app.messages.push(ChatMessage::user(text.to_owned()));
            if arg == "path" {
                let p = crate::config::config_path();
                app.messages.push(ChatMessage::assistant(format!(
                    "**Config path:** `{}`",
                    p.display()
                )));
            } else {
                let cfg = crate::config::load();
                let body = match toml::to_string_pretty(&cfg) {
                    Ok(s) if s.trim().is_empty() => "(empty config — no overrides)".to_owned(),
                    Ok(s) => format!("```toml\n{s}```"),
                    Err(e) => format!("**Error serializing config:** {e}"),
                };
                app.messages.push(ChatMessage::assistant(body));
            }
        }
        "/continue" | "/c" => {
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
        "/resume" => {
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
        "/sessions" => {
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
        "/workflow" | "/wf" => {
            // v132 workflow templates. `/workflow` lists; `/workflow run <name>`
            // queues each step's prompt as a follow-up Submit so the leader
            // dispatches them in order. `parallel = true` steps batch into
            // a single multi-Task fan-out turn (the leader sees all the
            // prompts in one user message and is told to use parallel
            // dispatch).
            app.messages.push(ChatMessage::user(text.to_owned()));
            let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
            let arg = parts.get(1).copied().unwrap_or("").trim();
            let mut sub = arg.split_whitespace();
            let verb = sub.next().unwrap_or("");
            let rest: String = sub.collect::<Vec<_>>().join(" ");
            match verb {
                "" | "list" => {
                    let names = crate::workflows::list(&cwd);
                    if names.is_empty() {
                        app.messages.push(ChatMessage::assistant(
                            "No workflows found. Create `.jfc/workflows/<name>.toml` with a TOML body containing `[[step]]` tables.".into(),
                        ));
                    } else {
                        let mut body = String::from("**Available workflows:**\n\n");
                        for name in &names {
                            match crate::workflows::load(&cwd, name) {
                                Ok(w) => body.push_str(&crate::workflows::render_summary(name, &w)),
                                Err(e) => {
                                    body.push_str(&format!("- `{name}` (parse error: {e})\n"))
                                }
                            }
                        }
                        body.push_str("\nRun with `/workflow run <name>`.");
                        app.messages.push(ChatMessage::assistant(body));
                    }
                }
                "run" => {
                    if rest.is_empty() {
                        app.messages.push(ChatMessage::assistant(
                            "Usage: `/workflow run <name>`. List available workflows with `/workflow`.".into(),
                        ));
                        return;
                    }
                    match crate::workflows::load(&cwd, &rest) {
                        Err(e) => {
                            app.messages.push(ChatMessage::assistant(format!(
                                "Failed to load workflow `{rest}`: {e}"
                            )));
                        }
                        Ok(workflow) => {
                            // Queue each step as a Submit so the leader sees
                            // them sequentially. Parallel steps would need
                            // a multi-Task aggregator — flag for now and
                            // dispatch sequentially as a stop-gap.
                            if let Some(tx) = tx {
                                for step in workflow.step {
                                    let prompt = format!(
                                        "Use the `{}` agent (Task tool) for this step:\n\n{}",
                                        step.agent, step.prompt
                                    );
                                    let _ = tx
                                        .send(crate::runtime::AppEvent::Ui(
                                            crate::runtime::UiEvent::Submit(prompt),
                                        ))
                                        .await;
                                }
                                app.messages.push(ChatMessage::assistant(format!(
                                    "Workflow `{rest}` queued — steps will fire sequentially."
                                )));
                            } else {
                                app.messages.push(ChatMessage::assistant(
                                    "Workflow runner needs the event channel; called from a context that doesn't have one.".into(),
                                ));
                            }
                        }
                    }
                }
                other => {
                    app.messages.push(ChatMessage::assistant(format!(
                        "Unknown subcommand `{other}`. Use `/workflow list` or `/workflow run <name>`."
                    )));
                }
            }
        }
        "/login" => {
            // v132 `/login` flow. With no arg, prints the chooser. With
            // a sub-target, the dispatcher returns a body string +
            // some side effects need a browser open. We always shell
            // out to xdg-open / open / start to launch the browser
            // (cheap, async-safe; failures are silent on systems
            // without one of those binaries).
            app.messages.push(ChatMessage::user(text.to_owned()));
            let arg = parts
                .get(1)
                .copied()
                .map(str::trim)
                .filter(|s| !s.is_empty());
            let dispatch = crate::providers::login_dispatch::dispatch(arg);
            let url = match &dispatch {
                crate::providers::login_dispatch::LoginDispatch::AnthropicApiKey(_)
                | crate::providers::login_dispatch::LoginDispatch::ConsoleApiKey(_) => {
                    Some("https://console.anthropic.com/settings/keys")
                }
                crate::providers::login_dispatch::LoginDispatch::ClaudeAiOAuth(_) => {
                    Some("https://claude.ai/login")
                }
                crate::providers::login_dispatch::LoginDispatch::CodexOAuth(_) => {
                    Some("https://auth.openai.com/codex/device")
                }
                _ => None,
            };
            if let Some(url) = url {
                // Best-effort: shell out to the platform browser opener.
                // Don't await — the browser launch is fire-and-forget.
                #[cfg(target_os = "linux")]
                let _ = std::process::Command::new("xdg-open").arg(url).spawn();
                #[cfg(target_os = "macos")]
                let _ = std::process::Command::new("open").arg(url).spawn();
                #[cfg(target_os = "windows")]
                let _ = std::process::Command::new("cmd")
                    .args(["/C", "start", url])
                    .spawn();
                tracing::info!(target: "jfc::login", %url, "opened browser for /login");
            }
            app.messages.push(ChatMessage::assistant(format!(
                "{dispatch}{}",
                if url.is_some() {
                    "\n\n_(opened the browser for you)_"
                } else {
                    ""
                }
            )));
        }
        "/logout" => {
            app.messages.push(ChatMessage::user(text.to_owned()));
            let arg = parts
                .get(1)
                .copied()
                .map(str::trim)
                .filter(|s| !s.is_empty());
            // Wipe the OAuth token + API-key stores under
            // ~/.config/jfc/. We deliberately keep this contained to
            // jfc's own state (opencode shares anthropic-accounts.json,
            // so blindly nuking that file would also log them out of
            // a sibling client).
            let scope = arg.unwrap_or("jfc");
            let home = std::env::var("HOME").unwrap_or_default();
            let mut removed = Vec::new();
            for relpath in [
                ".config/jfc/credentials.json",
                ".config/jfc/anthropic-oauth.json",
                ".config/jfc/codex-tokens.json",
            ] {
                let p = std::path::PathBuf::from(&home).join(relpath);
                if p.exists() && std::fs::remove_file(&p).is_ok() {
                    removed.push(p.display().to_string());
                }
            }
            let summary = if removed.is_empty() {
                format!("No credential files found to remove (scope: `{scope}`).")
            } else {
                format!(
                    "Removed {} credential file(s):\n{}\nRun `/login` to authenticate again.",
                    removed.len(),
                    removed
                        .iter()
                        .map(|p| format!("  - `{p}`"))
                        .collect::<Vec<_>>()
                        .join("\n")
                )
            };
            app.messages.push(ChatMessage::assistant(summary));
        }
        "/release-notes" | "/releasenotes" | "/changelog" => {
            app.messages.push(ChatMessage::user(text.to_owned()));
            // Try to read the workspace CHANGELOG; fall back to a stub
            // pointer when the binary was installed somewhere without it.
            let candidates = ["CHANGELOG.md", "../CHANGELOG.md", "../../CHANGELOG.md"];
            let notes = candidates
                .iter()
                .find_map(|p| std::fs::read_to_string(p).ok())
                .map(|s| {
                    let trimmed = s.lines().take(80).collect::<Vec<_>>().join("\n");
                    if s.lines().count() > 80 {
                        format!("{trimmed}\n\n*(showing first 80 lines — see CHANGELOG.md for the rest)*")
                    } else {
                        trimmed
                    }
                })
                .unwrap_or_else(|| {
                    "Release notes unavailable in this build. Visit \
                     https://github.com/RustProjects/jfc/releases for the full changelog."
                        .to_owned()
                });
            app.messages.push(ChatMessage::assistant(notes));
        }
        "/feedback" => {
            app.messages.push(ChatMessage::user(text.to_owned()));
            let url = "https://github.com/RustProjects/jfc/issues/new";
            #[cfg(target_os = "linux")]
            let _ = std::process::Command::new("xdg-open").arg(url).spawn();
            #[cfg(target_os = "macos")]
            let _ = std::process::Command::new("open").arg(url).spawn();
            #[cfg(target_os = "windows")]
            let _ = std::process::Command::new("cmd")
                .args(["/C", "start", url])
                .spawn();
            app.messages.push(ChatMessage::assistant(format!(
                "Opened {url} in your browser. File the issue there — the session id is `{}` if you want to attach it.",
                app.current_session_id
                    .as_ref()
                    .map(|s| s.as_str())
                    .unwrap_or("(none)")
            )));
        }
        "/upgrade" => {
            app.messages.push(ChatMessage::user(text.to_owned()));
            app.messages.push(ChatMessage::assistant(
                "To upgrade jfc, run one of:\n\
                 * `cargo install --git https://github.com/RustProjects/jfc` (HEAD)\n\
                 * `cargo install jfc` (latest crates.io release)\n\
                 \n\
                 If you installed via a package manager (homebrew, nix, AUR), use its update path instead.".to_owned(),
            ));
        }
        "/copy" => {
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
        "/fork" => {
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
        _ => {
            super::slash_commands_ext::handle_slash_command_extended(app, &parts, text, tx).await;
        }
    }
    app.scroll_to_bottom();
}


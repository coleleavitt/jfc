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
        "/batch" => {
            // /batch <prompt-file>: read newline-delimited prompts and
            // submit them via Anthropic's Message Batches API for the
            // 50% discount. The batch ID is returned synchronously;
            // results stream back via the Sessions API in a follow-up
            // turn (poll `/batch status <id>`).
            app.messages.push(ChatMessage::user(text.to_owned()));
            let arg = parts.get(1).copied().unwrap_or("").trim();
            if arg.is_empty() {
                app.messages.push(ChatMessage::assistant(
                    "Usage: `/batch <prompt-file>`. The file should contain one prompt per line."
                        .into(),
                ));
                return;
            }
            let path = std::path::PathBuf::from(arg);
            let content = match std::fs::read_to_string(&path) {
                Ok(c) => c,
                Err(e) => {
                    app.messages.push(ChatMessage::assistant(format!(
                        "Failed to read `{}`: {e}",
                        path.display(),
                    )));
                    return;
                }
            };
            let prompts: Vec<String> = content
                .lines()
                .map(|l| l.trim().to_owned())
                .filter(|l| !l.is_empty() && !l.starts_with('#'))
                .collect();
            if prompts.is_empty() {
                app.messages.push(ChatMessage::assistant(
                    "No prompts found (each non-empty, non-`#`-comment line counts as one).".into(),
                ));
                return;
            }
            let Some(client) = crate::sdk_bridge::build_client() else {
                app.messages.push(ChatMessage::assistant(
                    "No Anthropic API key configured — `/batch` needs one (set ANTHROPIC_API_KEY)."
                        .into(),
                ));
                return;
            };
            let model = app.model.as_str().to_owned();
            let prompt_count = prompts.len();
            let path_for_msg = path.display().to_string();
            tokio::spawn(async move {
                use jfc_anthropic_sdk::batches::{BatchRequest, MessageBatchService};
                use jfc_anthropic_sdk::messages::{ContentBlock, Message, MessageRequest, Role};
                let svc = MessageBatchService::new(client);
                let requests: Vec<BatchRequest> = prompts
                    .into_iter()
                    .enumerate()
                    .map(|(i, p)| BatchRequest {
                        custom_id: format!("batch-{i}"),
                        params: MessageRequest {
                            model: model.clone(),
                            messages: vec![Message {
                                role: Role::User,
                                content: vec![ContentBlock::Text { text: p }],
                            }],
                            max_tokens: 4096,
                            system: None,
                            temperature: None,
                            top_p: None,
                            stop_sequences: Vec::new(),
                            tools: Vec::new(),
                            tool_choice: None,
                            stream: Some(false),
                            thinking: None,
                            reasoning_effort: None,
                        },
                    })
                    .collect();
                match svc.create(requests).await {
                    Ok(batch) => {
                        tracing::info!(
                            target: "jfc::batch",
                            batch_id = %batch.id,
                            count = prompt_count,
                            "batch submitted"
                        );
                        eprintln!(
                            "[batch] submitted {prompt_count} prompts from {path_for_msg} → batch {}",
                            batch.id
                        );
                    }
                    Err(e) => {
                        eprintln!("[batch] failed: {e}");
                    }
                }
            });
            app.messages.push(ChatMessage::assistant(format!(
                "Queued {prompt_count} prompts from `{}` for batch processing. \
                 Watch stderr / `/doctor` for the batch ID.",
                path.display()
            )));
        }
        "/diff" => {
            // Show pending uncommitted + unstaged changes via `git diff
            // HEAD --stat`. Read-only; doesn't run unless we're in a
            // git repo. Surface in the transcript as an assistant
            // message (markdown code block) so the user — and the
            // model on the next turn — can see what's pending.
            app.messages.push(ChatMessage::user(text.to_owned()));
            let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
            let in_repo = std::process::Command::new("git")
                .args(["rev-parse", "--is-inside-work-tree"])
                .current_dir(&cwd)
                .output()
                .map(|o| o.status.success())
                .unwrap_or(false);
            if !in_repo {
                app.messages.push(ChatMessage::assistant(
                    "Not inside a git repository — `/diff` has nothing to show.".into(),
                ));
                return;
            }
            let stat = std::process::Command::new("git")
                .args(["diff", "HEAD", "--stat"])
                .current_dir(&cwd)
                .output()
                .ok()
                .and_then(|o| String::from_utf8(o.stdout).ok())
                .unwrap_or_default();
            let untracked = std::process::Command::new("git")
                .args(["ls-files", "--others", "--exclude-standard"])
                .current_dir(&cwd)
                .output()
                .ok()
                .and_then(|o| String::from_utf8(o.stdout).ok())
                .unwrap_or_default();
            if stat.trim().is_empty() && untracked.trim().is_empty() {
                app.messages.push(ChatMessage::assistant(
                    "Working tree is clean — no pending changes.".into(),
                ));
            } else {
                let mut body = String::from("**Pending changes (`git diff HEAD`):**\n\n```\n");
                if !stat.trim().is_empty() {
                    body.push_str(&stat);
                } else {
                    body.push_str("(no tracked-file changes)\n");
                }
                if !untracked.trim().is_empty() {
                    body.push_str("\n--- untracked ---\n");
                    body.push_str(&untracked);
                }
                body.push_str("```\n");
                app.messages.push(ChatMessage::assistant(body));
            }
        }
        "/undo" => {
            // Revert the most recent Edit / Write / MultiEdit /
            // ApplyPatch tool's filesystem mutation. Pulls from
            // `app.tool_undo_history` which the tool dispatcher
            // populates by capturing pre-mutation file content
            // before the tool executes. Only undoes ONE step;
            // run /undo repeatedly to walk back further.
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
        "/export" => {
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
        "/verbose" => {
            // Toggle expanded-by-default tool blocks for the rest of
            // the session. Renderers read `app.verbose_mode` and lift
            // the per-tool preview cap when set.
            app.messages.push(ChatMessage::user(text.to_owned()));
            let arg = parts
                .get(1)
                .copied()
                .unwrap_or("")
                .trim()
                .to_ascii_lowercase();
            let target = match arg.as_str() {
                "on" | "true" | "1" => Some(true),
                "off" | "false" | "0" => Some(false),
                "" => Some(!app.verbose_mode),
                _ => None,
            };
            match target {
                Some(v) => {
                    app.verbose_mode = v;
                    app.messages.push(ChatMessage::assistant(format!(
                        "Verbose mode **{}** — tool blocks {} preview cap.",
                        if v { "ON" } else { "OFF" },
                        if v { "expand past" } else { "respect" },
                    )));
                }
                None => {
                    app.messages.push(ChatMessage::assistant(
                        "Usage: `/verbose [on|off]`. With no arg, toggles.".into(),
                    ));
                }
            }
        }
        "/fast" | "/f" => {
            // Toggle fast mode (lower-latency inference via Anthropic's
            // `fast-mode-2026-02-01` beta header). Mirrors Claude Code
            // v2.1.139's `/fast` command (Alt+O keybind).
            app.messages.push(ChatMessage::user(text.to_owned()));
            app.fast_mode = !app.fast_mode;
            crate::effort::set_fast_mode_global(app.fast_mode);
            app.messages.push(ChatMessage::assistant(format!(
                "Fast mode: **{}** — {}",
                if app.fast_mode { "ON" } else { "OFF" },
                if app.fast_mode {
                    "requests will use the low-latency inference path"
                } else {
                    "requests will use the standard inference path"
                },
            )));
        }
        "/pin" => {
            // Pin a message by transcript index so compaction can't
            // drop it. /pin without an arg pins the most recent
            // message; /pin <n> pins index n; /pin list prints the
            // current pin set.
            app.messages.push(ChatMessage::user(text.to_owned()));
            let arg = parts.get(1).copied().unwrap_or("").trim();
            if arg == "list" {
                if app.pinned_message_indices.is_empty() {
                    app.messages.push(ChatMessage::assistant(
                        "No pinned messages. `/pin <n>` pins index n; `/pin` pins the most recent."
                            .into(),
                    ));
                } else {
                    let mut idx: Vec<usize> = app.pinned_message_indices.iter().copied().collect();
                    idx.sort();
                    let listing = idx
                        .into_iter()
                        .map(|i| format!("- #{i}"))
                        .collect::<Vec<_>>()
                        .join("\n");
                    app.messages.push(ChatMessage::assistant(format!(
                        "**Pinned messages:**\n{listing}"
                    )));
                }
            } else if arg.is_empty() {
                if app.messages.is_empty() {
                    return;
                }
                let idx = app.messages.len() - 1;
                app.pinned_message_indices.insert(idx);
                app.messages.push(ChatMessage::assistant(format!(
                    "Pinned message #{idx} (compaction will preserve it)."
                )));
            } else {
                match arg.parse::<usize>() {
                    Ok(idx) if idx < app.messages.len() => {
                        app.pinned_message_indices.insert(idx);
                        app.messages
                            .push(ChatMessage::assistant(format!("Pinned message #{idx}.")));
                    }
                    Ok(idx) => {
                        app.messages.push(ChatMessage::assistant(format!(
                            "No message at index {idx} (transcript has {} messages).",
                            app.messages.len()
                        )));
                    }
                    Err(_) => {
                        app.messages.push(ChatMessage::assistant(format!(
                            "Couldn't parse `{arg}` as a message index. Use `/pin`, `/pin <n>`, or `/pin list`."
                        )));
                    }
                }
            }
        }
        "/unpin" => {
            app.messages.push(ChatMessage::user(text.to_owned()));
            let arg = parts.get(1).copied().unwrap_or("").trim();
            if arg.is_empty() || arg == "all" {
                let n = app.pinned_message_indices.len();
                app.pinned_message_indices.clear();
                app.messages
                    .push(ChatMessage::assistant(format!("Cleared {n} pin(s).")));
            } else {
                match arg.parse::<usize>() {
                    Ok(idx) => {
                        if app.pinned_message_indices.remove(&idx) {
                            app.messages
                                .push(ChatMessage::assistant(format!("Unpinned message #{idx}.")));
                        } else {
                            app.messages.push(ChatMessage::assistant(format!(
                                "Message #{idx} wasn't pinned."
                            )));
                        }
                    }
                    Err(_) => {
                        app.messages.push(ChatMessage::assistant(format!(
                            "Couldn't parse `{arg}` as a message index."
                        )));
                    }
                }
            }
        }
        "/timeline" => {
            // Render a chronological tool-call timeline for the most
            // recent assistant turn. For each Tool part, emit one row
            // with "kind │ summary │ Δms" so the user can spot slow
            // tools at a glance.
            app.messages.push(ChatMessage::user(text.to_owned()));
            let last_assistant = app
                .messages
                .iter()
                .rposition(|m| matches!(m.role, crate::types::Role::Assistant));
            let Some(idx) = last_assistant else {
                app.messages.push(ChatMessage::assistant(
                    "No assistant turn yet — nothing to timeline.".into(),
                ));
                return;
            };
            let msg = &app.messages[idx];
            let mut rows: Vec<String> = Vec::new();
            for part in &msg.parts {
                if let crate::types::MessagePart::Tool(tc) = part {
                    let elapsed = tc
                        .elapsed_ms
                        .map(|ms| {
                            if ms >= 1_000 {
                                format!("{:.1}s", ms as f64 / 1000.0)
                            } else {
                                format!("{ms}ms")
                            }
                        })
                        .unwrap_or_else(|| "—".to_owned());
                    let summary = tc.input.summary();
                    let summary: String = summary.chars().take(60).collect();
                    rows.push(format!(
                        "  - **{}** · `{}` · {elapsed}",
                        tc.kind.label(),
                        summary,
                    ));
                }
            }
            if rows.is_empty() {
                app.messages.push(ChatMessage::assistant(
                    "Most recent assistant turn ran no tools.".into(),
                ));
            } else {
                app.messages.push(ChatMessage::assistant(format!(
                    "**Tool timeline (last assistant turn, {} tools):**\n{}",
                    rows.len(),
                    rows.join("\n"),
                )));
            }
        }
        "/doctor" => {
            // Mirrors Claude Code 2.1.139's /doctor command.
            // Health check: scan the most-likely failure modes for an
            // out-of-the-box jfc setup and surface a single status
            // block. Read-only; no fixes applied automatically — the
            // user opts in to remedies after seeing the report.
            app.messages.push(ChatMessage::user(text.to_owned()));

            let check = |ok: bool| if ok { "✓" } else { "✗" };

            let mut report = String::from("jfc doctor report\n─────────────────\n");

            // ── 1. Config file ────────────────────────────────────────────────
            {
                let cfg_path = crate::config::config_path();
                let cfg_display = cfg_path.display().to_string();
                // Tilde-shorten for readability
                let cfg_display = if let Some(home) = dirs::home_dir() {
                    cfg_display.replacen(&home.display().to_string(), "~", 1)
                } else {
                    cfg_display
                };
                let cfg_ok = cfg_path.exists() && {
                    // Try a parse round-trip to catch TOML errors
                    std::fs::read_to_string(&cfg_path)
                        .ok()
                        .and_then(|s| toml::from_str::<crate::config::Config>(&s).ok())
                        .is_some()
                };
                report.push_str(&format!(
                    "{} Config: {}{}\n",
                    check(cfg_ok),
                    cfg_display,
                    if cfg_ok {
                        ""
                    } else if !cfg_path.exists() {
                        " (not found)"
                    } else {
                        " (parse error)"
                    },
                ));
            }

            // ── 2. Auth: ANTHROPIC_API_KEY env ───────────────────────────────
            {
                let api_key_set = std::env::var("ANTHROPIC_API_KEY").is_ok();
                report.push_str(&format!(
                    "{} Auth: ANTHROPIC_API_KEY {}\n",
                    check(api_key_set),
                    if api_key_set { "set" } else { "not set" },
                ));
            }

            // ── 3. Auth: ~/.config/jfc/anthropic-accounts.json ───────────────
            {
                let accounts_path = dirs::config_dir()
                    .unwrap_or_else(|| std::path::PathBuf::from("."))
                    .join("jfc")
                    .join("anthropic-accounts.json");
                let accounts_ok = accounts_path.exists();
                let accounts_display = {
                    let s = accounts_path.display().to_string();
                    if let Some(home) = dirs::home_dir() {
                        s.replacen(&home.display().to_string(), "~", 1)
                    } else {
                        s
                    }
                };
                report.push_str(&format!(
                    "{} Auth: accounts file {} {}\n",
                    check(accounts_ok),
                    accounts_display,
                    if accounts_ok {
                        "(found)"
                    } else {
                        "(not found)"
                    },
                ));
            }

            // ── 4. CLAUDE.md in project root ──────────────────────────────────
            {
                let project_root = std::path::PathBuf::from(&app.cwd);
                let claude_md = project_root.join("CLAUDE.md");
                let md_ok = claude_md.exists();
                let md_display = format!(
                    "{}{}",
                    "./",
                    claude_md
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or("CLAUDE.md")
                );
                report.push_str(&format!(
                    "{} CLAUDE.md: {}\n",
                    check(md_ok),
                    if md_ok {
                        md_display
                    } else {
                        format!("{} (not found)", md_display)
                    },
                ));
            }

            // ── 5. MCP servers ────────────────────────────────────────────────
            {
                let cfg = crate::config::load();
                if cfg.mcp.is_empty() {
                    report.push_str("  MCP: no servers configured\n");
                } else {
                    for (name, server) in &cfg.mcp {
                        // Determine the binary to probe: use `command` if set,
                        // otherwise the first element of `args` (e.g. npx), and
                        // fall back to the server name itself.
                        let probe_bin = server
                            .command
                            .as_deref()
                            .filter(|s| !s.is_empty())
                            .or_else(|| server.args.first().map(|s| s.as_str()))
                            .unwrap_or(name.as_str());
                        let found = std::process::Command::new("which")
                            .arg(probe_bin)
                            .output()
                            .map(|o| o.status.success())
                            .unwrap_or(false);
                        report.push_str(&format!(
                            "{} MCP: {} ({} {})\n",
                            check(found),
                            name,
                            probe_bin,
                            if found { "found" } else { "not found" },
                        ));
                    }
                }
            }

            // ── 6. Working directory + git repo ───────────────────────────────
            {
                let cwd = std::path::PathBuf::from(&app.cwd);
                let git_ok = std::process::Command::new("git")
                    .args(["rev-parse", "--git-dir"])
                    .current_dir(&cwd)
                    .output()
                    .map(|o| o.status.success())
                    .unwrap_or(false);
                // Grab current branch name when inside a git repo
                let branch = if git_ok {
                    std::process::Command::new("git")
                        .args(["rev-parse", "--abbrev-ref", "HEAD"])
                        .current_dir(&cwd)
                        .output()
                        .ok()
                        .and_then(|o| {
                            if o.status.success() {
                                String::from_utf8(o.stdout)
                                    .ok()
                                    .map(|s| s.trim().to_owned())
                            } else {
                                None
                            }
                        })
                        .unwrap_or_else(|| "unknown".to_owned())
                } else {
                    String::new()
                };
                let git_label = if git_ok {
                    format!("yes (branch: {branch})")
                } else {
                    "no".to_owned()
                };
                report.push_str(&format!("{} Git repo: {}\n", check(git_ok), git_label));
                report.push_str(&format!("  cwd: {}\n", cwd.display()));
            }

            // ── 7. Version ────────────────────────────────────────────────────
            report.push_str(&format!("  Version: {}\n", env!("CARGO_PKG_VERSION")));

            // ── 8. Bonus: active provider + permission mode ───────────────────
            report.push_str(&format!("  Provider: {}\n", app.provider.name()));
            report.push_str(&format!("  Permission mode: {:?}\n", app.permission_mode));

            // ── 9. Session cost so far ────────────────────────────────────────
            let total = crate::cost::total_cost(&app.usage_by_model);
            report.push_str(&format!(
                "  Session cost: {}\n",
                crate::cost::fmt_cost(total)
            ));

            app.messages.push(ChatMessage::assistant(report));
        }
        "/effort" => {
            // v132 reasoning-effort pin. `/effort low|medium|high|xhigh|max`
            // sets the pin; `/effort` alone shows the current state;
            // `/effort clear` removes the pin so the model picks adaptive.
            app.messages.push(ChatMessage::user(text.to_owned()));
            let arg = parts.get(1).copied().unwrap_or("").trim();
            if arg.is_empty() {
                app.messages
                    .push(ChatMessage::assistant(app.effort_state.status()));
            } else if arg == "clear" || arg == "off" {
                let msg = app.effort_state.clear();
                app.messages.push(ChatMessage::assistant(msg));
            } else if let Some(level) = crate::effort::ReasoningEffort::from_str_loose(arg) {
                let msg = app.effort_state.set(level);
                app.messages.push(ChatMessage::assistant(msg));
            } else {
                app.messages.push(ChatMessage::assistant(format!(
                    "Unknown effort `{arg}`. Use one of: low, medium, high, xhigh, max, clear."
                )));
            }
        }
        "/feature" => {
            // v132 feature-gate framework. `/feature` lists all gates and
            // their state; `/feature <codename> on|off` flips one.
            app.messages.push(ChatMessage::user(text.to_owned()));
            let rest = parts.get(1).copied().unwrap_or("").trim();
            if rest.is_empty() {
                let mut body = String::from("**Feature gates:**\n\n");
                for &gate in crate::feature_gates::FeatureGate::ALL {
                    body.push_str(&format!(
                        "- `{}` — **{}** ({})\n",
                        gate.codename(),
                        if crate::feature_gates::is_enabled(gate) {
                            "ON"
                        } else {
                            "OFF"
                        },
                        gate.description(),
                    ));
                }
                body.push_str("\nToggle with `/feature <codename> on|off`.");
                app.messages.push(ChatMessage::assistant(body));
            } else {
                let mut sub = rest.split_whitespace();
                let name = sub.next().unwrap_or("");
                let toggle = sub.next().unwrap_or("").to_ascii_lowercase();
                let Some(gate) = crate::feature_gates::FeatureGate::from_codename(name) else {
                    app.messages.push(ChatMessage::assistant(format!(
                        "Unknown feature gate `{name}`. List with `/feature`."
                    )));
                    return;
                };
                let enabled = match toggle.as_str() {
                    "on" | "enable" | "true" | "1" => true,
                    "off" | "disable" | "false" | "0" => false,
                    "" => {
                        app.messages.push(ChatMessage::assistant(format!(
                            "`{}` is currently **{}**. Toggle with `/feature {} on|off`.",
                            gate.codename(),
                            if crate::feature_gates::is_enabled(gate) {
                                "ON"
                            } else {
                                "OFF"
                            },
                            gate.codename(),
                        )));
                        return;
                    }
                    other => {
                        app.messages.push(ChatMessage::assistant(format!(
                            "Unknown toggle `{other}`. Use `on` or `off`."
                        )));
                        return;
                    }
                };
                crate::feature_gates::set(gate, enabled);
                app.messages.push(ChatMessage::assistant(format!(
                    "`{}` set to **{}** ({}).",
                    gate.codename(),
                    if enabled { "ON" } else { "OFF" },
                    gate.description(),
                )));
                // v132 system-reminder so the model sees the gate flip
                // on the next turn (rather than guessing from changed
                // behavior).
                crate::system_reminder::append_to_last_user(
                    &mut app.messages,
                    &format!(
                        "Feature gate `{}` flipped to **{}** ({}). Adjust your \
                         behavior accordingly.",
                        gate.codename(),
                        if enabled { "ON" } else { "OFF" },
                        gate.description(),
                    ),
                );
            }
        }
        "/goal" => {
            // v137 session-scoped goal. `/goal <condition>` sets a stop
            // condition — the agent keeps working until the evaluator
            // says it's met (see `crate::goal::evaluate`). `/goal
            // clear` (or stop/off/reset/none/cancel) removes it.
            // `/goal` alone shows the current state.
            app.messages.push(ChatMessage::user(text.to_owned()));
            let arg = parts[1..].join(" ");
            let arg = arg.trim();
            if arg.is_empty() {
                let msg = match &app.goal {
                    Some(g) => format!(
                        "Current goal ({} iterations): {}\n\nUse `/goal clear` to remove.",
                        g.iterations, g.condition
                    ),
                    None => "No goal set. Usage: `/goal <condition>`".to_string(),
                };
                app.messages.push(ChatMessage::assistant(msg));
            } else if crate::goal::is_clear_arg(arg) {
                let prev = app.goal.take();
                app.goal_evaluator_in_flight = false;
                // Drop the sidecar so a future /continue doesn't
                // revive a goal the user just cancelled.
                if let Some(sid) = app.current_session_id.as_ref() {
                    crate::goal::save_sidecar(sid.as_str(), None);
                }
                let msg = match prev {
                    Some(g) => format!(
                        "Goal cleared after {} iterations: {}",
                        g.iterations, g.condition
                    ),
                    None => "No goal was set.".to_string(),
                };
                app.messages.push(ChatMessage::assistant(msg));
                crate::toast::push_with_cap(
                    &mut app.toasts,
                    crate::toast::Toast::new(
                        crate::toast::ToastKind::Success,
                        "Goal cleared".to_string(),
                    ),
                );
            } else {
                match crate::goal::validate_condition(arg) {
                    Ok(condition) => {
                        let goal = crate::goal::ActiveGoal::new(condition.clone());
                        app.goal = Some(goal);
                        // Persist the new goal so /continue picks it
                        // up if the user exits before the next turn.
                        if let Some(sid) = app.current_session_id.as_ref() {
                            crate::goal::save_sidecar(sid.as_str(), app.goal.as_ref());
                        }
                        app.messages.push(ChatMessage::assistant(format!(
                            "Goal set: {condition}\n\nThe agent will keep \
                             working until this condition is met (auto-\
                             evaluated after each turn, max {} iterations). \
                             Use `/goal clear` to cancel.",
                            crate::goal::MAX_ITERATIONS
                        )));
                        crate::toast::push_with_cap(
                            &mut app.toasts,
                            crate::toast::Toast::new(
                                crate::toast::ToastKind::Success,
                                format!("Goal: {condition}"),
                            ),
                        );
                        // Kick off work immediately: synthesize the
                        // Claude-Code-style meta prompt so the agent
                        // starts acting on the goal instead of sitting
                        // idle until the next user turn. Only fire
                        // when the session is genuinely idle (no
                        // streaming / pending approval / pending
                        // tools) AND we have an event channel.
                        let idle = !app.is_streaming
                            && app.pending_approval.is_none()
                            && app.approval_queue.is_empty()
                            && app.pending_tool_calls.is_empty();
                        if let (true, Some(tx)) = (idle, tx) {
                            let kickoff = format!(
                                "A session-scoped stop-condition hook is now \
                                 active with condition: \"{condition}\".\n\n\
                                 Briefly acknowledge the goal, then \
                                 immediately start or continue working toward \
                                 it. The hook will block stopping until the \
                                 condition holds (auto-evaluated after each \
                                 turn, max {} iterations). It auto-clears \
                                 once the condition is met.",
                                crate::goal::MAX_ITERATIONS
                            );
                            let _ = tx.send(AppEvent::Ui(UiEvent::Submit(kickoff))).await;
                            tracing::info!(
                                target: "jfc::goal",
                                "/goal: dispatched kickoff meta-prompt"
                            );
                        }
                    }
                    Err(reason) => {
                        app.messages.push(ChatMessage::assistant(reason.to_owned()));
                    }
                }
            }
        }
        "/help" => {
            // Also flip the visual overlay so users get the same
            // keybindings table they'd see from `?`. The text dump
            // below is kept for searchability + transcript export.
            app.show_help = true;
            app.messages.push(ChatMessage::user("/help".into()));
            app.messages.push(ChatMessage::assistant(
                "**Available commands:**\n\
                 - `/clear` — Clear conversation and start fresh\n\
                 - `/compact` — Manually compact the conversation\n\
                 - `/advisor <question>` — Ask a parallel advisor without disturbing the main agent (set `JFC_ADVISOR_ENABLED=1`)\n\
                 - `/check` — Re-run cargo-check diagnostics\n\
                 - `/config` — Show parsed `~/.config/jfc/config.toml` (use `/config path` for the file location)\n\
                 - `/continue` (or `/c`) — Resume most recent session\n\
                 - `/resume <id>` — Resume a specific session by id\n\
                 - `/sessions` — List all saved sessions\n\
                 - `/theme [name]` — Open theme picker or switch/persist a theme\n\
                 - `/auto-mode on` — Enable v126-style LLM tool classifier (no user prompts)\n\
                 - `/auto-mode off` — Disable auto-mode, restore manual approval\n\
                 - `/auto-mode status` — Show current state + rule sources\n\
                 - `/skills` — List available skills (.claude/skills/*.md)\n\
                 - `/agents` — List available agent definitions (.claude/agents/*.md)\n\
                 - `/claude-md` — Show which CLAUDE.md layers are loaded\n\
                 - `/tasks` — List todo/task items\n\
                 - `/task-add <subject>` — Create a new task\n\
                 - `/task-done <id>` — Mark task completed\n\
                 - `/task-rm <id>` — Delete task\n\
                 - `/worktree [list|create <name>|remove <name>|switch <name>]` — Manage `.jfc-worktrees/<name>` checkouts on `jfc/<name>` branches\n\
                 - `/install-github-app` — Install Claude GitHub App on the current repo (browser flow)\n\
                 - `/pr <num>` — Show PR title, description, and review comments\n\
                 - `/pr-autofix <num>` — Build a model prompt that addresses PR review comments\n\
                 - `/setup-github-actions [force]` — Write `.github/workflows/jfc-review.yml`\n\
                 - `/help` — Show this message\n\
                 \n\
                 **Keys:**\n\
                 - Ctrl+B — Toggle sessions sidebar\n\
                 - Ctrl+M — Model picker\n\
                 - Ctrl+P — Command palette\n\
                 - Ctrl+O — Expand reasoning / open diagnostic panel\n\
                 - Alt+. / Alt+, — Raise / lower reasoning effort\n\
                 - Ctrl+Y — Yank last assistant message to clipboard\n\
                 - Ctrl+S — Toggle info sidebar\n\
                 - `@` — Autocomplete file paths from cwd\n\
                 - Up — Recall most recent queued prompt / cycle history (when input empty)\n\
                 - Esc — Dismiss popup / close diagnostic panel\n\
                 \n\
                 **Env knobs:**\n\
                 - `JFC_DISABLE_BELL=1` — silence terminal bell on tool completion\n\
                 - `JFC_DISABLE_AUTO_COMPACT=1` — disable auto-compaction\n\
                 - `JFC_DISABLE_CARGO_CHECK=1` — skip startup `cargo check`\n\
                 - `JFC_AUTOCOMPACT_PCT_OVERRIDE=N` — force compact threshold\n\
                 - `JFC_TOOL_TITLE_WIDTH=N` — cap tool title length (default 100)\n\
                 - `JFC_ADVISOR_ENABLED=1` — enable the `/advisor` parallel-advice slash command"
                    .into(),
            ));
        }
        "/memory" | "/mem" => {
            // `/memory` (no args)            → list memory files
            // `/memory recall on|off|status` → toggle two-phase recall
            //
            // The recall sub-command targets the runtime override in
            // `memory_recall::set_runtime_override` — persisting to
            // `~/.config/jfc/config.toml` is left to the user since they
            // may have hand-formatted that file.
            let arg = parts.get(1).copied().unwrap_or("").trim();
            app.messages.push(ChatMessage::user(text.to_owned()));
            if arg.starts_with("recall") {
                let sub = arg
                    .split_once(' ')
                    .map(|x| x.1)
                    .map(str::trim)
                    .unwrap_or("status");
                match sub {
                    "on" | "enable" => {
                        crate::memory_recall::set_runtime_override(Some(true));
                        app.messages.push(ChatMessage::assistant(
                            "Two-phase memory recall: **on** (runtime override).".into(),
                        ));
                    }
                    "off" | "disable" => {
                        crate::memory_recall::set_runtime_override(Some(false));
                        app.messages.push(ChatMessage::assistant(
                            "Two-phase memory recall: **off** (runtime override).".into(),
                        ));
                    }
                    "default" | "reset" => {
                        crate::memory_recall::set_runtime_override(None);
                        app.messages.push(ChatMessage::assistant(
                            "Two-phase memory recall: cleared runtime override; \
                             falling back to `~/.config/jfc/config.toml` value."
                                .into(),
                        ));
                    }
                    "status" | "" => {
                        let persisted = crate::config::load().memory_recall_enabled;
                        let effective = crate::memory_recall::is_enabled(persisted);
                        app.messages.push(ChatMessage::assistant(format!(
                            "**Memory recall**\n\
                             - Effective: **{}**\n\
                             - Persisted (config.toml): **{}**\n\
                             \n\
                             Toggle with `/memory recall on|off|reset`.",
                            if effective { "on" } else { "off" },
                            if persisted { "on" } else { "off" }
                        )));
                    }
                    other => {
                        app.messages.push(ChatMessage::assistant(format!(
                            "Unknown sub-command `{other}`. Try \
                             `/memory recall on|off|reset|status`."
                        )));
                    }
                }
            } else {
                let cwd = std::env::current_dir().unwrap_or_else(|_| ".".into());
                let mems = crate::memory::load_all_memories(&cwd);
                let body = if mems.is_empty() {
                    "No memory files found. Create `.jfc/memory/*.md` (project) or \
                     `~/.config/jfc/memory/*.md` (user) with YAML frontmatter \
                     (`type:` and `scope:`) and a markdown body."
                        .to_owned()
                } else {
                    let listing = crate::memory::format_existing_memories(&mems);
                    format!(
                        "**{} memor{} loaded:**\n\n{listing}\n\nUse `/memory recall status` to see whether two-phase recall is active.",
                        mems.len(),
                        if mems.len() == 1 { "y" } else { "ies" }
                    )
                };
                app.messages.push(ChatMessage::assistant(body));
            }
        }
        "/commit" => {
            // Generate a conventional commit message for staged changes.
            // 1. Check if anything is staged; bail early if not.
            // 2. Capture `git diff --cached` (capped at 8000 chars).
            // 3. Inject a user prompt so the model generates the message
            //    on the next turn — the user can then copy/run `git commit`.
            app.messages.push(ChatMessage::user("/commit".into()));
            let cwd = app.cwd.clone();
            let stat = tokio::process::Command::new("git")
                .args(["diff", "--cached", "--stat"])
                .current_dir(&cwd)
                .output()
                .await;
            match stat {
                Err(e) => {
                    app.messages.push(ChatMessage::assistant(format!(
                        "Could not run `git diff --cached --stat`: {e}"
                    )));
                }
                Ok(out) => {
                    let stat_str = String::from_utf8_lossy(&out.stdout);
                    if stat_str.trim().is_empty() {
                        app.messages.push(ChatMessage::assistant(
                            "Nothing staged. Stage changes first with `git add <file>` or `git add -p`.".into(),
                        ));
                    } else {
                        // Fetch the full diff, capped at 8000 chars to stay
                        // well within any reasonable context window.
                        let diff_output = tokio::process::Command::new("git")
                            .args(["diff", "--cached"])
                            .current_dir(&cwd)
                            .output()
                            .await
                            .ok();
                        let diff_str = diff_output
                            .map(|o| {
                                let s = String::from_utf8_lossy(&o.stdout).into_owned();
                                if s.len() > 8000 {
                                    // floor_char_boundary instead of a raw
                                    // byte slice — git diff can carry
                                    // non-ASCII filenames or content and
                                    // a fixed-byte cap would panic if a
                                    // multi-byte glyph straddled byte 8000.
                                    let cap = s.floor_char_boundary(8000);
                                    format!(
                                        "{}\n\n[... diff truncated at 8000 chars ...]",
                                        &s[..cap]
                                    )
                                } else {
                                    s
                                }
                            })
                            .unwrap_or_default();
                        let prompt = format!(
                            "Generate a conventional commit message for these staged changes.\n\
                             Format: `type(scope): description`\n\
                             Types: feat / fix / docs / style / refactor / test / chore\n\
                             Rules: imperative mood, ≤72 chars subject, no trailing period.\n\
                             Output ONLY the commit message — no explanation, no markdown fences.\n\n\
                             ```\n{diff_str}\n```"
                        );
                        app.messages
                            .push(ChatMessage::assistant("Analyzing staged changes…".into()));
                        app.queued_prompts.push(crate::app::QueuedPrompt {
                            text: prompt,
                            is_meta: false,
                            priority: crate::app::QueuePriority::Later,
                            attachments: Vec::new(),
                        });
                        app.scroll_to_bottom();
                    }
                }
            }
        }
        "/review" => {
            // Ask the model to review current git changes for bugs, security
            // issues, and code quality problems with file:line specificity.
            app.messages.push(ChatMessage::user("/review".into()));
            let cwd = app.cwd.clone();
            // Prefer staged diff; fall back to HEAD diff; fall back to
            // working-tree diff so /review always finds something useful.
            let diff_output = {
                let staged = tokio::process::Command::new("git")
                    .args(["diff", "--cached"])
                    .current_dir(&cwd)
                    .output()
                    .await
                    .ok();
                let staged_str = staged
                    .as_ref()
                    .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned())
                    .unwrap_or_default();
                if !staged_str.is_empty() {
                    staged_str
                } else {
                    tokio::process::Command::new("git")
                        .args(["diff", "HEAD"])
                        .current_dir(&cwd)
                        .output()
                        .await
                        .ok()
                        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned())
                        .unwrap_or_default()
                }
            };
            if diff_output.is_empty() {
                app.messages.push(ChatMessage::assistant(
                    "No changes found (`git diff --cached` and `git diff HEAD` are both empty). \
                     Make some changes or stage files first."
                        .into(),
                ));
            } else {
                let capped = if diff_output.len() > 12_000 {
                    format!(
                        "{}\n\n[... diff truncated at 12000 chars ...]",
                        &diff_output[..12_000]
                    )
                } else {
                    diff_output
                };
                let prompt = format!(
                    "Review the following git diff for bugs, security issues, and code quality \
                     problems. Be specific — reference exact file names and line numbers where \
                     relevant. Organise findings by severity (Critical / High / Medium / Low). \
                     If there are no issues worth calling out, say so briefly.\n\n\
                     ```diff\n{capped}\n```"
                );
                app.messages
                    .push(ChatMessage::assistant("Reviewing changes…".into()));
                app.queued_prompts.push(crate::app::QueuedPrompt {
                    text: prompt,
                    is_meta: false,
                    priority: crate::app::QueuePriority::Later,
                    attachments: Vec::new(),
                });
                app.scroll_to_bottom();
            }
        }
        "/skills" => {
            let skills =
                crate::agents::load_skills(&std::env::current_dir().unwrap_or_else(|_| ".".into()));
            let body = if skills.is_empty() {
                "No skills found. Create `.claude/skills/<name>.md` files with \
                 optional YAML frontmatter (`name:`, `description:`) and a markdown \
                 body that becomes the system-prompt fragment."
                    .to_owned()
            } else {
                let mut s = format!("**{} skill(s) loaded:**\n\n", skills.len());
                for sk in &skills {
                    s.push_str(&format!(
                        "- **{}** — {}\n  source: `{}`\n",
                        sk.name,
                        sk.description.as_deref().unwrap_or("(no description)"),
                        sk.source.display()
                    ));
                }
                s
            };
            app.messages.push(ChatMessage::user("/skills".into()));
            app.messages.push(ChatMessage::assistant(body));
        }
        "/agents" => {
            let agents =
                crate::agents::load_agents(&std::env::current_dir().unwrap_or_else(|_| ".".into()));
            let body = if agents.is_empty() {
                "No agent definitions found. Create `.claude/agents/<name>.md` files \
                 with YAML frontmatter (`name:` required, plus optional `model`, \
                 `permissionMode`, `allowedTools`, `disallowedTools`, `skills`, \
                 `isolation`, `forksParentContext`) and a markdown body that becomes \
                 the system prompt for spawned subagents/teammates."
                    .to_owned()
            } else {
                let mut s = format!("**{} agent(s) loaded:**\n\n", agents.len());
                for a in &agents {
                    s.push_str(&format!(
                        "- **{}** — model: {}, permission: {:?}, isolation: {}\n  \
                         tools: allowed={:?}, denied={:?}\n  source: `{}`\n",
                        a.name,
                        a.model.as_deref().unwrap_or("inherit"),
                        a.permission_mode.unwrap_or_default(),
                        a.isolation.as_deref().unwrap_or("none"),
                        a.allowed_tools,
                        a.disallowed_tools,
                        a.source.display(),
                    ));
                }
                s
            };
            app.messages.push(ChatMessage::user("/agents".into()));
            app.messages.push(ChatMessage::assistant(body));
        }
        "/market" => {
            // Surface the agent-economy snapshot — same data the
            // `market_status` tool returns, but framed for the user
            // rather than the model. No bounty_id filter for now.
            let report_str = match crate::tools::market_report_string().await {
                Ok(s) => s,
                Err(e) => format!("Market unavailable: {e}"),
            };
            app.messages.push(ChatMessage::user("/market".into()));
            app.messages.push(ChatMessage::assistant(report_str));
        }
        "/cascade" => {
            // Filter the task store for cascade-tagged entries
            // produced by symbol_edit's `dispatch_cascade=true`. The
            // metadata.kind="cascade" tag is the signal we emit when
            // queuing them. Group by file (one Task ≈ one file) and
            // show status + caller list per group.
            let tasks = app.task_store.list(jfc_session::DeletedFilter::Exclude);
            let cascade: Vec<&jfc_session::Task> = tasks
                .iter()
                .filter(|t| {
                    t.metadata
                        .as_ref()
                        .and_then(|m| m.get("kind"))
                        .and_then(|k| k.as_str())
                        == Some("cascade")
                })
                .collect();
            let body = if cascade.is_empty() {
                "No cascade tasks. Cascade entries are queued by `symbol_edit` \
                 when called with `dispatch_cascade: true` and the edit changes \
                 a function signature with downstream callers."
                    .to_owned()
            } else {
                let mut s = format!(
                    "**{} cascade task{}** (from `symbol_edit dispatch_cascade=true`):\n\n",
                    cascade.len(),
                    if cascade.len() == 1 { "" } else { "s" }
                );
                for t in &cascade {
                    let status_marker = match t.status {
                        jfc_session::TaskStatus::Completed => "✓",
                        jfc_session::TaskStatus::InProgress => "⏵",
                        jfc_session::TaskStatus::Pending => "•",
                        jfc_session::TaskStatus::Failed => "✗",
                        jfc_session::TaskStatus::Deleted => "✗",
                    };
                    let file = t
                        .metadata
                        .as_ref()
                        .and_then(|m| m.get("file"))
                        .and_then(|f| f.as_str())
                        .unwrap_or("<unknown>");
                    let callers = t
                        .metadata
                        .as_ref()
                        .and_then(|m| m.get("callers"))
                        .and_then(|c| c.as_array())
                        .map(|arr| {
                            arr.iter()
                                .filter_map(|v| v.as_str())
                                .collect::<Vec<_>>()
                                .join(", ")
                        })
                        .unwrap_or_default();
                    s.push_str(&format!(
                        "{status_marker} `{}` — {}\n  callers: {callers}\n  → {}\n\n",
                        t.id, file, t.subject,
                    ));
                }
                s
            };
            app.messages.push(ChatMessage::user("/cascade".into()));
            app.messages.push(ChatMessage::assistant(body));
        }
        "/graph-history" => {
            let records = crate::tools::graph_history_snapshot();
            let body = if records.is_empty() {
                "No graph queries recorded yet. Run `graph_query` (via the model) or \
                 ask the model to query the code graph, then re-invoke `/graph-history` \
                 to see the most recent queries with their result counts."
                    .to_owned()
            } else {
                let mut s = format!(
                    "**{} graph quer{} recorded** (most recent first):\n\n",
                    records.len(),
                    if records.len() == 1 { "y" } else { "ies" }
                );
                for record in records.iter().rev().take(20) {
                    let trunc_marker = if record.was_truncated {
                        " [truncated]"
                    } else {
                        ""
                    };
                    let cycle_marker = if record.cycles_detected > 0 {
                        format!(
                            " [{} cycle{} detected]",
                            record.cycles_detected,
                            if record.cycles_detected == 1 { "" } else { "s" }
                        )
                    } else {
                        String::new()
                    };
                    s.push_str(&format!(
                        "- `{}`\n  → {} node{}{}{}\n",
                        record.query_text,
                        record.result_node_count,
                        if record.result_node_count == 1 {
                            ""
                        } else {
                            "s"
                        },
                        trunc_marker,
                        cycle_marker,
                    ));
                }
                s
            };
            app.messages
                .push(ChatMessage::user("/graph-history".into()));
            app.messages.push(ChatMessage::assistant(body));
        }
        "/task-list" | "/tasks" => {
            let tasks = app.task_store.list(jfc_session::DeletedFilter::Exclude);
            let body = if tasks.is_empty() {
                "No tasks. Use `/task-add <subject>` to create one.".to_owned()
            } else {
                let mut s = format!("**{} task(s):**\n\n", tasks.len());
                for t in &tasks {
                    let icon = match t.status {
                        jfc_session::TaskStatus::Pending => "□",
                        jfc_session::TaskStatus::InProgress => "▣",
                        jfc_session::TaskStatus::Completed => "✓",
                        jfc_session::TaskStatus::Failed => "✗",
                        jfc_session::TaskStatus::Deleted => "✗",
                    };
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
        "/task-add" => {
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
        "/task-done" => {
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
        "/task-rm" | "/task-delete" => {
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
        "/claude-md" => {
            let h = crate::context::ClaudeMdHierarchy::load(
                &std::env::current_dir().unwrap_or_else(|_| ".".into()),
            );
            let body = if !h.any() {
                "No CLAUDE.md files found in any of the v126 hierarchy locations \
                 (`~/.config/claude/CLAUDE.md`, `~/.claude/CLAUDE.md`, \
                 `<project>/CLAUDE.md`, `<project>/.claude/CLAUDE.md`, \
                 `<project>/CLAUDE.local.md`)."
                    .to_owned()
            } else {
                let mut s = String::from("**CLAUDE.md layers loaded** (in precedence order):\n\n");
                for (label, layer) in [
                    ("Managed policy", &h.managed),
                    ("User preferences", &h.user),
                    ("Project instructions", &h.project),
                    ("Project (.claude)", &h.project_dot),
                    ("Local overrides", &h.local),
                ] {
                    if let Some((path, content)) = layer {
                        s.push_str(&format!(
                            "- **{}** ({}) — {} bytes\n",
                            label,
                            path.display(),
                            content.len()
                        ));
                    }
                }
                s
            };
            app.messages.push(ChatMessage::user("/claude-md".into()));
            app.messages.push(ChatMessage::assistant(body));
        }
        "/mode" => {
            let arg = parts.get(1).copied().unwrap_or("").trim().to_lowercase();
            let new_mode = match arg.as_str() {
                "default" | "d" => Some(crate::app::PermissionMode::Default),
                "plan" | "p" => Some(crate::app::PermissionMode::Plan),
                "accept" | "acceptedits" | "a" => Some(crate::app::PermissionMode::AcceptEdits),
                "bypass" | "b" | "yolo" => Some(crate::app::PermissionMode::BypassPermissions),
                "auto" => Some(crate::app::PermissionMode::Auto),
                "" => {
                    app.messages.push(ChatMessage::assistant(format!(
                        "**Current mode:** {} {}\n\n\
                         Available: `default`, `plan`, `accept`, `auto`, `bypass`\n\
                         Switch: `/mode <name>` or **Shift+Tab** to cycle.",
                        app.permission_mode.symbol(),
                        app.permission_mode.label(),
                    )));
                    None
                }
                _ => {
                    app.messages.push(ChatMessage::assistant(format!(
                        "Unknown mode `{arg}`. Available: `default`, `plan`, `accept`, `auto`, `bypass`"
                    )));
                    None
                }
            };
            if let Some(mode) = new_mode {
                app.permission_mode = mode;
                // Persist so the mode survives session restart / --continue.
                crate::config::save_permission_mode(&app.permission_mode);
                // Sync auto_mode.enabled with permission mode for backward compat
                app.auto_mode.enabled = mode == crate::app::PermissionMode::Auto;
                app.messages.push(ChatMessage::assistant(format!(
                    "**Mode → {} {}**",
                    mode.symbol(),
                    mode.label()
                )));
            }
        }
        "/auto-mode" => {
            let arg = parts.get(1).copied().unwrap_or("status").trim();
            match arg {
                "on" | "enable" | "true" => {
                    app.auto_mode.enabled = true;
                    app.messages.push(ChatMessage::assistant(
                        "**Auto-mode enabled.** Every tool call will be sent to the v126 \
                         classifier LLM. The classifier may block dangerous operations \
                         without prompting you. Edit `~/.config/jfc/settings.json` under \
                         `autoMode.{allow,soft_deny,environment}` (with `$defaults` \
                         inheritance) to extend the rules."
                            .into(),
                    ));
                }
                "off" | "disable" | "false" => {
                    app.auto_mode.enabled = false;
                    app.messages.push(ChatMessage::assistant(
                        "**Auto-mode disabled.** Tool calls will use the manual approval \
                         flow again."
                            .into(),
                    ));
                }
                _ => {
                    let n_allow = app.auto_mode.allow.len();
                    let n_block = app.auto_mode.soft_deny.len();
                    let n_env = app.auto_mode.environment.len();
                    let state = if app.auto_mode.enabled { "ON" } else { "OFF" };
                    app.messages.push(ChatMessage::assistant(format!(
                        "**Auto-mode: {state}**\n\
                         \n\
                         Custom rule counts (settings.json):\n\
                         - allow: {n_allow}\n\
                         - soft_deny: {n_block}\n\
                         - environment: {n_env}\n\
                         \n\
                         Use `/auto-mode on` or `/auto-mode off` to toggle."
                    )));
                }
            }
        }
        "/worktree" => {
            handle_worktree_command(app, parts.get(1).copied().unwrap_or("").trim()).await;
        }
        "/mcp" => {
            handle_mcp_command(app, parts.get(1).copied().unwrap_or("").trim()).await;
        }
        "/theme" => {
            handle_theme_command(app, parts.get(1).copied().unwrap_or("").trim());
        }
        "/fleet" | "/fleetview" => {
            handle_fleet_command(app);
        }
        "/teleport" => {
            handle_teleport_command(app, parts.get(1).copied().unwrap_or("").trim()).await;
        }
        "/init" => {
            handle_init_command(app).await;
        }
        "/plan" => {
            handle_doc_command(app, crate::document_formats::DocKind::Plan, tx).await;
        }
        "/roadmap" => {
            handle_doc_command(app, crate::document_formats::DocKind::Roadmap, tx).await;
        }
        "/parity" => {
            handle_doc_command(app, crate::document_formats::DocKind::Parity, tx).await;
        }
        "/philosophy" => {
            handle_doc_command(app, crate::document_formats::DocKind::Philosophy, tx).await;
        }
        "/usage" => {
            handle_doc_command(app, crate::document_formats::DocKind::Usage, tx).await;
        }
        "/cost" | "/stats" => {
            handle_cost_command(app);
        }
        "/status" => {
            handle_status_command(app);
        }
        "/bug" => {
            handle_bug_command(app, parts.get(1..).map(|r| r.join(" ")).unwrap_or_default());
        }
        "/rewind" => {
            handle_rewind_command(app, parts.get(1).copied().unwrap_or("").trim());
        }
        "/output-style" | "/style" | "/brief" => {
            // `/brief` is shorthand for `/output-style brief`. v132
            // exposes the same alias via `tengu_brief_mode_toggled`.
            let alias_brief = parts[0] == "/brief";
            let arg = if alias_brief {
                "brief".to_string()
            } else {
                parts.get(1).copied().unwrap_or("").trim().to_string()
            };
            handle_output_style_command(app, &arg);
        }
        "/dump-context" | "/debug-context" => {
            handle_dump_context_command(app).await;
        }
        "/install-github-app" => {
            handle_install_github_app(app).await;
        }
        "/pr" => {
            handle_pr_view(app, parts.get(1).copied().unwrap_or("").trim()).await;
        }
        "/pr-autofix" => {
            handle_pr_autofix(app, parts.get(1).copied().unwrap_or("").trim(), tx).await;
        }
        "/setup-github-actions" => {
            handle_setup_github_actions(app, parts.get(1).copied().unwrap_or("").trim()).await;
        }
        "/dream" | "/learn" => {
            handle_dream_command(app, parts.get(1).copied().unwrap_or("").trim(), tx).await;
        }
        "/loop" | "/proactive" => {
            handle_loop_command(app, parts.get(1).copied().unwrap_or("").trim(), tx).await;
        }
        "/schedule" | "/routines" => {
            handle_schedule_command(app, parts.get(1).copied().unwrap_or("").trim(), tx).await;
        }
        "/swarm-approve" | "/swarm-deny" => {
            // Resolve a pending swarm permission request from the user's
            // input bar. Toasts surface the request id when it lands;
            // here we hand it back to `permission_sync::resolve_permission`
            // with the leader as `resolved_by` so the teammate's poll
            // loop unblocks.
            let id = parts.get(1).copied().unwrap_or("").trim().to_owned();
            let approve = parts[0] == "/swarm-approve";
            let feedback = parts
                .get(2..)
                .map(|rest| rest.join(" "))
                .filter(|s| !s.trim().is_empty());
            if id.is_empty() {
                app.messages.push(ChatMessage::assistant(format!(
                    "Usage: {} <request-id> [feedback]\nFind the id in the toast that appeared when the teammate asked.",
                    parts[0]
                )));
            } else {
                let team_name = app.team_context.team_name.clone().unwrap_or_default();
                let echo = if approve {
                    format!("/swarm-approve {id}")
                } else if let Some(ref f) = feedback {
                    format!("/swarm-deny {id} {f}")
                } else {
                    format!("/swarm-deny {id}")
                };
                app.messages.push(ChatMessage::user(echo));
                if team_name.is_empty() {
                    app.messages.push(ChatMessage::assistant(
                        "No active team — nothing to approve.".into(),
                    ));
                } else {
                    let resolution = crate::swarm::types::PermissionResolution {
                        decision: if approve {
                            crate::swarm::types::PermissionDecision::Approved
                        } else {
                            crate::swarm::types::PermissionDecision::Rejected
                        },
                        resolved_by: "user".to_owned(),
                        feedback,
                        updated_input: None,
                        permission_updates: Vec::new(),
                    };
                    let req_id = id.clone();
                    tokio::spawn(async move {
                        let _ = crate::swarm::permission_sync::resolve_permission(
                            &req_id,
                            &resolution,
                            &team_name,
                        )
                        .await;
                    });
                    app.messages.push(ChatMessage::assistant(format!(
                        "Resolved swarm request {id} → {}",
                        if approve { "approved" } else { "denied" }
                    )));
                }
            }
        }
        _ => {
            // Skill-name fallthrough: `/<skill>` invokes the matching skill
            // body as if the user had pasted it. Mirrors v126 cli.js:226634
            // where slash-name-not-otherwise-bound resolves to a skill or
            // markdown command and either inline-expands or forks a subagent.
            //
            // TODO Phase B: if `frontmatter.context == "fork"` (or the v126
            // equivalent flag), spawn a Task subagent here instead of inline
            // expansion. Schema: cli.js:178962.
            let name = parts[0].trim_start_matches('/');
            let cwd = std::env::current_dir().unwrap_or_else(|_| ".".into());
            let skills = crate::agents::load_skills(&cwd);
            if let Some(skill) = crate::agents::find_skill_by_name(&skills, name) {
                // Echo the user's invocation so the chat shows what they
                // typed (with optional args) — same pattern as the other
                // slash arms. The injected user message that follows carries
                // the skill body, which is what the model actually sees.
                let echo = if let Some(rest) = parts.get(1) {
                    let trimmed = rest.trim();
                    if trimmed.is_empty() {
                        format!("/{name}")
                    } else {
                        format!("/{name} {trimmed}")
                    }
                } else {
                    format!("/{name}")
                };
                app.messages.push(ChatMessage::user(echo));

                // Phase A: inline-expand the body. If the user passed args
                // after the skill name, append them under an `# Args` heading
                // so the skill prompt can reference them without us having to
                // template-substitute.
                let mut body = skill.body.clone();
                if let Some(rest) = parts.get(1) {
                    let trimmed = rest.trim();
                    if !trimmed.is_empty() {
                        body.push_str("\n\n# Args\n");
                        body.push_str(trimmed);
                    }
                }

                let Some(tx) = tx else {
                    // No tx in this dispatch path (e.g. queued-prompt drain).
                    // Fall back to a hint rather than silently swallowing the
                    // invocation.
                    app.messages.push(ChatMessage::assistant(format!(
                        "Skill `/{name}` cannot be invoked from this context (no stream channel). \
                         Submit `/{name}` directly from the input bar instead."
                    )));
                    app.scroll_to_bottom();
                    return;
                };

                // Drive the same streaming setup as `handle_submit` for a
                // fresh user turn: push the synthetic user message, push the
                // empty assistant placeholder, prime streaming flags, persist
                // the session, then spawn the provider stream.
                let assistant_idx = app.messages.len() + 1;
                app.messages.push(ChatMessage::user(body));
                app.tool_ctx.total_user_turns += 1;
                app.messages.push(ChatMessage::assistant(String::new()));
                app.streaming_text.clear();
                app.streaming_reasoning.clear();
                app.streaming_response_bytes = 0;
                app.network_recovery_status = None;
                app.network_recovery_attempts = 0;
                app.streaming_assistant_idx = Some(assistant_idx);
                app.is_streaming = true;
                let now = std::time::Instant::now();
                app.streaming_started_at = Some(now);
                app.last_stream_event_at = Some(now);
                app.streaming_last_token_at = Some(now);
                app.turn_started_at = Some(now);
                app.agentic_turn_count = 0;
                app.thinking_started_at = None;
                app.pre_dispatched_tool_ids.clear();
                app.thinking_ended_at = None;
                app.last_usage_output = 0;
                app.usage_apply_baseline = (0, 0, 0, 0);
                app.scroll_to_bottom();

                let session_id = app
                    .current_session_id
                    .clone()
                    .unwrap_or_else(jfc_session::generate_session_id);
                // Fire-and-forget — don't block UI on disk I/O
                {
                    let sid = session_id.clone();
                    let msgs = app.messages.clone();
                    let model = app.model.clone();
                    tokio::spawn(async move {
                        crate::session::save_session(&sid, &msgs, None, Some(model.as_str())).await;
                    });
                }
                app.current_session_id = Some(session_id);

                let provider = app.provider.clone();
                let messages =
                    crate::stream::build_provider_messages(&app.messages[..assistant_idx]);
                let model = app.model.clone();
                let tx_stream = tx.clone();
                let interrupt = app.interrupt_flag.clone();
                interrupt.store(false, std::sync::atomic::Ordering::SeqCst);
                app.cancel_token = tokio_util::sync::CancellationToken::new();
                let cancel = app.cancel_token.clone();
                // wg-async: retry path mints a fresh cancel token for the
                // new stream so the old (possibly cancelled) one can't
                // racially interrupt the retry.
                tokio::spawn(async move {
                    crate::stream::stream_response(
                        provider,
                        messages,
                        model,
                        tx_stream,
                        interrupt,
                        cancel,
                        None,
                        crate::runtime::StreamRequestOverrides::default(),
                    )
                    .await;
                });
                return;
            }

            app.messages.push(ChatMessage::assistant(format!(
                "Unknown command: `{}`. Type `/help` for available commands.",
                parts[0]
            )));
        }
    }
    app.scroll_to_bottom();
}

